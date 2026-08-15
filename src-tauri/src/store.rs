use std::path::Path;

use chrono::Utc;
use rusqlite::{params, Connection};

use crate::error::{AppError, AppResult};
use crate::models::{
    DiscoveredConfigProfile, DiscoveredProvider, Profile, ProfileKind, ProfileMode, ReplicaMapping,
};

pub struct Store {
    connection: Connection,
}

impl Store {
    pub fn open(data_dir: &Path) -> AppResult<Self> {
        let connection = Connection::open(data_dir.join("app.sqlite"))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> AppResult<()> {
        self.connection.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS profiles (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                mode TEXT NOT NULL,
                codex_home TEXT NOT NULL UNIQUE,
                provider_id TEXT NOT NULL,
                app_path TEXT,
                discovery_source TEXT NOT NULL DEFAULT '已有实例',
                providers_json TEXT NOT NULL DEFAULT '[]',
                config_profiles_json TEXT NOT NULL DEFAULT '[]',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS provider_thread_replicas (
                id TEXT PRIMARY KEY,
                profile_id TEXT NOT NULL,
                source_thread_id TEXT NOT NULL,
                source_provider_id TEXT NOT NULL,
                target_provider_id TEXT NOT NULL,
                replica_thread_id TEXT NOT NULL,
                source_sha256 TEXT NOT NULL,
                replica_sha256 TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at TEXT NOT NULL,
                verified_at TEXT,
                deleted_at TEXT,
                UNIQUE(profile_id, source_thread_id, target_provider_id),
                UNIQUE(profile_id, replica_thread_id)
            );

            DROP TABLE IF EXISTS sync_jobs;
            DROP TABLE IF EXISTS sync_baselines;
            DROP TABLE IF EXISTS provider_replication_jobs;
            "#,
        )?;
        self.add_column_if_missing(
            "profiles",
            "discovery_source",
            "TEXT NOT NULL DEFAULT '已有实例'",
        )?;
        self.add_column_if_missing("profiles", "providers_json", "TEXT NOT NULL DEFAULT '[]'")?;
        self.add_column_if_missing(
            "profiles",
            "config_profiles_json",
            "TEXT NOT NULL DEFAULT '[]'",
        )?;
        Ok(())
    }

    fn add_column_if_missing(&self, table: &str, column: &str, definition: &str) -> AppResult<()> {
        let mut statement = self
            .connection
            .prepare(&format!("PRAGMA table_info({table})"))?;
        let names = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        if !names.iter().any(|name| name == column) {
            self.connection.execute(
                &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
                [],
            )?;
        }
        Ok(())
    }

    pub fn list_profiles(&self) -> AppResult<Vec<Profile>> {
        let mut statement = self.connection.prepare(
            "SELECT id, name, kind, mode, codex_home, provider_id, app_path,
                    discovery_source, providers_json, config_profiles_json, created_at, updated_at
             FROM profiles ORDER BY created_at ASC",
        )?;
        let rows = statement.query_map([], row_to_profile)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    pub fn get_profile(&self, profile_id: &str) -> AppResult<Profile> {
        self.connection
            .query_row(
                "SELECT id, name, kind, mode, codex_home, provider_id, app_path,
                        discovery_source, providers_json, config_profiles_json, created_at, updated_at
                 FROM profiles WHERE id = ?1",
                [profile_id],
                row_to_profile,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    AppError::Message(format!("profile not found: {profile_id}"))
                }
                other => AppError::Database(other),
            })
    }

    pub fn insert_profile(&self, profile: &Profile) -> AppResult<()> {
        self.connection.execute(
            "INSERT INTO profiles
             (id, name, kind, mode, codex_home, provider_id, app_path, discovery_source,
              providers_json, config_profiles_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                profile.id,
                profile.name,
                kind_text(&profile.kind),
                mode_text(&profile.mode),
                profile.codex_home,
                profile.provider_id,
                profile.app_path,
                profile.discovery_source,
                serde_json::to_string(&profile.providers)?,
                serde_json::to_string(&profile.config_profiles)?,
                profile.created_at,
                profile.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn refresh_discovered_profile(&self, profile: &Profile) -> AppResult<()> {
        self.connection.execute(
            "UPDATE profiles SET kind = ?2, provider_id = ?3,
             app_path = COALESCE(?4, app_path), discovery_source = ?5, providers_json = ?6,
             config_profiles_json = ?7, updated_at = ?8 WHERE id = ?1",
            params![
                profile.id,
                kind_text(&profile.kind),
                profile.provider_id,
                profile.app_path,
                profile.discovery_source,
                serde_json::to_string(&profile.providers)?,
                serde_json::to_string(&profile.config_profiles)?,
                profile.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn delete_profile(&self, profile_id: &str) -> AppResult<()> {
        self.connection
            .execute("DELETE FROM profiles WHERE id = ?1", [profile_id])?;
        Ok(())
    }

    pub fn list_replicas(&self, profile_id: &str) -> AppResult<Vec<ReplicaMapping>> {
        let mut statement = self.connection.prepare(
            "SELECT id, profile_id, source_thread_id, source_provider_id,
                    target_provider_id, replica_thread_id, source_sha256, replica_sha256,
                    status, created_at, verified_at, deleted_at
             FROM provider_thread_replicas
             WHERE profile_id = ?1 ORDER BY created_at DESC",
        )?;
        let rows = statement.query_map([profile_id], row_to_replica)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    pub fn save_replica(&self, mapping: &ReplicaMapping) -> AppResult<()> {
        self.connection.execute(
            "INSERT INTO provider_thread_replicas
             (id, profile_id, source_thread_id, source_provider_id, target_provider_id,
              replica_thread_id, source_sha256, replica_sha256, status, created_at,
              verified_at, deleted_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(profile_id, source_thread_id, target_provider_id) DO UPDATE SET
              replica_thread_id = excluded.replica_thread_id,
              source_sha256 = excluded.source_sha256,
              replica_sha256 = excluded.replica_sha256,
              status = excluded.status,
              verified_at = excluded.verified_at,
              deleted_at = excluded.deleted_at",
            params![
                mapping.id,
                mapping.profile_id,
                mapping.source_thread_id,
                mapping.source_provider_id,
                mapping.target_provider_id,
                mapping.replica_thread_id,
                mapping.source_sha256,
                mapping.replica_sha256,
                mapping.status,
                mapping.created_at,
                mapping.verified_at,
                mapping.deleted_at,
            ],
        )?;
        Ok(())
    }

    pub fn update_replica_hashes(
        &self,
        mapping_id: &str,
        source_sha256: &str,
        replica_sha256: &str,
    ) -> AppResult<()> {
        let updated = self.connection.execute(
            "UPDATE provider_thread_replicas
             SET source_sha256 = ?2, replica_sha256 = ?3, verified_at = ?4
             WHERE id = ?1 AND status = 'verified' AND deleted_at IS NULL",
            params![
                mapping_id,
                source_sha256,
                replica_sha256,
                Utc::now().to_rfc3339()
            ],
        )?;
        if updated != 1 {
            return Err(AppError::Message(
                "replica mapping is no longer available for hash update".into(),
            ));
        }
        Ok(())
    }

    pub fn mark_replica_deleted(&self, mapping_id: &str) -> AppResult<()> {
        let updated = self.connection.execute(
            "UPDATE provider_thread_replicas
             SET status = 'deleted', deleted_at = ?2 WHERE id = ?1",
            params![mapping_id, Utc::now().to_rfc3339()],
        )?;
        if updated != 1 {
            return Err(AppError::Message(
                "replica mapping is no longer available for deletion".into(),
            ));
        }
        Ok(())
    }
}

fn row_to_profile(row: &rusqlite::Row<'_>) -> rusqlite::Result<Profile> {
    let kind: String = row.get(2)?;
    let mode: String = row.get(3)?;
    Ok(Profile {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: match kind.as_str() {
            "custom_api" => ProfileKind::CustomApi,
            _ => ProfileKind::ChatGptAccount,
        },
        mode: match mode.as_str() {
            "managed" => ProfileMode::Managed,
            _ => ProfileMode::External,
        },
        codex_home: row.get(4)?,
        provider_id: row.get(5)?,
        app_path: row.get(6)?,
        discovery_source: row.get(7)?,
        providers: json_column::<DiscoveredProvider>(row, 8),
        config_profiles: json_column::<DiscoveredConfigProfile>(row, 9),
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

fn json_column<T: serde::de::DeserializeOwned>(row: &rusqlite::Row<'_>, index: usize) -> Vec<T> {
    row.get::<_, String>(index)
        .ok()
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default()
}

fn row_to_replica(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReplicaMapping> {
    Ok(ReplicaMapping {
        id: row.get(0)?,
        profile_id: row.get(1)?,
        source_thread_id: row.get(2)?,
        source_provider_id: row.get(3)?,
        target_provider_id: row.get(4)?,
        replica_thread_id: row.get(5)?,
        source_sha256: row.get(6)?,
        replica_sha256: row.get(7)?,
        status: row.get(8)?,
        created_at: row.get(9)?,
        verified_at: row.get(10)?,
        deleted_at: row.get(11)?,
    })
}

fn kind_text(kind: &ProfileKind) -> &'static str {
    match kind {
        ProfileKind::ChatGptAccount => "chat_gpt_account",
        ProfileKind::CustomApi => "custom_api",
    }
}

fn mode_text(mode: &ProfileMode) -> &'static str {
    match mode {
        ProfileMode::External => "external",
        ProfileMode::Managed => "managed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn migrates_existing_profile_database_without_losing_rows() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("app.sqlite");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE profiles (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    mode TEXT NOT NULL,
                    codex_home TEXT NOT NULL UNIQUE,
                    provider_id TEXT NOT NULL,
                    app_path TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                INSERT INTO profiles VALUES (
                    'existing', 'Existing', 'chat_gpt_account', 'external',
                    'C:/codex', 'openai', NULL, 'created', 'updated'
                );
                "#,
            )
            .unwrap();
        drop(connection);

        let store = Store::open(temp.path()).unwrap();
        let profiles = store.list_profiles().unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].id, "existing");
        assert_eq!(profiles[0].discovery_source, "已有实例");
        assert!(profiles[0].providers.is_empty());
    }

    #[test]
    fn removes_legacy_sync_and_job_tables() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("app.sqlite");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE sync_jobs (id TEXT PRIMARY KEY);
                CREATE TABLE sync_baselines (thread_id TEXT PRIMARY KEY);
                CREATE TABLE provider_replication_jobs (id TEXT PRIMARY KEY);
                "#,
            )
            .unwrap();
        drop(connection);

        let store = Store::open(temp.path()).unwrap();
        let remaining: i64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name IN
                 ('sync_jobs', 'sync_baselines', 'provider_replication_jobs')",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(remaining, 0);
    }

    #[test]
    fn updates_both_replica_baseline_hashes_after_in_place_sync() {
        let temp = TempDir::new().unwrap();
        let store = Store::open(temp.path()).unwrap();
        store
            .save_replica(&ReplicaMapping {
                id: "mapping".into(),
                profile_id: "profile".into(),
                source_thread_id: "source".into(),
                source_provider_id: "openai".into(),
                target_provider_id: "relay".into(),
                replica_thread_id: "replica".into(),
                source_sha256: "source-old".into(),
                replica_sha256: "replica-old".into(),
                status: "verified".into(),
                created_at: Utc::now().to_rfc3339(),
                verified_at: Some(Utc::now().to_rfc3339()),
                deleted_at: None,
            })
            .unwrap();

        store
            .update_replica_hashes("mapping", "source-new", "replica-new")
            .unwrap();
        let mapping = store.list_replicas("profile").unwrap().remove(0);

        assert_eq!(mapping.source_sha256, "source-new");
        assert_eq!(mapping.replica_sha256, "replica-new");
    }
}
