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
        tx.execute(Statement::from_string(
            backend,
            table_sql(backend).to_string(),
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            "CREATE INDEX idx_store_refund_query_retries_due
             ON store_refund_query_retries (next_attempt_at, refund_id)"
                .to_string(),
        ))
        .await?;
        tx.commit().await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if !matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            return Ok(());
        }
        manager
            .get_connection()
            .execute(Statement::from_string(
                backend,
                "DROP TABLE IF EXISTS store_refund_query_retries".to_string(),
            ))
            .await?;
        Ok(())
    }
}

fn table_sql(backend: DbBackend) -> &'static str {
    if backend == DbBackend::Postgres {
        return "CREATE TABLE store_refund_query_retries (
            refund_id TEXT NOT NULL PRIMARY KEY,
            attempt_count BIGINT NOT NULL CHECK (attempt_count >= 0),
            next_attempt_at TEXT NOT NULL,
            last_error_category TEXT,
            alerted_at TEXT,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (refund_id) REFERENCES store_refunds(id) ON DELETE CASCADE
        )";
    }
    "CREATE TABLE store_refund_query_retries (
        refund_id TEXT NOT NULL PRIMARY KEY,
        attempt_count INTEGER NOT NULL CHECK (attempt_count >= 0),
        next_attempt_at TEXT NOT NULL,
        last_error_category TEXT,
        alerted_at TEXT,
        updated_at TEXT NOT NULL,
        FOREIGN KEY (refund_id) REFERENCES store_refunds(id) ON DELETE CASCADE
    )"
}
