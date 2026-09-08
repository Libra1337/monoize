use sea_orm::{ConnectionTrait, DbBackend, Statement, TransactionTrait};
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
        let tx = manager.get_connection().begin().await?;
        tx.execute(Statement::from_string(
            backend,
            "ALTER TABLE api_keys ADD COLUMN channel_bindings TEXT NOT NULL DEFAULT '[]'"
                .to_string(),
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            "ALTER TABLE api_keys DROP COLUMN use_user_group".to_string(),
        ))
        .await?;
        tx.commit().await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        if !matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
            return Ok(());
        }
        let tx = manager.get_connection().begin().await?;
        tx.execute(Statement::from_string(
            backend,
            "ALTER TABLE api_keys ADD COLUMN use_user_group INTEGER NOT NULL DEFAULT 0"
                .to_string(),
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            "ALTER TABLE api_keys DROP COLUMN channel_bindings".to_string(),
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
    async fn sqlite_replaces_default_group_flag_with_channel_bindings() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared(
            "CREATE TABLE api_keys (id TEXT PRIMARY KEY, use_user_group INTEGER NOT NULL DEFAULT 1, group_ids TEXT NOT NULL DEFAULT '[]')",
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
        assert!(names.contains(&"channel_bindings".to_string()));
        assert!(!names.contains(&"use_user_group".to_string()));
    }
}
