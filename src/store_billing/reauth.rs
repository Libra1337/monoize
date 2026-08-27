use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rsa::rand_core::{OsRng, RngCore};
use sea_orm::ConnectionTrait;
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::db::DbPool;

const GRANT_TTL: Duration = Duration::minutes(5);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReauthGrant {
    pub token: String,
    pub scope: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReauthError {
    #[error("reauthentication scope is invalid")]
    InvalidScope,
    #[error("reauthentication grant is invalid")]
    InvalidGrant,
    #[error("reauthentication storage failed: {0}")]
    Storage(String),
}

#[derive(Debug, Clone)]
pub struct ReauthStore {
    db: DbPool,
}

impl ReauthStore {
    pub fn new(db: DbPool) -> Self {
        Self { db }
    }

    pub async fn issue(
        &self,
        user_id: &str,
        session_token: &str,
        scope: &str,
    ) -> Result<ReauthGrant, ReauthError> {
        validate_scope(scope)?;
        if user_id.is_empty() || session_token.is_empty() {
            return Err(ReauthError::InvalidGrant);
        }
        let mut random = [0_u8; 32];
        OsRng.fill_bytes(&mut random);
        let token = URL_SAFE_NO_PAD.encode(random);
        let now = Utc::now();
        let expires_at = now + GRANT_TTL;
        self.delete_expired_before(now - Duration::hours(24))
            .await?;
        self.db
            .write()
            .await
            .execute(self.db.stmt(
                "INSERT INTO store_reauth_grants
                    (id, user_id, session_token_digest, token_digest, scope, created_at, expires_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
                vec![
                    Uuid::new_v4().to_string().into(),
                    user_id.into(),
                    digest(session_token).into(),
                    digest(&token).into(),
                    scope.into(),
                    timestamp(now).into(),
                    timestamp(expires_at).into(),
                ],
            ))
            .await
            .map_err(storage)?;
        Ok(ReauthGrant {
            token,
            scope: scope.to_string(),
            expires_at,
        })
    }

    pub async fn verify(
        &self,
        user_id: &str,
        session_token: &str,
        token: &str,
        scope: &str,
    ) -> Result<(), ReauthError> {
        validate_scope(scope)?;
        if user_id.is_empty() || session_token.is_empty() || token.is_empty() {
            return Err(ReauthError::InvalidGrant);
        }
        let row = self
            .db
            .read()
            .query_one(self.db.stmt(
                "SELECT expires_at FROM store_reauth_grants
                 WHERE user_id = $1 AND session_token_digest = $2
                   AND token_digest = $3 AND scope = $4",
                vec![
                    user_id.into(),
                    digest(session_token).into(),
                    digest(token).into(),
                    scope.into(),
                ],
            ))
            .await
            .map_err(storage)?
            .ok_or(ReauthError::InvalidGrant)?;
        let expires_at: String = row.try_get("", "expires_at").map_err(storage)?;
        let expires_at = DateTime::parse_from_rfc3339(&expires_at)
            .map_err(|_| ReauthError::InvalidGrant)?
            .with_timezone(&Utc);
        if expires_at <= Utc::now() {
            return Err(ReauthError::InvalidGrant);
        }
        Ok(())
    }

    pub async fn delete_expired_before(&self, cutoff: DateTime<Utc>) -> Result<u64, ReauthError> {
        self.db
            .write()
            .await
            .execute(self.db.stmt(
                "DELETE FROM store_reauth_grants WHERE expires_at <= $1",
                vec![timestamp(cutoff).into()],
            ))
            .await
            .map(|result| result.rows_affected())
            .map_err(storage)
    }
}

fn validate_scope(scope: &str) -> Result<(), ReauthError> {
    if matches!(scope, "credential_update" | "redemption_access") {
        Ok(())
    } else {
        Err(ReauthError::InvalidScope)
    }
}

fn digest(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn storage(error: impl ToString) -> ReauthError {
    ReauthError::Storage(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::Migrator;
    use sea_orm::ConnectionTrait;
    use sea_orm_migration::MigratorTrait;

    async fn store() -> ReauthStore {
        let db = DbPool::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&*db.write().await, None).await.unwrap();
        ReauthStore::new(db)
    }

    #[tokio::test]
    async fn grant_is_bound_to_the_user_session_and_scope() {
        let store = store().await;
        let grant = store
            .issue("admin-a", "session-a", "credential_update")
            .await
            .unwrap();

        store
            .verify("admin-a", "session-a", &grant.token, "credential_update")
            .await
            .unwrap();
        assert_eq!(
            store
                .verify("admin-a", "session-b", &grant.token, "credential_update")
                .await,
            Err(ReauthError::InvalidGrant)
        );
        assert_eq!(
            store
                .verify("admin-b", "session-a", &grant.token, "credential_update")
                .await,
            Err(ReauthError::InvalidGrant)
        );
        assert_eq!(
            store
                .verify("admin-a", "session-a", &grant.token, "refund")
                .await,
            Err(ReauthError::InvalidScope)
        );
    }

    #[tokio::test]
    async fn expired_grant_is_rejected_and_old_hashes_are_deleted() {
        let store = store().await;
        let grant = store
            .issue("admin-a", "session-a", "credential_update")
            .await
            .unwrap();
        store
            .db
            .write()
            .await
            .execute(store.db.stmt(
                "UPDATE store_reauth_grants SET expires_at = $1 WHERE token_digest = $2",
                vec![
                    timestamp(Utc::now() - Duration::hours(25)).into(),
                    digest(&grant.token).into(),
                ],
            ))
            .await
            .unwrap();

        assert_eq!(
            store
                .verify("admin-a", "session-a", &grant.token, "credential_update")
                .await,
            Err(ReauthError::InvalidGrant)
        );
        assert_eq!(
            store
                .delete_expired_before(Utc::now() - Duration::hours(24))
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn redemption_access_is_a_distinct_five_minute_scope() {
        let store = store().await;
        let grant = store
            .issue("admin-a", "session-a", "redemption_access")
            .await
            .unwrap();

        store
            .verify("admin-a", "session-a", &grant.token, "redemption_access")
            .await
            .unwrap();
        assert_eq!(
            store
                .verify("admin-a", "session-a", &grant.token, "credential_update")
                .await,
            Err(ReauthError::InvalidGrant)
        );
        assert!(grant.expires_at <= Utc::now() + Duration::minutes(5));
    }
}
