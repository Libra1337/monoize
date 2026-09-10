use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Indexes `billing_ledger` for the wallet query (WL-1).
///
/// The wallet reads one page ordered by time: `WHERE user_id = ? ORDER BY created_at DESC,
/// id DESC LIMIT n`. The existing single-column index on `user_id` locates the rows but
/// cannot supply that order, so SQLite materialises every row the user owns into a temporary
/// B-tree and sorts it to return ten. On production that meant reading 74,523 rows per
/// request for the busiest account, costing 137 ms serially and collapsing to 9 requests per
/// second under concurrency while other endpoints served 2,400.
///
/// Ordering the index columns the same way the query orders them lets the scan stop after
/// the requested page. `request_logs`, which is the same size and shape, already carries the
/// equivalent index; this brings the ledger in line with it.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        manager
            .get_connection()
            .execute(Statement::from_string(
                backend,
                "CREATE INDEX IF NOT EXISTS idx_billing_ledger_user_created
                 ON billing_ledger (user_id, created_at DESC, id DESC)"
                    .to_string(),
            ))
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        manager
            .get_connection()
            .execute(Statement::from_string(
                backend,
                "DROP INDEX IF EXISTS idx_billing_ledger_user_created".to_string(),
            ))
            .await?;
        Ok(())
    }
}
