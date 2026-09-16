use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// AR-3: per-day per-user revenue detail rows. Existing settled days are
/// backfilled by the settlement task's startup pass on the next process start,
/// because every recomputation writes the user rows alongside the summary.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(AdminRevenueDailyUserRows::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AdminRevenueDailyUserRows::Id)
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(AdminRevenueDailyUserRows::Day)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AdminRevenueDailyUserRows::UserId)
                            .text()
                            .not_null(),
                    )
                    .col(ColumnDef::new(AdminRevenueDailyUserRows::Username).text())
                    .col(
                        ColumnDef::new(AdminRevenueDailyUserRows::ChargeNanoUsd)
                            .text()
                            .not_null()
                            .default("0"),
                    )
                    .col(
                        ColumnDef::new(AdminRevenueDailyUserRows::Calls)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(AdminRevenueDailyUserRows::InputTokens)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(AdminRevenueDailyUserRows::OutputTokens)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .to_owned(),
            )
            .await?;

        // Recomputation deletes and re-inserts a day's user rows; the unique key
        // is both the conflict target and the per-day scan index.
        manager
            .create_index(
                Index::create()
                    .name("uq_admin_revenue_daily_user_day_user")
                    .table(AdminRevenueDailyUserRows::Table)
                    .col(AdminRevenueDailyUserRows::Day)
                    .col(AdminRevenueDailyUserRows::UserId)
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
                    .name("uq_admin_revenue_daily_user_day_user")
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(AdminRevenueDailyUserRows::Table)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum AdminRevenueDailyUserRows {
    Table,
    Id,
    Day,
    UserId,
    Username,
    ChargeNanoUsd,
    Calls,
    InputTokens,
    OutputTokens,
}
