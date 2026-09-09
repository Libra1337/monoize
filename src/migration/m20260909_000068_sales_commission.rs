use sea_orm::{ConnectionTrait, DbBackend, Statement, TransactionTrait};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Integer type for a bounded counter, which differs between the two supported backends.
const fn integer_type(backend: DbBackend) -> &'static str {
    match backend {
        DbBackend::Postgres => "BIGINT",
        _ => "INTEGER",
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if !matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            return Ok(());
        }
        let integer = integer_type(backend);
        let tx = manager.get_connection().begin().await?;

        // SC-D1: one agent per user. `code` is plaintext with a UNIQUE index rather than an
        // encrypted digest (SC-D1a): it carries no bearer value, and the agent's own page must
        // display it on every load, which an encrypt-and-reveal scheme cannot serve without a
        // reauthentication grant per view.
        tx.execute(Statement::from_string(
            backend,
            format!(
                "CREATE TABLE sales_agents (
                     user_id TEXT NOT NULL PRIMARY KEY,
                     code TEXT NOT NULL,
                     discount_bp {integer} NOT NULL CHECK (discount_bp BETWEEN 0 AND 2000),
                     commission_balance_fen TEXT NOT NULL,
                     enabled {integer} NOT NULL CHECK (enabled IN (0, 1)),
                     created_at TEXT NOT NULL,
                     updated_at TEXT NOT NULL
                 )"
            ),
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            "CREATE UNIQUE INDEX uq_sales_agents_code ON sales_agents (code)".to_string(),
        ))
        .await?;

        // SC-D2: the UNIQUE constraint on `order_id` is the sole mechanism that makes
        // commission at-most-once per order, for both an applied code and a later claim.
        tx.execute(Statement::from_string(
            backend,
            format!(
                "CREATE TABLE sales_commission_entries (
                     id TEXT NOT NULL PRIMARY KEY,
                     agent_user_id TEXT NOT NULL,
                     order_id TEXT NOT NULL,
                     order_number TEXT NOT NULL,
                     buyer_user_id TEXT NOT NULL,
                     base_fen TEXT NOT NULL,
                     commission_fen TEXT NOT NULL,
                     discount_bp {integer} NOT NULL,
                     commission_rate_bp {integer} NOT NULL,
                     origin TEXT NOT NULL CHECK (origin IN ('code', 'claim')),
                     reversed_at TEXT,
                     created_at TEXT NOT NULL
                 )"
            ),
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            "CREATE UNIQUE INDEX uq_sales_commission_entries_order
             ON sales_commission_entries (order_id)"
                .to_string(),
        ))
        .await?;
        // SC-6.2 aggregates scan one agent over a time window.
        tx.execute(Statement::from_string(
            backend,
            "CREATE INDEX idx_sales_commission_entries_agent_created
             ON sales_commission_entries (agent_user_id, created_at)"
                .to_string(),
        ))
        .await?;
        // SC-4.2 resolves a claim by order number.
        tx.execute(Statement::from_string(
            backend,
            "CREATE INDEX idx_sales_commission_entries_order_number
             ON sales_commission_entries (order_number)"
                .to_string(),
        ))
        .await?;

        tx.execute(Statement::from_string(
            backend,
            "CREATE TABLE sales_withdrawals (
                 id TEXT NOT NULL PRIMARY KEY,
                 agent_user_id TEXT NOT NULL,
                 amount_fen TEXT NOT NULL,
                 state TEXT NOT NULL CHECK (state IN ('requested', 'paid', 'rejected')),
                 requested_at TEXT NOT NULL,
                 decided_at TEXT,
                 decided_by TEXT,
                 decision_note TEXT
             )"
            .to_string(),
        ))
        .await?;
        // SC-5.3 admits one `requested` withdrawal per agent, and SC-5.8 lists newest first.
        tx.execute(Statement::from_string(
            backend,
            "CREATE INDEX idx_sales_withdrawals_agent_state
             ON sales_withdrawals (agent_user_id, state)"
                .to_string(),
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            "CREATE INDEX idx_sales_withdrawals_requested
             ON sales_withdrawals (requested_at)"
                .to_string(),
        ))
        .await?;

        // SC-D4: attempts exist only for the SC-4.7 rate limit. A successful claim is proven
        // by its commission entry, not by this table.
        tx.execute(Statement::from_string(
            backend,
            format!(
                "CREATE TABLE sales_claim_attempts (
                     id TEXT NOT NULL PRIMARY KEY,
                     agent_user_id TEXT NOT NULL,
                     succeeded {integer} NOT NULL CHECK (succeeded IN (0, 1)),
                     attempted_at TEXT NOT NULL
                 )"
            ),
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            "CREATE INDEX idx_sales_claim_attempts_agent_time
             ON sales_claim_attempts (agent_user_id, attempted_at)"
                .to_string(),
        ))
        .await?;

        // SC-D5: what the buyer submitted, frozen with the rest of the order snapshot.
        for column in [
            "ALTER TABLE store_orders ADD COLUMN sales_code TEXT",
            "ALTER TABLE store_orders ADD COLUMN sales_discount_bp {integer}",
        ] {
            tx.execute(Statement::from_string(
                backend,
                column.replace("{integer}", integer),
            ))
            .await?;
        }

        tx.commit().await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if !matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            return Ok(());
        }
        let tx = manager.get_connection().begin().await?;

        for statement in [
            "ALTER TABLE store_orders DROP COLUMN sales_discount_bp",
            "ALTER TABLE store_orders DROP COLUMN sales_code",
            "DROP TABLE sales_claim_attempts",
            "DROP TABLE sales_withdrawals",
            "DROP TABLE sales_commission_entries",
            "DROP TABLE sales_agents",
        ] {
            tx.execute(Statement::from_string(backend, statement.to_string()))
                .await?;
        }

        tx.commit().await
    }
}

#[cfg(test)]
mod tests {
    use super::Migration;
    use sea_orm::{
        ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement, TryGetable,
    };
    use sea_orm_migration::prelude::*;

    async fn database_before_068() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect SQLite");
        db.execute_unprepared("PRAGMA foreign_keys = ON")
            .await
            .expect("enable foreign keys");
        let manager = SchemaManager::new(&db);
        for migration in crate::migration::Migrator::migrations() {
            if migration.name() == Migration.name() {
                break;
            }
            migration.up(&manager).await.expect(migration.name());
        }
        db
    }

    async fn seed_agent(db: &DatabaseConnection, user_id: &str, code: &str, discount: i64) {
        db.execute_unprepared(&format!(
            "INSERT INTO sales_agents
                (user_id, code, discount_bp, commission_balance_fen, enabled,
                 created_at, updated_at)
             VALUES ('{user_id}', '{code}', {discount}, '0', 1,
                     '2026-09-09T00:00:00Z', '2026-09-09T00:00:00Z')"
        ))
        .await
        .expect("seed agent");
    }

    async fn seed_entry(db: &DatabaseConnection, id: &str, order_id: &str) -> Result<(), DbErr> {
        db.execute_unprepared(&format!(
            "INSERT INTO sales_commission_entries
                (id, agent_user_id, order_id, order_number, buyer_user_id, base_fen,
                 commission_fen, discount_bp, commission_rate_bp, origin, created_at)
             VALUES ('{id}', 'agent-a', '{order_id}', 'LS-{order_id}', 'buyer-1', '10000',
                     '400', 100, 500, 'code', '2026-09-09T00:00:00Z')"
        ))
        .await
        .map(|_| ())
    }

    /// SC-D2a: one order can carry at most one commission entry, which is what stops a claim
    /// from duplicating an accrual a code already produced.
    #[tokio::test]
    async fn one_order_accrues_commission_at_most_once() {
        let db = database_before_068().await;
        Migration
            .up(&SchemaManager::new(&db))
            .await
            .expect("apply 068");
        seed_agent(&db, "agent-a", "ABCD2345", 100).await;

        seed_entry(&db, "entry-1", "order-1")
            .await
            .expect("first accrual");
        seed_entry(&db, "entry-2", "order-1")
            .await
            .expect_err("a second entry for the same order must violate the unique index");
        seed_entry(&db, "entry-3", "order-2")
            .await
            .expect("a different order accrues independently");
    }

    /// SC-1.2: the column bound is the maximum configurable rate. The exact
    /// `discount_bp <= commission_rate_bp` rule depends on a mutable setting, so it is
    /// enforced in the write path rather than by a CHECK that would need a migration to
    /// change every time the rate moves.
    #[tokio::test]
    async fn discount_basis_points_are_bounded_by_the_maximum_rate() {
        let db = database_before_068().await;
        Migration
            .up(&SchemaManager::new(&db))
            .await
            .expect("apply 068");

        seed_agent(&db, "agent-zero", "AAAA2345", 0).await;
        seed_agent(&db, "agent-max", "BBBB2345", 500).await;
        db.execute_unprepared(
            "INSERT INTO sales_agents
                (user_id, code, discount_bp, commission_balance_fen, enabled,
                 created_at, updated_at)
             VALUES ('agent-over', 'CCCC2345', 2001, '0', 1,
                     '2026-09-09T00:00:00Z', '2026-09-09T00:00:00Z')",
        )
        .await
        .expect_err("a discount above the maximum rate must fail the CHECK");
    }

    #[tokio::test]
    async fn agent_codes_are_unique_and_order_columns_exist() {
        let db = database_before_068().await;
        Migration
            .up(&SchemaManager::new(&db))
            .await
            .expect("apply 068");
        seed_agent(&db, "agent-a", "ABCD2345", 0).await;

        db.execute_unprepared(
            "INSERT INTO sales_agents
                (user_id, code, discount_bp, commission_balance_fen, enabled,
                 created_at, updated_at)
             VALUES ('agent-b', 'ABCD2345', 0, '0', 1,
                     '2026-09-09T00:00:00Z', '2026-09-09T00:00:00Z')",
        )
        .await
        .expect_err("a duplicate code must violate the unique index");

        let columns = db
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT name FROM pragma_table_info('store_orders')".to_string(),
            ))
            .await
            .expect("query columns")
            .iter()
            .map(|row| String::try_get(row, "", "name").expect("name"))
            .collect::<Vec<_>>();
        assert!(
            columns.iter().any(|name| name == "sales_code"),
            "{columns:?}"
        );
        assert!(
            columns.iter().any(|name| name == "sales_discount_bp"),
            "{columns:?}"
        );
    }

    /// Reverting must leave the store schema exactly as it was, so the release can be rolled
    /// back without stranding columns on an immutable, trigger-guarded table.
    #[tokio::test]
    async fn revert_drops_every_object_it_created() {
        let db = database_before_068().await;
        let manager = SchemaManager::new(&db);
        Migration.up(&manager).await.expect("apply 068");
        Migration.down(&manager).await.expect("revert 068");

        let tables = db
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT name FROM sqlite_master WHERE type = 'table'
                 AND name LIKE 'sales_%'"
                    .to_string(),
            ))
            .await
            .expect("query tables");
        assert!(tables.is_empty(), "{} sales tables remain", tables.len());

        let columns = db
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT name FROM pragma_table_info('store_orders')".to_string(),
            ))
            .await
            .expect("query columns")
            .iter()
            .map(|row| String::try_get(row, "", "name").expect("name"))
            .collect::<Vec<_>>();
        assert!(
            !columns.iter().any(|name| name == "sales_code"),
            "{columns:?}"
        );

        Migration.up(&manager).await.expect("re-apply 068");
    }
}
