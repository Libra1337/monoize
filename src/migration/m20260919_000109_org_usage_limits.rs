use sea_orm::{ConnectionTrait, Statement, TransactionTrait};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Organization usage limits (org-usage-limits.spec.md): nullable nano-USD spend-limit
/// columns at three levels — orgs (space), org_members (member), api_keys (org key) —
/// with total / hourly / daily windows, plus the space daily-reset bookkeeping column.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let is_sqlite = matches!(backend, sea_orm::DbBackend::Sqlite);
        let tx = manager.get_connection().begin().await?;
        for statement in [
            "ALTER TABLE orgs ADD COLUMN spend_limit_total_nano_usd TEXT",
            "ALTER TABLE orgs ADD COLUMN spend_limit_hourly_nano_usd TEXT",
            "ALTER TABLE orgs ADD COLUMN spend_limit_daily_nano_usd TEXT",
            "ALTER TABLE orgs ADD COLUMN spend_limit_daily_reset_at TEXT",
            "ALTER TABLE org_members ADD COLUMN spend_limit_total_nano_usd TEXT",
            "ALTER TABLE org_members ADD COLUMN spend_limit_hourly_nano_usd TEXT",
            "ALTER TABLE org_members ADD COLUMN spend_limit_daily_nano_usd TEXT",
            "ALTER TABLE api_keys ADD COLUMN spend_limit_total_nano_usd TEXT",
            "ALTER TABLE api_keys ADD COLUMN spend_limit_hourly_nano_usd TEXT",
            "ALTER TABLE api_keys ADD COLUMN spend_limit_daily_nano_usd TEXT",
        ] {
            tx.execute(Statement::from_string(
                backend,
                if is_sqlite {
                    statement.to_string()
                } else {
                    statement.replace("ADD COLUMN", "ADD COLUMN IF NOT EXISTS")
                },
            ))
            .await?;
        }
        tx.commit().await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let is_sqlite = matches!(backend, sea_orm::DbBackend::Sqlite);
        let tx = manager.get_connection().begin().await?;
        for statement in [
            "ALTER TABLE api_keys DROP COLUMN spend_limit_daily_nano_usd",
            "ALTER TABLE api_keys DROP COLUMN spend_limit_hourly_nano_usd",
            "ALTER TABLE api_keys DROP COLUMN spend_limit_total_nano_usd",
            "ALTER TABLE org_members DROP COLUMN spend_limit_daily_nano_usd",
            "ALTER TABLE org_members DROP COLUMN spend_limit_hourly_nano_usd",
            "ALTER TABLE org_members DROP COLUMN spend_limit_total_nano_usd",
            "ALTER TABLE orgs DROP COLUMN spend_limit_daily_reset_at",
            "ALTER TABLE orgs DROP COLUMN spend_limit_daily_nano_usd",
            "ALTER TABLE orgs DROP COLUMN spend_limit_hourly_nano_usd",
            "ALTER TABLE orgs DROP COLUMN spend_limit_total_nano_usd",
        ] {
            tx.execute(Statement::from_string(
                backend,
                if is_sqlite {
                    statement.to_string()
                } else {
                    statement.replace("DROP COLUMN", "DROP COLUMN IF EXISTS")
                },
            ))
            .await?;
        }
        tx.commit().await
    }
}
