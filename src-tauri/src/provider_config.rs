//! Second-stage provider configuration support.
//!
//! This module deliberately edits only the primary CODEX_HOME files. Profile
//! overlays and project-level configuration are outside the tool's ownership.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use chrono::Utc;
use fs2::FileExt;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::{json, Value as JsonValue};
use sha2::{Digest, Sha256};
use toml_edit::{value, DocumentMut, Item, Table};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::models::{
    DiscoveredProvider, Profile, ProviderConfigInput, ProviderConfigTemplate, ProviderConfigView,
    ProviderSwitchResult,
};
use crate::store::{OfficialSnapshotRow, ProviderConfigRow, ProviderSwitchTransactionRow, Store};

const OFFICIAL_ID: &str = "openai";
const SEVEN_ELEVEN_ID: &str = "711EV";
const SEVEN_ELEVEN_URL: &str = "https://ai.711ev.com/v1";
const RESPONSES_WIRE_API: &str = "responses";
const AUTH_EVENT_DEBOUNCE: Duration = Duration::from_millis(500);
const WATCH_PROFILE_REFRESH: Duration = Duration::from_secs(30);
const SNAPSHOT_FALLBACK_SCAN: Duration = Duration::from_secs(60);

pub fn templates() -> Vec<ProviderConfigTemplate> {
    vec![ProviderConfigTemplate {
        id: "711ev".into(),
        fixed_provider_id: SEVEN_ELEVEN_ID.into(),
        fixed_base_url: SEVEN_ELEVEN_URL.into(),
    }]
}

pub fn read(
    _data_dir: &Path,
    store: &Store,
    profile_id: &str,
    provider_id: &str,
) -> AppResult<ProviderConfigView> {
    let profile = store.get_profile(profile_id)?;
    read_for_profile(_data_dir, store, &profile, provider_id)
}

pub fn reveal_key(store: &Store, profile_id: &str, provider_id: &str) -> AppResult<String> {
    store
        .get_provider_config(profile_id, provider_id)?
        .map(|value| value.api_key)
        .ok_or_else(|| AppError::Message("当前供应商没有可显示的 API 密钥".into()))
}

pub fn save(
    data_dir: &Path,
    store: &Store,
    input: ProviderConfigInput,
) -> AppResult<ProviderConfigView> {
    recover_transactions(data_dir, store)?;
    let profile = store.get_profile(&input.profile_id)?;
    let use_711ev_defaults = input.template.as_deref() == Some("711ev");
    let input_provider_id = input.provider_id.trim();
    let provider_id = if use_711ev_defaults && input_provider_id.is_empty() {
        SEVEN_ELEVEN_ID
    } else {
        input_provider_id
    };
    validate_provider_id(provider_id)?;
    if provider_id.eq_ignore_ascii_case(OFFICIAL_ID) {
        return Err(AppError::Message("官方供应商不支持编辑 API 密钥".into()));
    }
    let api_key = input
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            store
                .get_provider_config(&input.profile_id, provider_id)
                .ok()
                .flatten()
                .map(|value| value.api_key)
        })
        .ok_or_else(|| AppError::Message("API 密钥不能为空".into()))?;
    let raw_url = input
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| use_711ev_defaults.then_some(SEVEN_ELEVEN_URL))
        .ok_or_else(|| AppError::Message("API 地址不能为空".into()))?;
    let base_url = normalize_url(raw_url)?;

    let _lock = acquire_home_lock(data_dir, &profile.id)?;
    let config_path = profile.home_path().join("config.toml");
    let content_bytes = read_optional(&config_path)?;
    let content = String::from_utf8_lossy(&content_bytes).into_owned();
    let parsed = content
        .parse::<toml::Value>()
        .map_err(|error| AppError::Message(format!("config.toml 解析失败: {error}")))?;
    let conflicting_file_id = parsed
        .get("model_providers")
        .and_then(toml::Value::as_table)
        .and_then(|providers| {
            providers
                .keys()
                .find(|existing| {
                    existing.eq_ignore_ascii_case(provider_id) && existing.as_str() != provider_id
                })
                .cloned()
        });
    let conflicting_database_id = store
        .list_provider_configs(&input.profile_id)?
        .into_iter()
        .map(|value| value.provider_id)
        .find(|existing| {
            existing.eq_ignore_ascii_case(provider_id) && existing.as_str() != provider_id
        });
    if let Some(existing) = conflicting_file_id.or(conflicting_database_id) {
        return Err(AppError::Message(format!(
            "供应商 ID 与已有配置 {existing} 仅大小写不同，请使用其他 ID"
        )));
    }
    let timestamp = Utc::now().to_rfc3339();
    let row = ProviderConfigRow {
        profile_id: input.profile_id.clone(),
        provider_id: provider_id.to_string(),
        base_url,
        requires_openai_auth: true,
        api_key,
        managed_by_tool: true,
        created_at: timestamp.clone(),
        updated_at: timestamp,
    };
    let previous_row = store.get_provider_config(&input.profile_id, provider_id)?;
    let current_provider = top_level_provider(&parsed).unwrap_or_default();
    let editing_current = current_provider.eq_ignore_ascii_case(provider_id);

    // Editing the active provider only changes the desired SQLite record. The
    // existing disk projection is applied later by the explicit switch/apply
    // action, so saving a form cannot change a running Codex configuration.
    if !editing_current {
        let candidate = upsert_provider_section(&content, provider_id, &row, false)?;
        store.upsert_provider_config(&row)?;
        if let Err(error) = write_atomic(&config_path, candidate.as_bytes()).and_then(|_| {
            validate_saved_provider_definition(&config_path, provider_id, &row, &current_provider)
        }) {
            let _ = write_atomic(&config_path, &content_bytes);
            restore_provider_row(store, previous_row, &input.profile_id, provider_id);
            return Err(error);
        }
    } else {
        store.upsert_provider_config(&row)?;
    }
    ensure_profile_provider(store, profile, provider_id, &config_path)?;
    read(data_dir, store, &input.profile_id, provider_id)
}

fn restore_provider_row(
    store: &Store,
    previous: Option<ProviderConfigRow>,
    profile_id: &str,
    provider_id: &str,
) {
    if let Some(previous) = previous {
        let _ = store.upsert_provider_config(&previous);
    } else {
        let _ = store.delete_provider_config(profile_id, provider_id);
    }
}

pub fn switch(
    data_dir: &Path,
    store: &Store,
    profile_id: &str,
    provider_id: &str,
) -> AppResult<ProviderSwitchResult> {
    // A previous process may have stopped between the two file replacements.
    // Resolve that durable transaction before accepting another write.
    recover_transactions(data_dir, store)?;
    let profile = store.get_profile(profile_id)?;
    let home = profile.home_path();
    let _lock = acquire_home_lock(data_dir, &profile.id)?;
    validate_auth_store(&home)?;

    // Capture an OAuth login immediately before replacing auth.json. This
    // handles a token refresh that happened after the last polling pass.
    capture_official_snapshot_locked(data_dir, store, &profile)?;

    let config_path = home.join("config.toml");
    let auth_path = home.join("auth.json");
    let config_existed = config_path.is_file();
    let config_before = read_optional(&config_path)?;
    let auth_before = read_optional(&auth_path)?;
    let auth_existed = auth_path.is_file();
    let target = build_switch_target(store, &profile, provider_id, &config_before)?;
    let transaction_id = Uuid::new_v4().to_string();
    let transaction_dir = data_dir.join("transactions").join(&transaction_id);
    fs::create_dir_all(&transaction_dir)?;
    set_private_directory_permissions(&transaction_dir)?;
    let config_backup = transaction_dir.join("config.before");
    let auth_backup = transaction_dir.join("auth.before");
    let config_candidate = transaction_dir.join("config.after");
    let auth_candidate = transaction_dir.join("auth.after");
    write_atomic(&config_backup, &config_before)?;
    write_atomic(&auth_backup, &auth_before)?;
    write_atomic(&config_candidate, &target.config)?;
    if let Some(auth) = &target.auth {
        write_atomic(&auth_candidate, auth)?;
    }
    set_private_file_permissions(&config_backup)?;
    set_private_file_permissions(&auth_backup)?;
    set_private_file_permissions(&config_candidate)?;
    if auth_candidate.is_file() {
        set_private_file_permissions(&auth_candidate)?;
    }
    let transaction = ProviderSwitchTransactionRow {
        id: transaction_id.clone(),
        profile_id: profile_id.to_string(),
        provider_id: provider_id.to_string(),
        codex_home: home.to_string_lossy().to_string(),
        config_backup_path: config_backup.to_string_lossy().to_string(),
        config_existed,
        auth_backup_path: auth_backup.to_string_lossy().to_string(),
        auth_existed,
        config_candidate_path: config_candidate.to_string_lossy().to_string(),
        auth_candidate_path: auth_candidate.to_string_lossy().to_string(),
        auth_target_exists: target.auth.is_some(),
        expected_config_sha256: digest(&target.config),
        expected_auth_sha256: target.auth.as_deref().map(digest),
        phase: "prepared".into(),
        created_at: Utc::now().to_rfc3339(),
    };
    if let Err(error) = store.insert_switch_transaction(&transaction) {
        let _ = fs::remove_dir_all(&transaction_dir);
        return Err(error);
    }

    let result = commit_switch_transaction(store, &transaction);
    match result {
        Ok(()) => {
            finish_transaction(store, &transaction)?;
            let mut updated_profile = profile.clone();
            updated_profile.provider_id = provider_id.to_string();
            for provider in &mut updated_profile.providers {
                provider.active = provider.id == provider_id;
            }
            if !updated_profile
                .providers
                .iter()
                .any(|provider| provider.id == provider_id)
            {
                updated_profile.providers.push(DiscoveredProvider {
                    id: provider_id.to_string(),
                    source_file: config_path.to_string_lossy().to_string(),
                    active: true,
                });
            }
            updated_profile.updated_at = Utc::now().to_rfc3339();
            store.refresh_discovered_profile(&updated_profile)?;
            Ok(ProviderSwitchResult {
                profile_id: profile_id.to_string(),
                provider_id: provider_id.to_string(),
                config_file: config_path.to_string_lossy().to_string(),
                auth_file: auth_path.to_string_lossy().to_string(),
                restarted: false,
                warning: None,
            })
        }
        Err(error) => {
            let rollback = restore_transaction(&transaction);
            if let Err(rollback_error) = rollback {
                return Err(AppError::Message(format!(
                    "供应商切换失败，且回滚未完成: {error}; {rollback_error}"
                )));
            }
            finish_transaction(store, &transaction)?;
            Err(error)
        }
    }
}

struct SwitchTarget {
    config: Vec<u8>,
    auth: Option<Vec<u8>>,
}

fn build_switch_target(
    store: &Store,
    profile: &Profile,
    provider_id: &str,
    config: &[u8],
) -> AppResult<SwitchTarget> {
    let config_text = std::str::from_utf8(config)
        .map_err(|error| AppError::Message(format!("config.toml 不是有效 UTF-8: {error}")))?;
    if provider_id.eq_ignore_ascii_case(OFFICIAL_ID) {
        let config = set_top_level_provider(config_text, OFFICIAL_ID)?;
        let auth = if let Some(snapshot) = store.get_official_snapshot(&profile.id)? {
            let path = PathBuf::from(snapshot.snapshot_path);
            if path.is_file() {
                let bytes = fs::read(path)?;
                if digest(&bytes) != snapshot.source_sha256 {
                    return Err(AppError::Message(
                        "官方凭据快照完整性校验失败，已中止切换".into(),
                    ));
                }
                Some(bytes)
            } else {
                None
            }
        } else {
            None
        };
        return Ok(SwitchTarget {
            config: config.into_bytes(),
            auth,
        });
    }

    let row = store
        .get_provider_config(&profile.id, provider_id)?
        .ok_or_else(|| AppError::Message("该供应商尚未配置 API 密钥，请先完成配置".into()))?;
    let config = upsert_provider_section(config_text, provider_id, &row, true)?;
    let config = set_top_level_provider(&config, provider_id)?;
    let auth = serde_json::to_vec_pretty(&json!({ "OPENAI_API_KEY": row.api_key }))?;
    Ok(SwitchTarget {
        config: config.into_bytes(),
        auth: Some(auth),
    })
}

fn commit_switch_transaction(
    store: &Store,
    transaction: &ProviderSwitchTransactionRow,
) -> AppResult<()> {
    let home = Path::new(&transaction.codex_home);
    let config_path = home.join("config.toml");
    let auth_path = home.join("auth.json");
    let config_candidate = read_and_verify(
        Path::new(&transaction.config_candidate_path),
        &transaction.expected_config_sha256,
        "配置候选文件",
    )?;
    write_atomic(&config_path, &config_candidate)?;
    verify_file_hash(
        &config_path,
        &transaction.expected_config_sha256,
        "config.toml",
    )?;
    store.update_switch_transaction_phase(&transaction.id, "config_committed")?;

    apply_auth_target(transaction, &auth_path)?;
    store.update_switch_transaction_phase(&transaction.id, "auth_committed")?;
    validate_transaction_target(transaction)?;
    store.update_switch_transaction_phase(&transaction.id, "validated")?;
    Ok(())
}

pub fn recover_transactions(data_dir: &Path, store: &Store) -> AppResult<()> {
    for transaction in store.list_pending_switch_transactions()? {
        let _lock = acquire_home_lock(data_dir, &transaction.profile_id)?;
        if transaction.config_candidate_path.is_empty()
            || transaction.expected_config_sha256.is_empty()
        {
            restore_transaction(&transaction)?;
            finish_transaction(store, &transaction)?;
            continue;
        }
        if let Err(recovery_error) = resume_transaction(store, &transaction) {
            if recovery_error.to_string().contains("不能覆盖") {
                return Err(AppError::Message(format!(
                    "未完成的供应商切换检测到外部文件改动，事务已保留: {recovery_error}"
                )));
            }
            if let Err(rollback_error) = restore_transaction(&transaction) {
                return Err(AppError::Message(format!(
                    "未完成的供应商切换无法恢复或回滚: {recovery_error}; {rollback_error}"
                )));
            }
        }
        finish_transaction(store, &transaction)?;
    }
    Ok(())
}

fn resume_transaction(store: &Store, transaction: &ProviderSwitchTransactionRow) -> AppResult<()> {
    let home = Path::new(&transaction.codex_home);
    let config_path = home.join("config.toml");
    let auth_path = home.join("auth.json");
    match transaction.phase.as_str() {
        "prepared" => {
            let config_hash = file_hash(&config_path);
            let original_config_hash = original_file_hash(
                Path::new(&transaction.config_backup_path),
                transaction.config_existed,
            );
            if config_hash.as_deref() != Some(transaction.expected_config_sha256.as_str())
                && config_hash != original_config_hash
            {
                return Err(AppError::Message(
                    "config.toml 在事务恢复前已被其他内容修改，不能覆盖".into(),
                ));
            }
            if config_hash.as_deref() != Some(transaction.expected_config_sha256.as_str()) {
                let candidate = read_and_verify(
                    Path::new(&transaction.config_candidate_path),
                    &transaction.expected_config_sha256,
                    "配置候选文件",
                )?;
                write_atomic(&config_path, &candidate)?;
            }
            verify_file_hash(
                &config_path,
                &transaction.expected_config_sha256,
                "config.toml",
            )?;
            store.update_switch_transaction_phase(&transaction.id, "config_committed")?;
            ensure_auth_is_original_or_target(transaction, &auth_path)?;
            apply_auth_target(transaction, &auth_path)?;
            store.update_switch_transaction_phase(&transaction.id, "auth_committed")?;
        }
        "config_committed" => {
            verify_file_hash(
                &config_path,
                &transaction.expected_config_sha256,
                "config.toml",
            )?;
            ensure_auth_is_original_or_target(transaction, &auth_path)?;
            apply_auth_target(transaction, &auth_path)?;
            store.update_switch_transaction_phase(&transaction.id, "auth_committed")?;
        }
        "auth_committed" | "validated" => {}
        phase => {
            return Err(AppError::Message(format!(
                "无法识别的供应商切换事务阶段: {phase}"
            )))
        }
    }
    validate_transaction_target(transaction)?;
    store.update_switch_transaction_phase(&transaction.id, "validated")?;
    Ok(())
}

fn apply_auth_target(
    transaction: &ProviderSwitchTransactionRow,
    auth_path: &Path,
) -> AppResult<()> {
    if transaction.auth_target_exists {
        let expected = transaction
            .expected_auth_sha256
            .as_deref()
            .ok_or_else(|| AppError::Message("认证候选文件缺少期望哈希，无法继续切换".into()))?;
        let candidate = read_and_verify(
            Path::new(&transaction.auth_candidate_path),
            expected,
            "认证候选文件",
        )?;
        write_atomic(auth_path, &candidate)?;
        verify_file_hash(auth_path, expected, "auth.json")?;
    } else if auth_path.exists() {
        fs::remove_file(auth_path)?;
        sync_parent(auth_path)?;
    }
    Ok(())
}

fn original_file_hash(path: &Path, existed: bool) -> Option<String> {
    if existed {
        file_hash(path)
    } else {
        None
    }
}

fn ensure_auth_is_original_or_target(
    transaction: &ProviderSwitchTransactionRow,
    auth_path: &Path,
) -> AppResult<()> {
    let current = file_hash(auth_path);
    let original = original_file_hash(
        Path::new(&transaction.auth_backup_path),
        transaction.auth_existed,
    );
    let target = transaction.expected_auth_sha256.clone();
    if current != original && current != target {
        return Err(AppError::Message(
            "auth.json 在事务恢复前已被其他内容修改，不能覆盖".into(),
        ));
    }
    Ok(())
}

fn validate_transaction_target(transaction: &ProviderSwitchTransactionRow) -> AppResult<()> {
    let home = Path::new(&transaction.codex_home);
    verify_file_hash(
        &home.join("config.toml"),
        &transaction.expected_config_sha256,
        "config.toml",
    )?;
    let auth_path = home.join("auth.json");
    if transaction.auth_target_exists {
        verify_file_hash(
            &auth_path,
            transaction
                .expected_auth_sha256
                .as_deref()
                .unwrap_or_default(),
            "auth.json",
        )?;
    } else if auth_path.exists() {
        return Err(AppError::Message(
            "auth.json 应已删除，但重新读取时仍然存在".into(),
        ));
    }
    Ok(())
}

fn finish_transaction(store: &Store, transaction: &ProviderSwitchTransactionRow) -> AppResult<()> {
    store.delete_switch_transaction(&transaction.id)?;
    if let Some(directory) = Path::new(&transaction.config_backup_path).parent() {
        let _ = fs::remove_dir_all(directory);
    }
    Ok(())
}

pub fn poll_official_snapshots(data_dir: &Path) -> AppResult<()> {
    let store = Store::open(data_dir)?;
    for profile in store.list_profiles()? {
        if validate_auth_store(&profile.home_path()).is_err() {
            continue;
        }
        let Ok(_lock) = acquire_home_lock(data_dir, &profile.id) else {
            continue;
        };
        capture_official_snapshot_locked(data_dir, &store, &profile)?;
    }
    Ok(())
}

pub fn start_official_snapshot_watcher(data_dir: PathBuf) {
    thread::spawn(move || {
        if let Err(error) = run_official_snapshot_watcher(&data_dir) {
            eprintln!("official auth watcher stopped: {error}");
        }
    });
}

fn run_official_snapshot_watcher(data_dir: &Path) -> AppResult<()> {
    let (sender, receiver) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |event| {
        let _ = sender.send(event);
    })
    .map_err(|error| AppError::Message(format!("无法启动 auth.json 监听器: {error}")))?;
    let mut profiles = HashMap::<PathBuf, Profile>::new();
    let mut watched_homes = HashSet::<PathBuf>::new();
    let mut pending = HashMap::<PathBuf, Instant>::new();
    let mut last_profile_refresh = Instant::now() - WATCH_PROFILE_REFRESH;
    let mut last_fallback_scan = Instant::now();

    loop {
        if last_profile_refresh.elapsed() >= WATCH_PROFILE_REFRESH {
            refresh_watched_homes(data_dir, &mut watcher, &mut watched_homes, &mut profiles)?;
            last_profile_refresh = Instant::now();
        }

        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(event)) => queue_auth_events(event, &profiles, &mut pending),
            Ok(Err(error)) => eprintln!("auth.json watch event error: {error}"),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(AppError::Message("auth.json 监听通道已关闭".into()))
            }
        }

        let ready = pending
            .iter()
            .filter(|(_, queued_at)| queued_at.elapsed() >= AUTH_EVENT_DEBOUNCE)
            .map(|(home, _)| home.clone())
            .collect::<Vec<_>>();
        for home in ready {
            let Some(profile) = profiles.get(&home) else {
                pending.remove(&home);
                continue;
            };
            match capture_snapshot_for_event(data_dir, profile) {
                Ok(true) => {
                    pending.remove(&home);
                }
                Ok(false) => {
                    pending.insert(home, Instant::now());
                }
                Err(error) => {
                    eprintln!("auth.json snapshot update failed: {error}");
                    pending.insert(home, Instant::now());
                }
            }
        }

        if last_fallback_scan.elapsed() >= SNAPSHOT_FALLBACK_SCAN {
            if let Err(error) = poll_official_snapshots(data_dir) {
                eprintln!("official auth fallback scan failed: {error}");
            }
            last_fallback_scan = Instant::now();
        }
    }
}

fn refresh_watched_homes(
    data_dir: &Path,
    watcher: &mut RecommendedWatcher,
    watched_homes: &mut HashSet<PathBuf>,
    profiles: &mut HashMap<PathBuf, Profile>,
) -> AppResult<()> {
    let store = Store::open(data_dir)?;
    let next_profiles = store
        .list_profiles()?
        .into_iter()
        .filter(|profile| validate_auth_store(&profile.home_path()).is_ok())
        .map(|profile| (normalized_path(&profile.home_path()), profile))
        .collect::<HashMap<_, _>>();
    let next_homes = next_profiles.keys().cloned().collect::<HashSet<_>>();

    for home in watched_homes.difference(&next_homes) {
        let _ = watcher.unwatch(home);
    }
    for home in next_homes.difference(watched_homes) {
        watcher
            .watch(home, RecursiveMode::NonRecursive)
            .map_err(|error| {
                AppError::Message(format!("无法监听存储位置 {}: {error}", home.display()))
            })?;
    }
    *watched_homes = next_homes;
    *profiles = next_profiles;
    Ok(())
}

fn queue_auth_events(
    event: Event,
    profiles: &HashMap<PathBuf, Profile>,
    pending: &mut HashMap<PathBuf, Instant>,
) {
    for path in event.paths {
        let is_auth = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("auth.json"));
        if !is_auth {
            continue;
        }
        if let Some(parent) = path.parent() {
            let home = normalized_path(parent);
            if profiles.contains_key(&home) {
                pending.insert(home, Instant::now());
            }
        }
    }
}

fn capture_snapshot_for_event(data_dir: &Path, profile: &Profile) -> AppResult<bool> {
    let Ok(_lock) = acquire_home_lock(data_dir, &profile.id) else {
        return Ok(false);
    };
    let store = Store::open(data_dir)?;
    capture_official_snapshot_locked(data_dir, &store, profile)?;
    Ok(true)
}

fn normalized_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn capture_official_snapshot_locked(
    data_dir: &Path,
    store: &Store,
    profile: &Profile,
) -> AppResult<()> {
    let auth_path = profile.home_path().join("auth.json");
    let bytes = match read_stable_file(&auth_path)? {
        Some(bytes) => bytes,
        None => return Ok(()),
    };
    if !is_oauth_auth(&bytes) {
        return Ok(());
    }
    let snapshot_dir = data_dir
        .join("auth-snapshots")
        .join(path_hash(&profile.home_path()));
    fs::create_dir_all(&snapshot_dir)?;
    let snapshot_path = snapshot_dir.join("auth.json");
    let source_sha256 = digest(&bytes);
    let source_modified_at = fs::metadata(&auth_path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .map(chrono::DateTime::<Utc>::from)
        .map(|value| value.to_rfc3339());
    let should_write = match store.get_official_snapshot(&profile.id)? {
        Some(value) => {
            value.source_sha256 != source_sha256 || !Path::new(&value.snapshot_path).is_file()
        }
        None => true,
    };
    if should_write {
        write_atomic(&snapshot_path, &bytes)?;
        if let Err(error) = set_snapshot_permissions(&snapshot_dir, &snapshot_path) {
            let _ = fs::remove_file(&snapshot_path);
            return Err(error);
        }
        store.save_official_snapshot(&OfficialSnapshotRow {
            profile_id: profile.id.clone(),
            snapshot_path: snapshot_path.to_string_lossy().to_string(),
            source_sha256,
            source_modified_at,
            captured_at: Utc::now().to_rfc3339(),
        })?;
    }
    Ok(())
}

fn read_stable_file(path: &Path) -> AppResult<Option<Vec<u8>>> {
    for attempt in 0..5 {
        let before = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let after = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if before.len() == after.len()
            && before.modified().ok() == after.modified().ok()
            && bytes.len() as u64 == after.len()
        {
            return Ok(Some(bytes));
        }
        if attempt < 4 {
            thread::sleep(Duration::from_millis(100));
        }
    }
    Err(AppError::Message(
        "auth.json 持续变化，暂时无法读取稳定内容".into(),
    ))
}

fn read_for_profile(
    _data_dir: &Path,
    store: &Store,
    profile: &Profile,
    provider_id: &str,
) -> AppResult<ProviderConfigView> {
    let home = profile.home_path();
    let config_path = home.join("config.toml");
    let auth_path = home.join("auth.json");
    let config_bytes = read_optional(&config_path)?;
    let config_text = String::from_utf8_lossy(&config_bytes);
    let parsed = config_text
        .parse::<toml::Value>()
        .map_err(|error| AppError::Message(format!("config.toml 解析失败: {error}")))?;
    let provider = parsed
        .get("model_providers")
        .and_then(|value| value.get(provider_id));
    let row = store.get_provider_config(&profile.id, provider_id)?;
    let base_url = row
        .as_ref()
        .map(|value| value.base_url.clone())
        .or_else(|| {
            provider
                .and_then(|value| value.get("base_url"))
                .and_then(toml::Value::as_str)
                .map(str::to_string)
        });
    let auth_bytes = read_optional(&auth_path)?;
    let auth = serde_json::from_slice::<JsonValue>(&auth_bytes).ok();
    let api_key = auth
        .as_ref()
        .and_then(|value| value.get("OPENAI_API_KEY"))
        .and_then(JsonValue::as_str)
        .filter(|value| !value.is_empty());
    let oauth = is_oauth_auth(&auth_bytes);
    let auth_storage = parsed
        .get("cli_auth_credentials_store")
        .and_then(toml::Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| "unknown".into());
    let snapshot = store.get_official_snapshot(&profile.id)?;
    let snapshot_status = snapshot
        .as_ref()
        .map(|value| {
            let path = Path::new(&value.snapshot_path);
            match fs::read(path) {
                Ok(bytes) if digest(&bytes) == value.source_sha256 => "available",
                Ok(_) => "conflict",
                Err(_) => "missing",
            }
        })
        .unwrap_or("missing");
    let db_key = row.as_ref().map(|value| value.api_key.as_str());
    let config_key = provider
        .and_then(|value| value.get("experimental_bearer_token"))
        .and_then(toml::Value::as_str);
    let current_provider = top_level_provider(&parsed).unwrap_or_default();
    let is_current = current_provider.eq_ignore_ascii_case(provider_id);
    let match_database = row
        .as_ref()
        .is_some_and(|_| is_current && db_key == config_key && db_key == api_key);
    let auth_kind = if oauth {
        "oauth"
    } else if provider
        .and_then(|value| value.get("env_key"))
        .and_then(toml::Value::as_str)
        .is_some()
    {
        "environment_api_key"
    } else if !auth_bytes.is_empty() && auth.is_none() {
        "unknown"
    } else if api_key.is_some() {
        "database_api_key"
    } else {
        "missing"
    };
    let configured =
        provider_id.eq_ignore_ascii_case(OFFICIAL_ID) || row.is_some() || provider.is_some();
    let has_pending_changes = row.as_ref().is_some_and(|value| {
        provider
            .and_then(|item| item.get("name"))
            .and_then(toml::Value::as_str)
            != Some(provider_id)
            || provider
                .and_then(|item| item.get("base_url"))
                .and_then(toml::Value::as_str)
                != Some(value.base_url.as_str())
            || provider
                .and_then(|item| item.get("wire_api"))
                .and_then(toml::Value::as_str)
                != Some(RESPONSES_WIRE_API)
            || provider
                .and_then(|item| item.get("requires_openai_auth"))
                .and_then(toml::Value::as_bool)
                != Some(value.requires_openai_auth)
            || provider
                .and_then(|item| item.get("experimental_bearer_token"))
                .and_then(toml::Value::as_str)
                != Some(value.api_key.as_str())
    });
    let can_switch = auth_storage == "file"
        && (provider_id.eq_ignore_ascii_case(OFFICIAL_ID) || row.is_some())
        && !(provider_id.eq_ignore_ascii_case(OFFICIAL_ID) && snapshot_status == "conflict");
    Ok(ProviderConfigView {
        profile_id: profile.id.clone(),
        provider_id: provider_id.to_string(),
        base_url,
        env_key: provider
            .and_then(|value| value.get("env_key"))
            .and_then(toml::Value::as_str)
            .map(str::to_string),
        requires_openai_auth: provider
            .and_then(|value| value.get("requires_openai_auth"))
            .and_then(toml::Value::as_bool),
        experimental_bearer_token_present: config_key.is_some(),
        auth_json_api_key_present: api_key.is_some(),
        active_key_files_match_database: match_database,
        config_file: config_path.to_string_lossy().to_string(),
        auth_file: auth_path.to_string_lossy().to_string(),
        auth_kind: auth_kind.into(),
        auth_storage,
        official_auth_snapshot_status: snapshot_status.into(),
        official_auth_captured_at: snapshot.map(|value| value.captured_at),
        api_key_masked: row.as_ref().map(|value| mask_key(&value.api_key)),
        managed_by_tool: row.as_ref().is_some_and(|value| value.managed_by_tool),
        configured,
        can_switch,
        has_pending_changes,
        config_fingerprint: digest(&config_bytes),
    })
}

fn ensure_profile_provider(
    store: &Store,
    mut profile: Profile,
    provider_id: &str,
    config_path: &Path,
) -> AppResult<()> {
    if !profile
        .providers
        .iter()
        .any(|provider| provider.id == provider_id)
    {
        profile.providers.push(DiscoveredProvider {
            id: provider_id.to_string(),
            source_file: config_path.to_string_lossy().to_string(),
            active: false,
        });
        profile.updated_at = Utc::now().to_rfc3339();
        store.refresh_discovered_profile(&profile)?;
    }
    Ok(())
}

fn validate_saved_provider_definition(
    config_path: &Path,
    provider_id: &str,
    expected: &ProviderConfigRow,
    previous_current_provider: &str,
) -> AppResult<()> {
    let bytes = fs::read(config_path)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|error| AppError::Message(format!("config.toml 不是有效 UTF-8: {error}")))?;
    let parsed = text
        .parse::<toml::Value>()
        .map_err(|error| AppError::Message(format!("config.toml 写入后解析失败: {error}")))?;
    if top_level_provider(&parsed).unwrap_or_default() != previous_current_provider {
        return Err(AppError::Message(
            "保存供应商时当前 model_provider 被意外改变".into(),
        ));
    }
    let provider = parsed
        .get("model_providers")
        .and_then(|value| value.get(provider_id))
        .ok_or_else(|| AppError::Message("供应商配置写入后未找到目标 section".into()))?;
    let matches = provider.get("name").and_then(toml::Value::as_str)
        == Some(expected.provider_id.as_str())
        && provider.get("base_url").and_then(toml::Value::as_str)
            == Some(expected.base_url.as_str())
        && provider.get("wire_api").and_then(toml::Value::as_str) == Some(RESPONSES_WIRE_API)
        && provider
            .get("requires_openai_auth")
            .and_then(toml::Value::as_bool)
            == Some(expected.requires_openai_auth);
    if !matches {
        return Err(AppError::Message(
            "供应商配置写入后字段一致性校验失败".into(),
        ));
    }
    Ok(())
}

fn validate_provider_id(value: &str) -> AppResult<()> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .chars()
            .next()
            .is_some_and(|value| value.is_ascii_alphanumeric())
        || !value
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, '.' | '-' | '_'))
    {
        return Err(AppError::Message(
            "供应商 ID 只能使用 ASCII 字母、数字、点、短横线和下划线".into(),
        ));
    }
    Ok(())
}

fn top_level_provider(value: &toml::Value) -> Option<String> {
    value
        .get("model_provider")
        .and_then(toml::Value::as_str)
        .map(str::to_string)
}

fn normalize_url(value: &str) -> AppResult<String> {
    let value = value.trim();
    let suffix = [value.find('?'), value.find('#')]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(value.len());
    let mut value = value[..suffix].to_string();
    if !value.starts_with("https://") {
        return Err(AppError::Message("其他供应商只允许使用 HTTPS 地址".into()));
    }
    while value.ends_with('/') {
        value.pop();
    }
    Ok(value)
}

fn validate_auth_store(home: &Path) -> AppResult<()> {
    let path = home.join("config.toml");
    let content = read_optional(&path)?;
    let parsed = String::from_utf8_lossy(&content)
        .parse::<toml::Value>()
        .map_err(|error| AppError::Message(format!("config.toml 解析失败: {error}")))?;
    let store = parsed
        .get("cli_auth_credentials_store")
        .and_then(toml::Value::as_str)
        .unwrap_or("unknown");
    if store != "file" {
        return Err(AppError::Message(
            "不支持当前认证存储方式，仅支持 cli_auth_credentials_store = \"file\"".into(),
        ));
    }
    Ok(())
}

fn is_oauth_auth(bytes: &[u8]) -> bool {
    serde_json::from_slice::<JsonValue>(bytes)
        .ok()
        .and_then(|value| value.get("tokens").cloned())
        .and_then(|value| value.as_object().cloned())
        .is_some_and(|tokens| {
            ["access_token", "refresh_token", "id_token"]
                .iter()
                .any(|key| {
                    tokens
                        .get(*key)
                        .and_then(JsonValue::as_str)
                        .is_some_and(|value| !value.trim().is_empty())
                })
        })
}

fn read_optional(path: &Path) -> AppResult<Vec<u8>> {
    match fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.into()),
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension(format!("tmp-{}", Uuid::new_v4()));
    let mut file = File::create(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    if let Err(error) = replace_file(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(error.into());
    }
    sync_parent(path)?;
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn sync_parent(_path: &Path) -> AppResult<()> {
    #[cfg(unix)]
    if let Some(parent) = _path.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

fn restore_transaction(transaction: &ProviderSwitchTransactionRow) -> AppResult<()> {
    let home = Path::new(&transaction.codex_home);
    let config_path = home.join("config.toml");
    let auth_path = home.join("auth.json");
    if transaction.config_existed {
        let backup = Path::new(&transaction.config_backup_path);
        if !backup.is_file() {
            return Err(AppError::Message(
                "事务中的 config.toml 原始文件已丢失，无法回滚".into(),
            ));
        }
        write_atomic(&config_path, &fs::read(backup)?)?;
    } else if config_path.exists() {
        fs::remove_file(&config_path)?;
        sync_parent(&config_path)?;
    }
    if transaction.auth_existed {
        let backup = Path::new(&transaction.auth_backup_path);
        if !backup.is_file() {
            return Err(AppError::Message(
                "事务中的 auth.json 原始文件已丢失，无法回滚".into(),
            ));
        }
        write_atomic(&auth_path, &fs::read(backup)?)?;
    } else if auth_path.exists() {
        fs::remove_file(&auth_path)?;
        sync_parent(&auth_path)?;
    }
    Ok(())
}

fn read_and_verify(path: &Path, expected: &str, label: &str) -> AppResult<Vec<u8>> {
    let bytes =
        fs::read(path).map_err(|error| AppError::Message(format!("无法读取{label}: {error}")))?;
    if digest(&bytes) != expected {
        return Err(AppError::Message(format!("{label}完整性校验失败")));
    }
    Ok(bytes)
}

fn file_hash(path: &Path) -> Option<String> {
    fs::read(path).ok().map(|bytes| digest(&bytes))
}

fn verify_file_hash(path: &Path, expected: &str, label: &str) -> AppResult<()> {
    let actual = file_hash(path)
        .ok_or_else(|| AppError::Message(format!("{label} 重新读取失败或文件不存在")))?;
    if actual != expected {
        return Err(AppError::Message(format!("{label} 写入后校验失败")));
    }
    Ok(())
}

fn acquire_home_lock(data_dir: &Path, profile_id: &str) -> AppResult<File> {
    let locks = data_dir.join("locks");
    fs::create_dir_all(&locks)?;
    let path = locks.join(format!("{profile_id}.provider.lock"));
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(path)?;
    file.try_lock_exclusive()
        .map_err(|_| AppError::Message("当前存储位置正在处理，请稍后重试".into()))?;
    Ok(file)
}

fn upsert_provider_section(
    content: &str,
    provider_id: &str,
    row: &ProviderConfigRow,
    include_api_key: bool,
) -> AppResult<String> {
    let mut document = parse_document(content)?;
    if document.get("model_providers").is_none() {
        document
            .as_table_mut()
            .insert("model_providers", Item::Table(Table::new()));
    }
    let providers = document["model_providers"]
        .as_table_mut()
        .ok_or_else(|| AppError::Message("config.toml 中 model_providers 不是有效配置表".into()))?;
    if providers.get(provider_id).is_none() {
        providers.insert(provider_id, Item::Table(Table::new()));
    }
    let provider = providers[provider_id].as_table_mut().ok_or_else(|| {
        AppError::Message(format!(
            "config.toml 中 model_providers.{provider_id} 不是有效配置表"
        ))
    })?;
    set_string_value(provider, "name", &row.provider_id);
    set_string_value(provider, "base_url", &row.base_url);
    set_string_value(provider, "wire_api", RESPONSES_WIRE_API);
    set_bool_value(provider, "requires_openai_auth", row.requires_openai_auth);
    if include_api_key {
        set_string_value(provider, "experimental_bearer_token", &row.api_key);
    }
    Ok(document.to_string())
}

fn set_top_level_provider(content: &str, provider_id: &str) -> AppResult<String> {
    let mut document = parse_document(content)?;
    if document
        .get("model_provider")
        .and_then(Item::as_str)
        .is_some_and(|current| current == provider_id)
    {
        return Ok(content.to_string());
    }
    set_string_value(document.as_table_mut(), "model_provider", provider_id);
    Ok(document.to_string())
}

fn set_string_value(table: &mut Table, key: &str, new_value: &str) {
    if table
        .get(key)
        .and_then(Item::as_str)
        .is_some_and(|current| current == new_value)
    {
        return;
    }
    insert_preserving_value_decor(table, key, value(new_value));
}

fn set_bool_value(table: &mut Table, key: &str, new_value: bool) {
    if table
        .get(key)
        .and_then(Item::as_bool)
        .is_some_and(|current| current == new_value)
    {
        return;
    }
    insert_preserving_value_decor(table, key, value(new_value));
}

fn insert_preserving_value_decor(table: &mut Table, key: &str, mut item: Item) {
    let decor = table
        .get(key)
        .and_then(Item::as_value)
        .map(|value| value.decor().clone());
    if let (Some(decor), Some(value)) = (decor, item.as_value_mut()) {
        *value.decor_mut() = decor;
    }
    table.insert(key, item);
}

fn parse_document(content: &str) -> AppResult<DocumentMut> {
    if content.trim().is_empty() {
        return Ok(DocumentMut::new());
    }
    content
        .parse::<DocumentMut>()
        .map_err(|error| AppError::Message(format!("config.toml 解析失败: {error}")))
}

fn mask_key(value: &str) -> String {
    let characters = value.chars().collect::<Vec<_>>();
    if characters.len() <= 8 {
        return "••••".into();
    }
    let prefix = characters.iter().take(4).collect::<String>();
    let suffix = characters.iter().rev().take(4).rev().collect::<String>();
    format!("{prefix}...{suffix}")
}

fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn path_hash(path: &Path) -> String {
    digest(path.to_string_lossy().as_bytes())
}

fn set_snapshot_permissions(directory: &Path, file: &Path) -> AppResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
        fs::set_permissions(file, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(windows)]
    {
        let account = match (std::env::var("USERDOMAIN"), std::env::var("USERNAME")) {
            (Ok(domain), Ok(name)) if !domain.is_empty() && !name.is_empty() => {
                format!("{domain}\\{name}")
            }
            (_, Ok(name)) if !name.is_empty() => name,
            _ => {
                return Err(AppError::Message(
                    "无法确定当前 Windows 用户，不能设置官方快照权限".into(),
                ))
            }
        };
        for target in [directory, file] {
            let status = std::process::Command::new("icacls.exe")
                .arg(target)
                .args(["/inheritance:r", "/grant:r"])
                .arg(format!("{account}:F"))
                .arg("/grant:r")
                .arg("SYSTEM:F")
                .args(["/C", "/Q"])
                .status()?;
            if !status.success() {
                return Err(AppError::Message(format!(
                    "无法设置官方快照权限: {}",
                    target.display()
                )));
            }
        }
    }
    Ok(())
}

fn set_private_directory_permissions(directory: &Path) -> AppResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(windows)]
    {
        let account = match (std::env::var("USERDOMAIN"), std::env::var("USERNAME")) {
            (Ok(domain), Ok(name)) if !domain.is_empty() && !name.is_empty() => {
                format!("{domain}\\{name}")
            }
            (_, Ok(name)) if !name.is_empty() => name,
            _ => {
                return Err(AppError::Message(
                    "无法确定当前 Windows 用户，不能设置事务文件权限".into(),
                ))
            }
        };
        let status = std::process::Command::new("icacls.exe")
            .arg(directory)
            .args(["/inheritance:r", "/grant:r"])
            .arg(format!("{account}:F"))
            .arg("/grant:r")
            .arg("SYSTEM:F")
            .args(["/C", "/Q"])
            .status()?;
        if !status.success() {
            return Err(AppError::Message("无法设置事务文件权限".into()));
        }
    }
    Ok(())
}

fn set_private_file_permissions(file: &Path) -> AppResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(file, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(windows)]
    {
        // Transaction files inherit the already restricted ACL of their
        // per-operation directory. Rewriting the child ACL is unnecessary and
        // can briefly loosen inheritance on some Windows versions.
        let _ = file;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ProfileKind;

    fn test_profile(home: &Path, id: &str, provider_id: &str) -> Profile {
        let timestamp = Utc::now().to_rfc3339();
        Profile {
            id: id.into(),
            name: "Test".into(),
            kind: ProfileKind::ChatGptAccount,
            codex_home: home.to_string_lossy().to_string(),
            provider_id: provider_id.into(),
            app_path: None,
            discovery_source: "test".into(),
            providers: vec![],
            config_profiles: vec![],
            created_at: timestamp.clone(),
            updated_at: timestamp,
        }
    }

    fn test_provider_row(profile_id: &str, provider_id: &str, key: &str) -> ProviderConfigRow {
        let timestamp = Utc::now().to_rfc3339();
        ProviderConfigRow {
            profile_id: profile_id.into(),
            provider_id: provider_id.into(),
            base_url: "https://example.com/v1".into(),
            requires_openai_auth: true,
            api_key: key.into(),
            managed_by_tool: true,
            created_at: timestamp.clone(),
            updated_at: timestamp,
        }
    }

    #[test]
    fn reveals_only_the_requested_provider_key() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path()).unwrap();
        store
            .upsert_provider_config(&test_provider_row("profile", "relay", "sk-secret"))
            .unwrap();

        assert_eq!(reveal_key(&store, "profile", "relay").unwrap(), "sk-secret");
        assert!(reveal_key(&store, "profile", "missing").is_err());
    }

    #[test]
    fn updates_primary_provider_without_removing_unknown_fields() {
        let content = "# keep\nmodel_provider = \"openai\"\n\n[model_providers.foo]\nbase_url = \"https://old\"\nunknown = 1\n\n[mcp]\n";
        let result = upsert_provider_section(
            content,
            "foo",
            &ProviderConfigRow {
                profile_id: "profile".into(),
                provider_id: "foo".into(),
                base_url: "https://new".into(),
                requires_openai_auth: true,
                api_key: "secret".into(),
                managed_by_tool: true,
                created_at: String::new(),
                updated_at: String::new(),
            },
            false,
        )
        .unwrap();
        assert!(result.contains("# keep"));
        assert!(result.contains("unknown = 1"));
        assert!(result.contains("name = \"foo\""));
        assert!(result.contains("base_url = \"https://new\""));
        assert!(result.contains("[mcp]"));
    }

    #[test]
    fn saves_switches_and_restores_official_auth_without_touching_unknown_config() {
        let temp = tempfile::tempdir().unwrap();
        let data_dir = temp.path().join("data");
        let home = temp.path().join("codex-home");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&home).unwrap();
        fs::write(
            home.join("config.toml"),
            "# keep\nmodel_provider = \"openai\"\ncli_auth_credentials_store = \"file\"\n\n[mcp.keep]\ncommand = \"demo\"\n",
        )
        .unwrap();
        let official_auth = br#"{"tokens":{"access_token":"official-token"},"unknown":true}"#;
        fs::write(home.join("auth.json"), official_auth).unwrap();
        let store = Store::open(&data_dir).unwrap();
        let timestamp = Utc::now().to_rfc3339();
        store
            .insert_profile(&Profile {
                id: "profile".into(),
                name: "Test".into(),
                kind: ProfileKind::ChatGptAccount,
                codex_home: home.to_string_lossy().to_string(),
                provider_id: "openai".into(),
                app_path: None,
                discovery_source: "test".into(),
                providers: vec![],
                config_profiles: vec![],
                created_at: timestamp.clone(),
                updated_at: timestamp,
            })
            .unwrap();

        capture_official_snapshot_locked(&data_dir, &store, &store.get_profile("profile").unwrap())
            .unwrap();
        save(
            &data_dir,
            &store,
            ProviderConfigInput {
                profile_id: "profile".into(),
                provider_id: "711EV".into(),
                base_url: None,
                api_key: Some("sk-test-provider-key".into()),
                template: Some("711ev".into()),
            },
        )
        .unwrap();
        let added = fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(added.contains("model_provider = \"openai\""));
        let added_config = added.parse::<toml::Value>().unwrap();
        assert!(added_config["model_providers"].get("711EV").is_some());
        assert!(!added.contains("experimental_bearer_token"));

        switch(&data_dir, &store, "profile", "711EV").unwrap();
        let relay = fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(relay.contains("model_provider = \"711EV\""));
        assert!(relay.contains("experimental_bearer_token = \"sk-test-provider-key\""));
        assert!(relay.contains("[mcp.keep]"));
        let relay_auth: JsonValue =
            serde_json::from_slice(&fs::read(home.join("auth.json")).unwrap()).unwrap();
        assert_eq!(
            relay_auth,
            json!({ "OPENAI_API_KEY": "sk-test-provider-key" })
        );

        switch(&data_dir, &store, "profile", OFFICIAL_ID).unwrap();
        let official = fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(official.contains("model_provider = \"openai\""));
        assert!(official.contains("experimental_bearer_token = \"sk-test-provider-key\""));
        assert_eq!(fs::read(home.join("auth.json")).unwrap(), official_auth);
    }

    #[test]
    fn seven_eleven_template_preserves_explicit_provider_values() {
        let temp = tempfile::tempdir().unwrap();
        let data_dir = temp.path().join("data");
        let home = temp.path().join("codex-home");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&home).unwrap();
        fs::write(
            home.join("config.toml"),
            "model_provider = \"openai\"\ncli_auth_credentials_store = \"file\"\n",
        )
        .unwrap();
        let store = Store::open(&data_dir).unwrap();
        store
            .insert_profile(&test_profile(&home, "profile", OFFICIAL_ID))
            .unwrap();

        let saved = save(
            &data_dir,
            &store,
            ProviderConfigInput {
                profile_id: "profile".into(),
                provider_id: "custom-711".into(),
                base_url: Some("https://custom.example/v1".into()),
                api_key: Some("sk-custom".into()),
                template: Some("711ev".into()),
            },
        )
        .unwrap();

        assert_eq!(saved.provider_id, "custom-711");
        assert_eq!(saved.base_url.as_deref(), Some("https://custom.example/v1"));
    }

    #[test]
    fn editing_current_provider_only_updates_database_until_apply() {
        let temp = tempfile::tempdir().unwrap();
        let data_dir = temp.path().join("data");
        let home = temp.path().join("home");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&home).unwrap();
        let config_before = b"model_provider = \"relay\"\ncli_auth_credentials_store = \"file\"\n\n[model_providers.relay]\nname = \"Old\"\nbase_url = \"https://old.example/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = true\nexperimental_bearer_token = \"sk-old\"\n";
        let auth_before = br#"{"OPENAI_API_KEY":"sk-old"}"#;
        fs::write(home.join("config.toml"), config_before).unwrap();
        fs::write(home.join("auth.json"), auth_before).unwrap();
        let store = Store::open(&data_dir).unwrap();
        store
            .insert_profile(&test_profile(&home, "profile", "relay"))
            .unwrap();
        store
            .upsert_provider_config(&test_provider_row("profile", "relay", "sk-old"))
            .unwrap();

        let saved = save(
            &data_dir,
            &store,
            ProviderConfigInput {
                profile_id: "profile".into(),
                provider_id: "relay".into(),
                base_url: Some("https://new.example/v1".into()),
                api_key: Some("sk-new".into()),
                template: Some("other".into()),
            },
        )
        .unwrap();

        assert_eq!(fs::read(home.join("config.toml")).unwrap(), config_before);
        assert_eq!(fs::read(home.join("auth.json")).unwrap(), auth_before);
        assert!(saved.has_pending_changes);
        assert_eq!(
            store
                .get_provider_config("profile", "relay")
                .unwrap()
                .unwrap()
                .api_key,
            "sk-new"
        );
    }

    #[test]
    fn unsupported_auth_store_never_modifies_codex_files() {
        let temp = tempfile::tempdir().unwrap();
        let data_dir = temp.path().join("data");
        let home = temp.path().join("home");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&home).unwrap();
        let config_before =
            b"model_provider = \"openai\"\ncli_auth_credentials_store = \"keyring\"\n";
        let auth_before = br#"{"tokens":{"access_token":"official"}}"#;
        fs::write(home.join("config.toml"), config_before).unwrap();
        fs::write(home.join("auth.json"), auth_before).unwrap();
        let store = Store::open(&data_dir).unwrap();
        store
            .insert_profile(&test_profile(&home, "profile", "openai"))
            .unwrap();
        store
            .upsert_provider_config(&test_provider_row("profile", "relay", "sk-relay"))
            .unwrap();

        let error = switch(&data_dir, &store, "profile", "relay").unwrap_err();
        assert!(error.to_string().contains("不支持当前认证存储方式"));
        assert_eq!(fs::read(home.join("config.toml")).unwrap(), config_before);
        assert_eq!(fs::read(home.join("auth.json")).unwrap(), auth_before);
    }

    #[test]
    fn corrupted_official_snapshot_blocks_restore_without_modifying_files() {
        let temp = tempfile::tempdir().unwrap();
        let data_dir = temp.path().join("data");
        let home = temp.path().join("home");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&home).unwrap();
        fs::write(
            home.join("config.toml"),
            "model_provider = \"relay\"\ncli_auth_credentials_store = \"file\"\n",
        )
        .unwrap();
        let official = br#"{"tokens":{"refresh_token":"official"}}"#;
        fs::write(home.join("auth.json"), official).unwrap();
        let store = Store::open(&data_dir).unwrap();
        let profile = test_profile(&home, "profile", "relay");
        store.insert_profile(&profile).unwrap();
        capture_official_snapshot_locked(&data_dir, &store, &profile).unwrap();
        let snapshot = store.get_official_snapshot("profile").unwrap().unwrap();
        fs::write(&snapshot.snapshot_path, b"corrupted").unwrap();
        fs::write(home.join("auth.json"), br#"{"OPENAI_API_KEY":"sk-relay"}"#).unwrap();
        let config_before = fs::read(home.join("config.toml")).unwrap();
        let auth_before = fs::read(home.join("auth.json")).unwrap();

        let view = read(&data_dir, &store, "profile", OFFICIAL_ID).unwrap();
        assert_eq!(view.official_auth_snapshot_status, "conflict");
        assert!(!view.can_switch);
        let error = switch(&data_dir, &store, "profile", OFFICIAL_ID).unwrap_err();
        assert!(error.to_string().contains("完整性校验失败"));
        assert_eq!(fs::read(home.join("config.toml")).unwrap(), config_before);
        assert_eq!(fs::read(home.join("auth.json")).unwrap(), auth_before);
    }

    #[test]
    fn recovery_continues_after_config_was_committed() {
        let temp = tempfile::tempdir().unwrap();
        let data_dir = temp.path().join("data");
        let home = temp.path().join("home");
        let transaction_dir = data_dir.join("transactions").join("transaction");
        fs::create_dir_all(&transaction_dir).unwrap();
        fs::create_dir_all(&home).unwrap();
        let config_before = b"model_provider = \"openai\"\n";
        let config_after = b"model_provider = \"relay\"\n";
        let auth_before = br#"{"tokens":{"access_token":"official"}}"#;
        let auth_after = br#"{"OPENAI_API_KEY":"sk-relay"}"#;
        fs::write(home.join("config.toml"), config_after).unwrap();
        fs::write(home.join("auth.json"), auth_before).unwrap();
        let config_backup = transaction_dir.join("config.before");
        let auth_backup = transaction_dir.join("auth.before");
        let config_candidate = transaction_dir.join("config.after");
        let auth_candidate = transaction_dir.join("auth.after");
        fs::write(&config_backup, config_before).unwrap();
        fs::write(&auth_backup, auth_before).unwrap();
        fs::write(&config_candidate, config_after).unwrap();
        fs::write(&auth_candidate, auth_after).unwrap();
        let store = Store::open(&data_dir).unwrap();
        let transaction = ProviderSwitchTransactionRow {
            id: "transaction".into(),
            profile_id: "profile".into(),
            provider_id: "relay".into(),
            codex_home: home.to_string_lossy().to_string(),
            config_backup_path: config_backup.to_string_lossy().to_string(),
            config_existed: true,
            auth_backup_path: auth_backup.to_string_lossy().to_string(),
            auth_existed: true,
            config_candidate_path: config_candidate.to_string_lossy().to_string(),
            auth_candidate_path: auth_candidate.to_string_lossy().to_string(),
            auth_target_exists: true,
            expected_config_sha256: digest(config_after),
            expected_auth_sha256: Some(digest(auth_after)),
            phase: "config_committed".into(),
            created_at: Utc::now().to_rfc3339(),
        };
        store.insert_switch_transaction(&transaction).unwrap();

        recover_transactions(&data_dir, &store).unwrap();

        assert_eq!(fs::read(home.join("config.toml")).unwrap(), config_after);
        assert_eq!(fs::read(home.join("auth.json")).unwrap(), auth_after);
        assert!(store.list_pending_switch_transactions().unwrap().is_empty());
        assert!(!transaction_dir.exists());
    }

    #[test]
    fn failed_rollback_keeps_transaction_for_manual_recovery() {
        let temp = tempfile::tempdir().unwrap();
        let data_dir = temp.path().join("data");
        let home = temp.path().join("home");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&home).unwrap();
        let store = Store::open(&data_dir).unwrap();
        let transaction = ProviderSwitchTransactionRow {
            id: "broken".into(),
            profile_id: "profile".into(),
            provider_id: "relay".into(),
            codex_home: home.to_string_lossy().to_string(),
            config_backup_path: data_dir
                .join("missing-config")
                .to_string_lossy()
                .to_string(),
            config_existed: true,
            auth_backup_path: data_dir.join("missing-auth").to_string_lossy().to_string(),
            auth_existed: true,
            config_candidate_path: String::new(),
            auth_candidate_path: String::new(),
            auth_target_exists: false,
            expected_config_sha256: String::new(),
            expected_auth_sha256: None,
            phase: "prepared".into(),
            created_at: Utc::now().to_rfc3339(),
        };
        store.insert_switch_transaction(&transaction).unwrap();

        assert!(recover_transactions(&data_dir, &store).is_err());
        assert_eq!(store.list_pending_switch_transactions().unwrap().len(), 1);
    }

    #[test]
    fn recovery_never_overwrites_a_third_party_config_change() {
        let temp = tempfile::tempdir().unwrap();
        let data_dir = temp.path().join("data");
        let home = temp.path().join("home");
        let transaction_dir = data_dir.join("transactions").join("external-change");
        fs::create_dir_all(&transaction_dir).unwrap();
        fs::create_dir_all(&home).unwrap();
        let config_before = b"model_provider = \"openai\"\n";
        let config_after = b"model_provider = \"relay\"\n";
        let external_config = b"model_provider = \"other-tool\"\n";
        let auth_before = br#"{"tokens":{"access_token":"official"}}"#;
        fs::write(home.join("config.toml"), external_config).unwrap();
        fs::write(home.join("auth.json"), auth_before).unwrap();
        let config_backup = transaction_dir.join("config.before");
        let auth_backup = transaction_dir.join("auth.before");
        let config_candidate = transaction_dir.join("config.after");
        fs::write(&config_backup, config_before).unwrap();
        fs::write(&auth_backup, auth_before).unwrap();
        fs::write(&config_candidate, config_after).unwrap();
        let store = Store::open(&data_dir).unwrap();
        store
            .insert_switch_transaction(&ProviderSwitchTransactionRow {
                id: "external-change".into(),
                profile_id: "profile".into(),
                provider_id: "relay".into(),
                codex_home: home.to_string_lossy().to_string(),
                config_backup_path: config_backup.to_string_lossy().to_string(),
                config_existed: true,
                auth_backup_path: auth_backup.to_string_lossy().to_string(),
                auth_existed: true,
                config_candidate_path: config_candidate.to_string_lossy().to_string(),
                auth_candidate_path: String::new(),
                auth_target_exists: false,
                expected_config_sha256: digest(config_after),
                expected_auth_sha256: None,
                phase: "prepared".into(),
                created_at: Utc::now().to_rfc3339(),
            })
            .unwrap();

        let error = recover_transactions(&data_dir, &store).unwrap_err();

        assert!(error.to_string().contains("外部文件改动"));
        assert_eq!(fs::read(home.join("config.toml")).unwrap(), external_config);
        assert_eq!(store.list_pending_switch_transactions().unwrap().len(), 1);
    }

    #[test]
    fn auth_events_are_queued_by_home_and_unrelated_files_are_ignored() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let normalized = normalized_path(&home);
        let profiles =
            HashMap::from([(normalized.clone(), test_profile(&home, "profile", "openai"))]);
        let mut pending = HashMap::new();
        let event = Event::new(notify::EventKind::Any)
            .add_path(home.join("config.toml"))
            .add_path(home.join("auth.json"));

        queue_auth_events(event, &profiles, &mut pending);

        assert_eq!(pending.len(), 1);
        assert!(pending.contains_key(&normalized));
    }
}
