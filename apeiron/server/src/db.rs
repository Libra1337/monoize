//! SQLite pool bootstrap and idempotent schema migration.

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use std::str::FromStr;

pub type Pool = sqlx::sqlite::SqlitePool;

pub async fn connect(dsn: &str) -> Result<Pool, String> {
    let path = dsn
        .strip_prefix("sqlite://")
        .unwrap_or(dsn)
        .trim_start_matches("//");
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create data dir {:?}: {e}", parent))?;
    }
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{path}"))
        .map_err(|e| format!("parse dsn: {e}"))?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(std::time::Duration::from_secs(30));
    SqlitePoolOptions::new()
        .max_connections(8)
        .acquire_timeout(std::time::Duration::from_secs(30))
        .connect_with(options)
        .await
        .map_err(|e| format!("connect sqlite: {e}"))
}

const SCHEMA_VERSION: &str = "1";

pub async fn migrate(pool: &Pool) -> Result<(), String> {
    sqlx::raw_sql(include_str!("../migrations/0001_init.sql"))
        .execute(pool)
        .await
        .map_err(|e| format!("apply schema: {e}"))?;
    sqlx::query(
        "INSERT INTO apeiron_meta (key, value) VALUES ('schema_version', $1) \
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
    )
    .bind(SCHEMA_VERSION)
    .execute(pool)
    .await
    .map_err(|e| format!("record schema version: {e}"))?;
    Ok(())
}
