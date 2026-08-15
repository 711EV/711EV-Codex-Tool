use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use chrono::Utc;
use fs2::FileExt;
use serde_json::Value;
use uuid::Uuid;

use crate::app_server::{AppServerClient, ThreadListFilters};
use crate::error::{AppError, AppResult};
use crate::models::{
    ArchiveCleanupItem, ArchiveCleanupPreview, ArchiveCleanupResult, ArchiveCleanupResultItem,
    Profile, ProviderBucket, ProviderSessionRecord, ProviderWorkspaceSnapshot, ReplicaMapping,
    ReplicaResultItem, ReplicationAction, ReplicationEligibility, ReplicationPlanItem,
    ReplicationPreview, ReplicationResult, ThreadSourceKind, UpdateSyncAction, UpdateSyncPlanItem,
    UpdateSyncPreview,
};
use crate::process;
use crate::profiles;
use crate::sessions::{
    find_snapshot_by_thread_id, hash_file_raw, scan_profile, thread_parent_ids, ThreadSnapshot,
};
use crate::store::Store;

struct ClientRestartGuard {
    app_path: Option<String>,
    home: PathBuf,
    active: bool,
}

impl ClientRestartGuard {
    fn finish(&mut self) -> AppResult<bool> {
        if !self.active {
            return Ok(false);
        }
        self.active = false;
        process::restart(self.app_path.as_deref(), &self.home)
    }
}

impl Drop for ClientRestartGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = process::restart(self.app_path.as_deref(), &self.home);
        }
    }
}

#[derive(Clone)]
pub struct ProviderScan {
    buckets: Vec<ProviderBucket>,
    sessions_by_provider: HashMap<String, Vec<ProviderSessionRecord>>,
}

impl ProviderScan {
    pub fn workspace(&self, requested_provider: Option<&str>) -> ProviderWorkspaceSnapshot {
        let selected_provider_id = requested_provider
            .filter(|provider_id| {
                self.buckets
                    .iter()
                    .any(|bucket| bucket.provider_id == *provider_id)
            })
            .map(str::to_string)
            .or_else(|| {
                self.buckets
                    .iter()
                    .find(|bucket| !bucket.is_current && bucket.active_root_thread_count > 0)
                    .map(|bucket| bucket.provider_id.clone())
            })
            .or_else(|| {
                self.buckets
                    .first()
                    .map(|bucket| bucket.provider_id.clone())
            });
        let provider_sessions = selected_provider_id
            .as_ref()
            .and_then(|provider_id| self.sessions_by_provider.get(provider_id))
            .cloned()
            .unwrap_or_default();

        ProviderWorkspaceSnapshot {
            provider_buckets: self.buckets.clone(),
            selected_provider_id,
            provider_sessions,
        }
    }
}

fn is_official_provider(provider_id: &str) -> bool {
    provider_id.eq_ignore_ascii_case("openai")
}

pub fn provider_scan_from_snapshots(
    profile: &Profile,
    mappings: &[ReplicaMapping],
    snapshots: Vec<ThreadSnapshot>,
) -> AppResult<ProviderScan> {
    let current_provider = current_provider(profile)?;
    let archived_thread_ids = snapshots
        .iter()
        .filter(|snapshot| snapshot.archived)
        .map(|snapshot| snapshot.thread_id.clone())
        .collect::<HashSet<_>>();
    let mapping_is_active = |mapping: &&ReplicaMapping| {
        mapping.deleted_at.is_none()
            && !archived_thread_ids.contains(&mapping.source_thread_id)
            && !archived_thread_ids.contains(&mapping.replica_thread_id)
    };
    let mut buckets = BTreeMap::<String, ProviderBucket>::new();
    let mut ensure_bucket = |provider_id: &str| {
        buckets
            .entry(provider_id.to_string())
            .or_insert_with(|| ProviderBucket {
                profile_id: profile.id.clone(),
                provider_id: provider_id.to_string(),
                is_current: provider_id == current_provider,
                active_root_thread_count: 0,
                archived_thread_count: 0,
                internal_thread_count: 0,
                replicated_count: 0,
            });
    };
    ensure_bucket(&current_provider);
    for provider in &profile.providers {
        ensure_bucket(&provider.id);
    }
    drop(ensure_bucket);

    let replica_mappings = mappings
        .iter()
        .filter(mapping_is_active)
        .map(|mapping| (mapping.replica_thread_id.as_str(), mapping))
        .collect::<HashMap<_, _>>();
    let source_mappings = mappings
        .iter()
        .filter(|mapping| {
            mapping.target_provider_id == current_provider && mapping_is_active(mapping)
        })
        .map(|mapping| (mapping.source_thread_id.as_str(), mapping))
        .collect::<HashMap<_, _>>();
    let parent_thread_ids = thread_parent_ids(&profile.home_path());
    let mut sessions_by_provider = HashMap::<String, Vec<ProviderSessionRecord>>::new();

    for snapshot in snapshots {
        let provider_id = snapshot
            .provider_id
            .clone()
            .unwrap_or_else(|| "unknown".into());
        let bucket = buckets
            .entry(provider_id.clone())
            .or_insert_with(|| ProviderBucket {
                profile_id: profile.id.clone(),
                provider_id: provider_id.clone(),
                is_current: provider_id == current_provider,
                active_root_thread_count: 0,
                archived_thread_count: 0,
                internal_thread_count: 0,
                replicated_count: 0,
            });
        if snapshot.archived {
            bucket.archived_thread_count += 1;
        } else if is_interactive(&snapshot.source_kind) {
            bucket.active_root_thread_count += 1;
        } else {
            bucket.internal_thread_count += 1;
        }

        let replica_mapping = replica_mappings.get(snapshot.thread_id.as_str()).copied();
        let mapping = source_mappings.get(snapshot.thread_id.as_str()).copied();
        let (eligibility, reason, replica_thread_id) =
            eligibility(&snapshot, &current_provider, replica_mapping, mapping);
        let parent_thread_id = parent_thread_ids
            .get(&snapshot.thread_id)
            .cloned()
            .or_else(|| snapshot.parent_thread_id.clone());
        sessions_by_provider
            .entry(provider_id.clone())
            .or_default()
            .push(ProviderSessionRecord {
                thread_id: snapshot.thread_id,
                provider_id,
                source_kind: snapshot.source_kind,
                archived: snapshot.archived,
                title: snapshot.title,
                cwd: snapshot.cwd,
                updated_at: snapshot.updated_at,
                size_bytes: snapshot.size_bytes,
                sha256: snapshot.raw_sha256,
                agent_nickname: snapshot.agent_nickname,
                parent_thread_id,
                eligibility,
                eligibility_reason: reason,
                replica_thread_id,
                is_replica: replica_mapping.is_some(),
            });
    }
    for mapping in mappings
        .iter()
        .filter(|mapping| mapping.status == "verified" && mapping_is_active(mapping))
    {
        if let Some(bucket) = buckets.get_mut(&mapping.source_provider_id) {
            bucket.replicated_count += 1;
        }
    }
    let mut values = buckets.into_values().collect::<Vec<_>>();
    values.sort_by(|left, right| {
        is_official_provider(&right.provider_id)
            .cmp(&is_official_provider(&left.provider_id))
            .then_with(|| right.is_current.cmp(&left.is_current))
            .then_with(|| {
                left.provider_id
                    .to_lowercase()
                    .cmp(&right.provider_id.to_lowercase())
            })
            .then_with(|| left.provider_id.cmp(&right.provider_id))
    });
    Ok(ProviderScan {
        buckets: values,
        sessions_by_provider,
    })
}

pub fn reconcile_archived_mappings(
    store: &Store,
    profile_id: &str,
    snapshots: &[ThreadSnapshot],
) -> AppResult<Vec<ReplicaMapping>> {
    let archived_thread_ids = snapshots
        .iter()
        .filter(|snapshot| snapshot.archived)
        .map(|snapshot| snapshot.thread_id.as_str())
        .collect::<HashSet<_>>();
    let mappings = store.list_replicas(profile_id)?;

    for mapping in mappings.iter().filter(|mapping| {
        mapping.deleted_at.is_none()
            && (archived_thread_ids.contains(mapping.source_thread_id.as_str())
                || archived_thread_ids.contains(mapping.replica_thread_id.as_str()))
    }) {
        store.mark_replica_deleted(&mapping.id)?;
    }

    store.list_replicas(profile_id)
}

pub fn preview_archive_cleanup(
    store: &Store,
    profile: &Profile,
    provider_id: &str,
) -> AppResult<ArchiveCleanupPreview> {
    preview_cleanup(store, profile, provider_id, CleanupScope::Archived)
}

pub fn preview_invalid_child_cleanup(
    store: &Store,
    profile: &Profile,
    provider_id: &str,
) -> AppResult<ArchiveCleanupPreview> {
    preview_cleanup(store, profile, provider_id, CleanupScope::InvalidChild)
}

fn preview_cleanup(
    store: &Store,
    profile: &Profile,
    provider_id: &str,
    scope: CleanupScope,
) -> AppResult<ArchiveCleanupPreview> {
    let snapshots = scan_profile(profile)?;
    reconcile_archived_mappings(store, &profile.id, &snapshots)?;
    let items = cleanup_items(&snapshots, provider_id, scope);
    let total_bytes = items.iter().map(|item| item.size_bytes).sum();

    Ok(ArchiveCleanupPreview {
        profile_id: profile.id.clone(),
        provider_id: provider_id.into(),
        total_count: items.len(),
        total_bytes,
        items,
    })
}

pub fn cleanup_archived_sessions(
    data_dir: &Path,
    store: &Store,
    profile: &Profile,
    provider_id: &str,
    thread_ids: &[String],
    force_close_client: bool,
) -> AppResult<ArchiveCleanupResult> {
    cleanup_sessions(
        data_dir,
        store,
        profile,
        provider_id,
        thread_ids,
        force_close_client,
        CleanupScope::Archived,
    )
}

pub fn cleanup_invalid_child_sessions(
    data_dir: &Path,
    store: &Store,
    profile: &Profile,
    provider_id: &str,
    thread_ids: &[String],
    force_close_client: bool,
) -> AppResult<ArchiveCleanupResult> {
    cleanup_sessions(
        data_dir,
        store,
        profile,
        provider_id,
        thread_ids,
        force_close_client,
        CleanupScope::InvalidChild,
    )
}

fn cleanup_sessions(
    data_dir: &Path,
    store: &Store,
    profile: &Profile,
    provider_id: &str,
    thread_ids: &[String],
    force_close_client: bool,
    scope: CleanupScope,
) -> AppResult<ArchiveCleanupResult> {
    let snapshots = scan_profile(profile)?;
    reconcile_archived_mappings(store, &profile.id, &snapshots)?;
    let available = cleanup_items(&snapshots, provider_id, scope)
        .into_iter()
        .map(|item| (item.thread_id.clone(), item))
        .collect::<HashMap<_, _>>();
    let mut candidates = Vec::new();
    let mut failed = Vec::new();
    let mut seen = HashSet::new();

    for thread_id in thread_ids {
        if !seen.insert(thread_id) {
            continue;
        }
        match available.get(thread_id) {
            Some(item) => candidates.push(item.clone()),
            None => failed.push(ArchiveCleanupResultItem {
                thread_id: thread_id.clone(),
                title: thread_id.clone(),
                message: scope.missing_message().into(),
            }),
        }
    }

    if candidates.is_empty() {
        return Ok(ArchiveCleanupResult {
            provider_id: provider_id.into(),
            deleted: Vec::new(),
            failed,
            client_restarted: false,
            warning: None,
        });
    }

    fs::create_dir_all(data_dir.join("locks"))?;
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(
            data_dir
                .join("locks")
                .join(format!("{}.provider.lock", profile.id)),
        )?;
    lock.try_lock_exclusive().map_err(|_| {
        AppError::Message("another Provider operation is using this CODEX_HOME".into())
    })?;

    let home = profile.home_path();
    let shutdown = process::ensure_stopped(&home, force_close_client)?;
    let mut restart_guard = ClientRestartGuard {
        app_path: profile
            .app_path
            .clone()
            .or_else(|| shutdown.executable.clone()),
        home: home.clone(),
        active: shutdown.closed,
    };
    // The preview can become stale while waiting for the provider lock or for the
    // client to stop. Re-scan inside the protected window before deleting anything.
    let protected_available = cleanup_items(&scan_profile(profile)?, provider_id, scope)
        .into_iter()
        .map(|item| (item.thread_id.clone(), item))
        .collect::<HashMap<_, _>>();
    let mut protected_candidates = Vec::new();
    for item in candidates {
        match protected_available.get(&item.thread_id) {
            Some(current) => protected_candidates.push(current.clone()),
            None => failed.push(ArchiveCleanupResultItem {
                thread_id: item.thread_id,
                title: item.title,
                message: format!("{}，未执行删除", scope.missing_message()),
            }),
        }
    }

    if protected_candidates.is_empty() {
        FileExt::unlock(&lock)?;
        let mut warnings = Vec::new();
        let client_restarted = match restart_guard.finish() {
            Ok(value) => value,
            Err(error) => {
                warnings.push(format!("客户端重启失败：{error}"));
                false
            }
        };
        return Ok(ArchiveCleanupResult {
            provider_id: provider_id.into(),
            deleted: Vec::new(),
            failed,
            client_restarted,
            warning: (!warnings.is_empty()).then(|| warnings.join("；")),
        });
    }

    let mut client = AppServerClient::start(&home, profile.app_path.as_deref())?;
    let mut delete_attempts = Vec::new();

    for item in protected_candidates {
        let delete_error = client
            .thread_delete(&item.thread_id)
            .err()
            .map(|error| error.to_string());
        delete_attempts.push((item, delete_error));
    }
    drop(client);

    let mut deleted = Vec::new();
    let mut warnings = Vec::new();
    match scan_profile(profile) {
        Ok(snapshots) => {
            let remaining = snapshots
                .into_iter()
                .map(|snapshot| snapshot.thread_id)
                .collect::<HashSet<_>>();
            for (item, delete_error) in delete_attempts {
                classify_cleanup_attempt(
                    item,
                    delete_error,
                    &remaining,
                    scope,
                    &mut deleted,
                    &mut failed,
                );
            }
        }
        Err(error) => {
            warnings.push(format!("删除后重新扫描失败：{error}"));
            for (item, delete_error) in delete_attempts {
                failed.push(ArchiveCleanupResultItem {
                    thread_id: item.thread_id,
                    title: item.title,
                    message: match delete_error {
                        Some(delete_error) => {
                            format!("{delete_error}；删除后无法重新扫描确认结果")
                        }
                        None => "Codex 已接受删除请求，但无法重新扫描确认结果".into(),
                    },
                });
            }
        }
    }

    FileExt::unlock(&lock)?;
    let client_restarted = match restart_guard.finish() {
        Ok(value) => value,
        Err(error) => {
            warnings.push(format!("客户端重启失败：{error}"));
            false
        }
    };

    Ok(ArchiveCleanupResult {
        provider_id: provider_id.into(),
        deleted,
        failed,
        client_restarted,
        warning: (!warnings.is_empty()).then(|| warnings.join("；")),
    })
}

#[derive(Clone, Copy)]
enum CleanupScope {
    Archived,
    InvalidChild,
}

impl CleanupScope {
    fn missing_message(self) -> &'static str {
        match self {
            Self::Archived => "会话已不在当前供应商的归档列表中",
            Self::InvalidChild => "会话已不在当前供应商的子会话列表中",
        }
    }

    fn deleted_message(self, recovered_from_error: bool) -> &'static str {
        match (self, recovered_from_error) {
            (Self::Archived, true) => "本地归档会话已不存在，已按删除完成处理",
            (Self::Archived, false) => "已永久删除归档会话",
            (Self::InvalidChild, true) => "本地子会话已不存在，已按删除完成处理",
            (Self::InvalidChild, false) => "已永久删除子会话",
        }
    }

    fn still_exists_message(self) -> &'static str {
        match self {
            Self::Archived => "Codex 返回删除成功，但本地归档文件仍然存在",
            Self::InvalidChild => "Codex 返回删除成功，但本地子会话文件仍然存在",
        }
    }
}

fn classify_cleanup_attempt(
    item: ArchiveCleanupItem,
    delete_error: Option<String>,
    remaining: &HashSet<String>,
    scope: CleanupScope,
    deleted: &mut Vec<ArchiveCleanupResultItem>,
    failed: &mut Vec<ArchiveCleanupResultItem>,
) {
    if !remaining.contains(&item.thread_id) {
        deleted.push(ArchiveCleanupResultItem {
            thread_id: item.thread_id,
            title: item.title,
            message: scope.deleted_message(delete_error.is_some()).into(),
        });
        return;
    }

    failed.push(ArchiveCleanupResultItem {
        thread_id: item.thread_id,
        title: item.title,
        message: delete_error.unwrap_or_else(|| scope.still_exists_message().into()),
    });
}

fn cleanup_items(
    snapshots: &[ThreadSnapshot],
    provider_id: &str,
    scope: CleanupScope,
) -> Vec<ArchiveCleanupItem> {
    snapshots
        .iter()
        .filter(|snapshot| {
            let in_scope = match scope {
                CleanupScope::Archived => snapshot.archived,
                CleanupScope::InvalidChild => {
                    !snapshot.archived && snapshot.source_kind == ThreadSourceKind::Internal
                }
            };
            in_scope && snapshot.provider_id.as_deref().unwrap_or("unknown") == provider_id
        })
        .map(|snapshot| ArchiveCleanupItem {
            thread_id: snapshot.thread_id.clone(),
            title: snapshot.title.clone(),
            provider_id: provider_id.into(),
            source_kind: snapshot.source_kind.clone(),
            updated_at: snapshot.updated_at.clone(),
            size_bytes: snapshot.size_bytes,
        })
        .collect()
}

#[cfg(test)]
pub fn provider_list(store: &Store, profile: &Profile) -> AppResult<Vec<ProviderBucket>> {
    let snapshots = scan_profile(profile)?;
    let mappings = reconcile_archived_mappings(store, &profile.id, &snapshots)?;
    Ok(provider_scan_from_snapshots(profile, &mappings, snapshots)?.buckets)
}

#[cfg(test)]
pub fn provider_sessions(
    store: &Store,
    profile: &Profile,
    provider_id: &str,
) -> AppResult<Vec<ProviderSessionRecord>> {
    let snapshots = scan_profile(profile)?;
    let mappings = reconcile_archived_mappings(store, &profile.id, &snapshots)?;
    Ok(provider_scan_from_snapshots(profile, &mappings, snapshots)?
        .sessions_by_provider
        .remove(provider_id)
        .unwrap_or_default())
}

pub fn preview(
    store: &Store,
    profile: &Profile,
    selected_ids: &[String],
) -> AppResult<ReplicationPreview> {
    let target_provider = current_provider(profile)?;
    let scanned_snapshots = scan_profile(profile)?;
    let mappings = reconcile_archived_mappings(store, &profile.id, &scanned_snapshots)?;
    let snapshots = scanned_snapshots
        .into_iter()
        .map(|snapshot| (snapshot.thread_id.clone(), snapshot))
        .collect::<HashMap<_, _>>();
    let replica_ids = mappings
        .iter()
        .filter(|mapping| mapping.deleted_at.is_none())
        .map(|mapping| mapping.replica_thread_id.as_str())
        .collect::<HashSet<_>>();
    let source_mappings = mappings
        .iter()
        .filter(|mapping| {
            mapping.target_provider_id == target_provider && mapping.deleted_at.is_none()
        })
        .map(|mapping| (mapping.source_thread_id.as_str(), mapping))
        .collect::<HashMap<_, _>>();
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    for thread_id in selected_ids {
        if !seen.insert(thread_id) {
            continue;
        }
        let Some(snapshot) = snapshots.get(thread_id) else {
            items.push(ReplicationPlanItem {
                thread_id: thread_id.clone(),
                title: thread_id.clone(),
                source_provider_id: "unknown".into(),
                action: ReplicationAction::Invalid,
                reason: "来源会话不存在或 rollout 无效".into(),
                source_sha256: String::new(),
                replica_thread_id: None,
                size_bytes: 0,
            });
            continue;
        };
        let mapping = source_mappings.get(snapshot.thread_id.as_str()).copied();
        let (action, reason, replica_thread_id) = plan_action(
            snapshot,
            &target_provider,
            replica_ids.contains(snapshot.thread_id.as_str()),
            mapping,
        );
        items.push(ReplicationPlanItem {
            thread_id: snapshot.thread_id.clone(),
            title: snapshot.title.clone(),
            source_provider_id: snapshot
                .provider_id
                .clone()
                .unwrap_or_else(|| "unknown".into()),
            action,
            reason,
            source_sha256: snapshot.raw_sha256.clone(),
            replica_thread_id,
            size_bytes: snapshot.size_bytes,
        });
    }
    let create_count = items
        .iter()
        .filter(|item| item.action == ReplicationAction::CreateReplica)
        .count();
    let invalid_count = items
        .iter()
        .filter(|item| {
            matches!(
                item.action,
                ReplicationAction::Invalid
                    | ReplicationAction::SkipArchived
                    | ReplicationAction::SkipInternal
                    | ReplicationAction::SkipCurrentProvider
                    | ReplicationAction::SourceUpdated
            )
        })
        .count();
    let skip_count = items.len().saturating_sub(create_count + invalid_count);
    let estimated_bytes = items
        .iter()
        .filter(|item| item.action == ReplicationAction::CreateReplica)
        .map(|item| item.size_bytes)
        .sum();
    Ok(ReplicationPreview {
        profile_id: profile.id.clone(),
        target_provider_id: target_provider,
        items,
        create_count,
        skip_count,
        invalid_count,
        estimated_bytes,
    })
}

pub fn execute(
    data_dir: &Path,
    store: &Store,
    profile: &Profile,
    selected_ids: &[String],
    force_close_client: bool,
) -> AppResult<ReplicationResult> {
    let target_provider = current_provider(profile)?;
    let job_id = Uuid::new_v4().to_string();
    execute_inner(
        data_dir,
        store,
        profile,
        selected_ids,
        force_close_client,
        &job_id,
        &target_provider,
        false,
    )
}

pub fn migrate(
    data_dir: &Path,
    store: &Store,
    profile: &Profile,
    selected_ids: &[String],
    force_close_client: bool,
) -> AppResult<ReplicationResult> {
    let target_provider = current_provider(profile)?;
    let job_id = Uuid::new_v4().to_string();
    execute_inner(
        data_dir,
        store,
        profile,
        selected_ids,
        force_close_client,
        &job_id,
        &target_provider,
        true,
    )
}

pub fn preview_updates(store: &Store, profile: &Profile) -> AppResult<UpdateSyncPreview> {
    let target_provider = current_provider(profile)?;
    let scanned_snapshots = scan_profile(profile)?;
    let mappings = reconcile_archived_mappings(store, &profile.id, &scanned_snapshots)?
        .into_iter()
        .filter(|mapping| {
            mapping.target_provider_id == target_provider
                && mapping.status == "verified"
                && mapping.deleted_at.is_none()
        })
        .collect::<Vec<_>>();
    let snapshots = scanned_snapshots
        .into_iter()
        .map(|snapshot| (snapshot.thread_id.clone(), snapshot))
        .collect::<HashMap<_, _>>();
    let items = update_sync_plan_items(&mappings, &snapshots);
    let update_count = items
        .iter()
        .filter(|item| {
            matches!(
                item.action,
                UpdateSyncAction::SourceUpdated | UpdateSyncAction::ReplicaUpdated
            )
        })
        .count();
    let conflict_count = items
        .iter()
        .filter(|item| item.action == UpdateSyncAction::Conflict)
        .count();
    let invalid_count = items
        .iter()
        .filter(|item| item.action == UpdateSyncAction::Invalid)
        .count();
    Ok(UpdateSyncPreview {
        profile_id: profile.id.clone(),
        target_provider_id: target_provider,
        items,
        update_count,
        conflict_count,
        invalid_count,
    })
}

fn update_sync_plan_items(
    mappings: &[ReplicaMapping],
    snapshots: &HashMap<String, ThreadSnapshot>,
) -> Vec<UpdateSyncPlanItem> {
    mappings
        .iter()
        .filter_map(|mapping| {
            let source = snapshots.get(&mapping.source_thread_id)?;
            let replica = snapshots.get(&mapping.replica_thread_id)?;
            if source.archived
                || replica.archived
                || !is_interactive(&source.source_kind)
                || !is_interactive(&replica.source_kind)
                || source.provider_id.as_deref() != Some(mapping.source_provider_id.as_str())
                || replica.provider_id.as_deref() != Some(mapping.target_provider_id.as_str())
            {
                return None;
            }
            let source_updated = mapping.source_sha256 != source.raw_sha256;
            let replica_updated = mapping.replica_sha256 != replica.raw_sha256;
            if !source_updated && !replica_updated {
                return None;
            }
            let (action, title, reason) = if source_updated && replica_updated {
                (
                    UpdateSyncAction::Conflict,
                    source.title.clone(),
                    "来源和副本都产生了新内容，无法自动覆盖".into(),
                )
            } else if source_updated {
                (
                    UpdateSyncAction::SourceUpdated,
                    source.title.clone(),
                    "将用来源的最新内容更新现有副本".into(),
                )
            } else {
                (
                    UpdateSyncAction::ReplicaUpdated,
                    replica.title.clone(),
                    "将用副本的最新内容更新现有来源会话".into(),
                )
            };
            Some(UpdateSyncPlanItem {
                mapping_id: mapping.id.clone(),
                source_thread_id: mapping.source_thread_id.clone(),
                replica_thread_id: mapping.replica_thread_id.clone(),
                title,
                source_provider_id: mapping.source_provider_id.clone(),
                target_provider_id: mapping.target_provider_id.clone(),
                action,
                reason,
            })
        })
        .collect()
}

pub fn sync_updates(
    data_dir: &Path,
    store: &Store,
    profile: &Profile,
    force_close_client: bool,
) -> AppResult<ReplicationResult> {
    let target_provider = current_provider(profile)?;
    let mapping_count = store
        .list_replicas(&profile.id)?
        .into_iter()
        .filter(|mapping| {
            mapping.target_provider_id == target_provider
                && mapping.status == "verified"
                && mapping.deleted_at.is_none()
        })
        .count();
    let job_id = Uuid::new_v4().to_string();
    if mapping_count == 0 {
        return Ok(ReplicationResult {
            job_id: job_id.clone(),
            target_provider_id: target_provider,
            created: Vec::new(),
            skipped: Vec::new(),
            failed: Vec::new(),
            client_restarted: false,
            warning: None,
        });
    }
    sync_updates_inner(
        data_dir,
        store,
        profile,
        force_close_client,
        &job_id,
        &target_provider,
    )
}

#[allow(clippy::too_many_arguments)]
fn sync_updates_inner(
    data_dir: &Path,
    store: &Store,
    profile: &Profile,
    force_close_client: bool,
    job_id: &str,
    expected_target_provider: &str,
) -> AppResult<ReplicationResult> {
    let home = profile.home_path();
    let shutdown = process::ensure_stopped(&home, force_close_client)?;
    let mut restart_guard = ClientRestartGuard {
        app_path: profile
            .app_path
            .clone()
            .or_else(|| shutdown.executable.clone()),
        home: home.clone(),
        active: shutdown.closed,
    };
    fs::create_dir_all(data_dir.join("locks"))?;
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(
            data_dir
                .join("locks")
                .join(format!("{}.provider.lock", profile.id)),
        )?;
    lock.try_lock_exclusive().map_err(|_| {
        AppError::Message("another Provider replication is using this CODEX_HOME".into())
    })?;

    assert_current_provider(profile, expected_target_provider)?;
    {
        let mut client = AppServerClient::start(&home, profile.app_path.as_deref())?;
        cross_check_app_server_provider(&mut client, expected_target_provider)?;
        client.thread_list_all(&interactive_filters(None))?;
    }

    let scanned_snapshots = scan_profile(profile)?;
    let mappings = reconcile_archived_mappings(store, &profile.id, &scanned_snapshots)?
        .into_iter()
        .filter(|mapping| {
            mapping.target_provider_id == expected_target_provider
                && mapping.status == "verified"
                && mapping.deleted_at.is_none()
        })
        .collect::<Vec<_>>();
    let snapshots = scanned_snapshots
        .into_iter()
        .map(|snapshot| (snapshot.thread_id.clone(), snapshot))
        .collect::<HashMap<_, _>>();
    let plan_items = update_sync_plan_items(&mappings, &snapshots);
    let mappings_by_id = mappings
        .iter()
        .map(|mapping| (mapping.id.as_str(), mapping))
        .collect::<HashMap<_, _>>();
    let mut created = Vec::new();
    let mut skipped = Vec::new();
    let mut failed = Vec::new();
    let mut warnings = Vec::new();
    let mut updated_thread_ids = Vec::new();

    for item in &plan_items {
        if matches!(
            item.action,
            UpdateSyncAction::Conflict | UpdateSyncAction::Invalid
        ) {
            skipped.push(update_sync_skipped_item(item));
            continue;
        }
        let Some(mapping) = mappings_by_id.get(item.mapping_id.as_str()).copied() else {
            failed.push(update_sync_failed_item(item, "复制映射在执行前消失"));
            continue;
        };
        let Some(source) = snapshots.get(&mapping.source_thread_id) else {
            failed.push(update_sync_failed_item(item, "来源会话在执行前消失"));
            continue;
        };
        let Some(replica) = snapshots.get(&mapping.replica_thread_id) else {
            failed.push(update_sync_failed_item(item, "副本会话在执行前消失"));
            continue;
        };
        if hash_file_raw(&source.rollout_path)? != source.raw_sha256
            || hash_file_raw(&replica.rollout_path)? != replica.raw_sha256
        {
            failed.push(update_sync_failed_item(item, "会话在同步前发生变化"));
            continue;
        }

        assert_current_provider(profile, expected_target_provider)?;
        let source_updated = item.action == UpdateSyncAction::SourceUpdated;
        let (updated, target, target_provider, message) = if source_updated {
            (
                source,
                replica,
                mapping.target_provider_id.as_str(),
                "已用来源的最新内容更新现有副本",
            )
        } else {
            (
                replica,
                source,
                mapping.source_provider_id.as_str(),
                "已用副本的最新内容更新现有来源会话",
            )
        };
        if let Err(error) = rewrite_existing_session(
            &home,
            &updated.rollout_path,
            &target.rollout_path,
            &target.thread_id,
            target_provider,
        ) {
            failed.push(update_sync_failed_item(item, &error.to_string()));
            continue;
        }
        let Some(updated_source) = find_snapshot_by_thread_id(profile, &mapping.source_thread_id)?
        else {
            failed.push(update_sync_failed_item(item, "更新后无法读取来源会话"));
            continue;
        };
        let Some(updated_replica) =
            find_snapshot_by_thread_id(profile, &mapping.replica_thread_id)?
        else {
            failed.push(update_sync_failed_item(item, "更新后无法读取副本会话"));
            continue;
        };
        let updated_target = if source_updated {
            &updated_replica
        } else {
            &updated_source
        };
        if updated_target.provider_id.as_deref() != Some(target_provider)
            || updated_target.content_sha256 != updated.content_sha256
        {
            failed.push(update_sync_failed_item(
                item,
                "更新后的会话内容或供应商验证失败",
            ));
            continue;
        }
        if let Err(error) = store.update_replica_hashes(
            &mapping.id,
            &updated_source.raw_sha256,
            &updated_replica.raw_sha256,
        ) {
            failed.push(update_sync_failed_item(
                item,
                &format!("更新同步状态失败：{error}"),
            ));
            continue;
        }
        updated_thread_ids.push(target.thread_id.clone());
        created.push(ReplicaResultItem {
            source_thread_id: mapping.source_thread_id.clone(),
            replica_thread_id: Some(mapping.replica_thread_id.clone()),
            title: updated.title.clone(),
            status: "synchronized".into(),
            message: message.into(),
        });
    }

    if !updated_thread_ids.is_empty() {
        match AppServerClient::start(&home, profile.app_path.as_deref()) {
            Ok(mut client) => {
                if let Err(error) = client.thread_list_all(&interactive_filters(None)) {
                    warnings.push(format!("会话列表刷新失败：{error}"));
                } else {
                    for thread_id in updated_thread_ids {
                        if let Err(error) = client.thread_read(&thread_id, true) {
                            warnings.push(format!("会话 {thread_id} 验证失败：{error}"));
                        }
                    }
                }
            }
            Err(error) => warnings.push(format!("会话索引刷新失败：{error}")),
        }
    }

    FileExt::unlock(&lock)?;
    let client_restarted = match restart_guard.finish() {
        Ok(value) => value,
        Err(error) => {
            warnings.push(format!("客户端重启失败：{error}"));
            false
        }
    };
    Ok(ReplicationResult {
        job_id: job_id.into(),
        target_provider_id: expected_target_provider.into(),
        created,
        skipped,
        failed,
        client_restarted,
        warning: (!warnings.is_empty()).then(|| warnings.join("；")),
    })
}

#[allow(clippy::too_many_arguments)]
fn execute_inner(
    data_dir: &Path,
    store: &Store,
    profile: &Profile,
    selected_ids: &[String],
    force_close_client: bool,
    job_id: &str,
    expected_target_provider: &str,
    migrate_sources: bool,
) -> AppResult<ReplicationResult> {
    let home = profile.home_path();
    let shutdown = process::ensure_stopped(&home, force_close_client)?;
    let mut restart_guard = ClientRestartGuard {
        app_path: profile
            .app_path
            .clone()
            .or_else(|| shutdown.executable.clone()),
        home: home.clone(),
        active: shutdown.closed,
    };
    fs::create_dir_all(data_dir.join("locks"))?;
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(
            data_dir
                .join("locks")
                .join(format!("{}.provider.lock", profile.id)),
        )?;
    lock.try_lock_exclusive().map_err(|_| {
        AppError::Message("another Provider replication is using this CODEX_HOME".into())
    })?;

    assert_current_provider(profile, expected_target_provider)?;
    {
        let mut client = AppServerClient::start(&home, profile.app_path.as_deref())?;
        cross_check_app_server_provider(&mut client, expected_target_provider)?;
        client.thread_list_all(&interactive_filters(None))?;
    }
    let execution_preview = preview(store, profile, selected_ids)?;
    if execution_preview.target_provider_id != expected_target_provider {
        return Err(AppError::Message(
            "current Provider changed after preview; rescan before retrying".into(),
        ));
    }
    let snapshots = scan_profile(profile)?
        .into_iter()
        .map(|snapshot| (snapshot.thread_id.clone(), snapshot))
        .collect::<HashMap<_, _>>();
    let mut created = Vec::new();
    let mut skipped = Vec::new();
    let mut failed = Vec::new();
    let mut warnings = Vec::new();

    for item in &execution_preview.items {
        if item.action != ReplicationAction::CreateReplica {
            skipped.push(ReplicaResultItem {
                source_thread_id: item.thread_id.clone(),
                replica_thread_id: item.replica_thread_id.clone(),
                title: item.title.clone(),
                status: action_status(&item.action).into(),
                message: item.reason.clone(),
            });
            continue;
        }
        assert_current_provider(profile, expected_target_provider)?;
        let Some(source) = snapshots.get(&item.thread_id) else {
            failed.push(failed_item(item, None, "来源会话在执行前消失"));
            continue;
        };
        if source.raw_sha256 != item.source_sha256
            || hash_file_raw(&source.rollout_path)? != item.source_sha256
        {
            failed.push(failed_item(item, None, "来源会话在执行前发生变化"));
            continue;
        }
        match replicate_one(store, profile, source, expected_target_provider) {
            Ok((mut result, warning, mapping)) => {
                if migrate_sources {
                    match finalize_migration(
                        store,
                        profile,
                        source,
                        expected_target_provider,
                        &mapping,
                    ) {
                        Ok(()) => {
                            result.status = "migrated".into();
                            result.message = "已迁移到当前供应商并永久删除来源会话".into();
                            created.push(result);
                        }
                        Err(message) => failed.push(ReplicaResultItem {
                            source_thread_id: result.source_thread_id,
                            replica_thread_id: result.replica_thread_id,
                            title: result.title,
                            status: "failed".into(),
                            message,
                        }),
                    }
                } else {
                    created.push(result);
                }
                if let Some(warning) = warning {
                    warnings.push(warning);
                }
            }
            Err(result) => failed.push(result),
        }
    }

    FileExt::unlock(&lock)?;
    let client_restarted = match restart_guard.finish() {
        Ok(value) => value,
        Err(error) => {
            warnings.push(format!("客户端重启失败：{error}"));
            false
        }
    };
    Ok(ReplicationResult {
        job_id: job_id.into(),
        target_provider_id: expected_target_provider.into(),
        created,
        skipped,
        failed,
        client_restarted,
        warning: (!warnings.is_empty()).then(|| warnings.join("；")),
    })
}

fn replicate_one(
    store: &Store,
    profile: &Profile,
    source: &ThreadSnapshot,
    target_provider: &str,
) -> Result<(ReplicaResultItem, Option<String>, ReplicaMapping), ReplicaResultItem> {
    let mut fork_client =
        match AppServerClient::start(&profile.home_path(), profile.app_path.as_deref()) {
            Ok(client) => client,
            Err(error) => return Err(snapshot_failed_item(source, None, &error.to_string())),
        };
    if let Err(error) = fork_client.thread_read(&source.thread_id, true) {
        return Err(snapshot_failed_item(source, None, &error.to_string()));
    }
    let replica_thread_id = match fork_client.thread_fork(&source.thread_id) {
        Ok(thread_id) if thread_id != source.thread_id => thread_id,
        Ok(_) => {
            return Err(snapshot_failed_item(
                source,
                None,
                "thread/fork returned the source Thread ID",
            ))
        }
        Err(error) => return Err(snapshot_failed_item(source, None, &error.to_string())),
    };
    let prepared = (|| {
        let replica = wait_for_snapshot(profile, &replica_thread_id)?;
        validate_replica_path(&profile.home_path(), &replica.rollout_path)?;
        if replica.provider_id.as_deref() != Some(target_provider) {
            rewrite_replica_provider(
                &profile.home_path(),
                &replica.rollout_path,
                &replica_thread_id,
                target_provider,
            )?;
        }
        let title_warning = if source.title != source.thread_id {
            fork_client
                .thread_name_set(&replica_thread_id, &source.title)
                .err()
                .map(|error| format!("{}：标题未能恢复（{error}）", source.title))
        } else {
            None
        };
        Ok::<_, AppError>(title_warning)
    })();
    let title_warning = match prepared {
        Ok(warning) => warning,
        Err(error) => {
            let status = cleanup_with_client(&mut fork_client, profile, &replica_thread_id);
            if status.is_err() {
                save_orphan_mapping(store, profile, source, target_provider, &replica_thread_id);
            }
            let message = cleanup_message(&error.to_string(), status);
            return Err(snapshot_failed_item(
                source,
                Some(replica_thread_id),
                &message,
            ));
        }
    };
    drop(fork_client);

    let verified = verify_replica(profile, source, target_provider, &replica_thread_id);
    let replica = match verified {
        Ok(replica) => replica,
        Err(error) => {
            let cleanup = cleanup_replica(profile, &replica_thread_id);
            if cleanup.is_err() {
                save_orphan_mapping(store, profile, source, target_provider, &replica_thread_id);
            }
            let message = cleanup_message(&error.to_string(), cleanup);
            return Err(snapshot_failed_item(
                source,
                Some(replica_thread_id),
                &message,
            ));
        }
    };
    let mapping = ReplicaMapping {
        id: Uuid::new_v4().to_string(),
        profile_id: profile.id.clone(),
        source_thread_id: source.thread_id.clone(),
        source_provider_id: source
            .provider_id
            .clone()
            .unwrap_or_else(|| "unknown".into()),
        target_provider_id: target_provider.into(),
        replica_thread_id: replica_thread_id.clone(),
        source_sha256: source.raw_sha256.clone(),
        replica_sha256: replica.raw_sha256,
        status: "verified".into(),
        created_at: Utc::now().to_rfc3339(),
        verified_at: Some(Utc::now().to_rfc3339()),
        deleted_at: None,
    };
    if let Err(error) = store.save_replica(&mapping) {
        let cleanup = cleanup_replica(profile, &replica_thread_id);
        let message = cleanup_message(&format!("映射记录失败：{error}"), cleanup);
        return Err(snapshot_failed_item(
            source,
            Some(replica_thread_id),
            &message,
        ));
    }
    let persisted_mapping = match store.list_replicas(&profile.id).and_then(|mappings| {
        mappings
            .into_iter()
            .find(|candidate| {
                candidate.source_thread_id == mapping.source_thread_id
                    && candidate.target_provider_id == mapping.target_provider_id
                    && candidate.replica_thread_id == mapping.replica_thread_id
                    && candidate.deleted_at.is_none()
            })
            .ok_or_else(|| AppError::Message("刚创建的迁移关系无法读取".into()))
    }) {
        Ok(mapping) => mapping,
        Err(error) => {
            let cleanup = cleanup_replica(profile, &replica_thread_id);
            let message = cleanup_message(&format!("映射校验失败：{error}"), cleanup);
            return Err(snapshot_failed_item(
                source,
                Some(replica_thread_id),
                &message,
            ));
        }
    };
    Ok((
        ReplicaResultItem {
            source_thread_id: source.thread_id.clone(),
            replica_thread_id: Some(replica_thread_id),
            title: source.title.clone(),
            status: "verified".into(),
            message: "已创建新会话并通过当前 Provider 验证".into(),
        },
        title_warning,
        persisted_mapping,
    ))
}

fn finalize_migration(
    store: &Store,
    profile: &Profile,
    source: &ThreadSnapshot,
    target_provider: &str,
    mapping: &ReplicaMapping,
) -> Result<(), String> {
    store
        .mark_replica_deleted(&mapping.id)
        .map_err(|error| format!("迁移关系解除失败，未删除来源会话：{error}"))?;
    match cleanup_replica(profile, &source.thread_id) {
        Ok(()) => Ok(()),
        Err(delete_error) => {
            let rollback = cleanup_replica(profile, &mapping.replica_thread_id);
            match rollback {
                Ok(()) => Err(format!(
                    "来源会话删除失败：{delete_error}；已删除刚创建的目标会话，来源会话保持不变"
                )),
                Err(rollback_error) => {
                    save_orphan_mapping(
                        store,
                        profile,
                        source,
                        target_provider,
                        &mapping.replica_thread_id,
                    );
                    Err(format!(
                        "来源会话删除失败：{delete_error}；目标会话回滚失败，需清理异常副本：{rollback_error}"
                    ))
                }
            }
        }
    }
}

fn verify_replica(
    profile: &Profile,
    source: &ThreadSnapshot,
    target_provider: &str,
    replica_thread_id: &str,
) -> AppResult<ThreadSnapshot> {
    let mut client = AppServerClient::start(&profile.home_path(), profile.app_path.as_deref())?;
    cross_check_app_server_provider(&mut client, target_provider)?;
    let listed = client.thread_list_all(&interactive_filters(Some(target_provider)))?;
    let list_entry = listed
        .iter()
        .find(|thread| thread_id(thread) == Some(replica_thread_id))
        .ok_or_else(|| {
            AppError::InvalidSession("replica is absent from the current Provider list".into())
        })?;
    if let Some(provider) = thread_provider(list_entry) {
        if provider != target_provider {
            return Err(AppError::InvalidSession(format!(
                "replica Provider is {provider}, expected {target_provider}"
            )));
        }
    }
    let read = client.thread_read(replica_thread_id, true)?;
    if thread_id(&read) != Some(replica_thread_id)
        && read.pointer("/thread/id").and_then(Value::as_str) != Some(replica_thread_id)
    {
        return Err(AppError::InvalidSession(
            "thread/read returned a different replica".into(),
        ));
    }
    let replica = wait_for_snapshot(profile, replica_thread_id)?;
    if replica.provider_id.as_deref() != Some(target_provider) {
        return Err(AppError::InvalidSession(
            "replica rollout does not belong to the current Provider".into(),
        ));
    }
    if replica.cwd != source.cwd {
        return Err(AppError::InvalidSession(
            "replica project path does not match the source".into(),
        ));
    }
    if hash_file_raw(&source.rollout_path)? != source.raw_sha256 {
        return Err(AppError::InvalidSession(
            "source rollout changed during replication".into(),
        ));
    }
    Ok(replica)
}

fn cleanup_replica(profile: &Profile, replica_thread_id: &str) -> AppResult<()> {
    let mut client = AppServerClient::start(&profile.home_path(), profile.app_path.as_deref())?;
    cleanup_with_client(&mut client, profile, replica_thread_id)
}

fn cleanup_with_client(
    client: &mut AppServerClient,
    profile: &Profile,
    replica_thread_id: &str,
) -> AppResult<()> {
    let delete_error = client.thread_delete(replica_thread_id).err();
    match find_snapshot_by_thread_id(profile, replica_thread_id) {
        Ok(None) => Ok(()),
        Ok(Some(_)) => Err(delete_error.unwrap_or_else(|| {
            AppError::Message(
                "thread/delete returned success but the replica rollout still exists".into(),
            )
        })),
        Err(scan_error) => Err(delete_error.unwrap_or(scan_error)),
    }
}

pub fn history(store: &Store, profile_id: &str) -> AppResult<Vec<ReplicaMapping>> {
    store.list_replicas(profile_id)
}

pub fn cleanup_orphans(
    store: &Store,
    profile: &Profile,
    force_close_client: bool,
) -> AppResult<Vec<ReplicaMapping>> {
    let home = profile.home_path();
    let shutdown = process::ensure_stopped(&home, force_close_client)?;
    let mut restart_guard = ClientRestartGuard {
        app_path: profile
            .app_path
            .clone()
            .or_else(|| shutdown.executable.clone()),
        home,
        active: shutdown.closed,
    };
    let orphans = store
        .list_replicas(&profile.id)?
        .into_iter()
        .filter(|mapping| mapping.status == "orphaned" && mapping.deleted_at.is_none())
        .collect::<Vec<_>>();
    for mapping in &orphans {
        cleanup_replica(profile, &mapping.replica_thread_id)?;
        store.mark_replica_deleted(&mapping.id)?;
    }
    let _ = restart_guard.finish();
    store.list_replicas(&profile.id)
}

fn current_provider(profile: &Profile) -> AppResult<String> {
    let provider = profiles::read_provider(&profile.home_path(), &profile.provider_id);
    if provider.trim().is_empty() {
        return Err(AppError::Message(
            "current Provider is missing from config.toml".into(),
        ));
    }
    Ok(provider)
}

fn assert_current_provider(profile: &Profile, expected: &str) -> AppResult<()> {
    let actual = current_provider(profile)?;
    if actual != expected {
        return Err(AppError::Message(format!(
            "current Provider changed from {expected} to {actual}; rescan before retrying"
        )));
    }
    Ok(())
}

fn cross_check_app_server_provider(client: &mut AppServerClient, expected: &str) -> AppResult<()> {
    let config = client.config_read()?;
    if let Some(actual) = config_provider(&config) {
        if actual != expected {
            return Err(AppError::Message(format!(
                "config.toml Provider is {expected}, but App Server reports {actual}"
            )));
        }
    }
    Ok(())
}

fn config_provider(value: &Value) -> Option<&str> {
    [
        "/config/modelProvider",
        "/config/model_provider",
        "/modelProvider",
        "/model_provider",
    ]
    .iter()
    .find_map(|pointer| value.pointer(pointer).and_then(Value::as_str))
    .filter(|value| !value.trim().is_empty())
}

fn interactive_filters(provider: Option<&str>) -> ThreadListFilters {
    ThreadListFilters {
        model_providers: provider.map(|value| vec![value.to_string()]),
        source_kinds: Some(vec!["cli".into(), "vscode".into()]),
        archived: false,
    }
}

fn eligibility(
    snapshot: &ThreadSnapshot,
    current_provider: &str,
    replica_mapping: Option<&ReplicaMapping>,
    mapping: Option<&ReplicaMapping>,
) -> (ReplicationEligibility, String, Option<String>) {
    if snapshot.archived {
        return (
            ReplicationEligibility::Archived,
            "会话已归档".into(),
            replica_mapping.map(|mapping| mapping.replica_thread_id.clone()),
        );
    }
    if !is_interactive(&snapshot.source_kind) {
        return (
            ReplicationEligibility::InternalThread,
            "仅支持 cli/vscode 主会话".into(),
            replica_mapping.map(|mapping| mapping.replica_thread_id.clone()),
        );
    }
    if let Some(replica_mapping) = replica_mapping {
        if replica_mapping.status == "verified" {
            let (eligibility, reason) = if replica_mapping.replica_sha256 == snapshot.raw_sha256 {
                (
                    ReplicationEligibility::Replica,
                    "这是工具创建并验证过的独立会话副本".into(),
                )
            } else {
                (
                    ReplicationEligibility::ReplicaUpdated,
                    "被复制的副本又产生了新内容".into(),
                )
            };
            return (eligibility, reason, Some(snapshot.thread_id.clone()));
        }
        return (
            ReplicationEligibility::InvalidRollout,
            format!("这是状态为 {} 的复制副本，需先清理", replica_mapping.status),
            Some(snapshot.thread_id.clone()),
        );
    }
    let (action, reason, replica_id) = plan_action(snapshot, current_provider, false, mapping);
    let eligibility = match action {
        ReplicationAction::CreateReplica => ReplicationEligibility::Eligible,
        ReplicationAction::SkipAlreadyReplicated => ReplicationEligibility::AlreadyReplicated,
        ReplicationAction::SourceUpdated => ReplicationEligibility::SourceUpdated,
        ReplicationAction::SkipCurrentProvider => ReplicationEligibility::CurrentProvider,
        ReplicationAction::SkipArchived => ReplicationEligibility::Archived,
        ReplicationAction::SkipInternal => ReplicationEligibility::InternalThread,
        ReplicationAction::Invalid => ReplicationEligibility::InvalidRollout,
    };
    (eligibility, reason, replica_id)
}

fn plan_action(
    snapshot: &ThreadSnapshot,
    current_provider: &str,
    is_replica: bool,
    mapping: Option<&ReplicaMapping>,
) -> (ReplicationAction, String, Option<String>) {
    if snapshot.archived {
        return (ReplicationAction::SkipArchived, "会话已归档".into(), None);
    }
    if !is_interactive(&snapshot.source_kind) {
        return (
            ReplicationAction::SkipInternal,
            "仅支持 cli/vscode 主会话".into(),
            None,
        );
    }
    if is_replica {
        return (
            ReplicationAction::Invalid,
            "这是工具已创建的会话副本".into(),
            None,
        );
    }
    let Some(provider) = snapshot.provider_id.as_deref() else {
        return (
            ReplicationAction::Invalid,
            "rollout 缺少 model_provider".into(),
            None,
        );
    };
    if provider == current_provider {
        return (
            ReplicationAction::SkipCurrentProvider,
            "会话已经属于当前 Provider".into(),
            None,
        );
    }
    if let Some(mapping) = mapping {
        if mapping.status == "verified" && mapping.source_sha256 == snapshot.raw_sha256 {
            return (
                ReplicationAction::SkipAlreadyReplicated,
                "已存在经过验证的独立副本".into(),
                Some(mapping.replica_thread_id.clone()),
            );
        }
        if mapping.status == "verified" {
            return (
                ReplicationAction::SourceUpdated,
                "来源会话已更新；首版不会覆盖已有副本".into(),
                Some(mapping.replica_thread_id.clone()),
            );
        }
        return (
            ReplicationAction::Invalid,
            "存在尚未清理的复制副本".into(),
            Some(mapping.replica_thread_id.clone()),
        );
    }
    (
        ReplicationAction::CreateReplica,
        format!("将创建新的 Thread ID 并归入 {current_provider}"),
        None,
    )
}

fn is_interactive(source: &ThreadSourceKind) -> bool {
    matches!(source, ThreadSourceKind::Cli | ThreadSourceKind::Vscode)
}

fn wait_for_snapshot(profile: &Profile, thread_id: &str) -> AppResult<ThreadSnapshot> {
    for _ in 0..30 {
        if let Some(snapshot) = find_snapshot_by_thread_id(profile, thread_id)? {
            return Ok(snapshot);
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err(AppError::InvalidSession(format!(
        "forked rollout was not found for {thread_id}"
    )))
}

fn validate_replica_path(home: &Path, path: &Path) -> AppResult<()> {
    let canonical_home = fs::canonicalize(home)?;
    let canonical_path = fs::canonicalize(path)?;
    let sessions = canonical_home.join("sessions");
    if !canonical_path.starts_with(&sessions) {
        return Err(AppError::InvalidPath(format!(
            "replica is outside the active sessions directory: {}",
            path.display()
        )));
    }
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(AppError::InvalidPath(format!(
            "replica rollout is a symbolic link: {}",
            path.display()
        )));
    }
    Ok(())
}

fn rewrite_replica_provider(
    home: &Path,
    path: &Path,
    expected_thread_id: &str,
    provider_id: &str,
) -> AppResult<()> {
    validate_replica_path(home, path)?;
    let content = fs::read_to_string(path)?;
    let mut output = Vec::new();
    let mut first_meta_provider = None::<String>;
    let mut rewrote = false;
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut value: Value = serde_json::from_str(line)?;
        if value.get("type").and_then(Value::as_str) == Some("session_meta") {
            let payload = value
                .get_mut("payload")
                .and_then(Value::as_object_mut)
                .ok_or_else(|| {
                    AppError::InvalidSession("session_meta payload is invalid".into())
                })?;
            let thread_id = payload
                .get("id")
                .or_else(|| payload.get("session_id"))
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::InvalidSession("session_meta has no thread id".into()))?;
            if thread_id != expected_thread_id {
                return Err(AppError::InvalidSession(
                    "replica session_meta contains a conflicting Thread ID".into(),
                ));
            }
            let existing_provider = payload
                .get("model_provider")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if let Some(first) = &first_meta_provider {
                if first != &existing_provider {
                    return Err(AppError::InvalidSession(
                        "replica contains conflicting session_meta Providers".into(),
                    ));
                }
            } else {
                first_meta_provider = Some(existing_provider);
            }
            if !rewrote {
                payload.insert(
                    "model_provider".into(),
                    Value::String(provider_id.to_string()),
                );
                rewrote = true;
            }
        }
        output.push(serde_json::to_string(&value)?);
    }
    if !rewrote {
        return Err(AppError::InvalidSession(
            "replica session_meta was not found".into(),
        ));
    }
    atomic_write(path, format!("{}\n", output.join("\n")).as_bytes())
}

fn rewrite_existing_session(
    home: &Path,
    source_path: &Path,
    target_path: &Path,
    target_thread_id: &str,
    target_provider: &str,
) -> AppResult<()> {
    validate_replica_path(home, source_path)?;
    validate_replica_path(home, target_path)?;
    if source_path == target_path {
        return Err(AppError::InvalidSession(
            "source and target rollout paths are identical".into(),
        ));
    }

    let content = fs::read_to_string(source_path)?;
    let mut output = Vec::new();
    let mut source_thread_id = None::<String>;
    let mut meta_count = 0usize;
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut value: Value = serde_json::from_str(line)?;
        if value.get("type").and_then(Value::as_str) == Some("session_meta") {
            let payload = value
                .get_mut("payload")
                .and_then(Value::as_object_mut)
                .ok_or_else(|| {
                    AppError::InvalidSession("session_meta payload is invalid".into())
                })?;
            let current_thread_id = payload
                .get("id")
                .or_else(|| payload.get("session_id"))
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| AppError::InvalidSession("session_meta has no thread id".into()))?
                .to_string();
            if source_thread_id
                .as_ref()
                .is_some_and(|expected| expected != &current_thread_id)
            {
                return Err(AppError::InvalidSession(
                    "source rollout contains conflicting Thread IDs".into(),
                ));
            }
            source_thread_id = Some(current_thread_id);
            if payload.contains_key("id") {
                payload.insert("id".into(), Value::String(target_thread_id.into()));
            } else {
                payload.insert("session_id".into(), Value::String(target_thread_id.into()));
            }
            payload.insert(
                "model_provider".into(),
                Value::String(target_provider.into()),
            );
            meta_count += 1;
        }
        output.push(serde_json::to_string(&value)?);
    }
    if meta_count == 0 {
        return Err(AppError::InvalidSession(
            "source rollout has no session_meta".into(),
        ));
    }
    atomic_write(target_path, format!("{}\n", output.join("\n")).as_bytes())
}

fn thread_id(value: &Value) -> Option<&str> {
    value
        .get("id")
        .or_else(|| value.get("threadId"))
        .and_then(Value::as_str)
}

fn thread_provider(value: &Value) -> Option<&str> {
    value
        .get("modelProvider")
        .or_else(|| value.get("model_provider"))
        .and_then(Value::as_str)
}

fn action_status(action: &ReplicationAction) -> &'static str {
    match action {
        ReplicationAction::SkipAlreadyReplicated => "already_replicated",
        ReplicationAction::SourceUpdated => "source_updated",
        ReplicationAction::SkipCurrentProvider => "current_provider",
        ReplicationAction::SkipArchived => "archived",
        ReplicationAction::SkipInternal => "internal",
        ReplicationAction::Invalid => "invalid",
        ReplicationAction::CreateReplica => "planned",
    }
}

fn failed_item(
    item: &ReplicationPlanItem,
    replica_thread_id: Option<String>,
    message: &str,
) -> ReplicaResultItem {
    ReplicaResultItem {
        source_thread_id: item.thread_id.clone(),
        replica_thread_id,
        title: item.title.clone(),
        status: "failed".into(),
        message: message.into(),
    }
}

fn update_sync_failed_item(item: &UpdateSyncPlanItem, message: &str) -> ReplicaResultItem {
    ReplicaResultItem {
        source_thread_id: item.source_thread_id.clone(),
        replica_thread_id: Some(item.replica_thread_id.clone()),
        title: item.title.clone(),
        status: "failed".into(),
        message: message.into(),
    }
}

fn update_sync_skipped_item(item: &UpdateSyncPlanItem) -> ReplicaResultItem {
    ReplicaResultItem {
        source_thread_id: item.source_thread_id.clone(),
        replica_thread_id: Some(item.replica_thread_id.clone()),
        title: item.title.clone(),
        status: match item.action {
            UpdateSyncAction::Conflict => "conflict",
            _ => "invalid",
        }
        .into(),
        message: item.reason.clone(),
    }
}

fn snapshot_failed_item(
    source: &ThreadSnapshot,
    replica_thread_id: Option<String>,
    message: &str,
) -> ReplicaResultItem {
    ReplicaResultItem {
        source_thread_id: source.thread_id.clone(),
        replica_thread_id,
        title: source.title.clone(),
        status: if message.contains("未清理") {
            "orphaned"
        } else {
            "failed_cleaned"
        }
        .into(),
        message: message.into(),
    }
}

fn cleanup_message(error: &str, cleanup: AppResult<()>) -> String {
    match cleanup {
        Ok(()) => format!("{error}；本次未验证副本已删除"),
        Err(cleanup_error) => format!("{error}；副本未清理：{cleanup_error}"),
    }
}

fn save_orphan_mapping(
    store: &Store,
    profile: &Profile,
    source: &ThreadSnapshot,
    target_provider: &str,
    replica_thread_id: &str,
) {
    let replica_sha256 = find_snapshot_by_thread_id(profile, replica_thread_id)
        .ok()
        .flatten()
        .map(|snapshot| snapshot.raw_sha256)
        .unwrap_or_default();
    let _ = store.save_replica(&ReplicaMapping {
        id: Uuid::new_v4().to_string(),
        profile_id: profile.id.clone(),
        source_thread_id: source.thread_id.clone(),
        source_provider_id: source
            .provider_id
            .clone()
            .unwrap_or_else(|| "unknown".into()),
        target_provider_id: target_provider.into(),
        replica_thread_id: replica_thread_id.into(),
        source_sha256: source.raw_sha256.clone(),
        replica_sha256,
        status: "orphaned".into(),
        created_at: Utc::now().to_rfc3339(),
        verified_at: None,
        deleted_at: None,
    });
}

fn atomic_write(path: &Path, content: &[u8]) -> AppResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::InvalidPath(format!("{} has no parent", path.display())))?;
    let temp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        Uuid::new_v4()
    ));
    let mut file = File::create(&temp)?;
    file.write_all(content)?;
    file.sync_all()?;
    replace_file(&temp, path)
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

    fn profile(root: &Path, provider: &str) -> Profile {
        Profile {
            id: "profile".into(),
            name: "Profile".into(),
            kind: ProfileKind::CustomApi,
            mode: ProfileMode::External,
            codex_home: root.to_string_lossy().to_string(),
            provider_id: provider.into(),
            app_path: None,
            discovery_source: "test".into(),
            providers: Vec::new(),
            config_profiles: Vec::new(),
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        }
    }

    fn write_rollout(root: &Path, id: &str, provider: &str, source: &str) -> PathBuf {
        let directory = root.join("sessions/2026/08/15");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join(format!("rollout-{id}.jsonl"));
        fs::write(
            &path,
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"C:/work\",\"model_provider\":\"{provider}\",\"source\":\"{source}\",\"thread_source\":\"user\"}}}}\n{{\"type\":\"message\",\"payload\":{{\"text\":\"hello\"}}}}\n"
            ),
        )
        .unwrap();
        path
    }

    fn write_internal_rollout(root: &Path, id: &str, provider: &str) -> PathBuf {
        let directory = root.join("sessions/2026/08/15");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join(format!("rollout-{id}.jsonl"));
        fs::write(
            &path,
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"C:/work\",\"model_provider\":\"{provider}\",\"source\":{{\"subagent\":{{\"other\":\"guardian\"}}}},\"thread_source\":\"subagent\"}}}}\n{{\"type\":\"message\",\"payload\":{{\"text\":\"hello\"}}}}\n"
            ),
        )
        .unwrap();
        path
    }

    #[test]
    fn puts_the_official_provider_first_without_changing_provider_ids() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("config.toml"),
            "model_provider = \"custom\"\n",
        )
        .unwrap();
        write_rollout(temp.path(), "one", "OpenAI-API", "vscode");
        write_rollout(temp.path(), "two", "custom", "cli");
        write_rollout(temp.path(), "three", "openai", "vscode");
        let data = tempfile::tempdir().unwrap();
        let store = Store::open(data.path()).unwrap();
        let buckets = provider_list(&store, &profile(temp.path(), "custom")).unwrap();
        assert_eq!(buckets[0].provider_id, "openai");
        assert_eq!(buckets[1].provider_id, "custom");
        assert!(buckets
            .iter()
            .any(|bucket| bucket.provider_id == "OpenAI-API"));
    }

    #[test]
    fn archive_cleanup_preview_only_lists_the_selected_provider() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("config.toml"),
            "model_provider = \"custom\"\n",
        )
        .unwrap();
        let openai = write_rollout(temp.path(), "archived-openai", "openai", "vscode");
        let custom = write_rollout(temp.path(), "archived-custom", "custom", "cli");
        let archived = temp.path().join("archived_sessions/2026/08/15");
        fs::create_dir_all(&archived).unwrap();
        fs::rename(
            &openai,
            archived.join(openai.file_name().expect("openai file name")),
        )
        .unwrap();
        fs::rename(
            &custom,
            archived.join(custom.file_name().expect("custom file name")),
        )
        .unwrap();
        let data = tempfile::tempdir().unwrap();
        let store = Store::open(data.path()).unwrap();

        let preview =
            preview_archive_cleanup(&store, &profile(temp.path(), "custom"), "openai").unwrap();

        assert_eq!(preview.provider_id, "openai");
        assert_eq!(preview.total_count, 1);
        assert_eq!(preview.items[0].thread_id, "archived-openai");
        assert_eq!(preview.items[0].provider_id, "openai");
        assert!(preview.total_bytes > 0);
    }

    #[test]
    fn child_cleanup_preview_only_lists_active_internal_sessions_for_the_provider() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("config.toml"),
            "model_provider = \"custom\"\n",
        )
        .unwrap();
        write_rollout(temp.path(), "main-openai", "openai", "vscode");
        write_internal_rollout(temp.path(), "child-openai", "openai");
        write_internal_rollout(temp.path(), "child-custom", "custom");
        let archived_child = write_internal_rollout(temp.path(), "archived-child", "openai");
        let archived = temp.path().join("archived_sessions/2026/08/15");
        fs::create_dir_all(&archived).unwrap();
        fs::rename(
            &archived_child,
            archived.join(archived_child.file_name().expect("child file name")),
        )
        .unwrap();
        let data = tempfile::tempdir().unwrap();
        let store = Store::open(data.path()).unwrap();

        let preview =
            preview_invalid_child_cleanup(&store, &profile(temp.path(), "custom"), "openai")
                .unwrap();

        assert_eq!(preview.provider_id, "openai");
        assert_eq!(preview.total_count, 1);
        assert_eq!(preview.items[0].thread_id, "child-openai");
        assert_eq!(preview.items[0].source_kind, ThreadSourceKind::Internal);
    }

    #[test]
    fn archive_cleanup_treats_a_missing_rollout_as_deleted_after_server_error() {
        let item = ArchiveCleanupItem {
            thread_id: "missing-thread".into(),
            title: "已不存在的归档会话".into(),
            provider_id: "custom".into(),
            source_kind: ThreadSourceKind::Cli,
            updated_at: None,
            size_bytes: 42,
        };
        let remaining = HashSet::new();
        let mut deleted = Vec::new();
        let mut failed = Vec::new();

        classify_cleanup_attempt(
            item,
            Some("app-server thread/delete error: no rollout found".into()),
            &remaining,
            CleanupScope::Archived,
            &mut deleted,
            &mut failed,
        );

        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0].thread_id, "missing-thread");
        assert!(deleted[0].message.contains("已按删除完成处理"));
        assert!(failed.is_empty());
    }

    #[test]
    fn preview_marks_current_internal_and_eligible_sessions() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("config.toml"),
            "model_provider = \"custom\"\n",
        )
        .unwrap();
        write_rollout(temp.path(), "source", "OpenAI-API", "vscode");
        write_rollout(temp.path(), "current", "custom", "cli");
        write_rollout(temp.path(), "unknown", "OpenAI-API", "other");
        let data = tempfile::tempdir().unwrap();
        let store = Store::open(data.path()).unwrap();
        let preview = preview(
            &store,
            &profile(temp.path(), "custom"),
            &["source".into(), "current".into(), "unknown".into()],
        )
        .unwrap();
        assert_eq!(preview.create_count, 1);
        assert!(preview
            .items
            .iter()
            .any(|item| item.action == ReplicationAction::SkipCurrentProvider));
        assert!(preview
            .items
            .iter()
            .any(|item| item.action == ReplicationAction::SkipInternal));
    }

    #[test]
    fn rewrites_only_the_replica_and_preserves_source_hash() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = write_rollout(temp.path(), "source", "OpenAI-API", "vscode");
        let replica_path = write_rollout(temp.path(), "replica", "OpenAI-API", "vscode");
        let source_hash = hash_file_raw(&source_path).unwrap();
        rewrite_replica_provider(temp.path(), &replica_path, "replica", "custom").unwrap();
        assert_eq!(hash_file_raw(&source_path).unwrap(), source_hash);
        let replica = find_snapshot_by_thread_id(&profile(temp.path(), "custom"), "replica")
            .unwrap()
            .unwrap();
        assert_eq!(replica.provider_id.as_deref(), Some("custom"));
    }

    #[test]
    fn updates_existing_pair_without_creating_another_thread() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = write_rollout(temp.path(), "source", "OpenAI-API", "vscode");
        let replica_path = write_rollout(temp.path(), "replica", "custom", "vscode");
        let mut source_content = fs::read_to_string(&source_path).unwrap();
        source_content.push_str("{\"type\":\"message\",\"payload\":{\"text\":\"later\"}}\n");
        fs::write(&source_path, source_content).unwrap();

        let source = find_snapshot_by_thread_id(&profile(temp.path(), "custom"), "source")
            .unwrap()
            .unwrap();
        rewrite_existing_session(
            temp.path(),
            &source_path,
            &replica_path,
            "replica",
            "custom",
        )
        .unwrap();
        let replica = find_snapshot_by_thread_id(&profile(temp.path(), "custom"), "replica")
            .unwrap()
            .unwrap();

        assert_eq!(replica.thread_id, "replica");
        assert_eq!(replica.provider_id.as_deref(), Some("custom"));
        assert_eq!(replica.content_sha256, source.content_sha256);
        assert_eq!(
            scan_profile(&profile(temp.path(), "custom")).unwrap().len(),
            2
        );
    }

    #[test]
    fn unchanged_archived_pair_is_not_reported_as_a_sync_failure() {
        let temp = tempfile::tempdir().unwrap();
        write_rollout(temp.path(), "source", "openai", "vscode");
        write_rollout(temp.path(), "replica", "custom", "vscode");
        let mut source = find_snapshot_by_thread_id(&profile(temp.path(), "custom"), "source")
            .unwrap()
            .unwrap();
        let mut replica = find_snapshot_by_thread_id(&profile(temp.path(), "custom"), "replica")
            .unwrap()
            .unwrap();
        replica.archived = true;
        let mapping = ReplicaMapping {
            id: "mapping".into(),
            profile_id: "profile".into(),
            source_thread_id: source.thread_id.clone(),
            source_provider_id: "openai".into(),
            target_provider_id: "custom".into(),
            replica_thread_id: replica.thread_id.clone(),
            source_sha256: source.raw_sha256.clone(),
            replica_sha256: replica.raw_sha256.clone(),
            status: "verified".into(),
            created_at: Utc::now().to_rfc3339(),
            verified_at: Some(Utc::now().to_rfc3339()),
            deleted_at: None,
        };
        let mut snapshots = HashMap::from([
            (source.thread_id.clone(), source.clone()),
            (replica.thread_id.clone(), replica.clone()),
        ]);

        assert!(update_sync_plan_items(&[mapping.clone()], &snapshots).is_empty());

        source.raw_sha256 = "source-updated".into();
        snapshots.insert(source.thread_id.clone(), source);
        assert!(update_sync_plan_items(&[mapping], &snapshots).is_empty());
    }

    #[test]
    fn archiving_either_side_permanently_disconnects_the_replica_pair() {
        let temp = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let store = Store::open(data.path()).unwrap();
        fs::write(
            temp.path().join("config.toml"),
            "model_provider = \"custom\"\n",
        )
        .unwrap();
        write_rollout(temp.path(), "source", "openai", "vscode");
        write_rollout(temp.path(), "replica", "custom", "vscode");
        let source = find_snapshot_by_thread_id(&profile(temp.path(), "custom"), "source")
            .unwrap()
            .unwrap();
        let mut replica = find_snapshot_by_thread_id(&profile(temp.path(), "custom"), "replica")
            .unwrap()
            .unwrap();
        replica.archived = true;
        let mapping = ReplicaMapping {
            id: "mapping".into(),
            profile_id: "profile".into(),
            source_thread_id: source.thread_id.clone(),
            source_provider_id: "openai".into(),
            target_provider_id: "custom".into(),
            replica_thread_id: replica.thread_id.clone(),
            source_sha256: source.raw_sha256.clone(),
            replica_sha256: replica.raw_sha256.clone(),
            status: "verified".into(),
            created_at: Utc::now().to_rfc3339(),
            verified_at: Some(Utc::now().to_rfc3339()),
            deleted_at: None,
        };
        store.save_replica(&mapping).unwrap();

        let mappings =
            reconcile_archived_mappings(&store, "profile", &[source.clone(), replica.clone()])
                .unwrap();
        assert_eq!(mappings[0].status, "deleted");
        assert!(mappings[0].deleted_at.is_some());

        let scan = provider_scan_from_snapshots(
            &profile(temp.path(), "custom"),
            &mappings,
            vec![source.clone(), replica.clone()],
        )
        .unwrap();
        let archived_workspace = scan.workspace(Some("custom"));
        let archived_replica = archived_workspace
            .provider_sessions
            .iter()
            .find(|session| session.thread_id == "replica")
            .unwrap();

        assert_eq!(
            archived_replica.eligibility,
            ReplicationEligibility::Archived
        );
        assert!(!archived_replica.is_replica);

        let source_workspace = scan.workspace(Some("openai"));
        let disconnected_source = source_workspace
            .provider_sessions
            .iter()
            .find(|session| session.thread_id == "source")
            .unwrap();
        assert_eq!(
            disconnected_source.eligibility,
            ReplicationEligibility::Eligible
        );

        replica.archived = false;
        let restored_workspace = provider_scan_from_snapshots(
            &profile(temp.path(), "custom"),
            &mappings,
            vec![source, replica],
        )
        .unwrap()
        .workspace(Some("custom"));
        let restored_replica = restored_workspace
            .provider_sessions
            .iter()
            .find(|session| session.thread_id == "replica")
            .unwrap();
        assert_eq!(
            restored_replica.eligibility,
            ReplicationEligibility::CurrentProvider
        );
        assert!(!restored_replica.is_replica);
    }

    #[test]
    fn verified_mapping_is_idempotent_and_detects_source_updates() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("config.toml"),
            "model_provider = \"custom\"\n",
        )
        .unwrap();
        let source_path = write_rollout(temp.path(), "source", "OpenAI-API", "vscode");
        let data = tempfile::tempdir().unwrap();
        let store = Store::open(data.path()).unwrap();
        let source = find_snapshot_by_thread_id(&profile(temp.path(), "custom"), "source")
            .unwrap()
            .unwrap();
        store
            .save_replica(&ReplicaMapping {
                id: "mapping".into(),
                profile_id: "profile".into(),
                source_thread_id: "source".into(),
                source_provider_id: "OpenAI-API".into(),
                target_provider_id: "custom".into(),
                replica_thread_id: "replica".into(),
                source_sha256: source.raw_sha256,
                replica_sha256: "replica-sha".into(),
                status: "verified".into(),
                created_at: Utc::now().to_rfc3339(),
                verified_at: Some(Utc::now().to_rfc3339()),
                deleted_at: None,
            })
            .unwrap();

        let first = preview(&store, &profile(temp.path(), "custom"), &["source".into()]).unwrap();
        assert_eq!(
            first.items[0].action,
            ReplicationAction::SkipAlreadyReplicated
        );

        let mut content = fs::read_to_string(&source_path).unwrap();
        content.push_str("{\"type\":\"message\",\"payload\":{\"text\":\"later\"}}\n");
        fs::write(&source_path, content).unwrap();
        let updated = preview(&store, &profile(temp.path(), "custom"), &["source".into()]).unwrap();
        assert_eq!(updated.items[0].action, ReplicationAction::SourceUpdated);
    }

    #[test]
    fn verified_replica_is_labeled_as_replica_in_provider_list() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("config.toml"),
            "model_provider = \"custom\"\n",
        )
        .unwrap();
        write_rollout(temp.path(), "source", "OpenAI-API", "vscode");
        let replica_path = write_rollout(temp.path(), "replica", "custom", "vscode");
        let data = tempfile::tempdir().unwrap();
        let store = Store::open(data.path()).unwrap();
        let source = find_snapshot_by_thread_id(&profile(temp.path(), "custom"), "source")
            .unwrap()
            .unwrap();
        let replica_snapshot =
            find_snapshot_by_thread_id(&profile(temp.path(), "custom"), "replica")
                .unwrap()
                .unwrap();
        store
            .save_replica(&ReplicaMapping {
                id: "mapping".into(),
                profile_id: "profile".into(),
                source_thread_id: "source".into(),
                source_provider_id: "OpenAI-API".into(),
                target_provider_id: "custom".into(),
                replica_thread_id: "replica".into(),
                source_sha256: source.raw_sha256,
                replica_sha256: replica_snapshot.raw_sha256,
                status: "verified".into(),
                created_at: Utc::now().to_rfc3339(),
                verified_at: Some(Utc::now().to_rfc3339()),
                deleted_at: None,
            })
            .unwrap();

        let records = provider_sessions(&store, &profile(temp.path(), "custom"), "custom")
            .expect("provider sessions");
        let replica = records
            .iter()
            .find(|record| record.thread_id == "replica")
            .expect("replica record");
        assert_eq!(replica.eligibility, ReplicationEligibility::Replica);
        assert!(replica.is_replica);
        assert_eq!(replica.replica_thread_id.as_deref(), Some("replica"));

        let mut content = fs::read_to_string(&replica_path).unwrap();
        content.push_str("{\"type\":\"message\",\"payload\":{\"text\":\"replica later\"}}\n");
        fs::write(&replica_path, content).unwrap();
        let records = provider_sessions(&store, &profile(temp.path(), "custom"), "custom")
            .expect("provider sessions after replica update");
        let updated_replica = records
            .iter()
            .find(|record| record.thread_id == "replica")
            .expect("updated replica record");
        assert_eq!(
            updated_replica.eligibility,
            ReplicationEligibility::ReplicaUpdated
        );
        assert_eq!(
            updated_replica.eligibility_reason,
            "被复制的副本又产生了新内容"
        );

        store
            .save_replica(&ReplicaMapping {
                id: "mapping".into(),
                profile_id: "profile".into(),
                source_thread_id: "source".into(),
                source_provider_id: "OpenAI-API".into(),
                target_provider_id: "custom".into(),
                replica_thread_id: "replica".into(),
                source_sha256: "source-sha".into(),
                replica_sha256: "replica-sha".into(),
                status: "orphaned".into(),
                created_at: Utc::now().to_rfc3339(),
                verified_at: None,
                deleted_at: None,
            })
            .unwrap();
        let records = provider_sessions(&store, &profile(temp.path(), "custom"), "custom")
            .expect("provider sessions");
        let orphaned = records
            .iter()
            .find(|record| record.thread_id == "replica")
            .expect("orphaned replica record");
        assert_eq!(orphaned.eligibility, ReplicationEligibility::InvalidRollout);
        assert!(orphaned.is_replica);
    }
}
