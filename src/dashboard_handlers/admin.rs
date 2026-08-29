use crate::app::AppState;
use crate::dashboard_handlers::session_helpers::require_admin;
use crate::error::{AppError, AppResult};
use crate::handlers::routing::health_key;
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use chrono::{NaiveTime, Utc};
use serde_json::{Value, json};
use std::collections::HashMap;

pub(crate) fn sort_usage_models(rows: &mut [crate::users::UserModelUsageRankingRow]) {
    rows.sort_by(|left, right| {
        let left_tokens = left
            .input_tokens
            .saturating_add(left.cache_read_tokens)
            .saturating_add(left.output_tokens);
        let right_tokens = right
            .input_tokens
            .saturating_add(right.cache_read_tokens)
            .saturating_add(right.output_tokens);
        right_tokens
            .cmp(&left_tokens)
            .then_with(|| right.call_count.cmp(&left.call_count))
            .then_with(|| left.model.as_bytes().cmp(right.model.as_bytes()))
    });
}

/// security-access-control.spec.md SAC-1..SAC-5: metrics expose runtime topology
/// and therefore use the same authorization boundary as the admin dashboard.
pub async fn get_metrics(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<impl axum::response::IntoResponse> {
    require_admin(&headers, &state).await?;
    Ok(state.metrics.render())
}

/// admin-dashboard.spec.md AD-1..AD-5: one admin-only aggregate snapshot of
/// node/system status, replica state, user usage ranking, and channel health.
pub async fn get_admin_overview(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    require_admin(&headers, &state).await?;

    let now = chrono::Utc::now();
    let started_at = state.started_at;
    let uptime_seconds = now
        .signed_duration_since(started_at)
        .to_std()
        .map(|duration| duration.as_secs())
        .unwrap_or(0);

    let database_backend = if state.db_pool.is_sqlite() {
        "sqlite"
    } else if state.db_pool.is_postgres() {
        "postgres"
    } else {
        "unknown"
    };
    let dsn_redacted = redact_dsn(&state.runtime.database_dsn);
    let role = state.node.role.as_str();

    let (spool_pending_count, spool_pending_bytes) = match (role, state.metering.as_ref()) {
        ("replica", Some(metering)) => {
            let (count, bytes) = metering.delta_spool().pending_stats();
            (count, bytes)
        }
        _ => (0usize, 0u64),
    };

    let sse_connections: usize = state
        .sse_connections
        .iter()
        .map(|entry| entry.value().load(std::sync::atomic::Ordering::Relaxed))
        .sum::<usize>()
        .min(usize::MAX);

    let ranking_window_from = (now - chrono::Duration::hours(24)).to_rfc3339();
    let ranking = state
        .user_store
        .get_users_usage_ranking(&ranking_window_from, &now.to_rfc3339(), 20)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;
    let users_ranking: Vec<Value> = ranking
        .into_iter()
        .map(|row| {
            json!({
                "user_id": row.user_id,
                "username": row.username,
                "call_count": row.call_count,
                "cost_nano_usd": row.cost_nano_usd.to_string(),
            })
        })
        .collect();

    let today_start = Utc::now()
        .date_naive()
        .and_time(NaiveTime::MIN)
        .and_utc()
        .to_rfc3339();
    let (today_calls, today_cost_nano_usd) = state
        .user_store
        .get_today_usage_totals(&today_start)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;
    let channel_today: HashMap<String, crate::users::ChannelTodayUsage> = state
        .user_store
        .get_channels_today_usage(&today_start)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?
        .into_iter()
        .map(|row| (row.channel_id.clone(), row))
        .collect();

    let providers = state
        .monoize_store
        .list_providers()
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;
    let health = state.channel_health.lock().await;
    let now_ms = crate::handlers::routing::now_ts();
    let mut channel_health: Vec<Value> = Vec::new();
    for provider in providers {
        for channel in [provider.channel] {
            let base_entry = health.get(&channel.id);
            let mut healthy = base_entry.map(|entry| entry.healthy).unwrap_or(true);
            let mut cooldown_until = base_entry.and_then(|entry| entry.cooldown_until);
            let mut last_success_at = base_entry.and_then(|entry| entry.last_success_at);
            let mut last_probe_at = base_entry.and_then(|entry| entry.last_probe_at);
            let mut probe_success_count = base_entry
                .map(|entry| entry.probe_success_count)
                .unwrap_or(0);
            let mut unhealthy_models: Vec<String> = Vec::new();
            if provider.per_model_circuit_break {
                for model in channel.models.keys() {
                    let Some(entry) = health.get(&health_key(&channel.id, Some(model))) else {
                        continue;
                    };
                    healthy &= entry.healthy;
                    if entry.cooldown_until.is_some_and(|until| until > now_ms) || !entry.healthy {
                        unhealthy_models.push(model.clone());
                    }
                    cooldown_until = match (cooldown_until, entry.cooldown_until) {
                        (Some(left), Some(right)) => Some(left.max(right)),
                        (Some(value), None) | (None, Some(value)) => Some(value),
                        (None, None) => None,
                    };
                    last_success_at = match (last_success_at, entry.last_success_at) {
                        (Some(left), Some(right)) => Some(left.max(right)),
                        (Some(value), None) | (None, Some(value)) => Some(value),
                        (None, None) => None,
                    };
                    last_probe_at = match (last_probe_at, entry.last_probe_at) {
                        (Some(left), Some(right)) => Some(left.max(right)),
                        (Some(value), None) | (None, Some(value)) => Some(value),
                        (None, None) => None,
                    };
                    probe_success_count = probe_success_count.max(entry.probe_success_count);
                }
                unhealthy_models.sort();
            }
            let today = channel_today.get(&channel.id);
            channel_health.push(json!({
                "provider_id": provider.id,
                "provider_name": provider.name,
                "channel_id": channel.id,
                "channel_name": channel.name,
                "enabled": channel.enabled,
                "session_affinity_auto": channel.session_affinity_auto.unwrap_or(false),
                "healthy": healthy,
                "last_success_at": last_success_at,
                "cooldown_until": cooldown_until,
                "probe_success_count": probe_success_count,
                "last_probe_at": last_probe_at,
                "cooldown_active": cooldown_until.is_some_and(|until| until > now_ms),
                "unhealthy_models": unhealthy_models,
                "today_calls": today.map(|row| row.today_calls).unwrap_or(0),
                "today_cost_nano_usd": today
                    .map(|row| row.today_cost_nano_usd.to_string())
                    .unwrap_or_else(|| "0".to_string()),
            }));
        }
    }
    drop(health);

    let stale_after_ms = (state.node.metering_ship_interval.as_millis() as i64)
        .saturating_mul(crate::replica::metering::HEARTBEAT_STALE_INTERVALS as i64);
    let now_unix_ms = now.timestamp_millis();
    crate::replica::metering::evict_expired_heartbeats(
        &state.replica_heartbeats,
        now_unix_ms,
        state.node.metering_ship_interval,
    );
    let mut replicas: Vec<Value> = state
        .replica_heartbeats
        .iter()
        .map(|entry| {
            let record = entry.value();
            let stale = now_unix_ms.saturating_sub(record.last_seen_unix_ms) > stale_after_ms;
            json!({
                "id": record.heartbeat.id,
                "hostname": record.heartbeat.hostname,
                "listen": record.heartbeat.listen,
                "version": record.heartbeat.version,
                "started_at": record.heartbeat.started_at,
                "last_seen_at": chrono::DateTime::<chrono::Utc>::from_timestamp_millis(record.last_seen_unix_ms)
                    .unwrap_or(now)
                    .to_rfc3339(),
                "uptime_seconds": record.heartbeat.uptime_seconds,
                "spool_pending_count": record.heartbeat.spool_pending_count,
                "spool_pending_bytes": record.heartbeat.spool_pending_bytes,
                "stale": stale,
            })
        })
        .collect();
    replicas.sort_by(|left, right| {
        let left_host = left.get("hostname").and_then(Value::as_str).unwrap_or("");
        let right_host = right.get("hostname").and_then(Value::as_str).unwrap_or("");
        left_host.cmp(right_host)
    });

    Ok(Json(json!({
        "node": {
            "role": role,
            "version": env!("CARGO_PKG_VERSION"),
            "started_at": started_at.to_rfc3339(),
            "uptime_seconds": uptime_seconds,
            "listen": state.runtime.listen,
            "metrics_path": state.runtime.metrics_path,
            "database_backend": database_backend,
            "database_dsn_redacted": dsn_redacted,
            "upstream_proxy_url": state.node.upstream_proxy_url,
        },
        "replica": {
            "ingest_enabled": state.metering_token_digest.is_some(),
            "spool_pending_count": spool_pending_count,
            "spool_pending_bytes": spool_pending_bytes,
            "replicas": replicas,
        },
        "system": {
            "pending_request_logs": state.pending_request_logs.len(),
            "sse_connections": sse_connections,
            "channel_health_entries": state.channel_health.lock().await.len(),
            "channel_affinity_entries": state.channel_affinity.lock().await.len(),
            "routing_config_revision": state.routing_config_revision
                .load(std::sync::atomic::Ordering::Relaxed)
                .to_string(),
        },
        "today": {
            "calls": today_calls,
            "cost_nano_usd": today_cost_nano_usd.to_string(),
        },
        "users_ranking": users_ranking,
        "channel_health": channel_health,
    })))
}

fn redact_dsn(dsn: &str) -> String {
    if let Some(at_pos) = dsn.find('@')
        && let Some(scheme_end) = dsn.find("://")
    {
        return format!("{}://***@{}", &dsn[..scheme_end], &dsn[at_pos + 1..]);
    }
    if dsn.starts_with("sqlite") {
        return dsn.to_string();
    }
    "***".to_string()
}

/// Admin usage ranking with per-user and per-model token totals.
pub async fn get_admin_usage_ranking(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    require_admin(&headers, &state).await?;
    let now = Utc::now();
    let time_to = now.to_rfc3339();
    let time_from = (now - chrono::Duration::hours(24)).to_rfc3339();
    let rows = state
        .user_store
        .get_users_model_usage_ranking(&time_from, &time_to)
        .await
        .map_err(|error| {
            AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", error)
        })?;
    let totals = state
        .user_store
        .get_admin_usage_totals(&time_from, &time_to)
        .await
        .map_err(|error| {
            AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", error)
        })?;

    struct UserAccumulator {
        user_id: String,
        username: Option<String>,
        call_count: i64,
        cost_nano_usd: i128,
        input_tokens: i128,
        cache_read_tokens: i128,
        output_tokens: i128,
        models: Vec<crate::users::UserModelUsageRankingRow>,
    }

    let aggregate_error = |message: &'static str| {
        AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
    };
    let mut users = Vec::<UserAccumulator>::new();
    let mut user_indices = HashMap::<String, usize>::new();
    for row in rows {
        if row.call_count < 0 {
            return Err(aggregate_error("usage call aggregate is negative"));
        }
        let index = if let Some(index) = user_indices.get(&row.user_id).copied() {
            index
        } else {
            let index = users.len();
            user_indices.insert(row.user_id.clone(), index);
            users.push(UserAccumulator {
                user_id: row.user_id.clone(),
                username: row.username.clone(),
                call_count: 0,
                cost_nano_usd: 0,
                input_tokens: 0,
                cache_read_tokens: 0,
                output_tokens: 0,
                models: Vec::new(),
            });
            index
        };
        let user = &mut users[index];
        user.call_count = user
            .call_count
            .checked_add(row.call_count)
            .ok_or_else(|| aggregate_error("usage call aggregate overflow"))?;
        user.cost_nano_usd = user
            .cost_nano_usd
            .checked_add(row.cost_nano_usd)
            .ok_or_else(|| aggregate_error("usage charge aggregate overflow"))?;
        user.input_tokens = user
            .input_tokens
            .checked_add(row.input_tokens)
            .ok_or_else(|| aggregate_error("usage input token aggregate overflow"))?;
        user.cache_read_tokens = user
            .cache_read_tokens
            .checked_add(row.cache_read_tokens)
            .ok_or_else(|| aggregate_error("usage cache-read token aggregate overflow"))?;
        user.output_tokens = user
            .output_tokens
            .checked_add(row.output_tokens)
            .ok_or_else(|| aggregate_error("usage output token aggregate overflow"))?;
        user.models.push(row);
    }
    users.sort_by(|left, right| {
        right
            .cost_nano_usd
            .cmp(&left.cost_nano_usd)
            .then_with(|| right.call_count.cmp(&left.call_count))
            .then_with(|| left.user_id.as_bytes().cmp(right.user_id.as_bytes()))
    });
    users.truncate(20);
    let users = users
        .into_iter()
        .map(|mut user| {
            sort_usage_models(&mut user.models);
            let models = user
                .models
                .into_iter()
                .map(|model| {
                    json!({
                        "model": model.model,
                        "call_count": model.call_count,
                        "cost_nano_usd": model.cost_nano_usd.to_string(),
                        "input_tokens": model.input_tokens.to_string(),
                        "cache_read_tokens": model.cache_read_tokens.to_string(),
                        "output_tokens": model.output_tokens.to_string(),
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "user_id": user.user_id,
                "username": user.username,
                "call_count": user.call_count,
                "cost_nano_usd": user.cost_nano_usd.to_string(),
                "input_tokens": user.input_tokens.to_string(),
                "cache_read_tokens": user.cache_read_tokens.to_string(),
                "output_tokens": user.output_tokens.to_string(),
                "models": models,
            })
        })
        .collect::<Vec<_>>();
    let total_tokens = totals
        .input_tokens
        .checked_add(totals.cache_read_tokens)
        .and_then(|value| value.checked_add(totals.output_tokens))
        .ok_or_else(|| aggregate_error("usage total token aggregate overflow"))?;

    Ok(Json(json!({
        "time_from": time_from,
        "time_to": time_to,
        "total_tokens": total_tokens.to_string(),
        "total_input_tokens": totals.input_tokens.to_string(),
        "total_cache_read_tokens": totals.cache_read_tokens.to_string(),
        "total_output_tokens": totals.output_tokens.to_string(),
        "total_calls": totals.call_count,
        "total_cost_nano_usd": totals.cost_nano_usd.to_string(),
        "users": users,
    })))
}

#[cfg(test)]
mod tests {
    use super::sort_usage_models;
    use crate::users::UserModelUsageRankingRow;

    #[test]
    fn usage_models_sort_by_total_tokens_then_calls_then_utf8_name() {
        let mut rows = vec![
            UserModelUsageRankingRow {
                user_id: "u".into(),
                username: None,
                model: "zeta".into(),
                call_count: 4,
                cost_nano_usd: 0,
                input_tokens: 10,
                cache_read_tokens: 0,
                output_tokens: 0,
            },
            UserModelUsageRankingRow {
                user_id: "u".into(),
                username: None,
                model: "alpha".into(),
                call_count: 4,
                cost_nano_usd: 0,
                input_tokens: 10,
                cache_read_tokens: 0,
                output_tokens: 0,
            },
            UserModelUsageRankingRow {
                user_id: "u".into(),
                username: None,
                model: "busy".into(),
                call_count: 9,
                cost_nano_usd: 0,
                input_tokens: 9,
                cache_read_tokens: 0,
                output_tokens: 0,
            },
        ];

        sort_usage_models(&mut rows);

        assert_eq!(
            rows.into_iter().map(|row| row.model).collect::<Vec<_>>(),
            ["alpha", "zeta", "busy"]
        );
    }
}
