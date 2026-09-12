use sea_orm::{ConnectionTrait, Statement, TransactionTrait};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Org admin controls (orgs.spec.md): per-user creation quota, per-org member
/// cap, and a short invite code alongside the invite link. NULL keeps the
/// compile-time default; the admin endpoints overwrite it.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let tx = manager.get_connection().begin().await?;
        for statement in [
            "ALTER TABLE users ADD COLUMN org_creation_limit INTEGER",
            "ALTER TABLE orgs ADD COLUMN max_members INTEGER",
            "ALTER TABLE orgs ADD COLUMN invite_code TEXT",
        ] {
            tx.execute(Statement::from_string(backend, statement.to_string()))
                .await?;
        }
        // Backfill a unique unambiguous code per existing org.
        let rows = tx
            .query_all(Statement::from_string(
                backend,
                "SELECT id FROM orgs WHERE invite_code IS NULL OR invite_code = ''".to_string(),
            ))
            .await?;
        for row in rows {
            let id: String = row.try_get("", "id")?;
            let mut code = generate_invite_code();
            let mut attempts = 0;
            loop {
                let taken = tx
                    .query_one(Statement::from_sql_and_values(
                        backend,
                        "SELECT id FROM orgs WHERE invite_code = $1",
                        [code.clone().into()],
                    ))
                    .await?;
                if taken.is_none() {
                    break;
                }
                attempts += 1;
                if attempts > 32 {
                    return Err(DbErr::Custom("invite code backfill exhausted".into()));
                }
                code = generate_invite_code();
            }
            tx.execute(Statement::from_sql_and_values(
                backend,
                "UPDATE orgs SET invite_code = $2 WHERE id = $1",
                [id.into(), code.into()],
            ))
            .await?;
        }
        tx.execute(Statement::from_string(
            backend,
            "CREATE UNIQUE INDEX uq_orgs_invite_code ON orgs (invite_code)".to_string(),
        ))
        .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let tx = manager.get_connection().begin().await?;
        tx.execute(Statement::from_string(
            backend,
            "DROP INDEX uq_orgs_invite_code".to_string(),
        ))
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

const CODE_ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";

fn generate_invite_code() -> String {
    // UUID v4 bytes carry 122 random bits; taking 6 alphabet-indexed bytes is
    // enough for a display code, and the unique index plus retry loop above
    // guarantees uniqueness even on a collision.
    let uuid = uuid::Uuid::new_v4();
    (0..6)
        .map(|i| CODE_ALPHABET[(uuid.as_bytes()[i] as usize) % CODE_ALPHABET.len()] as char)
        .collect()
}
