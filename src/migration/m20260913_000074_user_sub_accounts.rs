use sea_orm::{ConnectionTrait, Statement, TransactionTrait};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Adds `users.parent_user_id` (SAU-1). A NULL value marks an ordinary main account; a
/// non-NULL value references the owning main account and marks the row a sub-account.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let tx = manager.get_connection().begin().await?;
        tx.execute(Statement::from_string(
            backend,
            "ALTER TABLE users ADD COLUMN parent_user_id TEXT".to_string(),
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            "CREATE INDEX idx_users_parent_user_id ON users (parent_user_id)".to_string(),
        ))
        .await?;
        tx.commit().await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let tx = manager.get_connection().begin().await?;
        tx.execute(Statement::from_string(
            backend,
            "DROP INDEX idx_users_parent_user_id".to_string(),
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            "ALTER TABLE users DROP COLUMN parent_user_id".to_string(),
        ))
        .await?;
        tx.commit().await
    }
}
