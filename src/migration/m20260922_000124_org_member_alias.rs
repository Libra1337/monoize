use sea_orm::{ConnectionTrait, Statement, TransactionTrait};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Org member aliases (orgs.spec.md ORG-3a): one optional display alias per
/// org-membership row so members can recognize each other by nickname, shown
/// next to the username wherever org members and key creators are listed.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let is_sqlite = matches!(backend, sea_orm::DbBackend::Sqlite);
        let tx = manager.get_connection().begin().await?;
        let statement = "ALTER TABLE org_members ADD COLUMN alias TEXT";
        tx.execute(Statement::from_string(
            backend,
            if is_sqlite {
                statement.to_string()
            } else {
                statement.replace("ADD COLUMN", "ADD COLUMN IF NOT EXISTS")
            },
        ))
        .await?;
        tx.commit().await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let tx = manager.get_connection().begin().await?;
        // SQLite cannot DROP COLUMN on all supported builds via plain ALTER;
        // recreate without the column is out of scope for a down migration.
        if !matches!(backend, sea_orm::DbBackend::Sqlite) {
            tx.execute(Statement::from_string(
                backend,
                "ALTER TABLE org_members DROP COLUMN IF EXISTS alias",
            ))
            .await?;
        }
        tx.commit().await
    }
}
