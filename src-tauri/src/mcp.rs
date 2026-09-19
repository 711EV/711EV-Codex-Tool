use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::json;
#[cfg(not(windows))]
use tauri::Manager;
use tauri::{AppHandle, State};

use crate::discovery::{validate_codex_home, CodexHomeStatus};
use crate::error::{AppError, AppResult};
use crate::models::Profile;
use crate::provider_config::{acquire_home_lock, write_atomic};
use crate::{mcp_configuration, process, AppContext, Store};

#[cfg(target_os = "windows")]
const WINDOWS_AMD64: &[u8] = include_bytes!("../resources/mcp/image-mcp-windows-amd64.exe");
#[cfg(target_os = "windows")]
const WINDOWS_ARM64: &[u8] = include_bytes!("../resources/mcp/image-mcp-windows-arm64.exe");

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageMcpInstallation {
    pub path: String,
    pub platform: String,
    pub architecture: String,
    pub installed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageMcpRefreshResult {
    #[serde(flatten)]
    pub installation: ImageMcpInstallation,
    pub config_registered: bool,
    pub agents_configured: bool,
    pub eligible: bool,
    pub error: Option<String>,
}

#[derive(Clone, Copy)]
enum Operation {
    Ensure,
    Refresh,
    Repair,
}

#[tauri::command]
pub async fn ensure_image_mcp(
    app: AppHandle,
    context: State<'_, AppContext>,
    profile_id: String,
) -> Result<ImageMcpInstallation, String> {
    run(app, context.data_dir.clone(), profile_id, Operation::Ensure)
        .await?
        .map(|result| result.installation)
        .ok_or_else(|| "图片工具未安装".into())
}

#[tauri::command]
pub async fn refresh_image_mcp_if_installed(
    app: AppHandle,
    context: State<'_, AppContext>,
    profile_id: String,
) -> Result<ImageMcpRefreshResult, String> {
    run(
        app,
        context.data_dir.clone(),
        profile_id,
        Operation::Refresh,
    )
    .await?
    .ok_or_else(|| "无法读取生图状态".into())
}

#[tauri::command]
pub async fn repair_image_mcp(
    app: AppHandle,
    context: State<'_, AppContext>,
    profile_id: String,
) -> Result<ImageMcpInstallation, String> {
    run(app, context.data_dir.clone(), profile_id, Operation::Repair)
        .await?
        .map(|result| result.installation)
        .ok_or_else(|| "图片工具未安装".into())
}

async fn run(
    app: AppHandle,
    data_dir: PathBuf,
    profile_id: String,
    operation: Operation,
) -> Result<Option<ImageMcpRefreshResult>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let store = Store::open(&data_dir)?;
        let profile = store.get_profile(&profile_id)?;
        let home = validated_home(&profile)?;
        let _lock = acquire_home_lock(&data_dir, &profile.id)?;
        let root = runtime_directory(&data_dir, &profile.id)?;
        let target = target_resource()?;
        let destination = root.join(target.destination_name);
        let eligible =
            mcp_configuration::provider_is_eligible(&fs::read(home.join("config.toml"))?)
                .map_err(AppError::Message)?;
        let status = |config_registered, agents_configured| ImageMcpRefreshResult {
            installation: ImageMcpInstallation {
                path: destination.to_string_lossy().into_owned(),
                platform: target.platform.into(),
                architecture: target.architecture.into(),
                installed: false,
            },
            config_registered,
            agents_configured,
            eligible,
            error: None,
        };
        if !eligible {
            if matches!(operation, Operation::Refresh) {
                return Ok(Some(status(false, false)));
            }
            return Err(AppError::Message(
                "生图功能仅支持当前 API 地址包含 ai.711ev.com 的中转供应商".into(),
            ));
        }
        let contents = resource_contents(&app, &target)?;
        let inspection = (|| -> AppResult<(bool, bool, bool)> {
            validate_runtime_paths(&data_dir, &root, &destination)?;
            reject_link(&home.join("AGENTS.md"))?;
            let (registered, agents_configured) =
                mcp_configuration::inspect(&home, &destination).map_err(AppError::Message)?;
            let current = registered
                && agents_configured
                && fs::read(&destination).ok().as_deref() == Some(contents.as_slice())
                && settings_contents(&home)?
                    == fs::read(root.join("settings.json")).unwrap_or_default();
            Ok((registered, agents_configured, current))
        })();
        if matches!(operation, Operation::Refresh) {
            return Ok(Some(match inspection {
                Ok((registered, agents_configured, current)) => ImageMcpRefreshResult {
                    error: (registered && !current).then(|| "生图配置或工具文件需要修复".into()),
                    ..status(registered, agents_configured)
                },
                Err(error) => ImageMcpRefreshResult {
                    error: Some(error.to_string()),
                    ..status(false, false)
                },
            }));
        }
        let (_, _, current) = inspection?;
        if matches!(operation, Operation::Ensure) && current {
            return Ok(Some(status(true, true)));
        }
        let apply = || -> AppResult<ImageMcpRefreshResult> {
            let installed = install_contents(&root, &home, &target, &contents)?;
            mcp_configuration::configure(&home, &destination).map_err(AppError::Message)?;
            Ok(ImageMcpRefreshResult {
                installation: ImageMcpInstallation {
                    path: destination.to_string_lossy().into_owned(),
                    platform: target.platform.into(),
                    architecture: target.architecture.into(),
                    installed,
                },
                config_registered: true,
                agents_configured: true,
                eligible,
                error: None,
            })
        };
        let result = if matches!(operation, Operation::Repair) {
            process::repair_and_restart(profile.app_path.as_deref(), &home, apply)?
        } else {
            apply()?
        };
        Ok(Some(result))
    })
    .await
    .map_err(|error| format!("图片工具任务失败：{error}"))?
    .map_err(String::from)
}

fn validated_home(profile: &Profile) -> AppResult<PathBuf> {
    let home = profile.home_path();
    if profile.discovery_state != "active"
        || !home.is_absolute()
        || validate_codex_home(&home) != CodexHomeStatus::Valid
    {
        return Err(AppError::Message(
            "当前配置目录不可用，请刷新配置目录后重试".into(),
        ));
    }
    reject_link(&home.join("config.toml"))?;
    Ok(home)
}

fn runtime_directory(data_dir: &Path, profile_id: &str) -> AppResult<PathBuf> {
    if profile_id.is_empty()
        || !profile_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(AppError::Message("无效的配置目录标识".into()));
    }
    Ok(data_dir.join("mcp").join(profile_id))
}

fn validate_runtime_paths(data_dir: &Path, root: &Path, destination: &Path) -> AppResult<()> {
    for path in [
        data_dir.join("mcp"),
        root.to_owned(),
        destination.to_owned(),
        root.join("settings.json"),
    ] {
        reject_link(&path)?;
    }
    Ok(())
}

pub(crate) fn reject_link(path: &Path) -> AppResult<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let reparse = false;
    if metadata.file_type().is_symlink() || reparse {
        return Err(AppError::Message(format!(
            "图片工具不支持链接或重解析点：{}",
            path.display()
        )));
    }
    Ok(())
}

struct ResourceTarget {
    platform: &'static str,
    architecture: &'static str,
    resource_name: &'static str,
    destination_name: &'static str,
}

fn resource_target(platform: &str, architecture: &str) -> AppResult<ResourceTarget> {
    let (platform, architecture, resource_name, destination_name) = match (platform, architecture) {
        ("windows", "amd64") => (
            "windows",
            "amd64",
            "image-mcp-windows-amd64.exe",
            "image-mcp.exe",
        ),
        ("windows", "arm64") => (
            "windows",
            "arm64",
            "image-mcp-windows-arm64.exe",
            "image-mcp.exe",
        ),
        ("macos", "amd64") => ("macos", "amd64", "image-mcp-darwin-amd64", "image-mcp"),
        ("macos", "arm64") => ("macos", "arm64", "image-mcp-darwin-arm64", "image-mcp"),
        _ => {
            return Err(AppError::Message(format!(
                "当前系统或架构暂不支持图片工具：{platform}/{architecture}"
            )))
        }
    };
    Ok(ResourceTarget {
        platform,
        architecture,
        resource_name,
        destination_name,
    })
}

fn target_resource() -> AppResult<ResourceTarget> {
    #[cfg(windows)]
    let architecture = {
        let native = std::env::var("PROCESSOR_ARCHITEW6432")
            .or_else(|_| std::env::var("PROCESSOR_ARCHITECTURE"))
            .unwrap_or_else(|_| std::env::consts::ARCH.into())
            .to_ascii_uppercase();
        match native.as_str() {
            "ARM64" | "AARCH64" => "arm64",
            "AMD64" | "X86_64" => "amd64",
            _ => "unsupported",
        }
    };
    #[cfg(not(windows))]
    let architecture = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        _ => "unsupported",
    };
    resource_target(std::env::consts::OS, architecture)
}

fn resource_contents(app: &AppHandle, target: &ResourceTarget) -> AppResult<Vec<u8>> {
    #[cfg(windows)]
    {
        let _ = (app, target.resource_name);
        // Both Windows architectures are embedded for the standalone portable EXE.
        return Ok(if target.architecture == "arm64" {
            WINDOWS_ARM64
        } else {
            WINDOWS_AMD64
        }
        .to_vec());
    }
    #[cfg(not(windows))]
    {
        let source = app
            .path()
            .resource_dir()
            .map_err(|error| AppError::Message(error.to_string()))?
            .join("resources/mcp")
            .join(target.resource_name);
        fs::read(source)
            .map_err(|error| AppError::Message(format!("无法读取图片工具资源：{error}")))
    }
}

fn install_contents(
    root: &Path,
    home: &Path,
    target: &ResourceTarget,
    contents: &[u8],
) -> AppResult<bool> {
    fs::create_dir_all(root)?;
    let destination = root.join(target.destination_name);
    let installed = match fs::read(&destination) {
        Ok(existing) => existing != contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) => return Err(error.into()),
    };
    if installed {
        write_atomic(&destination, contents)
            .map_err(|error| AppError::Message(format!("无法安装图片工具，请修复生图：{error}")))?;
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o700))?;
    }
    let settings = settings_contents(home)?;
    let settings_path = root.join("settings.json");
    if fs::read(&settings_path).ok().as_deref() != Some(settings.as_slice()) {
        write_atomic(&settings_path, &settings)?;
    }
    Ok(installed)
}

fn settings_contents(home: &Path) -> AppResult<Vec<u8>> {
    Ok(serde_json::to_vec_pretty(
        &json!({ "schema_version": 1, "codex_home": home }),
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_copies_keep_directory_settings_separate_and_install_idempotently() {
        let temp = tempfile::tempdir().unwrap();
        let target = resource_target("windows", "amd64").unwrap();
        let first = runtime_directory(temp.path(), "profile-a").unwrap();
        let second = runtime_directory(temp.path(), "profile-b").unwrap();
        let home_a = temp.path().join("home-a");
        let home_b = temp.path().join("home-b");
        assert!(install_contents(&first, &home_a, &target, b"mcp-v1").unwrap());
        assert!(install_contents(&second, &home_b, &target, b"mcp-v1").unwrap());
        assert!(!install_contents(&first, &home_a, &target, b"mcp-v1").unwrap());
        assert!(install_contents(&first, &home_a, &target, b"mcp-v2").unwrap());
        for (root, home) in [(&first, &home_a), (&second, &home_b)] {
            let settings: serde_json::Value =
                serde_json::from_slice(&fs::read(root.join("settings.json")).unwrap()).unwrap();
            assert_eq!(settings["codex_home"], home.to_string_lossy().as_ref());
            assert_eq!(settings["schema_version"], 1);
            assert_eq!(settings.as_object().unwrap().len(), 2);
        }
        assert_eq!(fs::read(second.join("image-mcp.exe")).unwrap(), b"mcp-v1");
        for id in ["", "..", "../outside", "a/b", "a\\b", "C:"] {
            assert!(runtime_directory(temp.path(), id).is_err());
        }
    }

    #[test]
    fn architecture_mapping_never_crosses_operating_systems() {
        for arch in ["amd64", "arm64"] {
            let windows = resource_target("windows", arch).unwrap();
            assert!(windows.resource_name.starts_with("image-mcp-windows-"));
            let mac = resource_target("macos", arch).unwrap();
            assert!(mac.resource_name.starts_with("image-mcp-darwin-"));
        }
        assert!(resource_target("linux", "amd64").is_err());
        assert!(resource_target("windows", "x86").is_err());
    }

    #[test]
    fn mcp_operations_share_the_provider_lock() {
        let temp = tempfile::tempdir().unwrap();
        let lock = acquire_home_lock(temp.path(), "profile").unwrap();
        assert!(acquire_home_lock(temp.path(), "profile").is_err());
        assert!(acquire_home_lock(temp.path(), "other-profile").is_ok());
        drop(lock);
        assert!(acquire_home_lock(temp.path(), "profile").is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn locked_executable_is_not_overwritten_or_redirected() {
        use std::fs::OpenOptions;
        use std::os::windows::fs::OpenOptionsExt;
        let temp = tempfile::tempdir().unwrap();
        let target = resource_target("windows", "amd64").unwrap();
        let home = temp.path().join("home");
        install_contents(temp.path(), &home, &target, b"old").unwrap();
        let destination = temp.path().join("image-mcp.exe");
        let locked = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&destination)
            .unwrap();
        assert!(install_contents(temp.path(), &home, &target, b"new").is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"old");
        drop(locked);
        assert!(install_contents(temp.path(), &home, &target, b"new").unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn embedded_binaries_are_windows_images_for_both_architectures() {
        for (bytes, machine) in [(WINDOWS_AMD64, 0x8664u16), (WINDOWS_ARM64, 0xaa64u16)] {
            assert_eq!(&bytes[..2], b"MZ");
            let pe = u32::from_le_bytes(bytes[0x3c..0x40].try_into().unwrap()) as usize;
            assert_eq!(&bytes[pe..pe + 4], b"PE\0\0");
            assert_eq!(
                u16::from_le_bytes(bytes[pe + 4..pe + 6].try_into().unwrap()),
                machine
            );
        }
    }
}
