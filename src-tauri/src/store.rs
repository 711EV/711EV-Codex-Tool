use std::path::Path;

use chrono::Utc;
use rusqlite::{params, Connection};

use crate::error::{AppError, AppResult};
use crate::models::{
    DiscoveredConfigProfile, DiscoveredProvider, Profile, ProfileKind, ReplicaMapping,
};

#[derive(Debug, Clone)]
pub struct ProviderConfigRow {
    pub profile_id: String,
    pub provider_id: String,
    pub base_url: String,
    pub requires_openai_auth: bool,
    pub api_key: String,
    pub managed_by_tool: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct OfficialSnapshotRow {
    pub profile_id: String,
    pub snapshot_path: String,
    pub source_sha256: String,
    pub source_modified_at: Option<String>,
    pub captured_at: String,
}

#[derive(Debug, Clone)]
pub struct ProviderSwitchTransactionRow {
    pub id: String,
    pub profile_id: String,
    pub provider_id: String,
    pub codex_home: String,
    pub config_backup_path: String,
    pub config_existed: bool,
    pub auth_backup_path: String,
    pub auth_existed: bool,
    pub config_candidate_path: String,
    pub auth_candidate_path: String,
    pub auth_target_exists: bool,
    pub expected_config_sha256: String,
    pub expected_auth_sha256: Option<String>,
    pub phase: String,
    pub created_at: String,
}

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

            CREATE TABLE IF NOT EXISTS provider_configurations (
                profile_id TEXT NOT NULL,
                provider_id TEXT NOT NULL,
                base_url TEXT NOT NULL,
                requires_openai_auth INTEGER NOT NULL DEFAULT 1,
                api_key TEXT NOT NULL,
                managed_by_tool INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (profile_id, provider_id)
            );

            CREATE TABLE IF NOT EXISTS official_auth_snapshots (
                profile_id TEXT PRIMARY KEY,
                snapshot_path TEXT NOT NULL,
                source_sha256 TEXT NOT NULL,
                source_modified_at TEXT,
                captured_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS provider_switch_transactions (
                id TEXT PRIMARY KEY,
                profile_id TEXT NOT NULL,
                provider_id TEXT NOT NULL,
                codex_home TEXT NOT NULL,
                config_backup_path TEXT NOT NULL,
                config_existed INTEGER NOT NULL DEFAULT 1,
                auth_backup_path TEXT NOT NULL,
                auth_existed INTEGER NOT NULL,
                config_candidate_path TEXT NOT NULL DEFAULT '',
                auth_candidate_path TEXT NOT NULL DEFAULT '',
                auth_target_exists INTEGER NOT NULL DEFAULT 0,
                expected_config_sha256 TEXT NOT NULL DEFAULT '',
                expected_auth_sha256 TEXT,
                phase TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            DROP TABLE IF EXISTS sync_jobs;
            DROP TABLE IF EXISTS sync_baselines;
            DROP TABLE IF EXISTS provider_replication_jobs;
            "#,
        )?;
        self.remove_legacy_profile_mode()?;
        self.drop_column_if_present("provider_configurations", "display_name")?;
        self.drop_column_if_present("provider_configurations", "url_input_mode")?;
        self.drop_column_if_present("provider_configurations", "wire_api")?;
        self.add_column_if_missing(
            "profiles",
            "discovery_source",
            "TEXT NOT NULL DEFAULT '已有实例'",
        )?;
        self.add_column_if_missing("official_auth_snapshots", "source_modified_at", "TEXT")?;
        self.add_column_if_missing(
            "provider_switch_transactions",
            "config_existed",
            "INTEGER NOT NULL DEFAULT 1",
        )?;
        self.add_column_if_missing(
            "provider_switch_transactions",
            "config_candidate_path",
            "TEXT NOT NULL DEFAULT ''",
        )?;
        self.add_column_if_missing(
            "provider_switch_transactions",
            "auth_candidate_path",
            "TEXT NOT NULL DEFAULT ''",
        )?;
        self.add_column_if_missing(
            "provider_switch_transactions",
            "auth_target_exists",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        self.add_column_if_missing(
            "provider_switch_transactions",
            "expected_config_sha256",
            "TEXT NOT NULL DEFAULT ''",
        )?;
        self.add_column_if_missing(
            "provider_switch_transactions",
            "expected_auth_sha256",
            "TEXT",
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

    fn drop_column_if_present(&self, table: &str, column: &str) -> AppResult<()> {
        let exists = {
            let mut statement = self
                .connection
                .prepare(&format!("PRAGMA table_info({table})"))?;
            let names = statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<Result<Vec<_>, _>>()?;
            names.iter().any(|name| name == column)
        };
        if exists {
            self.connection
                .execute(&format!("ALTER TABLE {table} DROP COLUMN {column}"), [])?;
        }
        Ok(())
    }

    fn remove_legacy_profile_mode(&self) -> AppResult<()> {
        let has_mode = {
            let mut statement = self.connection.prepare("PRAGMA table_info(profiles)")?;
            let names = statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<Result<Vec<_>, _>>()?;
            names.iter().any(|name| name == "mode")
        };
        if has_mode {
            self.connection
                .execute("DELETE FROM profiles WHERE mode = 'managed'", [])?;
            self.connection
                .execute("ALTER TABLE profiles DROP COLUMN mode", [])?;
        }
        Ok(())
    }

    pub fn list_profiles(&self) -> AppResult<Vec<Profile>> {
        let mut statement = self.connection.prepare(
            "SELECT id, name, kind, codex_home, provider_id, app_path,
                    discovery_source, providers_json, config_profiles_json, created_at, updated_at
             FROM profiles ORDER BY created_at ASC",
        )?;
        let rows = statement.query_map([], row_to_profile)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    pub fn get_profile(&self, profile_id: &str) -> AppResult<Profile> {
        self.connection
            .query_row(
                "SELECT id, name, kind, codex_home, provider_id, app_path,
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
             (id, name, kind, codex_home, provider_id, app_path, discovery_source,
              providers_json, config_profiles_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                profile.id,
                profile.name,
                kind_text(&profile.kind),
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

    pub fn upsert_provider_config(&self, value: &ProviderConfigRow) -> AppResult<()> {
        self.connection.execute(
            "INSERT INTO provider_configurations
             (profile_id, provider_id, base_url, requires_openai_auth,
              api_key, managed_by_tool, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(profile_id, provider_id) DO UPDATE SET
              base_url = excluded.base_url,
              requires_openai_auth = excluded.requires_openai_auth,
              api_key = excluded.api_key,
              managed_by_tool = excluded.managed_by_tool,
              updated_at = excluded.updated_at",
            params![
                value.profile_id,
                value.provider_id,
                value.base_url,
                value.requires_openai_auth,
                value.api_key,
                value.managed_by_tool,
                value.created_at,
                value.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn delete_provider_config(&self, profile_id: &str, provider_id: &str) -> AppResult<()> {
        self.connection.execute(
            "DELETE FROM provider_configurations WHERE profile_id = ?1 AND provider_id = ?2",
            params![profile_id, provider_id],
        )?;
        Ok(())
    }

    pub fn get_provider_config(
        &self,
        profile_id: &str,
        provider_id: &str,
    ) -> AppResult<Option<ProviderConfigRow>> {
        let result = self.connection.query_row(
            "SELECT profile_id, provider_id, base_url, requires_openai_auth,
                    api_key, managed_by_tool, created_at, updated_at
             FROM provider_configurations WHERE profile_id = ?1 AND provider_id = ?2",
            params![profile_id, provider_id],
            row_to_provider_config,
        );
        match result {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(AppError::Database(error)),
        }
    }

    pub fn list_provider_configs(&self, profile_id: &str) -> AppResult<Vec<ProviderConfigRow>> {
        let mut statement = self.connection.prepare(
            "SELECT profile_id, provider_id, base_url, requires_openai_auth,
                    api_key, managed_by_tool, created_at, updated_at
             FROM provider_configurations WHERE profile_id = ?1 ORDER BY provider_id COLLATE NOCASE",
        )?;
        let rows = statement.query_map([profile_id], row_to_provider_config)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    pub fn save_official_snapshot(&self, value: &OfficialSnapshotRow) -> AppResult<()> {
        self.connection.execute(
            "INSERT INTO official_auth_snapshots
             (profile_id, snapshot_path, source_sha256, source_modified_at, captured_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(profile_id) DO UPDATE SET
              snapshot_path = excluded.snapshot_path,
              source_sha256 = excluded.source_sha256,
              source_modified_at = excluded.source_modified_at,
              captured_at = excluded.captured_at",
            params![
                value.profile_id,
                value.snapshot_path,
                value.source_sha256,
                value.source_modified_at,
                value.captured_at
            ],
        )?;
        Ok(())
    }

    pub fn get_official_snapshot(
        &self,
        profile_id: &str,
    ) -> AppResult<Option<OfficialSnapshotRow>> {
        let result = self.connection.query_row(
            "SELECT profile_id, snapshot_path, source_sha256, source_modified_at, captured_at
             FROM official_auth_snapshots WHERE profile_id = ?1",
            [profile_id],
            |row| {
                Ok(OfficialSnapshotRow {
                    profile_id: row.get(0)?,
                    snapshot_path: row.get(1)?,
                    source_sha256: row.get(2)?,
                    source_modified_at: row.get(3)?,
                    captured_at: row.get(4)?,
                })
            },
        );
        match result {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(AppError::Database(error)),
        }
    }

    pub fn insert_switch_transaction(&self, value: &ProviderSwitchTransactionRow) -> AppResult<()> {
        self.connection.execute(
            "INSERT INTO provider_switch_transactions
             (id, profile_id, provider_id, codex_home, config_backup_path, auth_backup_path,
              auth_existed, config_existed, config_candidate_path, auth_candidate_path, auth_target_exists,
              expected_config_sha256, expected_auth_sha256, phase, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                value.id,
                value.profile_id,
                value.provider_id,
                value.codex_home,
                value.config_backup_path,
                value.auth_backup_path,
                value.auth_existed,
                value.config_existed,
                value.config_candidate_path,
                value.auth_candidate_path,
                value.auth_target_exists,
                value.expected_config_sha256,
                value.expected_auth_sha256,
                value.phase,
                value.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn update_switch_transaction_phase(&self, id: &str, phase: &str) -> AppResult<()> {
        self.connection.execute(
            "UPDATE provider_switch_transactions SET phase = ?2 WHERE id = ?1",
            params![id, phase],
        )?;
        Ok(())
    }

    pub fn list_pending_switch_transactions(&self) -> AppResult<Vec<ProviderSwitchTransactionRow>> {
        let mut statement = self.connection.prepare(
            "SELECT id, profile_id, provider_id, codex_home, config_backup_path,
                    auth_backup_path, auth_existed, config_existed, config_candidate_path, auth_candidate_path,
                    auth_target_exists, expected_config_sha256, expected_auth_sha256, phase,
                    created_at
             FROM provider_switch_transactions WHERE phase != 'complete'",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(ProviderSwitchTransactionRow {
                id: row.get(0)?,
                profile_id: row.get(1)?,
                provider_id: row.get(2)?,
                codex_home: row.get(3)?,
                config_backup_path: row.get(4)?,
                auth_backup_path: row.get(5)?,
                auth_existed: row.get::<_, i64>(6)? != 0,
                config_existed: row.get::<_, i64>(7)? != 0,
                config_candidate_path: row.get(8)?,
                auth_candidate_path: row.get(9)?,
                auth_target_exists: row.get::<_, i64>(10)? != 0,
                expected_config_sha256: row.get(11)?,
                expected_auth_sha256: row.get(12)?,
                phase: row.get(13)?,
                created_at: row.get(14)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    pub fn delete_switch_transaction(&self, id: &str) -> AppResult<()> {
        self.connection.execute(
            "DELETE FROM provider_switch_transactions WHERE id = ?1",
            [id],
        )?;
        Ok(())
    }
}

fn row_to_profile(row: &rusqlite::Row<'_>) -> rusqlite::Result<Profile> {
    let kind: String = row.get(2)?;
    Ok(Profile {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: match kind.as_str() {
            "custom_api" => ProfileKind::CustomApi,
            _ => ProfileKind::ChatGptAccount,
        },
        codex_home: row.get(3)?,
        provider_id: row.get(4)?,
        app_path: row.get(5)?,
        discovery_source: row.get(6)?,
        providers: json_column::<DiscoveredProvider>(row, 7),
        config_profiles: json_column::<DiscoveredConfigProfile>(row, 8),
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
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

fn row_to_provider_config(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProviderConfigRow> {
    Ok(ProviderConfigRow {
        profile_id: row.get(0)?,
        provider_id: row.get(1)?,
        base_url: row.get(2)?,
        requires_openai_auth: row.get::<_, i64>(3)? != 0,
        api_key: row.get(4)?,
        managed_by_tool: row.get::<_, i64>(5)? != 0,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn kind_text(kind: &ProfileKind) -> &'static str {
    match kind {
        ProfileKind::ChatGptAccount => "chat_gpt_account",
        ProfileKind::CustomApi => "custom_api",
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
                INSERT INTO profiles VALUES (
                    'managed', 'Managed', 'custom_api', 'managed',
                    'C:/legacy-managed', 'relay', NULL, 'created', 'updated'
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
        let columns: Vec<String> = store
            .connection
            .prepare("PRAGMA table_info(profiles)")
            .unwrap()
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(!columns.iter().any(|column| column == "mode"));
    }

    #[test]
    fn removes_legacy_provider_metadata_columns_without_losing_config() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("app.sqlite");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE provider_configurations (
                    profile_id TEXT NOT NULL,
                    provider_id TEXT NOT NULL,
                    display_name TEXT NOT NULL,
                    base_url TEXT NOT NULL,
                    url_input_mode TEXT NOT NULL,
                    wire_api TEXT NOT NULL DEFAULT 'responses',
                    requires_openai_auth INTEGER NOT NULL DEFAULT 1,
                    api_key TEXT NOT NULL,
                    managed_by_tool INTEGER NOT NULL DEFAULT 1,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    PRIMARY KEY (profile_id, provider_id)
                );
                INSERT INTO provider_configurations
                    (profile_id, provider_id, display_name, base_url, url_input_mode,
                     wire_api, requires_openai_auth, api_key, managed_by_tool, created_at, updated_at)
                VALUES ('profile', 'relay', 'Old relay', 'https://relay.example/v1',
                        'responses_endpoint', 'responses', 1, 'sk-test', 1, 'created', 'updated');
                "#,
            )
            .unwrap();
        drop(connection);

        let store = Store::open(temp.path()).unwrap();
        let columns: Vec<String> = store
            .connection
            .prepare("PRAGMA table_info(provider_configurations)")
            .unwrap()
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(!columns.iter().any(|column| column == "display_name"));
        assert!(!columns.iter().any(|column| column == "url_input_mode"));
        assert!(!columns.iter().any(|column| column == "wire_api"));

        let saved = store
            .get_provider_config("profile", "relay")
            .unwrap()
            .unwrap();
        assert_eq!(saved.provider_id, "relay");
        assert_eq!(saved.base_url, "https://relay.example/v1");
        assert_eq!(saved.api_key, "sk-test");
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
