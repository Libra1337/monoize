use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// AR-3/AR-7: persisted per-day revenue aggregates and the revenue exclusion list.
/// Days are Asia/Shanghai calendar day ids (`YYYY-MM-DD`); amounts are canonical
/// decimal-text nano-USD so no binary floating point ever touches the values.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(AdminRevenueDailySummaries::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AdminRevenueDailySummaries::Id)
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(AdminRevenueDailySummaries::Day)
                            .text()
                            .not_null()
                            .unique_key(),
                    )
                    .col(
                        ColumnDef::new(AdminRevenueDailySummaries::TotalChargeNanoUsd)
                            .text()
                            .not_null()
                            .default("0"),
                    )
                    .col(
                        ColumnDef::new(AdminRevenueDailySummaries::TotalCalls)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(AdminRevenueDailySummaries::TotalInputTokens)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(AdminRevenueDailySummaries::TotalOutputTokens)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(AdminRevenueDailySummaries::ComputedAt)
                            .text()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(AdminRevenueDailyModelRows::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AdminRevenueDailyModelRows::Id)
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(AdminRevenueDailyModelRows::Day)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AdminRevenueDailyModelRows::Model)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AdminRevenueDailyModelRows::ChargeNanoUsd)
                            .text()
                            .not_null()
                            .default("0"),
                    )
                    .col(
                        ColumnDef::new(AdminRevenueDailyModelRows::Calls)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(AdminRevenueDailyModelRows::InputTokens)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(AdminRevenueDailyModelRows::OutputTokens)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .to_owned(),
            )
            .await?;

        // One recomputation deletes and re-inserts a day's rows; this key makes the
        // upsert conflict target unambiguous and the day scan cheap.
        manager
            .create_index(
                Index::create()
                    .name("uq_admin_revenue_daily_model_day_model")
                    .table(AdminRevenueDailyModelRows::Table)
                    .col(AdminRevenueDailyModelRows::Day)
                    .col(AdminRevenueDailyModelRows::Model)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(AdminRevenueExclusions::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AdminRevenueExclusions::UserId)
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(AdminRevenueExclusions::Username)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AdminRevenueExclusions::CreatedAt)
                            .text()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(AdminRevenueExclusions::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .drop_index(
                Index::drop()
                    .name("uq_admin_revenue_daily_model_day_model")
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(AdminRevenueDailyModelRows::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(AdminRevenueDailySummaries::Table)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum AdminRevenueDailySummaries {
    Table,
    Id,
    Day,
    TotalChargeNanoUsd,
    TotalCalls,
    TotalInputTokens,
    TotalOutputTokens,
    ComputedAt,
}

#[derive(DeriveIden)]
enum AdminRevenueDailyModelRows {
    Table,
    Id,
    Day,
    Model,
    ChargeNanoUsd,
    Calls,
    InputTokens,
    OutputTokens,
}

#[derive(DeriveIden)]
enum AdminRevenueExclusions {
    Table,
    UserId,
    Username,
    CreatedAt,
}
