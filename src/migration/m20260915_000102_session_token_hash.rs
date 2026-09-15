use sea_orm::Statement;
use sea_orm_migration::prelude::*;
use sha2::{Digest, Sha256};

#[derive(DeriveMigrationName)]
pub struct Migration;

/// CF-71: `sessions.token` held the session token verbatim, so anyone who could read the
/// table could authenticate as any logged-in user. The column now holds the token's
/// SHA-256 hex digest.
///
/// Existing rows are converted in place. The plaintext format is
/// `urp_session_<32 hex>`, which is never 64 hex characters, so a digest and a plaintext
/// token cannot be confused. A row already holding a digest is left alone, which makes
/// this migration safe to re-run.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let rows = db
            .query_all(Statement::from_string(
                db.get_database_backend(),
                "SELECT id, token FROM sessions".to_string(),
            ))
            .await?;

        for row in rows {
            let id: String = row.try_get("", "id")?;
            let token: String = row.try_get("", "token")?;
            if is_hex_digest(&token) {
                continue;
            }
            let digest: String = Sha256::digest(token.as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            db.execute(Statement::from_sql_and_values(
                db.get_database_backend(),
                "UPDATE sessions SET token = $1 WHERE id = $2",
                vec![digest.into(), id.into()],
            ))
            .await?;
        }
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // A digest cannot be reversed, so the plaintext tokens cannot be restored. The
        // only correct rollback is to invalidate every session, which the previous image
        // tolerates: its lookups simply miss and every user logs in again.
        Err(DbErr::Migration(
            "session tokens are one-way hashed and cannot be restored; delete sessions and redeploy the previous image to force re-login".to_string(),
        ))
    }
}

/// A SHA-256 hex digest is exactly 64 lowercase hex characters. A plaintext token is
/// `urp_session_` plus 32 hex characters, so it never matches.
fn is_hex_digest(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit())
}
