use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use ed25519_dalek::{SigningKey, VerifyingKey};
use sea_orm::{ConnectionTrait, DbErr, QueryResult};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use super::admission_token::{
    ADMISSION_ISSUER, AdmissionKeyRing, AdmissionSigningKey, AdmissionTokenInput,
    PriorAdmissionSigningKey, TerminalKind,
};
use super::crypto::{CryptoError, EncryptedSecret, PaymentKeyRing};
use super::quota::{QuotaError, QuotaReservationInput, reserve_tx, terminal_tx};
use crate::db::DbPool;

const TOKEN_TTL_SECONDS: i64 = 30;
const CLOCK_SKEW_SECONDS: i64 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueAdmissionInput {
    pub audience: String,
    pub user_id: String,
    pub request_id: String,
    pub effective_groups: Vec<String>,
    pub maximum_nano_usd: i128,
    pub pricing_revision: String,
    pub issued_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedAdmission {
    pub token_id: String,
    pub reservation_id: String,
    pub compact_jws: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub duplicate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmAdmissionInput {
    pub audience: String,
    pub token_id: String,
    pub reservation_id: String,
    pub request_id: String,
    pub confirmed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmAdmissionResult {
    pub duplicate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmissionDecision {
    Balance,
    Plan(IssuedAdmission),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicAdmissionKey {
    pub key_id: String,
    pub public_key_base64: String,
    pub state: String,
    pub activated_at: DateTime<Utc>,
    pub verify_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StoredKeyState {
    Active,
    Retired,
}

impl StoredKeyState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Retired => "retired",
        }
    }
}

struct StoredAdmissionKey {
    key_id: String,
    public_key_base64: String,
    state: StoredKeyState,
    activated_at: DateTime<Utc>,
    retired_at: Option<DateTime<Utc>>,
    last_issued_expires_at: Option<DateTime<Utc>>,
    verify_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalApplyInput {
    pub token_id: String,
    pub reservation_id: String,
    pub request_id: String,
    pub audience: String,
    pub kind: TerminalKind,
    pub actual_nano_usd: Option<i128>,
    pub canonical_digest: String,
    pub applied_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalApplyResult {
    Applied,
    Duplicate,
}

#[derive(Debug, Error)]
pub enum AdmissionRuntimeError {
    #[error("admission input is invalid")]
    InputInvalid,
    #[error("active admission key is missing")]
    ActiveKeyMissing,
    #[error("admission wrap key is missing")]
    WrapKeyMissing,
    #[error("admission key is invalid")]
    KeyInvalid,
    #[error("admission issuer is invalid")]
    IssuerInvalid,
    #[error("admission issue conflicts with the stored request")]
    IssueConflict,
    #[error("admission token is not found")]
    TokenNotFound,
    #[error("admission binding does not match")]
    BindingMismatch,
    #[error("admission terminal digest is invalid")]
    TerminalDigestInvalid,
    #[error("admission terminal conflicts with the stored receipt")]
    TerminalConflict,
    #[error("admission confirmation is expired")]
    ConfirmationExpired,
    #[error("admission storage failed: {0}")]
    Storage(String),
    #[error(transparent)]
    Quota(#[from] QuotaError),
}

impl AdmissionRuntimeError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InputInvalid => "admission_input_invalid",
            Self::ActiveKeyMissing => "admission_active_key_missing",
            Self::WrapKeyMissing => "admission_wrap_key_missing",
            Self::KeyInvalid => "admission_key_invalid",
            Self::IssuerInvalid => "admission_issuer_invalid",
            Self::IssueConflict => "admission_issue_conflict",
            Self::TokenNotFound => "admission_token_not_found",
            Self::BindingMismatch => "admission_binding_mismatch",
            Self::TerminalDigestInvalid => "admission_terminal_digest_invalid",
            Self::TerminalConflict => "admission_terminal_conflict",
            Self::ConfirmationExpired => "admission_confirmation_expired",
            Self::Storage(_) => "admission_storage_error",
            Self::Quota(error) => error.code(),
        }
    }
}

impl From<DbErr> for AdmissionRuntimeError {
    fn from(value: DbErr) -> Self {
        storage(value)
    }
}

#[derive(Clone)]
pub struct AdmissionService {
    db: DbPool,
    wrap_keys: Option<Arc<PaymentKeyRing>>,
    issuer: String,
}

impl AdmissionService {
    pub fn new(
        db: DbPool,
        wrap_keys: impl Into<Option<Arc<PaymentKeyRing>>>,
        issuer: impl Into<String>,
    ) -> Result<Self, AdmissionRuntimeError> {
        let issuer = issuer.into();
        if issuer != ADMISSION_ISSUER {
            return Err(AdmissionRuntimeError::IssuerInvalid);
        }
        Ok(Self {
            db,
            wrap_keys: wrap_keys.into(),
            issuer,
        })
    }

    pub async fn issue(
        &self,
        mut input: IssueAdmissionInput,
    ) -> Result<AdmissionDecision, AdmissionRuntimeError> {
        validate_issue(&input)?;
        input
            .effective_groups
            .sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        input.effective_groups.dedup();
        if self.db.is_sqlite() {
            let service = self.clone();
            self.db
                .with_immediate_write(move |connection| {
                    Box::pin(async move { service.issue_tx(connection, input).await })
                })
                .await
        } else {
            let tx = self.db.begin_write().await.map_err(storage)?;
            let outcome = self.issue_tx(&*tx, input).await;
            match outcome {
                Ok(value) => {
                    tx.commit().await.map_err(storage)?;
                    Ok(value)
                }
                Err(error) => {
                    tx.rollback().await.map_err(storage)?;
                    Err(error)
                }
            }
        }
    }

    pub async fn public_keyset(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<PublicAdmissionKey>, AdmissionRuntimeError> {
        let rows = self
            .db
            .read()
            .query_all(self.db.stmt(
                "SELECT key_id, public_key_base64, state, published_at, activated_at,
                        retired_at, last_issued_expires_at, verify_until
                 FROM store_admission_keys
                 WHERE state IN ('active', 'retired')
                 ORDER BY key_id",
                vec![],
            ))
            .await
            .map_err(storage)?;
        let mut keys = Vec::new();
        for row in rows {
            let key = stored_key_from_row(&row)?;
            validate_public_key(&key.public_key_base64)?;
            if key.state == StoredKeyState::Retired
                && key.verify_until.is_some_and(|until| until <= now)
            {
                continue;
            }
            keys.push(PublicAdmissionKey {
                key_id: key.key_id,
                public_key_base64: key.public_key_base64,
                state: key.state.as_str().to_string(),
                activated_at: key.activated_at,
                verify_until: key.verify_until,
            });
        }
        Ok(keys)
    }

    pub async fn confirm(
        &self,
        input: ConfirmAdmissionInput,
    ) -> Result<ConfirmAdmissionResult, AdmissionRuntimeError> {
        validate_confirmation(&input)?;
        if self.db.is_sqlite() {
            let service = self.clone();
            self.db
                .with_immediate_write(move |connection| {
                    Box::pin(async move { service.confirm_tx(connection, input).await })
                })
                .await
        } else {
            let tx = self.db.begin_write().await.map_err(storage)?;
            let outcome = self.confirm_tx(&*tx, input).await;
            match outcome {
                Ok(value) => {
                    tx.commit().await.map_err(storage)?;
                    Ok(value)
                }
                Err(error) => {
                    tx.rollback().await.map_err(storage)?;
                    Err(error)
                }
            }
        }
    }

    pub async fn recover_unconfirmed(
        &self,
        now: DateTime<Utc>,
        limit: usize,
    ) -> Result<usize, AdmissionRuntimeError> {
        let rows = self
            .db
            .read()
            .query_all(self.db.stmt(
                unconfirmed_recovery_candidate_sql(),
                vec![
                    (now.timestamp() - CLOCK_SKEW_SECONDS).into(),
                    i64::try_from(limit.min(100)).unwrap_or(100).into(),
                ],
            ))
            .await
            .map_err(storage)?;
        let candidates = rows
            .into_iter()
            .map(|row| Ok((row_string(&row, "token_id")?, stored_expiry(&row)?)))
            .collect::<Result<Vec<_>, AdmissionRuntimeError>>()?;
        let mut applied = 0usize;
        for (token_id, _) in candidates {
            let outcome = if self.db.is_sqlite() {
                let service = self.clone();
                self.db
                    .with_immediate_write(move |connection| {
                        Box::pin(async move {
                            service
                                .recover_unconfirmed_tx(connection, &token_id, now)
                                .await
                        })
                    })
                    .await?
            } else {
                let tx = self.db.begin_write().await.map_err(storage)?;
                let outcome = self.recover_unconfirmed_tx(&*tx, &token_id, now).await;
                match outcome {
                    Ok(value) => {
                        tx.commit().await.map_err(storage)?;
                        value
                    }
                    Err(error) => {
                        tx.rollback().await.map_err(storage)?;
                        return Err(error);
                    }
                }
            };
            if outcome == Some(TerminalApplyResult::Applied) {
                applied += 1;
            }
        }
        Ok(applied)
    }

    pub async fn apply_terminal(
        &self,
        input: TerminalApplyInput,
    ) -> Result<TerminalApplyResult, AdmissionRuntimeError> {
        validate_terminal(&input)?;
        if terminal_digest(&input)? != input.canonical_digest {
            return Err(AdmissionRuntimeError::TerminalDigestInvalid);
        }
        if self.db.is_sqlite() {
            let service = self.clone();
            self.db
                .with_immediate_write(move |connection| {
                    Box::pin(async move { service.apply_terminal_tx(connection, input).await })
                })
                .await
        } else {
            let tx = self.db.begin_write().await.map_err(storage)?;
            let outcome = self.apply_terminal_tx(&*tx, input).await;
            match outcome {
                Ok(value) => {
                    tx.commit().await.map_err(storage)?;
                    Ok(value)
                }
                Err(error) => {
                    tx.rollback().await.map_err(storage)?;
                    Err(error)
                }
            }
        }
    }

    async fn issue_tx<C: ConnectionTrait>(
        &self,
        connection: &C,
        input: IssueAdmissionInput,
    ) -> Result<AdmissionDecision, AdmissionRuntimeError> {
        if self.db.is_postgres() {
            let (first, second) = issue_advisory_lock_keys(&input.audience, &input.request_id);
            connection
                .query_one(self.db.stmt(
                    issue_advisory_lock_sql(true),
                    vec![first.into(), second.into()],
                ))
                .await
                .map_err(storage)?
                .ok_or_else(|| {
                    AdmissionRuntimeError::Storage(
                        "PostgreSQL admission advisory lock returned no row".to_string(),
                    )
                })?;
        }
        let groups_json = serde_json::to_string(&input.effective_groups).map_err(serialize)?;
        let lock = if self.db.is_postgres() {
            " FOR UPDATE"
        } else {
            ""
        };
        if let Some(row) = connection
            .query_one(self.db.stmt(
                &format!(
                    "SELECT token_id, reservation_id, compact_jws, issued_at, expires_at,
                            user_id, effective_groups_json, maximum_nano_usd, pricing_revision
                     FROM store_admission_tokens
                     WHERE audience = $1 AND request_id = $2{lock}"
                ),
                vec![
                    input.audience.clone().into(),
                    input.request_id.clone().into(),
                ],
            ))
            .await
            .map_err(storage)?
        {
            if row_string(&row, "user_id")? != input.user_id
                || row_string(&row, "effective_groups_json")? != groups_json
                || parse_amount(&row_string(&row, "maximum_nano_usd")?)? != input.maximum_nano_usd
                || row_string(&row, "pricing_revision")? != input.pricing_revision
            {
                return Err(AdmissionRuntimeError::IssueConflict);
            }
            return Ok(AdmissionDecision::Plan(IssuedAdmission {
                token_id: row_string(&row, "token_id")?,
                reservation_id: row_string(&row, "reservation_id")?,
                compact_jws: row_string(&row, "compact_jws")?,
                issued_at: row_time(&row, "issued_at")?,
                expires_at: row_time(&row, "expires_at")?,
                duplicate: true,
            }));
        }

        let current = connection
            .query_one(self.db.stmt(
                &format!(
                    "SELECT g.id, g.generation, g.group_ids, g.starts_at, g.ends_at,
                            l.suspended_at, l.revoked_at
                     FROM store_plan_entitlement_current p
                     JOIN store_plan_entitlement_generations g ON g.id = p.entitlement_id
                     JOIN store_plan_entitlement_lifecycle l ON l.entitlement_id = g.id
                     WHERE p.user_id = $1{lock}"
                ),
                vec![input.user_id.clone().into()],
            ))
            .await
            .map_err(storage)?;
        let Some(current) = current else {
            return Ok(AdmissionDecision::Balance);
        };
        let starts_at = entitlement_time(&current, "starts_at")?;
        let ends_at = entitlement_time(&current, "ends_at")?;
        let suspended_at = optional_entitlement_time(&current, "suspended_at")?;
        let revoked_at = optional_entitlement_time(&current, "revoked_at")?;
        if starts_at > input.issued_at
            || ends_at <= input.issued_at
            || suspended_at.is_some()
            || revoked_at.is_some()
        {
            return Ok(AdmissionDecision::Balance);
        }
        let entitlement_id = row_string(&current, "id")?;
        let generation = row_i64(&current, "generation")?;
        let entitlement_groups: Vec<String> =
            serde_json::from_str(&row_string(&current, "group_ids")?).map_err(serialize)?;
        if !entitlement_groups.is_empty()
            && !entitlement_groups
                .iter()
                .any(|group| input.effective_groups.iter().any(|value| value == group))
        {
            return Ok(AdmissionDecision::Balance);
        }
        let (ring, active_key_id, active_last_expiry) =
            self.load_key_ring(connection, input.issued_at).await?;
        let quota_request_id = quota_request_key(&input.audience, &input.request_id);
        let reservation = reserve_tx(
            &self.db,
            connection,
            QuotaReservationInput {
                user_id: input.user_id.clone(),
                request_id: quota_request_id,
                maximum_nano_usd: input.maximum_nano_usd,
                pricing_revision: input.pricing_revision.clone(),
                now: input.issued_at,
            },
            Some((entitlement_id, generation)),
        )
        .await
        .map_err(AdmissionRuntimeError::from)?;
        let token_id = Uuid::new_v4().to_string();
        let compact_jws = ring
            .issue(AdmissionTokenInput {
                audience: input.audience.clone(),
                token_id: token_id.clone(),
                reservation_id: reservation.id.clone(),
                request_id: input.request_id.clone(),
                entitlement_id: reservation.entitlement_id.clone(),
                generation: reservation.generation,
                maximum_nano_usd: reservation.maximum_nano_usd,
                reserved_fen_cny: reservation.reserved_fen_cny,
                pricing_revision: reservation.pricing_revision.clone(),
                issued_at: input.issued_at,
            })
            .map_err(|_| AdmissionRuntimeError::KeyInvalid)?;
        let expires_at = input.issued_at + chrono::Duration::seconds(TOKEN_TTL_SECONDS);
        let digest = hex_digest(Sha256::digest(compact_jws.as_bytes()));
        connection
            .execute(self.db.stmt(
                "INSERT INTO store_admission_tokens
                 (token_id, audience, request_id, user_id, effective_groups_json,
                  reservation_id, entitlement_id, generation, maximum_nano_usd,
                  reserved_fen_cny, pricing_revision, key_id, compact_jws,
                   compact_jws_digest, issued_at, expires_at, expires_at_unix, confirmed_at)
                  VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)",
                vec![
                    token_id.clone().into(),
                    input.audience.into(),
                    input.request_id.into(),
                    input.user_id.into(),
                    groups_json.into(),
                    reservation.id.clone().into(),
                    reservation.entitlement_id.into(),
                    reservation.generation.into(),
                    reservation.maximum_nano_usd.to_string().into(),
                    reservation.reserved_fen_cny.to_string().into(),
                    reservation.pricing_revision.into(),
                    active_key_id.clone().into(),
                    compact_jws.clone().into(),
                    digest.into(),
                    timestamp(input.issued_at).into(),
                    timestamp(expires_at).into(),
                    expires_at.timestamp().into(),
                    Option::<String>::None.into(),
                ],
            ))
            .await
            .map_err(storage)?;
        let key_updated = connection
            .execute(self.db.stmt(
                "UPDATE store_admission_keys
                 SET last_issued_expires_at = $2
                 WHERE key_id = $1 AND state = 'active'",
                vec![
                    active_key_id.into(),
                    timestamp(active_last_expiry.map_or(expires_at, |value| value.max(expires_at)))
                        .into(),
                ],
            ))
            .await
            .map_err(storage)?;
        if key_updated.rows_affected() != 1 {
            return Err(AdmissionRuntimeError::KeyInvalid);
        }
        Ok(AdmissionDecision::Plan(IssuedAdmission {
            token_id,
            reservation_id: reservation.id,
            compact_jws,
            issued_at: input.issued_at,
            expires_at,
            duplicate: false,
        }))
    }

    async fn load_key_ring<C: ConnectionTrait>(
        &self,
        connection: &C,
        now: DateTime<Utc>,
    ) -> Result<(AdmissionKeyRing, String, Option<DateTime<Utc>>), AdmissionRuntimeError> {
        let rows = connection
            .query_all(
                self.db
                    .stmt(&signing_key_select_sql(self.db.is_postgres()), vec![]),
            )
            .await
            .map_err(storage)?;
        let mut active = None;
        let mut prior = Vec::new();
        for row in rows {
            let stored = stored_key_from_row(&row)?;
            if stored.state == StoredKeyState::Retired
                && stored.verify_until.is_some_and(|until| until <= now)
            {
                continue;
            }
            validate_public_key(&stored.public_key_base64)?;
            let seed = self.decrypt_seed(&row, &stored.key_id)?;
            let key =
                AdmissionSigningKey::from_seed(stored.key_id.clone(), seed, stored.activated_at)
                    .map_err(|_| AdmissionRuntimeError::KeyInvalid)?;
            match stored.state {
                StoredKeyState::Active if active.is_none() => {
                    active = Some((stored.key_id, key, stored.last_issued_expires_at));
                }
                StoredKeyState::Retired => prior.push(PriorAdmissionSigningKey {
                    key,
                    deactivated_at: stored.retired_at.ok_or(AdmissionRuntimeError::KeyInvalid)?,
                    last_issued_expires_at: stored.last_issued_expires_at,
                    verify_until: stored.verify_until,
                }),
                _ => return Err(AdmissionRuntimeError::KeyInvalid),
            }
        }
        let (active_id, active, active_last_expiry) =
            active.ok_or(AdmissionRuntimeError::ActiveKeyMissing)?;
        AdmissionKeyRing::new(self.issuer.clone(), active, prior)
            .map(|ring| (ring, active_id, active_last_expiry))
            .map_err(|_| AdmissionRuntimeError::KeyInvalid)
    }

    fn decrypt_seed(
        &self,
        row: &QueryResult,
        key_id: &str,
    ) -> Result<[u8; 32], AdmissionRuntimeError> {
        let encrypted: EncryptedSecret =
            serde_json::from_str(&row_string(row, "encrypted_private_key_json")?)
                .map_err(|_| AdmissionRuntimeError::KeyInvalid)?;
        let plaintext = self
            .wrap_keys
            .as_ref()
            .ok_or(AdmissionRuntimeError::WrapKeyMissing)?
            .decrypt(&format!("store-admission-key:{key_id}:seed:v1"), &encrypted)
            .map_err(|error| match error {
                CryptoError::UnknownKey => AdmissionRuntimeError::WrapKeyMissing,
                _ => AdmissionRuntimeError::KeyInvalid,
            })?;
        let seed = <[u8; 32]>::try_from(plaintext.as_slice())
            .map_err(|_| AdmissionRuntimeError::KeyInvalid)?;
        let expected =
            URL_SAFE_NO_PAD.encode(SigningKey::from_bytes(&seed).verifying_key().as_bytes());
        if row_string(row, "public_key_base64")? != expected {
            return Err(AdmissionRuntimeError::KeyInvalid);
        }
        Ok(seed)
    }

    async fn confirm_tx<C: ConnectionTrait>(
        &self,
        connection: &C,
        input: ConfirmAdmissionInput,
    ) -> Result<ConfirmAdmissionResult, AdmissionRuntimeError> {
        let token = connection
            .query_one(self.db.stmt(
                &format!(
                    "SELECT audience, request_id, reservation_id, expires_at, confirmed_at
                     FROM store_admission_tokens WHERE token_id = $1{}",
                    if self.db.is_postgres() {
                        " FOR UPDATE"
                    } else {
                        ""
                    }
                ),
                vec![input.token_id.clone().into()],
            ))
            .await
            .map_err(storage)?
            .ok_or(AdmissionRuntimeError::TokenNotFound)?;
        if row_string(&token, "audience")? != input.audience
            || row_string(&token, "request_id")? != input.request_id
            || row_string(&token, "reservation_id")? != input.reservation_id
        {
            return Err(AdmissionRuntimeError::BindingMismatch);
        }
        if connection
            .query_one(self.db.stmt(
                "SELECT token_id FROM store_admission_terminal_receipts WHERE token_id = $1",
                vec![input.token_id.clone().into()],
            ))
            .await
            .map_err(storage)?
            .is_some()
        {
            return Err(AdmissionRuntimeError::ConfirmationExpired);
        }
        let reservation = connection
            .query_one(self.db.stmt(
                "SELECT state FROM store_quota_reservations WHERE id = $1",
                vec![input.reservation_id.clone().into()],
            ))
            .await
            .map_err(storage)?
            .ok_or(AdmissionRuntimeError::ConfirmationExpired)?;
        if row_string(&reservation, "state")? != "reserved" {
            return Err(AdmissionRuntimeError::ConfirmationExpired);
        }
        let confirmed_at: Option<String> = token.try_get("", "confirmed_at").map_err(storage)?;
        if let Some(value) = confirmed_at {
            DateTime::parse_from_rfc3339(&value).map_err(storage)?;
            return Ok(ConfirmAdmissionResult { duplicate: true });
        }
        if input.confirmed_at
            >= stored_time(&token, "expires_at")? + chrono::Duration::seconds(CLOCK_SKEW_SECONDS)
        {
            return Err(AdmissionRuntimeError::ConfirmationExpired);
        }
        let updated = connection
            .execute(self.db.stmt(
                "UPDATE store_admission_tokens SET confirmed_at = $2
                 WHERE token_id = $1 AND confirmed_at IS NULL",
                vec![input.token_id.into(), timestamp(input.confirmed_at).into()],
            ))
            .await
            .map_err(storage)?;
        if updated.rows_affected() != 1 {
            return Err(AdmissionRuntimeError::Storage(
                "admission confirmation update affected an unexpected row count".to_string(),
            ));
        }
        Ok(ConfirmAdmissionResult { duplicate: false })
    }

    async fn recover_unconfirmed_tx<C: ConnectionTrait>(
        &self,
        connection: &C,
        token_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<TerminalApplyResult>, AdmissionRuntimeError> {
        let token = connection
            .query_one(self.db.stmt(
                &format!(
                    "SELECT token_id, reservation_id, request_id, audience, expires_at,
                            expires_at_unix, confirmed_at
                     FROM store_admission_tokens WHERE token_id = $1{}",
                    if self.db.is_postgres() {
                        " FOR UPDATE"
                    } else {
                        ""
                    }
                ),
                vec![token_id.into()],
            ))
            .await
            .map_err(storage)?;
        let Some(token) = token else {
            return Ok(None);
        };
        let confirmed_at: Option<String> = token.try_get("", "confirmed_at").map_err(storage)?;
        if confirmed_at.is_some()
            || stored_expiry(&token)? + chrono::Duration::seconds(CLOCK_SKEW_SECONDS) > now
        {
            return Ok(None);
        }
        let mut terminal = TerminalApplyInput {
            token_id: row_string(&token, "token_id")?,
            reservation_id: row_string(&token, "reservation_id")?,
            request_id: row_string(&token, "request_id")?,
            audience: row_string(&token, "audience")?,
            kind: TerminalKind::Release,
            actual_nano_usd: None,
            canonical_digest: String::new(),
            applied_at: now,
        };
        terminal.canonical_digest = terminal_digest(&terminal)?;
        self.apply_terminal_tx(connection, terminal).await.map(Some)
    }

    pub(crate) async fn apply_terminal_tx<C: ConnectionTrait>(
        &self,
        connection: &C,
        input: TerminalApplyInput,
    ) -> Result<TerminalApplyResult, AdmissionRuntimeError> {
        let token = connection
            .query_one(self.db.stmt(
                &terminal_token_select_sql(self.db.is_postgres()),
                vec![input.token_id.clone().into()],
            ))
            .await
            .map_err(storage)?
            .ok_or(AdmissionRuntimeError::TokenNotFound)?;
        if let Some(receipt) = connection
            .query_one(self.db.stmt(
                "SELECT canonical_digest FROM store_admission_terminal_receipts
                 WHERE token_id = $1",
                vec![input.token_id.clone().into()],
            ))
            .await
            .map_err(storage)?
        {
            return if row_string(&receipt, "canonical_digest")? == input.canonical_digest {
                Ok(TerminalApplyResult::Duplicate)
            } else {
                Err(AdmissionRuntimeError::TerminalConflict)
            };
        }
        if row_string(&token, "reservation_id")? != input.reservation_id
            || row_string(&token, "request_id")? != input.request_id
            || row_string(&token, "audience")? != input.audience
        {
            return Err(AdmissionRuntimeError::BindingMismatch);
        }
        let confirmed_at: Option<String> = token.try_get("", "confirmed_at").map_err(storage)?;
        if input.kind == TerminalKind::Settlement && confirmed_at.is_none() {
            return Err(AdmissionRuntimeError::TerminalConflict);
        }
        terminal_tx(
            &self.db,
            connection,
            &input.reservation_id,
            input.actual_nano_usd,
            input.applied_at,
        )
        .await
        .map_err(|error| match error.code() {
            "quota_terminal_conflict" => AdmissionRuntimeError::TerminalConflict,
            _ => AdmissionRuntimeError::Storage(error.to_string()),
        })?;
        connection
            .execute(self.db.stmt(
                "INSERT INTO store_admission_terminal_receipts
                 (token_id, reservation_id, request_id, audience, terminal_kind,
                  actual_nano_usd, canonical_digest, applied_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                vec![
                    input.token_id.into(),
                    input.reservation_id.into(),
                    input.request_id.into(),
                    input.audience.into(),
                    terminal_kind(input.kind).into(),
                    input.actual_nano_usd.map(|value| value.to_string()).into(),
                    input.canonical_digest.into(),
                    timestamp(input.applied_at).into(),
                ],
            ))
            .await
            .map_err(storage)?;
        Ok(TerminalApplyResult::Applied)
    }
}

fn issue_advisory_lock_sql(postgres: bool) -> &'static str {
    if postgres {
        "SELECT pg_advisory_xact_lock($1, $2)"
    } else {
        ""
    }
}

fn signing_key_select_sql(postgres: bool) -> String {
    format!(
        "SELECT key_id, public_key_base64, encrypted_private_key_json, state,
                published_at, activated_at, retired_at, last_issued_expires_at, verify_until
         FROM store_admission_keys
         WHERE state IN ('active', 'retired')
         ORDER BY key_id{}",
        if postgres { " FOR UPDATE" } else { "" }
    )
}

fn terminal_token_select_sql(postgres: bool) -> String {
    format!(
        "SELECT reservation_id, request_id, audience, confirmed_at FROM store_admission_tokens
         WHERE token_id = $1{}",
        if postgres { " FOR UPDATE" } else { "" }
    )
}

fn unconfirmed_recovery_candidate_sql() -> &'static str {
    "SELECT token_id, expires_at, expires_at_unix FROM store_admission_tokens
     WHERE confirmed_at IS NULL AND expires_at_unix <= $1
       AND NOT EXISTS (
           SELECT 1 FROM store_admission_terminal_receipts receipt
           WHERE receipt.token_id = store_admission_tokens.token_id
       )
     ORDER BY expires_at_unix ASC, token_id ASC
     LIMIT $2"
}

fn validate_confirmation(input: &ConfirmAdmissionInput) -> Result<(), AdmissionRuntimeError> {
    validate_identifier(&input.audience)?;
    validate_identifier(&input.token_id)?;
    validate_identifier(&input.reservation_id)?;
    validate_identifier(&input.request_id)
}

fn admission_identity_digest(audience: &str, request_id: &str) -> [u8; 32] {
    Sha256::digest(format!("v=1\naudience={audience}\nrequest_id={request_id}\n").as_bytes()).into()
}

fn issue_advisory_lock_keys(audience: &str, request_id: &str) -> (i32, i32) {
    let digest = admission_identity_digest(audience, request_id);
    (
        i32::from_be_bytes(digest[0..4].try_into().expect("four-byte digest word")),
        i32::from_be_bytes(digest[4..8].try_into().expect("four-byte digest word")),
    )
}

fn quota_request_key(audience: &str, request_id: &str) -> String {
    format!(
        "admission:{}",
        hex_digest(admission_identity_digest(audience, request_id))
    )
}

pub fn terminal_digest(input: &TerminalApplyInput) -> Result<String, AdmissionRuntimeError> {
    validate_terminal(input)?;
    let actual = input
        .actual_nano_usd
        .map(|value| value.to_string())
        .unwrap_or_default();
    let canonical = format!(
        "v=1\ntoken_id={}\nreservation_id={}\nrequest_id={}\naudience={}\nkind={}\nactual_nano_usd={}\n",
        input.token_id,
        input.reservation_id,
        input.request_id,
        input.audience,
        terminal_kind(input.kind),
        actual,
    );
    Ok(hex_digest(Sha256::digest(canonical.as_bytes())))
}

fn validate_issue(input: &IssueAdmissionInput) -> Result<(), AdmissionRuntimeError> {
    validate_identifier(&input.audience)?;
    validate_identifier(&input.user_id)?;
    validate_identifier(&input.request_id)?;
    validate_identifier(&input.pricing_revision)?;
    for group in &input.effective_groups {
        validate_identifier(group)?;
    }
    if input.maximum_nano_usd <= 0 {
        return Err(AdmissionRuntimeError::InputInvalid);
    }
    Ok(())
}

fn validate_terminal(input: &TerminalApplyInput) -> Result<(), AdmissionRuntimeError> {
    validate_identifier(&input.token_id)?;
    validate_identifier(&input.reservation_id)?;
    validate_identifier(&input.request_id)?;
    validate_identifier(&input.audience)?;
    match (input.kind, input.actual_nano_usd) {
        (TerminalKind::Settlement, Some(value)) if value >= 0 => Ok(()),
        (TerminalKind::Release, None) => Ok(()),
        _ => Err(AdmissionRuntimeError::InputInvalid),
    }
}

fn validate_identifier(value: &str) -> Result<(), AdmissionRuntimeError> {
    if value.is_empty()
        || value.len() > 256
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(AdmissionRuntimeError::InputInvalid);
    }
    Ok(())
}

fn stored_key_from_row(row: &QueryResult) -> Result<StoredAdmissionKey, AdmissionRuntimeError> {
    let key_id = row_string(row, "key_id")?;
    validate_stored_key_id(&key_id)?;
    let public_key_base64 = row_string(row, "public_key_base64")?;
    let state = match row_string(row, "state")?.as_str() {
        "active" => StoredKeyState::Active,
        "retired" => StoredKeyState::Retired,
        _ => return Err(AdmissionRuntimeError::KeyInvalid),
    };
    let published_at = row_time(row, "published_at")?;
    let activated_at =
        row_optional_time(row, "activated_at")?.ok_or(AdmissionRuntimeError::KeyInvalid)?;
    let retired_at = row_optional_time(row, "retired_at")?;
    let last_issued_expires_at = row_optional_time(row, "last_issued_expires_at")?;
    let verify_until = row_optional_time(row, "verify_until")?;
    if published_at > activated_at
        || last_issued_expires_at.is_some_and(|value| value < activated_at)
    {
        return Err(AdmissionRuntimeError::KeyInvalid);
    }
    match state {
        StoredKeyState::Active if retired_at.is_none() && verify_until.is_none() => {}
        StoredKeyState::Retired
            if retired_at.is_some_and(|value| value >= activated_at)
                && verify_until
                    .is_some_and(|value| retired_at.is_some_and(|retired| value >= retired)) => {}
        _ => return Err(AdmissionRuntimeError::KeyInvalid),
    }
    Ok(StoredAdmissionKey {
        key_id,
        public_key_base64,
        state,
        activated_at,
        retired_at,
        last_issued_expires_at,
        verify_until,
    })
}

fn validate_stored_key_id(value: &str) -> Result<(), AdmissionRuntimeError> {
    validate_identifier(value).map_err(|_| AdmissionRuntimeError::KeyInvalid)
}

fn validate_public_key(value: &str) -> Result<(), AdmissionRuntimeError> {
    let decoded = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| AdmissionRuntimeError::KeyInvalid)?;
    let bytes =
        <[u8; 32]>::try_from(decoded.as_slice()).map_err(|_| AdmissionRuntimeError::KeyInvalid)?;
    if URL_SAFE_NO_PAD.encode(bytes) != value {
        return Err(AdmissionRuntimeError::KeyInvalid);
    }
    VerifyingKey::from_bytes(&bytes).map_err(|_| AdmissionRuntimeError::KeyInvalid)?;
    Ok(())
}

fn entitlement_time(
    row: &QueryResult,
    column: &str,
) -> Result<DateTime<Utc>, AdmissionRuntimeError> {
    let value = row
        .try_get::<String>("", column)
        .map_err(entitlement_storage)?;
    DateTime::parse_from_rfc3339(&value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(entitlement_storage)
}

fn optional_entitlement_time(
    row: &QueryResult,
    column: &str,
) -> Result<Option<DateTime<Utc>>, AdmissionRuntimeError> {
    let value = row
        .try_get::<Option<String>>("", column)
        .map_err(entitlement_storage)?;
    match value {
        Some(value) => DateTime::parse_from_rfc3339(&value)
            .map(|value| Some(value.with_timezone(&Utc)))
            .map_err(entitlement_storage),
        None => Ok(None),
    }
}

fn row_string(row: &QueryResult, column: &str) -> Result<String, AdmissionRuntimeError> {
    row.try_get("", column).map_err(storage)
}

fn row_i64(row: &QueryResult, column: &str) -> Result<i64, AdmissionRuntimeError> {
    row.try_get("", column).map_err(storage)
}

fn row_time(row: &QueryResult, column: &str) -> Result<DateTime<Utc>, AdmissionRuntimeError> {
    let value = row_string(row, column)?;
    DateTime::parse_from_rfc3339(&value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| AdmissionRuntimeError::KeyInvalid)
}

fn stored_time(row: &QueryResult, column: &str) -> Result<DateTime<Utc>, AdmissionRuntimeError> {
    let value = row_string(row, column)?;
    DateTime::parse_from_rfc3339(&value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(storage)
}

fn stored_expiry(row: &QueryResult) -> Result<DateTime<Utc>, AdmissionRuntimeError> {
    let expires_at = stored_time(row, "expires_at")?;
    let expires_at_unix = row.try_get::<i64>("", "expires_at_unix").map_err(storage)?;
    if expires_at.timestamp() != expires_at_unix {
        return Err(AdmissionRuntimeError::Storage(
            "stored admission expiry mirror does not match expires_at".to_string(),
        ));
    }
    Ok(expires_at)
}

fn row_optional_time(
    row: &QueryResult,
    column: &str,
) -> Result<Option<DateTime<Utc>>, AdmissionRuntimeError> {
    let value: Option<String> = row.try_get("", column).map_err(storage)?;
    value
        .map(|value| {
            DateTime::parse_from_rfc3339(&value)
                .map(|value| value.with_timezone(&Utc))
                .map_err(|_| AdmissionRuntimeError::KeyInvalid)
        })
        .transpose()
}

fn parse_amount(value: &str) -> Result<i128, AdmissionRuntimeError> {
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(AdmissionRuntimeError::Storage(
            "stored admission amount is not canonical".to_string(),
        ));
    }
    value.parse().map_err(storage)
}

fn terminal_kind(kind: TerminalKind) -> &'static str {
    match kind {
        TerminalKind::Settlement => "settlement",
        TerminalKind::Release => "release",
    }
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    let mut output = String::with_capacity(bytes.as_ref().len() * 2);
    for byte in bytes.as_ref() {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn storage(error: impl std::fmt::Display) -> AdmissionRuntimeError {
    AdmissionRuntimeError::Storage(error.to_string())
}

fn entitlement_storage(error: impl std::fmt::Display) -> AdmissionRuntimeError {
    AdmissionRuntimeError::Quota(QuotaError::Storage(error.to_string()))
}

fn serialize(error: impl std::fmt::Display) -> AdmissionRuntimeError {
    AdmissionRuntimeError::Storage(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        issue_advisory_lock_sql, signing_key_select_sql, terminal_token_select_sql,
        unconfirmed_recovery_candidate_sql,
    };

    #[test]
    fn postgres_runtime_sql_locks_issue_key_rows_and_terminal_token() {
        assert!(issue_advisory_lock_sql(true).contains("pg_advisory_xact_lock($1, $2)"));
        assert!(signing_key_select_sql(true).ends_with(" FOR UPDATE"));
        assert!(terminal_token_select_sql(true).ends_with(" FOR UPDATE"));
        assert!(issue_advisory_lock_sql(false).is_empty());
        assert!(!signing_key_select_sql(false).contains("FOR UPDATE"));
        assert!(!terminal_token_select_sql(false).contains("FOR UPDATE"));
    }

    #[test]
    fn admission_identity_derivation_matches_the_protocol_vector() {
        assert_eq!(
            super::issue_advisory_lock_keys("replica-a", "request-a"),
            (268_470_176, 895_439_656)
        );
        assert_eq!(
            super::quota_request_key("replica-a", "request-a"),
            "admission:100087a0355f53286a77044e517cbed3db75e2faf7a20115af0552e7b9038250"
        );
    }

    #[test]
    fn unconfirmed_recovery_query_is_bounded_and_uses_numeric_expiry() {
        let sql = unconfirmed_recovery_candidate_sql();
        assert!(sql.contains("expires_at_unix <= $1"));
        assert!(sql.contains("NOT EXISTS"));
        assert!(sql.contains("ORDER BY expires_at_unix ASC, token_id ASC"));
        assert!(sql.contains("LIMIT $2"));
        assert!(!sql.contains("expires_at <="));
    }
}
