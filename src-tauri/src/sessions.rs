use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::error::{AppError, AppResult};
use crate::models::{Profile, SessionLocation, SessionRecord, ThreadSourceKind};

const SESSION_INDEX: &str = "session_index.jsonl";
const SESSION_DIRS: [(&str, bool); 2] = [("sessions", false), ("archived_sessions", true)];

#[derive(Debug, Clone)]
pub struct ThreadSnapshot {
    pub thread_id: String,
    pub title: String,
    pub cwd: Option<String>,
    pub provider_id: Option<String>,
    pub updated_at: Option<String>,
    pub archived: bool,
    pub size_bytes: u64,
    pub content_sha256: String,
    pub raw_sha256: String,
    pub source_kind: ThreadSourceKind,
    pub agent_nickname: Option<String>,
    pub parent_thread_id: Option<String>,
    pub rollout_path: PathBuf,
    pub normalized_lines: Vec<String>,
    pub session_index_entry: Value,
}

impl ThreadSnapshot {
    pub fn to_record(&self, profile: &Profile) -> SessionRecord {
        SessionRecord {
            thread_id: self.thread_id.clone(),
            title: self.title.clone(),
            cwd: self.cwd.clone(),
            provider_id: self.provider_id.clone(),
            updated_at: self.updated_at.clone(),
            archived: self.archived,
            size_bytes: self.size_bytes,
            sha256: self.content_sha256.clone(),
            locations: vec![SessionLocation {
                profile_id: profile.id.clone(),
                profile_name: profile.name.clone(),
                provider_id: profile.provider_id.clone(),
            }],
        }
    }
}

pub fn scan_profile(profile: &Profile) -> AppResult<Vec<ThreadSnapshot>> {
    let home = profile.home_path();
    if !home.is_dir() {
        return Err(AppError::InvalidPath(format!(
            "CODEX_HOME does not exist or is not a directory: {}",
            home.display()
        )));
    }
    let index = read_session_index(&home)?;
    let mut snapshots = Vec::new();

    for (directory, archived) in SESSION_DIRS {
        let root = home.join(directory);
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(&root).follow_links(false) {
            let entry = entry.map_err(|error| AppError::Message(error.to_string()))?;
            if !entry.file_type().is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy();
            if !name.starts_with("rollout-") || !name.ends_with(".jsonl") {
                continue;
            }
            match read_snapshot(&home, entry.path(), archived, &index) {
                Ok(snapshot) => snapshots.push(snapshot),
                Err(error) => eprintln!(
                    "skipping invalid rollout {}: {error}",
                    entry.path().display()
                ),
            }
        }
    }
    snapshots.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    let mut seen = std::collections::HashSet::new();
    snapshots.retain(|snapshot| seen.insert(snapshot.thread_id.clone()));
    Ok(snapshots)
}

#[derive(Default)]
pub struct IncrementalSessionCache {
    entries: HashMap<PathBuf, CachedThreadSnapshot>,
}

#[derive(Clone)]
struct CachedThreadSnapshot {
    size_bytes: u64,
    modified_nanos: u128,
    archived: bool,
    snapshot: ThreadSnapshot,
}

pub fn scan_profile_incremental(
    profile: &Profile,
    cache: &mut IncrementalSessionCache,
) -> AppResult<Vec<ThreadSnapshot>> {
    let home = profile.home_path();
    if !home.is_dir() {
        return Err(AppError::InvalidPath(format!(
            "CODEX_HOME does not exist or is not a directory: {}",
            home.display()
        )));
    }
    let index = read_session_index(&home)?;
    let mut snapshots = Vec::new();
    let mut refreshed_entries = HashMap::new();

    for (directory, archived) in SESSION_DIRS {
        let root = home.join(directory);
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(&root).follow_links(false) {
            let entry = entry.map_err(|error| AppError::Message(error.to_string()))?;
            if !entry.file_type().is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy();
            if !name.starts_with("rollout-") || !name.ends_with(".jsonl") {
                continue;
            }

            let path = entry.path().to_path_buf();
            let metadata = fs::metadata(&path)?;
            let modified_nanos = metadata
                .modified()?
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let cached = cache.entries.get(&path).filter(|cached| {
                cached.size_bytes == metadata.len()
                    && cached.modified_nanos == modified_nanos
                    && cached.archived == archived
            });
            let snapshot = match cached {
                Some(cached) => Ok(cached.snapshot.clone()),
                None => read_snapshot(&home, &path, archived, &index),
            };
            match snapshot {
                Ok(mut snapshot) => {
                    refresh_snapshot_index_metadata(&mut snapshot, &index);
                    snapshot.normalized_lines.clear();
                    refreshed_entries.insert(
                        path,
                        CachedThreadSnapshot {
                            size_bytes: metadata.len(),
                            modified_nanos,
                            archived,
                            snapshot: snapshot.clone(),
                        },
                    );
                    snapshots.push(snapshot);
                }
                Err(error) => eprintln!(
                    "skipping invalid rollout {}: {error}",
                    entry.path().display()
                ),
            }
        }
    }
    cache.entries = refreshed_entries;
    snapshots.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    let mut seen = std::collections::HashSet::new();
    snapshots.retain(|snapshot| seen.insert(snapshot.thread_id.clone()));
    Ok(snapshots)
}

fn refresh_snapshot_index_metadata(snapshot: &mut ThreadSnapshot, index: &HashMap<String, Value>) {
    let index_entry = index.get(&snapshot.thread_id).cloned().unwrap_or_else(|| {
        json!({
            "id": snapshot.thread_id,
            "thread_name": snapshot.thread_id,
        })
    });
    snapshot.title = index_title(&index_entry).unwrap_or_else(|| snapshot.thread_id.clone());
    snapshot.updated_at =
        index_updated_at(&index_entry).or_else(|| modified_at(&snapshot.rollout_path));
    snapshot.session_index_entry = index_entry;
}

pub fn find_snapshot_by_thread_id(
    profile: &Profile,
    thread_id: &str,
) -> AppResult<Option<ThreadSnapshot>> {
    Ok(scan_profile(profile)?
        .into_iter()
        .find(|snapshot| snapshot.thread_id == thread_id))
}

pub fn aggregate_sessions(profiles: &[Profile]) -> AppResult<Vec<SessionRecord>> {
    let mut records = HashMap::<String, SessionRecord>::new();
    for profile in profiles {
        if !profile.home_path().is_dir() {
            continue;
        }
        for snapshot in scan_profile(profile)? {
            let record = snapshot.to_record(profile);
            match records.get_mut(&snapshot.thread_id) {
                Some(existing) => {
                    existing.locations.extend(record.locations);
                    if record.updated_at > existing.updated_at {
                        existing.title = record.title;
                        existing.cwd = record.cwd;
                        existing.provider_id = record.provider_id;
                        existing.updated_at = record.updated_at;
                        existing.archived = record.archived;
                        existing.size_bytes = record.size_bytes;
                        existing.sha256 = record.sha256;
                    }
                }
                None => {
                    records.insert(snapshot.thread_id, record);
                }
            }
        }
    }
    let mut values = records.into_values().collect::<Vec<_>>();
    values.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    Ok(values)
}

pub(crate) fn read_snapshot(
    home: &Path,
    rollout_path: &Path,
    archived: bool,
    index: &HashMap<String, Value>,
) -> AppResult<ThreadSnapshot> {
    validate_rollout_path(home, rollout_path)?;
    let raw_bytes = fs::read(rollout_path)?;
    let raw_sha256 = hash_bytes(&raw_bytes);
    let raw = String::from_utf8(raw_bytes).map_err(|error| {
        AppError::InvalidSession(format!(
            "{} is not valid UTF-8: {error}",
            rollout_path.display()
        ))
    })?;
    let mut normalized_lines = Vec::new();
    let mut session_meta = None;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parsed: Value = serde_json::from_str(trimmed).map_err(|error| {
            AppError::InvalidSession(format!(
                "{} contains invalid JSON: {error}",
                rollout_path.display()
            ))
        })?;
        if session_meta.is_none()
            && parsed.get("type").and_then(Value::as_str) == Some("session_meta")
        {
            session_meta = Some(parsed.clone());
        }
        normalized_lines.push(normalize_for_comparison(parsed)?);
    }

    let meta = session_meta.ok_or_else(|| {
        AppError::InvalidSession(format!("{} has no session_meta", rollout_path.display()))
    })?;
    let payload = meta.get("payload").unwrap_or(&meta);
    let thread_id = payload
        .get("id")
        .or_else(|| payload.get("session_id"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| AppError::InvalidSession("session_meta has no thread id".into()))?
        .to_string();
    let cwd = payload
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::to_string);
    let provider_id = payload
        .get("model_provider")
        .and_then(Value::as_str)
        .map(str::to_string);
    let source_kind = classify_source(payload);
    let agent_nickname = explicit_agent_nickname(payload);
    let parent_thread_id = explicit_parent_thread_id(payload);
    let index_entry = index.get(&thread_id).cloned().unwrap_or_else(|| {
        json!({
            "id": thread_id,
            "thread_name": thread_id,
        })
    });
    let title = index_title(&index_entry).unwrap_or_else(|| thread_id.clone());
    let updated_at = index_updated_at(&index_entry).or_else(|| modified_at(rollout_path));
    let content_sha256 = hash_lines(&normalized_lines);
    let size_bytes = fs::metadata(rollout_path)?.len();
    Ok(ThreadSnapshot {
        thread_id,
        title,
        cwd,
        provider_id,
        updated_at,
        archived,
        size_bytes,
        content_sha256,
        raw_sha256,
        source_kind,
        agent_nickname,
        parent_thread_id,
        rollout_path: rollout_path.to_path_buf(),
        normalized_lines,
        session_index_entry: index_entry,
    })
}

pub fn thread_parent_ids(home: &Path) -> HashMap<String, String> {
    let Some(database_path) = latest_state_database(home) else {
        return HashMap::new();
    };
    let Ok(connection) = Connection::open_with_flags(
        database_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return HashMap::new();
    };
    let Ok(mut statement) =
        connection.prepare("SELECT parent_thread_id, child_thread_id FROM thread_spawn_edges")
    else {
        return HashMap::new();
    };
    let Ok(rows) = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) else {
        return HashMap::new();
    };

    rows.filter_map(Result::ok)
        .filter(|(parent, child)| !parent.trim().is_empty() && !child.trim().is_empty())
        .map(|(parent, child)| (child, parent))
        .collect()
}

fn latest_state_database(home: &Path) -> Option<PathBuf> {
    fs::read_dir(home)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            let version = name
                .strip_prefix("state_")?
                .strip_suffix(".sqlite")?
                .parse::<u64>()
                .ok()?;
            Some((version, entry.path()))
        })
        .max_by_key(|(version, _)| *version)
        .map(|(_, path)| path)
}

fn explicit_parent_thread_id(payload: &Value) -> Option<String> {
    const KEYS: [&str; 4] = [
        "parent_thread_id",
        "parentThreadId",
        "parent_id",
        "parentId",
    ];
    KEYS.iter()
        .find_map(|key| payload.get(key).and_then(Value::as_str))
        .or_else(|| {
            payload.pointer("/source/subagent").and_then(|source| {
                KEYS.iter()
                    .find_map(|key| source.get(key).and_then(Value::as_str))
            })
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn explicit_agent_nickname(payload: &Value) -> Option<String> {
    const KEYS: [&str; 2] = ["agent_nickname", "agentNickname"];
    const NESTED_PATHS: [&str; 2] = ["/source/subagent/thread_spawn", "/source/subagent"];

    KEYS.iter()
        .find_map(|key| payload.get(key).and_then(Value::as_str))
        .or_else(|| {
            NESTED_PATHS.iter().find_map(|path| {
                let source = payload.pointer(path)?;
                KEYS.iter()
                    .find_map(|key| source.get(key).and_then(Value::as_str))
            })
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn classify_source(payload: &Value) -> ThreadSourceKind {
    if payload
        .get("thread_source")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.eq_ignore_ascii_case("user"))
    {
        return ThreadSourceKind::Internal;
    }
    match payload.get("source") {
        Some(Value::String(value)) if value.eq_ignore_ascii_case("cli") => ThreadSourceKind::Cli,
        Some(Value::String(value)) if value.eq_ignore_ascii_case("vscode") => {
            ThreadSourceKind::Vscode
        }
        Some(Value::Object(source))
            if ["subagent", "review", "compact"]
                .iter()
                .any(|key| source.contains_key(*key)) =>
        {
            ThreadSourceKind::Internal
        }
        Some(_) | None => ThreadSourceKind::Unknown,
    }
}

fn validate_rollout_path(home: &Path, rollout_path: &Path) -> AppResult<()> {
    if fs::symlink_metadata(rollout_path)?.file_type().is_symlink() {
        return Err(AppError::InvalidPath(format!(
            "rollout is a symbolic link: {}",
            rollout_path.display()
        )));
    }
    let canonical_home = fs::canonicalize(home)?;
    let canonical_rollout = fs::canonicalize(rollout_path)?;
    if !canonical_rollout.starts_with(&canonical_home) {
        return Err(AppError::InvalidPath(format!(
            "rollout escapes CODEX_HOME: {}",
            rollout_path.display()
        )));
    }
    Ok(())
}

fn normalize_for_comparison(mut value: Value) -> AppResult<String> {
    if value.get("type").and_then(Value::as_str) == Some("session_meta") {
        if let Some(payload) = value.get_mut("payload").and_then(Value::as_object_mut) {
            if payload.contains_key("id") {
                payload.insert("id".into(), Value::String("__thread_id__".into()));
            }
            if payload.contains_key("session_id") {
                payload.insert("session_id".into(), Value::String("__thread_id__".into()));
            }
            payload.insert(
                "model_provider".into(),
                Value::String("__provider__".into()),
            );
        }
    }
    serde_json::to_string(&value).map_err(AppError::from)
}

fn hash_lines(lines: &[String]) -> String {
    let mut hasher = Sha256::new();
    for line in lines {
        hasher.update(line.as_bytes());
        hasher.update(b"\n");
    }
    hex::encode(hasher.finalize())
}

pub fn hash_file_raw(path: &Path) -> AppResult<String> {
    Ok(hash_bytes(&fs::read(path)?))
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn read_session_index(home: &Path) -> AppResult<HashMap<String, Value>> {
    let path = home.join(SESSION_INDEX);
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let reader = BufReader::new(File::open(path)?);
    let mut index = HashMap::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(id) = value
            .get("id")
            .or_else(|| value.get("thread_id"))
            .and_then(Value::as_str)
        {
            index.insert(id.to_string(), value);
        }
    }
    Ok(index)
}

fn index_title(value: &Value) -> Option<String> {
    ["thread_name", "title", "name"]
        .iter()
        .find_map(|key| value.get(key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn index_updated_at(value: &Value) -> Option<String> {
    ["updated_at", "updatedAt"]
        .iter()
        .find_map(|key| value.get(key))
        .and_then(|value| match value {
            Value::String(text) => Some(text.clone()),
            Value::Number(number) => number.as_i64().and_then(timestamp_to_rfc3339),
            _ => None,
        })
}

fn timestamp_to_rfc3339(timestamp: i64) -> Option<String> {
    let value = if timestamp > 1_000_000_000_000 {
        DateTime::<Utc>::from_timestamp_millis(timestamp)
    } else {
        DateTime::<Utc>::from_timestamp(timestamp, 0)
    }?;
    Some(value.to_rfc3339())
}

fn modified_at(path: &Path) -> Option<String> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    Some(DateTime::<Utc>::from(modified).to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_rollout(root: &Path, provider: &str, extra: &str) -> PathBuf {
        let directory = root.join("sessions/2026/08/13");
        fs::create_dir_all(&directory).expect("create sessions");
        let path = directory.join("rollout-test.jsonl");
        fs::write(
            &path,
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"thread-1\",\"cwd\":\"/tmp/project\",\"model_provider\":\"{provider}\"}}}}\n{{\"type\":\"message\",\"payload\":{{\"text\":\"hello\"}}}}\n{extra}"
            ),
        )
        .expect("write rollout");
        path
    }

    #[test]
    fn provider_does_not_change_content_hash() {
        let left = tempfile::tempdir().expect("left");
        let right = tempfile::tempdir().expect("right");
        let left_path = write_rollout(left.path(), "openai", "");
        let right_path = write_rollout(right.path(), "relay", "");
        let a =
            read_snapshot(left.path(), &left_path, false, &HashMap::new()).expect("left snapshot");
        let b = read_snapshot(right.path(), &right_path, false, &HashMap::new())
            .expect("right snapshot");
        assert_eq!(a.content_sha256, b.content_sha256);
    }

    #[test]
    fn reads_parent_relationships_from_the_latest_official_state_database() {
        let root = tempfile::tempdir().expect("root");
        for (version, parent) in [(1, "old-parent"), (3, "current-parent")] {
            let connection = Connection::open(root.path().join(format!("state_{version}.sqlite")))
                .expect("state database");
            connection
                .execute_batch(
                    "CREATE TABLE thread_spawn_edges (
                        parent_thread_id TEXT NOT NULL,
                        child_thread_id TEXT NOT NULL,
                        status TEXT NOT NULL
                    );",
                )
                .expect("spawn edge table");
            connection
                .execute(
                    "INSERT INTO thread_spawn_edges VALUES (?1, 'child', 'completed')",
                    [parent],
                )
                .expect("spawn edge");
        }

        let relationships = thread_parent_ids(root.path());

        assert_eq!(
            relationships.get("child").map(String::as_str),
            Some("current-parent")
        );
    }

    #[test]
    fn only_explicit_internal_sources_are_classified_as_internal() {
        assert_eq!(
            classify_source(&json!({
                "thread_source": "subagent",
                "source": { "subagent": { "other": "worker" } }
            })),
            ThreadSourceKind::Internal
        );
        assert_eq!(
            classify_source(&json!({
                "thread_source": "user",
                "source": { "desktop": { "surface": "codex" } }
            })),
            ThreadSourceKind::Unknown
        );
        assert_eq!(
            classify_source(&json!({
                "thread_source": "user",
                "source": ["vscode"]
            })),
            ThreadSourceKind::Unknown
        );
    }

    #[test]
    fn reads_subagent_nickname_and_rejects_missing_or_blank_names() {
        assert_eq!(
            explicit_agent_nickname(&json!({ "agent_nickname": "Fermat" })).as_deref(),
            Some("Fermat")
        );
        assert_eq!(
            explicit_agent_nickname(&json!({
                "source": {
                    "subagent": {
                        "thread_spawn": { "agent_nickname": "Boyle" }
                    }
                }
            }))
            .as_deref(),
            Some("Boyle")
        );
        assert_eq!(
            explicit_agent_nickname(&json!({ "agent_nickname": "   " })),
            None
        );
        assert_eq!(
            explicit_agent_nickname(&json!({
                "source": { "subagent": { "other": "guardian" } }
            })),
            None
        );
    }

    #[test]
    fn incremental_scan_refreshes_changed_rollouts_and_removes_deleted_rollouts() {
        let root = tempfile::tempdir().expect("root");
        let profile = Profile {
            id: "profile".into(),
            name: "profile".into(),
            kind: crate::models::ProfileKind::CustomApi,
            mode: crate::models::ProfileMode::External,
            codex_home: root.path().to_string_lossy().to_string(),
            provider_id: "openai".into(),
            app_path: None,
            discovery_source: "test".into(),
            providers: Vec::new(),
            config_profiles: Vec::new(),
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        };
        let rollout = write_rollout(root.path(), "openai", "");
        let mut cache = IncrementalSessionCache::default();
        let initial = scan_profile_incremental(&profile, &mut cache).expect("initial scan");
        assert_eq!(initial.len(), 1);
        let initial_hash = initial[0].raw_sha256.clone();
        cache
            .entries
            .get_mut(&rollout)
            .expect("cached rollout")
            .snapshot
            .raw_sha256 = "cached-marker".into();
        let unchanged = scan_profile_incremental(&profile, &mut cache).expect("cached scan");
        assert_eq!(unchanged[0].raw_sha256, "cached-marker");

        fs::write(
            &rollout,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"model_provider\":\"openai\"}}\n{\"type\":\"message\",\"payload\":{\"text\":\"changed\"}}\n",
        )
        .expect("update rollout");
        let changed = scan_profile_incremental(&profile, &mut cache).expect("changed scan");
        assert_eq!(changed.len(), 1);
        assert_ne!(initial_hash, changed[0].raw_sha256);

        fs::remove_file(rollout).expect("delete rollout");
        let deleted = scan_profile_incremental(&profile, &mut cache).expect("deleted scan");
        assert!(deleted.is_empty());
    }
}
