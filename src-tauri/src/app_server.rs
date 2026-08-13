use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::error::{AppError, AppResult};

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(20);

pub fn detect(explicit: Option<&str>) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = explicit.filter(|value| !value.trim().is_empty()) {
        candidates.extend(from_app_path(Path::new(path)));
    }
    if let Some(path) = std::env::var_os("CODEX_APP_SERVER_EXECUTABLE") {
        candidates.push(PathBuf::from(path));
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&paths) {
            #[cfg(windows)]
            candidates.push(directory.join("codex.exe"));
            #[cfg(not(windows))]
            candidates.push(directory.join("codex"));
        }
    }

    #[cfg(target_os = "macos")]
    candidates.extend([
        PathBuf::from("/Applications/ChatGPT.app/Contents/Resources/codex"),
        PathBuf::from("/Applications/Codex.app/Contents/Resources/codex"),
    ]);

    #[cfg(target_os = "windows")]
    {
        for variable in ["LOCALAPPDATA", "ProgramFiles", "ProgramW6432"] {
            if let Some(root) = std::env::var_os(variable) {
                let root = PathBuf::from(root);
                candidates.push(root.join("OpenAI/ChatGPT/resources/codex.exe"));
                candidates.push(root.join("OpenAI/Codex/resources/codex.exe"));
            }
        }
    }

    candidates.into_iter().find(|candidate| candidate.is_file())
}

fn from_app_path(path: &Path) -> Vec<PathBuf> {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if name.ends_with(".app") {
        vec![path.join("Contents/Resources/codex")]
    } else if name == "chatgpt.exe" || name == "codex.exe" {
        if let Some(parent) = path.parent() {
            let bundled = parent.join("resources/codex.exe");
            if bundled.is_file() {
                return vec![bundled];
            }
        }
        vec![path.to_path_buf()]
    } else {
        vec![path.to_path_buf()]
    }
}

pub fn rebuild_index(codex_home: &Path, explicit_app_path: Option<&str>) -> AppResult<()> {
    let executable = detect(explicit_app_path).ok_or_else(|| {
        AppError::Message("official Codex app-server executable was not found".into())
    })?;
    let mut command = Command::new(&executable);
    command
        .args(["app-server", "--listen", "stdio://"])
        .env("CODEX_HOME", codex_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }

    let mut child = command.spawn().map_err(|error| {
        AppError::Message(format!("failed to start {}: {error}", executable.display()))
    })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::Message("missing app-server stdout".into()))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| AppError::Message("missing app-server stdin".into()))?;
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = sender.send(line);
        }
    });

    let result = (|| {
        send(
            &mut stdin,
            json!({
                "method": "initialize",
                "id": 1,
                "params": { "clientInfo": { "name": "codex-local-sync", "version": env!("CARGO_PKG_VERSION") } }
            }),
        )?;
        wait(&receiver, 1)?;
        send(&mut stdin, json!({ "method": "initialized", "params": {} }))?;
        send(
            &mut stdin,
            json!({
                "method": "thread/list",
                "id": 2,
                "params": {
                    "cursor": null,
                    "limit": 1,
                    "sortKey": "updated_at",
                    "sortDirection": "desc",
                    "modelProviders": null,
                    "sourceKinds": [],
                    "archived": false
                }
            }),
        )?;
        wait(&receiver, 2)
    })();

    let _ = child.kill();
    let _ = child.wait();
    let _ = reader.join();
    result
}

fn send(stdin: &mut impl Write, value: Value) -> AppResult<()> {
    serde_json::to_writer(&mut *stdin, &value)?;
    stdin.write_all(b"\n")?;
    stdin.flush()?;
    Ok(())
}

fn wait(receiver: &mpsc::Receiver<String>, id: i64) -> AppResult<()> {
    loop {
        let line = receiver.recv_timeout(RESPONSE_TIMEOUT).map_err(|_| {
            AppError::Message(format!("app-server response timed out for request {id}"))
        })?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("id").and_then(Value::as_i64) != Some(id) {
            continue;
        }
        if let Some(error) = value.get("error") {
            return Err(AppError::Message(format!("app-server error: {error}")));
        }
        return Ok(());
    }
}
