use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::error::{AppError, AppResult};
use crate::models::{Profile, SessionLocation, SessionRecord};

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
    pub rollout_path: PathBuf,
    pub relative_path: PathBuf,
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

fn read_snapshot(
    home: &Path,
    rollout_path: &Path,
    archived: bool,
    index: &HashMap<String, Value>,
) -> AppResult<ThreadSnapshot> {
    let raw = fs::read_to_string(rollout_path)?;
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
    let relative_path = rollout_path
        .strip_prefix(home)
        .map_err(|_| AppError::InvalidPath("rollout escapes CODEX_HOME".into()))?
        .to_path_buf();

    Ok(ThreadSnapshot {
        thread_id,
        title,
        cwd,
        provider_id,
        updated_at,
        archived,
        size_bytes,
        content_sha256,
        rollout_path: rollout_path.to_path_buf(),
        relative_path,
        normalized_lines,
        session_index_entry: index_entry,
    })
}

fn normalize_for_comparison(mut value: Value) -> AppResult<String> {
    if value.get("type").and_then(Value::as_str) == Some("session_meta") {
        if let Some(payload) = value.get_mut("payload").and_then(Value::as_object_mut) {
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

pub fn is_prefix(older: &ThreadSnapshot, newer: &ThreadSnapshot) -> bool {
    older.normalized_lines.len() <= newer.normalized_lines.len()
        && older
            .normalized_lines
            .iter()
            .zip(&newer.normalized_lines)
            .all(|(left, right)| left == right)
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
    fn detects_prefix_history() {
        let left = tempfile::tempdir().expect("left");
        let right = tempfile::tempdir().expect("right");
        let left_path = write_rollout(left.path(), "openai", "");
        let right_path = write_rollout(
            right.path(),
            "relay",
            "{\"type\":\"message\",\"payload\":{\"text\":\"later\"}}\n",
        );
        let a =
            read_snapshot(left.path(), &left_path, false, &HashMap::new()).expect("left snapshot");
        let b = read_snapshot(right.path(), &right_path, false, &HashMap::new())
            .expect("right snapshot");
        assert!(is_prefix(&a, &b));
        assert!(!is_prefix(&b, &a));
    }
}
