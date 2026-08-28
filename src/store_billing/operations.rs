use std::sync::Arc;

use async_trait::async_trait;
use sea_orm::{ConnectionTrait, QueryResult};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::adapters::alipay::{self, AlipayCredential};
use super::adapters::stripe::{self, StripeCredential};
use super::adapters::wechat::{self, WechatCredential, WechatPlatformVerifier};
use super::callbacks::{
    ApplyProviderEventInput, CallbackApplyResult, CallbackStoreError, PaymentCallbackStore,
};
use super::crypto::{EncryptedSecret, PaymentKeyRing};
use super::money::Currency;
use super::order::{PaymentAttempt, PaymentOrder, PaymentOrderError, PaymentOrderStore};
use super::payment::{AdapterError, PaymentQuery, ProviderPaymentState, validate_payment_query};
use super::recovery::{RecoveryError, RecoveryStore, RefundRecord};
use super::state_machine::PaymentState;
use crate::db::DbPool;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PaymentOperationsError {
    #[error("payment attempt does not exist")]
    AttemptNotFound,
    #[error("historical payment credential does not exist")]
    CredentialNotFound,
    #[error("historical payment credential does not match the immutable attempt")]
    CredentialBindingMismatch,
    #[error("historical payment credential cannot be decrypted")]
    CredentialDecryptionFailed,
    #[error("historical payment credential is invalid")]
    CredentialInvalid,
    #[error("payment credential account identity does not match the immutable attempt")]
    AccountIdentityMismatch,
    #[error("payment query contract is invalid")]
    PaymentContractInvalid,
    #[error("payment adapter is unsupported")]
    UnsupportedAdapter,
    #[error(transparent)]
    Provider(#[from] AdapterError),
    #[error("Store payment operations storage failed: {0}")]
    Storage(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentQueryOutcome {
    pub attempt_id: String,
    pub attempt_state: String,
    pub order_id: String,
    pub channel_id: String,
    pub credential_version_id: String,
    pub provider_object_id: String,
    pub merchant_account_identity: String,
    pub order_number: String,
    pub amount_minor: String,
    pub currency: Currency,
    pub payment_hold: bool,
    pub state: ProviderPaymentState,
}

#[async_trait]
pub trait PaymentQueryProvider: Send + Sync {
    async fn query_stripe_payment(
        &self,
        credential: &StripeCredential,
        query: &PaymentQuery,
    ) -> Result<ProviderPaymentState, AdapterError>;

    async fn query_alipay_payment(
        &self,
        credential: &AlipayCredential,
        query: &PaymentQuery,
    ) -> Result<ProviderPaymentState, AdapterError>;

    async fn query_wechat_payment(
        &self,
        credential: &WechatCredential,
        verifiers: &[WechatPlatformVerifier],
        query: &PaymentQuery,
    ) -> Result<ProviderPaymentState, AdapterError>;
}

#[derive(Clone)]
pub struct ReqwestPaymentQueryProvider {
    client: reqwest::Client,
}

impl ReqwestPaymentQueryProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl PaymentQueryProvider for ReqwestPaymentQueryProvider {
    async fn query_stripe_payment(
        &self,
        credential: &StripeCredential,
        query: &PaymentQuery,
    ) -> Result<ProviderPaymentState, AdapterError> {
        stripe::query_payment(&self.client, credential, query).await
    }

    async fn query_alipay_payment(
        &self,
        credential: &AlipayCredential,
        query: &PaymentQuery,
    ) -> Result<ProviderPaymentState, AdapterError> {
        alipay::query_payment(&self.client, credential, query).await
    }

    async fn query_wechat_payment(
        &self,
        credential: &WechatCredential,
        verifiers: &[WechatPlatformVerifier],
        query: &PaymentQuery,
    ) -> Result<ProviderPaymentState, AdapterError> {
        wechat::query_payment_with_verifiers(&self.client, credential, verifiers, query).await
    }
}

#[derive(Clone)]
pub struct PaymentQueryOperations {
    db: DbPool,
    key_ring: Arc<PaymentKeyRing>,
    provider: Arc<dyn PaymentQueryProvider>,
}

impl PaymentQueryOperations {
    pub fn new(
        db: DbPool,
        key_ring: Arc<PaymentKeyRing>,
        provider: Arc<dyn PaymentQueryProvider>,
    ) -> Self {
        Self {
            db,
            key_ring,
            provider,
        }
    }

    pub async fn query_attempt(
        &self,
        attempt_id: &str,
    ) -> Result<ProviderPaymentState, PaymentOperationsError> {
        Ok(self.query_attempt_with_context(attempt_id).await?.state)
    }

    pub async fn query_attempt_with_context(
        &self,
        attempt_id: &str,
    ) -> Result<PaymentQueryOutcome, PaymentOperationsError> {
        let loaded = load_attempt(&self.db, attempt_id).await?;
        if loaded.credential.channel_id != loaded.channel_id
            || loaded.credential.adapter_kind != loaded.adapter_kind
            || loaded.credential.account_identity_digest != loaded.merchant_account_identity
        {
            return Err(PaymentOperationsError::CredentialBindingMismatch);
        }
        let aad = format!(
            "store_channel_credentials:{}:secret",
            loaded.credential_version_id
        );
        let plaintext = self
            .key_ring
            .decrypt(&aad, &loaded.credential.encrypted_secret)
            .map_err(|_| PaymentOperationsError::CredentialDecryptionFailed)?;
        let query = PaymentQuery {
            provider_object_id: loaded.provider_object_id.clone(),
            merchant_order_number: loaded.order_number.clone(),
            amount_minor: loaded.amount_minor.clone(),
            currency: loaded.currency,
        };
        validate_payment_query(&query)
            .map_err(|_| PaymentOperationsError::PaymentContractInvalid)?;

        let state = match loaded.adapter_kind.as_str() {
            "stripe" => {
                let credential = StripeCredential::from_json(&plaintext)
                    .map_err(|_| PaymentOperationsError::CredentialInvalid)?;
                validate_account_identity(credential.account_id(), &loaded)?;
                self.provider
                    .query_stripe_payment(&credential, &query)
                    .await?
            }
            "alipay" => {
                let credential = AlipayCredential::from_json(&plaintext)
                    .map_err(|_| PaymentOperationsError::CredentialInvalid)?;
                validate_account_identity(credential.seller_id(), &loaded)?;
                self.provider
                    .query_alipay_payment(&credential, &query)
                    .await?
            }
            "wechat" => {
                let credential = WechatCredential::from_json(&plaintext)
                    .map_err(|_| PaymentOperationsError::CredentialInvalid)?;
                validate_account_identity_digest(&credential.account_identity_digest(), &loaded)?;
                let verifiers = load_wechat_platform_verifiers(
                    &self.db,
                    &self.key_ring,
                    &loaded.channel_id,
                    &loaded.merchant_account_identity,
                )
                .await?;
                self.provider
                    .query_wechat_payment(&credential, &verifiers, &query)
                    .await?
            }
            _ => return Err(PaymentOperationsError::UnsupportedAdapter),
        };

        Ok(PaymentQueryOutcome {
            attempt_id: loaded.attempt_id,
            attempt_state: loaded.attempt_state,
            order_id: loaded.order_id,
            channel_id: loaded.channel_id,
            credential_version_id: loaded.credential_version_id,
            provider_object_id: loaded.provider_object_id,
            merchant_account_identity: loaded.merchant_account_identity,
            order_number: loaded.order_number,
            amount_minor: loaded.amount_minor,
            currency: loaded.currency,
            payment_hold: loaded.payment_hold,
            state,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AdminOrderDetail {
    pub order: PaymentOrder,
    pub attempts: Vec<PaymentAttempt>,
    pub refunds: Vec<RefundRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AdminProviderPaymentState {
    pub kind: String,
    pub provider_transaction_id: Option<String>,
}

impl From<&ProviderPaymentState> for AdminProviderPaymentState {
    fn from(state: &ProviderPaymentState) -> Self {
        match state {
            ProviderPaymentState::NotFound => Self {
                kind: "not_found".to_string(),
                provider_transaction_id: None,
            },
            ProviderPaymentState::Unpaid => Self {
                kind: "unpaid".to_string(),
                provider_transaction_id: None,
            },
            ProviderPaymentState::Paid {
                provider_transaction_id,
            } => Self {
                kind: "paid".to_string(),
                provider_transaction_id: Some(provider_transaction_id.clone()),
            },
            ProviderPaymentState::Closed => Self {
                kind: "closed".to_string(),
                provider_transaction_id: None,
            },
            ProviderPaymentState::Ambiguous => Self {
                kind: "ambiguous".to_string(),
                provider_transaction_id: None,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AdminOrderOperationResult {
    pub order: PaymentOrder,
    pub attempt: PaymentAttempt,
    pub provider_state: AdminProviderPaymentState,
    pub projection: Option<String>,
    pub closed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AdminOrderOperationError {
    #[error("Admin order operation input is invalid")]
    InvalidInput,
    #[error("Admin order or attempt was not found")]
    NotFound,
    #[error("legacy closed orders reject new operations")]
    LegacyClosed,
    #[error("order cannot be closed")]
    OrderNotPayable,
    #[error("Provider payment state is ambiguous")]
    Ambiguous,
    #[error("historical payment configuration is unavailable")]
    ConfigurationUnavailable,
    #[error("Provider payment query failed")]
    ProviderQueryFailed,
    #[error("verified Provider payment projection failed")]
    ProjectionFailed,
    #[error("Admin order operation storage failed: {0}")]
    Storage(String),
}

#[derive(Clone)]
pub struct AdminOrderOperations {
    db: DbPool,
    query: PaymentQueryOperations,
}

impl AdminOrderOperations {
    pub fn new(
        db: DbPool,
        key_ring: Arc<PaymentKeyRing>,
        provider: Arc<dyn PaymentQueryProvider>,
    ) -> Self {
        Self {
            query: PaymentQueryOperations::new(db.clone(), key_ring, provider),
            db,
        }
    }

    pub async fn detail(&self, order_id: &str) -> Result<AdminOrderDetail, AdminOrderOperationError> {
        Self::detail_from_db(&self.db, order_id).await
    }

    pub async fn detail_from_db(
        db: &DbPool,
        order_id: &str,
    ) -> Result<AdminOrderDetail, AdminOrderOperationError> {
        if !valid_identifier(order_id) {
            return Err(AdminOrderOperationError::InvalidInput);
        }
        let orders = PaymentOrderStore::new(db.clone());
        let order = orders
            .get_order_admin(order_id)
            .await
            .map_err(map_order_error)?
            .ok_or(AdminOrderOperationError::NotFound)?;
        let attempts = orders
            .list_attempts_admin(order_id)
            .await
            .map_err(map_order_error)?;
        let refunds = RecoveryStore::new(db.clone())
            .list_refunds_for_order(order_id)
            .await
            .map_err(map_recovery_error)?;
        Ok(AdminOrderDetail {
            order,
            attempts,
            refunds,
        })
    }

    pub async fn query(
        &self,
        order_id: &str,
        attempt_id: &str,
    ) -> Result<AdminOrderOperationResult, AdminOrderOperationError> {
        self.execute(order_id, attempt_id, false).await
    }

    pub async fn close(
        &self,
        order_id: &str,
        attempt_id: &str,
    ) -> Result<AdminOrderOperationResult, AdminOrderOperationError> {
        self.execute(order_id, attempt_id, true).await
    }

    async fn execute(
        &self,
        order_id: &str,
        attempt_id: &str,
        close: bool,
    ) -> Result<AdminOrderOperationResult, AdminOrderOperationError> {
        if !valid_identifier(order_id) || !valid_identifier(attempt_id) {
            return Err(AdminOrderOperationError::InvalidInput);
        }
        let orders = PaymentOrderStore::new(self.db.clone());
        let order = orders
            .get_order_admin(order_id)
            .await
            .map_err(map_order_error)?
            .ok_or(AdminOrderOperationError::NotFound)?;
        let attempt = orders
            .get_attempt_admin(attempt_id)
            .await
            .map_err(map_order_error)?
            .filter(|attempt| attempt.order_id == order_id)
            .ok_or(AdminOrderOperationError::NotFound)?;
        if close && (order.contract_version != 2 || order.payment_state != PaymentState::Unpaid) {
            return Err(AdminOrderOperationError::OrderNotPayable);
        }
        if order.contract_version == 1 && order.payment_state == PaymentState::Closed {
            return Err(AdminOrderOperationError::LegacyClosed);
        }
        let outcome = self
            .query
            .query_attempt_with_context(attempt_id)
            .await
            .map_err(map_query_error)?;
        if outcome.order_id != order_id {
            return Err(AdminOrderOperationError::NotFound);
        }
        let provider_state = outcome.state.clone();
        match &provider_state {
            ProviderPaymentState::Ambiguous => return Err(AdminOrderOperationError::Ambiguous),
            ProviderPaymentState::Paid {
                provider_transaction_id,
            } => {
                let projection = self
                    .project_paid_query(&outcome, provider_transaction_id)
                    .await?;
                let order = orders
                    .get_order_admin(order_id)
                    .await
                    .map_err(map_order_error)?
                    .ok_or(AdminOrderOperationError::NotFound)?;
                let attempt = orders
                    .get_attempt_admin(attempt_id)
                    .await
                    .map_err(map_order_error)?
                    .ok_or(AdminOrderOperationError::NotFound)?;
                return Ok(AdminOrderOperationResult {
                    order,
                    attempt,
                    provider_state: AdminProviderPaymentState::from(&provider_state),
                    projection: Some(projection),
                    closed: false,
                });
            }
            ProviderPaymentState::NotFound
            | ProviderPaymentState::Unpaid
            | ProviderPaymentState::Closed => {}
        }
        if close {
            let (order, attempt) = orders
                .close_confirmed_unpaid_admin(order_id, attempt_id)
                .await
                .map_err(map_order_error)?;
            return Ok(AdminOrderOperationResult {
                order,
                attempt,
                provider_state: AdminProviderPaymentState::from(&provider_state),
                projection: None,
                closed: true,
            });
        }
        Ok(AdminOrderOperationResult {
            order,
            attempt,
            provider_state: AdminProviderPaymentState::from(&provider_state),
            projection: None,
            closed: false,
        })
    }

    async fn project_paid_query(
        &self,
        outcome: &PaymentQueryOutcome,
        provider_transaction_id: &str,
    ) -> Result<String, AdminOrderOperationError> {
        let identity = query_event_identity(outcome, provider_transaction_id);
        let digest = hex_sha256(identity.as_bytes());
        let event = ApplyProviderEventInput {
            event_row_id: deterministic_uuid(&digest).to_string(),
            credential_version_id: outcome.credential_version_id.clone(),
            verification_credential_version_id: outcome.credential_version_id.clone(),
            provider_event_id: format!("payment-query:{digest}"),
            event_kind: "payment_query_succeeded".to_string(),
            order_id: outcome.order_id.clone(),
            attempt_id: outcome.attempt_id.clone(),
            provider_transaction_id: provider_transaction_id.to_string(),
            provider_object_id: outcome.provider_object_id.clone(),
            order_number: outcome.order_number.clone(),
            merchant_account_identity: outcome.merchant_account_identity.clone(),
            amount_minor: outcome.amount_minor.clone(),
            currency: outcome.currency,
            body_digest: digest,
            parsed_json: serde_json::json!({
                "event_kind": "payment_query_succeeded",
                "attempt_id": outcome.attempt_id,
                "provider_object_id": outcome.provider_object_id,
                "provider_transaction_id": provider_transaction_id,
                "order_number": outcome.order_number,
                "amount_minor": outcome.amount_minor,
                "currency": currency_string(outcome.currency),
            }),
            raw_body: None,
            source_ip: None,
            user_agent: Some("monoize-admin-provider-query".to_string()),
            received_at: chrono::Utc::now(),
        };
        let store = PaymentCallbackStore::new(self.db.clone());
        match store
            .apply_verified_query_payment(event)
            .await
            .map_err(map_callback_error)?
        {
            CallbackApplyResult::Applied => Ok("applied".to_string()),
            CallbackApplyResult::Duplicate => {
                store
                    .fulfill_paid_order(&outcome.order_id)
                    .await
                    .map_err(map_callback_error)?;
                Ok("duplicate".to_string())
            }
            CallbackApplyResult::ManualReview => Err(AdminOrderOperationError::ProjectionFailed),
        }
    }
}

fn valid_identifier(value: &str) -> bool {
    let mut count = 0;
    for character in value.chars() {
        if character.is_whitespace() || count == 128 {
            return false;
        }
        count += 1;
    }
    count > 0
}

fn query_event_identity(outcome: &PaymentQueryOutcome, provider_transaction_id: &str) -> String {
    [
        outcome.attempt_id.as_str(),
        outcome.order_id.as_str(),
        outcome.channel_id.as_str(),
        outcome.credential_version_id.as_str(),
        outcome.provider_object_id.as_str(),
        outcome.merchant_account_identity.as_str(),
        outcome.order_number.as_str(),
        outcome.amount_minor.as_str(),
        currency_string(outcome.currency),
        provider_transaction_id,
    ]
    .into_iter()
    .map(|value| format!("{}:{value}", value.len()))
    .collect::<Vec<_>>()
    .join("|")
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn deterministic_uuid(hex_digest: &str) -> Uuid {
    let mut uuid = [0_u8; 16];
    for (index, byte) in uuid.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex_digest[index * 2..index * 2 + 2], 16)
            .expect("SHA-256 hex is valid");
    }
    uuid[6] = (uuid[6] & 0x0f) | 0x50;
    uuid[8] = (uuid[8] & 0x3f) | 0x80;
    Uuid::from_bytes(uuid)
}

fn currency_string(currency: Currency) -> &'static str {
    match currency {
        Currency::CNY => "CNY",
        Currency::USD => "USD",
    }
}

fn map_query_error(error: PaymentOperationsError) -> AdminOrderOperationError {
    match error {
        PaymentOperationsError::AttemptNotFound => AdminOrderOperationError::NotFound,
        PaymentOperationsError::CredentialNotFound
        | PaymentOperationsError::CredentialBindingMismatch
        | PaymentOperationsError::CredentialDecryptionFailed
        | PaymentOperationsError::CredentialInvalid
        | PaymentOperationsError::AccountIdentityMismatch
        | PaymentOperationsError::PaymentContractInvalid
        | PaymentOperationsError::UnsupportedAdapter => {
            AdminOrderOperationError::ConfigurationUnavailable
        }
        PaymentOperationsError::Provider(
            AdapterError::InvalidConfiguration | AdapterError::InvalidRequest,
        ) => {
            AdminOrderOperationError::ConfigurationUnavailable
        }
        PaymentOperationsError::Provider(_) => AdminOrderOperationError::ProviderQueryFailed,
        PaymentOperationsError::Storage(detail) => AdminOrderOperationError::Storage(detail),
    }
}

fn map_order_error(error: PaymentOrderError) -> AdminOrderOperationError {
    match error {
        PaymentOrderError::OrderNotFound => AdminOrderOperationError::NotFound,
        PaymentOrderError::OrderNotPayable => AdminOrderOperationError::OrderNotPayable,
        PaymentOrderError::Storage(detail) => AdminOrderOperationError::Storage(detail),
        _ => AdminOrderOperationError::Storage(error.to_string()),
    }
}

fn map_recovery_error(error: RecoveryError) -> AdminOrderOperationError {
    match error {
        RecoveryError::NotFound => AdminOrderOperationError::NotFound,
        RecoveryError::Storage(detail) => AdminOrderOperationError::Storage(detail),
        _ => AdminOrderOperationError::Storage(error.to_string()),
    }
}

fn map_callback_error(error: CallbackStoreError) -> AdminOrderOperationError {
    match error {
        CallbackStoreError::InvalidInput | CallbackStoreError::NotFound => {
            AdminOrderOperationError::ProjectionFailed
        }
        CallbackStoreError::Storage(detail) | CallbackStoreError::Fulfillment(detail) => {
            AdminOrderOperationError::Storage(detail)
        }
    }
}

async fn load_wechat_platform_verifiers(
    db: &DbPool,
    key_ring: &PaymentKeyRing,
    channel_id: &str,
    account_identity_digest: &str,
) -> Result<Vec<WechatPlatformVerifier>, PaymentOperationsError> {
    let rows = db
        .read()
        .query_all(db.stmt(
            "SELECT id, format_version, key_id, nonce_base64, ciphertext_base64
             FROM store_channel_credentials
             WHERE channel_id = $1 AND adapter_kind = 'wechat'
               AND account_identity_digest = $2
             ORDER BY CASE WHEN status = 'active' THEN 0 ELSE 1 END,
                      created_at DESC, id DESC",
            vec![channel_id.into(), account_identity_digest.into()],
        ))
        .await
        .map_err(storage)?;
    let mut verifiers = Vec::new();
    for row in rows {
        let credential_id = row_string(&row, "id")?;
        let version = row
            .try_get::<i32>("", "format_version")
            .map_err(storage)
            .and_then(|value| {
                u8::try_from(value).map_err(|_| PaymentOperationsError::CredentialInvalid)
            })?;
        let encrypted_secret = EncryptedSecret {
            version,
            key_id: row_string(&row, "key_id")?,
            nonce_base64: row_string(&row, "nonce_base64")?,
            ciphertext_base64: row_string(&row, "ciphertext_base64")?,
        };
        let aad = format!("store_channel_credentials:{credential_id}:secret");
        let Ok(plaintext) = key_ring.decrypt(&aad, &encrypted_secret) else {
            continue;
        };
        let Ok(credential) = WechatCredential::from_json(&plaintext) else {
            continue;
        };
        let digest = credential.account_identity_digest();
        if digest != account_identity_digest {
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
        return Err(PaymentOperationsError::CredentialDecryptionFailed);
    }
    Ok(verifiers)
}

struct LoadedPaymentQuery {
    attempt_id: String,
    attempt_state: String,
    order_id: String,
    channel_id: String,
    adapter_kind: String,
    credential_version_id: String,
    merchant_account_identity: String,
    provider_object_id: String,
    order_number: String,
    amount_minor: String,
    currency: Currency,
    payment_hold: bool,
    credential: StoredCredential,
}

struct StoredCredential {
    channel_id: String,
    adapter_kind: String,
    account_identity_digest: String,
    encrypted_secret: EncryptedSecret,
}

async fn load_attempt(
    db: &DbPool,
    attempt_id: &str,
) -> Result<LoadedPaymentQuery, PaymentOperationsError> {
    let row = db
        .read()
        .query_one(db.stmt(
            "SELECT a.id AS attempt_id, a.state AS attempt_state, a.order_id,
                    a.channel_id AS attempt_channel_id, a.payment_contract_version,
                    a.adapter_kind AS attempt_adapter_kind, a.credential_version_id,
                    a.merchant_account_identity, a.provider_object_id,
                    o.order_number, o.contract_version, o.payment_minor,
                    o.payment_currency, o.payment_hold,
                    c.id AS stored_credential_id, c.channel_id AS credential_channel_id,
                    c.adapter_kind AS credential_adapter_kind, c.format_version,
                    c.key_id, c.nonce_base64, c.ciphertext_base64,
                    c.account_identity_digest
             FROM store_payment_attempts a
             JOIN store_orders o ON o.id = a.order_id
             LEFT JOIN store_channel_credentials c ON c.id = a.credential_version_id
             WHERE a.id = $1",
            vec![attempt_id.into()],
        ))
        .await
        .map_err(storage)?
        .ok_or(PaymentOperationsError::AttemptNotFound)?;
    let credential_id = row_optional_string(&row, "stored_credential_id")?
        .ok_or(PaymentOperationsError::CredentialNotFound)?;
    let credential_version_id = row_string(&row, "credential_version_id")?;
    if credential_id != credential_version_id {
        return Err(PaymentOperationsError::CredentialBindingMismatch);
    }
    let attempt_contract_version = row
        .try_get::<i32>("", "payment_contract_version")
        .map_err(storage)?;
    let order_contract_version = row
        .try_get::<i32>("", "contract_version")
        .map_err(storage)?;
    if attempt_contract_version != order_contract_version
        || !matches!(attempt_contract_version, 1 | 2)
    {
        return Err(PaymentOperationsError::PaymentContractInvalid);
    }
    let version = row
        .try_get::<Option<i32>>("", "format_version")
        .map_err(storage)?
        .ok_or(PaymentOperationsError::CredentialNotFound)
        .and_then(|value| {
            u8::try_from(value).map_err(|_| PaymentOperationsError::CredentialInvalid)
        })?;
    let currency = match row_string(&row, "payment_currency")?.as_str() {
        "CNY" => Currency::CNY,
        "USD" => Currency::USD,
        _ => return Err(PaymentOperationsError::PaymentContractInvalid),
    };
    let adapter_kind = row_string(&row, "attempt_adapter_kind")?;
    let order_number = row_string(&row, "order_number")?;
    let provider_object_id = match row_optional_string(&row, "provider_object_id")? {
        Some(value) => value,
        None if matches!(adapter_kind.as_str(), "alipay" | "wechat") => order_number.clone(),
        None => return Err(PaymentOperationsError::PaymentContractInvalid),
    };

    Ok(LoadedPaymentQuery {
        attempt_id: row_string(&row, "attempt_id")?,
        attempt_state: row_string(&row, "attempt_state")?,
        order_id: row_string(&row, "order_id")?,
        channel_id: row_string(&row, "attempt_channel_id")?,
        adapter_kind,
        credential_version_id,
        merchant_account_identity: row_string(&row, "merchant_account_identity")?,
        provider_object_id,
        order_number,
        amount_minor: row_string(&row, "payment_minor")?,
        currency,
        payment_hold: row_i32(&row, "payment_hold")? != 0,
        credential: StoredCredential {
            channel_id: required_credential_string(&row, "credential_channel_id")?,
            adapter_kind: required_credential_string(&row, "credential_adapter_kind")?,
            account_identity_digest: required_credential_string(&row, "account_identity_digest")?,
            encrypted_secret: EncryptedSecret {
                version,
                key_id: required_credential_string(&row, "key_id")?,
                nonce_base64: required_credential_string(&row, "nonce_base64")?,
                ciphertext_base64: required_credential_string(&row, "ciphertext_base64")?,
            },
        },
    })
}

fn validate_account_identity(
    account_id: &str,
    loaded: &LoadedPaymentQuery,
) -> Result<(), PaymentOperationsError> {
    let digest = Sha256::digest(account_id.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    validate_account_identity_digest(&digest, loaded)
}

fn validate_account_identity_digest(
    digest: &str,
    loaded: &LoadedPaymentQuery,
) -> Result<(), PaymentOperationsError> {
    if digest != loaded.merchant_account_identity {
        return Err(PaymentOperationsError::AccountIdentityMismatch);
    }
    Ok(())
}

fn required_credential_string(
    row: &QueryResult,
    column: &str,
) -> Result<String, PaymentOperationsError> {
    row_optional_string(row, column)?.ok_or(PaymentOperationsError::CredentialNotFound)
}

fn row_string(row: &QueryResult, column: &str) -> Result<String, PaymentOperationsError> {
    row.try_get("", column).map_err(storage)
}

fn row_i32(row: &QueryResult, column: &str) -> Result<i32, PaymentOperationsError> {
    row.try_get("", column).map_err(storage)
}

fn row_optional_string(
    row: &QueryResult,
    column: &str,
) -> Result<Option<String>, PaymentOperationsError> {
    row.try_get("", column).map_err(storage)
}

fn storage(error: sea_orm::DbErr) -> PaymentOperationsError {
    PaymentOperationsError::Storage(error.to_string())
}
