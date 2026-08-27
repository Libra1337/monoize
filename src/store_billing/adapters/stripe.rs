use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use crate::store_billing::crypto::{CryptoError, verify_hmac_sha256_hex};
use crate::store_billing::money::Currency;
use crate::store_billing::payment::CheckoutAction;
use crate::store_billing::payment::{
    AdapterError, CheckoutRequest, PaymentQuery, ProviderPaymentState, validate_payment_query,
};

pub const STRIPE_CHECKOUT_SESSIONS_URL: &str = "https://api.stripe.com/v1/checkout/sessions";

#[derive(Clone, Deserialize, Zeroize)]
#[serde(deny_unknown_fields)]
#[zeroize(drop)]
pub struct StripeCredential {
    secret_key: String,
    publishable_key: String,
    webhook_signing_secret: String,
    api_version: String,
    account_id: String,
    live_mode: bool,
}

impl StripeCredential {
    pub fn from_json(raw: &[u8]) -> Result<Zeroizing<Self>, AdapterError> {
        let credential: Self =
            serde_json::from_slice(raw).map_err(|_| AdapterError::InvalidConfiguration)?;
        credential.validate()?;
        Ok(Zeroizing::new(credential))
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn api_version(&self) -> &str {
        &self.api_version
    }

    pub fn webhook_signing_secret(&self) -> &[u8] {
        self.webhook_signing_secret.as_bytes()
    }

    fn validate(&self) -> Result<(), AdapterError> {
        if [
            &self.secret_key,
            &self.publishable_key,
            &self.webhook_signing_secret,
            &self.api_version,
            &self.account_id,
        ]
        .into_iter()
        .any(|value| value.trim().is_empty())
            || self.secret_key.starts_with("sk_live_") != self.live_mode
            || self.publishable_key.starts_with("pk_live_") != self.live_mode
            || !valid_api_version(&self.api_version)
        {
            return Err(AdapterError::InvalidConfiguration);
        }
        Ok(())
    }
}

impl fmt::Debug for StripeCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StripeCredential")
            .field("secret_key", &"[REDACTED]")
            .field("publishable_key", &"[REDACTED]")
            .field("webhook_signing_secret", &"[REDACTED]")
            .field("api_version", &self.api_version)
            .field("account_id", &self.account_id)
            .field("live_mode", &self.live_mode)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct PreparedStripeCheckout {
    pub endpoint: &'static str,
    pub authorization: Zeroizing<String>,
    pub idempotency_key: String,
    pub api_version: String,
    pub form: BTreeMap<String, String>,
}

impl fmt::Debug for PreparedStripeCheckout {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedStripeCheckout")
            .field("endpoint", &self.endpoint)
            .field("authorization", &"[REDACTED]")
            .field("idempotency_key", &self.idempotency_key)
            .field("api_version", &self.api_version)
            .field("form", &self.form)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripeCheckoutResult {
    pub provider_object_id: String,
    pub action: CheckoutAction,
}

#[derive(Clone, PartialEq, Eq)]
pub struct PreparedStripePaymentQuery {
    pub endpoint: String,
    pub authorization: Zeroizing<String>,
    pub api_version: String,
}

impl fmt::Debug for PreparedStripePaymentQuery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedStripePaymentQuery")
            .field("endpoint", &self.endpoint)
            .field("authorization", &"[REDACTED]")
            .field("api_version", &self.api_version)
            .finish()
    }
}

pub fn prepare_payment_query(
    credential: &StripeCredential,
    query: &PaymentQuery,
) -> Result<PreparedStripePaymentQuery, AdapterError> {
    credential.validate()?;
    validate_payment_query(query)?;
    let mut endpoint = url::Url::parse(STRIPE_CHECKOUT_SESSIONS_URL)
        .map_err(|_| AdapterError::InvalidConfiguration)?;
    endpoint
        .path_segments_mut()
        .map_err(|_| AdapterError::InvalidConfiguration)?
        .push(&query.provider_object_id);
    Ok(PreparedStripePaymentQuery {
        endpoint: endpoint.to_string(),
        authorization: Zeroizing::new(format!("Bearer {}", credential.secret_key)),
        api_version: credential.api_version.clone(),
    })
}

pub fn parse_payment_query_response(
    status: reqwest::StatusCode,
    body: &[u8],
    query: &PaymentQuery,
) -> Result<ProviderPaymentState, AdapterError> {
    validate_payment_query(query)?;
    if status == reqwest::StatusCode::NOT_FOUND {
        #[derive(Deserialize)]
        struct ErrorEnvelope {
            error: QueryError,
        }
        #[derive(Deserialize)]
        struct QueryError {
            #[serde(rename = "type")]
            kind: String,
            code: String,
            message: String,
        }
        let error: ErrorEnvelope =
            serde_json::from_slice(body).map_err(|_| AdapterError::Ambiguous)?;
        if error.error.kind == "invalid_request_error"
            && error.error.code == "resource_missing"
            && !error.error.message.trim().is_empty()
        {
            return Ok(ProviderPaymentState::NotFound);
        }
        return Err(AdapterError::Ambiguous);
    }
    if !status.is_success() {
        return Err(AdapterError::Ambiguous);
    }
    #[derive(Deserialize)]
    struct Session {
        id: String,
        object: String,
        amount_total: u64,
        currency: String,
        client_reference_id: String,
        payment_intent: Option<String>,
        payment_status: String,
        status: String,
    }
    let session: Session = serde_json::from_slice(body).map_err(|_| AdapterError::Verification)?;
    let expected_currency = match query.currency {
        Currency::CNY => "cny",
        Currency::USD => "usd",
    };
    let expected_amount = validate_payment_query(query)?;
    if session.id != query.provider_object_id
        || session.object != "checkout.session"
        || session.amount_total != expected_amount
        || session.currency != expected_currency
        || session.client_reference_id != query.merchant_order_number
    {
        return Err(AdapterError::Verification);
    }
    match (session.payment_status.as_str(), session.status.as_str()) {
        ("paid", "complete") => session
            .payment_intent
            .filter(|value| !value.is_empty() && value.trim() == value)
            .map(|provider_transaction_id| ProviderPaymentState::Paid {
                provider_transaction_id,
            })
            .ok_or(AdapterError::Verification),
        ("unpaid", "open") => Ok(ProviderPaymentState::Unpaid),
        ("unpaid", "expired") => Ok(ProviderPaymentState::Closed),
        _ => Ok(ProviderPaymentState::Ambiguous),
    }
}

pub async fn query_payment(
    client: &reqwest::Client,
    credential: &StripeCredential,
    query: &PaymentQuery,
) -> Result<ProviderPaymentState, AdapterError> {
    let prepared = prepare_payment_query(credential, query)?;
    let response = client
        .get(&prepared.endpoint)
        .header(
            reqwest::header::AUTHORIZATION,
            prepared.authorization.as_str(),
        )
        .header("Stripe-Version", prepared.api_version)
        .send()
        .await
        .map_err(|_| AdapterError::Ambiguous)?;
    let status = response.status();
    let body = crate::bounded_response::read_response_body_with_limit(response, 65_536)
        .await
        .map_err(|_| AdapterError::Ambiguous)?;
    parse_payment_query_response(status, &body, query)
}

pub fn prepare_checkout_request(
    credential: &StripeCredential,
    request: &CheckoutRequest,
) -> Result<PreparedStripeCheckout, AdapterError> {
    credential.validate()?;
    let amount = request
        .amount_minor
        .parse::<u64>()
        .ok()
        .filter(|amount| *amount > 0)
        .ok_or(AdapterError::InvalidRequest)?;
    if request.success_url.scheme() != "https" || request.cancel_url.scheme() != "https" {
        return Err(AdapterError::InvalidRequest);
    }
    let currency = match request.currency {
        Currency::CNY => "cny",
        Currency::USD => "usd",
    };
    let form = BTreeMap::from([
        ("mode".to_string(), "payment".to_string()),
        (
            "client_reference_id".to_string(),
            request.order_number.clone(),
        ),
        (
            "metadata[store_attempt_id]".to_string(),
            request.attempt_id.clone(),
        ),
        ("line_items[0][quantity]".to_string(), "1".to_string()),
        (
            "line_items[0][price_data][currency]".to_string(),
            currency.to_string(),
        ),
        (
            "line_items[0][price_data][unit_amount]".to_string(),
            amount.to_string(),
        ),
        (
            "line_items[0][price_data][product_data][name]".to_string(),
            request.order_number.clone(),
        ),
        ("success_url".to_string(), request.success_url.to_string()),
        ("cancel_url".to_string(), request.cancel_url.to_string()),
    ]);
    Ok(PreparedStripeCheckout {
        endpoint: STRIPE_CHECKOUT_SESSIONS_URL,
        authorization: Zeroizing::new(format!("Bearer {}", credential.secret_key)),
        idempotency_key: request.order_number.clone(),
        api_version: credential.api_version.clone(),
        form,
    })
}

pub fn parse_checkout_response(body: &[u8]) -> Result<StripeCheckoutResult, AdapterError> {
    #[derive(Deserialize)]
    struct Response {
        id: String,
        url: String,
        expires_at: i64,
    }
    let response: Response = serde_json::from_slice(body).map_err(|_| AdapterError::Ambiguous)?;
    let url = url::Url::parse(&response.url).map_err(|_| AdapterError::Ambiguous)?;
    if response.id.trim().is_empty()
        || url.scheme() != "https"
        || url.host_str().is_none()
        || response.expires_at <= 0
    {
        return Err(AdapterError::Ambiguous);
    }
    let expires_at = chrono::DateTime::<chrono::Utc>::from_timestamp(response.expires_at, 0)
        .ok_or(AdapterError::Ambiguous)?
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    Ok(StripeCheckoutResult {
        provider_object_id: response.id,
        action: CheckoutAction::Redirect {
            url: response.url,
            expires_at,
        },
    })
}

pub async fn create_checkout(
    client: &reqwest::Client,
    credential: &StripeCredential,
    request: &CheckoutRequest,
) -> Result<StripeCheckoutResult, AdapterError> {
    let prepared = prepare_checkout_request(credential, request)?;
    let response = client
        .post(prepared.endpoint)
        .header(
            reqwest::header::AUTHORIZATION,
            prepared.authorization.as_str(),
        )
        .header("Stripe-Version", prepared.api_version)
        .header("Idempotency-Key", prepared.idempotency_key)
        .form(&prepared.form)
        .send()
        .await
        .map_err(|_| AdapterError::Ambiguous)?;
    let status = response.status();
    let body = crate::bounded_response::read_response_body_with_limit(response, 65_536)
        .await
        .map_err(|_| AdapterError::Ambiguous)?;
    if !status.is_success() {
        return Err(classify_checkout_error_response(status, &body));
    }
    parse_checkout_response(&body)
}

pub fn classify_checkout_error_response(status: reqwest::StatusCode, body: &[u8]) -> AdapterError {
    #[derive(Deserialize)]
    struct ErrorEnvelope {
        error: StripeError,
    }

    #[derive(Deserialize)]
    struct StripeError {
        #[serde(rename = "type")]
        kind: String,
        message: String,
    }

    let recognized = serde_json::from_slice::<ErrorEnvelope>(body)
        .ok()
        .is_some_and(|value| {
            !value.error.kind.trim().is_empty() && !value.error.message.trim().is_empty()
        });
    if status.is_client_error() && recognized {
        AdapterError::Rejected
    } else {
        AdapterError::Ambiguous
    }
}

fn valid_api_version(value: &str) -> bool {
    let (date, suffix_is_valid) = match value.split_once('.') {
        Some((date, suffix)) => (
            date,
            !suffix.is_empty()
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        ),
        None => (value, true),
    };
    let bytes = date.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
        && suffix_is_valid
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedStripeEvent {
    pub id: String,
    pub kind: String,
    pub api_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripePaymentEvent {
    pub provider_event_id: String,
    pub attempt_id: String,
    pub order_number: String,
    pub checkout_session_id: String,
    pub payment_intent_id: String,
    pub amount_minor: String,
    pub currency: Currency,
    pub account_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StripeWebhookError {
    #[error("Stripe-Signature is invalid")]
    InvalidSignatureHeader,
    #[error("Stripe Webhook timestamp is outside tolerance")]
    TimestampOutsideTolerance,
    #[error("Stripe Webhook signature verification failed")]
    Authentication,
    #[error("Stripe event JSON is invalid")]
    InvalidEvent,
    #[error("Stripe event API version differs from configured version")]
    ApiVersionMismatch,
    #[error("Stripe payment event fields are invalid")]
    InvalidPaymentEvent,
}

pub fn verify_stripe_webhook(
    signing_secret: &[u8],
    signature_header: &str,
    body: &[u8],
    now_unix: i64,
    tolerance_seconds: i64,
    expected_api_version: &str,
) -> Result<VerifiedStripeEvent, StripeWebhookError> {
    if tolerance_seconds < 0 {
        return Err(StripeWebhookError::TimestampOutsideTolerance);
    }
    let mut timestamp = None;
    let mut signatures = Vec::new();
    for field in signature_header.split(',') {
        let Some((name, value)) = field.trim().split_once('=') else {
            return Err(StripeWebhookError::InvalidSignatureHeader);
        };
        match name {
            "t" => {
                timestamp = Some(
                    value
                        .parse::<i64>()
                        .map_err(|_| StripeWebhookError::InvalidSignatureHeader)?,
                )
            }
            "v1" if !value.is_empty() => signatures.push(value),
            _ => {}
        }
    }
    let timestamp = timestamp.ok_or(StripeWebhookError::InvalidSignatureHeader)?;
    if signatures.is_empty() {
        return Err(StripeWebhookError::InvalidSignatureHeader);
    }
    if now_unix.abs_diff(timestamp) > tolerance_seconds as u64 {
        return Err(StripeWebhookError::TimestampOutsideTolerance);
    }

    let mut signed_payload = timestamp.to_string().into_bytes();
    signed_payload.push(b'.');
    signed_payload.extend_from_slice(body);
    let authenticated = signatures.iter().any(|signature| {
        verify_hmac_sha256_hex(signing_secret, &signed_payload, signature).is_ok()
    });
    if !authenticated {
        return Err(StripeWebhookError::Authentication);
    }

    #[derive(Deserialize)]
    struct StripeEventWire {
        id: String,
        #[serde(rename = "type")]
        kind: String,
        api_version: String,
    }
    let event: StripeEventWire =
        serde_json::from_slice(body).map_err(|_| StripeWebhookError::InvalidEvent)?;
    if event.id.is_empty() || event.kind.is_empty() || event.api_version.is_empty() {
        return Err(StripeWebhookError::InvalidEvent);
    }
    if event.api_version != expected_api_version {
        return Err(StripeWebhookError::ApiVersionMismatch);
    }
    Ok(VerifiedStripeEvent {
        id: event.id,
        kind: event.kind,
        api_version: event.api_version,
    })
}

pub fn parse_stripe_payment_event(
    body: &[u8],
    verified: &VerifiedStripeEvent,
    expected_account_id: &str,
) -> Result<StripePaymentEvent, StripeWebhookError> {
    #[derive(Deserialize)]
    struct Event {
        id: String,
        #[serde(rename = "type")]
        kind: String,
        account: String,
        data: EventData,
    }

    #[derive(Deserialize)]
    struct EventData {
        object: CheckoutSession,
    }

    #[derive(Deserialize)]
    struct CheckoutSession {
        id: String,
        object: String,
        amount_total: u64,
        currency: String,
        client_reference_id: String,
        metadata: CheckoutMetadata,
        payment_intent: String,
        payment_status: String,
    }

    #[derive(Deserialize)]
    struct CheckoutMetadata {
        store_attempt_id: String,
    }

    let event: Event =
        serde_json::from_slice(body).map_err(|_| StripeWebhookError::InvalidPaymentEvent)?;
    if event.id != verified.id
        || event.kind != verified.kind
        || !matches!(
            event.kind.as_str(),
            "checkout.session.completed" | "checkout.session.async_payment_succeeded"
        )
        || event.account != expected_account_id
        || event.data.object.object != "checkout.session"
        || event.data.object.id.is_empty()
        || event.data.object.amount_total == 0
        || event.data.object.client_reference_id.is_empty()
        || event.data.object.metadata.store_attempt_id.is_empty()
        || event.data.object.payment_intent.is_empty()
        || event.data.object.payment_status != "paid"
    {
        return Err(StripeWebhookError::InvalidPaymentEvent);
    }
    let currency = match event.data.object.currency.as_str() {
        "cny" => Currency::CNY,
        "usd" => Currency::USD,
        _ => return Err(StripeWebhookError::InvalidPaymentEvent),
    };
    Ok(StripePaymentEvent {
        provider_event_id: event.id,
        attempt_id: event.data.object.metadata.store_attempt_id,
        order_number: event.data.object.client_reference_id,
        checkout_session_id: event.data.object.id,
        payment_intent_id: event.data.object.payment_intent,
        amount_minor: event.data.object.amount_total.to_string(),
        currency,
        account_id: event.account,
    })
}

impl From<CryptoError> for StripeWebhookError {
    fn from(_: CryptoError) -> Self {
        Self::Authentication
    }
}
