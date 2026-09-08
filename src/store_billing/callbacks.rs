use chrono::{DateTime, SecondsFormat, Utc};
use sea_orm::{ConnectionTrait, QueryResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::crypto::{EncryptedSecret, PaymentKeyRing};
use super::money::{
    Currency, ExchangeRateRational, cny_fen_to_nano_usd, parse_minor, quoted_received_to_nano_usd,
};
use crate::db::DbPool;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyProviderEventInput {
    pub event_row_id: String,
    pub credential_version_id: String,
    pub verification_credential_version_id: String,
    pub provider_event_id: String,
    pub event_kind: String,
    pub order_id: String,
    pub attempt_id: String,
    pub provider_transaction_id: String,
    pub provider_object_id: String,
    pub order_number: String,
    pub merchant_account_identity: String,
    pub amount_minor: String,
    pub currency: Currency,
    pub body_digest: String,
    pub parsed_json: serde_json::Value,
    pub raw_body: Option<EncryptedSecret>,
    pub source_ip: Option<String>,
    pub user_agent: Option<String>,
    pub received_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordUnboundProviderEventInput {
    pub event_row_id: String,
    pub credential_version_id: String,
    pub provider_event_id: String,
    pub event_kind: String,
    pub body_digest: String,
    pub parsed_json: serde_json::Value,
    pub raw_body: EncryptedSecret,
    pub source_ip: Option<String>,
    pub user_agent: Option<String>,
    pub received_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackApplyResult {
    Applied,
    Duplicate,
    ManualReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReprocessProviderEventResult {
    pub event_id: String,
    pub projection: String,
    pub projection_state: String,
    pub state_revision: i64,
    pub order_id: Option<String>,
    pub attempt_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReprocessProviderEventError {
    #[error("Provider event input is invalid")]
    InvalidInput,
    #[error("Provider event was not found")]
    NotFound,
    #[error("Provider event cannot be reprocessed")]
    NotReprocessable,
    #[error("Provider event still requires manual review")]
    ManualReview,
    #[error("fresh Provider payment evidence is required")]
    ProviderQueryRequired,
    #[error("Provider event identity conflicts with stored evidence")]
    IdentityConflict,
    #[error("Provider event reprocess storage failed: {0}")]
    Storage(String),
    #[error("Provider event fulfillment failed: {0}")]
    Fulfillment(String),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CallbackStoreError {
    #[error("verified callback input is invalid")]
    InvalidInput,
    #[error("callback order or attempt is missing")]
    NotFound,
    #[error("callback storage failed: {0}")]
    Storage(String),
    #[error("callback fulfillment failed: {0}")]
    Fulfillment(String),
}

#[derive(Debug, Clone)]
pub struct PaymentCallbackStore {
    db: DbPool,
}

impl PaymentCallbackStore {
    pub fn new(db: DbPool) -> Self {
        Self { db }
    }

    pub async fn apply_verified_payment(
        &self,
        input: ApplyProviderEventInput,
    ) -> Result<CallbackApplyResult, CallbackStoreError> {
        if input.event_kind != "payment_succeeded" || input.raw_body.is_none() {
            return Err(CallbackStoreError::InvalidInput);
        }
        self.apply_verified_payment_inner(input, None, true).await
    }

    pub async fn apply_verified_query_payment(
        &self,
        input: ApplyProviderEventInput,
    ) -> Result<CallbackApplyResult, CallbackStoreError> {
        if input.event_kind != "payment_query_succeeded" || input.raw_body.is_some() {
            return Err(CallbackStoreError::InvalidInput);
        }
        self.apply_verified_payment_inner(input, None, true).await
    }

    pub async fn reprocess_verified_event(
        &self,
        event_id: &str,
        key_ring: Option<&PaymentKeyRing>,
        actor_id: &str,
    ) -> Result<ReprocessProviderEventResult, ReprocessProviderEventError> {
        if actor_id.trim().is_empty() {
            return Err(ReprocessProviderEventError::InvalidInput);
        }
        let tx = self.db.begin_write().await.map_err(reprocess_storage)?;
        if Uuid::parse_str(event_id).is_err() {
            append_reprocess_audit(
                &self.db,
                &*tx,
                actor_id,
                event_id,
                None,
                None,
                "invalid_request",
            )
            .await?;
            tx.commit().await.map_err(reprocess_storage)?;
            return Err(ReprocessProviderEventError::InvalidInput);
        }
        let lock = if self.db.is_postgres() {
            " FOR UPDATE"
        } else {
            ""
        };
        let event = tx
            .query_one(self.db.stmt(
                &format!(
                    "SELECT id, credential_version_id, provider_event_id, event_kind,
                            body_digest, parsed_json, verification_result, projection_state,
                            raw_format_version, raw_key_id, raw_nonce_base64,
                            raw_ciphertext_base64, state_revision, received_at
                     FROM store_provider_events WHERE id = $1{lock}"
                ),
                vec![event_id.into()],
            ))
            .await
            .map_err(reprocess_storage)?;
        let Some(event) = event else {
            append_reprocess_audit(
                &self.db,
                &*tx,
                actor_id,
                event_id,
                None,
                None,
                "event_not_found",
            )
            .await?;
            tx.commit().await.map_err(reprocess_storage)?;
            return Err(ReprocessProviderEventError::NotFound);
        };
        let prior_state = reprocess_row_string(&event, "projection_state")?;
        let prior_revision = reprocess_row_i64(&event, "state_revision")?;
        macro_rules! audited_reject {
            ($error:expr) => {{
                let error = $error;
                append_reprocess_audit(
                    &self.db,
                    &*tx,
                    actor_id,
                    event_id,
                    Some(&prior_state),
                    Some(prior_revision),
                    reprocess_error_result(&error),
                )
                .await?;
                tx.commit().await.map_err(reprocess_storage)?;
                return Err(error);
            }};
        }
        let event_kind = reprocess_row_string(&event, "event_kind")?;
        if reprocess_row_string(&event, "verification_result")? != "verified"
            || !matches!(
                event_kind.as_str(),
                "payment_succeeded" | "payment_query_succeeded"
            )
        {
            audited_reject!(ReprocessProviderEventError::NotReprocessable);
        }
        if prior_state == "applied" {
            let application = tx
                .query_one(self.db.stmt(
                    "SELECT order_id FROM store_order_event_applications
                     WHERE provider_event_row_id = $1",
                    vec![event_id.into()],
                ))
                .await
                .map_err(reprocess_storage)?;
            let order_id = application
                .as_ref()
                .map(|row| reprocess_row_string(row, "order_id"))
                .transpose()?;
            let attempt_id = stored_attempt_id(&event);
            append_reprocess_audit(
                &self.db,
                &*tx,
                actor_id,
                event_id,
                Some(&prior_state),
                Some(prior_revision),
                "duplicate",
            )
            .await?;
            tx.commit().await.map_err(reprocess_storage)?;
            return Ok(ReprocessProviderEventResult {
                event_id: event_id.to_string(),
                projection: "duplicate".to_string(),
                projection_state: prior_state,
                state_revision: prior_revision,
                order_id,
                attempt_id,
            });
        }
        if prior_state == "superseded" {
            append_reprocess_audit(
                &self.db,
                &*tx,
                actor_id,
                event_id,
                Some(&prior_state),
                Some(prior_revision),
                "event_not_reprocessable",
            )
            .await?;
            tx.commit().await.map_err(reprocess_storage)?;
            return Err(ReprocessProviderEventError::NotReprocessable);
        }
        if !matches!(prior_state.as_str(), "pending" | "manual_review") {
            audited_reject!(ReprocessProviderEventError::NotReprocessable);
        }
        if let Err(error) = verify_stored_raw_body(event_id, &event_kind, &event, key_ring) {
            audited_reject!(error);
        }
        let credential_version_id = reprocess_row_string(&event, "credential_version_id")?;
        let credential = tx
            .query_one(self.db.stmt(
                "SELECT channel_id, adapter_kind, account_identity_digest
                 FROM store_channel_credentials WHERE id = $1",
                vec![credential_version_id.clone().into()],
            ))
            .await
            .map_err(reprocess_storage)?;
        let Some(credential) = credential else {
            audited_reject!(ReprocessProviderEventError::ProviderQueryRequired);
        };
        let adapter_kind = reprocess_row_string(&credential, "adapter_kind")?;
        let channel_id = reprocess_row_string(&credential, "channel_id")?;
        let evidence = match parse_stored_evidence(&event, Some(&adapter_kind)) {
            Ok(evidence) => evidence,
            Err(error) => audited_reject!(error),
        };
        let verification_credential_version_id = evidence
            .verification_credential_version_id
            .as_deref()
            .unwrap_or(&credential_version_id)
            .to_string();
        let verification_credential = tx
            .query_one(self.db.stmt(
                "SELECT channel_id, adapter_kind, account_identity_digest
                 FROM store_channel_credentials WHERE id = $1",
                vec![verification_credential_version_id.clone().into()],
            ))
            .await
            .map_err(reprocess_storage)?;
        let Some(verification_credential) = verification_credential else {
            audited_reject!(ReprocessProviderEventError::IdentityConflict);
        };
        let provider_event_id = reprocess_row_string(&event, "provider_event_id")?;
        if evidence.event_kind != event_kind
            || evidence
                .event_id
                .as_deref()
                .is_some_and(|value| value != provider_event_id)
        {
            audited_reject!(ReprocessProviderEventError::IdentityConflict);
        }
        let conflict_id = format!(
            "provider-event-identity-conflict:{}",
            length_prefixed_sha256(&[&credential_version_id, &provider_event_id])
        );
        if has_open_reprocess_identity_conflict(&self.db, &*tx, &conflict_id).await? {
            audited_reject!(ReprocessProviderEventError::IdentityConflict);
        }

        let candidate = match select_reprocess_candidate(
            &self.db,
            &*tx,
            &event_kind,
            &adapter_kind,
            &channel_id,
            &credential_version_id,
            &evidence,
        )
        .await
        {
            Ok(candidate) => candidate,
            Err(error) => audited_reject!(error),
        };
        let order = tx
            .query_one(self.db.stmt(
                &format!("SELECT id FROM store_orders WHERE id = $1{lock}"),
                vec![candidate.order_id.clone().into()],
            ))
            .await
            .map_err(reprocess_storage)?;
        if order.is_none() {
            audited_reject!(ReprocessProviderEventError::ProviderQueryRequired);
        }
        let locked_candidate = match select_reprocess_candidate(
            &self.db,
            &*tx,
            &event_kind,
            &adapter_kind,
            &channel_id,
            &credential_version_id,
            &evidence,
        )
        .await
        {
            Ok(candidate) => candidate,
            Err(error) => audited_reject!(error),
        };
        if locked_candidate != candidate {
            audited_reject!(ReprocessProviderEventError::ManualReview);
        }
        if has_open_reprocess_identity_conflict(&self.db, &*tx, &conflict_id).await? {
            audited_reject!(ReprocessProviderEventError::IdentityConflict);
        }
        let contract = tx
            .query_one(self.db.stmt(
                &format!(
                    "SELECT a.id AS attempt_id, a.order_id, a.channel_id, a.adapter_kind,
                            a.credential_version_id, a.state AS attempt_state, a.failure_kind,
                            a.provider_object_id, a.provider_transaction_id,
                            a.merchant_account_identity,
                            o.order_number, o.payment_minor, o.payment_currency,
                            o.payment_state, o.payment_hold, o.contract_version, o.state_revision
                     FROM store_payment_attempts a
                     JOIN store_orders o ON o.id = a.order_id
                     WHERE a.id = $1 AND o.id = $2{lock}"
                ),
                vec![
                    candidate.attempt_id.clone().into(),
                    candidate.order_id.clone().into(),
                ],
            ))
            .await
            .map_err(reprocess_storage)?;
        let Some(contract) = contract else {
            audited_reject!(ReprocessProviderEventError::ProviderQueryRequired);
        };
        let attempt_credential_version_id =
            reprocess_row_string(&contract, "credential_version_id")?;
        let attempt_credential = tx
            .query_one(self.db.stmt(
                "SELECT channel_id, adapter_kind, account_identity_digest
                 FROM store_channel_credentials WHERE id = $1",
                vec![attempt_credential_version_id.clone().into()],
            ))
            .await
            .map_err(reprocess_storage)?;
        let Some(attempt_credential) = attempt_credential else {
            audited_reject!(ReprocessProviderEventError::IdentityConflict);
        };
        if let Err(error) = validate_reprocess_contract(
            &event_kind,
            &adapter_kind,
            &channel_id,
            &credential_version_id,
            &credential,
            &verification_credential_version_id,
            &verification_credential,
            &attempt_credential_version_id,
            &attempt_credential,
            &contract,
            &evidence,
        ) {
            audited_reject!(error);
        }
        if let Err(error) = validate_query_digest(&event_kind, &event, &contract, &evidence) {
            audited_reject!(error);
        }

        let payment_state = reprocess_row_string(&contract, "payment_state")?;
        let contract_version = reprocess_row_i32(&contract, "contract_version")?;
        if !(payment_state == "unpaid"
            || (payment_state == "closed" && contract_version == 2)
            || payment_state == "paid")
        {
            if matches!(payment_state.as_str(), "refund_pending" | "refunded") {
                audited_reject!(ReprocessProviderEventError::ProviderQueryRequired);
            }
            audited_reject!(ReprocessProviderEventError::ManualReview);
        }
        let now = Utc::now();
        let now_text = timestamp(now);
        tx.execute(self.db.stmt(
            "UPDATE store_payment_attempts
             SET state = 'paid', failure_kind = NULL,
                 provider_transaction_id = $2,
                 provider_object_id = COALESCE(provider_object_id, $3),
                 paid_at = $4, updated_at = $4
             WHERE id = $1",
            vec![
                candidate.attempt_id.clone().into(),
                evidence.provider_transaction_id.clone().into(),
                evidence.provider_object_id.clone().into(),
                now_text.clone().into(),
            ],
        ))
        .await
        .map_err(reprocess_storage)?;
        if payment_state != "paid" {
            let changed = tx
                .execute(self.db.stmt(
                    "UPDATE store_orders
                     SET payment_state = 'paid', paid_at = $3, updated_at = $3,
                         state_revision = state_revision + 1
                     WHERE id = $1 AND payment_state = $2 AND state_revision = $4",
                    vec![
                        candidate.order_id.clone().into(),
                        payment_state.into(),
                        now_text.clone().into(),
                        reprocess_row_i64(&contract, "state_revision")?.into(),
                    ],
                ))
                .await
                .map_err(reprocess_storage)?;
            if changed.rows_affected() != 1 {
                audited_reject!(ReprocessProviderEventError::ManualReview);
            }
        }
        let event_changed = tx
            .execute(self.db.stmt(
                "UPDATE store_provider_events
                 SET projection_state = 'applied', state_revision = state_revision + 1,
                     applied_at = $3
                 WHERE id = $1 AND state_revision = $2
                   AND projection_state IN ('pending', 'manual_review')",
                vec![
                    event_id.into(),
                    prior_revision.into(),
                    now_text.clone().into(),
                ],
            ))
            .await
            .map_err(reprocess_storage)?;
        if event_changed.rows_affected() != 1 {
            audited_reject!(ReprocessProviderEventError::ManualReview);
        }
        tx.execute(self.db.stmt(
            "INSERT INTO store_order_event_applications
                (provider_event_row_id, order_id, result, applied_at)
             VALUES ($1, $2, 'payment_applied', $3)",
            vec![
                event_id.into(),
                candidate.order_id.clone().into(),
                now_text.into(),
            ],
        ))
        .await
        .map_err(reprocess_storage)?;
        append_reprocess_audit(
            &self.db,
            &*tx,
            actor_id,
            event_id,
            Some(&prior_state),
            Some(prior_revision),
            "applied",
        )
        .await?;
        tx.commit().await.map_err(reprocess_storage)?;

        if reprocess_row_i32(&contract, "payment_hold")? == 0 {
            self.fulfill_paid_order(&candidate.order_id)
                .await
                .map_err(|error| ReprocessProviderEventError::Fulfillment(error.to_string()))?;
        }
        Ok(ReprocessProviderEventResult {
            event_id: event_id.to_string(),
            projection: "applied".to_string(),
            projection_state: "applied".to_string(),
            state_revision: prior_revision + 1,
            order_id: Some(candidate.order_id),
            attempt_id: Some(candidate.attempt_id),
        })
    }

    pub async fn audit_invalid_reprocess_request(
        &self,
        event_id: &str,
        actor_id: &str,
    ) -> Result<(), ReprocessProviderEventError> {
        if actor_id.trim().is_empty() {
            return Err(ReprocessProviderEventError::InvalidInput);
        }
        let tx = self.db.begin_write().await.map_err(reprocess_storage)?;
        let prior = if Uuid::parse_str(event_id).is_ok() {
            tx.query_one(self.db.stmt(
                "SELECT projection_state, state_revision
                 FROM store_provider_events WHERE id = $1",
                vec![event_id.into()],
            ))
            .await
            .map_err(reprocess_storage)?
        } else {
            None
        };
        let prior_state = prior
            .as_ref()
            .map(|row| reprocess_row_string(row, "projection_state"))
            .transpose()?;
        let prior_revision = prior
            .as_ref()
            .map(|row| reprocess_row_i64(row, "state_revision"))
            .transpose()?;
        append_reprocess_audit(
            &self.db,
            &*tx,
            actor_id,
            event_id,
            prior_state.as_deref(),
            prior_revision,
            "invalid_request",
        )
        .await?;
        tx.commit().await.map_err(reprocess_storage)?;
        Ok(())
    }

    pub async fn record_unbound_verified_event(
        &self,
        input: RecordUnboundProviderEventInput,
    ) -> Result<CallbackApplyResult, CallbackStoreError> {
        if input.event_row_id.trim().is_empty()
            || input.credential_version_id.trim().is_empty()
            || input.provider_event_id.trim().is_empty()
            || input.event_kind != "payment_succeeded"
            || input.body_digest.trim().is_empty()
        {
            return Err(CallbackStoreError::InvalidInput);
        }
        let tx = self.db.begin_write().await.map_err(storage)?;
        let inserted = tx
            .execute(self.db.stmt(
                "INSERT INTO store_provider_events
                    (id, credential_version_id, provider_event_id, event_kind,
                     body_digest, parsed_json, verification_result, projection_state,
                     raw_format_version, raw_key_id, raw_nonce_base64, raw_ciphertext_base64,
                     source_ip, user_agent, state_revision, received_at, applied_at)
                 VALUES ($1, $2, $3, $4, $5, $6, 'verified', 'manual_review',
                         $7, $8, $9, $10, $11, $12, 0, $13, NULL)
                 ON CONFLICT (credential_version_id, provider_event_id) DO NOTHING",
                vec![
                    input.event_row_id.into(),
                    input.credential_version_id.clone().into(),
                    input.provider_event_id.clone().into(),
                    input.event_kind.into(),
                    input.body_digest.clone().into(),
                    input.parsed_json.to_string().into(),
                    i32::from(input.raw_body.version).into(),
                    input.raw_body.key_id.into(),
                    input.raw_body.nonce_base64.into(),
                    input.raw_body.ciphertext_base64.into(),
                    input.source_ip.into(),
                    input.user_agent.into(),
                    timestamp(input.received_at).into(),
                ],
            ))
            .await
            .map_err(storage)?;
        let result = if inserted.rows_affected() == 1 {
            CallbackApplyResult::ManualReview
        } else {
            let duplicate = tx
                .query_one(self.db.stmt(
                    "SELECT projection_state, body_digest, parsed_json
                     FROM store_provider_events
                     WHERE credential_version_id = $1 AND provider_event_id = $2",
                    vec![
                        input.credential_version_id.clone().into(),
                        input.provider_event_id.clone().into(),
                    ],
                ))
                .await
                .map_err(storage)?
                .ok_or_else(|| {
                    CallbackStoreError::Storage(
                        "conflicting unbound callback event is missing".to_string(),
                    )
                })?;
            duplicate_result_with_evidence(
                &self.db,
                &*tx,
                &duplicate,
                &input.credential_version_id,
                &input.provider_event_id,
                &input.body_digest,
                &input.parsed_json,
                None,
                None,
                input.received_at,
            )
            .await?
        };
        tx.commit().await.map_err(storage)?;
        Ok(result)
    }

    pub(crate) async fn apply_verified_payment_fenced(
        &self,
        input: ApplyProviderEventInput,
        owner_id: &str,
        epoch: i64,
        now: DateTime<Utc>,
    ) -> Result<CallbackApplyResult, CallbackStoreError> {
        if input.event_kind != "payment_query_succeeded" || input.raw_body.is_some() {
            return Err(CallbackStoreError::InvalidInput);
        }
        self.apply_verified_payment_inner(input, Some((owner_id, epoch, now)), false)
            .await
    }

    async fn apply_verified_payment_inner(
        &self,
        input: ApplyProviderEventInput,
        fence: Option<(&str, i64, DateTime<Utc>)>,
        fulfill_after_projection: bool,
    ) -> Result<CallbackApplyResult, CallbackStoreError> {
        validate_input(&input)?;
        let tx = self.db.begin_write().await.map_err(storage)?;
        let lock = if self.db.is_postgres() {
            " FOR UPDATE"
        } else {
            ""
        };
        if let Some((owner_id, epoch, now)) = fence {
            validate_reconciliation_fence(&self.db, &*tx, owner_id, epoch, now, lock).await?;
        }
        tx.query_one(self.db.stmt(
            &format!("SELECT id FROM store_orders WHERE id = $1{lock}"),
            vec![input.order_id.clone().into()],
        ))
        .await
        .map_err(storage)?
        .ok_or(CallbackStoreError::NotFound)?;
        let row = tx
            .query_one(self.db.stmt(
                &format!(
                    "SELECT a.id AS attempt_id, a.order_id, a.channel_id, a.adapter_kind,
                            a.credential_version_id,
                            a.state AS attempt_state, a.failure_kind,
                            o.user_id, o.product_kind,
                            o.payment_state, o.fulfillment_state, o.payment_hold,
                            a.provider_object_id, a.merchant_account_identity,
                            o.order_number, o.payment_minor, o.payment_currency, o.rate_numerator,
                            o.rate_denominator, o.quote_json, o.contract_version,
                            o.state_revision
                     FROM store_payment_attempts a
                     JOIN store_orders o ON o.id = a.order_id
                     WHERE a.id = $1 AND o.id = $2{lock}"
                ),
                vec![
                    input.attempt_id.clone().into(),
                    input.order_id.clone().into(),
                ],
            ))
            .await
            .map_err(storage)?
            .ok_or(CallbackStoreError::NotFound)?;

        let stored_provider_object = row_optional_string(&row, "provider_object_id")?;
        let adapter_kind = row_string(&row, "adapter_kind")?;
        let channel_id = row_string(&row, "channel_id")?;
        let callback_requires_unique_binding = input.event_kind == "payment_succeeded"
            && input.raw_body.is_some()
            && matches!(adapter_kind.as_str(), "alipay" | "wechat");
        if callback_requires_unique_binding {
            let identity_column = if adapter_kind == "alipay" {
                "credential_version_id"
            } else {
                "merchant_account_identity"
            };
            let identity = if adapter_kind == "alipay" {
                &input.credential_version_id
            } else {
                &input.merchant_account_identity
            };
            let candidates = tx
                .query_all(self.db.stmt(
                    &format!(
                        "SELECT a.id AS attempt_id
                         FROM store_payment_attempts a
                         JOIN store_orders o ON o.id = a.order_id
                         WHERE o.id = $1 AND o.order_number = $2
                           AND a.channel_id = $3 AND a.adapter_kind = $4
                           AND a.{identity_column} = $5
                           AND (a.provider_object_id = $6
                                OR (a.provider_object_id IS NULL
                                    AND (a.state = 'created'
                                         OR (a.state = 'failed'
                                             AND a.failure_kind = 'provider_rejected'))))
                         ORDER BY a.created_at DESC, a.id DESC
                         LIMIT 2"
                    ),
                    vec![
                        input.order_id.clone().into(),
                        input.order_number.clone().into(),
                        channel_id.clone().into(),
                        adapter_kind.clone().into(),
                        identity.clone().into(),
                        input.provider_object_id.clone().into(),
                    ],
                ))
                .await
                .map_err(storage)?;
            if candidates.len() != 1
                || row_string(&candidates[0], "attempt_id")? != input.attempt_id
            {
                let duplicate = tx
                    .query_one(self.db.stmt(
                        "SELECT projection_state, body_digest, parsed_json
                         FROM store_provider_events
                         WHERE credential_version_id = $1 AND provider_event_id = $2",
                        vec![
                            input.verification_credential_version_id.clone().into(),
                            input.provider_event_id.clone().into(),
                        ],
                    ))
                    .await
                    .map_err(storage)?;
                if let Some(duplicate) = duplicate {
                    let result = duplicate_result_with_evidence(
                        &self.db,
                        &*tx,
                        &duplicate,
                        &input.verification_credential_version_id,
                        &input.provider_event_id,
                        &input.body_digest,
                        &input.parsed_json,
                        Some(&input.order_id),
                        Some(&channel_id),
                        input.received_at,
                    )
                    .await?;
                    tx.commit().await.map_err(storage)?;
                    return Ok(result);
                }
                insert_event(
                    &self.db,
                    &*tx,
                    &input.event_row_id,
                    &input.verification_credential_version_id,
                    &input,
                    "manual_review",
                )
                .await?;
                tx.commit().await.map_err(storage)?;
                return Ok(CallbackApplyResult::ManualReview);
            }
        }

        let duplicate = tx
            .query_one(self.db.stmt(
                "SELECT projection_state, body_digest, parsed_json
                 FROM store_provider_events
                 WHERE credential_version_id = $1 AND provider_event_id = $2",
                vec![
                    input.credential_version_id.clone().into(),
                    input.provider_event_id.clone().into(),
                ],
            ))
            .await
            .map_err(storage)?;
        if let Some(duplicate) = duplicate {
            let result = duplicate_result_with_evidence(
                &self.db,
                &*tx,
                &duplicate,
                &input.credential_version_id,
                &input.provider_event_id,
                &input.body_digest,
                &input.parsed_json,
                Some(&input.order_id),
                Some(&channel_id),
                input.received_at,
            )
            .await?;
            tx.commit().await.map_err(storage)?;
            return Ok(result);
        }

        let absent_provider_object_may_bind = if stored_provider_object.is_none()
            && matches!(adapter_kind.as_str(), "alipay" | "wechat")
            && input.provider_object_id == input.order_number
            && matches!(
                (
                    row_string(&row, "attempt_state")?.as_str(),
                    row_optional_string(&row, "failure_kind")?.as_deref(),
                ),
                ("created", None) | ("failed", Some("provider_rejected"))
            ) {
            true
        } else {
            false
        };
        let provider_object_matches = stored_provider_object
            .as_deref()
            .is_some_and(|value| value == input.provider_object_id)
            || absent_provider_object_may_bind;
        let matches_contract = row_string(&row, "credential_version_id")?
            == input.credential_version_id
            && provider_object_matches
            && row_string(&row, "merchant_account_identity")? == input.merchant_account_identity
            && row_string(&row, "order_number")? == input.order_number
            && row_string(&row, "payment_minor")? == input.amount_minor
            && row_string(&row, "payment_currency")? == currency_string(input.currency);
        let event_row_id = input.event_row_id.clone();
        if !matches_contract {
            insert_event(
                &self.db,
                &*tx,
                &event_row_id,
                &input.credential_version_id,
                &input,
                "manual_review",
            )
            .await?;
            tx.commit().await.map_err(storage)?;
            return Ok(CallbackApplyResult::ManualReview);
        }

        let payment_state = row_string(&row, "payment_state")?;
        let payment_hold = row_i32(&row, "payment_hold")? != 0;
        let contract_version = row_i32(&row, "contract_version")?;
        let can_apply = payment_state == "unpaid"
            || (payment_state == "closed" && contract_version == 2)
            || payment_state == "paid";
        if !can_apply {
            insert_event(
                &self.db,
                &*tx,
                &event_row_id,
                &input.credential_version_id,
                &input,
                "manual_review",
            )
            .await?;
            tx.commit().await.map_err(storage)?;
            return Ok(CallbackApplyResult::ManualReview);
        }

        insert_event(
            &self.db,
            &*tx,
            &event_row_id,
            &input.credential_version_id,
            &input,
            "applied",
        )
        .await?;
        let now = timestamp(input.received_at);
        tx.execute(self.db.stmt(
            "UPDATE store_payment_attempts
             SET state = 'paid', failure_kind = NULL,
                 provider_transaction_id = $2,
                 provider_object_id = COALESCE(provider_object_id, $3),
                 paid_at = $4, updated_at = $4
             WHERE id = $1",
            vec![
                input.attempt_id.clone().into(),
                input.provider_transaction_id.clone().into(),
                input.provider_object_id.clone().into(),
                now.clone().into(),
            ],
        ))
        .await
        .map_err(storage)?;
        if payment_state != "paid" {
            let changed = tx
                .execute(self.db.stmt(
                    "UPDATE store_orders
                     SET payment_state = 'paid', paid_at = $3, updated_at = $3,
                         state_revision = state_revision + 1
                     WHERE id = $1 AND payment_state = $2 AND state_revision = $4",
                    vec![
                        input.order_id.clone().into(),
                        payment_state.into(),
                        now.clone().into(),
                        row_i64(&row, "state_revision")?.into(),
                    ],
                ))
                .await
                .map_err(storage)?;
            if changed.rows_affected() != 1 {
                return Err(CallbackStoreError::Storage(
                    "order state changed during callback projection".to_string(),
                ));
            }
        }
        tx.execute(self.db.stmt(
            "INSERT INTO store_order_event_applications
                (provider_event_row_id, order_id, result, applied_at)
             VALUES ($1, $2, 'payment_applied', $3)",
            vec![
                event_row_id.into(),
                input.order_id.clone().into(),
                now.into(),
            ],
        ))
        .await
        .map_err(storage)?;
        tx.commit().await.map_err(storage)?;

        if payment_hold || !fulfill_after_projection {
            return Ok(CallbackApplyResult::Applied);
        }
        if let Some((owner_id, epoch, now)) = fence {
            self.fulfill_paid_order_fenced(&input.order_id, owner_id, epoch, now)
                .await?;
        } else {
            self.fulfill_paid_order(&input.order_id).await?;
        }
        Ok(CallbackApplyResult::Applied)
    }

    pub async fn fulfill_paid_order(&self, order_id: &str) -> Result<(), CallbackStoreError> {
        self.fulfill_paid_order_inner(order_id, None, Utc::now())
            .await
    }

    pub(crate) async fn fulfill_paid_order_fenced(
        &self,
        order_id: &str,
        owner_id: &str,
        epoch: i64,
        now: DateTime<Utc>,
    ) -> Result<(), CallbackStoreError> {
        self.fulfill_paid_order_inner(order_id, Some((owner_id, epoch)), now)
            .await
    }

    async fn fulfill_paid_order_inner(
        &self,
        order_id: &str,
        fence: Option<(&str, i64)>,
        now_at: DateTime<Utc>,
    ) -> Result<(), CallbackStoreError> {
        let tx = self.db.begin_write().await.map_err(storage)?;
        let lock = if self.db.is_postgres() {
            " FOR UPDATE"
        } else {
            ""
        };
        if let Some((owner_id, epoch)) = fence {
            validate_reconciliation_fence(&self.db, &*tx, owner_id, epoch, now_at, lock).await?;
        }
        let row = tx
            .query_one(self.db.stmt(
                &format!(
                    "SELECT id, user_id, product_kind, payment_state, fulfillment_state,
                            payment_hold, payment_currency, cny_per_usd, rate_numerator,
                            rate_denominator, quote_json, state_revision
                     FROM store_orders WHERE id = $1{lock}"
                ),
                vec![order_id.into()],
            ))
            .await
            .map_err(storage)?
            .ok_or(CallbackStoreError::NotFound)?;
        if row_string(&row, "fulfillment_state")? == "fulfilled" {
            tx.execute(self.db.stmt(
                "DELETE FROM store_fulfillment_retries WHERE order_id = $1",
                vec![order_id.into()],
            ))
            .await
            .map_err(storage)?;
            tx.commit().await.map_err(storage)?;
            return Ok(());
        }
        if row_string(&row, "payment_state")? != "paid" || row_i32(&row, "payment_hold")? != 0 {
            return Err(CallbackStoreError::Fulfillment(
                "order is not eligible for fulfillment".to_string(),
            ));
        }
        let quote: OrderQuote = serde_json::from_str(&row_string(&row, "quote_json")?)
            .map_err(|error| CallbackStoreError::Fulfillment(error.to_string()))?;
        if row_string(&row, "product_kind")? == "plan" {
            let duration = quote.product.duration_seconds.ok_or_else(|| {
                CallbackStoreError::Fulfillment("plan duration is missing".to_string())
            })?;
            let ends_at = now_at
                .checked_add_signed(chrono::Duration::seconds(duration))
                .ok_or_else(|| {
                    CallbackStoreError::Fulfillment("plan duration overflow".to_string())
                })?;
            let user_id = row_string(&row, "user_id")?;
            let expected_generation = tx
                .query_one(self.db.stmt(
                    "SELECT generation FROM store_plan_entitlement_current WHERE user_id = $1",
                    vec![user_id.clone().into()],
                ))
                .await
                .map_err(storage)?
                .map(|current| row_i64(&current, "generation"))
                .transpose()?;
            crate::store_billing::quota::replace_entitlement_tx(
                &self.db,
                &*tx,
                crate::store_billing::quota::EntitlementGenerationInput {
                    expected_generation,
                    user_id,
                    product_id: quote.product.id,
                    product_name: quote.product.name,
                    starts_at: now_at,
                    ends_at,
                    rate_numerator: row_string(&row, "rate_numerator")?,
                    rate_denominator: row_string(&row, "rate_denominator")?,
                    group_ids: quote.product.group_ids,
                    quotas: quote.product.quotas,
                    source_kind: "order".to_string(),
                    source_id: order_id.to_string(),
                },
            )
            .await
            .map_err(|error| CallbackStoreError::Fulfillment(error.to_string()))?;
            finish_order_fulfillment(
                &self.db,
                &*tx,
                order_id,
                row_i64(&row, "state_revision")?,
                now_at,
            )
            .await?;
            tx.commit().await.map_err(storage)?;
            return Ok(());
        }
        let received = quote
            .product
            .balance
            .ok_or_else(|| CallbackStoreError::Fulfillment("balance quote is missing".to_string()))?
            .actual_received_minor;
        let received_minor = parse_minor(&received)
            .map_err(|error| CallbackStoreError::Fulfillment(error.to_string()))?;
        let rate = ExchangeRateRational::parse(&row_string(&row, "cny_per_usd")?)
            .map_err(|error| CallbackStoreError::Fulfillment(error.to_string()))?;
        if rate.numerator().to_string() != row_string(&row, "rate_numerator")?
            || rate.denominator().to_string() != row_string(&row, "rate_denominator")?
        {
            return Err(CallbackStoreError::Fulfillment(
                "order rate rational does not match decimal snapshot".to_string(),
            ));
        }
        let delta = match row_string(&row, "payment_currency")?.as_str() {
            "CNY" => cny_fen_to_nano_usd(received_minor, &rate),
            "USD" => quoted_received_to_nano_usd(received_minor, Currency::USD, rate.decimal()),
            _ => {
                return Err(CallbackStoreError::Fulfillment(
                    "order currency is invalid".to_string(),
                ));
            }
        }
        .map_err(|error| CallbackStoreError::Fulfillment(error.to_string()))?;

        let user_id = row_string(&row, "user_id")?;
        let user = tx
            .query_one(self.db.stmt(
                &format!("SELECT balance_nano_usd FROM users WHERE id = $1{lock}"),
                vec![user_id.clone().into()],
            ))
            .await
            .map_err(storage)?
            .ok_or_else(|| CallbackStoreError::Fulfillment("reward user is missing".to_string()))?;
        let previous = parse_minor(&row_string(&user, "balance_nano_usd")?)
            .map_err(|error| CallbackStoreError::Fulfillment(error.to_string()))?;
        let balance = previous
            .checked_add(delta)
            .ok_or_else(|| CallbackStoreError::Fulfillment("balance overflow".to_string()))?;
        let now = timestamp(now_at);
        tx.execute(self.db.stmt(
            "UPDATE users SET balance_nano_usd = $2, updated_at = $3 WHERE id = $1",
            vec![
                user_id.clone().into(),
                balance.to_string().into(),
                now.clone().into(),
            ],
        ))
        .await
        .map_err(storage)?;
        tx.execute(self.db.stmt(
            "INSERT INTO billing_ledger
                (id, user_id, kind, delta_nano_usd, balance_after_nano_usd,
                 meta_json, created_at, idempotency_key)
             VALUES ($1, $2, 'store_recharge', $3, $4, $5, $6, $7)",
            vec![
                Uuid::new_v4().to_string().into(),
                user_id.into(),
                delta.to_string().into(),
                balance.to_string().into(),
                serde_json::json!({"order_id": order_id}).to_string().into(),
                now.clone().into(),
                format!("store:fulfillment:{order_id}").into(),
            ],
        ))
        .await
        .map_err(storage)?;
        let changed = tx
            .execute(self.db.stmt(
                "UPDATE store_orders
                 SET fulfillment_state = 'fulfilled', fulfillment_started_at = $2,
                     fulfilled_at = $2, updated_at = $2, state_revision = state_revision + 1
                 WHERE id = $1 AND payment_state = 'paid'
                   AND fulfillment_state IN ('pending', 'failed') AND payment_hold = 0
                   AND state_revision = $3",
                vec![
                    order_id.into(),
                    now.into(),
                    row_i64(&row, "state_revision")?.into(),
                ],
            ))
            .await
            .map_err(storage)?;
        if changed.rows_affected() != 1 {
            return Err(CallbackStoreError::Storage(
                "order state changed during fulfillment".to_string(),
            ));
        }
        tx.execute(self.db.stmt(
            "DELETE FROM store_fulfillment_retries WHERE order_id = $1",
            vec![order_id.into()],
        ))
        .await
        .map_err(storage)?;
        tx.commit().await.map_err(storage)
    }
}

async fn has_open_reprocess_identity_conflict<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    conflict_id: &str,
) -> Result<bool, ReprocessProviderEventError> {
    Ok(connection
        .query_one(db.stmt(
            "SELECT id FROM store_reconciliation_cases
             WHERE id = $1 AND state = 'open'",
            vec![conflict_id.into()],
        ))
        .await
        .map_err(reprocess_storage)?
        .is_some())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReprocessCandidate {
    order_id: String,
    attempt_id: String,
}

#[derive(Debug, Clone)]
struct StoredReprocessEvidence {
    event_id: Option<String>,
    event_kind: String,
    attempt_id: Option<String>,
    provider_object_id: String,
    provider_transaction_id: String,
    order_number: String,
    amount_minor: String,
    currency: String,
    account_identity: Option<String>,
    verification_credential_version_id: Option<String>,
}

fn stored_attempt_id(event: &QueryResult) -> Option<String> {
    let raw = event.try_get::<String>("", "parsed_json").ok()?;
    serde_json::from_str::<serde_json::Value>(&raw)
        .ok()?
        .get("attempt_id")?
        .as_str()
        .map(str::to_string)
}

fn parse_stored_evidence(
    event: &QueryResult,
    adapter_kind: Option<&str>,
) -> Result<StoredReprocessEvidence, ReprocessProviderEventError> {
    let event_kind = reprocess_row_string(event, "event_kind")?;
    let raw = reprocess_row_string(event, "parsed_json")?;
    let value: serde_json::Value =
        serde_json::from_str(&raw).map_err(|_| ReprocessProviderEventError::IdentityConflict)?;
    let object = value
        .as_object()
        .ok_or(ReprocessProviderEventError::IdentityConflict)?;
    if adapter_kind.is_none() {
        return Ok(StoredReprocessEvidence {
            event_id: optional_json_string(object, "event_id")?,
            event_kind,
            attempt_id: optional_json_string(object, "attempt_id")?,
            provider_object_id: String::new(),
            provider_transaction_id: String::new(),
            order_number: String::new(),
            amount_minor: String::new(),
            currency: String::new(),
            account_identity: None,
            verification_credential_version_id: None,
        });
    }
    if event_kind == "payment_query_succeeded" {
        const ADMIN_QUERY_KEYS: &[&str] = &[
            "event_kind",
            "attempt_id",
            "provider_object_id",
            "provider_transaction_id",
            "order_number",
            "amount_minor",
            "currency",
        ];
        const RECONCILIATION_QUERY_KEYS: &[&str] = &[
            "source",
            "attempt_id",
            "provider_object_id",
            "provider_transaction_id",
            "order_number",
            "amount_minor",
            "currency",
        ];
        if !(has_exact_keys(object, ADMIN_QUERY_KEYS)
            || has_exact_keys(object, RECONCILIATION_QUERY_KEYS))
            || object
                .get("event_kind")
                .is_some_and(|value| value.as_str() != Some("payment_query_succeeded"))
            || object
                .get("source")
                .is_some_and(|value| value.as_str() != Some("payment_query"))
        {
            return Err(ReprocessProviderEventError::IdentityConflict);
        }
        return Ok(StoredReprocessEvidence {
            event_id: None,
            event_kind,
            attempt_id: Some(required_json_string(object, "attempt_id")?),
            provider_object_id: required_json_string(object, "provider_object_id")?,
            provider_transaction_id: required_json_string(object, "provider_transaction_id")?,
            order_number: required_json_string(object, "order_number")?,
            amount_minor: required_json_string(object, "amount_minor")?,
            currency: required_json_string(object, "currency")?,
            account_identity: None,
            verification_credential_version_id: None,
        });
    }

    let adapter_kind = adapter_kind.expect("checked above");
    let (keys, provider_object_key, provider_transaction_key) = match adapter_kind {
        "stripe" => (
            &[
                "event_id",
                "event_kind",
                "checkout_session_id",
                "payment_intent_id",
                "attempt_id",
                "order_number",
                "amount_minor",
                "currency",
                "account_identity",
            ][..],
            "checkout_session_id",
            "payment_intent_id",
        ),
        "alipay" => (
            &[
                "event_id",
                "event_kind",
                "trade_no",
                "order_number",
                "amount_minor",
                "currency",
                "account_identity",
            ][..],
            "order_number",
            "trade_no",
        ),
        "wechat" => (
            &[
                "event_id",
                "event_kind",
                "transaction_id",
                "order_number",
                "amount_minor",
                "currency",
                "account_identity",
                "verification_credential_version_id",
            ][..],
            "order_number",
            "transaction_id",
        ),
        _ => return Err(ReprocessProviderEventError::NotReprocessable),
    };
    if !has_exact_keys(object, keys)
        || required_json_string(object, "event_kind")? != "payment_succeeded"
    {
        return Err(ReprocessProviderEventError::IdentityConflict);
    }
    Ok(StoredReprocessEvidence {
        event_id: Some(required_json_string(object, "event_id")?),
        event_kind,
        attempt_id: if adapter_kind == "stripe" {
            Some(required_json_string(object, "attempt_id")?)
        } else {
            None
        },
        provider_object_id: required_json_string(object, provider_object_key)?,
        provider_transaction_id: required_json_string(object, provider_transaction_key)?,
        order_number: required_json_string(object, "order_number")?,
        amount_minor: required_json_string(object, "amount_minor")?,
        currency: required_json_string(object, "currency")?,
        account_identity: Some(required_json_string(object, "account_identity")?),
        verification_credential_version_id: optional_json_string(
            object,
            "verification_credential_version_id",
        )?,
    })
}

fn has_exact_keys(object: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> bool {
    object.len() == keys.len() && keys.iter().all(|key| object.contains_key(*key))
}

fn required_json_string(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<String, ReprocessProviderEventError> {
    object
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or(ReprocessProviderEventError::IdentityConflict)
}

fn optional_json_string(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Option<String>, ReprocessProviderEventError> {
    object
        .get(key)
        .map(|_| required_json_string(object, key))
        .transpose()
}

fn verify_stored_raw_body(
    event_id: &str,
    event_kind: &str,
    event: &QueryResult,
    key_ring: Option<&PaymentKeyRing>,
) -> Result<(), ReprocessProviderEventError> {
    let version: Option<i32> = event
        .try_get("", "raw_format_version")
        .map_err(reprocess_storage)?;
    let key_id = reprocess_row_optional_string(event, "raw_key_id")?;
    let nonce_base64 = reprocess_row_optional_string(event, "raw_nonce_base64")?;
    let ciphertext_base64 = reprocess_row_optional_string(event, "raw_ciphertext_base64")?;
    if event_kind == "payment_query_succeeded" {
        if version.is_some()
            || key_id.is_some()
            || nonce_base64.is_some()
            || ciphertext_base64.is_some()
        {
            return Err(ReprocessProviderEventError::IdentityConflict);
        }
        return Ok(());
    }
    let encrypted = EncryptedSecret {
        version: u8::try_from(version.ok_or(ReprocessProviderEventError::IdentityConflict)?)
            .map_err(|_| ReprocessProviderEventError::IdentityConflict)?,
        key_id: key_id.ok_or(ReprocessProviderEventError::IdentityConflict)?,
        nonce_base64: nonce_base64.ok_or(ReprocessProviderEventError::IdentityConflict)?,
        ciphertext_base64: ciphertext_base64
            .ok_or(ReprocessProviderEventError::IdentityConflict)?,
    };
    let raw = key_ring
        .ok_or(ReprocessProviderEventError::ProviderQueryRequired)?
        .decrypt(
            &format!("store_provider_events:{event_id}:raw_body"),
            &encrypted,
        )
        .map_err(|_| ReprocessProviderEventError::IdentityConflict)?;
    let digest = Sha256::digest(raw.as_slice())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if digest != reprocess_row_string(event, "body_digest")? {
        return Err(ReprocessProviderEventError::IdentityConflict);
    }
    Ok(())
}

async fn select_reprocess_candidate<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    event_kind: &str,
    adapter_kind: &str,
    channel_id: &str,
    credential_version_id: &str,
    evidence: &StoredReprocessEvidence,
) -> Result<ReprocessCandidate, ReprocessProviderEventError> {
    if let Some(attempt_id) = &evidence.attempt_id {
        let row = connection
            .query_one(db.stmt(
                "SELECT order_id FROM store_payment_attempts WHERE id = $1",
                vec![attempt_id.clone().into()],
            ))
            .await
            .map_err(reprocess_storage)?
            .ok_or(ReprocessProviderEventError::ProviderQueryRequired)?;
        return Ok(ReprocessCandidate {
            order_id: reprocess_row_string(&row, "order_id")?,
            attempt_id: attempt_id.clone(),
        });
    }
    if event_kind != "payment_succeeded" || !matches!(adapter_kind, "alipay" | "wechat") {
        return Err(ReprocessProviderEventError::ProviderQueryRequired);
    }
    let account_identity = evidence
        .account_identity
        .as_deref()
        .ok_or(ReprocessProviderEventError::IdentityConflict)?;
    let (sql, values) = if adapter_kind == "alipay" {
        (
            "SELECT a.id AS attempt_id, a.order_id
         FROM store_payment_attempts a
         JOIN store_orders o ON o.id = a.order_id
         WHERE o.order_number = $1 AND a.channel_id = $2 AND a.adapter_kind = 'alipay'
           AND a.credential_version_id = $3
           AND a.merchant_account_identity = $4
           AND (a.provider_object_id = $1
                OR (a.provider_object_id IS NULL
                    AND (a.state = 'created'
                         OR (a.state = 'failed' AND a.failure_kind = 'provider_rejected'))))
         ORDER BY a.created_at DESC, a.id DESC LIMIT 2",
            vec![
                evidence.order_number.clone().into(),
                channel_id.into(),
                credential_version_id.into(),
                account_identity.into(),
            ],
        )
    } else {
        (
            "SELECT a.id AS attempt_id, a.order_id
         FROM store_payment_attempts a
         JOIN store_orders o ON o.id = a.order_id
         WHERE o.order_number = $1 AND a.channel_id = $2 AND a.adapter_kind = 'wechat'
           AND a.merchant_account_identity = $3
           AND (a.provider_object_id = $1
                OR (a.provider_object_id IS NULL
                    AND (a.state = 'created'
                         OR (a.state = 'failed' AND a.failure_kind = 'provider_rejected'))))
         ORDER BY a.created_at DESC, a.id DESC LIMIT 2",
            vec![
                evidence.order_number.clone().into(),
                channel_id.into(),
                account_identity.into(),
            ],
        )
    };
    let candidates = connection
        .query_all(db.stmt(sql, values))
        .await
        .map_err(reprocess_storage)?;
    if candidates.len() != 1 {
        return Err(ReprocessProviderEventError::ManualReview);
    }
    Ok(ReprocessCandidate {
        order_id: reprocess_row_string(&candidates[0], "order_id")?,
        attempt_id: reprocess_row_string(&candidates[0], "attempt_id")?,
    })
}

fn validate_reprocess_contract(
    event_kind: &str,
    adapter_kind: &str,
    channel_id: &str,
    event_credential_version_id: &str,
    event_credential: &QueryResult,
    verification_credential_version_id: &str,
    verification_credential: &QueryResult,
    attempt_credential_version_id: &str,
    attempt_credential: &QueryResult,
    contract: &QueryResult,
    evidence: &StoredReprocessEvidence,
) -> Result<(), ReprocessProviderEventError> {
    let merchant_identity = reprocess_row_string(contract, "merchant_account_identity")?;
    let expected_identity = evidence
        .account_identity
        .as_deref()
        .unwrap_or(&merchant_identity);
    let credential_ids_match =
        if event_kind == "payment_query_succeeded" || adapter_kind != "wechat" {
            event_credential_version_id == attempt_credential_version_id
                && verification_credential_version_id == event_credential_version_id
        } else {
            evidence.verification_credential_version_id.as_deref()
                == Some(verification_credential_version_id)
                && (event_credential_version_id == attempt_credential_version_id
                    || event_credential_version_id == verification_credential_version_id)
        };
    let credential_rows_match = [
        event_credential,
        verification_credential,
        attempt_credential,
    ]
    .iter()
    .all(|row| {
        reprocess_row_string(row, "channel_id").as_deref() == Ok(channel_id)
            && reprocess_row_string(row, "adapter_kind").as_deref() == Ok(adapter_kind)
            && reprocess_row_string(row, "account_identity_digest").as_deref()
                == Ok(expected_identity)
    });
    let stored_provider_object = reprocess_row_optional_string(contract, "provider_object_id")?;
    let absent_object_may_bind = stored_provider_object.is_none()
        && matches!(adapter_kind, "alipay" | "wechat")
        && evidence.provider_object_id == evidence.order_number
        && matches!(
            (
                reprocess_row_string(contract, "attempt_state")?.as_str(),
                reprocess_row_optional_string(contract, "failure_kind")?.as_deref(),
            ),
            ("created", None) | ("failed", Some("provider_rejected"))
        );
    let provider_object_matches = stored_provider_object
        .as_deref()
        .is_some_and(|value| value == evidence.provider_object_id)
        || absent_object_may_bind;
    let stored_transaction = reprocess_row_optional_string(contract, "provider_transaction_id")?;
    if reprocess_row_string(contract, "channel_id")? != channel_id
        || reprocess_row_string(contract, "adapter_kind")? != adapter_kind
        || reprocess_row_string(contract, "credential_version_id")? != attempt_credential_version_id
        || !credential_ids_match
        || !credential_rows_match
        || merchant_identity != expected_identity
        || !provider_object_matches
        || stored_transaction
            .as_deref()
            .is_some_and(|value| value != evidence.provider_transaction_id)
        || reprocess_row_string(contract, "order_number")? != evidence.order_number
        || reprocess_row_string(contract, "payment_minor")? != evidence.amount_minor
        || reprocess_row_string(contract, "payment_currency")? != evidence.currency
        || parse_minor(&evidence.amount_minor).is_err()
        || !matches!(evidence.currency.as_str(), "CNY" | "USD")
    {
        return Err(ReprocessProviderEventError::IdentityConflict);
    }
    Ok(())
}

fn validate_query_digest(
    event_kind: &str,
    event: &QueryResult,
    contract: &QueryResult,
    evidence: &StoredReprocessEvidence,
) -> Result<(), ReprocessProviderEventError> {
    if event_kind != "payment_query_succeeded" {
        return Ok(());
    }
    let stored = reprocess_row_string(event, "body_digest")?;
    let parsed = reprocess_row_string(event, "parsed_json")?;
    let parsed_digest = hex_sha256(parsed.as_bytes());
    let fields = [
        reprocess_row_string(contract, "attempt_id")?,
        reprocess_row_string(contract, "order_id")?,
        reprocess_row_string(contract, "channel_id")?,
        reprocess_row_string(contract, "credential_version_id")?,
        evidence.provider_object_id.clone(),
        reprocess_row_string(contract, "merchant_account_identity")?,
        evidence.order_number.clone(),
        evidence.amount_minor.clone(),
        evidence.currency.clone(),
        evidence.provider_transaction_id.clone(),
    ];
    let identity = fields
        .iter()
        .map(|value| format!("{}:{value}", value.len()))
        .collect::<Vec<_>>()
        .join("|");
    if stored != parsed_digest && stored != hex_sha256(identity.as_bytes()) {
        return Err(ReprocessProviderEventError::IdentityConflict);
    }
    Ok(())
}

async fn append_reprocess_audit<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    actor_id: &str,
    event_id: &str,
    prior_state: Option<&str>,
    prior_revision: Option<i64>,
    result: &str,
) -> Result<(), ReprocessProviderEventError> {
    connection
        .execute(db.stmt(
            "INSERT INTO store_access_audits
                (id, actor_id, actor_role, action, scope_json, reason, result, created_at)
             VALUES ($1, $2, 'admin', 'provider_event_reprocess', $3,
                     'reprocess', $4, $5)",
            vec![
                Uuid::new_v4().to_string().into(),
                actor_id.into(),
                serde_json::json!({
                    "event_id": event_id,
                    "prior_projection_state": prior_state,
                    "prior_state_revision": prior_revision,
                    "result": result,
                })
                .to_string()
                .into(),
                result.into(),
                timestamp(Utc::now()).into(),
            ],
        ))
        .await
        .map_err(reprocess_storage)?;
    Ok(())
}

fn hex_sha256(value: &[u8]) -> String {
    Sha256::digest(value)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn reprocess_row_string(
    row: &QueryResult,
    column: &str,
) -> Result<String, ReprocessProviderEventError> {
    row.try_get("", column).map_err(reprocess_storage)
}

fn reprocess_row_optional_string(
    row: &QueryResult,
    column: &str,
) -> Result<Option<String>, ReprocessProviderEventError> {
    row.try_get("", column).map_err(reprocess_storage)
}

fn reprocess_row_i32(row: &QueryResult, column: &str) -> Result<i32, ReprocessProviderEventError> {
    row.try_get("", column).map_err(reprocess_storage)
}

fn reprocess_row_i64(row: &QueryResult, column: &str) -> Result<i64, ReprocessProviderEventError> {
    row.try_get("", column).map_err(reprocess_storage)
}

fn reprocess_storage(error: impl ToString) -> ReprocessProviderEventError {
    ReprocessProviderEventError::Storage(error.to_string())
}

fn reprocess_error_result(error: &ReprocessProviderEventError) -> &'static str {
    match error {
        ReprocessProviderEventError::InvalidInput => "invalid_request",
        ReprocessProviderEventError::NotFound => "event_not_found",
        ReprocessProviderEventError::NotReprocessable => "event_not_reprocessable",
        ReprocessProviderEventError::ManualReview => "projection_manual_review",
        ReprocessProviderEventError::ProviderQueryRequired => "provider_query_required",
        ReprocessProviderEventError::IdentityConflict => "event_identity_conflict",
        ReprocessProviderEventError::Storage(_) | ReprocessProviderEventError::Fulfillment(_) => {
            "internal_error"
        }
    }
}

async fn finish_order_fulfillment<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    order_id: &str,
    expected_revision: i64,
    now: DateTime<Utc>,
) -> Result<(), CallbackStoreError> {
    let now = timestamp(now);
    let changed = connection
        .execute(db.stmt(
            "UPDATE store_orders
             SET fulfillment_state = 'fulfilled', fulfillment_started_at = $2,
                 fulfilled_at = $2, updated_at = $2, state_revision = state_revision + 1
             WHERE id = $1 AND payment_state = 'paid'
               AND fulfillment_state IN ('pending', 'failed') AND payment_hold = 0
               AND state_revision = $3",
            vec![order_id.into(), now.into(), expected_revision.into()],
        ))
        .await
        .map_err(storage)?;
    if changed.rows_affected() != 1 {
        return Err(CallbackStoreError::Storage(
            "order state changed during fulfillment".to_string(),
        ));
    }
    connection
        .execute(db.stmt(
            "DELETE FROM store_fulfillment_retries WHERE order_id = $1",
            vec![order_id.into()],
        ))
        .await
        .map_err(storage)?;
    Ok(())
}

async fn validate_reconciliation_fence<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    owner_id: &str,
    epoch: i64,
    now: DateTime<Utc>,
    lock: &str,
) -> Result<(), CallbackStoreError> {
    let lease = connection
        .query_one(db.stmt(
            &format!(
                "SELECT owner_id, epoch, expires_at FROM store_reconciliation_leases
                 WHERE name = 'store_reconciler'{lock}"
            ),
            vec![],
        ))
        .await
        .map_err(storage)?
        .ok_or_else(|| {
            CallbackStoreError::Storage("reconciliation lease is missing".to_string())
        })?;
    let expires_at = DateTime::parse_from_rfc3339(&row_string(&lease, "expires_at")?)
        .map_err(storage)?
        .with_timezone(&Utc);
    if row_string(&lease, "owner_id")? != owner_id
        || row_i64(&lease, "epoch")? != epoch
        || expires_at <= now
    {
        return Err(CallbackStoreError::Storage(
            "reconciliation lease was lost".to_string(),
        ));
    }
    Ok(())
}

async fn insert_event<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    id: &str,
    credential_version_id: &str,
    input: &ApplyProviderEventInput,
    projection_state: &str,
) -> Result<(), CallbackStoreError> {
    connection
        .execute(db.stmt(
            "INSERT INTO store_provider_events
                (id, credential_version_id, provider_event_id, event_kind,
                 body_digest, parsed_json, verification_result, projection_state,
                 raw_format_version, raw_key_id, raw_nonce_base64, raw_ciphertext_base64,
                 source_ip, user_agent, state_revision, received_at, applied_at)
             VALUES ($1, $2, $3, $4, $5, $6, 'verified', $7,
                     $8, $9, $10, $11, $12, $13, 0, $14, $15)",
            vec![
                id.into(),
                credential_version_id.into(),
                input.provider_event_id.clone().into(),
                input.event_kind.clone().into(),
                input.body_digest.clone().into(),
                input.parsed_json.to_string().into(),
                projection_state.into(),
                input
                    .raw_body
                    .as_ref()
                    .map(|raw| i32::from(raw.version))
                    .into(),
                input.raw_body.as_ref().map(|raw| raw.key_id.clone()).into(),
                input
                    .raw_body
                    .as_ref()
                    .map(|raw| raw.nonce_base64.clone())
                    .into(),
                input
                    .raw_body
                    .as_ref()
                    .map(|raw| raw.ciphertext_base64.clone())
                    .into(),
                input.source_ip.clone().into(),
                input.user_agent.clone().into(),
                timestamp(input.received_at).into(),
                (projection_state == "applied")
                    .then(|| timestamp(input.received_at))
                    .into(),
            ],
        ))
        .await
        .map_err(storage)?;
    Ok(())
}

fn duplicate_result(row: &QueryResult) -> Result<CallbackApplyResult, CallbackStoreError> {
    match row_string(row, "projection_state")?.as_str() {
        "applied" => Ok(CallbackApplyResult::Duplicate),
        "manual_review" => Ok(CallbackApplyResult::ManualReview),
        value => Err(CallbackStoreError::Storage(format!(
            "unknown callback projection state: {value}"
        ))),
    }
}

#[allow(clippy::too_many_arguments)]
async fn duplicate_result_with_evidence<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    row: &QueryResult,
    credential_version_id: &str,
    provider_event_id: &str,
    body_digest: &str,
    parsed_json: &serde_json::Value,
    order_id: Option<&str>,
    channel_id: Option<&str>,
    received_at: DateTime<Utc>,
) -> Result<CallbackApplyResult, CallbackStoreError> {
    let stored_body_digest = row_string(row, "body_digest")?;
    let stored_parsed_json = row_string(row, "parsed_json")?;
    let stored_parsed_value = serde_json::from_str(&stored_parsed_json).map_err(storage)?;
    let stored_identity = immutable_parsed_identity(&stored_parsed_value).to_string();
    let incoming_identity = immutable_parsed_identity(parsed_json).to_string();
    if stored_body_digest == body_digest && stored_identity == incoming_identity {
        return duplicate_result(row);
    }

    let case_id = format!(
        "provider-event-identity-conflict:{}",
        length_prefixed_sha256(&[credential_version_id, provider_event_id])
    );
    let evidence = serde_json::json!({
        "credential_version_id": credential_version_id,
        "provider_event_id": provider_event_id,
        "stored_body_digest": stored_body_digest,
        "incoming_body_digest": body_digest,
        "stored_parsed_identity_digest": length_prefixed_sha256(&[&stored_identity]),
        "incoming_parsed_identity_digest": length_prefixed_sha256(&[&incoming_identity]),
    })
    .to_string();
    let now = timestamp(received_at);
    connection
        .execute(db.stmt(
            "INSERT INTO store_reconciliation_cases
                (id, order_id, channel_id, severity, kind, state, evidence_json,
                 created_at, updated_at)
             VALUES ($1, $2, $3, 'high', 'provider_event_identity_conflict',
                     'open', $4, $5, $5)
             ON CONFLICT (id) DO UPDATE SET
                state = 'open', evidence_json = $4, updated_at = $5, closed_at = NULL",
            vec![
                case_id.into(),
                order_id.map(str::to_string).into(),
                channel_id.map(str::to_string).into(),
                evidence.into(),
                now.into(),
            ],
        ))
        .await
        .map_err(storage)?;
    Ok(CallbackApplyResult::ManualReview)
}

fn immutable_parsed_identity(value: &serde_json::Value) -> serde_json::Value {
    const KEYS: &[&str] = &[
        "event_id",
        "event_kind",
        "type",
        "checkout_session_id",
        "payment_intent_id",
        "attempt_id",
        "trade_no",
        "transaction_id",
        "provider_object_id",
        "provider_transaction_id",
        "order_number",
        "amount_minor",
        "currency",
    ];
    let mut identity = serde_json::Map::new();
    if let Some(object) = value.as_object() {
        for key in KEYS {
            if let Some(field) = object.get(*key) {
                identity.insert((*key).to_string(), field.clone());
            }
        }
    }
    serde_json::Value::Object(identity)
}

fn length_prefixed_sha256(values: &[&str]) -> String {
    let mut digest = Sha256::new();
    for value in values {
        let bytes = value.as_bytes();
        digest.update(
            u64::try_from(bytes.len())
                .expect("identity field length fits u64")
                .to_be_bytes(),
        );
        digest.update(bytes);
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Debug, Deserialize)]
struct OrderQuote {
    product: ProductQuote,
}

#[derive(Debug, Deserialize)]
struct ProductQuote {
    id: String,
    name: String,
    duration_seconds: Option<i64>,
    group_ids: Vec<String>,
    #[serde(default)]
    quotas: Vec<crate::store_billing::models::PlanQuota>,
    balance: Option<BalanceQuote>,
}

#[derive(Debug, Deserialize)]
struct BalanceQuote {
    actual_received_minor: String,
}

fn validate_input(input: &ApplyProviderEventInput) -> Result<(), CallbackStoreError> {
    if Uuid::parse_str(&input.event_row_id).is_err()
        || input.credential_version_id.is_empty()
        || input.verification_credential_version_id.is_empty()
        || input.provider_event_id.is_empty()
        || !matches!(
            input.event_kind.as_str(),
            "payment_succeeded" | "payment_query_succeeded"
        )
        || input.order_id.is_empty()
        || input.attempt_id.is_empty()
        || input.provider_transaction_id.is_empty()
        || input.provider_object_id.is_empty()
        || input.order_number.is_empty()
        || input.merchant_account_identity.len() != 64
        || !input
            .merchant_account_identity
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || input.raw_body.as_ref().is_some_and(|raw| {
            raw.version == 0
                || raw.key_id.is_empty()
                || raw.nonce_base64.is_empty()
                || raw.ciphertext_base64.is_empty()
        })
        || (input.event_kind == "payment_succeeded" && input.raw_body.is_none())
        || (input.event_kind == "payment_query_succeeded" && input.raw_body.is_some())
        || input.body_digest.len() != 64
        || !input
            .body_digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || parse_minor(&input.amount_minor).is_err()
    {
        return Err(CallbackStoreError::InvalidInput);
    }
    Ok(())
}

fn currency_string(currency: Currency) -> &'static str {
    match currency {
        Currency::CNY => "CNY",
        Currency::USD => "USD",
    }
}

fn row_string(row: &QueryResult, column: &str) -> Result<String, CallbackStoreError> {
    row.try_get("", column).map_err(storage)
}

fn row_optional_string(
    row: &QueryResult,
    column: &str,
) -> Result<Option<String>, CallbackStoreError> {
    row.try_get("", column).map_err(storage)
}

fn row_i32(row: &QueryResult, column: &str) -> Result<i32, CallbackStoreError> {
    row.try_get("", column).map_err(storage)
}

fn row_i64(row: &QueryResult, column: &str) -> Result<i64, CallbackStoreError> {
    row.try_get("", column).map_err(storage)
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn storage(error: impl ToString) -> CallbackStoreError {
    CallbackStoreError::Storage(error.to_string())
}
