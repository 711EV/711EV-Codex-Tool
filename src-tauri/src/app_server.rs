use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{json, Value};

use crate::error::{AppError, AppResult};

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
const PAGE_SIZE: usize = 100;

#[derive(Debug, Clone, Default)]
pub struct ThreadListFilters {
    pub model_providers: Option<Vec<String>>,
    pub source_kinds: Option<Vec<String>>,
    pub archived: bool,
}

pub struct AppServerClient {
    child: Child,
    stdin: ChildStdin,
    receiver: Receiver<Value>,
    reader: Option<JoinHandle<()>>,
    stderr_reader: Option<JoinHandle<()>>,
    next_request_id: i64,
}

impl AppServerClient {
    pub fn start(codex_home: &Path, explicit_app_path: Option<&str>) -> AppResult<Self> {
        let executables = app_server_candidates(explicit_app_path);
        if executables.is_empty() {
            return Err(AppError::Message(
                "official Codex app-server executable was not found".into(),
            ));
        }
        let mut failures = Vec::new();
        for executable in executables {
            match Self::start_executable(codex_home, &executable) {
                Ok(client) => return Ok(client),
                Err(error) => failures.push(format!("{}: {error}", executable.display())),
            }
        }
        Err(AppError::Message(format!(
            "failed to start an available Codex app-server executable: {}",
            failures.join(" | ")
        )))
    }

    fn start_executable(codex_home: &Path, executable: &Path) -> AppResult<Self> {
        if !executable.is_file() {
            return Err(AppError::Message(
                "official Codex app-server executable was not found".into(),
            ));
        }
        let mut command = Command::new(executable);
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

        let mut child = command
            .spawn()
            .map_err(|error| AppError::Message(format!("failed to start: {error}")))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::Message("missing app-server stdout".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| AppError::Message("missing app-server stderr".into()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AppError::Message("missing app-server stdin".into()))?;
        let (sender, receiver) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(value) = serde_json::from_str::<Value>(&line) {
                    let _ = sender.send(value);
                }
            }
        });
        let stderr_reader = std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                eprintln!("app-server: {line}");
            }
        });
        let mut client = Self {
            child,
            stdin,
            receiver,
            reader: Some(reader),
            stderr_reader: Some(stderr_reader),
            next_request_id: 1,
        };
        client.initialize()?;
        Ok(client)
    }

    fn initialize(&mut self) -> AppResult<()> {
        self.request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "codex-local-sync",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": { "experimentalApi": false }
            }),
        )?;
        self.notify("initialized", json!({}))
    }

    pub fn config_read(&mut self) -> AppResult<Value> {
        self.request("config/read", json!({}))
    }

    pub fn thread_list_all(&mut self, filters: &ThreadListFilters) -> AppResult<Vec<Value>> {
        let mut cursor = None::<String>;
        let mut seen_cursors = HashSet::new();
        let mut threads = Vec::new();
        loop {
            let result = self.request(
                "thread/list",
                json!({
                    "cursor": cursor,
                    "limit": PAGE_SIZE,
                    "sortKey": "updated_at",
                    "sortDirection": "desc",
                    "modelProviders": filters.model_providers,
                    "sourceKinds": filters.source_kinds,
                    "archived": filters.archived,
                    "useStateDbOnly": false
                }),
            )?;
            let next_cursor = append_thread_page(&result, &mut threads)?;
            let Some(next_cursor) = next_cursor else {
                break;
            };
            if !seen_cursors.insert(next_cursor.clone()) {
                return Err(AppError::Message(
                    "app-server returned a repeated thread/list cursor".into(),
                ));
            }
            cursor = Some(next_cursor);
        }
        Ok(threads)
    }

    pub fn thread_read(&mut self, thread_id: &str, include_turns: bool) -> AppResult<Value> {
        self.request(
            "thread/read",
            json!({ "threadId": thread_id, "includeTurns": include_turns }),
        )
    }

    pub fn thread_fork(&mut self, thread_id: &str) -> AppResult<String> {
        let result = self.request("thread/fork", json!({ "threadId": thread_id }))?;
        result
            .pointer("/thread/id")
            .or_else(|| result.get("threadId"))
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .ok_or_else(|| AppError::Message("thread/fork returned no new thread id".into()))
    }

    pub fn thread_name_set(&mut self, thread_id: &str, name: &str) -> AppResult<()> {
        self.request(
            "thread/name/set",
            json!({ "threadId": thread_id, "name": name }),
        )?;
        Ok(())
    }

    pub fn thread_delete(&mut self, thread_id: &str) -> AppResult<()> {
        self.request("thread/delete", json!({ "threadId": thread_id }))?;
        Ok(())
    }

    fn request(&mut self, method: &str, params: Value) -> AppResult<Value> {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        send(
            &mut self.stdin,
            json!({ "method": method, "id": request_id, "params": params }),
        )?;
        loop {
            let value = self
                .receiver
                .recv_timeout(RESPONSE_TIMEOUT)
                .map_err(|error| {
                    AppError::Message(format!(
                        "app-server response unavailable for {method} ({request_id}): {error}"
                    ))
                })?;
            if value.get("id").and_then(Value::as_i64) != Some(request_id) {
                continue;
            }
            if let Some(error) = value.get("error") {
                return Err(AppError::Message(format!(
                    "app-server {method} error: {error}"
                )));
            }
            return value.get("result").cloned().ok_or_else(|| {
                AppError::Message(format!("app-server {method} returned no result"))
            });
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> AppResult<()> {
        send(
            &mut self.stdin,
            json!({ "method": method, "params": params }),
        )
    }
}

impl Drop for AppServerClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
    }
}

pub fn detect(explicit: Option<&str>) -> Option<PathBuf> {
    app_server_candidates(explicit).into_iter().next()
}

fn app_server_candidates(explicit: Option<&str>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = explicit.filter(|value| !value.trim().is_empty()) {
        let path = Path::new(path);
        if path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| !name.eq_ignore_ascii_case("chatgpt.exe"))
        {
            candidates.extend(from_app_path(path));
        }
    }
    if let Some(path) = std::env::var_os("CODEX_APP_SERVER_EXECUTABLE") {
        candidates.push(PathBuf::from(path));
    }

    #[cfg(target_os = "windows")]
    if let Some(root) = std::env::var_os("LOCALAPPDATA") {
        candidates.extend(cached_windows_binaries(
            &PathBuf::from(root).join("OpenAI/Codex/bin"),
        ));
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
        if let Some(path) = explicit.filter(|value| !value.trim().is_empty()) {
            candidates.extend(from_app_path(Path::new(path)));
        }
        for variable in ["LOCALAPPDATA", "ProgramFiles", "ProgramW6432"] {
            if let Some(root) = std::env::var_os(variable) {
                let root = PathBuf::from(root);
                candidates.push(root.join("OpenAI/ChatGPT/resources/codex.exe"));
                candidates.push(root.join("OpenAI/Codex/resources/codex.exe"));
            }
        }
    }

    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|candidate| candidate.is_file())
        .filter(|candidate| !is_protected_windows_apps_path(candidate))
        .filter(|candidate| seen.insert(candidate_key(candidate)))
        .collect()
}

fn candidate_key(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        value.to_ascii_lowercase()
    } else {
        value
    }
}

fn is_protected_windows_apps_path(path: &Path) -> bool {
    cfg!(windows)
        && path.components().any(|component| {
            component
                .as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case("WindowsApps")
        })
}

#[cfg(target_os = "windows")]
fn cached_windows_binaries(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut values = entries
        .flatten()
        .map(|entry| entry.path().join("codex.exe"))
        .filter(|path| path.is_file())
        .map(|path| {
            let modified = fs::metadata(&path).and_then(|value| value.modified()).ok();
            (modified, path)
        })
        .collect::<Vec<_>>();
    values.sort_by(|left, right| right.0.cmp(&left.0));
    values.into_iter().map(|(_, path)| path).collect()
}

fn from_app_path(path: &Path) -> Vec<PathBuf> {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if name.ends_with(".app") {
        vec![path.join("Contents/Resources/codex")]
    } else if name == "chatgpt.exe" {
        path.parent()
            .map(|parent| vec![parent.join("resources/codex.exe")])
            .unwrap_or_default()
    } else {
        vec![path.to_path_buf()]
    }
}

fn append_thread_page(result: &Value, threads: &mut Vec<Value>) -> AppResult<Option<String>> {
    let data = result
        .get("data")
        .or_else(|| result.get("threads"))
        .and_then(Value::as_array)
        .ok_or_else(|| AppError::Message("thread/list returned no data array".into()))?;
    threads.extend(data.iter().cloned());
    Ok(result
        .get("nextCursor")
        .or_else(|| result.get("next_cursor"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string))
}

fn send(stdin: &mut impl Write, value: Value) -> AppResult<()> {
    serde_json::to_writer(&mut *stdin, &value)?;
    stdin.write_all(b"\n")?;
    stdin.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_windows_apps_binaries_are_not_launch_candidates() {
        let protected = Path::new(
            "C:/Program Files/WindowsApps/OpenAI.Codex_1.0.0.0_x64__publisher/app/resources/codex.exe",
        );
        let cached = Path::new("C:/Users/demo/AppData/Local/OpenAI/Codex/bin/version/codex.exe");
        assert_eq!(is_protected_windows_apps_path(protected), cfg!(windows));
        assert!(!is_protected_windows_apps_path(cached));
    }

    #[test]
    fn appends_thread_pages_and_reads_cursor() {
        let mut threads = Vec::new();
        let cursor = append_thread_page(
            &json!({ "data": [{ "id": "one" }], "nextCursor": "page-2" }),
            &mut threads,
        )
        .expect("page");
        assert_eq!(threads.len(), 1);
        assert_eq!(cursor.as_deref(), Some("page-2"));

        let cursor = append_thread_page(
            &json!({ "data": [{ "id": "two" }], "nextCursor": null }),
            &mut threads,
        )
        .expect("page");
        assert_eq!(threads.len(), 2);
        assert!(cursor.is_none());
    }
}
