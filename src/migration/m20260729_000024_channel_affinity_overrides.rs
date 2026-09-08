use sea_orm::{ConnectionTrait, DbBackend, Statement};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        let backend = manager.get_database_backend();

        add_column_if_missing(conn, backend, "affinity_enabled_override", "INTEGER").await?;
        add_column_if_missing(
            conn,
            backend,
            "affinity_idle_ttl_seconds_override",
            "INTEGER",
        )
        .await?;
        add_column_if_missing(conn, backend, "affinity_failback_mode_override", "TEXT").await?;
        add_column_if_missing(
            conn,
            backend,
            "affinity_failback_delay_seconds_override",
            "INTEGER",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}

async fn add_column_if_missing(
    conn: &SchemaManagerConnection<'_>,
    backend: DbBackend,
    column: &str,
    definition: &str,
) -> Result<(), DbErr> {
    if column_exists(conn, backend, column).await? {
        return Ok(());
    }
    let sql = match backend {
        DbBackend::Sqlite => {
            format!("ALTER TABLE monoize_channels ADD COLUMN {column} {definition}")
        }
        DbBackend::Postgres => {
            format!("ALTER TABLE monoize_channels ADD COLUMN IF NOT EXISTS {column} {definition}")
        }
        _ => return Ok(()),
    };
    conn.execute(Statement::from_string(backend, sql)).await?;
    Ok(())
}

async fn column_exists(
    conn: &SchemaManagerConnection<'_>,
    backend: DbBackend,
    column: &str,
) -> Result<bool, DbErr> {
    let (sql, values) = match backend {
        DbBackend::Sqlite => (
            "SELECT COUNT(*) AS n FROM pragma_table_info('monoize_channels') WHERE name = ?",
            vec![column.into()],
        ),
        DbBackend::Postgres => (
            "SELECT COUNT(*) AS n FROM information_schema.columns WHERE table_schema = current_schema() AND table_name = $1 AND column_name = $2",
            vec!["monoize_channels".into(), column.into()],
        ),
        _ => return Ok(false),
    };
    let row = conn
        .query_one(Statement::from_sql_and_values(backend, sql, values))
        .await?;
    let count: i64 = row
        .and_then(|value| value.try_get("", "n").ok())
        .unwrap_or(0);
    Ok(count > 0)
}
