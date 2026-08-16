use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};

use crate::models::{DiscoveredConfigProfile, DiscoveredProvider, Profile, ProfileKind};

const MAX_CONFIG_BYTES: u64 = 2 * 1024 * 1024;
const MAX_METADATA_BYTES: u64 = 4 * 1024 * 1024;
const MAX_DIRECTORY_ENTRIES: usize = 512;

#[derive(Debug, Clone)]
pub struct DiscoveredInstance {
    pub home: PathBuf,
    pub name: String,
    pub kind: ProfileKind,
    pub provider_id: String,
    pub discovery_source: String,
    pub providers: Vec<DiscoveredProvider>,
    pub config_profiles: Vec<DiscoveredConfigProfile>,
    pub app_path: Option<String>,
}

#[derive(Debug)]
pub struct DiscoveryScan {
    pub candidates_scanned: usize,
    pub instances: Vec<DiscoveredInstance>,
}

#[derive(Debug, Clone)]
struct Candidate {
    path: PathBuf,
    source: String,
    name_hint: Option<String>,
    app_path: Option<String>,
}

#[derive(Debug, Default)]
struct ConfigMetadata {
    active_provider: String,
    providers: Vec<DiscoveredProvider>,
    config_profiles: Vec<DiscoveredConfigProfile>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstanceStoreFile {
    #[serde(default)]
    instances: Vec<ManagedInstanceReference>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManagedInstanceReference {
    name: Option<String>,
    user_data_dir: String,
}

pub fn discover(known_profiles: &[Profile]) -> DiscoveryScan {
    let mut candidates = Vec::new();
    collect_standard_candidates(&mut candidates);
    collect_process_candidates(&mut candidates);
    collect_managed_metadata_candidates(&mut candidates);
    for profile in known_profiles {
        push_candidate(
            &mut candidates,
            profile.home_path(),
            "已登记实例",
            Some(profile.name.clone()),
            profile.app_path.clone(),
        );
    }

    discover_candidates(candidates)
}

fn collect_standard_candidates(candidates: &mut Vec<Candidate>) {
    if let Some(path) = std::env::var_os("CODEX_HOME").map(PathBuf::from) {
        push_candidate(candidates, path, "CODEX_HOME 环境变量", None, None);
    }

    if let Some(home) = dirs::home_dir() {
        push_candidate(
            candidates,
            home.join(".codex"),
            "Codex 默认目录",
            Some("当前登录账号".into()),
            None,
        );
        for root in known_cockpit_roots(&home) {
            collect_directories(
                &root.join("instances").join("codex"),
                "Cockpit Tools 托管实例",
                candidates,
            );
        }
    }

    if let Some(extra_roots) = std::env::var_os("CODEX_SYNC_DISCOVERY_ROOTS") {
        for root in std::env::split_paths(&extra_roots) {
            collect_directories(&root, "用户配置的发现目录", candidates);
            push_candidate(candidates, root, "用户配置的发现目录", None, None);
        }
    }

    for base in standard_config_bases() {
        for name in [
            ".antigravity_cockpit",
            ".antigravity_cockpit_dev",
            "cockpit-tools",
            "com.jlcodes.cockpit-tools",
        ] {
            collect_directories(
                &base.join(name).join("instances").join("codex"),
                "常见切换工具目录",
                candidates,
            );
        }
    }
}

fn collect_process_candidates(candidates: &mut Vec<Candidate>) {
    let mut system = System::new_with_specifics(
        RefreshKind::nothing().with_processes(ProcessRefreshKind::everything()),
    );
    system.refresh_processes(ProcessesToUpdate::All, true);
    for process in system.processes().values() {
        if process.pid().as_u32() == std::process::id() {
            continue;
        }
        let name = process.name().to_string_lossy().to_ascii_lowercase();
        let stem = name.strip_suffix(".exe").unwrap_or(&name);
        let supported_process = stem == "codex"
            || stem.starts_with("chatgpt")
            || stem == "cockpit-tools"
            || stem == "cockpit tools"
            || stem == "antigravity cockpit";
        if !supported_process {
            continue;
        }
        let app_path = restartable_process_path(stem, process.exe());
        for entry in process.environ() {
            if let Some(path) = environment_value(entry, "CODEX_HOME") {
                push_candidate(
                    candidates,
                    PathBuf::from(path),
                    "运行中的客户端",
                    None,
                    app_path.clone(),
                );
            }
        }
        let command = process
            .cmd()
            .iter()
            .map(|part| part.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        for path in command_line_codex_homes(&command) {
            push_candidate(candidates, path, "运行中的客户端", None, app_path.clone());
        }
    }
}

fn restartable_process_path(process_stem: &str, executable: Option<&Path>) -> Option<String> {
    #[cfg(target_os = "windows")]
    if !process_stem.starts_with("chatgpt") {
        return None;
    }
    executable.map(|path| path.to_string_lossy().to_string())
}

fn collect_managed_metadata_candidates(candidates: &mut Vec<Candidate>) {
    let mut roots = Vec::new();
    if let Some(home) = dirs::home_dir() {
        roots.extend(known_cockpit_roots(&home));
    }
    for key in [
        "COCKPIT_TOOLS_TEST_DATA_DIR",
        "COCKPIT_TEST_DATA_DIR",
        "COCKPIT_TOOLS_DATA_DIR",
    ] {
        if let Some(path) = std::env::var_os(key) {
            roots.push(PathBuf::from(path));
        }
    }
    for base in standard_config_bases() {
        roots.extend([
            base.join(".antigravity_cockpit"),
            base.join(".antigravity_cockpit_dev"),
            base.join("cockpit-tools"),
            base.join("com.jlcodes.cockpit-tools"),
        ]);
    }

    let mut seen = HashSet::new();
    for root in roots {
        let key = normalized_path_key(&root);
        if !seen.insert(key) {
            continue;
        }
        collect_instance_store(&root.join("codex_instances.json"), candidates);
    }
}

fn known_cockpit_roots(home: &Path) -> [PathBuf; 2] {
    [
        home.join(".antigravity_cockpit"),
        home.join(".antigravity_cockpit_dev"),
    ]
}

fn standard_config_bases() -> Vec<PathBuf> {
    let mut values = Vec::new();
    if let Some(path) = dirs::config_dir() {
        values.push(path);
    }
    if let Some(path) = dirs::data_local_dir() {
        values.push(path);
    }
    values
}

fn collect_instance_store(path: &Path, candidates: &mut Vec<Candidate>) {
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    if !metadata.is_file() || metadata.len() > MAX_METADATA_BYTES {
        return;
    }
    let Ok(content) = fs::read_to_string(path) else {
        return;
    };
    let Ok(store) = serde_json::from_str::<InstanceStoreFile>(&content) else {
        return;
    };
    for instance in store.instances {
        if instance.user_data_dir.trim().is_empty() {
            continue;
        }
        push_candidate(
            candidates,
            PathBuf::from(instance.user_data_dir),
            "Cockpit Tools 实例清单",
            instance.name.filter(|name| !name.trim().is_empty()),
            None,
        );
    }
}

fn collect_directories(root: &Path, source: &str, candidates: &mut Vec<Candidate>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten().take(MAX_DIRECTORY_ENTRIES) {
        let path = entry.path();
        if path.is_dir() {
            push_candidate(candidates, path, source, None, None);
        }
    }
}

fn discover_candidates(candidates: Vec<Candidate>) -> DiscoveryScan {
    let candidates = deduplicate_candidates(candidates);
    let initial_count = candidates.len();
    let mut verified = candidates
        .into_iter()
        .filter_map(inspect_candidate)
        .collect::<Vec<_>>();

    let mut sibling_candidates = Vec::new();
    for instance in &verified {
        let Some(parent) = instance.home.parent() else {
            continue;
        };
        if parent.parent().is_none() {
            continue;
        }
        collect_directories(parent, "已发现实例的同级目录", &mut sibling_candidates);
    }

    let existing = verified
        .iter()
        .map(|instance| normalized_path_key(&instance.home))
        .collect::<HashSet<_>>();
    let sibling_candidates = deduplicate_candidates(sibling_candidates);
    let sibling_count = sibling_candidates.len();
    for candidate in sibling_candidates {
        if existing.contains(&normalized_path_key(&candidate.path)) {
            continue;
        }
        if let Some(instance) = inspect_candidate(candidate) {
            verified.push(instance);
        }
    }

    let mut unique = BTreeMap::new();
    for instance in verified {
        unique
            .entry(normalized_path_key(&instance.home))
            .or_insert(instance);
    }
    DiscoveryScan {
        candidates_scanned: initial_count + sibling_count,
        instances: unique.into_values().collect(),
    }
}

fn inspect_candidate(candidate: Candidate) -> Option<DiscoveredInstance> {
    let home = canonical_path(&candidate.path);
    if !is_codex_home(&home) {
        return None;
    }
    let metadata = inspect_config(&home);
    let provider_id = if metadata.active_provider.is_empty() {
        "openai".to_string()
    } else {
        metadata.active_provider.clone()
    };
    let custom_provider = metadata
        .providers
        .iter()
        .any(|provider| provider.id == provider_id && provider.id != "openai");
    let kind = if provider_id != "openai" || custom_provider {
        ProfileKind::CustomApi
    } else {
        ProfileKind::ChatGptAccount
    };
    let name = candidate
        .name_hint
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| {
            home.file_name()
                .and_then(OsStr::to_str)
                .filter(|name| !name.is_empty())
                .map(|name| format!("Codex · {name}"))
                .unwrap_or_else(|| "Codex 本地实例".into())
        });
    Some(DiscoveredInstance {
        home,
        name,
        kind,
        provider_id,
        discovery_source: candidate.source,
        providers: metadata.providers,
        config_profiles: metadata.config_profiles,
        app_path: candidate.app_path,
    })
}

fn is_codex_home(home: &Path) -> bool {
    if !home.is_dir() {
        return false;
    }
    if ["session_index.jsonl", ".codex-global-state.json"]
        .iter()
        .any(|name| home.join(name).is_file())
        || ["sessions", "archived_sessions"]
            .iter()
            .any(|name| home.join(name).is_dir())
    {
        return true;
    }
    contains_state_database(home)
        || contains_state_database(&home.join("sqlite"))
        || is_codex_config(&home.join("config.toml"))
}

fn is_codex_config(path: &Path) -> bool {
    let Some(value) = read_toml(path) else {
        return false;
    };
    [
        "model",
        "model_provider",
        "model_providers",
        "profile",
        "profiles",
        "approval_policy",
        "sandbox_mode",
        "projects",
        "features",
        "mcp_servers",
        "personality",
        "web_search",
    ]
    .iter()
    .any(|key| value.get(*key).is_some())
}

fn contains_state_database(directory: &Path) -> bool {
    let Ok(entries) = fs::read_dir(directory) else {
        return false;
    };
    entries.flatten().take(MAX_DIRECTORY_ENTRIES).any(|entry| {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        entry.path().is_file() && name.starts_with("state_") && name.ends_with(".sqlite")
    })
}

fn inspect_config(home: &Path) -> ConfigMetadata {
    let main_path = home.join("config.toml");
    let main = read_toml(&main_path);
    let selected_profile = main
        .as_ref()
        .and_then(|value| value.get("profile"))
        .and_then(toml::Value::as_str)
        .map(str::to_string);
    let mut active_provider = main
        .as_ref()
        .and_then(|value| value.get("model_provider"))
        .and_then(toml::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("openai")
        .to_string();
    let mut provider_sources = HashMap::<String, String>::new();
    if let Some(table) = main
        .as_ref()
        .and_then(|value| value.get("model_providers"))
        .and_then(toml::Value::as_table)
    {
        for id in table.keys() {
            provider_sources.insert(id.clone(), main_path.to_string_lossy().to_string());
        }
    }
    provider_sources
        .entry(active_provider.clone())
        .or_insert_with(|| main_path.to_string_lossy().to_string());

    let mut config_profiles = Vec::new();
    if let Some(table) = main
        .as_ref()
        .and_then(|value| value.get("profiles"))
        .and_then(toml::Value::as_table)
    {
        for (name, value) in table {
            let provider_id = value
                .get("model_provider")
                .and_then(toml::Value::as_str)
                .map(str::to_string);
            if selected_profile.as_deref() == Some(name.as_str()) {
                if let Some(provider) = &provider_id {
                    active_provider = provider.clone();
                }
            }
            if let Some(provider) = &provider_id {
                provider_sources
                    .entry(provider.clone())
                    .or_insert_with(|| main_path.to_string_lossy().to_string());
            }
            config_profiles.push(DiscoveredConfigProfile {
                name: name.clone(),
                source_file: main_path.to_string_lossy().to_string(),
                provider_id,
                active: selected_profile.as_deref() == Some(name),
            });
        }
    }

    if let Ok(entries) = fs::read_dir(home) {
        for entry in entries.flatten().take(MAX_DIRECTORY_ENTRIES) {
            let path = entry.path();
            let Some(file_name) = path.file_name().and_then(OsStr::to_str) else {
                continue;
            };
            let Some(name) = file_name.strip_suffix(".config.toml") else {
                continue;
            };
            if name.is_empty() || file_name == "config.toml" {
                continue;
            }
            let value = read_toml(&path);
            let provider_id = value
                .as_ref()
                .and_then(|value| value.get("model_provider"))
                .and_then(toml::Value::as_str)
                .map(str::to_string);
            if selected_profile.as_deref() == Some(name) {
                if let Some(provider) = &provider_id {
                    active_provider = provider.clone();
                }
            }
            if let Some(provider) = &provider_id {
                provider_sources
                    .entry(provider.clone())
                    .or_insert_with(|| path.to_string_lossy().to_string());
            }
            config_profiles.push(DiscoveredConfigProfile {
                name: name.to_string(),
                source_file: path.to_string_lossy().to_string(),
                provider_id,
                active: selected_profile.as_deref() == Some(name),
            });
        }
    }

    let mut providers = provider_sources
        .into_iter()
        .map(|(id, source_file)| DiscoveredProvider {
            active: id == active_provider,
            id,
            source_file,
        })
        .collect::<Vec<_>>();
    providers.sort_by(|left, right| left.id.cmp(&right.id));
    config_profiles.sort_by(|left, right| left.name.cmp(&right.name));
    ConfigMetadata {
        active_provider,
        providers,
        config_profiles,
    }
}

fn read_toml(path: &Path) -> Option<toml::Value> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_CONFIG_BYTES {
        return None;
    }
    fs::read_to_string(path).ok()?.parse().ok()
}

fn environment_value(entry: &OsStr, name: &str) -> Option<String> {
    let value = entry.to_string_lossy();
    let (key, value) = value.split_once('=')?;
    (key.eq_ignore_ascii_case(name) && !value.trim().is_empty()).then(|| value.to_string())
}

fn command_line_codex_homes(command: &[String]) -> Vec<PathBuf> {
    let mut values = Vec::new();
    let mut index = 0;
    while index < command.len() {
        let value = command[index].trim_matches('"');
        let lowercase = value.to_ascii_lowercase();
        if let Some(path) = assignment_codex_home(value) {
            values.push(PathBuf::from(path));
        } else if lowercase == "--codex-home" && index + 1 < command.len() {
            index += 1;
            values.push(PathBuf::from(command[index].trim_matches('"')));
        } else if matches!(lowercase.as_str(), "--env" | "-e") && index + 1 < command.len() {
            index += 1;
            let next = command[index].trim_matches('"');
            if let Some(path) = assignment_codex_home(next) {
                values.push(PathBuf::from(path));
            } else if next.eq_ignore_ascii_case("CODEX_HOME") && index + 1 < command.len() {
                index += 1;
                values.push(PathBuf::from(command[index].trim_matches('"')));
            }
        }
        index += 1;
    }
    values
}

fn assignment_codex_home(value: &str) -> Option<String> {
    let (key, path) = value.split_once('=')?;
    let key = key.trim_start_matches("$env:").trim_start_matches("env:");
    (matches!(
        key.to_ascii_lowercase().as_str(),
        "codex_home" | "--codex-home"
    ) && !path.trim_matches('"').is_empty())
    .then(|| path.trim_matches('"').to_string())
}

fn push_candidate(
    candidates: &mut Vec<Candidate>,
    path: PathBuf,
    source: &str,
    name_hint: Option<String>,
    app_path: Option<String>,
) {
    if path.as_os_str().is_empty() {
        return;
    }
    candidates.push(Candidate {
        path,
        source: source.into(),
        name_hint,
        app_path,
    });
}

fn deduplicate_candidates(candidates: Vec<Candidate>) -> Vec<Candidate> {
    let mut values = BTreeMap::new();
    for candidate in candidates {
        values
            .entry(normalized_path_key(&candidate.path))
            .and_modify(|existing: &mut Candidate| {
                if existing.name_hint.is_none() {
                    existing.name_hint = candidate.name_hint.clone();
                }
                if existing.app_path.is_none() {
                    existing.app_path = candidate.app_path.clone();
                }
            })
            .or_insert(candidate);
    }
    values.into_values().collect()
}

pub fn canonical_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

pub fn normalized_path_key(path: &Path) -> String {
    let value = canonical_path(path).to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        value.to_ascii_lowercase()
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "windows")]
    #[test]
    fn cli_process_is_not_saved_as_the_restartable_client() {
        let cli = Path::new("C:/Program Files/WindowsApps/OpenAI.Codex/app/resources/codex.exe");
        let desktop = Path::new("C:/Program Files/WindowsApps/OpenAI.Codex/app/ChatGPT.exe");
        assert_eq!(restartable_process_path("codex", Some(cli)), None);
        assert_eq!(
            restartable_process_path("chatgpt", Some(desktop)),
            Some(desktop.to_string_lossy().to_string())
        );
    }
    use tempfile::TempDir;

    fn codex_home(root: &Path, name: &str, config: &str) -> PathBuf {
        let path = root.join(name);
        fs::create_dir_all(path.join("sessions")).unwrap();
        fs::write(path.join("config.toml"), config).unwrap();
        path
    }

    #[test]
    fn rejects_directories_without_a_codex_fingerprint() {
        let temp = TempDir::new().unwrap();
        fs::write(
            temp.path().join("config.toml"),
            "[build]\ntarget-dir = \"target\"\n",
        )
        .unwrap();
        let scan = discover_candidates(vec![Candidate {
            path: temp.path().to_path_buf(),
            source: "test".into(),
            name_hint: None,
            app_path: None,
        }]);
        assert!(scan.instances.is_empty());
    }

    #[test]
    fn discovers_sibling_homes_and_deduplicates_paths() {
        let temp = TempDir::new().unwrap();
        let first = codex_home(temp.path(), "first", "model_provider = \"openai\"\n");
        let second = codex_home(temp.path(), "second", "model_provider = \"relay\"\n");
        let scan = discover_candidates(vec![
            Candidate {
                path: first.clone(),
                source: "test".into(),
                name_hint: None,
                app_path: None,
            },
            Candidate {
                path: first,
                source: "duplicate".into(),
                name_hint: None,
                app_path: None,
            },
        ]);
        assert_eq!(scan.instances.len(), 2);
        let second_key = normalized_path_key(&second);
        assert!(scan
            .instances
            .iter()
            .any(|value| normalized_path_key(&value.home) == second_key));
    }

    #[test]
    fn reads_provider_and_named_profile_metadata() {
        let temp = TempDir::new().unwrap();
        let home = codex_home(
            temp.path(),
            "profile",
            "model_provider = \"relay\"\nprofile = \"work\"\n[model_providers.relay]\nbase_url = \"https://example.invalid\"\n[model_providers.backup]\nbase_url = \"https://backup.invalid\"\n",
        );
        fs::write(
            home.join("work.config.toml"),
            "model_provider = \"backup\"\n",
        )
        .unwrap();
        let scan = discover_candidates(vec![Candidate {
            path: home,
            source: "test".into(),
            name_hint: None,
            app_path: None,
        }]);
        let instance = &scan.instances[0];
        assert_eq!(instance.provider_id, "backup");
        assert_eq!(instance.providers.len(), 2);
        assert_eq!(instance.config_profiles.len(), 1);
        assert!(instance.config_profiles[0].active);
    }

    #[test]
    fn reads_supported_instance_store_references() {
        let temp = TempDir::new().unwrap();
        let home = codex_home(temp.path(), "managed", "model_provider = \"openai\"\n");
        let metadata = temp.path().join("codex_instances.json");
        fs::write(
            &metadata,
            serde_json::json!({
                "instances": [{"name": "工作账号", "userDataDir": home}]
            })
            .to_string(),
        )
        .unwrap();
        let mut candidates = Vec::new();
        collect_instance_store(&metadata, &mut candidates);
        let scan = discover_candidates(candidates);
        assert_eq!(scan.instances.len(), 1);
        assert_eq!(scan.instances[0].name, "工作账号");
    }

    #[test]
    fn does_not_parse_auth_contents_for_a_valid_home() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("account");
        fs::create_dir_all(home.join("sessions")).unwrap();
        fs::write(home.join("auth.json"), b"not-json-and-never-parsed").unwrap();
        let scan = discover_candidates(vec![Candidate {
            path: home,
            source: "test".into(),
            name_hint: None,
            app_path: None,
        }]);
        assert_eq!(scan.instances.len(), 1);
    }

    #[test]
    fn extracts_common_codex_home_command_line_forms() {
        let values = command_line_codex_homes(&[
            "codex".into(),
            "--codex-home=C:/one".into(),
            "--env".into(),
            "CODEX_HOME".into(),
            "C:/two".into(),
            "$env:CODEX_HOME=C:/three".into(),
        ]);
        assert_eq!(
            values,
            vec![
                PathBuf::from("C:/one"),
                PathBuf::from("C:/two"),
                PathBuf::from("C:/three")
            ]
        );
    }

    #[test]
    fn selected_inline_profile_controls_the_active_provider() {
        let temp = TempDir::new().unwrap();
        let home = codex_home(
            temp.path(),
            "profile",
            "profile = \"work\"\n[model_providers.relay]\nbase_url = \"https://example.invalid\"\n[profiles.work]\nmodel_provider = \"relay\"\n",
        );
        let scan = discover_candidates(vec![Candidate {
            path: home,
            source: "test".into(),
            name_hint: None,
            app_path: None,
        }]);
        assert_eq!(scan.instances[0].provider_id, "relay");
        assert!(scan.instances[0].config_profiles[0].active);
    }
}
