use sea_orm::{ConnectionTrait, DbBackend, Statement, TransactionTrait};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if !matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            return Ok(());
        }
        let tx = manager.get_connection().begin().await?;
        for sql in [
            "CREATE TABLE store_fulfillment_retries (
                order_id TEXT NOT NULL PRIMARY KEY,
                attempt_count BIGINT NOT NULL CHECK (attempt_count >= 1),
                next_attempt_at TEXT NOT NULL,
                last_error_category TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
            "CREATE INDEX idx_store_fulfillment_retries_due
             ON store_fulfillment_retries (next_attempt_at, order_id)",
        ] {
            tx.execute(Statement::from_string(backend, sql.to_string()))
                .await?;
        }
        if backend == DbBackend::Postgres {
            for sql in postgres_counter_upgrades() {
                tx.execute(Statement::from_string(backend, (*sql).to_string()))
                    .await?;
            }
        }
        tx.commit().await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            if backend == DbBackend::Postgres {
                for sql in postgres_counter_downgrades() {
                    manager
                        .get_connection()
                        .execute(Statement::from_string(backend, (*sql).to_string()))
                        .await?;
                }
            }
            manager
                .get_connection()
                .execute(Statement::from_string(
                    backend,
                    "DROP TABLE IF EXISTS store_fulfillment_retries".to_string(),
                ))
                .await?;
        }
        Ok(())
    }
}

fn postgres_counter_upgrades() -> &'static [&'static str] {
    &[
        "ALTER TABLE store_orders ALTER COLUMN state_revision TYPE BIGINT USING state_revision::BIGINT",
        "ALTER TABLE store_provider_events ALTER COLUMN state_revision TYPE BIGINT USING state_revision::BIGINT",
        "ALTER TABLE store_reconciliation_leases ALTER COLUMN epoch TYPE BIGINT USING epoch::BIGINT",
        "ALTER TABLE store_primary_leases ALTER COLUMN epoch TYPE BIGINT USING epoch::BIGINT",
    ]
}

fn postgres_counter_downgrades() -> &'static [&'static str] {
    &[
        "ALTER TABLE store_orders ALTER COLUMN state_revision TYPE INTEGER USING state_revision::INTEGER",
        "ALTER TABLE store_provider_events ALTER COLUMN state_revision TYPE INTEGER USING state_revision::INTEGER",
        "ALTER TABLE store_reconciliation_leases ALTER COLUMN epoch TYPE INTEGER USING epoch::INTEGER",
        "ALTER TABLE store_primary_leases ALTER COLUMN epoch TYPE INTEGER USING epoch::INTEGER",
    ]
}

#[cfg(test)]
mod tests {
    use super::postgres_counter_upgrades;

    #[test]
    fn postgres_upgrade_promotes_every_store_counter_read_as_i64() {
        let sql = postgres_counter_upgrades().join("\n");
        for column in [
            "store_orders ALTER COLUMN state_revision TYPE BIGINT",
            "store_provider_events ALTER COLUMN state_revision TYPE BIGINT",
            "store_reconciliation_leases ALTER COLUMN epoch TYPE BIGINT",
            "store_primary_leases ALTER COLUMN epoch TYPE BIGINT",
        ] {
            assert!(
                sql.contains(column),
                "missing PostgreSQL counter upgrade: {column}"
            );
        }
    }
}
