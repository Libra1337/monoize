use sea_orm::{ConnectionTrait, DbBackend, Statement};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// SC1 (`primary-replica-deployment.spec.md`): nullable dedupe key for replica balance
// deltas plus a partial unique index so replays of the same delta_id are no-ops while
// ordinary ledger rows keep NULL and stay unconstrained.
const UP_SQLITE: &str = "ALTER TABLE billing_ledger ADD COLUMN idempotency_key TEXT";
const UP_PG: &str = "ALTER TABLE billing_ledger ADD COLUMN idempotency_key TEXT";
const UP_INDEX: &str = "CREATE UNIQUE INDEX IF NOT EXISTS uidx_billing_ledger_idempotency_key ON billing_ledger (idempotency_key) WHERE idempotency_key IS NOT NULL";
const DOWN_INDEX: &str = "DROP INDEX IF EXISTS uidx_billing_ledger_idempotency_key";
const DOWN_COLUMN_SQLITE: &str = "ALTER TABLE billing_ledger DROP COLUMN idempotency_key";
const DOWN_COLUMN_PG: &str = "ALTER TABLE billing_ledger DROP COLUMN IF EXISTS idempotency_key";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if !matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            return Ok(());
        }
        let add_column = match backend {
            DbBackend::Postgres => UP_PG,
            _ => UP_SQLITE,
        };
        manager
            .get_connection()
            .execute(Statement::from_string(backend, add_column.to_string()))
            .await?;
        manager
            .get_connection()
            .execute(Statement::from_string(backend, UP_INDEX.to_string()))
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if !matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            return Ok(());
        }
        let drop_column = match backend {
            DbBackend::Postgres => DOWN_COLUMN_PG,
            _ => DOWN_COLUMN_SQLITE,
        };
        manager
            .get_connection()
            .execute(Statement::from_string(backend, DOWN_INDEX.to_string()))
            .await?;
        manager
            .get_connection()
            .execute(Statement::from_string(backend, drop_column.to_string()))
            .await?;
        Ok(())
    }
}
