mod app_server;
mod error;
mod models;
pub mod portable;
mod process;
mod profiles;
mod sessions;
mod store;
mod sync;

use std::path::PathBuf;
use std::sync::Mutex;

use models::{
    AppState, Profile, ProfileInput, ProfileKind, ProfileMode, SessionRecord, SyncPreview,
    SyncResult,
};
use store::Store;

struct AppContext {
    data_dir: PathBuf,
    store: Mutex<Store>,
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
fn preview_sync(
    context: tauri::State<'_, AppContext>,
    source_profile_id: String,
    target_profile_id: String,
    thread_ids: Vec<String>,
) -> Result<SyncPreview, String> {
    let store = lock_store(&context)?;
    let source = store
        .get_profile(&source_profile_id)
        .map_err(String::from)?;
    let target = store
        .get_profile(&target_profile_id)
        .map_err(String::from)?;
    sync::preview(&source, &target, &thread_ids).map_err(String::from)
}

#[tauri::command]
fn execute_sync(
    context: tauri::State<'_, AppContext>,
    source_profile_id: String,
    target_profile_id: String,
    thread_ids: Vec<String>,
    overwrite_conflicts: bool,
    force_close_target: bool,
) -> Result<SyncResult, String> {
    let store = lock_store(&context)?;
    let source = store
        .get_profile(&source_profile_id)
        .map_err(String::from)?;
    let target = store
        .get_profile(&target_profile_id)
        .map_err(String::from)?;
    sync::execute(
        &context.data_dir,
        &store,
        &source,
        &target,
        &thread_ids,
        overwrite_conflicts,
        force_close_target,
    )
    .map_err(String::from)
}

fn register_default_profile(data_dir: &std::path::Path, store: &Store) -> Result<(), String> {
    let Some(home) = profiles::discover_default() else {
        return Ok(());
    };
    let canonical = std::fs::canonicalize(&home).unwrap_or(home.clone());
    if store
        .list_profiles()
        .map_err(String::from)?
        .iter()
        .any(|profile| profile.home_path() == canonical)
    {
        return Ok(());
    }
    let provider = profiles::read_provider(&home, "openai");
    let profile = profiles::create(
        data_dir,
        ProfileInput {
            name: "当前登录账号".into(),
            kind: ProfileKind::ChatGptAccount,
            mode: ProfileMode::External,
            codex_home: Some(canonical.to_string_lossy().to_string()),
            provider_id: provider,
            app_path: None,
        },
    )
    .map_err(String::from)?;
    store.insert_profile(&profile).map_err(String::from)
}

pub fn run() {
    let data_dir = portable::resolve_data_dir()
        .unwrap_or_else(|error| panic!("failed to initialize portable storage: {error}"));
    let store = Store::open(&data_dir)
        .unwrap_or_else(|error| panic!("failed to open local database: {error}"));
    register_default_profile(&data_dir, &store)
        .unwrap_or_else(|error| eprintln!("failed to register default profile: {error}"));

    tauri::Builder::default()
        .manage(AppContext {
            data_dir,
            store: Mutex::new(store),
        })
        .invoke_handler(tauri::generate_handler![
            get_app_state,
            create_profile,
            delete_profile,
            scan_sessions,
            preview_sync,
            execute_sync,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Codex Local Sync");
}
