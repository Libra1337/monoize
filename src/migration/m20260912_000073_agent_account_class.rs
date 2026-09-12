use sea_orm::{ConnectionTrait, DbBackend, Statement, TransactionTrait};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Admits `agent` as the fourth account class (GR-E1b).
///
/// The CHECK constraints created by migration 063 and widened by migration 071 name the
/// three permitted values, so every one of them rejects the new value until it is widened
/// again. The SQLite path edits the stored DDL under `PRAGMA writable_schema` for the same
/// reason migration 071 documents: a table rebuild would silently delete every API key and
/// session through the `ON DELETE CASCADE` on `users`.
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
                    replace_sqlite_check(&tx, backend, table, &columns, THREE_CLASS_CHECK, FOUR_CLASS_CHECK)
                        .await?;
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
                             CHECK ({column} IN ('standard', 'enterprise', 'private', 'agent'))"
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
        // Narrowing the constraint would reject any row that already uses the agent class,
        // so those rows move back to the default class first.
        for statement in [
            "UPDATE users SET account_class = 'standard' WHERE account_class = 'agent'",
            "UPDATE monoize_groups SET account_class = 'standard' WHERE account_class = 'agent'",
            "DELETE FROM user_account_class_audits
             WHERE from_account_class = 'agent' OR to_account_class = 'agent'",
        ] {
            tx.execute(Statement::from_string(backend, statement.to_string()))
                .await?;
        }
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
                    replace_sqlite_check(
                        &tx,
                        backend,
                        table,
                        &columns,
                        FOUR_CLASS_CHECK,
                        THREE_CLASS_CHECK,
                    )
                    .await?;
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
}

const THREE_CLASS_CHECK: &str = "IN ('standard', 'enterprise', 'private')";
const FOUR_CLASS_CHECK: &str = "IN ('standard', 'enterprise', 'private', 'agent')";

/// Rewrites one table's account-class `CHECK` text in place. See migration 071 for why the
/// DDL is edited under `PRAGMA writable_schema` instead of rebuilding the table.
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
