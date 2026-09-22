use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// SB-13: the embedded studio is replaced by the standalone Apeiron service;
/// its tables are dropped without compatibility aliases.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            "studio_assets",
            "studio_steps",
            "studio_runs",
            "studio_templates",
            "studio_projects",
        ] {
            manager
                .get_connection()
                .execute_unprepared(&format!("DROP TABLE IF EXISTS {table}"))
                .await?;
        }
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // The embedded studio is not restorable; the schema lives in git
        // history (m20260922_000121) for operators who need a manual rollback.
        Ok(())
    }
}
