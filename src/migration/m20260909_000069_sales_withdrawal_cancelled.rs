use sea_orm::{ConnectionTrait, DbBackend, Statement, TransactionTrait};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Adds `cancelled` to the withdrawal state set (SC-5.1b).
///
/// SQLite cannot alter a `CHECK` constraint, so the table is rebuilt. Migration 068 created
/// it in this release and production holds no rows yet, but the copy is unconditional so the
/// migration is correct on any database that already has withdrawals.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let tx = manager.get_connection().begin().await?;
        match backend {
            DbBackend::Sqlite => {
                tx.execute(Statement::from_string(
                    backend,
                    "CREATE TABLE sales_withdrawals_next (
                         id TEXT NOT NULL PRIMARY KEY,
                         agent_user_id TEXT NOT NULL,
                         amount_fen TEXT NOT NULL,
                         state TEXT NOT NULL
                             CHECK (state IN ('requested', 'paid', 'rejected', 'cancelled')),
                         requested_at TEXT NOT NULL,
                         decided_at TEXT,
                         decided_by TEXT,
                         decision_note TEXT
                     )"
                        .to_string(),
                ))
                .await?;
                tx.execute(Statement::from_string(
                    backend,
                    "INSERT INTO sales_withdrawals_next
                     SELECT id, agent_user_id, amount_fen, state, requested_at, decided_at,
                            decided_by, decision_note
                     FROM sales_withdrawals"
                        .to_string(),
                ))
                .await?;
                for statement in [
                    "DROP TABLE sales_withdrawals",
                    "ALTER TABLE sales_withdrawals_next RENAME TO sales_withdrawals",
                    "CREATE INDEX idx_sales_withdrawals_agent_state
                     ON sales_withdrawals (agent_user_id, state)",
                    "CREATE INDEX idx_sales_withdrawals_requested
                     ON sales_withdrawals (requested_at)",
                ] {
                    tx.execute(Statement::from_string(backend, statement.to_string()))
                        .await?;
                }
            }
            DbBackend::Postgres => {
                // Postgres names the inline constraint after the table and column.
                tx.execute(Statement::from_string(
                    backend,
                    "ALTER TABLE sales_withdrawals DROP CONSTRAINT IF EXISTS
                     sales_withdrawals_state_check"
                        .to_string(),
                ))
                .await?;
                tx.execute(Statement::from_string(
                    backend,
                    "ALTER TABLE sales_withdrawals ADD CONSTRAINT sales_withdrawals_state_check
                     CHECK (state IN ('requested', 'paid', 'rejected', 'cancelled'))"
                        .to_string(),
                ))
                .await?;
            }
            _ => return Ok(()),
        }
        tx.commit().await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let tx = manager.get_connection().begin().await?;
        match backend {
            DbBackend::Sqlite => {
                // A cancelled row cannot satisfy the narrower constraint, so it is rejected
                // rather than silently rewritten into another state.
                tx.execute(Statement::from_string(
                    backend,
                    "DELETE FROM sales_withdrawals WHERE state = \'cancelled\'".to_string(),
                ))
                .await?;
                tx.execute(Statement::from_string(
                    backend,
                    "CREATE TABLE sales_withdrawals_prev (
                         id TEXT NOT NULL PRIMARY KEY,
                         agent_user_id TEXT NOT NULL,
                         amount_fen TEXT NOT NULL,
                         state TEXT NOT NULL
                             CHECK (state IN ('requested', 'paid', 'rejected')),
                         requested_at TEXT NOT NULL,
                         decided_at TEXT,
                         decided_by TEXT,
                         decision_note TEXT
                     )"
                        .to_string(),
                ))
                .await?;
                for statement in [
                    "INSERT INTO sales_withdrawals_prev SELECT * FROM sales_withdrawals",
                    "DROP TABLE sales_withdrawals",
                    "ALTER TABLE sales_withdrawals_prev RENAME TO sales_withdrawals",
                    "CREATE INDEX idx_sales_withdrawals_agent_state
                     ON sales_withdrawals (agent_user_id, state)",
                    "CREATE INDEX idx_sales_withdrawals_requested
                     ON sales_withdrawals (requested_at)",
                ] {
                    tx.execute(Statement::from_string(backend, statement.to_string()))
                        .await?;
                }
            }
            DbBackend::Postgres => {
                tx.execute(Statement::from_string(
                    backend,
                    "DELETE FROM sales_withdrawals WHERE state = \'cancelled\'".to_string(),
                ))
                .await?;
                tx.execute(Statement::from_string(
                    backend,
                    "ALTER TABLE sales_withdrawals DROP CONSTRAINT IF EXISTS
                     sales_withdrawals_state_check"
                        .to_string(),
                ))
                .await?;
                tx.execute(Statement::from_string(
                    backend,
                    "ALTER TABLE sales_withdrawals ADD CONSTRAINT sales_withdrawals_state_check
                     CHECK (state IN ('requested', 'paid', 'rejected'))"
                        .to_string(),
                ))
                .await?;
            }
            _ => return Ok(()),
        }
        tx.commit().await
    }
}

#[cfg(test)]
mod tests {
    use super::Migration;
    use sea_orm::{ConnectionTrait, Database, DatabaseConnection};
    use sea_orm_migration::prelude::*;

    async fn database_before_069() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect SQLite");
        let manager = SchemaManager::new(&db);
        for migration in crate::migration::Migrator::migrations() {
            if migration.name() == Migration.name() {
                break;
            }
            migration.up(&manager).await.expect(migration.name());
        }
        db
    }

    async fn insert(db: &DatabaseConnection, id: &str, state: &str) -> Result<(), DbErr> {
        db.execute_unprepared(&format!(
            "INSERT INTO sales_withdrawals
                (id, agent_user_id, amount_fen, state, requested_at)
             VALUES (\'{id}\', \'agent-a\', \'1\', \'{state}\', \'2026-09-09T00:00:00Z\')"
        ))
        .await
        .map(|_| ())
    }

    /// SC-5.1b: a cancelled withdrawal is admissible only after this migration, and an
    /// existing row must survive the table rebuild.
    #[tokio::test]
    async fn cancelled_becomes_admissible_and_existing_rows_survive() {
        let db = database_before_069().await;
        insert(&db, "w-existing", "requested")
            .await
            .expect("seed before migration");
        insert(&db, "w-early", "cancelled")
            .await
            .expect_err("cancelled must fail the pre-069 CHECK");

        Migration
            .up(&SchemaManager::new(&db))
            .await
            .expect("apply 069");

        insert(&db, "w-cancelled", "cancelled")
            .await
            .expect("cancelled is admissible after 069");
        insert(&db, "w-bogus", "abandoned")
            .await
            .expect_err("an unknown state must still fail");

        let rows = db
            .query_all(sea_orm::Statement::from_string(
                sea_orm::DbBackend::Sqlite,
                "SELECT id FROM sales_withdrawals ORDER BY id".to_string(),
            ))
            .await
            .expect("query");
        assert_eq!(rows.len(), 2, "the pre-existing row must survive the rebuild");
    }

    #[tokio::test]
    async fn revert_drops_cancelled_rows_and_narrows_the_constraint() {
        let db = database_before_069().await;
        let manager = SchemaManager::new(&db);
        Migration.up(&manager).await.expect("apply 069");
        insert(&db, "w-keep", "paid").await.expect("seed paid");
        insert(&db, "w-drop", "cancelled")
            .await
            .expect("seed cancelled");

        Migration.down(&manager).await.expect("revert 069");

        let rows = db
            .query_all(sea_orm::Statement::from_string(
                sea_orm::DbBackend::Sqlite,
                "SELECT id FROM sales_withdrawals".to_string(),
            ))
            .await
            .expect("query");
        assert_eq!(rows.len(), 1, "the cancelled row must be gone");
        insert(&db, "w-after", "cancelled")
            .await
            .expect_err("the narrower CHECK must be back");
    }
}
