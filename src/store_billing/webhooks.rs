use std::time::Duration;

use axum::Json;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, StatusCode};
use chrono::Utc;
use sea_orm::{ConnectionTrait, QueryResult};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::adapters::stripe::{
    StripeCredential, StripeWebhookError, parse_stripe_payment_event, verify_stripe_webhook,
};
use super::callbacks::{ApplyProviderEventInput, CallbackStoreError, PaymentCallbackStore};
use super::crypto::EncryptedSecret;
use crate::app::AppState;
use crate::client_ip::canonical_client_ip_from_headers;
use crate::error::{AppError, AppResult};

const CALLBACK_BODY_MAX_BYTES: usize = 131_072;
const CALLBACK_BODY_TIMEOUT: Duration = Duration::from_secs(5);
const STRIPE_SIGNATURE_TOLERANCE_SECONDS: i64 = 300;

struct StoredStripeCredential {
    id: String,
    account_identity_digest: String,
    encrypted_secret: EncryptedSecret,
}

pub async fn store_payment_callback(
    Path(channel_id): Path<String>,
    State(state): State<AppState>,
    request: Request,
) -> AppResult<Json<Value>> {
    let headers = request.headers().clone();
    let source_ip = canonical_client_ip_from_headers(&headers);
    if !state.store_callback_limiter.allow(&channel_id, source_ip) {
        return Err(AppError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "callback_rate_limited",
            "payment callback rate limit exceeded",
        ));
    }
    let signature = required_header(&headers, "stripe-signature")?;
    let body = read_callback_body(request.into_body()).await?;
    let payment_keys = state.payment_keys.as_ref().ok_or_else(|| {
        AppError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "payment_configuration_unavailable",
            "payment callback configuration is unavailable",
        )
    })?;
    let credentials = load_stripe_credentials(&state, &channel_id).await?;
    if credentials.is_empty() {
        return Err(AppError::new(
            StatusCode::NOT_FOUND,
            "payment_channel_not_found",
            "payment Channel was not found",
        ));
    }

    let mut usable_credential = false;
    let mut authenticated_error = None;
    let mut selected = None;
    for stored in credentials {
        let aad = format!("store_channel_credentials:{}:secret", stored.id);
        let Ok(plaintext) = payment_keys.decrypt(&aad, &stored.encrypted_secret) else {
            continue;
        };
        let Ok(credential) = StripeCredential::from_json(&plaintext) else {
            continue;
        };
        if account_identity_digest(credential.account_id()) != stored.account_identity_digest {
            continue;
        }
        usable_credential = true;
        match verify_stripe_webhook(
            credential.webhook_signing_secret(),
            &signature,
            &body,
            Utc::now().timestamp(),
            STRIPE_SIGNATURE_TOLERANCE_SECONDS,
            credential.api_version(),
        ) {
            Ok(verified) => {
                match parse_stripe_payment_event(&body, &verified, credential.account_id()) {
                    Ok(payment) => {
                        selected = Some((stored, payment));
                        break;
                    }
                    Err(error) => authenticated_error = Some(error),
                }
            }
            Err(StripeWebhookError::Authentication) => {}
            Err(StripeWebhookError::ApiVersionMismatch) => {
                authenticated_error = Some(StripeWebhookError::ApiVersionMismatch)
            }
            Err(error) => return Err(invalid_stripe_event(error)),
        }
    }
    let Some((stored, payment)) = selected else {
        if !usable_credential {
            return Err(AppError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "payment_configuration_unavailable",
                "payment callback configuration is unavailable",
            ));
        }
        return Err(invalid_stripe_event(
            authenticated_error.unwrap_or(StripeWebhookError::Authentication),
        ));
    };

    let order_id = load_attempt_order_id(&state, &payment.attempt_id).await?;
    let event_row_id = Uuid::new_v4().to_string();
    let raw_body = payment_keys
        .encrypt(
            &format!("store_provider_events:{event_row_id}:raw_body"),
            &body,
        )
        .map_err(|error| {
            AppError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "payment_configuration_unavailable",
                "payment callback configuration is unavailable",
            )
            .with_internal_message(error.to_string())
        })?;
    let body_digest = Sha256::digest(&body)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let parsed_json = json!({
        "event_id": &payment.provider_event_id,
        "event_kind": "payment_succeeded",
        "checkout_session_id": &payment.checkout_session_id,
        "payment_intent_id": &payment.payment_intent_id,
        "attempt_id": &payment.attempt_id,
        "order_number": &payment.order_number,
        "amount_minor": &payment.amount_minor,
        "currency": payment.currency,
        "account_identity": &stored.account_identity_digest,
    });
    let user_agent = headers
        .get("user-agent")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.chars().take(512).collect::<String>());
    PaymentCallbackStore::new(state.db_pool.clone())
        .apply_verified_payment(ApplyProviderEventInput {
            event_row_id,
            credential_version_id: stored.id,
            provider_event_id: payment.provider_event_id,
            event_kind: "payment_succeeded".to_string(),
            order_id,
            attempt_id: payment.attempt_id,
            provider_transaction_id: payment.payment_intent_id,
            provider_object_id: payment.checkout_session_id,
            order_number: payment.order_number,
            merchant_account_identity: stored.account_identity_digest,
            amount_minor: payment.amount_minor,
            currency: payment.currency,
            body_digest,
            parsed_json,
            raw_body,
            source_ip: source_ip.map(|ip| ip.to_string()),
            user_agent,
            received_at: Utc::now(),
        })
        .await
        .map_err(map_callback_store_error)?;
    Ok(Json(json!({"received": true})))
}

async fn read_callback_body(body: Body) -> AppResult<Bytes> {
    match tokio::time::timeout(
        CALLBACK_BODY_TIMEOUT,
        to_bytes(body, CALLBACK_BODY_MAX_BYTES),
    )
    .await
    {
        Ok(Ok(body)) => Ok(body),
        Ok(Err(error)) => Err(AppError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "callback_body_too_large",
            "payment callback body is too large",
        )
        .with_internal_message(error.to_string())),
        Err(_) => Err(AppError::new(
            StatusCode::REQUEST_TIMEOUT,
            "callback_body_timeout",
            "payment callback body timed out",
        )),
    }
}

fn required_header(headers: &HeaderMap, name: &'static str) -> AppResult<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| invalid_stripe_event(StripeWebhookError::InvalidSignatureHeader))
}

async fn load_stripe_credentials(
    state: &AppState,
    channel_id: &str,
) -> AppResult<Vec<StoredStripeCredential>> {
    let rows = state
        .db_pool
        .read()
        .query_all(state.db_pool.stmt(
            "SELECT c.id, c.format_version, c.key_id, c.nonce_base64,
                    c.ciphertext_base64, c.account_identity_digest
             FROM store_channel_credentials c
             JOIN store_payment_channels p ON p.id = c.channel_id
             WHERE c.channel_id = $1 AND c.adapter_kind = 'stripe'
               AND p.adapter_kind = 'stripe'
             ORDER BY CASE c.status WHEN 'active' THEN 0 ELSE 1 END,
                      c.created_at DESC, c.id DESC",
            vec![channel_id.into()],
        ))
        .await
        .map_err(callback_storage_error)?;
    rows.iter().map(stored_credential).collect()
}

fn stored_credential(row: &QueryResult) -> AppResult<StoredStripeCredential> {
    let version = row
        .try_get::<i32>("", "format_version")
        .map_err(callback_storage_error)
        .and_then(|value| {
            u8::try_from(value).map_err(|error| callback_storage_error(error.to_string()))
        })?;
    Ok(StoredStripeCredential {
        id: row_string(row, "id")?,
        account_identity_digest: row_string(row, "account_identity_digest")?,
        encrypted_secret: EncryptedSecret {
            version,
            key_id: row_string(row, "key_id")?,
            nonce_base64: row_string(row, "nonce_base64")?,
            ciphertext_base64: row_string(row, "ciphertext_base64")?,
        },
    })
}

async fn load_attempt_order_id(state: &AppState, attempt_id: &str) -> AppResult<String> {
    state
        .db_pool
        .read()
        .query_one(state.db_pool.stmt(
            "SELECT order_id FROM store_payment_attempts WHERE id = $1",
            vec![attempt_id.into()],
        ))
        .await
        .map_err(callback_storage_error)?
        .ok_or_else(|| invalid_stripe_event(StripeWebhookError::InvalidPaymentEvent))
        .and_then(|row| row_string(&row, "order_id"))
}

fn account_identity_digest(account_id: &str) -> String {
    Sha256::digest(account_id.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn row_string(row: &QueryResult, column: &str) -> AppResult<String> {
    row.try_get("", column).map_err(callback_storage_error)
}

fn invalid_stripe_event(error: StripeWebhookError) -> AppError {
    AppError::new(
        StatusCode::BAD_REQUEST,
        "invalid_payment_callback",
        "payment callback verification failed",
    )
    .with_internal_message(error.to_string())
}

fn callback_storage_error(error: impl ToString) -> AppError {
    AppError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "payment callback processing failed",
    )
    .with_internal_message(error.to_string())
}

fn map_callback_store_error(error: CallbackStoreError) -> AppError {
    match error {
        CallbackStoreError::InvalidInput | CallbackStoreError::NotFound => {
            invalid_stripe_event(StripeWebhookError::InvalidPaymentEvent)
        }
        CallbackStoreError::Storage(detail) | CallbackStoreError::Fulfillment(detail) => {
            callback_storage_error(detail)
        }
    }
}
