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
                    "CREATE INDEX IF NOT EXISTS idx_mcm_model_name_channel_id ON monoize_channel_models (model_name, channel_id)".to_string(),
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
                    "DROP INDEX IF EXISTS idx_mcm_model_name_channel_id".to_string(),
                ))
                .await?;
        }
        Ok(())
    }
}
