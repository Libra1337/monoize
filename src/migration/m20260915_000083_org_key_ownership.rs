use sea_orm::{ConnectionTrait, Statement, TransactionTrait};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Org keys become wallet-owned (ORG-4): `api_keys.user_id = org_id`,
/// `api_keys.created_by` keeps the human creator, and historical request-log rows for
/// those keys are re-attributed to the org wallet so org analytics survive leave/delete.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let tx = manager.get_connection().begin().await?;
        for statement in [
            "ALTER TABLE api_keys ADD COLUMN created_by TEXT",
            "UPDATE api_keys SET created_by = user_id WHERE org_id IS NOT NULL",
            "UPDATE api_keys SET user_id = org_id WHERE org_id IS NOT NULL",
            "UPDATE request_logs SET user_id = (SELECT k.org_id FROM api_keys k WHERE k.id = request_logs.api_key_id)
             WHERE EXISTS (SELECT 1 FROM api_keys k
                           WHERE k.id = request_logs.api_key_id
                             AND k.org_id IS NOT NULL
                             AND request_logs.user_id = k.created_by)",
            "CREATE INDEX idx_request_logs_org ON request_logs (user_id, created_at_unix_ms)",
        ] {
            tx.execute(Statement::from_string(backend, statement.to_string()))
                .await?;
        }
        tx.commit().await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let tx = manager.get_connection().begin().await?;
        for statement in [
            "DROP INDEX idx_request_logs_org",
            "UPDATE request_logs SET user_id = (SELECT k.created_by FROM api_keys k WHERE k.id = request_logs.api_key_id)
             WHERE EXISTS (SELECT 1 FROM api_keys k
                           WHERE k.id = request_logs.api_key_id
                             AND k.org_id IS NOT NULL
                             AND request_logs.user_id = k.org_id)",
            "UPDATE api_keys SET user_id = created_by WHERE org_id IS NOT NULL",
            "ALTER TABLE api_keys DROP COLUMN created_by",
        ] {
            tx.execute(Statement::from_string(backend, statement.to_string()))
                .await?;
        }
        tx.commit().await
    }
}
