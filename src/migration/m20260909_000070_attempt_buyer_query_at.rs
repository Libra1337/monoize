use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Adds `buyer_query_at` to `store_payment_attempts` (SB-P-Q2).
///
/// The buyer-triggered provider query is rate limited per Attempt. Holding the last contact
/// time in memory would reset on restart and would not be shared across callers, so a buyer
/// could bypass the limit by reloading or opening another tab. The column is nullable
/// because an Attempt that has never been queried has no contact time; a null value means
/// the next query may contact the provider immediately.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        manager
            .get_connection()
            .execute(Statement::from_string(
                backend,
                "ALTER TABLE store_payment_attempts ADD COLUMN buyer_query_at TEXT".to_string(),
            ))
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        manager
            .get_connection()
            .execute(Statement::from_string(
                backend,
                "ALTER TABLE store_payment_attempts DROP COLUMN buyer_query_at".to_string(),
            ))
            .await?;
        Ok(())
    }
}
