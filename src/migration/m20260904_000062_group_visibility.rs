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
        tx.execute(Statement::from_string(
            backend,
            "ALTER TABLE monoize_groups ADD COLUMN is_public INTEGER NOT NULL DEFAULT 1",
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            "CREATE TABLE user_group_grants (user_id TEXT NOT NULL, group_id TEXT NOT NULL, created_at TEXT NOT NULL, PRIMARY KEY (user_id, group_id))",
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            "CREATE INDEX idx_user_group_grants_group ON user_group_grants (group_id, user_id)",
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
            "DROP TABLE user_group_grants",
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            "ALTER TABLE monoize_groups DROP COLUMN is_public",
        ))
        .await?;
        tx.commit().await
    }
}
