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
        manager
            .get_connection()
            .execute(Statement::from_string(
                backend,
                "ALTER TABLE users ADD COLUMN usage_ranking_anonymous INTEGER NOT NULL DEFAULT 1"
                    .to_string(),
            ))
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
                "ALTER TABLE users DROP COLUMN usage_ranking_anonymous".to_string(),
            ))
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Migration;
    use sea_orm::{ConnectionTrait, Database, Statement};
    use sea_orm_migration::{MigrationTrait, SchemaManager};

    #[tokio::test]
    async fn sqlite_migrates_existing_users_to_anonymous() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared("CREATE TABLE users (id TEXT PRIMARY KEY)")
            .await
            .unwrap();
        db.execute_unprepared("INSERT INTO users (id) VALUES ('existing')")
            .await
            .unwrap();
        Migration.up(&SchemaManager::new(&db)).await.unwrap();
        let row = db
            .query_one(Statement::from_string(
                sea_orm::DbBackend::Sqlite,
                "SELECT usage_ranking_anonymous FROM users WHERE id = 'existing'".to_string(),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row.try_get::<i32>("", "usage_ranking_anonymous").unwrap(),
            1
        );
    }
}
