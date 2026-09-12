use sea_orm::{ConnectionTrait, Statement, TransactionTrait};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Organization spaces (orgs.spec.md): org wallet rows are flagged on `users`, org metadata
/// and membership live in their own tables, and API keys gain org-sharing columns.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let tx = manager.get_connection().begin().await?;
        for statement in [
            "ALTER TABLE users ADD COLUMN is_org INTEGER NOT NULL DEFAULT 0",
            "CREATE TABLE orgs (
                id TEXT NOT NULL PRIMARY KEY,
                owner_user_id TEXT NOT NULL,
                display_name TEXT NOT NULL,
                avatar_emoji TEXT NOT NULL DEFAULT '🏢',
                avatar_color TEXT NOT NULL DEFAULT '#6366f1',
                avatar_image TEXT,
                invite_token TEXT NOT NULL UNIQUE,
                invite_expires_at TEXT,
                invite_created_at TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
            "CREATE INDEX idx_orgs_owner ON orgs (owner_user_id)",
            "CREATE TABLE org_members (
                org_id TEXT NOT NULL,
                user_id TEXT NOT NULL,
                role TEXT NOT NULL CHECK (role IN ('owner', 'member')),
                joined_at TEXT NOT NULL,
                PRIMARY KEY (org_id, user_id)
            )",
            "CREATE INDEX idx_org_members_user ON org_members (user_id)",
            "CREATE TABLE org_key_shares (
                api_key_id TEXT NOT NULL,
                member_user_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                PRIMARY KEY (api_key_id, member_user_id)
            )",
            "ALTER TABLE api_keys ADD COLUMN org_id TEXT",
            "ALTER TABLE api_keys ADD COLUMN org_share_mode TEXT",
        ] {
            tx.execute(Statement::from_string(backend, statement.to_string()))
                .await?;
        }
        tx.commit().await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let tx = manager.get_connection().begin().await?;
        for statement in [
            "ALTER TABLE api_keys DROP COLUMN org_share_mode",
            "ALTER TABLE api_keys DROP COLUMN org_id",
            "DROP TABLE org_key_shares",
            "DROP TABLE org_members",
            "DROP TABLE orgs",
            "ALTER TABLE users DROP COLUMN is_org",
        ] {
            tx.execute(Statement::from_string(backend, statement.to_string()))
                .await?;
        }
        tx.commit().await
    }
}
