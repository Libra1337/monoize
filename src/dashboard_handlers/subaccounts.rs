//! User-level sub-accounts (`user-sub-accounts.spec.md`).
//!
//! A sub-account is an ordinary user row whose `parent_user_id` names the owning main
//! account. It can use the platform and create its own API keys, but it cannot recharge,
//! change its profile, or create further sub-accounts; its balance comes only from
//! distributions made by the main account (SAU-4, SAU-6).

use crate::app::AppState;
use crate::dashboard_handlers::auth::UserResponse;
use crate::dashboard_handlers::session_helpers::{
    get_current_user, is_reserved_internal_username, is_valid_username,
};
use crate::error::{AppError, AppResult};
use crate::users::User;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateSubAccountRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferToSubRequest {
    /// Positive integer nano-USD string (SAU-6).
    pub amount_nano_usd: String,
}

#[derive(Debug, Serialize)]
pub struct SubAccountResponse {
    pub id: String,
    pub username: String,
    pub enabled: bool,
    pub balance_nano_usd: String,
    pub created_at: String,
    pub last_login_at: Option<String>,
    pub api_key_count: i64,
    pub today_calls: i64,
    pub today_cost_nano_usd: String,
}

/// SAU-2: agent-class accounts, sales agents, and sub-accounts themselves may not open a
/// sub-account.
pub fn sub_account_creation_allowed(user: &User, is_sales_agent: bool) -> AppResult<()> {
    if user.parent_user_id.is_some() {
        return Err(forbidden("a sub-account cannot open its own sub-accounts"));
    }
    if user.account_class == crate::users::AccountClass::Agent {
        return Err(forbidden("agent-class accounts cannot open sub-accounts"));
    }
    if is_sales_agent {
        return Err(forbidden("sales agent accounts cannot open sub-accounts"));
    }
    Ok(())
}

fn forbidden(message: &str) -> AppError {
    AppError::new(StatusCode::FORBIDDEN, "sub_account_forbidden", message)
}

async fn is_sales_agent(state: &AppState, user_id: &str) -> AppResult<bool> {
    Ok(crate::store_billing::sales_store::SalesStore::new(state.db_pool.clone())
        .list_agents()
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e.to_string()))?
        .iter()
        .any(|agent| agent.user_id == user_id))
}

pub async fn create_sub_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateSubAccountRequest>,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;
    sub_account_creation_allowed(&user, is_sales_agent(&state, &user.id).await?)?;

    if !is_valid_username(&body.username) {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "invalid_username",
            "username must be 3-22 characters, only letters, digits and underscores",
        ));
    }
    if is_reserved_internal_username(&body.username) {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "reserved_username",
            "username prefix _monoize_ is reserved",
        ));
    }
    if body.password.len() < 8 {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "invalid_password",
            "password must be at least 8 characters",
        ));
    }

    let sub = state
        .user_store
        .create_sub_user(&user, body.username.trim(), &body.password)
        .await
        .map_err(|e| {
            if e == "username_exists" {
                AppError::new(StatusCode::CONFLICT, "username_exists", "username already taken")
            } else {
                AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e)
            }
        })?;

    Ok((
        StatusCode::CREATED,
        Json(SubAccountResponse {
            id: sub.id,
            username: sub.username,
            enabled: sub.enabled,
            balance_nano_usd: sub.balance_nano_usd,
            created_at: sub.created_at.to_rfc3339(),
            last_login_at: None,
            api_key_count: 0,
            today_calls: 0,
            today_cost_nano_usd: "0".to_string(),
        }),
    ))
}

pub async fn list_sub_accounts(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;
    let subs = state
        .user_store
        .list_sub_users(&user.id)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;
    if subs.is_empty() {
        return Ok(Json(Vec::<SubAccountResponse>::new()));
    }

    let today_start = chrono::Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", "invalid day start"))?
        .and_utc()
        .to_rfc3339();
    let usage_rows = state
        .user_store
        .get_users_today_usage(&today_start)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;
    let usage_by_id: HashMap<_, _> = usage_rows
        .into_iter()
        .map(|row| (row.user_id.clone(), row))
        .collect();

    let key_counts = state
        .user_store
        .count_api_keys_by_users(&subs.iter().map(|sub| sub.id.clone()).collect::<Vec<_>>())
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;
    let responses = subs
        .into_iter()
        .map(|sub| {
            let today = usage_by_id.get(&sub.id);
            SubAccountResponse {
                api_key_count: key_counts.get(&sub.id).copied().unwrap_or(0),
                id: sub.id,
                username: sub.username,
                enabled: sub.enabled,
                balance_nano_usd: sub.balance_nano_usd,
                created_at: sub.created_at.to_rfc3339(),
                last_login_at: sub.last_login_at.map(|d| d.to_rfc3339()),
                today_calls: today.map(|row| row.today_calls).unwrap_or(0),
                today_cost_nano_usd: today
                    .map(|row| row.today_cost_nano_usd.to_string())
                    .unwrap_or_else(|| "0".to_string()),
            }
        })
        .collect::<Vec<_>>();
    Ok(Json(responses))
}

pub async fn distribute_to_sub_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(sub_user_id): Path<String>,
    Json(body): Json<TransferToSubRequest>,
) -> AppResult<impl IntoResponse> {
    let user = get_current_user(&headers, &state).await?;
    let amount = body
        .amount_nano_usd
        .trim()
        .parse::<i128>()
        .map_err(|_| {
            AppError::new(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "amount_nano_usd must be an integer string",
            )
        })?;
    if amount <= 0 {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "amount_nano_usd must be positive",
        ));
    }

    let (parent_after, sub_after) = state
        .user_store
        .transfer_balance_to_sub(&user.id, &sub_user_id, amount)
        .await
        .map_err(|e| match e.as_str() {
            "insufficient_balance" => AppError::new(
                StatusCode::BAD_REQUEST,
                "insufficient_balance",
                "main account balance is insufficient",
            ),
            "sub account not found" => {
                AppError::new(StatusCode::NOT_FOUND, "not_found", "sub account not found")
            }
            "user not found" => AppError::new(StatusCode::NOT_FOUND, "not_found", "user not found"),
            other => AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", other),
        })?;

    Ok(Json(serde_json::json!({
        "parent_balance_nano_usd": parent_after,
        "sub_balance_nano_usd": sub_after,
    })))
}
