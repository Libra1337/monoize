use sea_orm::{DbBackend, Statement};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let Some(statement) = create_statement(backend) else {
            return Ok(());
        };
        manager
            .get_connection()
            .execute(Statement::from_string(backend, statement))
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if !matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            return Ok(());
        }
        manager
            .get_connection()
            .execute(Statement::from_string(
                backend,
                "DROP TABLE IF EXISTS store_payment_icons".to_string(),
            ))
            .await?;
        Ok(())
    }
}

fn create_statement(backend: DbBackend) -> Option<String> {
    let binary_type = match backend {
        DbBackend::Sqlite => "BLOB",
        DbBackend::Postgres => "BYTEA",
        _ => return None,
    };
    Some(format!(
        "CREATE TABLE IF NOT EXISTS store_payment_icons (id TEXT NOT NULL PRIMARY KEY, content_type TEXT NOT NULL, content {binary_type} NOT NULL, created_at TEXT NOT NULL, CONSTRAINT ck_store_payment_icons_content_type CHECK (content_type IN ('image/png', 'image/jpeg', 'image/webp', 'image/svg+xml')))"
    ))
}

#[cfg(test)]
mod tests {
    use super::{Migration, create_statement};
    use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
    use sea_orm_migration::{MigrationTrait, SchemaManager};

    #[tokio::test]
    async fn sqlite_schema_round_trips_exact_binary_content() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect SQLite");
        Migration
            .up(&SchemaManager::new(&db))
            .await
            .expect("apply icon migration");
        let columns = db
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT name, type FROM pragma_table_info('store_payment_icons') ORDER BY cid"
                    .to_string(),
            ))
            .await
            .expect("query icon columns")
            .into_iter()
            .map(|row| {
                (
                    row.try_get::<String>("", "name").expect("column name"),
                    row.try_get::<String>("", "type").expect("column type"),
                )
            })
            .collect::<Vec<_>>();
        assert!(columns.contains(&("content".to_string(), "BLOB".to_string())));

        let content = vec![0_u8, 1, 2, 0xff];
        db.execute(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "INSERT INTO store_payment_icons (id, content_type, content, created_at) VALUES (?, ?, ?, ?)",
            [
                "icon-1".into(),
                "image/png".into(),
                content.clone().into(),
                "2026-08-27T00:00:00Z".into(),
            ],
        ))
        .await
        .expect("insert icon bytes");
        let row = db
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT content FROM store_payment_icons WHERE id = 'icon-1'".to_string(),
            ))
            .await
            .expect("query icon bytes")
            .expect("icon row exists");
        assert_eq!(
            row.try_get::<Vec<u8>>("", "content").expect("icon bytes"),
            content
        );

        Migration
            .down(&SchemaManager::new(&db))
            .await
            .expect("revert icon migration");
        let table = db
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'store_payment_icons'"
                    .to_string(),
            ))
            .await
            .expect("query icon table after down");
        assert!(table.is_none());
    }

    #[test]
    fn postgres_schema_uses_bytea() {
        let ddl = create_statement(DbBackend::Postgres).expect("PostgreSQL DDL");
        assert!(ddl.contains("content BYTEA NOT NULL"));
        assert!(!ddl.contains(" BLOB "));
    }
}
