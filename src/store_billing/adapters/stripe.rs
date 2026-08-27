use serde::{Deserialize, Serialize};

use crate::store_billing::crypto::{CryptoError, verify_hmac_sha256_hex};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedStripeEvent {
    pub id: String,
    pub kind: String,
    pub api_version: String,
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

impl From<CryptoError> for StripeWebhookError {
    fn from(_: CryptoError) -> Self {
        Self::Authentication
    }
}
