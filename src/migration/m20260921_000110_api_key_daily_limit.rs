use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// AKDL-1: per-API-key daily spend limit in nano-USD. NULL = unlimited. The
/// window is one Asia/Shanghai calendar day; spend is aggregated live from
/// request_logs by api_key_id, so the counter resets at Beijing midnight
/// without a scheduled job and without a per-key balance transfer.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ApiKeys::Table)
                    .add_column(ColumnDef::new(ApiKeys::DailyLimitNanoUsd).text())
                    .to_owned(),
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ApiKeys::Table)
                    .drop_column(ApiKeys::DailyLimitNanoUsd)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum ApiKeys {
    Table,
    DailyLimitNanoUsd,
}
