use chrono::{DateTime, SecondsFormat, Utc};
use sea_orm::{ConnectionTrait, QueryResult};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::crypto::EncryptedSecret;
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
        if row_string(&row, "product_kind")? != "balance" {
            return Err(CallbackStoreError::Fulfillment(
                "plan fulfillment is handled by the quota workstream".to_string(),
            ));
        }

        let quote: OrderQuote = serde_json::from_str(&row_string(&row, "quote_json")?)
            .map_err(|error| CallbackStoreError::Fulfillment(error.to_string()))?;
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
