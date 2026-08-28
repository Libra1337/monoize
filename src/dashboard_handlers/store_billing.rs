use crate::app::AppState;
use crate::error::{AppError, AppResult};
use crate::store_billing::callbacks::{PaymentCallbackStore, ReprocessProviderEventError};
use crate::store_billing::checkout::{CheckoutError, CheckoutService};
use crate::store_billing::credentials::{CredentialStore, CredentialStoreError};
use crate::store_billing::exchange_rate::ExchangeRateError;
use crate::store_billing::governance::PaymentGovernanceStore;
use crate::store_billing::operations::{AdminOrderOperationError, AdminOrderOperations};
use crate::store_billing::order::{
    CreatePaymentAttemptInput, CreatePaymentOrderInput, PaymentOrderError, PaymentOrderStore,
};
use crate::store_billing::reauth::{ReauthError, ReauthStore};
use crate::store_billing::recovery::{RecoveryError, RecoveryStore};
use crate::store_billing::redemption::{
    RedemptionAccessAction, RedemptionAuditContext, RevealRedemptionInput,
};
use crate::store_billing::refund_operations::{RefundOperations, RefundOperationsError};
use crate::store_billing::{
    ConfirmStoreComplianceInput, CreatePaymentChannelInput, CreateProductInput, Currency,
    GenerateRedemptionCodesInput, MerchantCapabilityKind, PAYMENT_ICON_MAX_BYTES,
    PutStoreMerchantCapabilityInput, StoreBillingError, StoreSettings, UpdatePaymentChannelInput,
};
use axum::Json;
use axum::body::Body;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Multipart, Path, Query, State};
use axum::http::header::{
    AUTHORIZATION, CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_SECURITY_POLICY, CONTENT_TYPE,
    ORIGIN, PRAGMA, REFERRER_POLICY, USER_AGENT, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::users::UserStore;

use super::session_helpers::{extract_session_token, get_current_user, require_admin};

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

#[derive(Debug, Deserialize)]
pub struct StoreReauthRequest {
    pub current_password: String,
    pub scope: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminOrderAttemptRequest {
    pub attempt_id: String,
}

#[derive(Debug, Deserialize)]
pub struct RedemptionCodeIdsRequest {
    pub code_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyStoreMutation {}

fn no_store_headers() -> [(axum::http::HeaderName, HeaderValue); 4] {
    [
        (CACHE_CONTROL, HeaderValue::from_static("no-store")),
        (PRAGMA, HeaderValue::from_static("no-cache")),
        (REFERRER_POLICY, HeaderValue::from_static("no-referrer")),
        (X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")),
    ]
}

fn apply_no_store_headers(response: &mut Response) {
    for (name, value) in no_store_headers() {
        response.headers_mut().insert(name, value);
    }
}

fn no_store_response<T: IntoResponse>(result: AppResult<T>) -> Response {
    let mut response = match result {
        Ok(value) => value.into_response(),
        Err(error) => error.into_response(),
    };
    apply_no_store_headers(&mut response);
    response
}

fn is_admin_order_operation_path(path: &str) -> bool {
    let segments = path.trim_matches('/').split('/').collect::<Vec<_>>();
    let direct_order_operation = segments.len() >= 6
        && segments[segments.len() - 6..segments.len() - 2]
            == ["dashboard", "store", "admin", "orders"]
        && !segments[segments.len() - 2].is_empty()
        && matches!(segments[segments.len() - 1], "query" | "close" | "refunds");
    let refund_query = segments.len() >= 8
        && segments[segments.len() - 8..segments.len() - 4]
            == ["dashboard", "store", "admin", "orders"]
        && !segments[segments.len() - 4].is_empty()
        && segments[segments.len() - 3] == "refunds"
        && !segments[segments.len() - 2].is_empty()
        && segments[segments.len() - 1] == "query";
    let provider_event_reprocess = segments.len() >= 6
        && segments[segments.len() - 5..segments.len() - 2]
            == ["store", "admin", "provider-events"]
        && !segments[segments.len() - 2].is_empty()
        && segments[segments.len() - 1] == "reprocess";
    direct_order_operation || refund_query || provider_event_reprocess
}

async fn require_refund_access(
    headers: &HeaderMap,
    state: &AppState,
    admin_id: &str,
) -> AppResult<()> {
    let session_token = extract_session_token(headers).ok_or_else(|| {
        AppError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "missing dashboard session",
        )
    })?;
    let grant_token = headers
        .get("X-Store-Reauth-Token")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| map_reauth_error(ReauthError::InvalidGrant))?;
    ReauthStore::new(state.db_pool.clone())
        .verify(admin_id, &session_token, grant_token, "refund")
        .await
        .map_err(map_reauth_error)
}

async fn require_reprocess_access(
    headers: &HeaderMap,
    state: &AppState,
    admin_id: &str,
) -> AppResult<()> {
    let session_token = extract_session_token(headers).ok_or_else(|| {
        AppError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "missing dashboard session",
        )
    })?;
    let grant_token = headers
        .get("X-Store-Reauth-Token")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| map_reauth_error(ReauthError::InvalidGrant))?;
    ReauthStore::new(state.db_pool.clone())
        .verify(admin_id, &session_token, grant_token, "reprocess")
        .await
        .map_err(map_reauth_error)
}

async fn require_redemption_access(
    headers: &HeaderMap,
    state: &AppState,
    admin_id: &str,
) -> AppResult<()> {
    let session_token = extract_session_token(headers).ok_or_else(|| {
        AppError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "missing dashboard session",
        )
    })?;
    let grant_token = headers
        .get("X-Store-Reauth-Token")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| map_reauth_error(ReauthError::InvalidGrant))?;
    ReauthStore::new(state.db_pool.clone())
        .verify(admin_id, &session_token, grant_token, "redemption_access")
        .await
        .map_err(map_reauth_error)
}

fn redemption_audit_context(headers: &HeaderMap, admin_id: &str) -> RedemptionAuditContext {
    RedemptionAuditContext {
        admin_user_id: admin_id.to_string(),
        source_ip: crate::client_ip::canonical_client_ip_from_headers(headers)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        user_agent: headers
            .get(USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("unknown")
            .chars()
            .take(512)
            .collect(),
    }
}

fn require_store_mutation(headers: &HeaderMap, state: &AppState) -> AppResult<()> {
    let uses_bearer = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("Bearer "));
    let uses_cookie = headers
        .get("cookie")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|cookies| {
            cookies.split(';').map(str::trim).any(|cookie| {
                cookie
                    .strip_prefix("monoize_session=")
                    .is_some_and(|value| !value.is_empty())
            })
        });
    if uses_cookie && !uses_bearer {
        let expected = state
            .payment_public_origin
            .as_ref()
            .map(|origin| origin.origin().ascii_serialization());
        let actual = headers.get(ORIGIN).and_then(|value| value.to_str().ok());
        if !matches!((expected.as_deref(), actual), (Some(expected), Some(actual)) if expected == actual)
        {
            return Err(AppError::new(
                StatusCode::FORBIDDEN,
                "store_origin_invalid",
                "Store mutation origin is invalid",
            ));
        }
    }
    state.store_billing.require_write().map_err(map_store_error)
}

pub(crate) async fn store_mutation_guard(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let admin_order_operation = is_admin_order_operation_path(request.uri().path());
    if let Err(error) = require_store_mutation(request.headers(), &state) {
        let mut response = error.into_response();
        if admin_order_operation {
            apply_no_store_headers(&mut response);
        }
        return response;
    }
    next.run(request).await
}

fn map_reauth_error(error: ReauthError) -> AppError {
    match error {
        ReauthError::InvalidScope => AppError::new(
            StatusCode::BAD_REQUEST,
            "invalid_reauth_scope",
            "reauthentication scope is invalid",
        ),
        ReauthError::InvalidGrant => AppError::new(
            StatusCode::FORBIDDEN,
            "invalid_reauth_grant",
            "reauthentication grant is invalid or expired",
        ),
        ReauthError::Storage(detail) => AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "reauthentication failed",
        )
        .with_internal_message(detail),
    }
}

fn map_credential_store_error(error: CredentialStoreError) -> AppError {
    match error {
        CredentialStoreError::ChannelNotFound => AppError::new(
            StatusCode::NOT_FOUND,
            "payment_channel_not_found",
            "payment Channel was not found",
        ),
        CredentialStoreError::InvalidCredential => AppError::new(
            StatusCode::BAD_REQUEST,
            "invalid_payment_credential",
            "payment credential is invalid",
        ),
        CredentialStoreError::EncryptionUnavailable => AppError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "payment_configuration_unavailable",
            "payment credential encryption is unavailable",
        ),
        CredentialStoreError::Storage(detail) => AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "payment credential storage failed",
        )
        .with_internal_message(detail),
    }
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
        StoreBillingError::RedemptionCodeRevoked => (
            StatusCode::CONFLICT,
            "redemption_code_revoked",
            "redemption code is revoked",
        ),
        StoreBillingError::EncryptionUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "payment_configuration_unavailable",
            "redemption-code encryption is unavailable",
        ),
        StoreBillingError::RedemptionRateLimited => (
            StatusCode::TOO_MANY_REQUESTS,
            "redemption_rate_limited",
            "too many redemption attempts",
        ),
        StoreBillingError::RedemptionCooldown => (
            StatusCode::TOO_MANY_REQUESTS,
            "redemption_cooldown",
            "redemption attempts are temporarily blocked",
        ),
        StoreBillingError::PaymentHold => (
            StatusCode::LOCKED,
            "payment_hold",
            "payment hold blocks Store mutations",
        ),
        StoreBillingError::NotFound => (
            StatusCode::NOT_FOUND,
            "not_found",
            "Store record was not found",
        ),
        StoreBillingError::Conflict => (StatusCode::CONFLICT, "conflict", "Store record is in use"),
        StoreBillingError::WriteRejected => (
            StatusCode::SERVICE_UNAVAILABLE,
            "store_write_rejected",
            "Store writes require the Primary node",
        ),
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
        PaymentOrderError::PaymentHold => (
            StatusCode::LOCKED,
            "payment_hold",
            "payment hold blocks Store purchases",
        ),
        PaymentOrderError::ActiveAttemptExists => (
            StatusCode::CONFLICT,
            "active_payment_attempt",
            "an active payment attempt already exists",
        ),
        PaymentOrderError::ProviderQueryRequired => (
            StatusCode::CONFLICT,
            "provider_query_required",
            "provider state must be queried before another payment attempt",
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

fn map_admin_order_operation_error(error: AdminOrderOperationError) -> AppError {
    let (status, code, message) = match error {
        AdminOrderOperationError::InvalidInput => (
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Admin order operation input is invalid",
        ),
        AdminOrderOperationError::NotFound => (
            StatusCode::NOT_FOUND,
            "order_not_found",
            "order was not found",
        ),
        AdminOrderOperationError::LegacyClosed | AdminOrderOperationError::OrderNotPayable => (
            StatusCode::CONFLICT,
            "order_not_payable",
            "order does not allow this operation",
        ),
        AdminOrderOperationError::Ambiguous => (
            StatusCode::CONFLICT,
            "payment_provider_ambiguous",
            "Provider payment state is ambiguous",
        ),
        AdminOrderOperationError::ConfigurationUnavailable => (
            StatusCode::CONFLICT,
            "payment_configuration_unavailable",
            "historical payment configuration is unavailable",
        ),
        AdminOrderOperationError::ProviderQueryFailed => (
            StatusCode::BAD_GATEWAY,
            "payment_provider_query_failed",
            "Provider payment query failed",
        ),
        AdminOrderOperationError::ProjectionFailed => (
            StatusCode::CONFLICT,
            "payment_projection_failed",
            "verified Provider payment could not be projected",
        ),
        AdminOrderOperationError::Storage(detail) => {
            return AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Admin order operation failed",
            )
            .with_internal_message(detail);
        }
    };
    AppError::new(status, code, message)
}

fn map_reprocess_provider_event_error(error: ReprocessProviderEventError) -> AppError {
    let (status, code, message) = match error {
        ReprocessProviderEventError::InvalidInput => (
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Provider event reprocess input is invalid",
        ),
        ReprocessProviderEventError::NotFound => (
            StatusCode::NOT_FOUND,
            "event_not_found",
            "Provider event was not found",
        ),
        ReprocessProviderEventError::NotReprocessable => (
            StatusCode::CONFLICT,
            "event_not_reprocessable",
            "Provider event cannot be reprocessed",
        ),
        ReprocessProviderEventError::ManualReview => (
            StatusCode::CONFLICT,
            "projection_manual_review",
            "Provider event requires manual review",
        ),
        ReprocessProviderEventError::ProviderQueryRequired => (
            StatusCode::CONFLICT,
            "provider_query_required",
            "fresh Provider payment evidence is required",
        ),
        ReprocessProviderEventError::IdentityConflict => (
            StatusCode::CONFLICT,
            "event_identity_conflict",
            "Provider event identity conflicts with stored evidence",
        ),
        ReprocessProviderEventError::Storage(detail)
        | ReprocessProviderEventError::Fulfillment(detail) => {
            return AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Provider event reprocess failed",
            )
            .with_internal_message(detail);
        }
    };
    AppError::new(status, code, message)
}

fn map_refund_operations_error(error: RefundOperationsError) -> AppError {
    let (status, code, message) = match error {
        RefundOperationsError::InvalidInput => (
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "refund operation input is invalid",
        ),
        RefundOperationsError::NotFound => (
            StatusCode::NOT_FOUND,
            "refund_not_found",
            "refund was not found",
        ),
        RefundOperationsError::OrderNotRefundable => (
            StatusCode::CONFLICT,
            "order_not_refundable",
            "order is not refundable",
        ),
        RefundOperationsError::IdempotencyConflict => (
            StatusCode::CONFLICT,
            "idempotency_conflict",
            "idempotency key was used with another order",
        ),
        RefundOperationsError::InsufficientBalance => (
            StatusCode::CONFLICT,
            "refund_reserve_unavailable",
            "refund reserve is unavailable",
        ),
        RefundOperationsError::ConfigurationUnavailable => (
            StatusCode::CONFLICT,
            "payment_configuration_unavailable",
            "historical payment configuration is unavailable",
        ),
        RefundOperationsError::Storage(detail) => {
            return AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "refund operation failed",
            )
            .with_internal_message(detail);
        }
    };
    AppError::new(status, code, message)
}

fn map_refund_read_error(error: RecoveryError) -> AppError {
    match error {
        RecoveryError::InvalidInput => {
            map_refund_operations_error(RefundOperationsError::InvalidInput)
        }
        RecoveryError::NotFound => map_refund_operations_error(RefundOperationsError::NotFound),
        RecoveryError::Storage(detail) => {
            map_refund_operations_error(RefundOperationsError::Storage(detail))
        }
        other => map_refund_operations_error(RefundOperationsError::Storage(other.to_string())),
    }
}

fn map_checkout_error(error: CheckoutError) -> AppError {
    match error {
        CheckoutError::ConfigurationUnavailable => AppError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "payment_configuration_unavailable",
            "payment checkout configuration is unavailable",
        ),
        CheckoutError::ProviderAmbiguous => AppError::new(
            StatusCode::BAD_GATEWAY,
            "payment_provider_ambiguous",
            "payment provider state is ambiguous",
        ),
        CheckoutError::ProviderRejected => AppError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "payment_provider_rejected",
            "payment provider rejected checkout",
        ),
        CheckoutError::Order(error) => map_payment_order_error(error),
    }
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
    let store = PaymentOrderStore::new(state.db_pool.clone());
    let input = CreatePaymentOrderInput {
        idempotency_key,
        product_id: input.product_id,
        payment_channel_id: input.payment_channel_id,
        payment_currency: input.payment_currency,
        custom_recharge_minor: input.custom_recharge_minor,
    };
    if let Some(order) = store
        .replay_order(&user.id, &input)
        .await
        .map_err(map_payment_order_error)?
    {
        return Ok((StatusCode::OK, Json(order)));
    }
    let rate = current_rate(&state).await?;
    let order = store
        .create_order(&user.id, input, &rate)
        .await
        .map_err(map_payment_order_error)?;
    Ok((StatusCode::CREATED, Json(order)))
}

pub async fn get_store_order(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;
    if !state.store_order_poll_limiter.allow(&user.id) {
        return Err(AppError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "order_poll_rate_limited",
            "too many order status requests",
        ));
    }
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
    let checkout = CheckoutService::new(
        state.db_pool.clone(),
        state.payment_keys.clone(),
        state.payment_public_origin.clone(),
        state.checkout_provider.clone(),
    )
    .with_client_ip(crate::client_ip::canonical_client_ip_from_headers(&headers));
    let result = checkout
        .create_attempt(&user.id, &id, input)
        .await
        .map_err(map_checkout_error)?;
    Ok((
        if result.replayed {
            StatusCode::OK
        } else {
            StatusCode::CREATED
        },
        Json(json!({
            "attempt": result.attempt,
            "action": result.action,
        })),
    ))
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
        .redeem(
            &user.id,
            &input.code,
            rate.as_ref(),
            &crate::client_ip::canonical_client_ip_from_headers(&headers)
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
        )
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
    body: Result<Json<EmptyStoreMutation>, JsonRejection>,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    let _ = parse_store_json(body)?;
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

pub async fn get_store_payment_compliance_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    let view = PaymentGovernanceStore::new(state.db_pool.clone())
        .compliance(&id)
        .await
        .map_err(map_store_error)?;
    Ok((no_store_headers(), Json(view)))
}

pub async fn confirm_store_payment_compliance_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Result<Json<ConfirmStoreComplianceInput>, JsonRejection>,
) -> AppResult<impl IntoResponse> {
    let admin = require_admin(&headers, &state).await?;
    let input = parse_store_json(body)?;
    let session_token = extract_session_token(&headers).ok_or_else(|| {
        AppError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "missing dashboard session",
        )
    })?;
    let grant_token = headers
        .get("X-Store-Reauth-Token")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| map_reauth_error(ReauthError::InvalidGrant))?;
    ReauthStore::new(state.db_pool.clone())
        .verify(&admin.id, &session_token, grant_token, "compliance_confirm")
        .await
        .map_err(map_reauth_error)?;
    let source_ip = crate::client_ip::canonical_client_ip_from_headers(&headers)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let compliance = PaymentGovernanceStore::new(state.db_pool.clone())
        .confirm_compliance(&id, input, &admin.id, &source_ip)
        .await
        .map_err(map_store_error)?;
    Ok((no_store_headers(), Json(compliance)))
}

pub async fn list_store_payment_capabilities_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    let capabilities = PaymentGovernanceStore::new(state.db_pool.clone())
        .capabilities(&id)
        .await
        .map_err(map_store_error)?;
    Ok((no_store_headers(), Json(capabilities)))
}

pub async fn put_store_payment_capability_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, capability)): Path<(String, String)>,
    body: Result<Json<PutStoreMerchantCapabilityInput>, JsonRejection>,
) -> AppResult<impl IntoResponse> {
    let admin = require_admin(&headers, &state).await?;
    let capability = MerchantCapabilityKind::from_str(&capability)
        .ok_or_else(|| map_store_error(StoreBillingError::InvalidInput))?;
    let input = parse_store_json(body)?;
    let capability = PaymentGovernanceStore::new(state.db_pool.clone())
        .put_capability(&id, capability, input, &admin.id)
        .await
        .map_err(map_store_error)?;
    Ok((no_store_headers(), Json(capability)))
}

pub async fn get_store_payment_availability_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    let availability = PaymentGovernanceStore::new(state.db_pool.clone())
        .availability(&id)
        .await
        .map_err(map_store_error)?;
    Ok((no_store_headers(), Json(availability)))
}

pub async fn create_store_reauth_grant(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<StoreReauthRequest>, JsonRejection>,
) -> AppResult<impl IntoResponse> {
    let admin = require_admin(&headers, &state).await?;
    let request = parse_store_json(body)?;
    let password_is_valid =
        UserStore::verify_password_async(&request.current_password, &admin.password_hash)
            .await
            .map_err(|detail| {
                AppError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "reauthentication failed",
                )
                .with_internal_message(detail)
            })?;
    if !password_is_valid {
        return Err(AppError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_current_password",
            "current password is incorrect",
        ));
    }
    let session_token = extract_session_token(&headers).ok_or_else(|| {
        AppError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "missing dashboard session",
        )
    })?;
    let grant = ReauthStore::new(state.db_pool.clone())
        .issue(&admin.id, &session_token, &request.scope)
        .await
        .map_err(map_reauth_error)?;
    Ok((StatusCode::CREATED, no_store_headers(), Json(grant)))
}

pub async fn replace_store_payment_credential_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Result<Json<Value>, JsonRejection>,
) -> AppResult<impl IntoResponse> {
    let admin = require_admin(&headers, &state).await?;
    let credential = parse_store_json(body)?;
    let session_token = extract_session_token(&headers).ok_or_else(|| {
        AppError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "missing dashboard session",
        )
    })?;
    let grant_token = headers
        .get("X-Store-Reauth-Token")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| map_reauth_error(ReauthError::InvalidGrant))?;
    ReauthStore::new(state.db_pool.clone())
        .verify(&admin.id, &session_token, grant_token, "credential_update")
        .await
        .map_err(map_reauth_error)?;
    let key_ring = state
        .payment_keys
        .clone()
        .ok_or_else(|| map_credential_store_error(CredentialStoreError::EncryptionUnavailable))?;
    let saved = CredentialStore::new(state.db_pool.clone(), key_ring)
        .replace(&id, credential)
        .await
        .map_err(map_credential_store_error)?;
    Ok((no_store_headers(), Json(saved)))
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
    body: Result<Json<EmptyStoreMutation>, JsonRejection>,
) -> AppResult<impl IntoResponse> {
    require_admin(&headers, &state).await?;
    let _ = parse_store_json(body)?;
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

pub async fn get_store_order_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    no_store_response(
        async {
            require_admin(&headers, &state).await?;
            let detail = AdminOrderOperations::detail_from_db(&state.db_pool, &id)
                .await
                .map_err(map_admin_order_operation_error)?;
            Ok(Json(detail))
        }
        .await,
    )
}

pub async fn query_store_order_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Result<Json<AdminOrderAttemptRequest>, JsonRejection>,
) -> Response {
    no_store_response(
        async {
            require_admin(&headers, &state).await?;
            let input = parse_store_json(body)?;
            let key_ring = state.payment_keys.clone().ok_or_else(|| {
                map_admin_order_operation_error(AdminOrderOperationError::ConfigurationUnavailable)
            })?;
            let result = AdminOrderOperations::new(
                state.db_pool.clone(),
                key_ring,
                state.payment_query_provider.clone(),
            )
            .query(&id, &input.attempt_id)
            .await
            .map_err(map_admin_order_operation_error)?;
            Ok(Json(result))
        }
        .await,
    )
}

pub async fn close_store_order_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Result<Json<AdminOrderAttemptRequest>, JsonRejection>,
) -> Response {
    no_store_response(
        async {
            require_admin(&headers, &state).await?;
            let input = parse_store_json(body)?;
            let key_ring = state.payment_keys.clone().ok_or_else(|| {
                map_admin_order_operation_error(AdminOrderOperationError::ConfigurationUnavailable)
            })?;
            let result = AdminOrderOperations::new(
                state.db_pool.clone(),
                key_ring,
                state.payment_query_provider.clone(),
            )
            .close(&id, &input.attempt_id)
            .await
            .map_err(map_admin_order_operation_error)?;
            Ok(Json(result))
        }
        .await,
    )
}

pub async fn create_store_refund_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Result<Json<EmptyStoreMutation>, JsonRejection>,
) -> Response {
    no_store_response(
        async {
            let admin = require_admin(&headers, &state).await?;
            let _ = parse_store_json(body)?;
            require_refund_access(&headers, &state, &admin.id).await?;
            let idempotency_key = required_idempotency_key(&headers)?;
            let key_ring = state.payment_keys.clone().ok_or_else(|| {
                map_refund_operations_error(RefundOperationsError::ConfigurationUnavailable)
            })?;
            let refund = RefundOperations::new(
                state.db_pool.clone(),
                key_ring,
                state.refund_provider.clone(),
            )
            .begin(&id, &admin.id, &idempotency_key)
            .await
            .map_err(map_refund_operations_error)?;
            Ok(Json(refund))
        }
        .await,
    )
}

pub async fn get_store_refund_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, refund_id)): Path<(String, String)>,
) -> Response {
    no_store_response(
        async {
            require_admin(&headers, &state).await?;
            let refund = RecoveryStore::new(state.db_pool.clone())
                .get_refund(&refund_id)
                .await
                .map_err(map_refund_read_error)?
                .filter(|refund| refund.order_id == id)
                .ok_or_else(|| map_refund_operations_error(RefundOperationsError::NotFound))?;
            Ok(Json(refund))
        }
        .await,
    )
}

pub async fn query_store_refund_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, refund_id)): Path<(String, String)>,
    body: Result<Json<EmptyStoreMutation>, JsonRejection>,
) -> Response {
    no_store_response(
        async {
            let admin = require_admin(&headers, &state).await?;
            let _ = parse_store_json(body)?;
            require_refund_access(&headers, &state, &admin.id).await?;
            let key_ring = state.payment_keys.clone().ok_or_else(|| {
                map_refund_operations_error(RefundOperationsError::ConfigurationUnavailable)
            })?;
            let refund = RefundOperations::new(
                state.db_pool.clone(),
                key_ring,
                state.refund_provider.clone(),
            )
            .query(&id, &refund_id)
            .await
            .map_err(map_refund_operations_error)?;
            Ok(Json(refund))
        }
        .await,
    )
}

pub async fn reprocess_store_provider_event_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(event_id): Path<String>,
    body: Result<Json<EmptyStoreMutation>, JsonRejection>,
) -> Response {
    no_store_response(
        async {
            let admin = require_admin(&headers, &state).await?;
            require_reprocess_access(&headers, &state, &admin.id).await?;
            let callbacks = PaymentCallbackStore::new(state.db_pool.clone());
            if let Err(error) = parse_store_json(body) {
                callbacks
                    .audit_invalid_reprocess_request(&event_id, &admin.id)
                    .await
                    .map_err(map_reprocess_provider_event_error)?;
                return Err(error);
            }
            let result = callbacks
                .reprocess_verified_event(&event_id, state.payment_keys.as_deref(), &admin.id)
                .await
                .map_err(map_reprocess_provider_event_error)?;
            Ok(Json(result))
        }
        .await,
    )
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
    let key_ring = state
        .payment_keys
        .as_deref()
        .ok_or_else(|| map_store_error(StoreBillingError::EncryptionUnavailable))?;
    let codes = state
        .store_billing
        .generate_redemption_codes(key_ring, &admin.id, input)
        .await
        .map_err(map_store_error)?;
    Ok((StatusCode::CREATED, no_store_headers(), Json(codes)))
}

pub async fn reveal_store_redemption_codes_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<RevealRedemptionInput>, JsonRejection>,
) -> AppResult<impl IntoResponse> {
    let admin = require_admin(&headers, &state).await?;
    require_redemption_access(&headers, &state, &admin.id).await?;
    let input = parse_store_json(body)?;
    if input.action == RedemptionAccessAction::Export {
        return Err(map_store_error(StoreBillingError::InvalidInput));
    }
    let key_ring = state
        .payment_keys
        .as_deref()
        .ok_or_else(|| map_store_error(StoreBillingError::EncryptionUnavailable))?;
    let context = redemption_audit_context(&headers, &admin.id);
    let codes = state
        .store_billing
        .reveal_redemption_codes(key_ring, input, &context)
        .await
        .map_err(map_store_error)?;
    Ok((no_store_headers(), Json(codes)))
}

pub async fn export_store_redemption_codes_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<RedemptionCodeIdsRequest>, JsonRejection>,
) -> AppResult<Response> {
    let admin = require_admin(&headers, &state).await?;
    require_redemption_access(&headers, &state, &admin.id).await?;
    let request = parse_store_json(body)?;
    let key_ring = state
        .payment_keys
        .as_deref()
        .ok_or_else(|| map_store_error(StoreBillingError::EncryptionUnavailable))?;
    let context = redemption_audit_context(&headers, &admin.id);
    let codes = state
        .store_billing
        .reveal_redemption_codes(
            key_ring,
            RevealRedemptionInput {
                code_ids: request.code_ids,
                action: RedemptionAccessAction::Export,
            },
            &context,
        )
        .await
        .map_err(map_store_error)?;
    let mut csv = String::from("code_id,code\r\n");
    for code in codes {
        csv.push_str(&code.id);
        csv.push(',');
        csv.push_str(&code.code);
        csv.push_str("\r\n");
    }
    let mut response = Response::new(Body::from(csv));
    for (name, value) in no_store_headers() {
        response.headers_mut().insert(name, value);
    }
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    response.headers_mut().insert(
        CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=lynshen-redemption-codes.csv"),
    );
    Ok(response)
}

pub async fn revoke_store_redemption_code_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Result<Json<EmptyStoreMutation>, JsonRejection>,
) -> AppResult<impl IntoResponse> {
    let admin = require_admin(&headers, &state).await?;
    let _ = parse_store_json(body)?;
    let record = state
        .store_billing
        .revoke_redemption_code(&id, &admin.id)
        .await
        .map_err(map_store_error)?;
    Ok(Json(record))
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
