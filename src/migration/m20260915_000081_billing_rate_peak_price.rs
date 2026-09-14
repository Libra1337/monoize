use sea_orm::{ConnectionTrait, Statement, TransactionTrait};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Peak/off-peak dual pricing (metered-billing.spec.md MB-D3g): the optional
/// high-demand price per rate row. NULL means the row always bills at
/// `unit_price_nano`.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let tx = manager.get_connection().begin().await?;
        tx.execute(Statement::from_string(
            backend,
            "ALTER TABLE billing_rate_records ADD COLUMN peak_unit_price_nano TEXT".to_string(),
        ))
        .await?;
        tx.commit().await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let tx = manager.get_connection().begin().await?;
        tx.execute(Statement::from_string(
            backend,
            "ALTER TABLE billing_rate_records DROP COLUMN peak_unit_price_nano".to_string(),
        ))
        .await?;
        tx.commit().await
    }
}
