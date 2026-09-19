use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};

use crate::error::{AppError, AppResult};

#[cfg(target_os = "windows")]
const OFFICIAL_CODEX_APP_ID: &str = "OpenAI.Codex_2p2nqsd0c76g0!App";

const CLIENT_START_TIMEOUT: Duration = Duration::from_secs(10);
const CLIENT_EXIT_TIMEOUT: Duration = Duration::from_secs(5);

pub struct ShutdownOutcome {
    pub closed: bool,
    pub executable: Option<String>,
}

/// Return the explicit CODEX_HOME values exposed by running ChatGPT desktop
/// processes.  CLI/app-server processes are intentionally excluded so a
/// command-line invocation cannot be mistaken for the desktop client.
pub fn running_codex_homes() -> Vec<PathBuf> {
    let system = process_system();
    let mut homes: Vec<PathBuf> = Vec::new();
    for process in system.processes().values() {
        if !is_main_desktop_process(process) || process.exe().is_none() {
            continue;
        }
        let Some(home) = process_codex_home(process) else {
            continue;
        };
        if !homes.iter().any(|current| paths_equal(current, &home)) {
            homes.push(home);
        }
    }
    homes
}

struct MatchingProcess {
    pid: u32,
    executable: Option<String>,
}

#[derive(Debug, Clone)]
enum RestartTarget {
    #[cfg(target_os = "windows")]
    MicrosoftStore {
        package_family_name: String,
        app_user_model_id: String,
        executable: Option<PathBuf>,
    },
    Standalone {
        executable: PathBuf,
        working_directory: Option<PathBuf>,
    },
    #[cfg(target_os = "windows")]
    LegacyPath {
        executable: PathBuf,
        working_directory: Option<PathBuf>,
    },
    #[cfg(target_os = "macos")]
    MacApplication { application: PathBuf },
}

fn matching_processes(codex_home: &Path, app_path: Option<&str>) -> Vec<MatchingProcess> {
    let target = normalize(codex_home);
    let target_executable = app_path.map(normalize_text);
    let default_home = dirs::home_dir()
        .map(|path| path.join(".codex"))
        .is_some_and(|path| normalize(&path) == target);
    let mut system = System::new_with_specifics(
        RefreshKind::nothing().with_processes(ProcessRefreshKind::everything()),
    );
    system.refresh_processes(ProcessesToUpdate::All, true);
    system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            if pid.as_u32() == std::process::id() {
                return None;
            }
            let name = process.name().to_string_lossy().to_ascii_lowercase();
            let process_stem = name.strip_suffix(".exe").unwrap_or(&name);
            let executable = process.exe().map(|path| path.to_string_lossy().to_string());
            let command = process
                .cmd()
                .iter()
                .map(|part| part.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ");
            let environment = process
                .environ()
                .iter()
                .map(|part| part.to_string_lossy())
                .collect::<Vec<_>>()
                .join("\n");
            matches_target_client(
                &target,
                default_home,
                target_executable.as_deref(),
                process_stem,
                executable.as_deref(),
                &command,
                &environment,
            )
            .then(|| MatchingProcess {
                pid: pid.as_u32(),
                executable,
            })
        })
        .collect()
}

fn matches_target_client(
    target_home: &str,
    default_home: bool,
    target_executable: Option<&str>,
    process_stem: &str,
    executable: Option<&str>,
    command: &str,
    environment: &str,
) -> bool {
    if !is_desktop_client_process(process_stem, executable) || is_desktop_auxiliary_process(command)
    {
        return false;
    }
    let haystack = normalize_text(&format!("{command}\n{environment}"));
    let explicit_home = environment.split('\n').find_map(|value| {
        let (name, path) = value.split_once('=')?;
        name.eq_ignore_ascii_case("CODEX_HOME").then_some(path)
    });
    if explicit_home
        .is_some_and(|value| paths_equal(Path::new(value.trim()), Path::new(target_home)))
    {
        return true;
    }
    // A saved executable path is useful only for the default home.  A custom
    // home must be explicit on the process, otherwise another ChatGPT instance
    // could be terminated accidentally.
    target_executable
        .zip(executable)
        .is_some_and(|(target, actual)| {
            default_home && executable_paths_match(target, actual) && explicit_home.is_none()
        })
        || (default_home && explicit_home.is_none() && !haystack.contains("codex_home="))
}

fn executable_paths_match(expected: &str, actual: &str) -> bool {
    let expected = normalize_text(expected);
    let actual = normalize_text(actual);
    actual == expected
        || (expected.ends_with(".app")
            && actual.starts_with(&format!("{}\\contents\\macos\\", expected)))
}

fn is_desktop_client_process(process_stem: &str, executable: Option<&str>) -> bool {
    process_stem == "chatgpt"
        || (process_stem == "codex"
            && executable
                .is_some_and(|path| normalize_text(path).contains(".app\\contents\\macos")))
}

fn is_desktop_auxiliary_process(command: &str) -> bool {
    command
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .any(|argument| {
            argument == "--type"
                || argument.starts_with("--type=")
                || argument == "app-server"
                || argument == "--app-server"
                || argument == "app_server"
        })
}

pub fn restart(app_path: Option<&str>, codex_home: &Path) -> AppResult<bool> {
    let Some(app_path) = app_path.map(str::trim).filter(|path| !path.is_empty()) else {
        return Ok(false);
    };
    let path = Path::new(app_path);
    #[cfg(target_os = "windows")]
    let path_may_be_virtualized = is_windows_store_path(path);
    #[cfg(not(target_os = "windows"))]
    let path_may_be_virtualized = false;
    if !path.exists() && !path_may_be_virtualized {
        return Err(AppError::InvalidPath(format!(
            "ChatGPT application does not exist: {}",
            path.display()
        )));
    }
    if !is_restartable_application_path(path) {
        return Ok(false);
    }

    let Some(target) = target_from_executable(path) else {
        return Ok(false);
    };
    launch(&target, codex_home)?;
    if !wait_for_start(&target, codex_home, CLIENT_START_TIMEOUT) {
        return Err(AppError::Message(
            "已发送 ChatGPT 启动命令，但未检测到 ChatGPT 进程".into(),
        ));
    }
    Ok(true)
}

fn wait_for_start(target: &RestartTarget, codex_home: &Path, timeout: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if !matching_root_pids(target, codex_home).is_empty() {
            return true;
        }
        thread::sleep(Duration::from_millis(250));
    }
    false
}

pub fn ensure_stopped(
    codex_home: &Path,
    app_path: Option<&str>,
    force: bool,
) -> AppResult<ShutdownOutcome> {
    let processes = matching_processes(codex_home, app_path);
    let pids = processes
        .iter()
        .map(|process| process.pid)
        .collect::<Vec<_>>();
    let executable = processes
        .iter()
        .find_map(|process| process.executable.clone());
    if pids.is_empty() {
        return Ok(ShutdownOutcome {
            closed: false,
            executable: None,
        });
    }
    let process_tree_pids = collect_process_tree_pids(&pids);
    request_graceful_exit(&pids)?;
    if wait_for_exit(&pids, Duration::from_secs(12)) {
        return Ok(ShutdownOutcome {
            closed: true,
            executable,
        });
    }
    if !force {
        return Err(AppError::Message(
            "ChatGPT 客户端未正常退出，请确认后强制关闭并重试".into(),
        ));
    }
    force_exit(&process_tree_pids)?;
    if !wait_for_exit(&process_tree_pids, Duration::from_secs(5)) {
        return Err(AppError::Message("ChatGPT 客户端强制关闭后仍在运行".into()));
    }
    Ok(ShutdownOutcome {
        closed: true,
        executable,
    })
}

pub fn force_restart(app_path: Option<&str>, codex_home: &Path) -> AppResult<bool> {
    let Some(target) = resolve_restart_target(app_path, codex_home)? else {
        return Ok(false);
    };
    force_stop(&target, codex_home)?;
    start_resolved_target(&target, codex_home)?;
    Ok(true)
}

fn resolve_restart_target(
    app_path: Option<&str>,
    codex_home: &Path,
) -> AppResult<Option<RestartTarget>> {
    let target = running_target(codex_home).or_else(|| {
        app_path.and_then(|path| {
            let executable = Path::new(path);
            (executable.exists() && is_restartable_application_path(executable))
                .then(|| target_from_executable(executable))
                .flatten()
        })
    });
    let target = match target {
        Some(target) => Some(target),
        None => installed_target()?,
    };
    Ok(target)
}

/// Resolve before shutdown so repair always restarts the same application.
pub fn repair_and_restart<T>(
    app_path: Option<&str>,
    codex_home: &Path,
    repair: impl FnOnce() -> AppResult<T>,
) -> AppResult<T> {
    let target = resolve_restart_target(app_path, codex_home)?
        .ok_or_else(|| AppError::Message("未检测到 ChatGPT 启动路径".into()))?;
    force_stop(&target, codex_home)?;
    let result = repair()?;
    start_resolved_target(&target, codex_home)?;
    Ok(result)
}

fn start_resolved_target(target: &RestartTarget, codex_home: &Path) -> AppResult<()> {
    launch(target, codex_home)?;
    if !wait_for_start(target, codex_home, CLIENT_START_TIMEOUT) {
        return Err(AppError::Message("ChatGPT 启动失败，请手动启动".into()));
    }
    Ok(())
}

fn force_stop(target: &RestartTarget, codex_home: &Path) -> AppResult<()> {
    let root_pids = matching_root_pids(target, codex_home);
    if root_pids.is_empty() {
        return Ok(());
    }

    let process_tree_pids = collect_process_tree_pids(&root_pids);
    force_exit(&process_tree_pids)?;
    if !wait_for_exit(&process_tree_pids, CLIENT_EXIT_TIMEOUT) {
        return Err(AppError::Message(
            "ChatGPT 进程未能完全结束，已停止重启".into(),
        ));
    }
    Ok(())
}

fn running_target(codex_home: &Path) -> Option<RestartTarget> {
    let default_home = is_default_codex_home(codex_home);
    let system = process_system();
    for process in system.processes().values() {
        if !is_main_desktop_process(process) {
            continue;
        }
        let explicit_home = process_codex_home(process);
        if explicit_home
            .as_deref()
            .is_some_and(|value| !paths_equal(value, codex_home))
        {
            continue;
        }
        let Some(executable) = process.exe().map(Path::to_path_buf) else {
            continue;
        };
        let Some(target) = target_from_running_process(process, &executable) else {
            continue;
        };
        if explicit_home.is_some() || default_home {
            return Some(target);
        }
    }
    None
}

fn matching_root_pids(target: &RestartTarget, codex_home: &Path) -> Vec<u32> {
    let default_home = is_default_codex_home(codex_home);
    let system = process_system();
    let mut explicit = Vec::new();
    let mut fallback = Vec::new();
    for (pid, process) in system.processes() {
        if !is_main_desktop_process(process) || !process_matches_target(process, target) {
            continue;
        }
        match process_codex_home(process) {
            Some(home) if paths_equal(&home, codex_home) => explicit.push(pid.as_u32()),
            Some(_) => {}
            None => fallback.push(pid.as_u32()),
        }
    }
    select_root_pids(default_home, explicit, fallback)
}

fn select_root_pids(default_home: bool, explicit: Vec<u32>, fallback: Vec<u32>) -> Vec<u32> {
    if default_home {
        explicit.into_iter().chain(fallback).collect()
    } else {
        explicit
    }
}

fn process_matches_target(process: &sysinfo::Process, target: &RestartTarget) -> bool {
    let Some(executable) = process.exe() else {
        return false;
    };
    match target {
        #[cfg(target_os = "windows")]
        RestartTarget::MicrosoftStore {
            package_family_name,
            executable: expected,
            ..
        } => {
            let path_matches = expected
                .as_deref()
                .is_some_and(|expected| paths_equal(expected, executable));
            match windows_package_family(process.pid().as_u32()) {
                Ok(Some(actual)) => actual.eq_ignore_ascii_case(package_family_name),
                Ok(None) => false,
                // If package identity cannot be queried, retain the executable
                // path captured during discovery as a compatibility fallback.
                Err(_) => path_matches,
            }
        }
        RestartTarget::Standalone {
            executable: expected,
            ..
        } => paths_equal(expected, executable),
        #[cfg(target_os = "windows")]
        RestartTarget::LegacyPath {
            executable: expected,
            ..
        } => paths_equal(expected, executable),
        #[cfg(target_os = "macos")]
        RestartTarget::MacApplication { application } => {
            normalize_text(&executable.to_string_lossy())
                .starts_with(&format!("{}\\contents\\macos\\", normalize(application)))
        }
    }
}

fn target_from_executable(path: &Path) -> Option<RestartTarget> {
    #[cfg(target_os = "windows")]
    {
        let working_directory = path.parent().map(Path::to_path_buf);
        if is_windows_store_path(path) {
            return Some(RestartTarget::LegacyPath {
                executable: path.to_path_buf(),
                working_directory,
            });
        }
        Some(RestartTarget::Standalone {
            executable: path.to_path_buf(),
            working_directory,
        })
    }
    #[cfg(target_os = "macos")]
    {
        let application = path
            .ancestors()
            .find(|ancestor| ancestor.extension().and_then(|value| value.to_str()) == Some("app"))?
            .to_path_buf();
        return Some(RestartTarget::MacApplication { application });
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    Some(RestartTarget::Standalone {
        executable: path.to_path_buf(),
        working_directory: path.parent().map(Path::to_path_buf),
    })
}

fn is_restartable_application_path(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let stem = name.strip_suffix(".exe").unwrap_or(&name);
    stem == "chatgpt"
        || (cfg!(target_os = "macos")
            && ((stem == "codex" && normalize(path).contains(".app\\contents\\macos\\"))
                || path
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value.eq_ignore_ascii_case("app"))))
}

/// Returns whether a persisted application path points to a ChatGPT desktop
/// entry point rather than Codex CLI or the embedded app-server binary.
pub fn is_restartable_client_path(path: &str) -> bool {
    is_restartable_application_path(Path::new(path.trim()))
}

fn target_from_running_process(process: &sysinfo::Process, path: &Path) -> Option<RestartTarget> {
    #[cfg(target_os = "windows")]
    {
        if let Ok(Some(package_family_name)) = windows_package_family(process.pid().as_u32()) {
            if is_openai_chatgpt_package(&package_family_name) {
                return Some(RestartTarget::MicrosoftStore {
                    app_user_model_id: windows_app_user_model_id(&package_family_name),
                    package_family_name,
                    executable: Some(path.to_path_buf()),
                });
            }
        }
    }
    target_from_executable(path)
}

fn launch(target: &RestartTarget, codex_home: &Path) -> AppResult<()> {
    match target {
        #[cfg(target_os = "windows")]
        RestartTarget::MicrosoftStore {
            app_user_model_id, ..
        } => activate_windows_store(app_user_model_id)?,
        RestartTarget::Standalone {
            executable,
            working_directory,
        } => launch_executable(executable, working_directory.as_deref(), codex_home)?,
        #[cfg(target_os = "windows")]
        RestartTarget::LegacyPath {
            executable,
            working_directory,
        } => {
            if is_windows_store_path(executable) {
                let app_user_model_id = detect_windows_store_package()
                    .ok()
                    .flatten()
                    .and_then(|target| match target {
                        RestartTarget::MicrosoftStore {
                            app_user_model_id, ..
                        } => Some(app_user_model_id),
                        _ => None,
                    })
                    .or_else(|| {
                        windows_store_family_from_path(executable)
                            .map(|family| windows_app_user_model_id(&family))
                    })
                    .unwrap_or_else(|| OFFICIAL_CODEX_APP_ID.to_string());
                activate_windows_store(&app_user_model_id)?;
            } else {
                launch_executable(executable, working_directory.as_deref(), codex_home)?;
            }
        }
        #[cfg(target_os = "macos")]
        RestartTarget::MacApplication { application } => {
            let mut command = hidden_command("/usr/bin/open");
            command.arg(application).env("CODEX_HOME", codex_home);
            hide_inherited_stdio(&mut command);
            command.spawn()?;
        }
    }
    Ok(())
}

fn launch_executable(
    executable: &Path,
    working_directory: Option<&Path>,
    codex_home: &Path,
) -> AppResult<()> {
    let mut command = hidden_command(executable);
    if let Some(directory) = working_directory {
        command.current_dir(directory);
    }
    command.env("CODEX_HOME", codex_home);
    hide_inherited_stdio(&mut command);
    command.spawn()?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn installed_target() -> AppResult<Option<RestartTarget>> {
    let store_result = detect_windows_store_package();
    if let Ok(Some(target)) = &store_result {
        return Ok(Some(target.clone()));
    }
    for path in standalone_windows_candidates() {
        if path.is_file() {
            return Ok(Some(RestartTarget::Standalone {
                working_directory: path.parent().map(Path::to_path_buf),
                executable: path,
            }));
        }
    }
    if let Err(error) = store_result {
        return Err(error);
    }
    Ok(None)
}

#[cfg(target_os = "windows")]
fn detect_windows_store_package() -> AppResult<Option<RestartTarget>> {
    use windows::core::HSTRING;
    use windows::Management::Deployment::PackageManager;

    let manager = PackageManager::new()
        .map_err(|_| AppError::Message("无法检查 ChatGPT 安装状态，请重试".into()))?;
    let packages = manager
        .FindPackagesByUserSecurityId(&HSTRING::new())
        .map_err(|_| AppError::Message("无法检查 ChatGPT 安装状态，请重试".into()))?;
    for package in packages {
        let identity = package
            .Id()
            .map_err(|_| AppError::Message("无法检查 ChatGPT 安装状态，请重试".into()))?;
        let name = identity
            .Name()
            .map_err(|_| AppError::Message("无法检查 ChatGPT 安装状态，请重试".into()))?
            .to_string();
        if !is_openai_chatgpt_package(&name) {
            continue;
        }
        let family = identity
            .FamilyName()
            .map_err(|_| AppError::Message("无法检查 ChatGPT 安装状态，请重试".into()))?
            .to_string();
        let entries = package
            .GetAppListEntries()
            .map_err(|_| AppError::Message("无法检查 ChatGPT 安装状态，请重试".into()))?;
        for entry in entries {
            let app_user_model_id = entry
                .AppUserModelId()
                .map_err(|_| AppError::Message("无法检查 ChatGPT 安装状态，请重试".into()))?
                .to_string();
            if !app_user_model_id.trim().is_empty() {
                return Ok(Some(RestartTarget::MicrosoftStore {
                    package_family_name: family,
                    app_user_model_id,
                    executable: None,
                }));
            }
        }
    }
    Ok(None)
}

#[cfg(target_os = "windows")]
fn is_openai_chatgpt_package(name: &str) -> bool {
    let normalized = name.trim().to_ascii_lowercase();
    normalized.starts_with("openai.codex") || normalized.starts_with("openai.chatgpt")
}

#[cfg(target_os = "windows")]
fn standalone_windows_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        candidates.extend([
            local.join("Programs").join("ChatGPT").join("ChatGPT.exe"),
            local.join("Programs").join("OpenAI").join("ChatGPT.exe"),
            local.join("OpenAI").join("ChatGPT.exe"),
        ]);
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates
            .extend(std::env::split_paths(&path).map(|directory| directory.join("ChatGPT.exe")));
    }
    candidates
}

#[cfg(target_os = "macos")]
fn installed_target() -> AppResult<Option<RestartTarget>> {
    let mut candidates = vec![
        PathBuf::from("/Applications/ChatGPT.app"),
        PathBuf::from("/Applications/Codex.app"),
    ];
    if let Some(home) = dirs::home_dir() {
        candidates.extend([
            home.join("Applications").join("ChatGPT.app"),
            home.join("Applications").join("Codex.app"),
        ]);
    }
    Ok(candidates
        .into_iter()
        .find(|path| path.is_dir())
        .map(|application| RestartTarget::MacApplication { application }))
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn installed_target() -> AppResult<Option<RestartTarget>> {
    Ok(None)
}

fn hidden_command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

fn hide_inherited_stdio(command: &mut std::process::Command) -> &mut std::process::Command {
    use std::process::Stdio;

    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
}

#[cfg(target_os = "windows")]
fn activate_windows_store(app_user_model_id: &str) -> AppResult<()> {
    use windows::core::HSTRING;
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_LOCAL_SERVER};
    use windows::Win32::UI::Shell::{
        ApplicationActivationManager, IApplicationActivationManager, AO_NONE,
    };

    let manager: IApplicationActivationManager =
        unsafe { CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_LOCAL_SERVER) }
            .map_err(|_| AppError::Message("ChatGPT 启动失败，请手动启动".into()))?;
    let app_id = HSTRING::from(app_user_model_id);
    unsafe { manager.ActivateApplication(&app_id, &HSTRING::new(), AO_NONE) }
        .map_err(|_| AppError::Message("ChatGPT 启动失败，请手动启动".into()))?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn windows_app_user_model_id(package_family_name: &str) -> String {
    format!("{package_family_name}!App")
}

fn is_main_desktop_process(process: &sysinfo::Process) -> bool {
    let name = process.name().to_string_lossy().to_ascii_lowercase();
    let stem = name.strip_suffix(".exe").unwrap_or(&name);
    let supported = stem == "chatgpt"
        || (cfg!(target_os = "macos")
            && stem == "codex"
            && process.exe().is_some_and(|path| {
                normalize_text(&path.to_string_lossy()).contains(".app\\contents\\macos")
            }));
    supported
        && !process.cmd().iter().any(|argument| {
            let argument = argument.to_string_lossy().to_ascii_lowercase();
            argument == "--type"
                || argument.starts_with("--type=")
                || argument == "app-server"
                || argument == "--app-server"
                || argument == "app_server"
        })
}

fn process_codex_home(process: &sysinfo::Process) -> Option<PathBuf> {
    process.environ().iter().find_map(|entry| {
        let entry = entry.to_string_lossy();
        let (name, value) = entry.split_once('=')?;
        name.eq_ignore_ascii_case("CODEX_HOME")
            .then(|| PathBuf::from(value.trim()))
            .filter(|path| !path.as_os_str().is_empty())
    })
}

fn process_system() -> System {
    let mut system = System::new_with_specifics(
        RefreshKind::nothing().with_processes(ProcessRefreshKind::everything()),
    );
    system.refresh_processes(ProcessesToUpdate::All, true);
    system
}

fn is_default_codex_home(path: &Path) -> bool {
    dirs::home_dir()
        .map(|home| paths_equal(&home.join(".codex"), path))
        .unwrap_or(false)
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    normalize(left) == normalize(right)
}

#[cfg(target_os = "windows")]
fn is_windows_store_path(path: &Path) -> bool {
    normalize(path).contains("\\windowsapps\\openai.")
}

#[cfg(target_os = "windows")]
fn windows_store_family_from_path(path: &Path) -> Option<String> {
    path.components().find_map(|component| {
        let value = component.as_os_str().to_string_lossy();
        let normalized = value.to_ascii_lowercase();
        (normalized.starts_with("openai.codex_") || normalized.starts_with("openai.chatgpt_"))
            .then(|| value.to_string())
    })
}

#[cfg(target_os = "windows")]
fn windows_package_family(pid: u32) -> AppResult<Option<String>> {
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    const ERROR_INSUFFICIENT_BUFFER: i32 = 122;
    const APPMODEL_ERROR_NO_PACKAGE: i32 = 15_700;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetPackageFamilyName(
            process: *mut c_void,
            package_family_name_length: *mut u32,
            package_family_name: *mut u16,
        ) -> i32;
    }

    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut length = 0u32;
    let first = unsafe { GetPackageFamilyName(process, &mut length, std::ptr::null_mut()) };
    if first == APPMODEL_ERROR_NO_PACKAGE {
        unsafe { CloseHandle(process) };
        return Ok(None);
    }
    if first != ERROR_INSUFFICIENT_BUFFER || length == 0 {
        unsafe { CloseHandle(process) };
        return Err(AppError::Message("无法读取 ChatGPT 包身份".into()));
    }
    let mut buffer = vec![0u16; length as usize];
    let result = unsafe { GetPackageFamilyName(process, &mut length, buffer.as_mut_ptr()) };
    unsafe { CloseHandle(process) };
    if result != 0 {
        return Err(AppError::Message("无法读取 ChatGPT 包身份".into()));
    }
    if buffer.last() == Some(&0) {
        buffer.pop();
    }
    String::from_utf16(&buffer)
        .map(Some)
        .map_err(|_| AppError::Message("ChatGPT 包身份格式无效".into()))
}

fn collect_process_tree_pids(root_pids: &[u32]) -> Vec<u32> {
    let system = process_system();
    let mut depths = root_pids
        .iter()
        .copied()
        .map(|pid| (Pid::from_u32(pid), 0usize))
        .collect::<HashMap<_, _>>();

    loop {
        let mut changed = false;
        for (pid, process) in system.processes() {
            let Some(parent) = process.parent() else {
                continue;
            };
            let Some(parent_depth) = depths.get(&parent).copied() else {
                continue;
            };
            if depths.insert(*pid, parent_depth + 1).is_none() {
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let mut pids = depths
        .into_iter()
        .map(|(pid, depth)| (pid.as_u32(), depth))
        .collect::<Vec<_>>();
    pids.sort_by(|left, right| right.1.cmp(&left.1).then(right.0.cmp(&left.0)));
    pids.into_iter().map(|(pid, _)| pid).collect()
}

fn wait_for_exit(pids: &[u32], timeout: Duration) -> bool {
    let started = Instant::now();
    let targets = pids.iter().copied().collect::<HashSet<_>>();
    while started.elapsed() < timeout {
        let system = process_system();
        if targets
            .iter()
            .all(|pid| system.process(Pid::from_u32(*pid)).is_none())
        {
            return true;
        }
        thread::sleep(Duration::from_millis(200));
    }
    false
}

#[cfg(unix)]
fn request_graceful_exit(pids: &[u32]) -> AppResult<()> {
    for pid in pids {
        let result = unsafe { libc::kill(*pid as i32, libc::SIGTERM) };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error.into());
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn request_graceful_exit(pids: &[u32]) -> AppResult<()> {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, PostMessageW, WM_CLOSE,
    };

    struct Context<'a> {
        pids: &'a [u32],
    }

    unsafe extern "system" fn callback(window: HWND, lparam: LPARAM) -> BOOL {
        let context = &*(lparam as *const Context<'_>);
        let mut pid = 0u32;
        GetWindowThreadProcessId(window, &mut pid);
        if context.pids.contains(&pid) {
            let _ = PostMessageW(window, WM_CLOSE, 0, 0);
        }
        1
    }

    let context = Context { pids };
    unsafe {
        EnumWindows(Some(callback), &context as *const Context<'_> as LPARAM);
    }
    Ok(())
}

#[cfg(unix)]
fn force_exit(pids: &[u32]) -> AppResult<()> {
    for pid in pids {
        let result = unsafe { libc::kill(*pid as i32, libc::SIGKILL) };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error.into());
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn force_exit(pids: &[u32]) -> AppResult<()> {
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    for pid in pids {
        if let Some(process) = system.process(Pid::from_u32(*pid)) {
            let _ = process.kill();
        }
    }
    Ok(())
}

fn normalize(path: &Path) -> String {
    normalize_text(&path.to_string_lossy())
}

fn normalize_text(value: &str) -> String {
    let normalized = value
        .replace('/', "\\")
        .to_ascii_lowercase()
        .replace(r"\\?\unc\", r"\\")
        .replace(r"\\?\", "");
    normalized.trim_end_matches('\\').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excludes_codex_cli_from_desktop_processes() {
        assert!(!is_desktop_client_process(
            "codex",
            Some(r"C:\Users\tester\AppData\Roaming\npm\node_modules\@openai\codex\bin\codex.exe")
        ));
        assert!(!is_desktop_client_process("codex", None));
        assert!(!is_restartable_application_path(Path::new(
            r"C:\Users\tester\AppData\Roaming\npm\codex.exe",
        )));
        assert!(is_restartable_client_path(
            r"C:\Program Files\OpenAI\ChatGPT\ChatGPT.exe"
        ));
    }

    #[test]
    fn recognizes_desktop_processes_on_windows_and_macos() {
        assert!(is_desktop_client_process(
            "chatgpt",
            Some(r"C:\Program Files\WindowsApps\OpenAI.Codex\ChatGPT.exe")
        ));
        assert!(is_desktop_client_process(
            "codex",
            Some("/Applications/Codex.app/Contents/MacOS/Codex")
        ));
    }

    #[test]
    fn custom_home_does_not_match_saved_desktop_path_without_explicit_environment() {
        let app_path = r"C:\Program Files\WindowsApps\OpenAI.Codex\app\ChatGPT.exe";
        assert!(!matches_target_client(
            r"f:\codex",
            false,
            Some(app_path),
            "chatgpt",
            Some(app_path),
            "ChatGPT.exe",
            "",
        ));
    }

    #[test]
    fn explicit_home_matches_saved_desktop_path_for_custom_home() {
        let app_path = r"C:\Program Files\WindowsApps\OpenAI.Codex\app\ChatGPT.exe";
        assert!(matches_target_client(
            r"f:\codex",
            false,
            Some(app_path),
            "chatgpt",
            Some(app_path),
            "ChatGPT.exe",
            r"CODEX_HOME=F:\codex",
        ));
    }

    #[test]
    fn excludes_chromium_auxiliary_processes_from_exit_waiting() {
        let app_path = r"C:\Program Files\WindowsApps\OpenAI.Codex\app\ChatGPT.exe";
        assert!(!matches_target_client(
            r"f:\codex",
            true,
            Some(app_path),
            "chatgpt",
            Some(app_path),
            &format!(r#""{app_path}" --type=renderer --renderer-client-id=3"#),
            "",
        ));
        assert!(!matches_target_client(
            r"f:\codex",
            true,
            Some(app_path),
            "chatgpt",
            Some(app_path),
            &format!(r#""{app_path}" --type=crashpad-handler"#),
            "",
        ));
        assert!(matches_target_client(
            r"f:\codex",
            true,
            Some(app_path),
            "chatgpt",
            Some(app_path),
            &format!(r#""{app_path}""#),
            "",
        ));
    }

    #[test]
    fn excludes_app_server_processes_from_desktop_matching() {
        let app_path = r"C:\Program Files\WindowsApps\OpenAI.Codex\app\ChatGPT.exe";
        assert!(!matches_target_client(
            r"f:\codex",
            true,
            Some(app_path),
            "chatgpt",
            Some(app_path),
            &format!(r#""{app_path}" app-server --listen stdio://"#),
            "",
        ));
    }

    #[test]
    fn normalizes_windows_extended_path_prefixes() {
        assert_eq!(
            normalize_text(r"CODEX_HOME=\\?\F:\Codex"),
            normalize_text(r"CODEX_HOME=F:\Codex"),
        );
    }

    #[test]
    fn explicit_codex_home_matches_take_priority() {
        assert_eq!(
            select_root_pids(false, vec![11, 12], vec![21]),
            vec![11, 12]
        );
    }

    #[test]
    fn custom_codex_home_rejects_ambiguous_unlabeled_instances() {
        assert!(select_root_pids(false, Vec::new(), vec![21, 22]).is_empty());
        assert!(select_root_pids(false, Vec::new(), vec![21]).is_empty());
    }

    #[test]
    fn default_codex_home_accepts_unlabeled_matching_instances() {
        assert_eq!(
            select_root_pids(true, Vec::new(), vec![21, 22]),
            vec![21, 22],
        );
        assert_eq!(select_root_pids(true, vec![11], vec![21]), vec![11, 21]);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn classifies_saved_windows_paths_as_legacy_or_standalone_targets() {
        let legacy_path = Path::new(
            "C:/Program Files/WindowsApps/OpenAI.Codex_1.0_x64__publisher/app/ChatGPT.exe",
        );
        let standalone_path = Path::new("C:/Users/demo/AppData/Local/Programs/ChatGPT/ChatGPT.exe");

        match target_from_executable(legacy_path) {
            Some(RestartTarget::LegacyPath {
                executable,
                working_directory,
            }) => {
                assert_eq!(executable, legacy_path);
                assert_eq!(working_directory.as_deref(), legacy_path.parent());
            }
            target => panic!("unexpected legacy target: {target:?}"),
        }
        match target_from_executable(standalone_path) {
            Some(RestartTarget::Standalone {
                executable,
                working_directory,
            }) => {
                assert_eq!(executable, standalone_path);
                assert_eq!(working_directory.as_deref(), standalone_path.parent());
            }
            target => panic!("unexpected standalone target: {target:?}"),
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn builds_the_store_app_user_model_id_from_the_detected_package_family() {
        assert_eq!(
            windows_app_user_model_id("OpenAI.Codex_2p2nqsd0c76g0"),
            OFFICIAL_CODEX_APP_ID,
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn recognizes_supported_openai_chatgpt_package_names() {
        assert!(is_openai_chatgpt_package("OpenAI.Codex"));
        assert!(is_openai_chatgpt_package("OpenAI.ChatGPT-Desktop"));
        assert!(!is_openai_chatgpt_package("Example.ChatGPT"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn derives_store_family_from_saved_chatgpt_path() {
        let path = Path::new(
            r"C:\Program Files\WindowsApps\OpenAI.ChatGPT_1.2.3.0_x64__publisher\app\ChatGPT.exe",
        );
        assert_eq!(
            windows_store_family_from_path(path).as_deref(),
            Some("OpenAI.ChatGPT_1.2.3.0_x64__publisher"),
        );
    }
}
