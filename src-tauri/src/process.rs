use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};

use crate::error::{AppError, AppResult};

pub struct ShutdownOutcome {
    pub closed: bool,
    pub executable: Option<String>,
}

struct MatchingProcess {
    pid: u32,
    executable: Option<String>,
}

fn matching_processes(codex_home: &Path) -> Vec<MatchingProcess> {
    let target = normalize(codex_home);
    let default_home =
        crate::profiles::discover_default().is_some_and(|path| normalize(&path) == target);
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
            let is_desktop_client = process_stem == "chatgpt"
                || (process_stem == "codex"
                    && executable.as_deref().is_some_and(|path| {
                        normalize_text(path).contains(".app\\contents\\macos")
                    }));
            if !is_desktop_client {
                return None;
            }
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
            let haystack = normalize_text(&format!("{command}\n{environment}"));
            let exposes_other_home = environment
                .split('\n')
                .any(|value| value.to_ascii_lowercase().starts_with("codex_home="));
            (haystack.contains(&target) || (default_home && !exposes_other_home)).then(|| {
                MatchingProcess {
                    pid: pid.as_u32(),
                    executable,
                }
            })
        })
        .collect()
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

    #[cfg(not(target_os = "macos"))]
    let mut command = std::process::Command::new(path);

    command.env("CODEX_HOME", codex_home).spawn()?;
    Ok(true)
}

pub fn ensure_stopped(codex_home: &Path, force: bool) -> AppResult<ShutdownOutcome> {
    let processes = matching_processes(codex_home);
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
    value.replace('/', "\\").to_ascii_lowercase()
}
