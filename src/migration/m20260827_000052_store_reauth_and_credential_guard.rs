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
        for sql in [
            "CREATE TABLE store_reauth_grants (
                id TEXT NOT NULL PRIMARY KEY, user_id TEXT NOT NULL,
                session_token_digest TEXT NOT NULL, token_digest TEXT NOT NULL,
                scope TEXT NOT NULL CHECK (scope IN ('credential_update', 'redemption_access')),
                created_at TEXT NOT NULL, expires_at TEXT NOT NULL
            )",
            "CREATE UNIQUE INDEX uq_store_reauth_token_digest ON store_reauth_grants (token_digest)",
            "CREATE INDEX idx_store_reauth_expiry ON store_reauth_grants (expires_at, id)",
            "DROP INDEX uq_store_credentials_channel_active",
            "CREATE UNIQUE INDEX uq_store_credentials_channel_active ON store_channel_credentials (channel_id) WHERE status = 'active'",
        ] {
            tx.execute(Statement::from_string(backend, sql.to_string()))
                .await?;
        }
        tx.commit().await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if !matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            return Ok(());
        }

        let tx = manager.get_connection().begin().await?;
        for sql in [
            "DROP TABLE IF EXISTS store_reauth_grants",
            "DROP INDEX IF EXISTS uq_store_credentials_channel_active",
            "CREATE UNIQUE INDEX uq_store_credentials_channel_active ON store_channel_credentials (channel_id, id)",
        ] {
            tx.execute(Statement::from_string(backend, sql.to_string()))
                .await?;
        }
        tx.commit().await
    }
}
