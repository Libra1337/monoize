use sea_orm::{ConnectionTrait, DbBackend, Statement, TransactionTrait};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Admits `private` as a third account class (GR-E1).
///
/// Four `CHECK` constraints created by migration 063 name the two permitted values, so every
/// one of them rejects the new value until it is widened. SQLite cannot alter a `CHECK`, so
/// each affected table is rebuilt and its rows copied; Postgres can drop and re-add the
/// constraint in place.
///
/// `users` and `monoize_groups` carry unrelated columns that differ across deployments, so
/// the SQLite rebuild copies them by name through `INSERT INTO ... SELECT *`, which requires
/// the rebuilt table to declare the same columns in the same order. That is why the rebuild
/// reads the existing DDL and rewrites only the constraint text rather than restating the
/// schema.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let tx = manager.get_connection().begin().await?;
        match backend {
            DbBackend::Sqlite => {
                for (table, columns) in [
                    ("users", vec!["account_class"]),
                    ("monoize_groups", vec!["account_class"]),
                    (
                        "user_account_class_audits",
                        vec!["from_account_class", "to_account_class"],
                    ),
                ] {
                    rebuild_sqlite_table(&tx, backend, table, &columns).await?;
                }
            }
            _ => {
                for (table, column) in [
                    ("users", "account_class"),
                    ("monoize_groups", "account_class"),
                    ("user_account_class_audits", "from_account_class"),
                    ("user_account_class_audits", "to_account_class"),
                ] {
                    tx.execute(Statement::from_string(
                        backend,
                        format!(
                            "ALTER TABLE {table} DROP CONSTRAINT IF EXISTS {table}_{column}_check"
                        ),
                    ))
                    .await?;
                    tx.execute(Statement::from_string(
                        backend,
                        format!(
                            "ALTER TABLE {table} ADD CONSTRAINT {table}_{column}_check
                             CHECK ({column} IN ('standard', 'enterprise', 'private'))"
                        ),
                    ))
                    .await?;
                }
            }
        }
        tx.commit().await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let tx = manager.get_connection().begin().await?;
        // Narrowing the constraint again would reject any row that already uses the new
        // value, so those rows are moved back to the default class first.
        tx.execute(Statement::from_string(
            backend,
            "UPDATE users SET account_class = 'standard' WHERE account_class = 'private'"
                .to_string(),
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            "UPDATE monoize_groups SET account_class = 'standard' WHERE account_class = 'private'"
                .to_string(),
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            "DELETE FROM user_account_class_audits
             WHERE from_account_class = 'private' OR to_account_class = 'private'"
                .to_string(),
        ))
        .await?;
        match backend {
            DbBackend::Sqlite => {
                for (table, columns) in [
                    ("users", vec!["account_class"]),
                    ("monoize_groups", vec!["account_class"]),
                    (
                        "user_account_class_audits",
                        vec!["from_account_class", "to_account_class"],
                    ),
                ] {
                    narrow_sqlite_table(&tx, backend, table, &columns).await?;
                }
            }
            _ => {
                for (table, column) in [
                    ("users", "account_class"),
                    ("monoize_groups", "account_class"),
                    ("user_account_class_audits", "from_account_class"),
                    ("user_account_class_audits", "to_account_class"),
                ] {
                    tx.execute(Statement::from_string(
                        backend,
                        format!(
                            "ALTER TABLE {table} DROP CONSTRAINT IF EXISTS {table}_{column}_check"
                        ),
                    ))
                    .await?;
                    tx.execute(Statement::from_string(
                        backend,
                        format!(
                            "ALTER TABLE {table} ADD CONSTRAINT {table}_{column}_check
                             CHECK ({column} IN ('standard', 'enterprise'))"
                        ),
                    ))
                    .await?;
                }
            }
        }
        tx.commit().await
    }
}

const TWO_CLASS_CHECK: &str = "IN ('standard', 'enterprise')";
const THREE_CLASS_CHECK: &str = "IN ('standard', 'enterprise', 'private')";

async fn rebuild_sqlite_table<C: ConnectionTrait>(
    tx: &C,
    backend: DbBackend,
    table: &str,
    columns: &[&str],
) -> Result<(), DbErr> {
    replace_sqlite_check(tx, backend, table, columns, TWO_CLASS_CHECK, THREE_CLASS_CHECK).await
}

async fn narrow_sqlite_table<C: ConnectionTrait>(
    tx: &C,
    backend: DbBackend,
    table: &str,
    columns: &[&str],
) -> Result<(), DbErr> {
    replace_sqlite_check(tx, backend, table, columns, THREE_CLASS_CHECK, TWO_CLASS_CHECK).await
}

/// Rewrites one table's account-class `CHECK` text in place.
///
/// The obvious approach, rebuilding the table and copying rows, cannot be used here.
/// `api_keys` and `sessions` reference `users` with `ON DELETE CASCADE`, so the rebuild's
/// `DROP TABLE users` does not fail: it silently deletes every API key and every session.
/// `PRAGMA foreign_keys` is a no-op inside a transaction, and sea-orm runs each migration in
/// one, so the constraint cannot be turned off around the rebuild either.
///
/// Editing `sqlite_master` under `PRAGMA writable_schema` changes only the stored DDL. No row
/// is read, written, or deleted, no cascade fires, and every index survives. `writable_schema
/// = RESET` then makes the connection re-read the schema, and `integrity_check` confirms the
/// edited DDL parses.
async fn replace_sqlite_check<C: ConnectionTrait>(
    tx: &C,
    backend: DbBackend,
    table: &str,
    columns: &[&str],
    from: &str,
    to: &str,
) -> Result<(), DbErr> {
    let row = tx
        .query_one(Statement::from_sql_and_values(
            backend,
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table.into()],
        ))
        .await?
        .ok_or_else(|| DbErr::Custom(format!("table {table} is missing")))?;
    let ddl: String = row.try_get("", "sql")?;

    let mut rewritten = ddl.clone();
    for column in columns {
        // The constraint text may quote the column name; the surrounding CHECK keeps the
        // match unambiguous either way.
        let candidates = [
            (format!("{column} {from}"), format!("{column} {to}")),
            (format!("\"{column}\" {from}"), format!("\"{column}\" {to}")),
        ];
        let Some((needle, replacement)) = candidates
            .iter()
            .find(|(needle, _)| rewritten.contains(needle.as_str()))
        else {
            return Err(DbErr::Custom(format!(
                "table {table} has no account-class check for {column}: {rewritten}"
            )));
        };
        rewritten = rewritten.replace(needle.as_str(), replacement.as_str());
    }
    if rewritten == ddl {
        return Err(DbErr::Custom(format!("table {table} DDL did not change")));
    }

    tx.execute(Statement::from_string(
        backend,
        "PRAGMA writable_schema = ON".to_string(),
    ))
    .await?;
    let updated = tx
        .execute(Statement::from_sql_and_values(
            backend,
            "UPDATE sqlite_master SET sql = ?1 WHERE type = 'table' AND name = ?2",
            [rewritten.into(), table.into()],
        ))
        .await;
    // The schema must be re-read whether or not the update succeeded, or the connection is
    // left able to write to `sqlite_master`.
    let reset = tx
        .execute(Statement::from_string(
            backend,
            "PRAGMA writable_schema = RESET".to_string(),
        ))
        .await;
    let updated = updated?;
    reset?;
    if updated.rows_affected() != 1 {
        return Err(DbErr::Custom(format!(
            "table {table} schema row was not updated"
        )));
    }

    let check = tx
        .query_one(Statement::from_string(
            backend,
            "PRAGMA integrity_check".to_string(),
        ))
        .await?
        .ok_or_else(|| DbErr::Custom("integrity_check returned no row".to_string()))?;
    let result: String = check.try_get("", "integrity_check")?;
    if result != "ok" {
        return Err(DbErr::Custom(format!(
            "schema edit left the database inconsistent: {result}"
        )));
    }
    Ok(())
}
