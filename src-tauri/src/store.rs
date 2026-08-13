use std::path::Path;

use chrono::Utc;
use rusqlite::{params, Connection};

use crate::error::{AppError, AppResult};
use crate::models::{Profile, ProfileKind, ProfileMode};

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
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS sync_jobs (
                id TEXT PRIMARY KEY,
                source_profile_id TEXT NOT NULL,
                target_profile_id TEXT NOT NULL,
                status TEXT NOT NULL,
                copied_count INTEGER NOT NULL DEFAULT 0,
                updated_count INTEGER NOT NULL DEFAULT 0,
                skipped_count INTEGER NOT NULL DEFAULT 0,
                conflict_count INTEGER NOT NULL DEFAULT 0,
                backup_dir TEXT,
                error TEXT,
                created_at TEXT NOT NULL,
                completed_at TEXT
            );

            CREATE TABLE IF NOT EXISTS sync_baselines (
                source_profile_id TEXT NOT NULL,
                target_profile_id TEXT NOT NULL,
                thread_id TEXT NOT NULL,
                source_sha256 TEXT NOT NULL,
                target_sha256 TEXT NOT NULL,
                synced_at TEXT NOT NULL,
                PRIMARY KEY (source_profile_id, target_profile_id, thread_id)
            );
            "#,
        )?;
        Ok(())
    }

    pub fn list_profiles(&self) -> AppResult<Vec<Profile>> {
        let mut statement = self.connection.prepare(
            "SELECT id, name, kind, mode, codex_home, provider_id, app_path, created_at, updated_at
             FROM profiles ORDER BY created_at ASC",
        )?;
        let rows = statement.query_map([], row_to_profile)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    pub fn get_profile(&self, profile_id: &str) -> AppResult<Profile> {
        self.connection
            .query_row(
                "SELECT id, name, kind, mode, codex_home, provider_id, app_path, created_at, updated_at
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
             (id, name, kind, mode, codex_home, provider_id, app_path, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                profile.id,
                profile.name,
                kind_text(&profile.kind),
                mode_text(&profile.mode),
                profile.codex_home,
                profile.provider_id,
                profile.app_path,
                profile.created_at,
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

    pub fn begin_job(&self, job_id: &str, source: &str, target: &str) -> AppResult<()> {
        self.connection.execute(
            "INSERT INTO sync_jobs
             (id, source_profile_id, target_profile_id, status, created_at)
             VALUES (?1, ?2, ?3, 'running', ?4)",
            params![job_id, source, target, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn finish_job(
        &self,
        job_id: &str,
        copied: usize,
        updated: usize,
        skipped: usize,
        conflicts: usize,
        backup_dir: Option<&str>,
    ) -> AppResult<()> {
        self.connection.execute(
            "UPDATE sync_jobs SET status = 'completed', copied_count = ?2, updated_count = ?3,
             skipped_count = ?4, conflict_count = ?5, backup_dir = ?6, completed_at = ?7 WHERE id = ?1",
            params![
                job_id,
                copied as i64,
                updated as i64,
                skipped as i64,
                conflicts as i64,
                backup_dir,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn fail_job(&self, job_id: &str, error: &str) -> AppResult<()> {
        self.connection.execute(
            "UPDATE sync_jobs SET status = 'failed', error = ?2, completed_at = ?3 WHERE id = ?1",
            params![job_id, error, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn save_baselines(&self, values: &[(&str, &str, &str, &str, &str)]) -> AppResult<()> {
        let transaction = self.connection.unchecked_transaction()?;
        for (source_profile, target_profile, thread_id, source_sha, target_sha) in values {
            transaction.execute(
                "INSERT INTO sync_baselines
                 (source_profile_id, target_profile_id, thread_id, source_sha256, target_sha256, synced_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(source_profile_id, target_profile_id, thread_id) DO UPDATE SET
                 source_sha256 = excluded.source_sha256,
                 target_sha256 = excluded.target_sha256,
                 synced_at = excluded.synced_at",
                params![
                    source_profile,
                    target_profile,
                    thread_id,
                    source_sha,
                    target_sha,
                    Utc::now().to_rfc3339(),
                ],
            )?;
        }
        transaction.commit()?;
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
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
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
