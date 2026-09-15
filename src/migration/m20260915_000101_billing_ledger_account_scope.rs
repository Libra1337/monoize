use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// SA-SCOPE1/SA-SCOPE2: `billing_ledger` mixes two accounts. Some kinds describe the owning
/// user's wallet balance; others describe an API key's sub-account balance. The
/// `balance_after_nano_usd` column carries the resulting balance of whichever account the row
/// describes, so the column cannot be read without knowing the account. Recording that account
/// explicitly makes wallet reconciliation possible.
///
/// Existing rows are classified by kind. The backfill is exhaustive: it sets `sub_account` for
/// exactly the three kinds whose `balance_after_nano_usd` was written from a sub-account balance,
/// and leaves every other row at the `user` default.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(BillingLedger::Table)
                    .add_column(
                        ColumnDef::new(BillingLedger::AccountScope)
                            .string()
                            .not_null()
                            .default("user"),
                    )
                    .to_owned(),
            )
            .await?;

        // A reader that reconciles a wallet sums only `user` rows. This index keeps that
        // per-user scan off the full table.
        manager
            .create_index(
                Index::create()
                    .name("idx_billing_ledger_user_scope_created")
                    .table(BillingLedger::Table)
                    .col(BillingLedger::UserId)
                    .col(BillingLedger::AccountScope)
                    .col((BillingLedger::CreatedAt, IndexOrder::Desc))
                    .if_not_exists()
                    .to_owned(),
            )
            .await?;

        let db = manager.get_connection();
        db.execute_unprepared(
            "UPDATE billing_ledger SET account_scope = 'sub_account'
             WHERE kind IN ('api_key_charge', 'sub_account_transfer_in',
                            'admin_sub_account_adjustment')",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("idx_billing_ledger_user_scope_created")
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(BillingLedger::Table)
                    .drop_column(BillingLedger::AccountScope)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum BillingLedger {
    Table,
    UserId,
    AccountScope,
    CreatedAt,
}
