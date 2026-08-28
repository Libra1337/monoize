use sea_orm::{ConnectionTrait, DbBackend, Statement, TransactionTrait};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        migrate(manager, true).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        migrate(manager, false).await
    }
}

async fn migrate(manager: &SchemaManager<'_>, up: bool) -> Result<(), DbErr> {
    let backend = manager.get_database_backend();
    if !matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
        return Ok(());
    }
    let tx = manager.get_connection().begin().await?;
    if up {
        migrate_reauth_scopes(&tx, backend, true).await?;
        tx.execute(Statement::from_string(
            backend,
            "ALTER TABLE store_retention_runs
             ADD COLUMN worker_owner_id TEXT NOT NULL DEFAULT ''"
                .to_string(),
        ))
        .await?;
        for sql in retention_runtime_tables(backend) {
            tx.execute(Statement::from_string(backend, sql)).await?;
        }
    } else {
        migrate_reauth_scopes(&tx, backend, false).await?;
        for table in [
            "store_legal_hold_items",
            "store_legal_hold_approvals",
            "store_retention_alerts",
            "store_retention_containments",
            "store_retention_state",
        ] {
            tx.execute(Statement::from_string(
                backend,
                format!("DROP TABLE IF EXISTS {table}"),
            ))
            .await?;
        }
        tx.execute(Statement::from_string(
            backend,
            "ALTER TABLE store_retention_runs DROP COLUMN worker_owner_id".to_string(),
        ))
        .await?;
    }
    tx.commit().await
}

async fn migrate_reauth_scopes<C: ConnectionTrait>(
    connection: &C,
    backend: DbBackend,
    include_retention: bool,
) -> Result<(), DbErr> {
    if backend == DbBackend::Postgres {
        if !include_retention {
            connection
                .execute(Statement::from_string(
                    backend,
                    "DELETE FROM store_reauth_grants
                     WHERE scope IN ('retention_operation', 'legal_hold')"
                        .to_string(),
                ))
                .await?;
        }
        connection
            .execute(Statement::from_string(
                backend,
                "ALTER TABLE store_reauth_grants
                 DROP CONSTRAINT IF EXISTS store_reauth_grants_scope_check"
                    .to_string(),
            ))
            .await?;
        connection
            .execute(Statement::from_string(
                backend,
                format!(
                    "ALTER TABLE store_reauth_grants
                     ADD CONSTRAINT store_reauth_grants_scope_check
                     CHECK (scope IN ({}))",
                    allowed_scopes(include_retention)
                ),
            ))
            .await?;
        return Ok(());
    }

    for index in ["uq_store_reauth_token_digest", "idx_store_reauth_expiry"] {
        connection
            .execute(Statement::from_string(
                backend,
                format!("DROP INDEX IF EXISTS {index}"),
            ))
            .await?;
    }
    connection
        .execute(Statement::from_string(
            backend,
            "ALTER TABLE store_reauth_grants RENAME TO store_reauth_grants_v058".to_string(),
        ))
        .await?;
    connection
        .execute(Statement::from_string(
            backend,
            sqlite_reauth_table(include_retention),
        ))
        .await?;
    connection
        .execute(Statement::from_string(
            backend,
            format!(
                "INSERT INTO store_reauth_grants
                    (id, user_id, session_token_digest, token_digest, scope, created_at, expires_at)
                 SELECT id, user_id, session_token_digest, token_digest, scope, created_at, expires_at
                 FROM store_reauth_grants_v058{}",
                if include_retention {
                    ""
                } else {
                    " WHERE scope NOT IN ('retention_operation', 'legal_hold')"
                }
            ),
        ))
        .await?;
    connection
        .execute(Statement::from_string(
            backend,
            "DROP TABLE store_reauth_grants_v058".to_string(),
        ))
        .await?;
    connection
        .execute(Statement::from_string(
            backend,
            "CREATE UNIQUE INDEX uq_store_reauth_token_digest
             ON store_reauth_grants (token_digest)"
                .to_string(),
        ))
        .await?;
    connection
        .execute(Statement::from_string(
            backend,
            "CREATE INDEX idx_store_reauth_expiry ON store_reauth_grants (expires_at, id)"
                .to_string(),
        ))
        .await?;
    Ok(())
}

fn allowed_scopes(include_retention: bool) -> &'static str {
    if include_retention {
        "'credential_update', 'redemption_access', 'compliance_confirm', 'refund', 'reprocess',
         'retention_operation', 'legal_hold'"
    } else {
        "'credential_update', 'redemption_access', 'compliance_confirm', 'refund', 'reprocess'"
    }
}

fn sqlite_reauth_table(include_retention: bool) -> String {
    format!(
        "CREATE TABLE store_reauth_grants (
            id TEXT PRIMARY KEY NOT NULL,
            user_id TEXT NOT NULL,
            session_token_digest TEXT NOT NULL,
            token_digest TEXT NOT NULL,
            scope TEXT NOT NULL CHECK (scope IN ({})),
            created_at TEXT NOT NULL,
            expires_at TEXT NOT NULL,
            FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
        )",
        allowed_scopes(include_retention)
    )
}

fn retention_runtime_tables(backend: DbBackend) -> Vec<String> {
    let integer = if backend == DbBackend::Postgres {
        "BIGINT"
    } else {
        "INTEGER"
    };
    [
        format!(
            "CREATE TABLE store_retention_state (
                singleton_id INTEGER NOT NULL PRIMARY KEY CHECK (singleton_id = 1),
                run_in_progress INTEGER NOT NULL CHECK (run_in_progress IN (0, 1)),
                current_run_id TEXT,
                current_worker_owner_id TEXT,
                last_run_id TEXT,
                consecutive_failures {integer} NOT NULL CHECK (consecutive_failures >= 0),
                checkout_paused INTEGER NOT NULL CHECK (checkout_paused IN (0, 1)),
                active_alert_id TEXT,
                latest_containment_id TEXT,
                updated_at TEXT NOT NULL,
                CHECK (
                    (run_in_progress = 1 AND current_run_id IS NOT NULL
                     AND current_worker_owner_id IS NOT NULL)
                    OR
                    (run_in_progress = 0 AND current_run_id IS NULL
                     AND current_worker_owner_id IS NULL)
                )
            )"
        ),
        format!(
            "CREATE TABLE store_retention_alerts (
                id TEXT NOT NULL PRIMARY KEY,
                run_id TEXT NOT NULL,
                severity TEXT NOT NULL CHECK (severity = 'critical'),
                consecutive_failures {integer} NOT NULL CHECK (consecutive_failures >= 3),
                created_at TEXT NOT NULL,
                contained_at TEXT,
                containment_id TEXT UNIQUE,
                CHECK (
                    (contained_at IS NULL AND containment_id IS NULL)
                    OR (contained_at IS NOT NULL AND containment_id IS NOT NULL)
                )
            )"
        ),
        "CREATE TABLE store_retention_containments (
            id TEXT NOT NULL PRIMARY KEY,
            alert_id TEXT NOT NULL UNIQUE,
            actor_id TEXT NOT NULL,
            reason TEXT NOT NULL,
            evidence_digest TEXT NOT NULL,
            created_at TEXT NOT NULL
        )"
        .to_string(),
        "CREATE TABLE store_legal_hold_approvals (
            hold_id TEXT NOT NULL PRIMARY KEY,
            requester_id TEXT NOT NULL,
            approver_role TEXT NOT NULL CHECK (approver_role IN ('privacy', 'legal')),
            extends_hold_id TEXT,
            FOREIGN KEY (hold_id) REFERENCES store_legal_holds(id) ON DELETE RESTRICT,
            FOREIGN KEY (extends_hold_id) REFERENCES store_legal_holds(id) ON DELETE RESTRICT
        )"
        .to_string(),
        "CREATE TABLE store_legal_hold_items (
            hold_id TEXT NOT NULL,
            data_class TEXT NOT NULL CHECK (
                data_class IN (
                    'raw_callback_bodies', 'network_metadata', 'financial_records',
                    'redemption_audits', 'expired_reauth_grants'
                )
            ),
            identifier TEXT NOT NULL,
            PRIMARY KEY (hold_id, identifier),
            FOREIGN KEY (hold_id) REFERENCES store_legal_holds(id) ON DELETE RESTRICT
        )"
        .to_string(),
        "CREATE INDEX idx_store_legal_hold_items_lookup
         ON store_legal_hold_items (data_class, identifier, hold_id)"
            .to_string(),
        "CREATE INDEX idx_store_legal_holds_expiry
         ON store_legal_holds (starts_at, expires_at, id)"
            .to_string(),
        "CREATE INDEX idx_store_retention_runs_started
         ON store_retention_runs (started_at DESC, id DESC)"
            .to_string(),
        "CREATE INDEX idx_store_retention_alerts_open
         ON store_retention_alerts (contained_at, created_at DESC, id DESC)"
            .to_string(),
        "INSERT INTO store_retention_state
            (singleton_id, run_in_progress, current_run_id, current_worker_owner_id,
             last_run_id, consecutive_failures, checkout_paused, active_alert_id,
             latest_containment_id, updated_at)
         VALUES (1, 0, NULL, NULL, NULL, 0, 0, NULL, NULL,
                 '2026-08-28T00:00:00.000000Z')"
            .to_string(),
    ]
    .into_iter()
    .collect()
}

#[cfg(test)]
mod tests {
    use super::{allowed_scopes, retention_runtime_tables};
    use sea_orm::DbBackend;

    #[test]
    fn retention_schema_has_persistent_pause_and_normalized_hold_items() {
        let sql = retention_runtime_tables(DbBackend::Postgres).join("\n");
        for required in [
            "consecutive_failures",
            "checkout_paused",
            "store_retention_alerts",
            "store_retention_containments",
            "store_legal_hold_items",
            "financial_records",
        ] {
            assert!(sql.contains(required), "missing {required}");
        }
        assert!(allowed_scopes(true).contains("'retention_operation'"));
        assert!(allowed_scopes(true).contains("'legal_hold'"));
    }
}
