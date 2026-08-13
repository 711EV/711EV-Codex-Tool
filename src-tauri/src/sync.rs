use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Utc;
use fs2::FileExt;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::app_server;
use crate::error::{AppError, AppResult};
use crate::models::{Profile, SyncAction, SyncPlanItem, SyncPreview, SyncResult};
use crate::process;
use crate::profiles;
use crate::sessions::{is_prefix, scan_profile, ThreadSnapshot};
use crate::store::Store;

struct ClientRestartGuard {
    app_path: Option<String>,
    home: PathBuf,
    active: bool,
}

impl ClientRestartGuard {
    fn finish(&mut self) -> AppResult<()> {
        if !self.active {
            return Ok(());
        }
        self.active = false;
        process::restart(self.app_path.as_deref(), &self.home).map(|_| ())
    }
}

impl Drop for ClientRestartGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = process::restart(self.app_path.as_deref(), &self.home);
        }
    }
}

pub fn preview(
    source: &Profile,
    target: &Profile,
    selected_ids: &[String],
) -> AppResult<SyncPreview> {
    validate_pair(source, target)?;
    let selected = selected_ids.iter().cloned().collect::<HashSet<_>>();
    let source_map = snapshot_map(scan_profile(source)?);
    let target_map = snapshot_map(scan_profile(target)?);
    let mut items = Vec::new();

    for thread_id in selected {
        let Some(source_snapshot) = source_map.get(&thread_id) else {
            items.push(SyncPlanItem {
                thread_id: thread_id.clone(),
                title: thread_id,
                action: SyncAction::Invalid,
                reason: "source session was not found".into(),
                source_sha256: String::new(),
                target_sha256: None,
                size_bytes: 0,
            });
            continue;
        };
        let (action, reason, target_sha) = match target_map.get(&thread_id) {
            None => (
                SyncAction::Copy,
                "target profile does not contain this session",
                None,
            ),
            Some(target_snapshot)
                if source_snapshot.content_sha256 == target_snapshot.content_sha256 =>
            {
                (
                    SyncAction::SkipIdentical,
                    "source and target content are identical",
                    Some(target_snapshot.content_sha256.clone()),
                )
            }
            Some(target_snapshot) if is_prefix(target_snapshot, source_snapshot) => (
                SyncAction::Update,
                "target is an older prefix of source",
                Some(target_snapshot.content_sha256.clone()),
            ),
            Some(target_snapshot) if is_prefix(source_snapshot, target_snapshot) => (
                SyncAction::SkipTargetAhead,
                "target contains newer events",
                Some(target_snapshot.content_sha256.clone()),
            ),
            Some(target_snapshot) => (
                SyncAction::Conflict,
                "source and target histories have diverged",
                Some(target_snapshot.content_sha256.clone()),
            ),
        };
        items.push(SyncPlanItem {
            thread_id: source_snapshot.thread_id.clone(),
            title: source_snapshot.title.clone(),
            action,
            reason: reason.into(),
            source_sha256: source_snapshot.content_sha256.clone(),
            target_sha256: target_sha,
            size_bytes: source_snapshot.size_bytes,
        });
    }
    items.sort_by(|left, right| left.title.cmp(&right.title));

    Ok(summarize_preview(source, target, items))
}

#[allow(clippy::too_many_arguments)]
pub fn execute(
    data_dir: &Path,
    store: &Store,
    source: &Profile,
    target: &Profile,
    selected_ids: &[String],
    overwrite_conflicts: bool,
    force_close_target: bool,
) -> AppResult<SyncResult> {
    let job_id = Uuid::new_v4().to_string();
    store.begin_job(&job_id, &source.id, &target.id)?;
    let result = execute_inner(
        data_dir,
        store,
        &job_id,
        source,
        target,
        selected_ids,
        overwrite_conflicts,
        force_close_target,
    );
    if let Err(error) = &result {
        let _ = store.fail_job(&job_id, &error.to_string());
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn execute_inner(
    data_dir: &Path,
    store: &Store,
    job_id: &str,
    source: &Profile,
    target: &Profile,
    selected_ids: &[String],
    overwrite_conflicts: bool,
    force_close_target: bool,
) -> AppResult<SyncResult> {
    validate_pair(source, target)?;
    let preview = preview(source, target, selected_ids)?;
    let target_home = target.home_path();
    let shutdown = process::ensure_stopped(&target_home, force_close_target)?;
    let mut restart_guard = ClientRestartGuard {
        app_path: target
            .app_path
            .clone()
            .or_else(|| shutdown.executable.clone()),
        home: target_home.clone(),
        active: shutdown.closed,
    };

    let lock_path = data_dir.join("locks").join(format!("{}.lock", target.id));
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(lock_path)?;
    lock.try_lock_exclusive()
        .map_err(|_| AppError::Message("another sync is writing this target profile".into()))?;

    let source_map = snapshot_map(scan_profile(source)?);
    let target_map = snapshot_map(scan_profile(target)?);
    let executable_items = preview
        .items
        .iter()
        .filter(|item| {
            matches!(item.action, SyncAction::Copy | SyncAction::Update)
                || (overwrite_conflicts && item.action == SyncAction::Conflict)
        })
        .collect::<Vec<_>>();
    let backup_dir = if executable_items.is_empty() {
        None
    } else {
        Some(create_backup(
            data_dir,
            job_id,
            source,
            target,
            &executable_items,
            &target_map,
        )?)
    };

    let target_provider = profiles::read_provider(&target_home, &target.provider_id);
    validate_write_path(&target_home, &target_home.join("session_index.jsonl"))?;
    validate_write_path(&target_home, &target_home.join(".codex-global-state.json"))?;
    let original_index = read_optional(target_home.join("session_index.jsonl"))?;
    let original_global = read_optional(target_home.join(".codex-global-state.json"))?;
    let mut original_rollouts = HashMap::<PathBuf, Option<Vec<u8>>>::new();
    let mut copied = 0usize;
    let mut updated = 0usize;

    let write_result = (|| {
        for item in executable_items {
            let source_snapshot = source_map
                .get(&item.thread_id)
                .ok_or_else(|| AppError::InvalidSession("source changed during sync".into()))?;
            let target_path = target_map
                .get(&item.thread_id)
                .map(|snapshot| snapshot.rollout_path.clone())
                .unwrap_or_else(|| target_home.join(&source_snapshot.relative_path));
            validate_write_path(&target_home, &target_path)?;
            original_rollouts
                .entry(target_path.clone())
                .or_insert(read_optional(target_path.clone())?);
            copy_rewriting_provider(source_snapshot, &target_path, &target_provider)?;
            upsert_session_index(&target_home, source_snapshot)?;
            update_project_index(&target_home, source_snapshot.cwd.as_deref())?;
            match item.action {
                SyncAction::Copy => copied += 1,
                SyncAction::Update | SyncAction::Conflict => updated += 1,
                _ => {}
            }
        }
        Ok::<(), AppError>(())
    })();

    if let Err(error) = write_result {
        let rollback_result = rollback_all(
            &target_home,
            original_index.as_deref(),
            original_global.as_deref(),
            &original_rollouts,
        );
        if let Err(rollback_error) = rollback_result {
            return Err(AppError::Message(format!(
                "sync failed: {error}; rollback also failed: {rollback_error}. Backup: {}",
                backup_dir
                    .as_deref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "not created".into())
            )));
        }
        return Err(error);
    }

    let mut index_rebuilt = false;
    let mut warning = None;
    match app_server::rebuild_index(&target_home, target.app_path.as_deref()) {
        Ok(()) => index_rebuilt = true,
        Err(error) => warning = Some(format!(
            "sessions were synchronized, but the official index rebuild failed: {error}. Restart the client to retry."
        )),
    }

    let verification_result = (|| {
        let refreshed = snapshot_map(scan_profile(target)?);
        let mut baselines = Vec::new();
        for item in &preview.items {
            if !matches!(
                item.action,
                SyncAction::Copy | SyncAction::Update | SyncAction::Conflict
            ) {
                continue;
            }
            if item.action == SyncAction::Conflict && !overwrite_conflicts {
                continue;
            }
            let target_snapshot = refreshed.get(&item.thread_id).ok_or_else(|| {
                AppError::InvalidSession(format!(
                    "target verification failed for {}",
                    item.thread_id
                ))
            })?;
            if target_snapshot.content_sha256 != item.source_sha256 {
                return Err(AppError::InvalidSession(format!(
                    "target hash verification failed for {}",
                    item.thread_id
                )));
            }
            baselines.push((
                source.id.as_str(),
                target.id.as_str(),
                item.thread_id.as_str(),
                item.source_sha256.as_str(),
                target_snapshot.content_sha256.as_str(),
            ));
        }
        store.save_baselines(&baselines)?;
        Ok::<(), AppError>(())
    })();
    if let Err(error) = verification_result {
        let rollback_result = rollback_all(
            &target_home,
            original_index.as_deref(),
            original_global.as_deref(),
            &original_rollouts,
        );
        if let Err(rollback_error) = rollback_result {
            return Err(AppError::Message(format!(
                "verification failed: {error}; rollback also failed: {rollback_error}. Backup: {}",
                backup_dir
                    .as_deref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "not created".into())
            )));
        }
        return Err(error);
    }

    let conflicts = preview
        .items
        .iter()
        .filter(|item| item.action == SyncAction::Conflict && !overwrite_conflicts)
        .count();
    let skipped = preview
        .items
        .len()
        .saturating_sub(copied + updated + conflicts);
    let backup_string = backup_dir
        .as_ref()
        .map(|path| path.to_string_lossy().to_string());
    store.finish_job(
        job_id,
        copied,
        updated,
        skipped,
        conflicts,
        backup_string.as_deref(),
    )?;
    FileExt::unlock(&lock)?;

    if shutdown.closed {
        if let Err(error) = restart_guard.finish() {
            let restart_warning = format!("client restart failed: {error}");
            warning = Some(match warning {
                Some(existing) => format!("{existing} {restart_warning}"),
                None => restart_warning,
            });
        }
    }

    Ok(SyncResult {
        job_id: job_id.into(),
        copied_count: copied,
        updated_count: updated,
        skipped_count: skipped,
        conflict_count: conflicts,
        backup_dir: backup_string,
        index_rebuilt,
        warning,
    })
}

fn summarize_preview(source: &Profile, target: &Profile, items: Vec<SyncPlanItem>) -> SyncPreview {
    let copy_count = items
        .iter()
        .filter(|item| item.action == SyncAction::Copy)
        .count();
    let update_count = items
        .iter()
        .filter(|item| item.action == SyncAction::Update)
        .count();
    let conflict_count = items
        .iter()
        .filter(|item| item.action == SyncAction::Conflict)
        .count();
    let skip_count = items
        .iter()
        .filter(|item| {
            matches!(
                item.action,
                SyncAction::SkipIdentical | SyncAction::SkipTargetAhead | SyncAction::Invalid
            )
        })
        .count();
    let backup_bytes = items
        .iter()
        .filter(|item| matches!(item.action, SyncAction::Update | SyncAction::Conflict))
        .map(|item| item.size_bytes)
        .sum();
    SyncPreview {
        source_profile_id: source.id.clone(),
        target_profile_id: target.id.clone(),
        items,
        copy_count,
        update_count,
        skip_count,
        conflict_count,
        backup_bytes,
    }
}

fn validate_pair(source: &Profile, target: &Profile) -> AppResult<()> {
    if source.id == target.id || source.home_path() == target.home_path() {
        return Err(AppError::Message(
            "source and target profiles must be different".into(),
        ));
    }
    if !source.home_path().is_dir() || !target.home_path().is_dir() {
        return Err(AppError::InvalidPath(
            "source or target CODEX_HOME is unavailable".into(),
        ));
    }
    Ok(())
}

fn validate_write_path(home: &Path, target: &Path) -> AppResult<()> {
    let relative = target.strip_prefix(home).map_err(|_| {
        AppError::InvalidPath(format!("target escapes CODEX_HOME: {}", target.display()))
    })?;
    let mut current = fs::canonicalize(home)?;
    for component in relative.components() {
        let std::path::Component::Normal(component) = component else {
            return Err(AppError::InvalidPath(format!(
                "target contains an unsafe component: {}",
                target.display()
            )));
        };
        current.push(component);
        if current.exists() && fs::symlink_metadata(&current)?.file_type().is_symlink() {
            return Err(AppError::InvalidPath(format!(
                "target contains a symbolic link: {}",
                current.display()
            )));
        }
    }
    Ok(())
}

fn snapshot_map(values: Vec<ThreadSnapshot>) -> HashMap<String, ThreadSnapshot> {
    let mut snapshots = HashMap::new();
    for snapshot in values {
        snapshots
            .entry(snapshot.thread_id.clone())
            .or_insert(snapshot);
    }
    snapshots
}

fn create_backup(
    data_dir: &Path,
    job_id: &str,
    source: &Profile,
    target: &Profile,
    items: &[&SyncPlanItem],
    target_map: &HashMap<String, ThreadSnapshot>,
) -> AppResult<PathBuf> {
    let root = data_dir.join("backups").join(job_id);
    fs::create_dir_all(root.join("rollouts"))?;
    for name in ["session_index.jsonl", ".codex-global-state.json"] {
        let path = target.home_path().join(name);
        if path.is_file() {
            fs::copy(&path, root.join(name))?;
        }
    }
    for item in items {
        if let Some(snapshot) = target_map.get(&item.thread_id) {
            fs::copy(
                &snapshot.rollout_path,
                root.join("rollouts")
                    .join(format!("{}.jsonl", item.thread_id)),
            )?;
        }
    }
    let manifest = json!({
        "version": 1,
        "jobId": job_id,
        "createdAt": Utc::now().to_rfc3339(),
        "sourceProfileId": source.id,
        "targetProfileId": target.id,
        "targetCodexHome": target.codex_home,
        "threads": items.iter().map(|item| &item.thread_id).collect::<Vec<_>>(),
    });
    atomic_write(
        &root.join("manifest.json"),
        serde_json::to_string_pretty(&manifest)?.as_bytes(),
    )?;
    Ok(root)
}

fn copy_rewriting_provider(
    source: &ThreadSnapshot,
    target_path: &Path,
    provider_id: &str,
) -> AppResult<()> {
    let content = fs::read_to_string(&source.rollout_path)?;
    let mut output = Vec::new();
    let mut rewrote = false;
    for line in content.lines() {
        let mut value: Value = serde_json::from_str(line)?;
        if !rewrote && value.get("type").and_then(Value::as_str) == Some("session_meta") {
            let payload = value
                .get_mut("payload")
                .and_then(Value::as_object_mut)
                .ok_or_else(|| {
                    AppError::InvalidSession("session_meta payload is invalid".into())
                })?;
            payload.insert("model_provider".into(), Value::String(provider_id.into()));
            rewrote = true;
        }
        output.push(serde_json::to_string(&value)?);
    }
    if !rewrote {
        return Err(AppError::InvalidSession(
            "session_meta was not found".into(),
        ));
    }
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)?;
    }
    atomic_write(target_path, format!("{}\n", output.join("\n")).as_bytes())
}

fn upsert_session_index(home: &Path, snapshot: &ThreadSnapshot) -> AppResult<()> {
    let path = home.join("session_index.jsonl");
    let content = fs::read_to_string(&path).unwrap_or_default();
    let mut lines = Vec::new();
    let mut replaced = false;
    for line in content.lines() {
        let matches = serde_json::from_str::<Value>(line)
            .ok()
            .and_then(|value| value.get("id").and_then(Value::as_str).map(str::to_string))
            .as_deref()
            == Some(&snapshot.thread_id);
        if matches {
            lines.push(serde_json::to_string(&snapshot.session_index_entry)?);
            replaced = true;
        } else if !line.trim().is_empty() {
            lines.push(line.to_string());
        }
    }
    if !replaced {
        lines.push(serde_json::to_string(&snapshot.session_index_entry)?);
    }
    atomic_write(&path, format!("{}\n", lines.join("\n")).as_bytes())
}

fn update_project_index(home: &Path, cwd: Option<&str>) -> AppResult<()> {
    let Some(cwd) = cwd.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    let path = home.join(".codex-global-state.json");
    let mut value = fs::read_to_string(&path)
        .ok()
        .and_then(|content| serde_json::from_str::<Value>(&content).ok())
        .unwrap_or_else(|| json!({}));
    let object = value
        .as_object_mut()
        .ok_or_else(|| AppError::Message("global state is not a JSON object".into()))?;
    for key in ["project-order", "electron-saved-workspace-roots"] {
        let values = object.entry(key).or_insert_with(|| json!([]));
        let array = values
            .as_array_mut()
            .ok_or_else(|| AppError::Message(format!("global state {key} is not an array")))?;
        if !array.iter().any(|value| value.as_str() == Some(cwd)) {
            array.push(Value::String(cwd.to_string()));
        }
    }
    atomic_write(
        &path,
        format!("{}\n", serde_json::to_string_pretty(&value)?).as_bytes(),
    )
}

fn read_optional(path: PathBuf) -> AppResult<Option<Vec<u8>>> {
    if path.exists() {
        Ok(Some(fs::read(path)?))
    } else {
        Ok(None)
    }
}

fn rollback_metadata(home: &Path, index: Option<&[u8]>, global: Option<&[u8]>) -> AppResult<()> {
    restore_optional(&home.join("session_index.jsonl"), index)?;
    restore_optional(&home.join(".codex-global-state.json"), global)
}

fn rollback_all(
    home: &Path,
    index: Option<&[u8]>,
    global: Option<&[u8]>,
    rollouts: &HashMap<PathBuf, Option<Vec<u8>>>,
) -> AppResult<()> {
    rollback_metadata(home, index, global)?;
    for (path, content) in rollouts {
        restore_optional(path, content.as_deref())?;
    }
    Ok(())
}

fn restore_optional(path: &Path, content: Option<&[u8]>) -> AppResult<()> {
    match content {
        Some(content) => atomic_write(path, content),
        None if path.exists() => {
            fs::remove_file(path)?;
            Ok(())
        }
        None => Ok(()),
    }
}

fn atomic_write(path: &Path, content: &[u8]) -> AppResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::InvalidPath(format!("{} has no parent", path.display())))?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        Uuid::new_v4()
    ));
    let mut file = File::create(&temp)?;
    file.write_all(content)?;
    file.sync_all()?;
    replace_file(&temp, path)?;
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(temp: &Path, path: &Path) -> AppResult<()> {
    fs::rename(temp, path)?;
    Ok(())
}

#[cfg(windows)]
fn replace_file(temp: &Path, path: &Path) -> AppResult<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let temp_wide = temp
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let path_wide = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let result = unsafe {
        MoveFileExW(
            temp_wide.as_ptr(),
            path_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ProfileKind, ProfileMode};

    fn profile(id: &str, root: &Path, provider: &str) -> Profile {
        Profile {
            id: id.into(),
            name: id.into(),
            kind: ProfileKind::CustomApi,
            mode: ProfileMode::Managed,
            codex_home: root.to_string_lossy().to_string(),
            provider_id: provider.into(),
            app_path: None,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        }
    }

    fn write(root: &Path, events: &[&str]) {
        let dir = root.join("sessions/2026/08/13");
        fs::create_dir_all(&dir).expect("create");
        let mut lines = vec![
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"model_provider\":\"openai\"}}".to_string(),
        ];
        lines.extend(
            events.iter().map(|event| {
                format!("{{\"type\":\"message\",\"payload\":{{\"text\":\"{event}\"}}}}")
            }),
        );
        fs::write(
            dir.join("rollout-thread-1.jsonl"),
            format!("{}\n", lines.join("\n")),
        )
        .expect("write");
    }

    #[test]
    fn plans_copy_update_skip_and_conflict() {
        let source_dir = tempfile::tempdir().expect("source");
        let target_dir = tempfile::tempdir().expect("target");
        write(source_dir.path(), &["one", "two"]);
        write(target_dir.path(), &["one"]);
        let source = profile("source", source_dir.path(), "openai");
        let target = profile("target", target_dir.path(), "relay");
        let value = preview(&source, &target, &["thread-1".into()]).expect("preview");
        assert_eq!(value.items[0].action, SyncAction::Update);
    }

    #[test]
    fn executes_copy_with_target_provider_and_backup() {
        let data = tempfile::tempdir().expect("data");
        for child in ["backups", "locks"] {
            fs::create_dir_all(data.path().join(child)).expect("layout");
        }
        let source_dir = tempfile::tempdir().expect("source");
        let target_dir = tempfile::tempdir().expect("target");
        write(source_dir.path(), &["one"]);
        fs::create_dir_all(target_dir.path().join("sessions")).expect("target sessions");
        fs::write(
            target_dir.path().join("config.toml"),
            "model_provider = \"relay\"\n",
        )
        .expect("target config");

        let store = Store::open(data.path()).expect("store");
        let source = profile("source", source_dir.path(), "openai");
        let target = profile("target", target_dir.path(), "relay");
        let result = execute(
            data.path(),
            &store,
            &source,
            &target,
            &["thread-1".into()],
            false,
            false,
        )
        .expect("sync");

        assert_eq!(result.copied_count, 1);
        assert!(result.backup_dir.is_some());
        let copied = scan_profile(&target).expect("scan target");
        assert_eq!(copied.len(), 1);
        assert_eq!(copied[0].provider_id.as_deref(), Some("relay"));
        assert!(target_dir.path().join("session_index.jsonl").is_file());
    }
}
