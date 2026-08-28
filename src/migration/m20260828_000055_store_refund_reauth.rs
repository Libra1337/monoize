use sea_orm::{ConnectionTrait, DbBackend, Statement, TransactionTrait};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        migrate_scope(manager, true).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        migrate_scope(manager, false).await
    }
}

async fn migrate_scope(manager: &SchemaManager<'_>, include_refund: bool) -> Result<(), DbErr> {
    let backend = manager.get_database_backend();
    if !matches!(backend, DbBackend::Sqlite | DbBackend::Postgres) {
        return Ok(());
    }
    let tx = manager.get_connection().begin().await?;
    if backend == DbBackend::Postgres {
        if !include_refund {
            tx.execute(Statement::from_string(
                backend,
                "DELETE FROM store_reauth_grants WHERE scope = 'refund'".to_string(),
            ))
            .await?;
        }
        tx.execute(Statement::from_string(
            backend,
            "ALTER TABLE store_reauth_grants DROP CONSTRAINT IF EXISTS store_reauth_grants_scope_check"
                .to_string(),
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            format!(
                "ALTER TABLE store_reauth_grants ADD CONSTRAINT store_reauth_grants_scope_check CHECK (scope IN ({}))",
                allowed_scopes(include_refund)
            ),
        ))
        .await?;
    } else {
        tx.execute(Statement::from_string(
            backend,
            "DROP INDEX IF EXISTS uq_store_reauth_token_digest".to_string(),
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            "DROP INDEX IF EXISTS idx_store_reauth_expiry".to_string(),
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            "ALTER TABLE store_reauth_grants RENAME TO store_reauth_grants_v055".to_string(),
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            sqlite_table_sql(include_refund),
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            format!(
                "INSERT INTO store_reauth_grants
                    (id, user_id, session_token_digest, token_digest, scope, created_at, expires_at)
                 SELECT id, user_id, session_token_digest, token_digest, scope, created_at, expires_at
                 FROM store_reauth_grants_v055{}",
                if include_refund {
                    ""
                } else {
                    " WHERE scope <> 'refund'"
                }
            ),
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            "DROP TABLE store_reauth_grants_v055".to_string(),
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            "CREATE UNIQUE INDEX uq_store_reauth_token_digest ON store_reauth_grants (token_digest)"
                .to_string(),
        ))
        .await?;
        tx.execute(Statement::from_string(
            backend,
            "CREATE INDEX idx_store_reauth_expiry ON store_reauth_grants (expires_at, id)"
                .to_string(),
        ))
        .await?;
    }
    tx.commit().await
}

fn allowed_scopes(include_refund: bool) -> &'static str {
    if include_refund {
        "'credential_update', 'redemption_access', 'compliance_confirm', 'refund'"
    } else {
        "'credential_update', 'redemption_access', 'compliance_confirm'"
    }
}

fn sqlite_table_sql(include_refund: bool) -> String {
    format!(
        "CREATE TABLE store_reauth_grants (
            id TEXT PRIMARY KEY NOT NULL,
            user_id TEXT NOT NULL,
            session_token_digest TEXT NOT NULL,
            token_digest TEXT NOT NULL,
            scope TEXT NOT NULL CHECK (scope IN ({})),
            created_at TEXT NOT NULL,
            expires_at TEXT NOT NULL,
            FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
        )",
        allowed_scopes(include_refund)
    )
}
