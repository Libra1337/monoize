use sea_orm::{ConnectionTrait, Statement, TransactionTrait};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// TM-MB-8: per-key Group pin for a logical model whose name appears in more
/// than one Group. An empty array is the fail-closed default.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let tx = manager.get_connection().begin().await?;
        tx.execute(Statement::from_string(
            backend,
            "ALTER TABLE api_keys ADD COLUMN model_bindings TEXT NOT NULL DEFAULT '[]'"
                .to_string(),
        ))
        .await?;
        tx.commit().await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let tx = manager.get_connection().begin().await?;
        tx.execute(Statement::from_string(
            backend,
            "ALTER TABLE api_keys DROP COLUMN model_bindings".to_string(),
        ))
        .await?;
        tx.commit().await
    }
}

#[cfg(test)]
mod tests {
    use super::Migration;
    use sea_orm::{ConnectionTrait, Database};
    use sea_orm_migration::{MigrationTrait, SchemaManager};

    #[tokio::test]
    async fn sqlite_adds_model_bindings_column() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared(
            "CREATE TABLE api_keys (id TEXT PRIMARY KEY, channel_bindings TEXT NOT NULL DEFAULT '[]')",
        )
        .await
        .unwrap();
        Migration.up(&SchemaManager::new(&db)).await.unwrap();
        let columns = db
            .query_all(sea_orm::Statement::from_string(
                sea_orm::DbBackend::Sqlite,
                "PRAGMA table_info(api_keys)".to_string(),
            ))
            .await
            .unwrap();
        let names = columns
            .iter()
            .map(|row| row.try_get::<String>("", "name").unwrap())
            .collect::<Vec<_>>();
        assert!(names.contains(&"model_bindings".to_string()));
    }
}
