use sea_orm::{ConnectionTrait, DbBackend, Statement, TransactionTrait, TryGetable};
use sea_orm_migration::SchemaManagerConnection;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const LEGACY_KINDS: &str = "('alipay', 'wechat')";
const EPAY_DRAFT_ID: &str = "store-channel-epay";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if !matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            return Ok(());
        }

        // SQLite cannot alter a CHECK constraint, so `store_payment_channels` is rebuilt. With
        // foreign keys enabled, `ALTER TABLE ... RENAME` rewrites every `REFERENCES` clause that
        // names the table and `DROP TABLE` cascades child rows away. The documented procedure is
        // to disable foreign keys around the rebuild and verify with `PRAGMA foreign_key_check`.
        let connection = manager.get_connection();
        let restore_foreign_keys = if backend == DbBackend::Sqlite {
            sqlite_foreign_keys_enabled(connection).await?
        } else {
            false
        };
        if backend == DbBackend::Sqlite {
            connection
                .execute_unprepared("PRAGMA legacy_alter_table = ON")
                .await?;
        }
        if restore_foreign_keys {
            connection
                .execute_unprepared("PRAGMA foreign_keys = OFF")
                .await?;
        }
        let result = self.apply(manager, backend).await;
        if restore_foreign_keys {
            connection
                .execute_unprepared("PRAGMA foreign_keys = ON")
                .await?;
        }
        if backend == DbBackend::Sqlite {
            connection
                .execute_unprepared("PRAGMA legacy_alter_table = OFF")
                .await?;
        }
        result
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if !matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            return Ok(());
        }
        let connection = manager.get_connection();
        let restore_foreign_keys = if backend == DbBackend::Sqlite {
            sqlite_foreign_keys_enabled(connection).await?
        } else {
            false
        };
        if backend == DbBackend::Sqlite {
            connection
                .execute_unprepared("PRAGMA legacy_alter_table = ON")
                .await?;
        }
        if restore_foreign_keys {
            connection
                .execute_unprepared("PRAGMA foreign_keys = OFF")
                .await?;
        }
        let result = self.revert(manager, backend).await;
        if restore_foreign_keys {
            connection
                .execute_unprepared("PRAGMA foreign_keys = ON")
                .await?;
        }
        if backend == DbBackend::Sqlite {
            connection
                .execute_unprepared("PRAGMA legacy_alter_table = OFF")
                .await?;
        }
        result
    }
}

impl Migration {
    async fn apply(&self, manager: &SchemaManager<'_>, backend: DbBackend) -> Result<(), DbErr> {
        let tx = manager.get_connection().begin().await?;

        // Legacy Alipay and WeChat configuration cannot describe an EPay merchant, so every
        // credential, compliance record, capability, and readiness profile is removed.
        for table in [
            "store_channel_credentials",
            "store_payment_compliance",
            "store_merchant_capabilities",
            "store_channel_readiness_profiles",
        ] {
            tx.execute(Statement::from_string(
                backend,
                format!(
                    "DELETE FROM {table} WHERE channel_id IN
                     (SELECT id FROM store_payment_channels WHERE adapter_kind IN {LEGACY_KINDS})"
                ),
            ))
            .await?;
        }

        // A legacy Channel that no order references carries no history and is removed. One that
        // orders reference must survive to keep the immutable order quote resolvable.
        tx.execute(Statement::from_string(
            backend,
            format!(
                "DELETE FROM store_payment_channels
                 WHERE adapter_kind IN {LEGACY_KINDS}
                   AND id NOT IN (SELECT payment_channel_id FROM store_orders)"
            ),
        ))
        .await?;

        match backend {
            DbBackend::Sqlite => {
                for sql in sqlite_rebuild_statements() {
                    tx.execute_unprepared(&sql).await?;
                }
            }
            DbBackend::Postgres => {
                let constraint = tx
                    .query_one(Statement::from_string(
                        backend,
                        "SELECT conname AS name FROM pg_constraint
                         WHERE conrelid = 'store_payment_channels'::regclass
                           AND contype = 'c'
                           AND pg_get_constraintdef(oid) LIKE '%alipay%'"
                            .to_string(),
                    ))
                    .await?;
                if let Some(constraint) = constraint {
                    let name = String::try_get(&constraint, "", "name")?;
                    tx.execute(Statement::from_string(
                        backend,
                        format!(
                            "ALTER TABLE store_payment_channels DROP CONSTRAINT {}",
                            quote_identifier(&name)
                        ),
                    ))
                    .await?;
                }
                tx.execute(Statement::from_string(
                    backend,
                    format!(
                        "UPDATE store_payment_channels
                         SET adapter_kind = 'epay', enabled = 0, revision = revision + 1
                         WHERE adapter_kind IN {LEGACY_KINDS}"
                    ),
                ))
                .await?;
                tx.execute(Statement::from_string(
                    backend,
                    "ALTER TABLE store_payment_channels
                     ADD CONSTRAINT ck_store_payment_channels_adapter
                     CHECK (adapter_kind IN ('epay', 'stripe', 'http'))"
                        .to_string(),
                ))
                .await?;
            }
            _ => {}
        }

        tx.execute(Statement::from_string(
            backend,
            format!(
                "INSERT INTO store_payment_channels
                    (id, adapter_kind, name, icon_kind, icon_value, sort_order, enabled,
                     revision, created_at, updated_at)
                 VALUES ('{EPAY_DRAFT_ID}', 'epay', 'EPay', 'builtin', 'epay', 10, 0, 1,
                         '2026-09-08T00:00:00Z', '2026-09-08T00:00:00Z')
                 ON CONFLICT (id) DO NOTHING"
            ),
        ))
        .await?;

        // The method switches are non-secret presentation state, so they live outside the
        // encrypted credential and stay editable without a credential rotation.
        tx.execute(Statement::from_string(
            backend,
            "CREATE TABLE store_epay_methods (
                channel_id TEXT NOT NULL,
                method TEXT NOT NULL CHECK (method IN ('alipay', 'wxpay')),
                label TEXT NOT NULL,
                icon_kind TEXT NOT NULL CHECK (icon_kind IN ('builtin', 'url', 'upload')),
                icon_value TEXT,
                sort_order INTEGER NOT NULL DEFAULT 0,
                enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (channel_id, method),
                FOREIGN KEY (channel_id) REFERENCES store_payment_channels (id) ON DELETE CASCADE
            )"
            .to_string(),
        ))
        .await?;
        for (method, label, sort_order) in [("alipay", "Alipay", 10), ("wxpay", "WeChat Pay", 20)] {
            tx.execute(Statement::from_string(
                backend,
                format!(
                    "INSERT INTO store_epay_methods
                        (channel_id, method, label, icon_kind, icon_value, sort_order,
                         enabled, created_at, updated_at)
                     SELECT id, '{method}', '{label}', 'builtin', '{method}', {sort_order}, 0,
                            '2026-09-08T00:00:00Z', '2026-09-08T00:00:00Z'
                     FROM store_payment_channels WHERE adapter_kind = 'epay'"
                ),
            ))
            .await?;
        }

        commit_with_foreign_key_check(tx, backend).await
    }

    async fn revert(&self, manager: &SchemaManager<'_>, backend: DbBackend) -> Result<(), DbErr> {
        // The legacy Alipay and WeChat configuration is destroyed by `up`, so `down` only
        // restores the permitted adapter kinds. It never recreates removed credentials.
        let tx = manager.get_connection().begin().await?;
        tx.execute(Statement::from_string(
            backend,
            "DROP TABLE store_epay_methods".to_string(),
        ))
        .await?;
        // An EPay Channel cannot exist under the restored constraint. Remove the draft when no
        // order references it and map every surviving EPay Channel back to Alipay.
        for table in [
            "store_channel_credentials",
            "store_payment_compliance",
            "store_merchant_capabilities",
            "store_channel_readiness_profiles",
        ] {
            tx.execute(Statement::from_string(
                backend,
                format!(
                    "DELETE FROM {table} WHERE channel_id = '{EPAY_DRAFT_ID}'
                     AND NOT EXISTS (SELECT 1 FROM store_orders
                                     WHERE payment_channel_id = '{EPAY_DRAFT_ID}')"
                ),
            ))
            .await?;
        }
        tx.execute(Statement::from_string(
            backend,
            format!(
                "DELETE FROM store_payment_channels
                 WHERE id = '{EPAY_DRAFT_ID}'
                   AND id NOT IN (SELECT payment_channel_id FROM store_orders)"
            ),
        ))
        .await?;
        if backend == DbBackend::Sqlite {
            for sql in sqlite_restore_statements() {
                tx.execute_unprepared(&sql).await?;
            }
        } else {
            // A row-level CHECK is evaluated immediately and cannot be deferred, so
            // the constraint must accept 'alipay' before any row is mapped back to
            // it. Mapping first would abort the transaction on the still-active
            // `epay|stripe|http` constraint and leave `down` unable to complete.
            tx.execute(Statement::from_string(
                backend,
                "ALTER TABLE store_payment_channels
                 DROP CONSTRAINT ck_store_payment_channels_adapter"
                    .to_string(),
            ))
            .await?;
            tx.execute(Statement::from_string(
                backend,
                "ALTER TABLE store_payment_channels
                 ADD CONSTRAINT ck_store_payment_channels_adapter
                 CHECK (adapter_kind IN ('alipay', 'wechat', 'stripe', 'http'))"
                    .to_string(),
            ))
            .await?;
            tx.execute(Statement::from_string(
                backend,
                "UPDATE store_payment_channels SET adapter_kind = 'alipay', enabled = 0
                 WHERE adapter_kind = 'epay'"
                    .to_string(),
            ))
            .await?;
        }
        commit_with_foreign_key_check(tx, backend).await
    }
}

/// Reports whether the SQLite connection currently enforces foreign keys.
async fn sqlite_foreign_keys_enabled(
    connection: &SchemaManagerConnection<'_>,
) -> Result<bool, DbErr> {
    let row = connection
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "PRAGMA foreign_keys".to_string(),
        ))
        .await?;
    match row {
        Some(row) => Ok(i32::try_get(&row, "", "foreign_keys")? != 0),
        None => Ok(false),
    }
}

/// Commits the rebuild only when no dangling reference remains. `PRAGMA foreign_key_check`
/// replaces the enforcement that stayed disabled for the rebuild.
async fn commit_with_foreign_key_check(
    tx: sea_orm::DatabaseTransaction,
    backend: DbBackend,
) -> Result<(), DbErr> {
    if backend == DbBackend::Sqlite {
        let violations = tx
            .query_all(Statement::from_string(
                backend,
                "PRAGMA foreign_key_check".to_string(),
            ))
            .await?;
        if !violations.is_empty() {
            return Err(DbErr::Custom(
                "store_epay_migration_left_dangling_references".to_string(),
            ));
        }
    }
    tx.commit().await
}

/// SQLite cannot alter a CHECK constraint, so the table is rebuilt in place. `legacy_alter_table`
/// keeps the rename from rewriting the `store_orders` foreign key, which must keep naming
/// `store_payment_channels` while the new table takes that name. Renaming instead of dropping
/// also avoids the implicit `DELETE FROM` that would cascade every readiness profile away.
fn sqlite_rebuild_statements() -> Vec<String> {
    vec![
        "ALTER TABLE store_payment_channels RENAME TO store_payment_channels_old".to_string(),
        channel_table("store_payment_channels", "('epay', 'stripe', 'http')"),
        format!(
            "INSERT INTO store_payment_channels
                (id, adapter_kind, name, icon_kind, icon_value, sort_order, enabled,
                 revision, created_at, updated_at)
             SELECT id,
                    CASE WHEN adapter_kind IN {LEGACY_KINDS} THEN 'epay' ELSE adapter_kind END,
                    name, icon_kind, icon_value, sort_order,
                    CASE WHEN adapter_kind IN {LEGACY_KINDS} THEN 0 ELSE enabled END,
                    revision + CASE WHEN adapter_kind IN {LEGACY_KINDS} THEN 1 ELSE 0 END,
                    created_at, updated_at
             FROM store_payment_channels_old"
        ),
        "DROP TABLE store_payment_channels_old".to_string(),
        // `RENAME TO` moves the migration-049 index onto the old table, so dropping
        // that table takes the index with it. Without recreating it the catalog
        // query degrades to a full table scan.
        CHANNEL_CATALOG_INDEX.to_string(),
    ]
}

/// Recreated after every table rebuild; the definition matches migration 049.
const CHANNEL_CATALOG_INDEX: &str = "CREATE INDEX IF NOT EXISTS idx_store_payment_channels_catalog ON store_payment_channels (enabled, sort_order, created_at, id)";

fn sqlite_restore_statements() -> Vec<String> {
    vec![
        "ALTER TABLE store_payment_channels RENAME TO store_payment_channels_old".to_string(),
        channel_table(
            "store_payment_channels",
            "('alipay', 'wechat', 'stripe', 'http')",
        ),
        "INSERT INTO store_payment_channels
            (id, adapter_kind, name, icon_kind, icon_value, sort_order, enabled,
             revision, created_at, updated_at)
         SELECT id,
                CASE WHEN adapter_kind = 'epay' THEN 'alipay' ELSE adapter_kind END,
                name, icon_kind, icon_value, sort_order,
                CASE WHEN adapter_kind = 'epay' THEN 0 ELSE enabled END,
                revision, created_at, updated_at
         FROM store_payment_channels_old"
            .to_string(),
        "DROP TABLE store_payment_channels_old".to_string(),
        CHANNEL_CATALOG_INDEX.to_string(),
    ]
}

fn channel_table(name: &str, adapter_kinds: &str) -> String {
    format!(
        "CREATE TABLE {name} (
            id TEXT NOT NULL PRIMARY KEY,
            adapter_kind TEXT NOT NULL CHECK (adapter_kind IN {adapter_kinds}),
            name TEXT NOT NULL,
            icon_kind TEXT NOT NULL,
            icon_value TEXT,
            sort_order INTEGER NOT NULL DEFAULT 0,
            enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
            revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )"
    )
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}
