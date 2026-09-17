use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// AR-3: per-day per-user per-model revenue detail. Existing settled days are
/// backfilled by the settlement startup pass, which recomputes any day whose
/// user-model rows are missing.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(AdminRevenueDailyUserModelRows::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AdminRevenueDailyUserModelRows::Id)
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(AdminRevenueDailyUserModelRows::Day)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AdminRevenueDailyUserModelRows::UserId)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AdminRevenueDailyUserModelRows::Model)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AdminRevenueDailyUserModelRows::ChargeNanoUsd)
                            .text()
                            .not_null()
                            .default("0"),
                    )
                    .col(
                        ColumnDef::new(AdminRevenueDailyUserModelRows::Calls)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(AdminRevenueDailyUserModelRows::InputTokens)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(AdminRevenueDailyUserModelRows::OutputTokens)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("uq_admin_revenue_daily_user_model")
                    .table(AdminRevenueDailyUserModelRows::Table)
                    .col(AdminRevenueDailyUserModelRows::Day)
                    .col(AdminRevenueDailyUserModelRows::UserId)
                    .col(AdminRevenueDailyUserModelRows::Model)
                    .unique()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("uq_admin_revenue_daily_user_model")
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(AdminRevenueDailyUserModelRows::Table)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum AdminRevenueDailyUserModelRows {
    Table,
    Id,
    Day,
    UserId,
    Model,
    ChargeNanoUsd,
    Calls,
    InputTokens,
    OutputTokens,
}
