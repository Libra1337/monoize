use chrono::{DateTime, SecondsFormat, Utc};
use sea_orm::{ConnectionTrait, QueryResult};
use serde::Deserialize;
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
        validate_input(&input)?;
        let tx = self.db.begin_write().await.map_err(storage)?;
        let lock = if self.db.is_postgres() {
            " FOR UPDATE"
        } else {
            ""
        };
        let row = tx
            .query_one(self.db.stmt(
                &format!(
                    "SELECT a.id AS attempt_id, a.order_id, a.credential_version_id,
                            a.state AS attempt_state, o.user_id, o.product_kind,
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

        let duplicate = tx
            .query_one(self.db.stmt(
                "SELECT projection_state FROM store_provider_events
                 WHERE credential_version_id = $1 AND provider_event_id = $2",
                vec![
                    input.credential_version_id.clone().into(),
                    input.provider_event_id.clone().into(),
                ],
            ))
            .await
            .map_err(storage)?;
        if let Some(duplicate) = duplicate {
            let result = match row_string(&duplicate, "projection_state")?.as_str() {
                "applied" => CallbackApplyResult::Duplicate,
                "manual_review" => CallbackApplyResult::ManualReview,
                value => {
                    return Err(CallbackStoreError::Storage(format!(
                        "unknown callback projection state: {value}"
                    )));
                }
            };
            tx.commit().await.map_err(storage)?;
            return Ok(result);
        }

        let matches_contract = row_string(&row, "credential_version_id")?
            == input.credential_version_id
            && row_string(&row, "provider_object_id")? == input.provider_object_id
            && row_string(&row, "merchant_account_identity")? == input.merchant_account_identity
            && row_string(&row, "order_number")? == input.order_number
            && row_string(&row, "payment_minor")? == input.amount_minor
            && row_string(&row, "payment_currency")? == currency_string(input.currency);
        let event_row_id = input.event_row_id.clone();
        if !matches_contract {
            insert_event(&self.db, &*tx, &event_row_id, &input, "manual_review").await?;
            tx.commit().await.map_err(storage)?;
            return Ok(CallbackApplyResult::ManualReview);
        }

        let payment_state = row_string(&row, "payment_state")?;
        let contract_version = row_i32(&row, "contract_version")?;
        let can_apply = payment_state == "unpaid"
            || (payment_state == "closed" && contract_version == 2)
            || payment_state == "paid";
        if !can_apply {
            insert_event(&self.db, &*tx, &event_row_id, &input, "manual_review").await?;
            tx.commit().await.map_err(storage)?;
            return Ok(CallbackApplyResult::ManualReview);
        }

        insert_event(&self.db, &*tx, &event_row_id, &input, "applied").await?;
        let now = timestamp(input.received_at);
        tx.execute(self.db.stmt(
            "UPDATE store_payment_attempts
             SET state = 'paid', failure_kind = NULL,
                 provider_transaction_id = $2, paid_at = $3, updated_at = $3
             WHERE id = $1",
            vec![
                input.attempt_id.clone().into(),
                input.provider_transaction_id.clone().into(),
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

        self.fulfill_paid_order(&input.order_id).await?;
        Ok(CallbackApplyResult::Applied)
    }

    pub async fn fulfill_paid_order(&self, order_id: &str) -> Result<(), CallbackStoreError> {
        let tx = self.db.begin_write().await.map_err(storage)?;
        let lock = if self.db.is_postgres() {
            " FOR UPDATE"
        } else {
            ""
        };
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
        let now = timestamp(Utc::now());
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
        tx.commit().await.map_err(storage)
    }
}

async fn insert_event<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    id: &str,
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
                input.credential_version_id.clone().into(),
                input.provider_event_id.clone().into(),
                input.event_kind.clone().into(),
                input.body_digest.clone().into(),
                input.parsed_json.to_string().into(),
                projection_state.into(),
                i32::from(input.raw_body.version).into(),
                input.raw_body.key_id.clone().into(),
                input.raw_body.nonce_base64.clone().into(),
                input.raw_body.ciphertext_base64.clone().into(),
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
        || input.provider_event_id.is_empty()
        || input.event_kind != "payment_succeeded"
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
        || input.raw_body.version == 0
        || input.raw_body.key_id.is_empty()
        || input.raw_body.nonce_base64.is_empty()
        || input.raw_body.ciphertext_base64.is_empty()
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
