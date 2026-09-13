use crate::app::AppState;
use crate::dashboard_handlers::session_helpers::require_admin;
use crate::error::{AppError, AppResult};
use crate::firewall_events::{FirewallEventFilter, FirewallStats};
use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
pub struct FirewallEventsQuery {
    #[serde(default = "default_events_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
    #[serde(default)]
    pub term: Option<String>,
    #[serde(default)]
    pub since_ms: Option<i64>,
    #[serde(default)]
    pub until_ms: Option<i64>,
}

fn default_events_limit() -> i64 {
    50
}

fn events_error(error: String) -> AppError {
    AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", error)
}

/// CF-24: server-side aggregation for the firewall dashboard page.
pub async fn get_firewall_stats(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    let _admin = require_admin(&headers, &state).await?;
    let stats: FirewallStats = crate::firewall_events::compute_stats(&state.db_pool)
        .await
        .map_err(events_error)?;
    Ok(Json(stats))
}

/// CF-25: newest-first paginated event rows for the calling tenant.
pub async fn list_firewall_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<FirewallEventsQuery>,
) -> AppResult<impl IntoResponse> {
    let _admin = require_admin(&headers, &state).await?;
    let limit = query.limit.clamp(1, 200);
    let offset = query.offset.max(0);
    let filter = FirewallEventFilter {
        term: query.term.filter(|t| !t.trim().is_empty()),
        since_ms: query.since_ms,
        until_ms: query.until_ms,
    };
    let (rows, total) = crate::firewall_events::list_events(
        &state.db_pool,
        &filter,
        limit as u64,
        offset as u64,
    )
    .await
    .map_err(events_error)?;
    let data: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|row| {
            json!({
                "id": row.id,
                "user_id": row.user_id,
                "username": row.username,
                "api_key_id": row.api_key_id,
                "api_key_name": row.api_key_name,
                "endpoint": row.endpoint,
                "model": row.model,
                "term": row.term,
                "content": row.content,
                "created_at": row.created_at,
                "created_at_unix_ms": row.created_at_unix_ms,
            })
        })
        .collect();
    Ok(Json(json!({
        "data": data,
        "total": total,
        "limit": limit,
        "offset": offset,
        "server_time_ms": Utc::now().timestamp_millis(),
    })))
}
