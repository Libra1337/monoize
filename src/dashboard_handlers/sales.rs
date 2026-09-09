//! Sales agent and Admin sales endpoints (`spec/sales-commission.spec.md`).

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::{Json, extract::rejection::JsonRejection};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::app::AppState;
use crate::dashboard_handlers::session_helpers::get_current_user;
use crate::error::{AppError, AppResult};
use crate::store_billing::sales::{MAX_COMMISSION_RATE_BP, MIN_WITHDRAWAL_MINOR};
use crate::store_billing::sales_store::{
    SalesAgent, SalesCommissionEntry, SalesStore, SalesStoreError, SalesWindow, SalesWithdrawal,
};
use crate::users::UserRole;

fn map_sales_error(error: SalesStoreError) -> AppError {
    let (status, code, message) = match error {
        SalesStoreError::InvalidInput => (
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "sales input is invalid".to_string(),
        ),
        SalesStoreError::CodeInvalid => (
            StatusCode::BAD_REQUEST,
            "sales_code_invalid",
            "Sales code is invalid; check it and enter it again".to_string(),
        ),
        SalesStoreError::SelfReferral => (
            StatusCode::BAD_REQUEST,
            "sales_code_self_referral",
            "An agent may not credit their own order".to_string(),
        ),
        // SC-2.7a: refusing beats accruing zero, so the buyer is told to raise the amount.
        SalesStoreError::AmountTooSmall => (
            StatusCode::BAD_REQUEST,
            "sales_code_amount_too_small",
            "A sales code requires an order of at least 1 CNY".to_string(),
        ),
        SalesStoreError::DiscountAboveRate => (
            StatusCode::BAD_REQUEST,
            "sales_discount_above_rate",
            "Discount may not exceed the commission rate".to_string(),
        ),
        SalesStoreError::NotAgent => (
            StatusCode::FORBIDDEN,
            "sales_agent_required",
            "This account is not a sales agent".to_string(),
        ),
        // SC-4.4: a missing order and a mismatched buyer return the same error, so a claim
        // cannot be used to learn whether an order number exists.
        SalesStoreError::ClaimMismatch => (
            StatusCode::NOT_FOUND,
            "sales_claim_mismatch",
            "The order number and user ID do not correspond; confirm them and claim again"
                .to_string(),
        ),
        SalesStoreError::ClaimAlreadyCredited => (
            StatusCode::CONFLICT,
            "sales_claim_already_credited",
            "This order already has commission".to_string(),
        ),
        SalesStoreError::ClaimRateLimited => (
            StatusCode::TOO_MANY_REQUESTS,
            "sales_claim_rate_limited",
            "Too many claim attempts; try again later".to_string(),
        ),
        SalesStoreError::WithdrawalPending => (
            StatusCode::CONFLICT,
            "sales_withdrawal_pending",
            "A withdrawal is already pending".to_string(),
        ),
        SalesStoreError::WithdrawalNotPending => (
            StatusCode::CONFLICT,
            "sales_withdrawal_not_pending",
            "This withdrawal is not pending".to_string(),
        ),
        SalesStoreError::InsufficientBalance => (
            StatusCode::BAD_REQUEST,
            "sales_insufficient_balance",
            "Withdrawal exceeds the available commission balance".to_string(),
        ),
        SalesStoreError::Storage(detail) => {
            (StatusCode::INTERNAL_SERVER_ERROR, "internal_error", detail)
        }
    };
    AppError::new(status, code, message)
}

fn parse_body<T: serde::de::DeserializeOwned>(
    body: Result<Json<T>, JsonRejection>,
) -> AppResult<T> {
    body.map(|Json(value)| value).map_err(|_| {
        AppError::new(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "request body is invalid",
        )
    })
}

/// Resolves the caller as an agent, or fails with SC-6.1's error.
async fn require_agent(headers: &HeaderMap, state: &AppState) -> AppResult<SalesAgent> {
    let user = get_current_user(headers, state).await?;
    SalesStore::new(state.db_pool.clone())
        .agent_for_user(&user.id)
        .await
        .map_err(map_sales_error)?
        .ok_or_else(|| {
            AppError::new(
                StatusCode::FORBIDDEN,
                "sales_agent_required",
                "This account is not a sales agent",
            )
        })
}

async fn require_admin_user(
    headers: &HeaderMap,
    state: &AppState,
) -> AppResult<crate::users::User> {
    let user = get_current_user(headers, state).await?;
    if !matches!(user.role, UserRole::Admin | UserRole::SuperAdmin) {
        return Err(AppError::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            "admin access required",
        ));
    }
    Ok(user)
}

#[derive(Debug, Serialize)]
pub struct SalesOverviewResponse {
    pub agent: SalesAgent,
    pub commission_rate_bp: i64,
    pub today: SalesWindow,
    pub last_7d: SalesWindow,
    pub last_30d: SalesWindow,
    pub pending_withdrawal: Option<SalesWithdrawal>,
}

/// SC-6.1.
pub async fn get_sales_overview(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    let agent = require_agent(&headers, &state).await?;
    let store = SalesStore::new(state.db_pool.clone());
    let now = Utc::now();
    let (today, last_7d, last_30d) = store
        .windows(&agent.user_id, now)
        .await
        .map_err(map_sales_error)?;
    let pending_withdrawal = store
        .list_withdrawals(Some(&agent.user_id))
        .await
        .map_err(map_sales_error)?
        .into_iter()
        .find(|withdrawal| withdrawal.state == "requested");
    let commission_rate_bp = store.commission_rate_bp().await.map_err(map_sales_error)?;
    Ok(Json(SalesOverviewResponse {
        agent,
        commission_rate_bp,
        today,
        last_7d,
        last_30d,
        pending_withdrawal,
    }))
}

/// SC-6.3.
pub async fn list_sales_entries(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    let agent = require_agent(&headers, &state).await?;
    let entries: Vec<SalesCommissionEntry> = SalesStore::new(state.db_pool.clone())
        .list_entries(&agent.user_id)
        .await
        .map_err(map_sales_error)?;
    Ok(Json(entries))
}

/// SC-6.4.
pub async fn list_sales_withdrawals(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    let agent = require_agent(&headers, &state).await?;
    let withdrawals = SalesStore::new(state.db_pool.clone())
        .list_withdrawals(Some(&agent.user_id))
        .await
        .map_err(map_sales_error)?;
    Ok(Json(withdrawals))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateSalesClaimRequest {
    pub order_number: String,
    pub user_id: String,
}

/// SC-4.1.
pub async fn create_sales_claim(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<CreateSalesClaimRequest>, JsonRejection>,
) -> AppResult<impl IntoResponse> {
    let agent = require_agent(&headers, &state).await?;
    let input = parse_body(body)?;
    let order_number = input.order_number.trim();
    let buyer_user_id = input.user_id.trim();
    if order_number.is_empty() || buyer_user_id.is_empty() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "order number and user ID are required",
        ));
    }
    let entry = SalesStore::new(state.db_pool.clone())
        .claim_order(&agent.user_id, order_number, buyer_user_id, Utc::now())
        .await
        .map_err(map_sales_error)?;
    Ok((StatusCode::CREATED, Json(entry)))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateSalesWithdrawalRequest {
    pub amount_fen: String,
}

/// SC-5.1.
pub async fn create_sales_withdrawal(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<CreateSalesWithdrawalRequest>, JsonRejection>,
) -> AppResult<impl IntoResponse> {
    let agent = require_agent(&headers, &state).await?;
    let input = parse_body(body)?;
    let amount: i128 = input.amount_fen.parse().map_err(|_| {
        AppError::new(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "amount is invalid",
        )
    })?;
    if amount < MIN_WITHDRAWAL_MINOR || amount.to_string() != input.amount_fen {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "sales_withdrawal_below_minimum",
            "The minimum withdrawal is 100 CNY",
        ));
    }
    let withdrawal = SalesStore::new(state.db_pool.clone())
        .request_withdrawal(&agent.user_id, amount, Utc::now())
        .await
        .map_err(map_sales_error)?;
    Ok((StatusCode::CREATED, Json(withdrawal)))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateSalesAgentRequest {
    pub discount_bp: i64,
}

#[derive(Debug, Serialize)]
pub struct CreatedSalesAgentResponse {
    pub agent: SalesAgent,
    /// SC-7.2a: shown once; only the hash is stored.
    pub password: String,
}

/// SC-7.1 and SC-7.2.
pub async fn create_sales_agent_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<CreateSalesAgentRequest>, JsonRejection>,
) -> AppResult<impl IntoResponse> {
    require_admin_user(&headers, &state).await?;
    let input = parse_body(body)?;
    let store = SalesStore::new(state.db_pool.clone());

    let code = store.available_code().await.map_err(map_sales_error)?;
    // SC-7.2: the username is the code, so the operator has one identifier to hand over.
    let password = crate::store_billing::sales::generate_password();
    let user = state
        .user_store
        .create_user(&code, &password, UserRole::User, None)
        .await
        .map_err(|error| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                error.to_string(),
            )
        })?;
    let agent = match store.insert_agent(&user.id, &code, input.discount_bp).await {
        Ok(agent) => agent,
        Err(error) => {
            // The user row would otherwise be an orphan that occupies the code's username.
            let _ = state.user_store.delete_user(&user.id).await;
            return Err(map_sales_error(error));
        }
    };
    Ok((
        StatusCode::CREATED,
        Json(CreatedSalesAgentResponse { agent, password }),
    ))
}

/// SC-7.3.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateSalesAgentRequest {
    pub discount_bp: i64,
    pub enabled: bool,
}

pub async fn update_sales_agent_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    body: Result<Json<UpdateSalesAgentRequest>, JsonRejection>,
) -> AppResult<impl IntoResponse> {
    require_admin_user(&headers, &state).await?;
    let input = parse_body(body)?;
    let agent = SalesStore::new(state.db_pool.clone())
        .update_agent(&user_id, input.discount_bp, input.enabled)
        .await
        .map_err(map_sales_error)?;
    Ok(Json(agent))
}

pub async fn list_sales_agents_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    require_admin_user(&headers, &state).await?;
    let agents = SalesStore::new(state.db_pool.clone())
        .list_agents()
        .await
        .map_err(map_sales_error)?;
    Ok(Json(agents))
}

/// SC-5.8.
pub async fn list_sales_withdrawals_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    require_admin_user(&headers, &state).await?;
    let withdrawals = SalesStore::new(state.db_pool.clone())
        .list_withdrawals(None)
        .await
        .map_err(map_sales_error)?;
    Ok(Json(withdrawals))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecideSalesWithdrawalRequest {
    pub decision: String,
    #[serde(default)]
    pub decision_note: String,
}

/// SC-5.4 through SC-5.7.
pub async fn decide_sales_withdrawal_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(withdrawal_id): Path<String>,
    body: Result<Json<DecideSalesWithdrawalRequest>, JsonRejection>,
) -> AppResult<impl IntoResponse> {
    let admin = require_admin_user(&headers, &state).await?;
    let input = parse_body(body)?;
    let paid = match input.decision.as_str() {
        "paid" => true,
        "rejected" => false,
        _ => {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "decision must be paid or rejected",
            ));
        }
    };
    if input.decision_note.chars().count() > 512 {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "decision note is too long",
        ));
    }
    let withdrawal = SalesStore::new(state.db_pool.clone())
        .decide_withdrawal(
            &withdrawal_id,
            &admin.id,
            paid,
            &input.decision_note,
            Utc::now(),
        )
        .await
        .map_err(map_sales_error)?;
    Ok(Json(withdrawal))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateSalesSettingsRequest {
    pub commission_rate_bp: i64,
}

/// SC-7.5.
pub async fn update_sales_settings_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<UpdateSalesSettingsRequest>, JsonRejection>,
) -> AppResult<impl IntoResponse> {
    require_admin_user(&headers, &state).await?;
    let input = parse_body(body)?;
    if !(0..=MAX_COMMISSION_RATE_BP).contains(&input.commission_rate_bp) {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "commission rate is out of range",
        ));
    }
    let rate = SalesStore::new(state.db_pool.clone())
        .set_commission_rate_bp(input.commission_rate_bp)
        .await
        .map_err(map_sales_error)?;
    Ok(Json(serde_json::json!({ "commission_rate_bp": rate })))
}

pub async fn get_sales_settings_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    require_admin_user(&headers, &state).await?;
    let rate = SalesStore::new(state.db_pool.clone())
        .commission_rate_bp()
        .await
        .map_err(map_sales_error)?;
    Ok(Json(serde_json::json!({ "commission_rate_bp": rate })))
}
