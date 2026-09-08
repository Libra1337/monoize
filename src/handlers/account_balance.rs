use super::auth_tenant;
use crate::app::AppState;
use crate::error::{AppError, AppResult};
use crate::users::{format_nano_to_usd, parse_nano_usd};
use axum::Json;
use axum::extract::State;
use axum::http::header::CACHE_CONTROL;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EffectiveBalance {
    nano_usd: i128,
    unlimited: bool,
}

fn unauthorized(message: &'static str) -> AppError {
    AppError::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
}

fn internal_error(message: impl Into<String>) -> AppError {
    AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
}

async fn effective_balance(
    state: &AppState,
    auth: &crate::auth::AuthResult,
) -> AppResult<EffectiveBalance> {
    let api_key_id = auth
        .api_key_id
        .as_deref()
        .ok_or_else(|| unauthorized("api key not found"))?;
    let authenticated_user_id = auth
        .user_id
        .as_deref()
        .ok_or_else(|| unauthorized("user not found"))?;
    let api_key = state
        .user_store
        .get_api_key_by_id(api_key_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| unauthorized("api key not found"))?;
    if api_key.user_id != authenticated_user_id {
        return Err(unauthorized("api key owner mismatch"));
    }

    let (stored_balance, unlimited, subject_id) = if api_key.sub_account_enabled {
        let balance = parse_nano_usd(&api_key.sub_account_balance_nano).map_err(|error| {
            internal_error(format!("invalid stored sub-account balance: {error}"))
        })?;
        (balance, false, api_key_id)
    } else {
        let balance = state
            .user_store
            .get_user_balance_uncached(authenticated_user_id)
            .await
            .map_err(internal_error)?
            .ok_or_else(|| unauthorized("user not found"))?;
        if balance.user_id != authenticated_user_id {
            return Err(unauthorized("user mismatch"));
        }
        (
            balance.balance_nano_usd,
            balance.balance_unlimited,
            authenticated_user_id,
        )
    };

    if unlimited {
        return Ok(EffectiveBalance {
            nano_usd: stored_balance,
            unlimited: true,
        });
    }

    let pending_deductions = state
        .metering
        .as_ref()
        .map(|metering| metering.pending().outstanding(subject_id))
        .unwrap_or(0);
    let nano_usd = stored_balance
        .checked_sub(pending_deductions)
        .ok_or_else(|| internal_error("effective balance overflow"))?;
    Ok(EffectiveBalance {
        nano_usd,
        unlimited: false,
    })
}

fn no_store_json(value: Value) -> Response {
    let mut response = Json(value).into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub async fn codex_usage(State(state): State<AppState>, headers: HeaderMap) -> AppResult<Response> {
    let auth = auth_tenant(&headers, &state).await?;
    let balance = effective_balance(&state, &auth).await?;
    let available = balance.unlimited || balance.nano_usd > 0;
    let balance_usd = (!balance.unlimited).then(|| format_nano_to_usd(balance.nano_usd));
    let rate_limit_reached_type = if available {
        Value::Null
    } else {
        json!({ "type": "rate_limit_reached" })
    };

    Ok(no_store_json(json!({
        "plan_type": "unknown",
        "rate_limit": {
            "allowed": available,
            "limit_reached": !available,
            "primary_window": null,
            "secondary_window": null
        },
        "credits": {
            "has_credits": !balance.unlimited && balance.nano_usd > 0,
            "unlimited": balance.unlimited,
            "balance": balance_usd
        },
        "spend_control": null,
        "additional_rate_limits": null,
        "rate_limit_reached_type": rate_limit_reached_type,
        "rate_limit_reset_credits": {
            "available_count": 0
        }
    })))
}

pub async fn deepseek_user_balance(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Response> {
    let auth = auth_tenant(&headers, &state).await?;
    let balance = effective_balance(&state, &auth).await?;
    let balance_usd = format_nano_to_usd(balance.nano_usd);

    Ok(no_store_json(json!({
        "is_available": balance.unlimited || balance.nano_usd > 0,
        "balance_infos": [
            {
                "currency": "USD",
                "total_balance": balance_usd,
                "granted_balance": "0",
                "topped_up_balance": balance_usd
            }
        ]
    })))
}
