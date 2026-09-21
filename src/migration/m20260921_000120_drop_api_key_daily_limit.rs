use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// AKDL-1 (superseded): the single-window daily_limit_nano_usd column is
/// replaced by the shared key-level spend_limit_{total,hourly,daily}_nano_usd
/// columns (ORGL-3), so the column is dropped. No row carried a value at
/// removal time, so no data move happens here.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
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

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
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
}

#[derive(DeriveIden)]
enum ApiKeys {
    Table,
    DailyLimitNanoUsd,
}
