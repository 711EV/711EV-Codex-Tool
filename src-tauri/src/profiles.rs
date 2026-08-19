use std::fs;
use std::path::Path;

use chrono::Utc;
use uuid::Uuid;

use crate::discovery::DiscoveredInstance;
use crate::models::Profile;

pub fn from_discovery(instance: DiscoveredInstance) -> Profile {
    let timestamp = Utc::now().to_rfc3339();
    Profile {
        id: Uuid::new_v4().to_string(),
        name: instance.name,
        kind: instance.kind,
        codex_home: instance.home.to_string_lossy().to_string(),
        provider_id: instance.provider_id,
        app_path: instance.app_path,
        discovery_source: instance.discovery_source,
        discovery_state: "active".into(),
        last_seen_at: Some(timestamp.clone()),
        unavailable_reason: None,
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
