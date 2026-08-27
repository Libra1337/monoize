use std::sync::Arc;

use async_trait::async_trait;
use sea_orm::{ConnectionTrait, QueryResult};
use sha2::{Digest, Sha256};
use url::Url;

use super::adapters::stripe::{
    StripeCheckoutResult, StripeCredential, create_checkout as create_stripe_checkout,
};
use super::crypto::{EncryptedSecret, PaymentKeyRing};
use super::order::{
    CreatePaymentAttemptInput, PaymentAttempt, PaymentAttemptFailureKind, PaymentAttemptState,
    PaymentOrder, PaymentOrderError, PaymentOrderStore,
};
use super::payment::{AdapterError, CheckoutAction, CheckoutRequest};
use crate::db::DbPool;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckoutResult {
    pub attempt: PaymentAttempt,
    pub action: CheckoutAction,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CheckoutError {
    #[error("payment checkout configuration is unavailable")]
    ConfigurationUnavailable,
    #[error("provider checkout state is ambiguous")]
    ProviderAmbiguous,
    #[error("provider rejected checkout")]
    ProviderRejected,
    #[error(transparent)]
    Order(#[from] PaymentOrderError),
}

#[async_trait]
pub trait CheckoutProvider: Send + Sync {
    async fn create_stripe_checkout(
        &self,
        credential: &StripeCredential,
        request: &CheckoutRequest,
    ) -> Result<StripeCheckoutResult, AdapterError>;
}

#[derive(Clone)]
pub struct ReqwestCheckoutProvider {
    client: reqwest::Client,
}

impl ReqwestCheckoutProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl CheckoutProvider for ReqwestCheckoutProvider {
    async fn create_stripe_checkout(
        &self,
        credential: &StripeCredential,
        request: &CheckoutRequest,
    ) -> Result<StripeCheckoutResult, AdapterError> {
        create_stripe_checkout(&self.client, credential, request).await
    }
}

#[derive(Clone)]
pub struct CheckoutService {
    db: DbPool,
    payment_keys: Option<Arc<PaymentKeyRing>>,
    public_origin: Option<Url>,
    provider: Arc<dyn CheckoutProvider>,
}

impl CheckoutService {
    pub fn new(
        db: DbPool,
        payment_keys: Option<Arc<PaymentKeyRing>>,
        public_origin: Option<Url>,
        provider: Arc<dyn CheckoutProvider>,
    ) -> Self {
        Self {
            db,
            payment_keys,
            public_origin,
            provider,
        }
    }

    pub async fn create_attempt(
        &self,
        user_id: &str,
        order_id: &str,
        input: CreatePaymentAttemptInput,
    ) -> Result<CheckoutResult, CheckoutError> {
        let orders = PaymentOrderStore::new(self.db.clone());
        let outcome = orders
            .create_attempt_with_outcome(user_id, order_id, input)
            .await?;
        let attempt = outcome.attempt;
        if outcome.replayed {
            return match attempt.state {
                PaymentAttemptState::Presented | PaymentAttemptState::Paid => {
                    let action = attempt
                        .action
                        .clone()
                        .ok_or(CheckoutError::ConfigurationUnavailable)?;
                    Ok(CheckoutResult {
                        attempt,
                        action,
                        replayed: true,
                    })
                }
                PaymentAttemptState::Created => Err(CheckoutError::ProviderAmbiguous),
                PaymentAttemptState::Failed => match attempt.failure_kind {
                    Some(PaymentAttemptFailureKind::ProviderRejected) => {
                        Err(CheckoutError::ProviderRejected)
                    }
                    Some(PaymentAttemptFailureKind::ConfigurationUnavailable) | None => {
                        Err(CheckoutError::ConfigurationUnavailable)
                    }
                },
                PaymentAttemptState::Expired => {
                    Err(CheckoutError::Order(PaymentOrderError::OrderNotPayable))
                }
            };
        }
        let order = orders
            .get_order_for_user(user_id, order_id)
            .await?
            .ok_or(PaymentOrderError::OrderNotFound)?;
        let provider_result = match self.create_provider_checkout(&attempt, &order).await {
            Ok(result) => result,
            Err(error @ CheckoutError::ConfigurationUnavailable)
            | Err(error @ CheckoutError::ProviderRejected) => {
                let failure_kind = match error {
                    CheckoutError::ProviderRejected => PaymentAttemptFailureKind::ProviderRejected,
                    _ => PaymentAttemptFailureKind::ConfigurationUnavailable,
                };
                orders
                    .fail_attempt(user_id, &attempt.id, failure_kind)
                    .await?;
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        let attempt = orders
            .present_attempt(
                user_id,
                &attempt.id,
                &provider_result.provider_object_id,
                &provider_result.action,
            )
            .await?;
        Ok(CheckoutResult {
            action: provider_result.action,
            attempt,
            replayed: false,
        })
    }

    async fn create_provider_checkout(
        &self,
        attempt: &PaymentAttempt,
        order: &PaymentOrder,
    ) -> Result<StripeCheckoutResult, CheckoutError> {
        let payment_keys = self
            .payment_keys
            .as_ref()
            .ok_or(CheckoutError::ConfigurationUnavailable)?;
        let public_origin = self
            .public_origin
            .as_ref()
            .ok_or(CheckoutError::ConfigurationUnavailable)?;
        let stored = load_credential(&self.db, &attempt.credential_version_id).await?;
        if attempt.adapter_kind != "stripe"
            || stored.adapter_kind != attempt.adapter_kind
            || stored.channel_id != attempt.channel_id
            || stored.account_identity_digest != attempt.merchant_account_identity
        {
            return Err(CheckoutError::ConfigurationUnavailable);
        }
        let aad = format!(
            "store_channel_credentials:{}:secret",
            attempt.credential_version_id
        );
        let plaintext = payment_keys
            .decrypt(&aad, &stored.encrypted_secret)
            .map_err(|_| CheckoutError::ConfigurationUnavailable)?;
        let credential = StripeCredential::from_json(&plaintext)
            .map_err(|_| CheckoutError::ConfigurationUnavailable)?;
        if account_identity_digest(credential.account_id()) != attempt.merchant_account_identity {
            return Err(CheckoutError::ConfigurationUnavailable);
        }
        let (success_url, cancel_url) = return_urls(public_origin, &order.id)?;
        let request = CheckoutRequest {
            attempt_id: attempt.id.clone(),
            order_number: order.order_number.clone(),
            amount_minor: order.payment_minor.clone(),
            currency: order.payment_currency,
            success_url,
            cancel_url,
        };
        self.provider
            .create_stripe_checkout(&credential, &request)
            .await
            .map_err(map_adapter_error)
    }
}

struct StoredCredential {
    channel_id: String,
    adapter_kind: String,
    account_identity_digest: String,
    encrypted_secret: EncryptedSecret,
}

async fn load_credential(
    db: &DbPool,
    credential_id: &str,
) -> Result<StoredCredential, CheckoutError> {
    let row = db
        .read()
        .query_one(db.stmt(
            "SELECT channel_id, adapter_kind, format_version, key_id, nonce_base64,
                    ciphertext_base64, account_identity_digest
             FROM store_channel_credentials WHERE id = $1",
            vec![credential_id.into()],
        ))
        .await
        .map_err(storage)?
        .ok_or(CheckoutError::ConfigurationUnavailable)?;
    let version = row
        .try_get::<i32>("", "format_version")
        .map_err(storage)
        .and_then(|value| {
            u8::try_from(value).map_err(|_| CheckoutError::ConfigurationUnavailable)
        })?;
    Ok(StoredCredential {
        channel_id: row_string(&row, "channel_id")?,
        adapter_kind: row_string(&row, "adapter_kind")?,
        account_identity_digest: row_string(&row, "account_identity_digest")?,
        encrypted_secret: EncryptedSecret {
            version,
            key_id: row_string(&row, "key_id")?,
            nonce_base64: row_string(&row, "nonce_base64")?,
            ciphertext_base64: row_string(&row, "ciphertext_base64")?,
        },
    })
}

fn return_urls(public_origin: &Url, order_id: &str) -> Result<(Url, Url), CheckoutError> {
    let mut success = public_origin
        .join("/dashboard/store")
        .map_err(|_| CheckoutError::ConfigurationUnavailable)?;
    success
        .query_pairs_mut()
        .append_pair("order_id", order_id)
        .append_pair("checkout", "success");
    let mut cancel = public_origin
        .join("/dashboard/store")
        .map_err(|_| CheckoutError::ConfigurationUnavailable)?;
    cancel
        .query_pairs_mut()
        .append_pair("order_id", order_id)
        .append_pair("checkout", "cancel");
    Ok((success, cancel))
}

fn account_identity_digest(account_id: &str) -> String {
    Sha256::digest(account_id.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn map_adapter_error(error: AdapterError) -> CheckoutError {
    match error {
        AdapterError::Ambiguous => CheckoutError::ProviderAmbiguous,
        AdapterError::Rejected => CheckoutError::ProviderRejected,
        AdapterError::InvalidConfiguration
        | AdapterError::InvalidRequest
        | AdapterError::Verification
        | AdapterError::Unsupported => CheckoutError::ConfigurationUnavailable,
    }
}

fn row_string(row: &QueryResult, column: &str) -> Result<String, CheckoutError> {
    row.try_get("", column).map_err(storage)
}

fn storage(error: sea_orm::DbErr) -> CheckoutError {
    CheckoutError::Order(PaymentOrderError::Storage(error.to_string()))
}
