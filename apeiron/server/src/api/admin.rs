//! AP-H2: operator surface — providers, price settings, run monitor, custom
//! templates.

use crate::error::{ApiError, ApiResult};
use crate::state::SharedState;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;

const PROVIDER_KINDS: &[&str] = &["llm", "image", "video", "tts", "material"];
const UPSTREAM_KINDS: &[&str] = &[
    "openai",
    "openai_video",
    "fal_video",
    "replicate",
    "openai_tts",
    "pexels",
];

fn provider_json(row: &sqlx::sqlite::SqliteRow) -> Value {
    json!({
        "id": row.try_get::<String, _>("id").unwrap_or_default(),
        "kind": row.try_get::<String, _>("kind").unwrap_or_default(),
        "upstream_kind": row.try_get::<String, _>("upstream_kind").unwrap_or_default(),
        "name": row.try_get::<String, _>("name").unwrap_or_default(),
        "base_url": row.try_get::<String, _>("base_url").unwrap_or_default(),
        "model": row.try_get::<Option<String>, _>("model").ok().flatten(),
        "params": serde_json::from_str::<Value>(
            &row.try_get::<String, _>("params_json").unwrap_or_default()
        ).unwrap_or(json!({})),
        "enabled": row.try_get::<i64, _>("enabled").unwrap_or(1) != 0,
        "weight": row.try_get::<i64, _>("weight").unwrap_or(0),
        "updated_at": row.try_get::<String, _>("updated_at").unwrap_or_default(),
        // Never return the raw key; a masked hint is enough for the editor.
        "api_key_set": !row.try_get::<String, _>("api_key").unwrap_or_default().is_empty(),
    })
}

pub async fn list_providers(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    crate::auth::require_admin(&state, &headers).await?;
    let rows = sqlx::query(
        "SELECT id, kind, upstream_kind, name, base_url, api_key, model, params_json, \
         enabled, weight, updated_at FROM providers ORDER BY kind ASC, weight DESC",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({
        "providers": rows.iter().map(provider_json).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
pub struct ProviderBody {
    pub kind: String,
    pub upstream_kind: String,
    pub name: String,
    pub base_url: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub params: Value,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub weight: i64,
}

fn default_true() -> bool {
    true
}

fn validate_provider(body: &ProviderBody) -> Result<(), ApiError> {
    if !PROVIDER_KINDS.contains(&body.kind.as_str()) {
        return Err(ApiError::bad_request("invalid_params", "unknown provider kind"));
    }
    if !UPSTREAM_KINDS.contains(&body.upstream_kind.as_str()) {
        return Err(ApiError::bad_request("invalid_params", "unknown upstream kind"));
    }
    if body.name.trim().is_empty() || body.name.chars().count() > 80 {
        return Err(ApiError::bad_request("invalid_params", "name must be 1..=80 characters"));
    }
    if !body.base_url.starts_with("http://") && !body.base_url.starts_with("https://") {
        return Err(ApiError::bad_request("invalid_params", "base_url must be an http(s) URL"));
    }
    if !body.params.is_object() {
        return Err(ApiError::bad_request("invalid_params", "params must be an object"));
    }
    Ok(())
}

pub async fn create_provider(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(body): Json<ProviderBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    crate::auth::require_admin(&state, &headers).await?;
    validate_provider(&body)?;
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::now_rfc3339();
    sqlx::query(
        "INSERT INTO providers (id, kind, upstream_kind, name, base_url, api_key, model, \
         params_json, enabled, weight, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11)",
    )
    .bind(&id)
    .bind(&body.kind)
    .bind(&body.upstream_kind)
    .bind(body.name.trim())
    .bind(body.base_url.trim().trim_end_matches('/'))
    .bind(body.api_key.unwrap_or_default())
    .bind(body.model.filter(|model| !model.trim().is_empty()))
    .bind(body.params.to_string())
    .bind(body.enabled as i64)
    .bind(body.weight)
    .bind(&now)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

pub async fn update_provider(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<ProviderBody>,
) -> ApiResult<impl IntoResponse> {
    crate::auth::require_admin(&state, &headers).await?;
    validate_provider(&body)?;
    // An empty api_key keeps the stored secret (masked editor round-trips).
    let outcome = if let Some(api_key) = body.api_key.filter(|key| !key.is_empty()) {
        sqlx::query(
            "UPDATE providers SET kind = $1, upstream_kind = $2, name = $3, base_url = $4, \
             model = $5, params_json = $6, enabled = $7, weight = $8, updated_at = $9, \
             api_key = $10 WHERE id = $11",
        )
        .bind(&body.kind)
        .bind(&body.upstream_kind)
        .bind(body.name.trim())
        .bind(body.base_url.trim().trim_end_matches('/'))
        .bind(body.model.filter(|model| !model.trim().is_empty()))
        .bind(body.params.to_string())
        .bind(body.enabled as i64)
        .bind(body.weight)
        .bind(crate::now_rfc3339())
        .bind(api_key)
        .bind(&id)
        .execute(&state.db)
        .await
    } else {
        sqlx::query(
            "UPDATE providers SET kind = $1, upstream_kind = $2, name = $3, base_url = $4, \
             model = $5, params_json = $6, enabled = $7, weight = $8, updated_at = $9 \
             WHERE id = $10",
        )
        .bind(&body.kind)
        .bind(&body.upstream_kind)
        .bind(body.name.trim())
        .bind(body.base_url.trim().trim_end_matches('/'))
        .bind(body.model.filter(|model| !model.trim().is_empty()))
        .bind(body.params.to_string())
        .bind(body.enabled as i64)
        .bind(body.weight)
        .bind(crate::now_rfc3339())
        .bind(&id)
        .execute(&state.db)
        .await
    };
    let outcome = outcome.map_err(|e| ApiError::internal(e.to_string()))?;
    if outcome.rows_affected() == 0 {
        return Err(ApiError::not_found("provider"));
    }
    Ok(Json(json!({ "ok": true })))
}

pub async fn delete_provider(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    crate::auth::require_admin(&state, &headers).await?;
    let outcome = sqlx::query("DELETE FROM providers WHERE id = $1")
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if outcome.rows_affected() == 0 {
        return Err(ApiError::not_found("provider"));
    }
    Ok(Json(json!({ "ok": true })))
}

const PRICE_KEYS: &[(&str, &str)] = &[
    ("price_image_nano", "4000000"),
    ("price_video_nano", "500000000"),
    ("price_tts_nano", "20000000"),
    ("price_assemble_nano", "10000000"),
    ("price_material_nano", "0"),
    ("price_subtitle_nano", "0"),
];

pub async fn get_settings(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    crate::auth::require_admin(&state, &headers).await?;
    let mut prices = serde_json::Map::new();
    for (key, default) in PRICE_KEYS {
        let value = crate::engine::get_setting(&state, key, default).await;
        prices.insert(key.to_string(), json!(value));
    }
    Ok(Json(json!({ "prices": prices })))
}

pub async fn put_settings(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<impl IntoResponse> {
    crate::auth::require_admin(&state, &headers).await?;
    let Some(prices) = body.get("prices").and_then(Value::as_object) else {
        return Err(ApiError::bad_request("invalid_params", "prices object is required"));
    };
    for (key, default) in PRICE_KEYS {
        if let Some(value) = prices.get(*key) {
            let raw = match value {
                Value::String(raw) => raw.clone(),
                Value::Number(number) => number.to_string(),
                _ => {
                    return Err(ApiError::bad_request(
                        "invalid_params",
                        format!("{key} must be a nano-USD integer"),
                    ))
                }
            };
            let parsed = crate::parse_nano(&raw);
            if parsed < 0 || parsed > 100_000_000_000_000 {
                return Err(ApiError::bad_request(
                    "invalid_params",
                    format!("{key} out of range"),
                ));
            }
            crate::engine::set_setting(&state, key, &parsed.to_string())
                .await
                .map_err(ApiError::internal)?;
        }
        let _ = default;
    }
    Ok(Json(json!({ "ok": true })))
}

pub async fn list_runs(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    crate::auth::require_admin(&state, &headers).await?;
    let rows = sqlx::query(
        "SELECT id, user_id, project_id, kind, status, error, created_at, updated_at, \
         finished_at FROM runs ORDER BY created_at DESC LIMIT 200",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let runs: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            json!({
                "id": row.try_get::<String, _>("id").unwrap_or_default(),
                "user_id": row.try_get::<String, _>("user_id").unwrap_or_default(),
                "project_id": row.try_get::<Option<String>, _>("project_id").ok().flatten(),
                "kind": row.try_get::<String, _>("kind").unwrap_or_default(),
                "status": row.try_get::<String, _>("status").unwrap_or_default(),
                "error": row.try_get::<Option<String>, _>("error").ok().flatten(),
                "created_at": row.try_get::<String, _>("created_at").unwrap_or_default(),
                "finished_at": row.try_get::<Option<String>, _>("finished_at").ok().flatten(),
            })
        })
        .collect();
    Ok(Json(json!({ "runs": runs })))
}

pub async fn cancel_run(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    crate::auth::require_admin(&state, &headers).await?;
    let outcome = sqlx::query(
        "UPDATE runs SET status = 'canceled', updated_at = $1, finished_at = $1 \
         WHERE id = $2 AND status IN ('queued', 'running')",
    )
    .bind(crate::now_rfc3339())
    .bind(&id)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if outcome.rows_affected() == 0 {
        return Err(ApiError::not_found("active run"));
    }
    let _ = sqlx::query(
        "UPDATE steps SET status = 'canceled', error = 'run canceled', \
         updated_at = $1, finished_at = $1 WHERE run_id = $2 AND status = 'pending'",
    )
    .bind(crate::now_rfc3339())
    .bind(&id)
    .execute(&state.db)
    .await;
    Ok(Json(json!({ "ok": true })))
}

pub async fn list_templates_admin(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    crate::auth::require_admin(&state, &headers).await?;
    let rows = sqlx::query(
        "SELECT id, source, name, description, enabled, sort FROM templates \
         ORDER BY sort ASC, created_at ASC",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let templates: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            json!({
                "id": row.try_get::<String, _>("id").unwrap_or_default(),
                "source": row.try_get::<String, _>("source").unwrap_or_default(),
                "name": row.try_get::<String, _>("name").unwrap_or_default(),
                "description": row.try_get::<String, _>("description").unwrap_or_default(),
                "enabled": row.try_get::<i64, _>("enabled").unwrap_or(1) != 0,
                "sort": row.try_get::<i64, _>("sort").unwrap_or(0),
            })
        })
        .collect();
    Ok(Json(json!({ "templates": templates })))
}

#[derive(Deserialize)]
pub struct TemplateBody {
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub graph: Value,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub sort: i64,
}

pub async fn create_template(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(body): Json<TemplateBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    crate::auth::require_admin(&state, &headers).await?;
    let graph = crate::graph::Graph::parse(&body.graph.to_string())
        .and_then(|g| crate::graph::validate(&g).map(|_| g))
        .map_err(|e| ApiError::bad_request("invalid_graph", e))?;
    let id = body
        .id
        .filter(|id| !id.trim().is_empty())
        .unwrap_or_else(|| format!("custom-{}", uuid::Uuid::new_v4().simple()));
    let now = crate::now_rfc3339();
    sqlx::query(
        "INSERT INTO templates (id, source, name, description, graph_json, params_json, \
         enabled, sort, created_at, updated_at) \
         VALUES ($1, 'custom', $2, $3, $4, '{}', $5, $6, $7, $7) \
         ON CONFLICT (id) DO UPDATE SET name = excluded.name, \
         description = excluded.description, graph_json = excluded.graph_json, \
         enabled = excluded.enabled, sort = excluded.sort, updated_at = excluded.updated_at",
    )
    .bind(&id)
    .bind(body.name.trim())
    .bind(&body.description)
    .bind(serde_json::to_string(&graph).unwrap_or_else(|_| "{}".into()))
    .bind(body.enabled as i64)
    .bind(body.sort)
    .bind(&now)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

pub async fn update_template(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<TemplateBody>,
) -> ApiResult<impl IntoResponse> {
    crate::auth::require_admin(&state, &headers).await?;
    let graph = crate::graph::Graph::parse(&body.graph.to_string())
        .and_then(|g| crate::graph::validate(&g).map(|_| g))
        .map_err(|e| ApiError::bad_request("invalid_graph", e))?;
    let outcome = sqlx::query(
        "UPDATE templates SET name = $1, description = $2, graph_json = $3, enabled = $4, \
         sort = $5, updated_at = $6 WHERE id = $7",
    )
    .bind(body.name.trim())
    .bind(&body.description)
    .bind(serde_json::to_string(&graph).unwrap_or_else(|_| "{}".into()))
    .bind(body.enabled as i64)
    .bind(body.sort)
    .bind(crate::now_rfc3339())
    .bind(&id)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if outcome.rows_affected() == 0 {
        return Err(ApiError::not_found("template"));
    }
    Ok(Json(json!({ "ok": true })))
}

pub async fn delete_template(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    crate::auth::require_admin(&state, &headers).await?;
    let outcome = sqlx::query(
        "DELETE FROM templates WHERE id = $1 AND source = 'custom'",
    )
    .bind(&id)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if outcome.rows_affected() == 0 {
        return Err(ApiError::not_found(
            "custom template (builtin templates cannot be deleted)",
        ));
    }
    Ok(Json(json!({ "ok": true })))
}
