use sea_orm::{ConnectionTrait, DbBackend, Statement, TransactionTrait};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if !matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            return Ok(());
        }

        let tx = manager.get_connection().begin().await?;
        for table in ["users", "monoize_groups"] {
            tx.execute(Statement::from_string(
                backend,
                format!(
                    "ALTER TABLE {table} ADD COLUMN account_class TEXT NOT NULL DEFAULT 'standard' CHECK (account_class IN ('standard', 'enterprise'))"
                ),
            ))
            .await?;
        }

        tx.execute(Statement::from_string(
            backend,
            "CREATE TABLE user_account_class_audits (
                id TEXT PRIMARY KEY NOT NULL,
                user_id TEXT NOT NULL,
                actor_user_id TEXT NOT NULL,
                from_account_class TEXT NOT NULL CHECK (from_account_class IN ('standard', 'enterprise')),
                to_account_class TEXT NOT NULL CHECK (to_account_class IN ('standard', 'enterprise')),
                deleted_api_key_count INTEGER NOT NULL CHECK (deleted_api_key_count >= 0),
                deleted_api_keys_json TEXT NOT NULL,
                created_at TEXT NOT NULL
            )"
            .to_string(),
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            "CREATE INDEX idx_user_account_class_audits_user_created ON user_account_class_audits (user_id, created_at)".to_string(),
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            "CREATE INDEX idx_request_logs_api_key_created ON request_logs (api_key_id, created_at_unix_ms)".to_string(),
        ))
        .await?;
        tx.commit().await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if !matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            return Ok(());
        }

        let tx = manager.get_connection().begin().await?;
        tx.execute(Statement::from_string(
            backend,
            "DROP INDEX idx_request_logs_api_key_created".to_string(),
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            "DROP TABLE user_account_class_audits".to_string(),
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            "ALTER TABLE monoize_groups DROP COLUMN account_class".to_string(),
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            "ALTER TABLE users DROP COLUMN account_class".to_string(),
        ))
        .await?;
        tx.commit().await
    }
}
