use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};

pub const DATA_DIR_NAME: &str = "CodexLocalSync.data";

pub fn resolve_data_dir() -> AppResult<PathBuf> {
    if let Some(overridden) = std::env::var_os("CODEX_SYNC_DATA_DIR") {
        let path = PathBuf::from(overridden);
        return prepare_with_elevation(path);
    }

    #[cfg(debug_assertions)]
    {
        let project_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .ok_or_else(|| AppError::InvalidPath("cannot resolve project root".into()))?;
        return prepare_with_elevation(project_root.join(DATA_DIR_NAME));
    }

    #[cfg(not(debug_assertions))]
    {
        let executable = std::env::current_exe()?;
        let launcher_dir = launcher_directory(&executable)?;
        prepare_with_elevation(launcher_dir.join(DATA_DIR_NAME))
    }
}

fn prepare_with_elevation(path: PathBuf) -> AppResult<PathBuf> {
    match prepare_data_dir(path.clone()) {
        Ok(path) => Ok(path),
        Err(AppError::Io(error)) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            request_elevated_prepare(&path)?;
            prepare_data_dir(path)
        }
        Err(error) => Err(error),
    }
}

#[cfg(not(debug_assertions))]
fn launcher_directory(executable: &Path) -> AppResult<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        for ancestor in executable.ancestors() {
            if ancestor.extension().and_then(|value| value.to_str()) == Some("app") {
                return ancestor.parent().map(Path::to_path_buf).ok_or_else(|| {
                    AppError::InvalidPath("application bundle has no parent".into())
                });
            }
        }
    }

    executable
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| AppError::InvalidPath("executable has no parent directory".into()))
}

fn prepare_data_dir(path: PathBuf) -> AppResult<PathBuf> {
    fs::create_dir_all(&path)?;

    for child in [
        "profiles",
        "backups",
        "exports",
        "logs",
        "locks",
        "migrations",
    ] {
        fs::create_dir_all(path.join(child))?;
    }

    let probe = path.join(".write-probe");
    fs::write(&probe, b"ok")?;
    fs::remove_file(probe)?;
    Ok(path)
}

pub fn run_elevated_helper() -> bool {
    let mut args = std::env::args();
    let _ = args.next();
    if args.next().as_deref() != Some("--prepare-data-dir") {
        return false;
    }
    let Some(encoded_path) = args.next() else {
        std::process::exit(2);
    };
    let Some(path) = decode_path(&encoded_path) else {
        std::process::exit(2);
    };
    if prepare_data_dir(path.clone()).is_err() {
        std::process::exit(3);
    }

    #[cfg(windows)]
    if let Some(user) = args.next().and_then(|value| decode_utf8(&value)) {
        let status = std::process::Command::new("icacls.exe")
            .arg(&path)
            .arg("/grant:r")
            .arg(format!("{user}:(OI)(CI)M"))
            .args(["/T", "/C", "/Q"])
            .status();
        if !status.is_ok_and(|status| status.success()) {
            std::process::exit(4);
        }
    }

    #[cfg(target_os = "macos")]
    if let Some(owner) = args.next() {
        let status = std::process::Command::new("/usr/sbin/chown")
            .arg("-R")
            .arg(owner)
            .arg(&path)
            .status();
        if !status.is_ok_and(|status| status.success()) {
            std::process::exit(4);
        }
    }
    std::process::exit(0);
}

#[cfg(windows)]
fn request_elevated_prepare(path: &Path) -> AppResult<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, WaitForSingleObject, INFINITE,
    };
    use windows_sys::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };

    let executable = std::env::current_exe()?;
    let user = match (std::env::var("USERDOMAIN"), std::env::var("USERNAME")) {
        (Ok(domain), Ok(name)) if !domain.is_empty() => format!("{domain}\\{name}"),
        (_, Ok(name)) => name,
        _ => {
            return Err(AppError::Message(
                "cannot determine the current Windows user".into(),
            ))
        }
    };
    let parameters = format!(
        "--prepare-data-dir {} {}",
        encode_path(path),
        encode_utf8(&user)
    );
    let verb = wide("runas");
    let file = executable
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let parameters = wide(&parameters);
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = parameters.as_ptr();
    info.nShow = 0;
    if unsafe { ShellExecuteExW(&mut info) } == 0 || info.hProcess.is_null() {
        return Err(AppError::Message(
            "administrator authorization was cancelled or could not be started".into(),
        ));
    }
    let wait = unsafe { WaitForSingleObject(info.hProcess, INFINITE) };
    let mut exit_code = 1u32;
    let got_exit = unsafe { GetExitCodeProcess(info.hProcess, &mut exit_code) } != 0;
    unsafe { CloseHandle(info.hProcess) };
    if wait != WAIT_OBJECT_0 || !got_exit || exit_code != 0 {
        return Err(AppError::Message(
            "administrator authorization did not create a writable portable data directory".into(),
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(Some(0))
        .collect()
}

#[cfg(target_os = "macos")]
fn request_elevated_prepare(path: &Path) -> AppResult<()> {
    let executable = std::env::current_exe()?;
    let owner = format!("{}:{}", unsafe { libc::getuid() }, unsafe {
        libc::getgid()
    });
    let command = format!(
        "{} --prepare-data-dir {} {}",
        shell_quote(&executable.to_string_lossy()),
        encode_path(path),
        owner
    );
    let script = format!(
        "do shell script {} with administrator privileges",
        apple_quote(&command)
    );
    let status = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .status()?;
    if !status.success() {
        return Err(AppError::Message(
            "administrator authorization was cancelled or could not create the portable data directory".into(),
        ));
    }
    Ok(())
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn request_elevated_prepare(_path: &Path) -> AppResult<()> {
    Err(AppError::Message(
        "the portable data directory is not writable".into(),
    ))
}

#[cfg(target_os = "macos")]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(target_os = "macos")]
fn apple_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn encode_path(path: &Path) -> String {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        return path
            .as_os_str()
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .map(|byte| format!("{byte:02x}"))
            .collect();
    }
    #[cfg(not(windows))]
    encode_utf8(&path.to_string_lossy())
}

fn decode_path(value: &str) -> Option<PathBuf> {
    let bytes = decode_hex(value)?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        if bytes.len() % 2 != 0 {
            return None;
        }
        let words = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        return Some(std::ffi::OsString::from_wide(&words).into());
    }
    #[cfg(not(windows))]
    String::from_utf8(bytes).ok().map(PathBuf::from)
}

fn encode_utf8(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn decode_utf8(value: &str) -> Option<String> {
    String::from_utf8(decode_hex(value)?).ok()
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if value.len() % 2 != 0 {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_portable_layout() {
        let temp = tempfile::tempdir().expect("tempdir");
        let data = prepare_data_dir(temp.path().join(DATA_DIR_NAME)).expect("prepare data");
        assert!(data.join("profiles").is_dir());
        assert!(data.join("backups").is_dir());
        assert!(data.join("app.sqlite").parent().is_some());
    }
}
