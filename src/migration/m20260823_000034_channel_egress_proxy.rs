use sea_orm::{ConnectionTrait, DbBackend, Statement};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// SC3 (`primary-replica-deployment.spec.md`): per-Channel egress proxy override.
// NULL means follow-global (`MONOIZE_UPSTREAM_PROXY_URL`); a non-empty value is a
// custom http(s) proxy URL validated at the dashboard boundary (CP-INV-14).
const UP_COLUMN_SQLITE: &str = "ALTER TABLE monoize_channels ADD COLUMN proxy_url TEXT";
const UP_COLUMN_PG: &str = "ALTER TABLE monoize_channels ADD COLUMN proxy_url TEXT";
const DOWN_COLUMN_SQLITE: &str = "ALTER TABLE monoize_channels DROP COLUMN proxy_url";
const DOWN_COLUMN_PG: &str = "ALTER TABLE monoize_channels DROP COLUMN IF EXISTS proxy_url";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if !matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            return Ok(());
        }
        let add_column = match backend {
            DbBackend::Postgres => UP_COLUMN_PG,
            _ => UP_COLUMN_SQLITE,
        };
        manager
            .get_connection()
            .execute(Statement::from_string(backend, add_column.to_string()))
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if !matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            return Ok(());
        }
        let drop_column = match backend {
            DbBackend::Postgres => DOWN_COLUMN_PG,
            _ => DOWN_COLUMN_SQLITE,
        };
        manager
            .get_connection()
            .execute(Statement::from_string(backend, drop_column.to_string()))
            .await?;
        Ok(())
    }
}
