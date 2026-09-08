use sea_orm::{ConnectionTrait, DbBackend, Statement};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            manager
                .get_connection()
                .execute(Statement::from_string(
                    backend,
                    "CREATE INDEX IF NOT EXISTS idx_request_logs_legacy_created_at ON request_logs (created_at) WHERE created_at_unix_ms IS NULL".to_string(),
                ))
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            manager
                .get_connection()
                .execute(Statement::from_string(
                    backend,
                    "DROP INDEX IF EXISTS idx_request_logs_legacy_created_at".to_string(),
                ))
                .await?;
        }
        Ok(())
    }
}
