use sea_orm::{ConnectionTrait, DbBackend, Statement};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if !matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            return Ok(());
        }

        let connection = manager.get_connection();
        connection
            .execute(Statement::from_string(
                backend,
                "CREATE INDEX IF NOT EXISTS idx_api_keys_key ON api_keys(key)".to_string(),
            ))
            .await?;
        connection
            .execute(Statement::from_string(
                backend,
                "DROP INDEX IF EXISTS idx_api_keys_key_hash".to_string(),
            ))
            .await?;

        if column_exists(connection, backend, "api_keys", "key_hash").await? {
            let sql = match backend {
                DbBackend::Postgres => {
                    "ALTER TABLE api_keys DROP COLUMN IF EXISTS key_hash".to_string()
                }
                _ => "ALTER TABLE api_keys DROP COLUMN key_hash".to_string(),
            };
            connection
                .execute(Statement::from_string(backend, sql))
                .await?;
        }

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // The plaintext-only storage decision forbids restoring the removed hash column.
        Ok(())
    }
}

async fn column_exists(
    connection: &SchemaManagerConnection<'_>,
    backend: DbBackend,
    table: &str,
    column: &str,
) -> Result<bool, DbErr> {
    let sql = match backend {
        DbBackend::Sqlite => format!("PRAGMA table_info({table})"),
        DbBackend::Postgres => format!(
            "SELECT column_name AS name FROM information_schema.columns WHERE table_schema = current_schema() AND table_name = '{table}'"
        ),
        _ => return Ok(false),
    };
    let rows = connection
        .query_all(Statement::from_string(backend, sql))
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| row.try_get::<String>("", "name").ok())
        .any(|name| name == column))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{Database, DbBackend};

    async fn sqlite_columns(db: &sea_orm::DatabaseConnection) -> Vec<String> {
        db.query_all(Statement::from_string(
            DbBackend::Sqlite,
            "PRAGMA table_info(api_keys)".to_string(),
        ))
        .await
        .expect("columns query")
        .into_iter()
        .map(|row| row.try_get("", "name").expect("column name decodes"))
        .collect()
    }

    async fn sqlite_indexes(db: &sea_orm::DatabaseConnection) -> Vec<String> {
        db.query_all(Statement::from_string(
            DbBackend::Sqlite,
            "PRAGMA index_list(api_keys)".to_string(),
        ))
        .await
        .expect("indexes query")
        .into_iter()
        .map(|row| row.try_get("", "name").expect("index name decodes"))
        .collect()
    }

    #[tokio::test]
    async fn removes_legacy_hash_storage_and_preserves_plaintext_keys() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("database connects");
        db.execute_unprepared(
            "CREATE TABLE api_keys (id TEXT PRIMARY KEY, key TEXT NOT NULL, key_hash TEXT NOT NULL);
             CREATE INDEX idx_api_keys_key_hash ON api_keys(key_hash);
             INSERT INTO api_keys (id, key, key_hash) VALUES ('k1', 'sk-plaintext', 'legacy-hash');",
        )
        .await
        .expect("legacy schema seeds");

        Migration
            .up(&SchemaManager::new(&db))
            .await
            .expect("migration succeeds");

        let columns = sqlite_columns(&db).await;
        assert!(columns.contains(&"key".to_string()));
        assert!(!columns.contains(&"key_hash".to_string()));
        let indexes = sqlite_indexes(&db).await;
        assert!(indexes.contains(&"idx_api_keys_key".to_string()));
        assert!(!indexes.contains(&"idx_api_keys_key_hash".to_string()));
        let key: String = db
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT key FROM api_keys WHERE id = 'k1'".to_string(),
            ))
            .await
            .expect("key query succeeds")
            .expect("key row exists")
            .try_get("", "key")
            .expect("key decodes");
        assert_eq!(key, "sk-plaintext");
    }

    #[tokio::test]
    async fn accepts_a_schema_that_is_already_plaintext_only() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("database connects");
        db.execute_unprepared(
            "CREATE TABLE api_keys (id TEXT PRIMARY KEY, key TEXT NOT NULL);
             CREATE INDEX idx_api_keys_key ON api_keys(key);",
        )
        .await
        .expect("plaintext schema seeds");

        Migration
            .up(&SchemaManager::new(&db))
            .await
            .expect("migration succeeds");

        assert_eq!(sqlite_columns(&db).await, vec!["id", "key"]);
        assert!(
            sqlite_indexes(&db)
                .await
                .contains(&"idx_api_keys_key".to_string())
        );
    }
}
