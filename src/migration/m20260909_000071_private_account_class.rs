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
                // Rewriting the stored DDL keeps every column, default, and unrelated
                // constraint intact, which restating the schema by hand would not.
                tx.execute(Statement::from_string(
                    backend,
                    "PRAGMA legacy_alter_table = ON".to_string(),
                ))
                .await?;
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
                tx.execute(Statement::from_string(
                    backend,
                    "PRAGMA legacy_alter_table = OFF".to_string(),
                ))
                .await?;
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
                tx.execute(Statement::from_string(
                    backend,
                    "PRAGMA legacy_alter_table = ON".to_string(),
                ))
                .await?;
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
                tx.execute(Statement::from_string(
                    backend,
                    "PRAGMA legacy_alter_table = OFF".to_string(),
                ))
                .await?;
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

/// Rebuilds one SQLite table with its account-class `CHECK` text replaced.
///
/// The replacement is applied only to the named columns' constraint text, so a table whose
/// other columns happen to contain the same literal is not altered elsewhere.
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

    let mut rebuilt = ddl.clone();
    for column in columns {
        // The constraint text may quote the column name; the surrounding CHECK keeps the
        // match unambiguous either way.
        let candidates = [
            (format!("{column} {from}"), format!("{column} {to}")),
            (format!("\"{column}\" {from}"), format!("\"{column}\" {to}")),
        ];
        let Some((needle, replacement)) = candidates
            .iter()
            .find(|(needle, _)| rebuilt.contains(needle.as_str()))
        else {
            return Err(DbErr::Custom(format!(
                "table {table} has no account-class check for {column}: {rebuilt}"
            )));
        };
        rebuilt = rebuilt.replace(needle.as_str(), replacement.as_str());
    }
    // The stored DDL may or may not quote the table name, depending on how it was created,
    // so both forms are tried before giving up rather than silently emitting a statement that
    // recreates the original table.
    let next_table = format!("{table}_account_class_next");
    let rebuilt = {
        let quoted = format!("CREATE TABLE \"{table}\"");
        let bare = format!("CREATE TABLE {table}");
        let target = format!("CREATE TABLE {next_table}");
        if rebuilt.contains(&quoted) {
            rebuilt.replacen(&quoted, &target, 1)
        } else if rebuilt.contains(&bare) {
            rebuilt.replacen(&bare, &target, 1)
        } else {
            return Err(DbErr::Custom(format!(
                "table {table} has unrecognized DDL: {rebuilt}"
            )));
        }
    };

    let indexes = tx
        .query_all(Statement::from_sql_and_values(
            backend,
            "SELECT sql FROM sqlite_master
             WHERE type = 'index' AND tbl_name = ?1 AND sql IS NOT NULL",
            [table.into()],
        ))
        .await?
        .into_iter()
        .map(|row| row.try_get::<String>("", "sql"))
        .collect::<Result<Vec<_>, _>>()?;

    tx.execute(Statement::from_string(backend, rebuilt)).await?;
    tx.execute(Statement::from_string(
        backend,
        format!("INSERT INTO {next_table} SELECT * FROM {table}"),
    ))
    .await?;
    tx.execute(Statement::from_string(
        backend,
        format!("DROP TABLE {table}"),
    ))
    .await?;
    tx.execute(Statement::from_string(
        backend,
        format!("ALTER TABLE {next_table} RENAME TO {table}"),
    ))
    .await?;
    // Dropping the table dropped its indexes, so they are recreated from their stored DDL.
    for index in indexes {
        tx.execute(Statement::from_string(backend, index)).await?;
    }
    Ok(())
}
