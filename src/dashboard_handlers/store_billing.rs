use crate::app::AppState;
use crate::error::{AppError, AppResult};
use crate::store_billing::exchange_rate::ExchangeRateError;
use crate::store_billing::order::{
    CreatePaymentAttemptInput, CreatePaymentOrderInput, PaymentOrderError, PaymentOrderStore,
};
use crate::store_billing::{
    CreatePaymentChannelInput, CreateProductInput, Currency, GenerateRedemptionCodesInput,
    PAYMENT_ICON_MAX_BYTES, StoreBillingError, StoreSettings, UpdatePaymentChannelInput,
};
use axum::Json;
use axum::body::Body;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Multipart, Path, Query, State};
use axum::http::header::{CONTENT_SECURITY_POLICY, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

use super::session_helpers::{get_current_user, require_admin};

const DEFAULT_PAGE_SIZE: u64 = 100;
const MAX_PAGE_SIZE: u64 = 100;

#[derive(Debug, Default, Deserialize)]
pub struct StoreListQuery {
    pub limit: Option<u64>,
}

impl StoreListQuery {
    fn limit(&self) -> u64 {
        self.limit.unwrap_or(DEFAULT_PAGE_SIZE).min(MAX_PAGE_SIZE)
    }
}

#[derive(Debug, Deserialize)]
pub struct RedeemRequest {
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct CreatePaymentAttemptRequest {
    pub expected_payment_method: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePaymentOrderRequest {
    pub product_id: String,
    pub payment_channel_id: String,
    pub payment_currency: Currency,
    pub custom_recharge_minor: Option<String>,
}

fn map_store_error(error: StoreBillingError) -> AppError {
    let (status, code, message) = match error {
        StoreBillingError::InvalidInput => (
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "invalid Store input",
        ),
        StoreBillingError::InvalidAmount => (
            StatusCode::BAD_REQUEST,
            "invalid_amount",
            "invalid monetary amount",
        ),
        StoreBillingError::AmountOverflow => (
            StatusCode::CONFLICT,
            "amount_overflow",
            "monetary amount overflow",
        ),
        StoreBillingError::InvalidExchangeRate => (
            StatusCode::SERVICE_UNAVAILABLE,
            "exchange_rate_unavailable",
            "no valid exchange rate is available",
        ),
        StoreBillingError::ProductNotAvailable => (
            StatusCode::NOT_FOUND,
            "product_not_available",
            "product is not available",
        ),
        StoreBillingError::InvalidPaymentChannel => (
            StatusCode::BAD_REQUEST,
            "invalid_payment_channel",
            "payment channel is invalid",
        ),
        StoreBillingError::InvalidIcon => (
            StatusCode::BAD_REQUEST,
            "invalid_icon",
            "payment channel icon is invalid",
        ),
        StoreBillingError::InvalidRedemptionCode => (
            StatusCode::NOT_FOUND,
            "invalid_redemption_code",
            "redemption code is invalid",
        ),
        StoreBillingError::RedemptionCodeExpired => (
            StatusCode::CONFLICT,
            "redemption_code_expired",
            "redemption code is expired",
        ),
        StoreBillingError::RedemptionCodeUsed => (
            StatusCode::CONFLICT,
            "redemption_code_used",
            "redemption code is used",
        ),
        StoreBillingError::NotFound => (
            StatusCode::NOT_FOUND,
            "not_found",
            "Store record was not found",
        ),
        StoreBillingError::Conflict => (StatusCode::CONFLICT, "conflict", "Store record is in use"),
        StoreBillingError::Storage(detail) => {
            return AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Store operation failed",
            )
            .with_internal_message(detail);
        }
    };
    AppError::new(status, code, message)
}

fn map_payment_order_error(error: PaymentOrderError) -> AppError {
    let (status, code, message) = match error {
        PaymentOrderError::InvalidInput => (
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "invalid payment order input",
        ),
        PaymentOrderError::InvalidAmount => (
            StatusCode::BAD_REQUEST,
            "invalid_amount",
            "invalid payment amount",
        ),
        PaymentOrderError::InvalidExchangeRate => (
            StatusCode::SERVICE_UNAVAILABLE,
            "exchange_rate_unavailable",
            "no valid exchange rate is available",
        ),
        PaymentOrderError::ProductUnavailable => (
            StatusCode::NOT_FOUND,
            "product_not_available",
            "product is not available",
        ),
        PaymentOrderError::ChannelUnavailable => (
            StatusCode::CONFLICT,
            "payment_channel_unavailable",
            "payment Channel is not available",
        ),
        PaymentOrderError::OrderNotFound => (
            StatusCode::NOT_FOUND,
            "order_not_found",
            "order was not found",
        ),
        PaymentOrderError::IdempotencyConflict => (
            StatusCode::CONFLICT,
            "idempotency_conflict",
            "idempotency key was used with different input",
        ),
        PaymentOrderError::CreationRateLimited => (
            StatusCode::TOO_MANY_REQUESTS,
            "order_rate_limited",
            "too many Store orders were created",
        ),
        PaymentOrderError::OpenOrderLimit => (
            StatusCode::TOO_MANY_REQUESTS,
            "open_order_limit",
            "too many unpaid Store orders exist",
        ),
        PaymentOrderError::ActiveAttemptExists => (
            StatusCode::CONFLICT,
            "active_payment_attempt",
            "an active payment attempt already exists",
        ),
        PaymentOrderError::OrderNotPayable => (
            StatusCode::CONFLICT,
            "order_not_payable",
            "order cannot accept a payment attempt",
        ),
        PaymentOrderError::AmountOverflow => (
            StatusCode::CONFLICT,
            "amount_overflow",
            "payment amount overflow",
        ),
        PaymentOrderError::Storage(detail) => {
            return AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "payment order operation failed",
            )
            .with_internal_message(detail);
        }
    };
    AppError::new(status, code, message)
}

fn required_idempotency_key(headers: &HeaderMap) -> AppResult<String> {
    let value = headers
        .get("Idempotency-Key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AppError::new(
                StatusCode::BAD_REQUEST,
                "missing_idempotency_key",
                "Idempotency-Key header is required",
            )
        })?;
    Ok(value.to_string())
}

fn parse_store_json<T>(body: Result<Json<T>, JsonRejection>) -> AppResult<T> {
    match body {
        Ok(Json(value)) => Ok(value),
        Err(rejection) => {
            let detail = rejection.body_text();
            if detail.contains("invalid_currency") {
                Err(AppError::new(
                    StatusCode::BAD_REQUEST,
                    "invalid_currency",
                    "currency must be CNY or USD",
                ))
            } else {
                Err(AppError::new(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "invalid JSON body",
                )
                .with_internal_message(detail))
            }
        }
    }
}

fn parse_store_query<T>(query: Result<Query<T>, QueryRejection>) -> AppResult<T> {
    query.map(|Query(value)| value).map_err(|rejection| {
        AppError::new(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "invalid query parameters",
        )
        .with_internal_message(rejection.body_text())
    })
}

fn map_exchange_rate_error(error: ExchangeRateError) -> AppError {
    match error {
        ExchangeRateError::Unavailable
        | ExchangeRateError::InvalidPayload(_)
        | ExchangeRateError::Request(_) => AppError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "exchange_rate_unavailable",
            "no valid exchange rate is available",
        ),
        ExchangeRateError::Storage(detail) => AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "exchange rate storage failed",
        )
        .with_internal_message(detail),
    }
}

async fn current_rate(state: &AppState) -> AppResult<crate::store_billing::ExchangeRateSnapshot> {
    state
        .exchange_rate_service
        .refresh_if_due(Utc::now())
        .await
        .map_err(map_exchange_rate_error)
}

pub async fn get_store_catalog(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    get_current_user(&headers, &state).await?;
    let catalog = state
        .store_billing
        .catalog()
        .await
        .map_err(map_store_error)?;
    Ok(Json(catalog))
}

pub async fn get_store_exchange_rate(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    get_current_user(&headers, &state).await?;
    Ok(Json(current_rate(&state).await?))
}

pub async fn get_store_entitlement(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;
    let entitlement = state
        .store_billing
        .current_entitlement(&user.id)
        .await
        .map_err(map_store_error)?;
    Ok(Json(entitlement))
}

pub async fn list_store_orders(
    State(state): State<AppState>,
    headers: HeaderMap,
    query: Result<Query<StoreListQuery>, QueryRejection>,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;
    let query = parse_store_query(query)?;
    let orders = PaymentOrderStore::new(state.db_pool.clone())
        .list_orders_for_user(&user.id, query.limit())
        .await
        .map_err(map_payment_order_error)?;
    Ok(Json(orders))
}

pub async fn create_store_order(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<CreatePaymentOrderRequest>, JsonRejection>,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;
    let input = parse_store_json(body)?;
    let idempotency_key = required_idempotency_key(&headers)?;
    let rate = current_rate(&state).await?;
    let store = PaymentOrderStore::new(state.db_pool.clone());
    let replayed = store
        .find_order_by_creation_key(&user.id, &idempotency_key)
        .await
        .map_err(map_payment_order_error)?
        .is_some();
    let order = store
        .create_order(
            &user.id,
            CreatePaymentOrderInput {
                idempotency_key,
                product_id: input.product_id,
                payment_channel_id: input.payment_channel_id,
                payment_currency: input.payment_currency,
                custom_recharge_minor: input.custom_recharge_minor,
            },
            &rate,
        )
        .await
        .map_err(map_payment_order_error)?;
    Ok((
        if replayed {
            StatusCode::OK
        } else {
            StatusCode::CREATED
        },
        Json(order),
    ))
}

pub async fn get_store_order(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;
    let order = PaymentOrderStore::new(state.db_pool.clone())
        .get_order_for_user(&user.id, &id)
        .await
        .map_err(map_payment_order_error)?
        .ok_or_else(|| map_payment_order_error(PaymentOrderError::OrderNotFound))?;
    Ok(Json(order))
}

pub async fn create_store_payment_attempt(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Result<Json<CreatePaymentAttemptRequest>, JsonRejection>,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;
    let request = parse_store_json(body)?;
    let input = CreatePaymentAttemptInput {
        idempotency_key: required_idempotency_key(&headers)?,
        expected_payment_method: request.expected_payment_method,
    };
    let attempt = PaymentOrderStore::new(state.db_pool.clone())
        .create_attempt(&user.id, &id, input)
        .await
        .map_err(map_payment_order_error)?;
    Ok((StatusCode::CREATED, Json(attempt)))
}

pub async fn redeem_store_code(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<RedeemRequest>, JsonRejection>,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;
    let input = parse_store_json(body)?;
    let rate = match state.exchange_rate_service.current().await {
        Ok(rate) => Some(rate),
        Err(ExchangeRateError::Unavailable) => None,
        Err(error) => return Err(map_exchange_rate_error(error)),
    };
    let record = state
        .store_billing
        .redeem(&user.id, &input.code, rate.as_ref())
        .await
        .map_err(map_store_error)?;
    Ok(Json(record))
}

pub async fn list_store_products_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    let products = state
        .store_billing
        .list_products_admin()
        .await
        .map_err(map_store_error)?;
    Ok(Json(products))
}

pub async fn create_store_product_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<CreateProductInput>, JsonRejection>,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    let input = parse_store_json(body)?;
    let product = state
        .store_billing
        .create_product(input)
        .await
        .map_err(map_store_error)?;
    Ok((StatusCode::CREATED, Json(product)))
}

pub async fn update_store_product_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Result<Json<CreateProductInput>, JsonRejection>,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    let input = parse_store_json(body)?;
    let product = state
        .store_billing
        .update_product(&id, input)
        .await
        .map_err(map_store_error)?;
    Ok(Json(product))
}

pub async fn delete_store_product_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    state
        .store_billing
        .delete_product(&id)
        .await
        .map_err(map_store_error)?;
    Ok(Json(json!({ "success": true })))
}

pub async fn list_store_payment_channels_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    let channels = state
        .store_billing
        .list_payment_channels_admin()
        .await
        .map_err(map_store_error)?;
    Ok(Json(channels))
}

pub async fn create_store_payment_channel_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<CreatePaymentChannelInput>, JsonRejection>,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    let input = parse_store_json(body)?;
    let channel = state
        .store_billing
        .create_payment_channel(input)
        .await
        .map_err(map_store_error)?;
    Ok((StatusCode::CREATED, Json(channel)))
}

pub async fn upload_store_payment_icon_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    let mut content = None;
    while let Some(mut field) = multipart.next_field().await.map_err(|error| {
        AppError::new(
            StatusCode::BAD_REQUEST,
            "invalid_icon",
            "payment channel icon is invalid",
        )
        .with_internal_message(error.to_string())
    })? {
        if field.name() != Some("file") {
            return Err(invalid_icon_error(
                "multipart contains a field other than file",
            ));
        }
        if content.is_some() {
            return Err(invalid_icon_error(
                "multipart contains more than one file field",
            ));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = field.chunk().await.map_err(|error| {
            invalid_icon_error(format!("failed to read multipart file: {error}"))
        })? {
            if bytes
                .len()
                .checked_add(chunk.len())
                .is_none_or(|length| length > PAYMENT_ICON_MAX_BYTES)
            {
                return Err(invalid_icon_error("multipart file exceeds 2 MiB"));
            }
            bytes.extend_from_slice(&chunk);
        }
        content = Some(bytes);
    }
    let content = content.ok_or_else(|| invalid_icon_error("multipart file field is missing"))?;
    let icon = state
        .store_billing
        .save_payment_icon(content)
        .await
        .map_err(map_store_error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "url": format!("/api/dashboard/store/icons/{}", icon.id),
        })),
    ))
}

pub async fn get_store_payment_icon(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Response> {
    get_current_user(&headers, &state).await?;
    let icon = state
        .store_billing
        .get_payment_icon(&id)
        .await
        .map_err(map_store_error)?
        .ok_or_else(|| map_store_error(StoreBillingError::NotFound))?;
    let content_type = HeaderValue::from_str(&icon.content_type).map_err(|error| {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "stored payment channel icon is invalid",
        )
        .with_internal_message(error.to_string())
    })?;
    let mut response = Response::new(Body::from(icon.content));
    response.headers_mut().insert(CONTENT_TYPE, content_type);
    response
        .headers_mut()
        .insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    if icon.content_type == "image/svg+xml" {
        response.headers_mut().insert(
            CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("sandbox; default-src 'none'"),
        );
    }
    Ok(response)
}

fn invalid_icon_error(detail: impl Into<String>) -> AppError {
    AppError::new(
        StatusCode::BAD_REQUEST,
        "invalid_icon",
        "payment channel icon is invalid",
    )
    .with_internal_message(detail)
}

pub async fn update_store_payment_channel_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Result<Json<UpdatePaymentChannelInput>, JsonRejection>,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    let input = parse_store_json(body)?;
    let channel = state
        .store_billing
        .update_payment_channel(&id, input)
        .await
        .map_err(map_store_error)?;
    Ok(Json(channel))
}

pub async fn delete_store_payment_channel_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    state
        .store_billing
        .delete_payment_channel(&id)
        .await
        .map_err(map_store_error)?;
    Ok(Json(json!({ "success": true })))
}

pub async fn list_all_store_orders_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    query: Result<Query<StoreListQuery>, QueryRejection>,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    let query = parse_store_query(query)?;
    let orders = PaymentOrderStore::new(state.db_pool.clone())
        .list_orders_admin(query.limit())
        .await
        .map_err(map_payment_order_error)?;
    Ok(Json(orders))
}

pub async fn list_store_redemption_codes_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    query: Result<Query<StoreListQuery>, QueryRejection>,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    let query = parse_store_query(query)?;
    let codes = state
        .store_billing
        .list_redemption_codes_admin(query.limit())
        .await
        .map_err(map_store_error)?;
    Ok(Json(codes))
}

pub async fn generate_store_redemption_codes_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<GenerateRedemptionCodesInput>, JsonRejection>,
) -> AppResult<impl IntoResponse> {
    let admin = require_admin(&headers, &state).await?;
    let input = parse_store_json(body)?;
    let codes = state
        .store_billing
        .generate_redemption_codes(&admin.id, input)
        .await
        .map_err(map_store_error)?;
    Ok((StatusCode::CREATED, Json(codes)))
}

pub async fn get_store_settings_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    let settings = state
        .store_billing
        .get_settings()
        .await
        .map_err(map_store_error)?;
    Ok(Json(settings))
}

pub async fn update_store_settings_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<StoreSettings>, JsonRejection>,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    let input = parse_store_json(body)?;
    let settings = state
        .store_billing
        .update_settings(input)
        .await
        .map_err(map_store_error)?;
    Ok(Json(settings))
}
