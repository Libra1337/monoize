use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use url::Url;

use super::money::Currency;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CheckoutAction {
    Redirect {
        url: String,
        expires_at: String,
    },
    Qr {
        payload: String,
        expires_at: String,
    },
    Form {
        action: String,
        fields: Vec<(String, String)>,
        expires_at: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckoutRequest {
    pub attempt_id: String,
    pub order_number: String,
    pub amount_minor: String,
    pub currency: Currency,
    pub success_url: Url,
    pub cancel_url: Url,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentQuery {
    pub provider_object_id: String,
    pub merchant_order_number: String,
    pub amount_minor: String,
    pub currency: Currency,
}

pub(crate) fn validate_payment_query(query: &PaymentQuery) -> Result<u64, AdapterError> {
    if query.provider_object_id.is_empty()
        || query.provider_object_id.len() > 255
        || query.provider_object_id.trim() != query.provider_object_id
        || query.merchant_order_number.is_empty()
        || query.merchant_order_number.len() > 255
        || query.merchant_order_number.trim() != query.merchant_order_number
    {
        return Err(AdapterError::InvalidRequest);
    }
    query
        .amount_minor
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or(AdapterError::InvalidRequest)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefundRequest {
    pub provider_transaction_id: String,
    pub merchant_order_number: String,
    pub amount_minor: String,
    pub currency: Currency,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderPaymentState {
    NotFound,
    Unpaid,
    Paid { provider_transaction_id: String },
    Closed,
    Ambiguous,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderRefundState {
    NotFound,
    Pending,
    Succeeded,
    Failed,
    Ambiguous,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedProviderEvent {
    pub provider_event_id: String,
    pub event_kind: String,
    pub merchant_order_number: String,
    pub provider_transaction_id: Option<String>,
    pub amount_minor: Option<String>,
    pub currency: Option<Currency>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AdapterError {
    #[error("adapter configuration is invalid")]
    InvalidConfiguration,
    #[error("adapter request is invalid")]
    InvalidRequest,
    #[error("provider response is ambiguous")]
    Ambiguous,
    #[error("provider rejected the request")]
    Rejected,
    #[error("provider response verification failed")]
    Verification,
    #[error("adapter operation is unsupported")]
    Unsupported,
}

#[async_trait]
pub trait PaymentAdapter: Send + Sync {
    async fn create_checkout(
        &self,
        request: &CheckoutRequest,
    ) -> Result<CheckoutAction, AdapterError>;

    async fn query_payment(
        &self,
        query: &PaymentQuery,
    ) -> Result<ProviderPaymentState, AdapterError>;

    async fn refund_payment(
        &self,
        request: &RefundRequest,
    ) -> Result<ProviderRefundState, AdapterError>;

    fn verify_callback(&self, body: &[u8]) -> Result<VerifiedProviderEvent, AdapterError>;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReturnUrlError {
    #[error("configured public origin is invalid")]
    InvalidPublicOrigin,
    #[error("return URL is outside the configured public origin")]
    ForeignOrigin,
}

pub fn validate_return_url(
    configured_public_origin: &str,
    candidate: &str,
) -> Result<Url, ReturnUrlError> {
    let origin =
        Url::parse(configured_public_origin).map_err(|_| ReturnUrlError::InvalidPublicOrigin)?;
    if origin.scheme() != "https"
        || origin.host_str().is_none()
        || !origin.username().is_empty()
        || origin.password().is_some()
        || origin.path() != "/"
        || origin.query().is_some()
        || origin.fragment().is_some()
    {
        return Err(ReturnUrlError::InvalidPublicOrigin);
    }

    let candidate = Url::parse(candidate).map_err(|_| ReturnUrlError::ForeignOrigin)?;
    if candidate.scheme() != "https"
        || candidate.host_str() != origin.host_str()
        || candidate.port_or_known_default() != origin.port_or_known_default()
        || !candidate.username().is_empty()
        || candidate.password().is_some()
        || candidate.fragment().is_some()
    {
        return Err(ReturnUrlError::ForeignOrigin);
    }
    Ok(candidate)
}
