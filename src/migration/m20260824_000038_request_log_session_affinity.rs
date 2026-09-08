use sea_orm::{ConnectionTrait, DbBackend, Statement};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// CM-AFF-2 diagnostics (`channel-management.spec.md`, RL-series
// `request-logs.spec.md`): persist the derived per-request
// `x-session-affinity` value when automatic session affinity produced one,
// so operators can distinguish client-side head drift from upstream
// instance churn when cache hit rates drop.
const UP_COLUMN_SQLITE: &str = "ALTER TABLE request_logs ADD COLUMN session_affinity_value TEXT";
const UP_COLUMN_PG: &str = "ALTER TABLE request_logs ADD COLUMN session_affinity_value TEXT";
const DOWN_COLUMN_SQLITE: &str = "ALTER TABLE request_logs DROP COLUMN session_affinity_value";
const DOWN_COLUMN_PG: &str =
    "ALTER TABLE request_logs DROP COLUMN IF EXISTS session_affinity_value";

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
