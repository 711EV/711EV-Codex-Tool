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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncAction {
    Copy,
    Update,
    SkipIdentical,
    SkipTargetAhead,
    Conflict,
    Invalid,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPlanItem {
    pub thread_id: String,
    pub title: String,
    pub action: SyncAction,
    pub reason: String,
    pub source_sha256: String,
    pub target_sha256: Option<String>,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPreview {
    pub source_profile_id: String,
    pub target_profile_id: String,
    pub items: Vec<SyncPlanItem>,
    pub copy_count: usize,
    pub update_count: usize,
    pub skip_count: usize,
    pub conflict_count: usize,
    pub backup_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResult {
    pub job_id: String,
    pub copied_count: usize,
    pub updated_count: usize,
    pub skipped_count: usize,
    pub conflict_count: usize,
    pub backup_dir: Option<String>,
    pub index_rebuilt: bool,
    pub warning: Option<String>,
}
