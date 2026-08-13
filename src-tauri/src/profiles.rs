use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::models::{Profile, ProfileInput, ProfileMode};

pub fn create(data_dir: &Path, input: ProfileInput) -> AppResult<Profile> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::Message("profile name is required".into()));
    }
    let provider_id = input.provider_id.trim();
    if provider_id.is_empty() || !provider_id.chars().all(valid_provider_character) {
        return Err(AppError::Message(
            "provider id may only contain letters, numbers, dot, dash, and underscore".into(),
        ));
    }

    let id = Uuid::new_v4().to_string();
    let codex_home = match input.mode {
        ProfileMode::Managed => data_dir.join("profiles").join(&id),
        ProfileMode::External => {
            let value = input
                .codex_home
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| AppError::Message("CODEX_HOME is required".into()))?;
            PathBuf::from(value)
        }
    };
    if input.mode == ProfileMode::Managed {
        fs::create_dir_all(codex_home.join("sessions"))?;
        fs::create_dir_all(codex_home.join("archived_sessions"))?;
        let config = codex_home.join("config.toml");
        if !config.exists() {
            fs::write(&config, format!("model_provider = \"{provider_id}\"\n"))?;
        }
    } else if !codex_home.is_dir() {
        return Err(AppError::InvalidPath(format!(
            "external CODEX_HOME does not exist: {}",
            codex_home.display()
        )));
    }

    let timestamp = Utc::now().to_rfc3339();
    Ok(Profile {
        id,
        name: name.to_string(),
        kind: input.kind,
        mode: input.mode,
        codex_home: absolute_display(&codex_home),
        provider_id: provider_id.to_string(),
        app_path: input.app_path.filter(|value| !value.trim().is_empty()),
        created_at: timestamp.clone(),
        updated_at: timestamp,
    })
}

pub fn discover_default() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".codex")))
        .filter(|path| path.is_dir())
}

pub fn read_provider(home: &Path, fallback: &str) -> String {
    let Ok(content) = fs::read_to_string(home.join("config.toml")) else {
        return fallback.to_string();
    };
    let Ok(value) = content.parse::<toml::Value>() else {
        return fallback.to_string();
    };
    value
        .get("model_provider")
        .and_then(toml::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn valid_provider_character(value: char) -> bool {
    value.is_ascii_alphanumeric() || matches!(value, '.' | '-' | '_')
}

fn absolute_display(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}
