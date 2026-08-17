use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};

use crate::error::{AppError, AppResult};

#[cfg(target_os = "windows")]
const OFFICIAL_CODEX_APP_ID: &str = "OpenAI.Codex_2p2nqsd0c76g0!App";

pub struct ShutdownOutcome {
    pub closed: bool,
    pub executable: Option<String>,
}

struct MatchingProcess {
    pid: u32,
    executable: Option<String>,
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
    if !is_desktop_client_process(process_stem, executable) {
        return false;
    }
    if target_executable
        .zip(executable)
        .is_some_and(|(target, actual)| executable_paths_match(target, actual))
    {
        return true;
    }

    let haystack = normalize_text(&format!("{command}\n{environment}"));
    let exposes_other_home = environment
        .split('\n')
        .any(|value| value.to_ascii_lowercase().starts_with("codex_home="));
    haystack.contains(target_home) || (default_home && !exposes_other_home)
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

pub fn restart(app_path: Option<&str>, codex_home: &Path) -> AppResult<bool> {
    let Some(app_path) = app_path.map(str::trim).filter(|path| !path.is_empty()) else {
        return Ok(false);
    };
    let path = Path::new(app_path);
    if !path.exists() {
        return Err(AppError::InvalidPath(format!(
            "client application does not exist: {}",
            path.display()
        )));
    }

    #[cfg(target_os = "macos")]
    let mut command = if path.extension().and_then(|value| value.to_str()) == Some("app") {
        let mut command = std::process::Command::new("/usr/bin/open");
        command.arg(path);
        command
    } else {
        std::process::Command::new(path)
    };

    #[cfg(target_os = "windows")]
    let mut command = if is_official_windows_package_path(path) {
        let mut command = std::process::Command::new("explorer.exe");
        command.arg(format!("shell:AppsFolder\\{OFFICIAL_CODEX_APP_ID}"));
        command
    } else {
        std::process::Command::new(path)
    };

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let mut command = std::process::Command::new(path);

    command.env("CODEX_HOME", codex_home).spawn()?;
    #[cfg(target_os = "windows")]
    if !wait_for_client_start(path, Duration::from_secs(10)) {
        return Err(AppError::Message(
            "已发送 Codex Desktop 启动命令，但未检测到客户端进程".into(),
        ));
    }
    Ok(true)
}

#[cfg(target_os = "windows")]
fn wait_for_client_start(app_path: &Path, timeout: Duration) -> bool {
    let expected = app_path.to_string_lossy();
    let started = Instant::now();
    while started.elapsed() < timeout {
        let mut system = System::new_with_specifics(
            RefreshKind::nothing().with_processes(ProcessRefreshKind::everything()),
        );
        system.refresh_processes(ProcessesToUpdate::All, true);
        if system.processes().values().any(|process| {
            let name = process.name().to_string_lossy().to_ascii_lowercase();
            let process_stem = name.strip_suffix(".exe").unwrap_or(&name);
            is_desktop_client_process(
                process_stem,
                process.exe().map(|path| path.to_string_lossy()).as_deref(),
            ) && process
                .exe()
                .is_some_and(|path| executable_paths_match(&expected, &path.to_string_lossy()))
        }) {
            return true;
        }
        thread::sleep(Duration::from_millis(200));
    }
    false
}

#[cfg(target_os = "windows")]
fn is_official_windows_package_path(path: &Path) -> bool {
    let mut has_windows_apps = false;
    let mut has_official_package = false;
    for component in path.components() {
        let value = component.as_os_str().to_string_lossy();
        has_windows_apps |= value.eq_ignore_ascii_case("WindowsApps");
        has_official_package |= value.to_ascii_lowercase().starts_with("openai.codex_");
    }
    has_windows_apps && has_official_package
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
    request_graceful_exit(&pids)?;
    if wait_for_exit(&pids, Duration::from_secs(12)) {
        return Ok(ShutdownOutcome {
            closed: true,
            executable,
        });
    }
    if !force {
        return Err(AppError::Message(
            "target client did not exit normally; confirm force close and retry".into(),
        ));
    }
    force_exit(&pids)?;
    if !wait_for_exit(&pids, Duration::from_secs(5)) {
        return Err(AppError::Message(
            "target client is still running after force close".into(),
        ));
    }
    Ok(ShutdownOutcome {
        closed: true,
        executable,
    })
}

fn wait_for_exit(pids: &[u32], timeout: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < timeout {
        let mut system = System::new();
        system.refresh_processes(ProcessesToUpdate::All, true);
        if pids
            .iter()
            .all(|pid| system.process(Pid::from_u32(*pid)).is_none())
        {
            return true;
        }
        thread::sleep(Duration::from_millis(250));
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
    value
        .replace('/', "\\")
        .to_ascii_lowercase()
        .replace(r"\\?\unc\", r"\\")
        .replace(r"\\?\", "")
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
    fn matches_saved_desktop_path_when_process_does_not_expose_codex_home() {
        let app_path = r"C:\Program Files\WindowsApps\OpenAI.Codex\app\ChatGPT.exe";
        assert!(matches_target_client(
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
    fn normalizes_windows_extended_path_prefixes() {
        assert_eq!(
            normalize_text(r"CODEX_HOME=\\?\F:\Codex"),
            normalize_text(r"CODEX_HOME=F:\Codex"),
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn recognizes_the_official_packaged_client_path() {
        assert!(is_official_windows_package_path(Path::new(
            "C:/Program Files/WindowsApps/OpenAI.Codex_26.810.7004.0_x64__publisher/app/ChatGPT.exe"
        )));
        assert!(is_official_windows_package_path(Path::new(
            "C:/Program Files/WindowsApps/OpenAI.Codex_26.810.7004.0_x64__publisher/app/resources/codex.exe"
        )));
        assert!(!is_official_windows_package_path(Path::new(
            "C:/Users/demo/AppData/Local/OpenAI/Codex/bin/version/codex.exe"
        )));
    }
}
