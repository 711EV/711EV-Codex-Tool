mod app_server;
mod discovery;
mod error;
mod models;
pub mod portable;
mod process;
mod profiles;
mod replication;
mod sessions;
mod store;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tauri::Manager;

use models::{
    AppState, ArchiveCleanupPreview, ArchiveCleanupResult, DiscoveryReport, Profile, ProfileInput,
    ProviderWorkspaceSnapshot, ReplicaMapping, ReplicationPreview, ReplicationResult,
    SessionRecord, UpdateSyncPreview,
};
use store::Store;

struct AppContext {
    data_dir: PathBuf,
    store: Mutex<Store>,
    provider_scan_cache: Arc<Mutex<HashMap<String, sessions::IncrementalSessionCache>>>,
}

fn lock_store(context: &AppContext) -> Result<std::sync::MutexGuard<'_, Store>, String> {
    context
        .store
        .lock()
        .map_err(|_| "local database lock is unavailable".to_string())
}

#[tauri::command]
fn get_app_state(context: tauri::State<'_, AppContext>) -> Result<AppState, String> {
    let profiles = lock_store(&context)?
        .list_profiles()
        .map_err(String::from)?;
    let app_server_path = profiles
        .iter()
        .find_map(|profile| app_server::detect(profile.app_path.as_deref()))
        .or_else(|| app_server::detect(None))
        .map(|path| path.to_string_lossy().to_string());
    Ok(AppState {
        data_dir: context.data_dir.to_string_lossy().to_string(),
        platform: std::env::consts::OS.to_string(),
        profiles,
        app_server_path,
    })
}

#[tauri::command]
fn create_profile(
    context: tauri::State<'_, AppContext>,
    input: ProfileInput,
) -> Result<Profile, String> {
    let profile = profiles::create(&context.data_dir, input).map_err(String::from)?;
    lock_store(&context)?
        .insert_profile(&profile)
        .map_err(String::from)?;
    Ok(profile)
}

#[tauri::command]
fn delete_profile(context: tauri::State<'_, AppContext>, profile_id: String) -> Result<(), String> {
    lock_store(&context)?
        .delete_profile(&profile_id)
        .map_err(String::from)
}

#[tauri::command]
fn discover_profiles(context: tauri::State<'_, AppContext>) -> Result<DiscoveryReport, String> {
    let store = lock_store(&context)?;
    discover_and_register(&context.data_dir, &store)
}

#[tauri::command]
fn scan_sessions(
    context: tauri::State<'_, AppContext>,
    profile_id: Option<String>,
) -> Result<Vec<SessionRecord>, String> {
    let store = lock_store(&context)?;
    if let Some(profile_id) = profile_id {
        let profile = store.get_profile(&profile_id).map_err(String::from)?;
        return sessions::scan_profile(&profile)
            .map(|values| {
                values
                    .iter()
                    .map(|value| value.to_record(&profile))
                    .collect()
            })
            .map_err(String::from);
    }
    let profiles = store.list_profiles().map_err(String::from)?;
    sessions::aggregate_sessions(&profiles).map_err(String::from)
}

#[tauri::command]
async fn provider_workspace(
    context: tauri::State<'_, AppContext>,
    profile_id: String,
    provider_id: Option<String>,
) -> Result<ProviderWorkspaceSnapshot, String> {
    let profile = {
        let store = lock_store(&context)?;
        store.get_profile(&profile_id).map_err(String::from)?
    };
    let cache = Arc::clone(&context.provider_scan_cache);
    let data_dir = context.data_dir.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let mut session_cache = cache
            .lock()
            .map_err(|_| "provider scan cache lock is unavailable".to_string())?
            .remove(&profile_id)
            .unwrap_or_default();
        let snapshots = sessions::scan_profile_incremental(&profile, &mut session_cache);
        cache
            .lock()
            .map_err(|_| "provider scan cache lock is unavailable".to_string())?
            .insert(profile_id, session_cache);
        let snapshots = snapshots.map_err(String::from)?;
        let store = Store::open(&data_dir).map_err(String::from)?;
        let mappings = replication::reconcile_archived_mappings(&store, &profile.id, &snapshots)
            .map_err(String::from)?;
        let scan = replication::provider_scan_from_snapshots(&profile, &mappings, snapshots)
            .map_err(String::from)?;
        Ok(scan.workspace(provider_id.as_deref()))
    })
    .await
    .map_err(|error| format!("provider scan task failed: {error}"))?
}

#[tauri::command]
async fn archive_cleanup_preview(
    context: tauri::State<'_, AppContext>,
    profile_id: String,
    provider_id: String,
) -> Result<ArchiveCleanupPreview, String> {
    let data_dir = context.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let store = Store::open(&data_dir).map_err(String::from)?;
        let profile = store.get_profile(&profile_id).map_err(String::from)?;
        replication::preview_archive_cleanup(&store, &profile, &provider_id).map_err(String::from)
    })
    .await
    .map_err(|error| format!("archive cleanup preview task failed: {error}"))?
}

#[tauri::command]
async fn archive_cleanup_execute(
    context: tauri::State<'_, AppContext>,
    profile_id: String,
    provider_id: String,
    thread_ids: Vec<String>,
    force_close_client: bool,
) -> Result<ArchiveCleanupResult, String> {
    let data_dir = context.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let store = Store::open(&data_dir).map_err(String::from)?;
        let profile = store.get_profile(&profile_id).map_err(String::from)?;
        replication::cleanup_archived_sessions(
            &data_dir,
            &store,
            &profile,
            &provider_id,
            &thread_ids,
            force_close_client,
        )
        .map_err(String::from)
    })
    .await
    .map_err(|error| format!("archive cleanup task failed: {error}"))?
}

#[tauri::command]
async fn invalid_child_cleanup_preview(
    context: tauri::State<'_, AppContext>,
    profile_id: String,
    provider_id: String,
) -> Result<ArchiveCleanupPreview, String> {
    let data_dir = context.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let store = Store::open(&data_dir).map_err(String::from)?;
        let profile = store.get_profile(&profile_id).map_err(String::from)?;
        replication::preview_invalid_child_cleanup(&store, &profile, &provider_id)
            .map_err(String::from)
    })
    .await
    .map_err(|error| format!("child cleanup preview task failed: {error}"))?
}

#[tauri::command]
async fn invalid_child_cleanup_execute(
    context: tauri::State<'_, AppContext>,
    profile_id: String,
    provider_id: String,
    thread_ids: Vec<String>,
    force_close_client: bool,
) -> Result<ArchiveCleanupResult, String> {
    let data_dir = context.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let store = Store::open(&data_dir).map_err(String::from)?;
        let profile = store.get_profile(&profile_id).map_err(String::from)?;
        replication::cleanup_invalid_child_sessions(
            &data_dir,
            &store,
            &profile,
            &provider_id,
            &thread_ids,
            force_close_client,
        )
        .map_err(String::from)
    })
    .await
    .map_err(|error| format!("child cleanup task failed: {error}"))?
}

#[tauri::command]
async fn replication_preview(
    context: tauri::State<'_, AppContext>,
    profile_id: String,
    source_thread_ids: Vec<String>,
) -> Result<ReplicationPreview, String> {
    let data_dir = context.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let store = Store::open(&data_dir).map_err(String::from)?;
        let profile = store.get_profile(&profile_id).map_err(String::from)?;
        replication::preview(&store, &profile, &source_thread_ids).map_err(String::from)
    })
    .await
    .map_err(|error| format!("replication preview task failed: {error}"))?
}

#[tauri::command]
async fn replication_execute(
    context: tauri::State<'_, AppContext>,
    profile_id: String,
    source_thread_ids: Vec<String>,
    force_close_client: bool,
) -> Result<ReplicationResult, String> {
    let data_dir = context.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let store = Store::open(&data_dir).map_err(String::from)?;
        let profile = store.get_profile(&profile_id).map_err(String::from)?;
        replication::execute(
            &data_dir,
            &store,
            &profile,
            &source_thread_ids,
            force_close_client,
        )
        .map_err(String::from)
    })
    .await
    .map_err(|error| format!("replication execute task failed: {error}"))?
}

#[tauri::command]
async fn replication_migrate(
    context: tauri::State<'_, AppContext>,
    profile_id: String,
    source_thread_ids: Vec<String>,
    force_close_client: bool,
) -> Result<ReplicationResult, String> {
    let data_dir = context.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let store = Store::open(&data_dir).map_err(String::from)?;
        let profile = store.get_profile(&profile_id).map_err(String::from)?;
        replication::migrate(
            &data_dir,
            &store,
            &profile,
            &source_thread_ids,
            force_close_client,
        )
        .map_err(String::from)
    })
    .await
    .map_err(|error| format!("replication migration task failed: {error}"))?
}

#[tauri::command]
async fn restart_codex_client(
    context: tauri::State<'_, AppContext>,
    profile_id: String,
    force_close_client: bool,
) -> Result<bool, String> {
    let data_dir = context.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let store = Store::open(&data_dir).map_err(String::from)?;
        let profile = store.get_profile(&profile_id).map_err(String::from)?;
        let home = profile.home_path();
        let shutdown = process::ensure_stopped(&home, force_close_client).map_err(String::from)?;
        let app_path = profile
            .app_path
            .as_deref()
            .or(shutdown.executable.as_deref());
        let started = process::restart(app_path, &home).map_err(String::from)?;
        if !started {
            return Err("未检测到 Codex Desktop 启动路径".into());
        }
        Ok(true)
    })
    .await
    .map_err(|error| format!("client restart task failed: {error}"))?
}

#[tauri::command]
async fn replication_sync_updates(
    context: tauri::State<'_, AppContext>,
    profile_id: String,
    force_close_client: bool,
) -> Result<ReplicationResult, String> {
    let data_dir = context.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let store = Store::open(&data_dir).map_err(String::from)?;
        let profile = store.get_profile(&profile_id).map_err(String::from)?;
        replication::sync_updates(&data_dir, &store, &profile, force_close_client)
            .map_err(String::from)
    })
    .await
    .map_err(|error| format!("replication update sync task failed: {error}"))?
}

#[tauri::command]
async fn replication_sync_preview(
    context: tauri::State<'_, AppContext>,
    profile_id: String,
) -> Result<UpdateSyncPreview, String> {
    let data_dir = context.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let store = Store::open(&data_dir).map_err(String::from)?;
        let profile = store.get_profile(&profile_id).map_err(String::from)?;
        replication::preview_updates(&store, &profile).map_err(String::from)
    })
    .await
    .map_err(|error| format!("replication update preview task failed: {error}"))?
}

#[tauri::command]
fn replication_history(
    context: tauri::State<'_, AppContext>,
    profile_id: String,
) -> Result<Vec<ReplicaMapping>, String> {
    let store = lock_store(&context)?;
    replication::history(&store, &profile_id).map_err(String::from)
}

#[tauri::command]
fn replication_cleanup_orphans(
    context: tauri::State<'_, AppContext>,
    profile_id: String,
    force_close_client: bool,
) -> Result<Vec<ReplicaMapping>, String> {
    let store = lock_store(&context)?;
    let profile = store.get_profile(&profile_id).map_err(String::from)?;
    replication::cleanup_orphans(&store, &profile, force_close_client).map_err(String::from)
}

fn discover_and_register(
    data_dir: &std::path::Path,
    store: &Store,
) -> Result<DiscoveryReport, String> {
    let current = store.list_profiles().map_err(String::from)?;
    let scan = discovery::discover(data_dir, &current);
    let discovered_count = scan.instances.len();
    let mut by_path = current
        .iter()
        .map(|profile| {
            (
                discovery::normalized_path_key(&profile.home_path()),
                profile.clone(),
            )
        })
        .collect::<std::collections::HashMap<_, _>>();
    let mut added_count = 0;
    let mut refreshed_count = 0;
    for instance in scan.instances {
        let key = discovery::normalized_path_key(&instance.home);
        if let Some(existing) = by_path.get(&key) {
            let mut refreshed = profiles::from_discovery(instance);
            refreshed.id = existing.id.clone();
            refreshed.name = existing.name.clone();
            refreshed.mode = existing.mode.clone();
            refreshed.created_at = existing.created_at.clone();
            if refreshed.discovery_source == "已登记实例" {
                refreshed.discovery_source = existing.discovery_source.clone();
            }
            let provider_is_configured = refreshed.providers.iter().any(|provider| {
                provider.active && std::path::Path::new(&provider.source_file).is_file()
            });
            if !provider_is_configured {
                refreshed.provider_id = existing.provider_id.clone();
                refreshed.kind = existing.kind.clone();
                if !existing.providers.is_empty() {
                    refreshed.providers = existing.providers.clone();
                }
            }
            if refreshed.app_path.is_none() {
                refreshed.app_path = existing.app_path.clone();
            }
            store
                .refresh_discovered_profile(&refreshed)
                .map_err(String::from)?;
            refreshed_count += 1;
        } else {
            let profile = profiles::from_discovery(instance);
            store.insert_profile(&profile).map_err(String::from)?;
            by_path.insert(key, profile);
            added_count += 1;
        }
    }
    Ok(DiscoveryReport {
        candidates_scanned: scan.candidates_scanned,
        discovered_count,
        added_count,
        refreshed_count,
        profiles: store.list_profiles().map_err(String::from)?,
    })
}

pub fn run() {
    let data_dir = portable::resolve_data_dir()
        .unwrap_or_else(|error| panic!("failed to initialize portable storage: {error}"));
    let store = Store::open(&data_dir)
        .unwrap_or_else(|error| panic!("failed to open local database: {error}"));
    if let Err(error) = discover_and_register(&data_dir, &store) {
        eprintln!("failed to discover local profiles: {error}");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            app.handle()
                .plugin(tauri_plugin_updater::Builder::new().build())?;
            let icon = app.default_window_icon().cloned();
            if let Some(window) = app.get_webview_window("main") {
                if let Some(icon) = icon {
                    window.set_icon(icon)?;
                }
                #[cfg(target_os = "windows")]
                window.set_shadow(false)?;
            }
            Ok(())
        })
        .manage(AppContext {
            data_dir,
            store: Mutex::new(store),
            provider_scan_cache: Arc::new(Mutex::new(HashMap::new())),
        })
        .invoke_handler(tauri::generate_handler![
            get_app_state,
            create_profile,
            delete_profile,
            discover_profiles,
            scan_sessions,
            provider_workspace,
            archive_cleanup_preview,
            archive_cleanup_execute,
            invalid_child_cleanup_preview,
            invalid_child_cleanup_execute,
            replication_preview,
            replication_execute,
            replication_migrate,
            restart_codex_client,
            replication_sync_preview,
            replication_sync_updates,
            replication_history,
            replication_cleanup_orphans,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Codex Local Sync");
}
