use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use sea_orm::{ConnectionTrait, QueryResult};
use sha2::{Digest, Sha256};

use super::adapters::alipay::{self, AlipayCredential};
use super::adapters::stripe::{self, StripeCredential};
use super::adapters::wechat::{self, WechatCredential, WechatPlatformVerifier};
use super::crypto::{EncryptedSecret, PaymentKeyRing};
use super::money::Currency;
use super::payment::{AdapterError, ProviderRefundState, RefundRequest};
use super::recovery::{BeginRefundInput, RecoveryError, RecoveryStore, RefundRecord};
use crate::db::DbPool;

#[derive(Clone)]
pub enum RefundCredential {
    Stripe(StripeCredential),
    Alipay(AlipayCredential),
    Wechat {
        credential: WechatCredential,
        verifiers: Vec<WechatPlatformVerifier>,
    },
}

#[derive(Clone)]
pub struct RefundProviderContract {
    pub channel_id: String,
    pub credential_version_id: String,
    pub merchant_account_identity: String,
    pub provider_refund_id: Option<String>,
    pub credential: RefundCredential,
    pub request: RefundRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefundProviderOutcome {
    pub state: ProviderRefundState,
    pub provider_refund_id: Option<String>,
    pub not_found_is_definitive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RefundQueryProjection {
    AlreadyTerminal,
    Pending { provider_refund_id: Option<String> },
    Succeeded { provider_refund_id: Option<String> },
    Failed { provider_refund_id: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RefundQueryOutcome {
    pub refund: RefundRecord,
    pub projection: RefundQueryProjection,
}

#[async_trait]
pub trait RefundProvider: Send + Sync {
    async fn create_refund(
        &self,
        contract: &RefundProviderContract,
    ) -> Result<RefundProviderOutcome, AdapterError>;

    async fn query_refund(
        &self,
        contract: &RefundProviderContract,
    ) -> Result<RefundProviderOutcome, AdapterError>;
}

#[derive(Clone)]
pub struct ReqwestRefundProvider {
    client: reqwest::Client,
}

impl ReqwestRefundProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl RefundProvider for ReqwestRefundProvider {
    async fn create_refund(
        &self,
        contract: &RefundProviderContract,
    ) -> Result<RefundProviderOutcome, AdapterError> {
        match &contract.credential {
            RefundCredential::Stripe(credential) => {
                stripe::create_refund(&self.client, credential, &contract.request)
                    .await
                    .map(|result| RefundProviderOutcome {
                        state: result.state,
                        provider_refund_id: result.provider_refund_id,
                        not_found_is_definitive: result.not_found_is_definitive,
                    })
            }
            RefundCredential::Alipay(credential) => {
                alipay::create_refund(&self.client, credential, &contract.request)
                    .await
                    .map(|result| RefundProviderOutcome {
                        state: result.state,
                        provider_refund_id: result.provider_refund_id,
                        not_found_is_definitive: result.not_found_is_definitive,
                    })
            }
            RefundCredential::Wechat {
                credential,
                verifiers,
            } => wechat::create_refund(&self.client, credential, verifiers, &contract.request)
                .await
                .map(|result| RefundProviderOutcome {
                    state: result.state,
                    provider_refund_id: result.provider_refund_id,
                    not_found_is_definitive: result.not_found_is_definitive,
                }),
        }
    }

    async fn query_refund(
        &self,
        contract: &RefundProviderContract,
    ) -> Result<RefundProviderOutcome, AdapterError> {
        match &contract.credential {
            RefundCredential::Stripe(credential) => stripe::query_refund(
                &self.client,
                credential,
                &contract.request,
                contract.provider_refund_id.as_deref(),
            )
            .await
            .map(|result| RefundProviderOutcome {
                state: result.state,
                provider_refund_id: result.provider_refund_id,
                not_found_is_definitive: result.not_found_is_definitive,
            }),
            RefundCredential::Alipay(credential) => {
                alipay::query_refund(&self.client, credential, &contract.request)
                    .await
                    .map(|result| RefundProviderOutcome {
                        state: result.state,
                        provider_refund_id: result.provider_refund_id,
                        not_found_is_definitive: result.not_found_is_definitive,
                    })
            }
            RefundCredential::Wechat {
                credential,
                verifiers,
            } => wechat::query_refund(&self.client, credential, verifiers, &contract.request)
                .await
                .map(|result| RefundProviderOutcome {
                    state: result.state,
                    provider_refund_id: result.provider_refund_id,
                    not_found_is_definitive: result.not_found_is_definitive,
                }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RefundOperationsError {
    #[error("refund operation input is invalid")]
    InvalidInput,
    #[error("refund was not found")]
    NotFound,
    #[error("order is not refundable")]
    OrderNotRefundable,
    #[error("refund idempotency key conflicts")]
    IdempotencyConflict,
    #[error("refund reserve is unavailable")]
    InsufficientBalance,
    #[error("historical payment configuration is unavailable")]
    ConfigurationUnavailable,
    #[error("refund operation storage failed: {0}")]
    Storage(String),
}

#[derive(Clone)]
pub struct RefundOperations {
    db: DbPool,
    key_ring: Arc<PaymentKeyRing>,
    provider: Arc<dyn RefundProvider>,
}

impl RefundOperations {
    pub fn new(
        db: DbPool,
        key_ring: Arc<PaymentKeyRing>,
        provider: Arc<dyn RefundProvider>,
    ) -> Self {
        Self {
            db,
            key_ring,
            provider,
        }
    }

    pub async fn begin(
        &self,
        order_id: &str,
        admin_id: &str,
        idempotency_key: &str,
    ) -> Result<RefundRecord, RefundOperationsError> {
        validate_operation_input(order_id, admin_id, idempotency_key)?;
        let recovery = RecoveryStore::new(self.db.clone());
        let outcome = recovery
            .begin_refund_with_outcome(BeginRefundInput {
                order_id: order_id.to_string(),
                requested_by_admin_id: admin_id.to_string(),
                idempotency_key: idempotency_key.to_string(),
            })
            .await
            .map_err(map_begin_error)?;
        if matches!(outcome.refund.state.as_str(), "succeeded" | "failed") {
            return Ok(outcome.refund);
        }
        if !outcome.created
            && outcome.refund.state == "created"
            && refund_create_is_recent(&self.db, &outcome.refund.id).await?
        {
            return Ok(outcome.refund);
        }
        let contract = load_contract(&self.db, &self.key_ring, &outcome.refund).await?;
        let provider_outcome = if outcome.created {
            self.provider.create_refund(&contract).await
        } else {
            self.provider.query_refund(&contract).await
        };
        let operation = if outcome.created {
            ProviderCallKind::Create
        } else {
            ProviderCallKind::Query
        };
        project_provider_result(&recovery, &outcome.refund, operation, provider_outcome).await
    }

    pub async fn query(
        &self,
        order_id: &str,
        refund_id: &str,
    ) -> Result<RefundRecord, RefundOperationsError> {
        let outcome = self
            .query_provider_with_context(order_id, refund_id)
            .await?;
        project_query_projection(
            &RecoveryStore::new(self.db.clone()),
            &outcome.refund,
            outcome.projection,
        )
        .await
    }

    pub(crate) async fn query_provider_with_context(
        &self,
        order_id: &str,
        refund_id: &str,
    ) -> Result<RefundQueryOutcome, RefundOperationsError> {
        if !valid_identifier(order_id) || !valid_identifier(refund_id) {
            return Err(RefundOperationsError::InvalidInput);
        }
        let refund = RecoveryStore::new(self.db.clone())
            .get_refund(refund_id)
            .await
            .map_err(map_projection_error)?
            .filter(|refund| refund.order_id == order_id)
            .ok_or(RefundOperationsError::NotFound)?;
        if matches!(refund.state.as_str(), "succeeded" | "failed") {
            return Ok(RefundQueryOutcome {
                refund,
                projection: RefundQueryProjection::AlreadyTerminal,
            });
        }
        let contract = load_contract(&self.db, &self.key_ring, &refund).await?;
        let provider_outcome = self.provider.query_refund(&contract).await;
        Ok(RefundQueryOutcome {
            refund,
            projection: query_projection(provider_outcome)?,
        })
    }
}

async fn refund_create_is_recent(
    db: &DbPool,
    refund_id: &str,
) -> Result<bool, RefundOperationsError> {
    let row = db
        .read()
        .query_one(db.stmt(
            "SELECT created_at FROM store_refunds WHERE id = $1",
            vec![refund_id.into()],
        ))
        .await
        .map_err(storage)?
        .ok_or(RefundOperationsError::NotFound)?;
    let created_at = DateTime::parse_from_rfc3339(&row_string(&row, "created_at")?)
        .map_err(|error| RefundOperationsError::Storage(error.to_string()))?
        .with_timezone(&Utc);
    Ok(Utc::now().signed_duration_since(created_at) < Duration::seconds(300))
}

struct StoredCredential {
    channel_id: String,
    adapter_kind: String,
    account_identity_digest: String,
    encrypted_secret: EncryptedSecret,
}

struct LoadedRefundContract {
    attempt_id: String,
    attempt_order_id: String,
    attempt_state: String,
    channel_id: String,
    adapter_kind: String,
    credential_version_id: String,
    merchant_account_identity: String,
    provider_transaction_id: String,
    merchant_order_number: String,
    amount_minor: String,
    currency: Currency,
    attempt_contract_version: i32,
    order_contract_version: i32,
    credential: StoredCredential,
}

async fn load_contract(
    db: &DbPool,
    key_ring: &PaymentKeyRing,
    refund: &RefundRecord,
) -> Result<RefundProviderContract, RefundOperationsError> {
    let loaded = load_refund_contract(db, refund).await?;
    if loaded.attempt_id != refund.attempt_id
        || loaded.attempt_order_id != refund.order_id
        || loaded.attempt_state != "paid"
        || loaded.attempt_contract_version != loaded.order_contract_version
        || !matches!(loaded.attempt_contract_version, 1 | 2)
        || loaded.amount_minor != refund.amount_minor
        || currency_code(loaded.currency) != refund.currency
        || loaded.credential.channel_id != loaded.channel_id
        || loaded.credential.adapter_kind != loaded.adapter_kind
        || loaded.credential.account_identity_digest != loaded.merchant_account_identity
    {
        return Err(RefundOperationsError::ConfigurationUnavailable);
    }
    validate_amount(&loaded.amount_minor)?;
    let aad = format!(
        "store_channel_credentials:{}:secret",
        loaded.credential_version_id
    );
    let plaintext = key_ring
        .decrypt(&aad, &loaded.credential.encrypted_secret)
        .map_err(|_| RefundOperationsError::ConfigurationUnavailable)?;
    let credential = match loaded.adapter_kind.as_str() {
        "stripe" => {
            let credential = StripeCredential::from_json(&plaintext)
                .map_err(|_| RefundOperationsError::ConfigurationUnavailable)?;
            validate_sha256_identity(credential.account_id(), &loaded.merchant_account_identity)?;
            RefundCredential::Stripe((*credential).clone())
        }
        "alipay" => {
            let credential = AlipayCredential::from_json(&plaintext)
                .map_err(|_| RefundOperationsError::ConfigurationUnavailable)?;
            validate_sha256_identity(credential.seller_id(), &loaded.merchant_account_identity)?;
            RefundCredential::Alipay((*credential).clone())
        }
        "wechat" => {
            let credential = WechatCredential::from_json(&plaintext)
                .map_err(|_| RefundOperationsError::ConfigurationUnavailable)?;
            if credential.account_identity_digest() != loaded.merchant_account_identity {
                return Err(RefundOperationsError::ConfigurationUnavailable);
            }
            let verifiers = load_wechat_platform_verifiers(
                db,
                key_ring,
                &loaded.channel_id,
                &loaded.merchant_account_identity,
            )
            .await?;
            RefundCredential::Wechat {
                credential: (*credential).clone(),
                verifiers,
            }
        }
        _ => return Err(RefundOperationsError::ConfigurationUnavailable),
    };
    Ok(RefundProviderContract {
        channel_id: loaded.channel_id,
        credential_version_id: loaded.credential_version_id,
        merchant_account_identity: loaded.merchant_account_identity,
        provider_refund_id: refund.provider_refund_id.clone(),
        credential,
        request: RefundRequest {
            provider_transaction_id: loaded.provider_transaction_id,
            merchant_order_number: loaded.merchant_order_number,
            amount_minor: loaded.amount_minor,
            currency: loaded.currency,
            idempotency_key: refund.id.clone(),
        },
    })
}

async fn load_refund_contract(
    db: &DbPool,
    refund: &RefundRecord,
) -> Result<LoadedRefundContract, RefundOperationsError> {
    let row = db
        .read()
        .query_one(db.stmt(
            "SELECT a.id AS attempt_id, a.order_id AS attempt_order_id,
                    a.state AS attempt_state, a.channel_id AS attempt_channel_id,
                    a.adapter_kind AS attempt_adapter_kind, a.credential_version_id,
                    a.merchant_account_identity, a.provider_transaction_id,
                    a.payment_contract_version, o.order_number, o.payment_minor,
                    o.payment_currency, o.contract_version,
                    c.id AS stored_credential_id, c.channel_id AS credential_channel_id,
                    c.adapter_kind AS credential_adapter_kind, c.format_version,
                    c.key_id, c.nonce_base64, c.ciphertext_base64,
                    c.account_identity_digest
             FROM store_payment_attempts a
             JOIN store_orders o ON o.id = a.order_id
             LEFT JOIN store_channel_credentials c ON c.id = a.credential_version_id
             WHERE a.id = $1",
            vec![refund.attempt_id.clone().into()],
        ))
        .await
        .map_err(storage)?
        .ok_or(RefundOperationsError::ConfigurationUnavailable)?;
    let credential_version_id = row_string(&row, "credential_version_id")?;
    if row_optional_string(&row, "stored_credential_id")?.as_deref()
        != Some(credential_version_id.as_str())
    {
        return Err(RefundOperationsError::ConfigurationUnavailable);
    }
    let version = row
        .try_get::<Option<i32>>("", "format_version")
        .map_err(storage)?
        .and_then(|value| u8::try_from(value).ok())
        .ok_or(RefundOperationsError::ConfigurationUnavailable)?;
    let currency = parse_currency(&row_string(&row, "payment_currency")?)?;
    let provider_transaction_id = row_optional_string(&row, "provider_transaction_id")?
        .filter(|value| valid_provider_value(value))
        .ok_or(RefundOperationsError::ConfigurationUnavailable)?;
    Ok(LoadedRefundContract {
        attempt_id: row_string(&row, "attempt_id")?,
        attempt_order_id: row_string(&row, "attempt_order_id")?,
        attempt_state: row_string(&row, "attempt_state")?,
        channel_id: row_string(&row, "attempt_channel_id")?,
        adapter_kind: row_string(&row, "attempt_adapter_kind")?,
        credential_version_id,
        merchant_account_identity: row_string(&row, "merchant_account_identity")?,
        provider_transaction_id,
        merchant_order_number: row_string(&row, "order_number")?,
        amount_minor: row_string(&row, "payment_minor")?,
        currency,
        attempt_contract_version: row_i32(&row, "payment_contract_version")?,
        order_contract_version: row_i32(&row, "contract_version")?,
        credential: StoredCredential {
            channel_id: required_string(&row, "credential_channel_id")?,
            adapter_kind: required_string(&row, "credential_adapter_kind")?,
            account_identity_digest: required_string(&row, "account_identity_digest")?,
            encrypted_secret: EncryptedSecret {
                version,
                key_id: required_string(&row, "key_id")?,
                nonce_base64: required_string(&row, "nonce_base64")?,
                ciphertext_base64: required_string(&row, "ciphertext_base64")?,
            },
        },
    })
}

async fn load_wechat_platform_verifiers(
    db: &DbPool,
    key_ring: &PaymentKeyRing,
    channel_id: &str,
    account_identity_digest: &str,
) -> Result<Vec<WechatPlatformVerifier>, RefundOperationsError> {
    let rows = db
        .read()
        .query_all(db.stmt(
            "SELECT id, format_version, key_id, nonce_base64, ciphertext_base64
             FROM store_channel_credentials
             WHERE channel_id = $1 AND adapter_kind = 'wechat'
               AND account_identity_digest = $2
             ORDER BY created_at DESC, id DESC",
            vec![channel_id.into(), account_identity_digest.into()],
        ))
        .await
        .map_err(storage)?;
    let mut verifiers = Vec::new();
    for row in rows {
        let Ok(version) = row.try_get::<i32>("", "format_version") else {
            continue;
        };
        let Ok(version) = u8::try_from(version) else {
            continue;
        };
        let Ok(credential_id) = row.try_get::<String>("", "id") else {
            continue;
        };
        let Ok(key_id) = row.try_get::<String>("", "key_id") else {
            continue;
        };
        let Ok(nonce_base64) = row.try_get::<String>("", "nonce_base64") else {
            continue;
        };
        let Ok(ciphertext_base64) = row.try_get::<String>("", "ciphertext_base64") else {
            continue;
        };
        let encrypted = EncryptedSecret {
            version,
            key_id,
            nonce_base64,
            ciphertext_base64,
        };
        let aad = format!("store_channel_credentials:{credential_id}:secret");
        let Ok(plaintext) = key_ring.decrypt(&aad, &encrypted) else {
            continue;
        };
        let Ok(credential) = WechatCredential::from_json(&plaintext) else {
            continue;
        };
        if credential.account_identity_digest() != account_identity_digest {
            continue;
        }
        let Ok(verifier) = credential.platform_verifier() else {
            continue;
        };
        if !verifiers.iter().any(|stored: &WechatPlatformVerifier| {
            stored.certificate_serial() == verifier.certificate_serial()
        }) {
            verifiers.push(verifier);
        }
    }
    if verifiers.is_empty() {
        return Err(RefundOperationsError::ConfigurationUnavailable);
    }
    Ok(verifiers)
}

async fn project_provider_result(
    recovery: &RecoveryStore,
    refund: &RefundRecord,
    operation: ProviderCallKind,
    provider_result: Result<RefundProviderOutcome, AdapterError>,
) -> Result<RefundRecord, RefundOperationsError> {
    let outcome = match provider_result {
        Ok(outcome) if valid_optional_provider_id(outcome.provider_refund_id.as_deref()) => outcome,
        Ok(_) | Err(AdapterError::Ambiguous | AdapterError::Verification) => {
            return mark_pending(recovery, refund, None).await;
        }
        Err(AdapterError::Rejected) if operation == ProviderCallKind::Create => {
            recovery
                .reject_refund(&refund.id)
                .await
                .map_err(map_projection_error)?;
            return reload_refund(recovery, &refund.id).await;
        }
        Err(AdapterError::Rejected) => return mark_pending(recovery, refund, None).await,
        Err(
            AdapterError::InvalidConfiguration
            | AdapterError::InvalidRequest
            | AdapterError::Unsupported,
        ) => return Err(RefundOperationsError::ConfigurationUnavailable),
    };
    match outcome.state {
        ProviderRefundState::Pending | ProviderRefundState::Ambiguous => {
            mark_pending(recovery, refund, outcome.provider_refund_id.as_deref()).await
        }
        ProviderRefundState::Succeeded => {
            if outcome.provider_refund_id.is_some() {
                mark_pending(recovery, refund, outcome.provider_refund_id.as_deref()).await?;
            }
            recovery
                .complete_refund(&refund.id)
                .await
                .map_err(map_projection_error)?;
            reload_refund(recovery, &refund.id).await
        }
        ProviderRefundState::Failed => {
            if outcome.provider_refund_id.is_some() {
                mark_pending(recovery, refund, outcome.provider_refund_id.as_deref()).await?;
            }
            recovery
                .reject_refund(&refund.id)
                .await
                .map_err(map_projection_error)?;
            reload_refund(recovery, &refund.id).await
        }
        ProviderRefundState::NotFound if outcome.not_found_is_definitive => {
            recovery
                .reject_refund(&refund.id)
                .await
                .map_err(map_projection_error)?;
            reload_refund(recovery, &refund.id).await
        }
        ProviderRefundState::NotFound => mark_pending(recovery, refund, None).await,
    }
}

fn query_projection(
    provider_result: Result<RefundProviderOutcome, AdapterError>,
) -> Result<RefundQueryProjection, RefundOperationsError> {
    let outcome = match provider_result {
        Ok(outcome) if valid_optional_provider_id(outcome.provider_refund_id.as_deref()) => outcome,
        Ok(_)
        | Err(AdapterError::Ambiguous | AdapterError::Verification | AdapterError::Rejected) => {
            return Ok(RefundQueryProjection::Pending {
                provider_refund_id: None,
            });
        }
        Err(
            AdapterError::InvalidConfiguration
            | AdapterError::InvalidRequest
            | AdapterError::Unsupported,
        ) => return Err(RefundOperationsError::ConfigurationUnavailable),
    };
    let provider_refund_id = outcome.provider_refund_id;
    Ok(match outcome.state {
        ProviderRefundState::Pending | ProviderRefundState::Ambiguous => {
            RefundQueryProjection::Pending { provider_refund_id }
        }
        ProviderRefundState::Succeeded => RefundQueryProjection::Succeeded { provider_refund_id },
        ProviderRefundState::Failed => RefundQueryProjection::Failed { provider_refund_id },
        ProviderRefundState::NotFound if outcome.not_found_is_definitive => {
            RefundQueryProjection::Failed { provider_refund_id }
        }
        ProviderRefundState::NotFound => RefundQueryProjection::Pending {
            provider_refund_id: None,
        },
    })
}

async fn project_query_projection(
    recovery: &RecoveryStore,
    refund: &RefundRecord,
    projection: RefundQueryProjection,
) -> Result<RefundRecord, RefundOperationsError> {
    match projection {
        RefundQueryProjection::AlreadyTerminal => Ok(refund.clone()),
        RefundQueryProjection::Pending { provider_refund_id } => {
            mark_pending(recovery, refund, provider_refund_id.as_deref()).await
        }
        RefundQueryProjection::Succeeded { provider_refund_id } => {
            if provider_refund_id.is_some() {
                mark_pending(recovery, refund, provider_refund_id.as_deref()).await?;
            }
            recovery
                .complete_refund(&refund.id)
                .await
                .map_err(map_projection_error)?;
            reload_refund(recovery, &refund.id).await
        }
        RefundQueryProjection::Failed { provider_refund_id } => {
            if provider_refund_id.is_some() {
                mark_pending(recovery, refund, provider_refund_id.as_deref()).await?;
            }
            recovery
                .reject_refund(&refund.id)
                .await
                .map_err(map_projection_error)?;
            reload_refund(recovery, &refund.id).await
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProviderCallKind {
    Create,
    Query,
}

async fn mark_pending(
    recovery: &RecoveryStore,
    refund: &RefundRecord,
    provider_refund_id: Option<&str>,
) -> Result<RefundRecord, RefundOperationsError> {
    recovery
        .mark_refund_pending_outcome(&refund.id, provider_refund_id)
        .await
        .map_err(map_projection_error)
}

async fn reload_refund(
    recovery: &RecoveryStore,
    refund_id: &str,
) -> Result<RefundRecord, RefundOperationsError> {
    recovery
        .get_refund(refund_id)
        .await
        .map_err(map_projection_error)?
        .ok_or(RefundOperationsError::NotFound)
}

fn validate_operation_input(
    order_id: &str,
    admin_id: &str,
    idempotency_key: &str,
) -> Result<(), RefundOperationsError> {
    if !valid_identifier(order_id)
        || !valid_identifier(admin_id)
        || idempotency_key.is_empty()
        || idempotency_key.len() > 255
        || idempotency_key.trim() != idempotency_key
    {
        return Err(RefundOperationsError::InvalidInput);
    }
    Ok(())
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= 255 && value.trim() == value
}

fn validate_amount(value: &str) -> Result<(), RefundOperationsError> {
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || value
            .parse::<u64>()
            .ok()
            .filter(|amount| *amount > 0)
            .is_none()
    {
        return Err(RefundOperationsError::ConfigurationUnavailable);
    }
    Ok(())
}

fn valid_provider_value(value: &str) -> bool {
    !value.is_empty() && value.len() <= 255 && value.trim() == value
}

fn valid_optional_provider_id(value: Option<&str>) -> bool {
    value.is_none_or(valid_provider_value)
}

fn validate_sha256_identity(account_id: &str, expected: &str) -> Result<(), RefundOperationsError> {
    let digest = Sha256::digest(account_id.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if digest != expected {
        return Err(RefundOperationsError::ConfigurationUnavailable);
    }
    Ok(())
}

fn parse_currency(value: &str) -> Result<Currency, RefundOperationsError> {
    match value {
        "CNY" => Ok(Currency::CNY),
        "USD" => Ok(Currency::USD),
        _ => Err(RefundOperationsError::ConfigurationUnavailable),
    }
}

fn currency_code(currency: Currency) -> &'static str {
    match currency {
        Currency::CNY => "CNY",
        Currency::USD => "USD",
    }
}

fn map_begin_error(error: RecoveryError) -> RefundOperationsError {
    match error {
        RecoveryError::InvalidInput => RefundOperationsError::InvalidInput,
        RecoveryError::NotFound | RecoveryError::OrderNotRecoverable => {
            RefundOperationsError::OrderNotRefundable
        }
        RecoveryError::InsufficientBalance => RefundOperationsError::InsufficientBalance,
        RecoveryError::Conflict => RefundOperationsError::IdempotencyConflict,
        RecoveryError::Storage(message) => RefundOperationsError::Storage(message),
    }
}

fn map_projection_error(error: RecoveryError) -> RefundOperationsError {
    match error {
        RecoveryError::InvalidInput => RefundOperationsError::InvalidInput,
        RecoveryError::NotFound => RefundOperationsError::NotFound,
        RecoveryError::OrderNotRecoverable => RefundOperationsError::OrderNotRefundable,
        RecoveryError::InsufficientBalance => RefundOperationsError::InsufficientBalance,
        RecoveryError::Conflict => RefundOperationsError::Storage(
            "refund state transition conflicted with persisted state".to_string(),
        ),
        RecoveryError::Storage(message) => RefundOperationsError::Storage(message),
    }
}

fn required_string(row: &QueryResult, column: &str) -> Result<String, RefundOperationsError> {
    row_optional_string(row, column)?.ok_or(RefundOperationsError::ConfigurationUnavailable)
}

fn row_string(row: &QueryResult, column: &str) -> Result<String, RefundOperationsError> {
    row.try_get("", column).map_err(storage)
}

fn row_optional_string(
    row: &QueryResult,
    column: &str,
) -> Result<Option<String>, RefundOperationsError> {
    row.try_get("", column).map_err(storage)
}

fn row_i32(row: &QueryResult, column: &str) -> Result<i32, RefundOperationsError> {
    row.try_get("", column).map_err(storage)
}

fn storage(error: sea_orm::DbErr) -> RefundOperationsError {
    RefundOperationsError::Storage(error.to_string())
}
