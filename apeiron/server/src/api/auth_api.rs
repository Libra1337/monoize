//! AP-A1..A4: handoff exchange, logout, /me.

use crate::error::{ApiError, ApiResult};
use crate::state::SharedState;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
pub struct ExchangeBody {
    pub token: String,
}

fn request_is_secure(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .map(|proto| proto == "https")
        .unwrap_or(false)
}

pub async fn exchange(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(body): Json<ExchangeBody>,
) -> ApiResult<impl IntoResponse> {
    if body.token.trim().is_empty() {
        return Err(ApiError::bad_request("invalid_token", "token is required"));
    }
    let bridge = crate::bridge::BridgeClient::new(
        &state.http,
        &state.cfg.bridge_url,
        state.cfg.bridge_service_token.as_deref(),
    );
    let bridge_user = bridge.exchange(&body.token).await.map_err(|error| {
        let message = error.message();
        match error {
            crate::bridge::BridgeError::Rejected(_) => {
                ApiError::new(StatusCode::BAD_REQUEST, "invalid_token", message)
            }
            crate::bridge::BridgeError::Transport(_) => ApiError::new(
                StatusCode::BAD_GATEWAY,
                "bridge_unavailable",
                message,
            ),
            other => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                other.message(),
            ),
        }
    })?;
    crate::auth::upsert_mirror_user(&state, &bridge_user)
        .await
        .map_err(ApiError::internal)?;
    let token = crate::auth::create_session(&state, &bridge_user.user_id)
        .await
        .map_err(ApiError::internal)?;
    let cookie = crate::auth::session_cookie(&token, request_is_secure(&headers));
    Ok((
        StatusCode::OK,
        [("set-cookie", cookie)],
        Json(json!({
            "user": {
                "id": bridge_user.user_id,
                "username": bridge_user.username,
                "display_name": bridge_user.display_name,
                "role": bridge_user.role,
            },
            "balance_nano_usd": bridge_user.balance_nano_usd,
            "balance_unlimited": bridge_user.balance_unlimited,
        })),
    ))
}

pub async fn logout(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let _ = crate::auth::delete_session(&state, &headers).await;
    Ok((
        StatusCode::OK,
        [("set-cookie", crate::auth::clear_cookie())],
        Json(json!({ "ok": true })),
    ))
}

#[derive(Deserialize)]
pub struct MeQuery {
    pub refresh: Option<String>,
}

pub async fn me(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(query): Query<MeQuery>,
) -> ApiResult<impl IntoResponse> {
    let user = crate::auth::current_user(&state, &headers).await?;
    let refreshed = if query.refresh.as_deref() == Some("1") {
        crate::engine::refresh_mirror_balance(&state, &user.id).await;
        true
    } else {
        // AP-A3: mirror is valid for at most 30 seconds.
        let synced = chrono::DateTime::parse_from_rfc3339(&user.balance_synced_at)
            .map(|time| time.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::DateTime::UNIX_EPOCH);
        let stale =
            chrono::Utc::now() - synced > chrono::Duration::seconds(30);
        if stale {
            crate::engine::refresh_mirror_balance(&state, &user.id).await;
            true
        } else {
            false
        }
    };
    let user = if refreshed {
        crate::auth::current_user(&state, &headers).await?
    } else {
        user
    };
    Ok(Json(json!({
        "user": user.to_json(),
        "balance_nano_usd": user.balance_nano_usd,
        "balance_unlimited": user.balance_unlimited,
        "platform_url": state.cfg.platform_url,
    })))
}
