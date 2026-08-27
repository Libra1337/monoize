use std::time::Duration;

use axum::Json;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{Path, Request, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use sea_orm::{ConnectionTrait, QueryResult};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::adapters::alipay::{
    AlipayCallbackError, AlipayCredential, verify_alipay_payment_callback,
};
use super::adapters::stripe::{
    StripeCredential, StripeWebhookError, parse_stripe_payment_event, verify_stripe_webhook,
};
use super::adapters::wechat::{
    WechatCallbackError, WechatCredential, verify_wechat_payment_callback,
};
use super::callbacks::{
    ApplyProviderEventInput, CallbackApplyResult, CallbackStoreError, PaymentCallbackStore,
};
use super::crypto::{EncryptedSecret, PaymentKeyRing};
use super::money::Currency;
use crate::app::AppState;
use crate::client_ip::canonical_client_ip_from_headers;
use crate::error::{AppError, AppResult};

const CALLBACK_BODY_MAX_BYTES: usize = 131_072;
const CALLBACK_BODY_TIMEOUT: Duration = Duration::from_secs(5);
const STRIPE_SIGNATURE_TOLERANCE_SECONDS: i64 = 300;
const WECHAT_SIGNATURE_TOLERANCE_SECONDS: i64 = 300;

struct StoredCallbackCredential {
    id: String,
    account_identity_digest: String,
    encrypted_secret: EncryptedSecret,
}

pub async fn store_payment_callback(
    Path(channel_id): Path<String>,
    State(state): State<AppState>,
    request: Request,
) -> AppResult<Response> {
    let headers = request.headers().clone();
    let source_ip = canonical_client_ip_from_headers(&headers);
    if !state.store_callback_limiter.allow(&channel_id, source_ip) {
        return Err(AppError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "callback_rate_limited",
            "payment callback rate limit exceeded",
        ));
    }
    let adapter_kind = load_channel_adapter_kind(&state, &channel_id).await?;
    let body = read_callback_body(request.into_body()).await?;
    let payment_keys = state.payment_keys.as_ref().ok_or_else(|| {
        AppError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "payment_configuration_unavailable",
            "payment callback configuration is unavailable",
        )
    })?;
    if adapter_kind == "alipay" {
        return handle_alipay_callback(
            &state,
            &channel_id,
            &headers,
            &body,
            source_ip,
            payment_keys,
        )
        .await;
    }
    if adapter_kind == "wechat" {
        return handle_wechat_callback(
            &state,
            &channel_id,
            &headers,
            &body,
            source_ip,
            payment_keys,
        )
        .await;
    }
    if adapter_kind != "stripe" {
        return Err(AppError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "payment_callback_unavailable",
            "payment callback adapter is unavailable",
        ));
    }
    let signature = required_header(&headers, "stripe-signature")?;
    let credentials = load_callback_credentials(&state, &channel_id, "stripe").await?;
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
            raw_body: Some(raw_body),
            source_ip: source_ip.map(|ip| ip.to_string()),
            user_agent,
            received_at: Utc::now(),
        })
        .await
        .map_err(map_callback_store_error)?;
    Ok(Json(json!({"received": true})).into_response())
}

async fn handle_alipay_callback(
    state: &AppState,
    channel_id: &str,
    headers: &HeaderMap,
    body: &[u8],
    source_ip: Option<std::net::IpAddr>,
    payment_keys: &PaymentKeyRing,
) -> AppResult<Response> {
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    if !content_type
        .is_some_and(|value| value.eq_ignore_ascii_case("application/x-www-form-urlencoded"))
    {
        return Err(invalid_alipay_event(AlipayCallbackError::InvalidEncoding));
    }
    let credentials = load_callback_credentials(state, channel_id, "alipay").await?;
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
        let Ok(credential) = AlipayCredential::from_json(&plaintext) else {
            continue;
        };
        if account_identity_digest(credential.seller_id()) != stored.account_identity_digest {
            continue;
        }
        usable_credential = true;
        match verify_alipay_payment_callback(&credential, body) {
            Ok(payment) => {
                selected = Some((stored, payment));
                break;
            }
            Err(AlipayCallbackError::Authentication) => {}
            Err(error) => authenticated_error = Some(error),
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
        return Err(invalid_alipay_event(
            authenticated_error.unwrap_or(AlipayCallbackError::Authentication),
        ));
    };
    let (attempt_id, order_id) =
        load_bound_order_attempt(state, channel_id, &stored.id, &payment.order_number).await?;
    let event_row_id = Uuid::new_v4().to_string();
    let raw_body = payment_keys
        .encrypt(
            &format!("store_provider_events:{event_row_id}:raw_body"),
            body,
        )
        .map_err(|error| {
            AppError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "payment_configuration_unavailable",
                "payment callback configuration is unavailable",
            )
            .with_internal_message(error.to_string())
        })?;
    let body_digest = Sha256::digest(body)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let parsed_json = json!({
        "event_id": &payment.provider_event_id,
        "event_kind": "payment_succeeded",
        "trade_no": &payment.provider_transaction_id,
        "order_number": &payment.order_number,
        "amount_minor": &payment.amount_minor,
        "currency": Currency::CNY,
        "account_identity": &stored.account_identity_digest,
    });
    let user_agent = headers
        .get("user-agent")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.chars().take(512).collect::<String>());
    let result = PaymentCallbackStore::new(state.db_pool.clone())
        .apply_verified_payment(ApplyProviderEventInput {
            event_row_id,
            credential_version_id: stored.id,
            provider_event_id: payment.provider_event_id,
            event_kind: "payment_succeeded".to_string(),
            order_id,
            attempt_id,
            provider_transaction_id: payment.provider_transaction_id,
            provider_object_id: payment.order_number.clone(),
            order_number: payment.order_number,
            merchant_account_identity: stored.account_identity_digest,
            amount_minor: payment.amount_minor,
            currency: Currency::CNY,
            body_digest,
            parsed_json,
            raw_body: Some(raw_body),
            source_ip: source_ip.map(|ip| ip.to_string()),
            user_agent,
            received_at: Utc::now(),
        })
        .await
        .map_err(map_callback_store_error)?;
    if result == CallbackApplyResult::ManualReview {
        return Err(invalid_alipay_event(
            AlipayCallbackError::InvalidPaymentEvent,
        ));
    }
    let mut response = Response::new(Body::from("success"));
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    Ok(response)
}

async fn handle_wechat_callback(
    state: &AppState,
    channel_id: &str,
    headers: &HeaderMap,
    body: &[u8],
    source_ip: Option<std::net::IpAddr>,
    payment_keys: &PaymentKeyRing,
) -> AppResult<Response> {
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    if !content_type.is_some_and(|value| value.eq_ignore_ascii_case("application/json")) {
        return Err(invalid_wechat_event(WechatCallbackError::InvalidHeaders));
    }
    let timestamp = required_wechat_header(headers, "wechatpay-timestamp")?;
    let nonce = required_wechat_header(headers, "wechatpay-nonce")?;
    let certificate_serial = required_wechat_header(headers, "wechatpay-serial")?;
    let signature = required_wechat_header(headers, "wechatpay-signature")?;
    let credentials = load_callback_credentials(state, channel_id, "wechat").await?;
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
        let Ok(credential) = WechatCredential::from_json(&plaintext) else {
            continue;
        };
        if account_identity_digest(credential.merchant_id()) != stored.account_identity_digest {
            continue;
        }
        usable_credential = true;
        match verify_wechat_payment_callback(
            &credential,
            &timestamp,
            &nonce,
            &certificate_serial,
            &signature,
            body,
            Utc::now().timestamp(),
            WECHAT_SIGNATURE_TOLERANCE_SECONDS,
        ) {
            Ok(payment) => {
                selected = Some((stored, payment));
                break;
            }
            Err(WechatCallbackError::Authentication) => {}
            Err(error) => authenticated_error = Some(error),
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
        return Err(invalid_wechat_event(
            authenticated_error.unwrap_or(WechatCallbackError::Authentication),
        ));
    };
    let verification_credential_version_id = stored.id;
    let merchant_account_identity = stored.account_identity_digest;
    let (attempt_id, order_id, attempt_credential_version_id) = load_wechat_order_attempt(
        state,
        channel_id,
        &merchant_account_identity,
        &payment.order_number,
    )
    .await?;
    let event_row_id = Uuid::new_v4().to_string();
    let raw_body = payment_keys
        .encrypt(
            &format!("store_provider_events:{event_row_id}:raw_body"),
            body,
        )
        .map_err(|error| {
            AppError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "payment_configuration_unavailable",
                "payment callback configuration is unavailable",
            )
            .with_internal_message(error.to_string())
        })?;
    let body_digest = Sha256::digest(body)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let parsed_json = json!({
        "event_id": &payment.provider_event_id,
        "event_kind": "payment_succeeded",
        "transaction_id": &payment.provider_transaction_id,
        "order_number": &payment.order_number,
        "amount_minor": &payment.amount_minor,
        "currency": Currency::CNY,
        "account_identity": &merchant_account_identity,
        "verification_credential_version_id": &verification_credential_version_id,
    });
    let user_agent = headers
        .get("user-agent")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.chars().take(512).collect::<String>());
    let result = PaymentCallbackStore::new(state.db_pool.clone())
        .apply_verified_payment(ApplyProviderEventInput {
            event_row_id,
            credential_version_id: attempt_credential_version_id,
            provider_event_id: payment.provider_event_id,
            event_kind: "payment_succeeded".to_string(),
            order_id,
            attempt_id,
            provider_transaction_id: payment.provider_transaction_id,
            provider_object_id: payment.order_number.clone(),
            order_number: payment.order_number,
            merchant_account_identity,
            amount_minor: payment.amount_minor,
            currency: Currency::CNY,
            body_digest,
            parsed_json,
            raw_body: Some(raw_body),
            source_ip: source_ip.map(|ip| ip.to_string()),
            user_agent,
            received_at: Utc::now(),
        })
        .await
        .map_err(map_callback_store_error)?;
    if result == CallbackApplyResult::ManualReview {
        return Err(invalid_wechat_event(
            WechatCallbackError::InvalidPaymentEvent,
        ));
    }
    Ok(Json(json!({"code":"SUCCESS","message":"成功"})).into_response())
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

async fn load_channel_adapter_kind(state: &AppState, channel_id: &str) -> AppResult<String> {
    state
        .db_pool
        .read()
        .query_one(state.db_pool.stmt(
            "SELECT adapter_kind FROM store_payment_channels WHERE id = $1",
            vec![channel_id.into()],
        ))
        .await
        .map_err(callback_storage_error)?
        .ok_or_else(|| {
            AppError::new(
                StatusCode::NOT_FOUND,
                "payment_channel_not_found",
                "payment Channel was not found",
            )
        })
        .and_then(|row| row_string(&row, "adapter_kind"))
}

async fn load_callback_credentials(
    state: &AppState,
    channel_id: &str,
    adapter_kind: &str,
) -> AppResult<Vec<StoredCallbackCredential>> {
    let rows = state
        .db_pool
        .read()
        .query_all(state.db_pool.stmt(
            "SELECT c.id, c.format_version, c.key_id, c.nonce_base64,
                    c.ciphertext_base64, c.account_identity_digest
             FROM store_channel_credentials c
             JOIN store_payment_channels p ON p.id = c.channel_id
             WHERE c.channel_id = $1 AND c.adapter_kind = $2
               AND p.adapter_kind = $2
             ORDER BY CASE c.status WHEN 'active' THEN 0 ELSE 1 END,
                      c.created_at DESC, c.id DESC",
            vec![channel_id.into(), adapter_kind.into()],
        ))
        .await
        .map_err(callback_storage_error)?;
    rows.iter().map(stored_credential).collect()
}

fn stored_credential(row: &QueryResult) -> AppResult<StoredCallbackCredential> {
    let version = row
        .try_get::<i32>("", "format_version")
        .map_err(callback_storage_error)
        .and_then(|value| {
            u8::try_from(value).map_err(|error| callback_storage_error(error.to_string()))
        })?;
    Ok(StoredCallbackCredential {
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

async fn load_bound_order_attempt(
    state: &AppState,
    channel_id: &str,
    credential_version_id: &str,
    order_number: &str,
) -> AppResult<(String, String)> {
    let rows = state
        .db_pool
        .read()
        .query_all(state.db_pool.stmt(
            "SELECT a.id AS attempt_id, a.order_id
             FROM store_payment_attempts a
             JOIN store_orders o ON o.id = a.order_id
             WHERE a.channel_id = $1 AND a.credential_version_id = $2
               AND (a.provider_object_id = $3 OR a.provider_object_id IS NULL)
               AND o.order_number = $3
             ORDER BY a.created_at DESC, a.id DESC
             LIMIT 2",
            vec![
                channel_id.into(),
                credential_version_id.into(),
                order_number.into(),
            ],
        ))
        .await
        .map_err(callback_storage_error)?;
    if rows.len() != 1 {
        return Err(invalid_alipay_event(
            AlipayCallbackError::InvalidPaymentEvent,
        ));
    }
    Ok((
        row_string(&rows[0], "attempt_id")?,
        row_string(&rows[0], "order_id")?,
    ))
}

async fn load_wechat_order_attempt(
    state: &AppState,
    channel_id: &str,
    merchant_account_identity: &str,
    order_number: &str,
) -> AppResult<(String, String, String)> {
    let rows = state
        .db_pool
        .read()
        .query_all(state.db_pool.stmt(
            "SELECT a.id AS attempt_id, a.order_id, a.credential_version_id
             FROM store_payment_attempts a
             JOIN store_orders o ON o.id = a.order_id
             WHERE a.channel_id = $1 AND a.adapter_kind = 'wechat'
               AND a.merchant_account_identity = $2
               AND (a.provider_object_id = $3 OR a.provider_object_id IS NULL)
               AND o.order_number = $3
             ORDER BY a.created_at DESC, a.id DESC
             LIMIT 2",
            vec![
                channel_id.into(),
                merchant_account_identity.into(),
                order_number.into(),
            ],
        ))
        .await
        .map_err(callback_storage_error)?;
    if rows.len() != 1 {
        return Err(invalid_wechat_event(
            WechatCallbackError::InvalidPaymentEvent,
        ));
    }
    Ok((
        row_string(&rows[0], "attempt_id")?,
        row_string(&rows[0], "order_id")?,
        row_string(&rows[0], "credential_version_id")?,
    ))
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

fn invalid_alipay_event(error: AlipayCallbackError) -> AppError {
    AppError::new(
        StatusCode::BAD_REQUEST,
        "invalid_payment_callback",
        "payment callback verification failed",
    )
    .with_internal_message(error.to_string())
}

fn required_wechat_header(headers: &HeaderMap, name: &'static str) -> AppResult<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| invalid_wechat_event(WechatCallbackError::InvalidHeaders))
}

fn invalid_wechat_event(error: WechatCallbackError) -> AppError {
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
