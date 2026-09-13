use sea_orm::{ConnectionTrait, Statement, TransactionTrait};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Content-firewall event log (content-firewall.spec.md §6): one row per
/// rejected request, capped at the newest 50,000 rows by runtime trimming.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let tx = manager.get_connection().begin().await?;
        for statement in [
            "CREATE TABLE firewall_events (
                id TEXT NOT NULL PRIMARY KEY,
                user_id TEXT,
                username TEXT,
                api_key_id TEXT,
                api_key_name TEXT,
                endpoint TEXT NOT NULL,
                model TEXT NOT NULL,
                term TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL,
                created_at_unix_ms INTEGER NOT NULL
            )",
            "CREATE INDEX idx_firewall_events_time ON firewall_events (created_at_unix_ms)",
            "CREATE INDEX idx_firewall_events_term ON firewall_events (term)",
        ] {
            tx.execute(Statement::from_string(backend, statement.to_string()))
                .await?;
        }
        tx.commit().await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let tx = manager.get_connection().begin().await?;
        tx.execute(Statement::from_string(
            backend,
            "DROP TABLE firewall_events".to_string(),
        ))
        .await?;
        tx.commit().await
    }
}
