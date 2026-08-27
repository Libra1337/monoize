use crate::app::AppState;
use crate::error::{AppError, AppResult};
use crate::store_billing::exchange_rate::ExchangeRateError;
use crate::store_billing::{
    CreateOrderInput, CreatePaymentChannelInput, CreateProductInput, GenerateRedemptionCodesInput,
    StoreBillingError, StoreSettings, UpdatePaymentChannelInput,
};
use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
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
        StoreBillingError::NoPaymentChannel => (
            StatusCode::CONFLICT,
            "no_payment_channel",
            "no payment channel is enabled",
        ),
        StoreBillingError::OrderNotFound => (
            StatusCode::NOT_FOUND,
            "order_not_found",
            "order was not found",
        ),
        StoreBillingError::OrderCancelled => (
            StatusCode::CONFLICT,
            "order_cancelled",
            "order is cancelled",
        ),
        StoreBillingError::OrderCompleted => (
            StatusCode::CONFLICT,
            "order_completed",
            "order is completed",
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
    let orders = state
        .store_billing
        .list_orders_for_user(&user.id, query.limit())
        .await
        .map_err(map_store_error)?;
    Ok(Json(orders))
}

pub async fn create_store_order(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<CreateOrderInput>, JsonRejection>,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;
    let input = parse_store_json(body)?;
    let rate = current_rate(&state).await?;
    let order = state
        .store_billing
        .create_order(&user.id, input, &rate)
        .await
        .map_err(map_store_error)?;
    Ok((StatusCode::CREATED, Json(order)))
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
    let orders = state
        .store_billing
        .list_orders_admin(query.limit())
        .await
        .map_err(map_store_error)?;
    Ok(Json(orders))
}

pub async fn complete_store_order_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    let order = state
        .store_billing
        .complete_order(&id)
        .await
        .map_err(map_store_error)?;
    Ok(Json(order))
}

pub async fn cancel_store_order_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    let order = state
        .store_billing
        .cancel_order(&id)
        .await
        .map_err(map_store_error)?;
    Ok(Json(order))
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
