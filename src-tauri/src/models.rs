use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProfileKind {
    ChatGptAccount,
    CustomApi,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProfileMode {
    External,
    Managed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredProvider {
    pub id: String,
    pub source_file: String,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredConfigProfile {
    pub name: String,
    pub source_file: String,
    pub provider_id: Option<String>,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub kind: ProfileKind,
    pub mode: ProfileMode,
    pub codex_home: String,
    pub provider_id: String,
    pub app_path: Option<String>,
    pub discovery_source: String,
    pub providers: Vec<DiscoveredProvider>,
    pub config_profiles: Vec<DiscoveredConfigProfile>,
    pub created_at: String,
    pub updated_at: String,
}

impl Profile {
    pub fn home_path(&self) -> PathBuf {
        PathBuf::from(&self.codex_home)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInput {
    pub name: String,
    pub kind: ProfileKind,
    pub mode: ProfileMode,
    pub codex_home: Option<String>,
    pub provider_id: String,
    pub app_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppState {
    pub data_dir: String,
    pub platform: String,
    pub profiles: Vec<Profile>,
    pub app_server_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryReport {
    pub candidates_scanned: usize,
    pub discovered_count: usize,
    pub added_count: usize,
    pub refreshed_count: usize,
    pub profiles: Vec<Profile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionLocation {
    pub profile_id: String,
    pub profile_name: String,
    pub provider_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
    pub thread_id: String,
    pub title: String,
    pub cwd: Option<String>,
    pub provider_id: Option<String>,
    pub updated_at: Option<String>,
    pub archived: bool,
    pub size_bytes: u64,
    pub sha256: String,
    pub locations: Vec<SessionLocation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThreadSourceKind {
    Cli,
    Vscode,
    Internal,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReplicationEligibility {
    Eligible,
    CurrentProvider,
    Archived,
    InternalThread,
    InvalidRollout,
    AlreadyReplicated,
    Replica,
    ReplicaUpdated,
    SourceUpdated,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderBucket {
    pub profile_id: String,
    pub provider_id: String,
    pub is_current: bool,
    pub active_root_thread_count: usize,
    pub archived_thread_count: usize,
    pub internal_thread_count: usize,
    pub replicated_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSessionRecord {
    pub thread_id: String,
    pub provider_id: String,
    pub source_kind: ThreadSourceKind,
    pub archived: bool,
    pub title: String,
    pub cwd: Option<String>,
    pub updated_at: Option<String>,
    pub size_bytes: u64,
    pub sha256: String,
    pub agent_nickname: Option<String>,
    pub parent_thread_id: Option<String>,
    pub eligibility: ReplicationEligibility,
    pub eligibility_reason: String,
    pub replica_thread_id: Option<String>,
    pub is_replica: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderWorkspaceSnapshot {
    pub provider_buckets: Vec<ProviderBucket>,
    pub selected_provider_id: Option<String>,
    pub provider_sessions: Vec<ProviderSessionRecord>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveCleanupItem {
    pub thread_id: String,
    pub title: String,
    pub provider_id: String,
    pub source_kind: ThreadSourceKind,
    pub updated_at: Option<String>,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveCleanupPreview {
    pub profile_id: String,
    pub provider_id: String,
    pub items: Vec<ArchiveCleanupItem>,
    pub total_count: usize,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveCleanupResultItem {
    pub thread_id: String,
    pub title: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveCleanupResult {
    pub provider_id: String,
    pub deleted: Vec<ArchiveCleanupResultItem>,
    pub failed: Vec<ArchiveCleanupResultItem>,
    pub client_restarted: bool,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReplicationAction {
    CreateReplica,
    SkipAlreadyReplicated,
    SourceUpdated,
    SkipCurrentProvider,
    SkipArchived,
    SkipInternal,
    Invalid,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplicationPlanItem {
    pub thread_id: String,
    pub title: String,
    pub source_provider_id: String,
    pub action: ReplicationAction,
    pub reason: String,
    pub source_sha256: String,
    pub replica_thread_id: Option<String>,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplicationPreview {
    pub profile_id: String,
    pub target_provider_id: String,
    pub items: Vec<ReplicationPlanItem>,
    pub create_count: usize,
    pub skip_count: usize,
    pub invalid_count: usize,
    pub estimated_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplicaResultItem {
    pub source_thread_id: String,
    pub replica_thread_id: Option<String>,
    pub title: String,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplicationResult {
    pub job_id: String,
    pub target_provider_id: String,
    pub created: Vec<ReplicaResultItem>,
    pub skipped: Vec<ReplicaResultItem>,
    pub failed: Vec<ReplicaResultItem>,
    pub client_restarted: bool,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UpdateSyncAction {
    SourceUpdated,
    ReplicaUpdated,
    Conflict,
    Invalid,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSyncPlanItem {
    pub mapping_id: String,
    pub source_thread_id: String,
    pub replica_thread_id: String,
    pub title: String,
    pub source_provider_id: String,
    pub target_provider_id: String,
    pub action: UpdateSyncAction,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSyncPreview {
    pub profile_id: String,
    pub target_provider_id: String,
    pub items: Vec<UpdateSyncPlanItem>,
    pub update_count: usize,
    pub conflict_count: usize,
    pub invalid_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplicaMapping {
    pub id: String,
    pub profile_id: String,
    pub source_thread_id: String,
    pub source_provider_id: String,
    pub target_provider_id: String,
    pub replica_thread_id: String,
    pub source_sha256: String,
    pub replica_sha256: String,
    pub status: String,
    pub created_at: String,
    pub verified_at: Option<String>,
    pub deleted_at: Option<String>,
}
