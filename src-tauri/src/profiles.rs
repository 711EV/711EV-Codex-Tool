use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use uuid::Uuid;

use crate::discovery::DiscoveredInstance;
use crate::error::{AppError, AppResult};
use crate::models::{DiscoveredProvider, Profile, ProfileInput, ProfileMode};

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
    let discovery_source = if input.mode == ProfileMode::Managed {
        "手动创建的托管实例"
    } else {
        "手动添加"
    };
    Ok(Profile {
        id,
        name: name.to_string(),
        kind: input.kind,
        mode: input.mode,
        codex_home: absolute_display(&codex_home),
        provider_id: provider_id.to_string(),
        app_path: input.app_path.filter(|value| !value.trim().is_empty()),
        discovery_source: discovery_source.into(),
        providers: vec![DiscoveredProvider {
            id: provider_id.to_string(),
            source_file: codex_home.join("config.toml").to_string_lossy().to_string(),
            active: true,
        }],
        config_profiles: Vec::new(),
        created_at: timestamp.clone(),
        updated_at: timestamp,
    })
}

pub fn from_discovery(instance: DiscoveredInstance) -> Profile {
    let timestamp = Utc::now().to_rfc3339();
    Profile {
        id: Uuid::new_v4().to_string(),
        name: instance.name,
        kind: instance.kind,
        mode: ProfileMode::External,
        codex_home: instance.home.to_string_lossy().to_string(),
        provider_id: instance.provider_id,
        app_path: instance.app_path,
        discovery_source: instance.discovery_source,
        providers: instance.providers,
        config_profiles: instance.config_profiles,
        created_at: timestamp.clone(),
        updated_at: timestamp,
    }
}

pub fn read_provider(home: &Path, fallback: &str) -> String {
    let Ok(content) = fs::read_to_string(home.join("config.toml")) else {
        return fallback.to_string();
    };
    let Ok(value) = content.parse::<toml::Value>() else {
        return fallback.to_string();
    };
    let root_provider = value
        .get("model_provider")
        .and_then(toml::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(fallback);
    let Some(selected_profile) = value
        .get("profile")
        .and_then(toml::Value::as_str)
        .filter(|value| !value.trim().is_empty())
    else {
        return root_provider.to_string();
    };
    let inline_provider = value
        .get("profiles")
        .and_then(toml::Value::as_table)
        .and_then(|profiles| profiles.get(selected_profile))
        .and_then(|profile| profile.get("model_provider"))
        .and_then(toml::Value::as_str)
        .filter(|value| !value.trim().is_empty());
    if selected_profile.chars().all(valid_provider_character) {
        let named_path = home.join(format!("{selected_profile}.config.toml"));
        if let Ok(named_content) = fs::read_to_string(named_path) {
            if let Ok(named) = named_content.parse::<toml::Value>() {
                if let Some(provider) = named
                    .get("model_provider")
                    .and_then(toml::Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                {
                    return provider.to_string();
                }
            }
        }
    }
    inline_provider.unwrap_or(root_provider).to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_named_profile_controls_current_provider() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("config.toml"),
            "model_provider = \"openai\"\nprofile = \"work\"\n[profiles.work]\nmodel_provider = \"inline\"\n",
        )
        .unwrap();
        fs::write(
            temp.path().join("work.config.toml"),
            "model_provider = \"relay\"\n",
        )
        .unwrap();
        assert_eq!(read_provider(temp.path(), "fallback"), "relay");
    }
}
