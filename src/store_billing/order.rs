use chrono::{DateTime, Duration, SecondsFormat, Utc};
use sea_orm::{ConnectionTrait, QueryResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::exchange_rate::ExchangeRateSnapshot;
use super::models::StoreSettings;
use super::money::{Currency, ExchangeRateRational, convert_minor_rational, parse_minor};
use super::payment::CheckoutAction;
use super::quota_gate::QuotaGateStore;
use super::state_machine::{FulfillmentState, PaymentState};
use super::store::StoreBillingStore;
use crate::db::DbPool;

const ORDER_LIFETIME_MINUTES: i64 = 30;
const ORDER_CREATION_LIMIT_PER_MINUTE: i64 = 5;
const OPEN_ORDER_LIMIT: i64 = 10;
const POSTGRES_ORDER_CREATION_USER_LOCK_SQL: &str = "SELECT id FROM users WHERE id = $1 FOR UPDATE";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatePaymentOrderInput {
    pub idempotency_key: String,
    pub product_id: String,
    pub payment_channel_id: String,
    pub payment_currency: Currency,
    pub custom_recharge_minor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatePaymentAttemptInput {
    pub idempotency_key: String,
    pub expected_payment_method: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentOrder {
    pub id: String,
    pub order_number: String,
    pub user_id: String,
    pub product_id: String,
    pub product_kind: String,
    pub payment_state: PaymentState,
    pub fulfillment_state: FulfillmentState,
    pub dispute_state: String,
    pub payment_hold: bool,
    pub payment_channel_id: String,
    pub payment_currency: Currency,
    pub payment_minor: String,
    pub cny_per_usd: String,
    pub rate_numerator: String,
    pub rate_denominator: String,
    pub quote: serde_json::Value,
    pub contract_version: i32,
    pub state_revision: i64,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentAttemptState {
    Created,
    Presented,
    Expired,
    Failed,
    Paid,
}

impl PaymentAttemptState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Presented => "presented",
            Self::Expired => "expired",
            Self::Failed => "failed",
            Self::Paid => "paid",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentAttemptFailureKind {
    ConfigurationUnavailable,
    ProviderRejected,
}

impl PaymentAttemptFailureKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConfigurationUnavailable => "configuration_unavailable",
            Self::ProviderRejected => "provider_rejected",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentAttempt {
    pub id: String,
    pub order_id: String,
    pub channel_id: String,
    pub adapter_kind: String,
    pub credential_version_id: String,
    pub merchant_account_identity: String,
    pub expected_payment_method: Option<String>,
    pub payment_contract_version: i32,
    pub state: PaymentAttemptState,
    pub failure_kind: Option<PaymentAttemptFailureKind>,
    pub idempotency_key: String,
    pub provider_object_id: Option<String>,
    pub action: Option<CheckoutAction>,
    pub provider_expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatePaymentAttemptOutcome {
    pub attempt: PaymentAttempt,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PaymentOrderError {
    #[error("invalid order input")]
    InvalidInput,
    #[error("invalid payment amount")]
    InvalidAmount,
    #[error("exchange rate is unavailable")]
    InvalidExchangeRate,
    #[error("product is unavailable")]
    ProductUnavailable,
    #[error("payment Channel is unavailable")]
    ChannelUnavailable,
    #[error("order does not exist")]
    OrderNotFound,
    #[error("idempotency key conflicts with another request")]
    IdempotencyConflict,
    #[error("order creation rate limit exceeded")]
    CreationRateLimited,
    #[error("too many open orders")]
    OpenOrderLimit,
    #[error("payment hold blocks Store purchases")]
    PaymentHold,
    #[error("an active payment attempt already exists")]
    ActiveAttemptExists,
    #[error("provider state must be queried before another payment attempt")]
    ProviderQueryRequired,
    #[error("order cannot accept a payment attempt")]
    OrderNotPayable,
    #[error("amount overflow")]
    AmountOverflow,
    #[error("Store payment storage failed: {0}")]
    Storage(String),
}

#[derive(Debug, Clone)]
pub struct PaymentOrderStore {
    db: DbPool,
}

impl PaymentOrderStore {
    pub fn new(db: DbPool) -> Self {
        Self { db }
    }

    pub async fn create_order(
        &self,
        user_id: &str,
        input: CreatePaymentOrderInput,
        snapshot: &ExchangeRateSnapshot,
    ) -> Result<PaymentOrder, PaymentOrderError> {
        validate_idempotency_key(&input.idempotency_key)?;
        let request_digest = order_request_digest(&input);
        if let Some(existing) = self
            .order_by_creation_key(user_id, &input.idempotency_key)
            .await?
        {
            if existing.1 == request_digest {
                return Ok(existing.0);
            }
            return Err(PaymentOrderError::IdempotencyConflict);
        }

        if snapshot.base != "USD" || snapshot.quote != "CNY" {
            return Err(PaymentOrderError::InvalidExchangeRate);
        }
        let rate = ExchangeRateRational::parse(&snapshot.cny_per_usd).map_err(map_money_error)?;
        let settings = StoreBillingStore::new(self.db.clone())
            .get_settings()
            .await
            .map_err(|error| PaymentOrderError::Storage(error.to_string()))?;
        let plan_features_enabled = QuotaGateStore::new(self.db.clone())
            .plan_features_enabled()
            .await
            .map_err(|error| PaymentOrderError::Storage(error.to_string()))?;
        let now = Utc::now();
        let now_text = timestamp(now);
        let recent_text = timestamp(now - Duration::minutes(1));
        let tx = self.db.begin_write().await.map_err(storage)?;

        lock_order_creation_user(&self.db, &*tx, user_id).await?;
        if let Some(existing) =
            query_order_by_creation_key(&self.db, &*tx, user_id, &input.idempotency_key).await?
        {
            if existing.1 == request_digest {
                tx.commit().await.map_err(storage)?;
                return Ok(existing.0);
            }
            return Err(PaymentOrderError::IdempotencyConflict);
        }

        let recent_count = count_value(
            tx.query_one(self.db.stmt(
                "SELECT COUNT(*) AS value FROM store_orders
                 WHERE user_id = $1 AND created_at >= $2",
                vec![user_id.into(), recent_text.into()],
            ))
            .await
            .map_err(storage)?,
        )?;
        let payment_hold_count = count_value(
            tx.query_one(self.db.stmt(
                "SELECT COUNT(*) AS value FROM store_balance_holds
                 WHERE user_id = $1 AND active = 1",
                vec![user_id.into()],
            ))
            .await
            .map_err(storage)?,
        )?;
        if payment_hold_count != 0 {
            return Err(PaymentOrderError::PaymentHold);
        }
        if recent_count >= ORDER_CREATION_LIMIT_PER_MINUTE {
            return Err(PaymentOrderError::CreationRateLimited);
        }
        let open_count = count_value(
            tx.query_one(self.db.stmt(
                "SELECT COUNT(*) AS value FROM store_orders
                 WHERE user_id = $1 AND payment_state = 'unpaid' AND expires_at > $2",
                vec![user_id.into(), now_text.clone().into()],
            ))
            .await
            .map_err(storage)?,
        )?;
        if open_count >= OPEN_ORDER_LIMIT {
            return Err(PaymentOrderError::OpenOrderLimit);
        }

        let product = tx
            .query_one(self.db.stmt(
                "SELECT p.id, p.kind, p.name, p.description, p.price_currency,
                        p.price_minor, p.duration_seconds, p.group_ids,
                        b.recharge_minor, b.bonus_minor
                 FROM store_products p
                 LEFT JOIN store_balance_products b ON b.product_id = p.id
                 WHERE p.id = $1 AND p.enabled = 1",
                vec![input.product_id.clone().into()],
            ))
            .await
            .map_err(storage)?
            .ok_or(PaymentOrderError::ProductUnavailable)?;
        let channel = tx
            .query_one(self.db.stmt(
                "SELECT id, adapter_kind, name, icon_kind, icon_value
                 FROM store_payment_channels WHERE id = $1 AND enabled = 1",
                vec![input.payment_channel_id.clone().into()],
            ))
            .await
            .map_err(storage)?
            .ok_or(PaymentOrderError::ChannelUnavailable)?;

        let product_kind = row_string(&product, "kind")?;
        if product_kind == "plan" && !plan_features_enabled {
            return Err(PaymentOrderError::ProductUnavailable);
        }
        let quota_quote = if product_kind == "plan" {
            tx.query_all(self.db.stmt(
                "SELECT id, window_kind, window_seconds, quota_fen_cny, sort_order
                 FROM store_plan_quotas WHERE product_id = $1
                 ORDER BY sort_order, id",
                vec![input.product_id.clone().into()],
            ))
            .await
            .map_err(storage)?
            .into_iter()
            .map(|row| {
                Ok(serde_json::json!({
                    "id": row_string(&row, "id")?,
                    "window_kind": row_string(&row, "window_kind")?,
                    "window_seconds": row.try_get::<i64>("", "window_seconds").map_err(storage)?,
                    "quota_fen_cny": row_string(&row, "quota_fen_cny")?,
                    "sort_order": row.try_get::<i32>("", "sort_order").map_err(storage)?,
                }))
            })
            .collect::<Result<Vec<_>, PaymentOrderError>>()?
        } else {
            Vec::new()
        };
        let product_currency = parse_currency(&row_string(&product, "price_currency")?)?;
        let product_price =
            parse_minor(&row_string(&product, "price_minor")?).map_err(map_money_error)?;
        let (payment_minor, balance_quote) = if product_kind == "balance" {
            quote_balance(
                &product,
                &input,
                &settings,
                product_currency,
                product_price,
                &rate,
            )?
        } else if product_kind == "plan" {
            if input.custom_recharge_minor.is_some() {
                return Err(PaymentOrderError::InvalidAmount);
            }
            (
                convert_minor_rational(
                    product_price,
                    product_currency,
                    input.payment_currency,
                    &rate,
                )
                .map_err(map_money_error)?
                .to_string(),
                None,
            )
        } else {
            return Err(PaymentOrderError::ProductUnavailable);
        };
        if payment_minor == "0" {
            return Err(PaymentOrderError::InvalidAmount);
        }

        let quote = serde_json::json!({
            "version": 2,
            "product": {
                "id": row_string(&product, "id")?,
                "kind": product_kind,
                "name": row_string(&product, "name")?,
                "description": row_string(&product, "description")?,
                "price_currency": currency_string(product_currency),
                "price_minor": row_string(&product, "price_minor")?,
                "duration_seconds": row_optional_i64(&product, "duration_seconds")?,
                "group_ids": serde_json::from_str::<serde_json::Value>(&row_string(&product, "group_ids")?)
                    .map_err(|error| PaymentOrderError::Storage(error.to_string()))?,
                "balance": balance_quote,
                "quotas": quota_quote,
            },
            "payment_channel": {
                "id": row_string(&channel, "id")?,
                "adapter_kind": row_string(&channel, "adapter_kind")?,
                "name": row_string(&channel, "name")?,
                "icon_kind": row_string(&channel, "icon_kind")?,
                "icon_value": row_optional_string(&channel, "icon_value")?,
            },
            "rate": {
                "decimal": rate.decimal(),
                "numerator": rate.numerator().to_string(),
                "denominator": rate.denominator().to_string(),
                "source_updated_at": timestamp(snapshot.source_updated_at),
                "refreshed_at": timestamp(snapshot.refreshed_at),
            }
        });

        let id = Uuid::new_v4().to_string();
        let order_number = format!("LS-{}", Uuid::new_v4().simple()).to_uppercase();
        let expires_at = now + Duration::minutes(ORDER_LIFETIME_MINUTES);
        tx.execute(self.db.stmt(
            "INSERT INTO store_orders
                (id, order_number, user_id, product_id, product_kind, payment_state,
                 fulfillment_state, dispute_state, payment_hold, payment_channel_id,
                 payment_currency, payment_minor, cny_per_usd, rate_numerator,
                 rate_denominator, rate_source_updated_at, quote_json, contract_version,
                 state_revision, creation_idempotency_key, creation_request_digest,
                 expires_at, created_at, updated_at)
             VALUES
                ($1, $2, $3, $4, $5, 'unpaid', 'pending', 'none', 0, $6,
                 $7, $8, $9, $10, $11, $12, $13, 2, 0, $14, $15, $16, $17, $17)",
            vec![
                id.clone().into(),
                order_number.into(),
                user_id.into(),
                input.product_id.into(),
                product_kind.into(),
                input.payment_channel_id.into(),
                currency_string(input.payment_currency).into(),
                payment_minor.into(),
                rate.decimal().into(),
                rate.numerator().to_string().into(),
                rate.denominator().to_string().into(),
                timestamp(snapshot.source_updated_at).into(),
                quote.to_string().into(),
                input.idempotency_key.into(),
                request_digest.into(),
                timestamp(expires_at).into(),
                now_text.into(),
            ],
        ))
        .await
        .map_err(storage)?;
        let order = query_order_by_id(&self.db, &*tx, &id, None)
            .await?
            .ok_or(PaymentOrderError::OrderNotFound)?;
        tx.commit().await.map_err(storage)?;
        Ok(order)
    }

    pub async fn list_orders_for_user(
        &self,
        user_id: &str,
        limit: u64,
    ) -> Result<Vec<PaymentOrder>, PaymentOrderError> {
        self.db
            .read()
            .query_all(self.db.stmt(
                &format!(
                    "{} WHERE user_id = $1 ORDER BY created_at DESC, id DESC LIMIT $2",
                    order_select()
                ),
                vec![user_id.into(), (limit.min(100) as i64).into()],
            ))
            .await
            .map_err(storage)?
            .into_iter()
            .map(payment_order_from_row)
            .collect()
    }

    pub async fn get_order_for_user(
        &self,
        user_id: &str,
        order_id: &str,
    ) -> Result<Option<PaymentOrder>, PaymentOrderError> {
        query_order_by_id(&self.db, self.db.read(), order_id, Some(user_id)).await
    }

    pub async fn find_order_by_creation_key(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<PaymentOrder>, PaymentOrderError> {
        Ok(self
            .order_by_creation_key(user_id, key)
            .await?
            .map(|(order, _)| order))
    }

    pub async fn replay_order(
        &self,
        user_id: &str,
        input: &CreatePaymentOrderInput,
    ) -> Result<Option<PaymentOrder>, PaymentOrderError> {
        validate_idempotency_key(&input.idempotency_key)?;
        let Some((order, digest)) = self
            .order_by_creation_key(user_id, &input.idempotency_key)
            .await?
        else {
            return Ok(None);
        };
        if digest != order_request_digest(input) {
            return Err(PaymentOrderError::IdempotencyConflict);
        }
        Ok(Some(order))
    }

    pub async fn list_orders_admin(
        &self,
        limit: u64,
    ) -> Result<Vec<PaymentOrder>, PaymentOrderError> {
        self.db
            .read()
            .query_all(self.db.stmt(
                &format!(
                    "{} ORDER BY created_at DESC, id DESC LIMIT $1",
                    order_select()
                ),
                vec![(limit.min(100) as i64).into()],
            ))
            .await
            .map_err(storage)?
            .into_iter()
            .map(payment_order_from_row)
            .collect()
    }

    pub async fn create_attempt(
        &self,
        user_id: &str,
        order_id: &str,
        input: CreatePaymentAttemptInput,
    ) -> Result<PaymentAttempt, PaymentOrderError> {
        Ok(self
            .create_attempt_with_outcome(user_id, order_id, input)
            .await?
            .attempt)
    }

    pub async fn create_attempt_with_outcome(
        &self,
        user_id: &str,
        order_id: &str,
        input: CreatePaymentAttemptInput,
    ) -> Result<CreatePaymentAttemptOutcome, PaymentOrderError> {
        validate_idempotency_key(&input.idempotency_key)?;
        let tx = self.db.begin_write().await.map_err(storage)?;
        if let Some(existing) = query_attempt_by_key(&self.db, &*tx, &input.idempotency_key).await?
        {
            if existing.order_id == order_id {
                query_order_by_id_for_update(&self.db, &*tx, order_id, Some(user_id))
                    .await?
                    .ok_or(PaymentOrderError::OrderNotFound)?;
                tx.commit().await.map_err(storage)?;
                return Ok(CreatePaymentAttemptOutcome {
                    attempt: existing,
                    replayed: true,
                });
            }
            return Err(PaymentOrderError::IdempotencyConflict);
        }
        let order = query_order_by_id_for_update(&self.db, &*tx, order_id, Some(user_id))
            .await?
            .ok_or(PaymentOrderError::OrderNotFound)?;
        if order.payment_state != PaymentState::Unpaid || order.expires_at <= Utc::now() {
            return Err(PaymentOrderError::OrderNotPayable);
        }
        let rejected_count = count_value(
            tx.query_one(self.db.stmt(
                "SELECT COUNT(*) AS value FROM store_payment_attempts
                 WHERE order_id = $1 AND state = 'failed'
                   AND failure_kind = 'provider_rejected' AND adapter_kind <> 'stripe'",
                vec![order_id.into()],
            ))
            .await
            .map_err(storage)?,
        )?;
        if rejected_count != 0 {
            return Err(PaymentOrderError::ProviderQueryRequired);
        }
        let active_count = count_value(
            tx.query_one(self.db.stmt(
                "SELECT COUNT(*) AS value FROM store_payment_attempts
                 WHERE order_id = $1 AND state IN ('created', 'presented')",
                vec![order_id.into()],
            ))
            .await
            .map_err(storage)?,
        )?;
        if active_count != 0 {
            return Err(PaymentOrderError::ActiveAttemptExists);
        }
        let credential = tx
            .query_one(self.db.stmt(
                "SELECT id, account_identity_digest FROM store_channel_credentials
                 WHERE channel_id = $1 AND status = 'active'
                 ORDER BY created_at DESC, id DESC LIMIT 1",
                vec![order.payment_channel_id.clone().into()],
            ))
            .await
            .map_err(storage)?
            .ok_or(PaymentOrderError::ChannelUnavailable)?;
        let channel = tx
            .query_one(self.db.stmt(
                "SELECT adapter_kind FROM store_payment_channels
                 WHERE id = $1 AND enabled = 1",
                vec![order.payment_channel_id.clone().into()],
            ))
            .await
            .map_err(storage)?
            .ok_or(PaymentOrderError::ChannelUnavailable)?;

        let id = Uuid::new_v4().to_string();
        let now = timestamp(Utc::now());
        tx.execute(self.db.stmt(
            "INSERT INTO store_payment_attempts
                    (id, order_id, channel_id, adapter_kind, credential_version_id,
                     merchant_account_identity, expected_payment_method,
                     payment_contract_version, state, idempotency_key, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'created', $9, $10, $10)",
            vec![
                id.clone().into(),
                order_id.into(),
                order.payment_channel_id.into(),
                row_string(&channel, "adapter_kind")?.into(),
                row_string(&credential, "id")?.into(),
                row_string(&credential, "account_identity_digest")?.into(),
                input.expected_payment_method.into(),
                order.contract_version.into(),
                input.idempotency_key.into(),
                now.into(),
            ],
        ))
        .await
        .map_err(storage)?;
        let attempt = query_attempt_by_id(&self.db, &*tx, &id)
            .await?
            .ok_or_else(|| PaymentOrderError::Storage("inserted attempt is missing".to_string()))?;
        tx.commit().await.map_err(storage)?;
        Ok(CreatePaymentAttemptOutcome {
            attempt,
            replayed: false,
        })
    }

    pub async fn present_attempt(
        &self,
        user_id: &str,
        attempt_id: &str,
        provider_object_id: &str,
        action: &CheckoutAction,
    ) -> Result<PaymentAttempt, PaymentOrderError> {
        if provider_object_id.trim().is_empty() {
            return Err(PaymentOrderError::InvalidInput);
        }
        let tx = self.db.begin_write().await.map_err(storage)?;
        let attempt = query_attempt_by_id(&self.db, &*tx, attempt_id)
            .await?
            .ok_or(PaymentOrderError::OrderNotFound)?;
        let order = query_order_by_id(&self.db, &*tx, &attempt.order_id, Some(user_id))
            .await?
            .ok_or(PaymentOrderError::OrderNotFound)?;
        if attempt.state == PaymentAttemptState::Presented {
            tx.commit().await.map_err(storage)?;
            return Ok(attempt);
        }
        if attempt.state != PaymentAttemptState::Created
            || order.payment_state != PaymentState::Unpaid
        {
            return Err(PaymentOrderError::OrderNotPayable);
        }
        let (action_kind, expires_at) = checkout_action_metadata(action)?;
        let now = timestamp(Utc::now());
        let changed = tx
            .execute(self.db.stmt(
                "UPDATE store_payment_attempts
                 SET state = 'presented', provider_object_id = $2, action_kind = $3,
                     action_json = $4, provider_expires_at = $5, presented_at = $6,
                     updated_at = $6
                 WHERE id = $1 AND state = 'created'",
                vec![
                    attempt_id.into(),
                    provider_object_id.into(),
                    action_kind.into(),
                    serde_json::to_string(action)
                        .map_err(|error| PaymentOrderError::Storage(error.to_string()))?
                        .into(),
                    timestamp(expires_at).into(),
                    now.into(),
                ],
            ))
            .await
            .map_err(storage)?;
        if changed.rows_affected() != 1 {
            return Err(PaymentOrderError::ActiveAttemptExists);
        }
        let presented = query_attempt_by_id(&self.db, &*tx, attempt_id)
            .await?
            .ok_or_else(|| {
                PaymentOrderError::Storage("presented attempt is missing".to_string())
            })?;
        tx.commit().await.map_err(storage)?;
        Ok(presented)
    }

    pub async fn fail_attempt(
        &self,
        user_id: &str,
        attempt_id: &str,
        failure_kind: PaymentAttemptFailureKind,
    ) -> Result<PaymentAttempt, PaymentOrderError> {
        let tx = self.db.begin_write().await.map_err(storage)?;
        let attempt = query_attempt_by_id(&self.db, &*tx, attempt_id)
            .await?
            .ok_or(PaymentOrderError::OrderNotFound)?;
        query_order_by_id(&self.db, &*tx, &attempt.order_id, Some(user_id))
            .await?
            .ok_or(PaymentOrderError::OrderNotFound)?;
        if attempt.state == PaymentAttemptState::Failed {
            if attempt.failure_kind != Some(failure_kind) {
                return Err(PaymentOrderError::OrderNotPayable);
            }
            tx.commit().await.map_err(storage)?;
            return Ok(attempt);
        }
        if attempt.state != PaymentAttemptState::Created {
            return Err(PaymentOrderError::OrderNotPayable);
        }
        let now = timestamp(Utc::now());
        let changed = tx
            .execute(self.db.stmt(
                "UPDATE store_payment_attempts
                 SET state = 'failed', failure_kind = $2, updated_at = $3
                 WHERE id = $1 AND state = 'created'",
                vec![attempt_id.into(), failure_kind.as_str().into(), now.into()],
            ))
            .await
            .map_err(storage)?;
        if changed.rows_affected() != 1 {
            return Err(PaymentOrderError::ActiveAttemptExists);
        }
        let failed = query_attempt_by_id(&self.db, &*tx, attempt_id)
            .await?
            .ok_or_else(|| PaymentOrderError::Storage("failed attempt is missing".to_string()))?;
        tx.commit().await.map_err(storage)?;
        Ok(failed)
    }

    async fn order_by_creation_key(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<(PaymentOrder, String)>, PaymentOrderError> {
        query_order_by_creation_key(&self.db, self.db.read(), user_id, key).await
    }
}

fn quote_balance(
    product: &QueryResult,
    input: &CreatePaymentOrderInput,
    settings: &StoreSettings,
    product_currency: Currency,
    product_price: i128,
    rate: &ExchangeRateRational,
) -> Result<(String, Option<serde_json::Value>), PaymentOrderError> {
    if let Some(custom) = input.custom_recharge_minor.as_deref() {
        let amount = parse_minor(custom).map_err(map_money_error)?;
        let (minimum, maximum) = match input.payment_currency {
            Currency::CNY => (
                parse_minor(&settings.custom_recharge_cny_min_minor).map_err(map_money_error)?,
                parse_minor(&settings.custom_recharge_cny_max_minor).map_err(map_money_error)?,
            ),
            Currency::USD => (
                parse_minor(&settings.custom_recharge_usd_min_minor).map_err(map_money_error)?,
                parse_minor(&settings.custom_recharge_usd_max_minor).map_err(map_money_error)?,
            ),
        };
        if amount < minimum || amount > maximum {
            return Err(PaymentOrderError::InvalidAmount);
        }
        return Ok((
            custom.to_string(),
            Some(serde_json::json!({
                "recharge_minor": custom,
                "bonus_minor": "0",
                "actual_received_minor": custom,
            })),
        ));
    }

    let recharge_source = row_optional_string(product, "recharge_minor")?
        .ok_or_else(|| PaymentOrderError::Storage("balance details are missing".to_string()))?;
    let bonus_source = row_optional_string(product, "bonus_minor")?
        .ok_or_else(|| PaymentOrderError::Storage("balance details are missing".to_string()))?;
    let recharge = convert_minor_rational(
        parse_minor(&recharge_source).map_err(map_money_error)?,
        product_currency,
        input.payment_currency,
        rate,
    )
    .map_err(map_money_error)?;
    let expected_price = convert_minor_rational(
        product_price,
        product_currency,
        input.payment_currency,
        rate,
    )
    .map_err(map_money_error)?;
    if recharge != expected_price {
        return Err(PaymentOrderError::Storage(
            "balance recharge and product price differ".to_string(),
        ));
    }
    let bonus = convert_minor_rational(
        parse_minor(&bonus_source).map_err(map_money_error)?,
        product_currency,
        input.payment_currency,
        rate,
    )
    .map_err(map_money_error)?;
    let actual = recharge
        .checked_add(bonus)
        .ok_or(PaymentOrderError::AmountOverflow)?;
    Ok((
        recharge.to_string(),
        Some(serde_json::json!({
            "recharge_minor": recharge.to_string(),
            "bonus_minor": bonus.to_string(),
            "actual_received_minor": actual.to_string(),
        })),
    ))
}

async fn lock_order_creation_user<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    user_id: &str,
) -> Result<(), PaymentOrderError> {
    if db.is_postgres() {
        let _ = connection
            .query_one(db.stmt(POSTGRES_ORDER_CREATION_USER_LOCK_SQL, vec![user_id.into()]))
            .await
            .map_err(storage)?;
    }
    Ok(())
}

async fn query_order_by_creation_key<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    user_id: &str,
    key: &str,
) -> Result<Option<(PaymentOrder, String)>, PaymentOrderError> {
    let row = connection
        .query_one(db.stmt(
            &format!(
                "{}, creation_request_digest FROM store_orders
                 WHERE user_id = $1 AND creation_idempotency_key = $2",
                order_select_columns()
            ),
            vec![user_id.into(), key.into()],
        ))
        .await
        .map_err(storage)?;
    row.map(|row| {
        let digest = row_string(&row, "creation_request_digest")?;
        Ok((payment_order_from_row(row)?, digest))
    })
    .transpose()
}

async fn query_order_by_id<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    order_id: &str,
    user_id: Option<&str>,
) -> Result<Option<PaymentOrder>, PaymentOrderError> {
    query_order_by_id_with_lock(db, connection, order_id, user_id, false).await
}

async fn query_order_by_id_for_update<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    order_id: &str,
    user_id: Option<&str>,
) -> Result<Option<PaymentOrder>, PaymentOrderError> {
    query_order_by_id_with_lock(db, connection, order_id, user_id, true).await
}

async fn query_order_by_id_with_lock<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    order_id: &str,
    user_id: Option<&str>,
    for_update: bool,
) -> Result<Option<PaymentOrder>, PaymentOrderError> {
    let (suffix, values) = if let Some(user_id) = user_id {
        (" AND user_id = $2", vec![order_id.into(), user_id.into()])
    } else {
        ("", vec![order_id.into()])
    };
    let lock = if for_update && db.is_postgres() {
        " FOR UPDATE"
    } else {
        ""
    };
    connection
        .query_one(db.stmt(
            &format!("{} WHERE id = $1{suffix}{lock}", order_select()),
            values,
        ))
        .await
        .map_err(storage)?
        .map(payment_order_from_row)
        .transpose()
}

async fn query_attempt_by_key<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    key: &str,
) -> Result<Option<PaymentAttempt>, PaymentOrderError> {
    connection
        .query_one(db.stmt(
            &format!("{} WHERE idempotency_key = $1", attempt_select()),
            vec![key.into()],
        ))
        .await
        .map_err(storage)?
        .map(payment_attempt_from_row)
        .transpose()
}

async fn query_attempt_by_id<C: ConnectionTrait>(
    db: &DbPool,
    connection: &C,
    id: &str,
) -> Result<Option<PaymentAttempt>, PaymentOrderError> {
    connection
        .query_one(db.stmt(
            &format!("{} WHERE id = $1", attempt_select()),
            vec![id.into()],
        ))
        .await
        .map_err(storage)?
        .map(payment_attempt_from_row)
        .transpose()
}

fn order_select_columns() -> &'static str {
    "SELECT id, order_number, user_id, product_id, product_kind, payment_state,
            fulfillment_state, dispute_state, payment_hold, payment_channel_id,
            payment_currency, payment_minor, cny_per_usd, rate_numerator,
            rate_denominator, quote_json, contract_version, state_revision,
            expires_at, created_at, updated_at"
}

fn order_select() -> &'static str {
    concat!(
        "SELECT id, order_number, user_id, product_id, product_kind, payment_state, ",
        "fulfillment_state, dispute_state, payment_hold, payment_channel_id, ",
        "payment_currency, payment_minor, cny_per_usd, rate_numerator, ",
        "rate_denominator, quote_json, contract_version, state_revision, ",
        "expires_at, created_at, updated_at FROM store_orders"
    )
}

fn attempt_select() -> &'static str {
    "SELECT id, order_id, channel_id, adapter_kind, credential_version_id,
            merchant_account_identity, expected_payment_method, payment_contract_version, state,
            failure_kind, idempotency_key, provider_object_id, action_json, provider_expires_at,
            created_at, updated_at
     FROM store_payment_attempts"
}

fn payment_order_from_row(row: QueryResult) -> Result<PaymentOrder, PaymentOrderError> {
    Ok(PaymentOrder {
        id: row_string(&row, "id")?,
        order_number: row_string(&row, "order_number")?,
        user_id: row_string(&row, "user_id")?,
        product_id: row_string(&row, "product_id")?,
        product_kind: row_string(&row, "product_kind")?,
        payment_state: parse_payment_state(&row_string(&row, "payment_state")?)?,
        fulfillment_state: parse_fulfillment_state(&row_string(&row, "fulfillment_state")?)?,
        dispute_state: row_string(&row, "dispute_state")?,
        payment_hold: row.try_get::<i32>("", "payment_hold").map_err(storage)? != 0,
        payment_channel_id: row_string(&row, "payment_channel_id")?,
        payment_currency: parse_currency(&row_string(&row, "payment_currency")?)?,
        payment_minor: row_string(&row, "payment_minor")?,
        cny_per_usd: row_string(&row, "cny_per_usd")?,
        rate_numerator: row_string(&row, "rate_numerator")?,
        rate_denominator: row_string(&row, "rate_denominator")?,
        quote: serde_json::from_str(&row_string(&row, "quote_json")?)
            .map_err(|error| PaymentOrderError::Storage(error.to_string()))?,
        contract_version: row
            .try_get::<i32>("", "contract_version")
            .map_err(storage)?,
        state_revision: row.try_get::<i64>("", "state_revision").map_err(storage)?,
        expires_at: parse_timestamp(&row_string(&row, "expires_at")?)?,
        created_at: parse_timestamp(&row_string(&row, "created_at")?)?,
        updated_at: parse_timestamp(&row_string(&row, "updated_at")?)?,
    })
}

fn payment_attempt_from_row(row: QueryResult) -> Result<PaymentAttempt, PaymentOrderError> {
    Ok(PaymentAttempt {
        id: row_string(&row, "id")?,
        order_id: row_string(&row, "order_id")?,
        channel_id: row_string(&row, "channel_id")?,
        adapter_kind: row_string(&row, "adapter_kind")?,
        credential_version_id: row_string(&row, "credential_version_id")?,
        merchant_account_identity: row_string(&row, "merchant_account_identity")?,
        expected_payment_method: row_optional_string(&row, "expected_payment_method")?,
        payment_contract_version: row
            .try_get::<i32>("", "payment_contract_version")
            .map_err(storage)?,
        state: parse_attempt_state(&row_string(&row, "state")?)?,
        failure_kind: row_optional_string(&row, "failure_kind")?
            .map(|value| parse_attempt_failure_kind(&value))
            .transpose()?,
        idempotency_key: row_string(&row, "idempotency_key")?,
        provider_object_id: row_optional_string(&row, "provider_object_id")?,
        action: row_optional_string(&row, "action_json")?
            .map(|value| serde_json::from_str(&value))
            .transpose()
            .map_err(|error| PaymentOrderError::Storage(error.to_string()))?,
        provider_expires_at: row_optional_string(&row, "provider_expires_at")?
            .map(|value| parse_timestamp(&value))
            .transpose()?,
        created_at: parse_timestamp(&row_string(&row, "created_at")?)?,
        updated_at: parse_timestamp(&row_string(&row, "updated_at")?)?,
    })
}

fn checkout_action_metadata(
    action: &CheckoutAction,
) -> Result<(&'static str, DateTime<Utc>), PaymentOrderError> {
    let (kind, expires_at) = match action {
        CheckoutAction::Redirect { url, expires_at } => {
            validate_checkout_url(url)?;
            ("redirect", expires_at)
        }
        CheckoutAction::Qr {
            payload,
            expires_at,
        } => {
            if payload.trim().is_empty() {
                return Err(PaymentOrderError::InvalidInput);
            }
            ("qr", expires_at)
        }
        CheckoutAction::Form {
            action, expires_at, ..
        } => {
            validate_checkout_url(action)?;
            ("form", expires_at)
        }
    };
    Ok((kind, parse_timestamp(expires_at)?))
}

fn validate_checkout_url(value: &str) -> Result<(), PaymentOrderError> {
    let url = url::Url::parse(value).map_err(|_| PaymentOrderError::InvalidInput)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(PaymentOrderError::InvalidInput);
    }
    Ok(())
}

fn validate_idempotency_key(value: &str) -> Result<(), PaymentOrderError> {
    if value.is_empty()
        || value.len() > 100
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(PaymentOrderError::InvalidInput);
    }
    Ok(())
}

fn order_request_digest(input: &CreatePaymentOrderInput) -> String {
    let payload = format!(
        "v1\0{}\0{}\0{}\0{}",
        input.product_id,
        input.payment_channel_id,
        currency_string(input.payment_currency),
        input.custom_recharge_minor.as_deref().unwrap_or("")
    );
    lower_hex(&Sha256::digest(payload.as_bytes()))
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[usize::from(byte >> 4)] as char);
        output.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    output
}

fn count_value(row: Option<QueryResult>) -> Result<i64, PaymentOrderError> {
    row.ok_or_else(|| PaymentOrderError::Storage("count query returned no row".to_string()))?
        .try_get("", "value")
        .map_err(storage)
}

fn row_string(row: &QueryResult, column: &str) -> Result<String, PaymentOrderError> {
    row.try_get("", column).map_err(storage)
}

fn row_optional_string(
    row: &QueryResult,
    column: &str,
) -> Result<Option<String>, PaymentOrderError> {
    row.try_get("", column).map_err(storage)
}

fn row_optional_i64(row: &QueryResult, column: &str) -> Result<Option<i64>, PaymentOrderError> {
    row.try_get("", column).map_err(storage)
}

fn parse_currency(value: &str) -> Result<Currency, PaymentOrderError> {
    match value {
        "CNY" => Ok(Currency::CNY),
        "USD" => Ok(Currency::USD),
        _ => Err(PaymentOrderError::Storage(
            "stored currency is invalid".to_string(),
        )),
    }
}

fn currency_string(value: Currency) -> &'static str {
    match value {
        Currency::CNY => "CNY",
        Currency::USD => "USD",
    }
}

fn parse_payment_state(value: &str) -> Result<PaymentState, PaymentOrderError> {
    match value {
        "unpaid" => Ok(PaymentState::Unpaid),
        "paid" => Ok(PaymentState::Paid),
        "refund_pending" => Ok(PaymentState::RefundPending),
        "refunded" => Ok(PaymentState::Refunded),
        "closed" => Ok(PaymentState::Closed),
        _ => Err(PaymentOrderError::Storage(
            "stored payment state is invalid".to_string(),
        )),
    }
}

fn parse_fulfillment_state(value: &str) -> Result<FulfillmentState, PaymentOrderError> {
    match value {
        "pending" => Ok(FulfillmentState::Pending),
        "fulfilled" => Ok(FulfillmentState::Fulfilled),
        "failed" => Ok(FulfillmentState::Failed),
        _ => Err(PaymentOrderError::Storage(
            "stored fulfillment state is invalid".to_string(),
        )),
    }
}

fn parse_attempt_state(value: &str) -> Result<PaymentAttemptState, PaymentOrderError> {
    match value {
        "created" => Ok(PaymentAttemptState::Created),
        "presented" => Ok(PaymentAttemptState::Presented),
        "expired" => Ok(PaymentAttemptState::Expired),
        "failed" => Ok(PaymentAttemptState::Failed),
        "paid" => Ok(PaymentAttemptState::Paid),
        _ => Err(PaymentOrderError::Storage(
            "stored payment attempt state is invalid".to_string(),
        )),
    }
}

fn parse_attempt_failure_kind(value: &str) -> Result<PaymentAttemptFailureKind, PaymentOrderError> {
    match value {
        "configuration_unavailable" => Ok(PaymentAttemptFailureKind::ConfigurationUnavailable),
        "provider_rejected" => Ok(PaymentAttemptFailureKind::ProviderRejected),
        _ => Err(PaymentOrderError::Storage(
            "stored payment attempt failure kind is invalid".to_string(),
        )),
    }
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>, PaymentOrderError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| PaymentOrderError::Storage(error.to_string()))
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn map_money_error(error: super::money::MoneyError) -> PaymentOrderError {
    match error {
        super::money::MoneyError::InvalidAmount => PaymentOrderError::InvalidAmount,
        super::money::MoneyError::InvalidExchangeRate => PaymentOrderError::InvalidExchangeRate,
        super::money::MoneyError::AmountOverflow => PaymentOrderError::AmountOverflow,
    }
}

fn storage(error: impl ToString) -> PaymentOrderError {
    PaymentOrderError::Storage(error.to_string())
}
