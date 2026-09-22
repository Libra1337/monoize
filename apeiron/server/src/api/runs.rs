//! AP-H1: one-click pipeline, run list/detail/cancel, cost estimates.

use crate::error::{ApiError, ApiResult};
use crate::state::SharedState;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;

#[derive(Deserialize)]
pub struct OneclickBody {
    pub topic: String,
    #[serde(default)]
    pub style: String,
    #[serde(default)]
    pub voice: String,
    #[serde(default = "default_duration")]
    pub duration_target_secs: u64,
    #[serde(default = "default_mode")]
    pub material_mode: String,
    #[serde(default = "default_true")]
    pub subtitle: bool,
    #[serde(default)]
    pub title: Option<String>,
    /// "16:9" (default) | "9:16" | "1:1"
    #[serde(default)]
    pub aspect: String,
    /// AP-AG1: "storyboard" stops the run after script+storyboard for
    /// per-shot review; absent runs the full pipeline.
    #[serde(default)]
    pub stop_after: Option<String>,
}

fn default_duration() -> u64 {
    30
}
fn default_mode() -> String {
    "stock".to_string()
}
fn default_true() -> bool {
    true
}

pub fn shot_count_for(duration_secs: u64) -> usize {
    ((duration_secs.max(1) + 4) / 5).clamp(1, 12) as usize
}

pub async fn oneclick(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(body): Json<OneclickBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let user = crate::auth::current_user(&state, &headers).await?;
    let topic = body.topic.trim();
    if topic.is_empty() || topic.chars().count() > 2000 {
        return Err(ApiError::bad_request(
            "invalid_params",
            "topic must be 1..=2000 characters",
        ));
    }
    if !matches!(body.material_mode.as_str(), "stock" | "ai_image") {
        return Err(ApiError::bad_request("invalid_params", "material_mode must be stock or ai_image"));
    }
    let shot_count = shot_count_for(body.duration_target_secs);
    let voice = if body.voice.is_empty() { "alloy".to_string() } else { body.voice };
    let resolution = match body.aspect.as_str() {
        "9:16" => "720x1280",
        "1:1" => "1080x1080",
        _ => "1280x720",
    };
    let graph = crate::seed::build_oneclick_graph(
        topic,
        &body.style,
        &voice,
        shot_count,
        &body.material_mode,
        body.subtitle,
        resolution,
    );
    let title: String = body
        .title
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| format!("{:.30}", topic.trim()))
        .trim()
        .to_string();
    let project_id = uuid::Uuid::new_v4().to_string();
    let now = crate::now_rfc3339();
    sqlx::query(
        "INSERT INTO projects (id, user_id, title, graph_json, version, template_id, \
         created_at, updated_at) VALUES ($1, $2, $3, $4, 1, 'oneclick-standard', $5, $5)",
    )
    .bind(&project_id)
    .bind(&user.id)
    .bind(&title)
    .bind(graph.to_string())
    .bind(&now)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let run = super::projects::create_run(
        &state,
        &user.id,
        Some(&project_id),
        "oneclick",
        json!({
            "topic": topic,
            "material_mode": body.material_mode,
            "subtitle": body.subtitle,
            "shot_count": shot_count,
            "root": "final",
            "stop_after": body.stop_after,
        }),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "project_id": project_id, "run": run, "graph": graph })),
    ))
}

#[derive(Deserialize)]
pub struct ContinueBody {
    pub project_id: String,
}

/// AP-AG1: continue a storyboard-reviewed project — a fresh full-pipeline
/// run of the project graph (the UI persists shot edits into the graph
/// before calling this).
pub async fn continue_run(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(body): Json<ContinueBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let user = crate::auth::current_user(&state, &headers).await?;
    let _ = sqlx::query("SELECT id FROM projects WHERE id = $1 AND user_id = $2")
        .bind(&body.project_id)
        .bind(&user.id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("project"))?;

    // Find the reviewed run (the most recent storyboard-gated run of this
    // project) so its script/storyboard results can be cloned forward.
    let previous: Option<String> = sqlx::query(
        "SELECT id FROM runs WHERE project_id = $1 AND user_id = $2 \
         AND (params_json LIKE '%\"stop_after\":\"storyboard\"%' \
              OR params_json LIKE '%\"stop_after\": \"storyboard\"%') \
         AND status IN ('partial', 'succeeded', 'failed') \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&body.project_id)
    .bind(&user.id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .and_then(|row| row.try_get::<String, _>("id").ok());

    let run = super::projects::create_run(
        &state,
        &user.id,
        Some(&body.project_id),
        "graph",
        json!({ "root": "final", "continue_from": previous }),
    )
    .await?;
    let run_id = run.get("id").and_then(Value::as_str).unwrap_or_default().to_string();

    // Clone the terminal script/storyboard steps of the reviewed run into
    // the new run as succeeded steps (same payloads and results) so the
    // engine skips them: no re-execution, no double LLM billing.
    if let Some(previous_id) = previous {
        let rows = sqlx::query(
            "SELECT node_id, kind, payload_json, result_json, charge_nano_usd              FROM steps WHERE run_id = $1 AND kind IN ('script', 'storyboard')              AND status = 'succeeded'",
        )
        .bind(&previous_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
        let now = crate::now_rfc3339();
        for row in rows {
            let step_id = uuid::Uuid::new_v4().to_string();
            let payload: String = row.try_get("payload_json").unwrap_or_default();
            let result: Option<String> = row.try_get::<Option<String>, _>("result_json").ok().flatten();
            let charge: Option<String> = row.try_get::<Option<String>, _>("charge_nano_usd").ok().flatten();
            let _ = sqlx::query(
                "INSERT INTO steps (id, run_id, node_id, kind, status, payload_json,                  result_json, charge_nano_usd, attempts, created_at, updated_at, finished_at)                  VALUES ($1, $2, $3, $4, 'succeeded', $5, $6, $7, 1, $8, $8, $8)",
            )
            .bind(&step_id)
            .bind(&run_id)
            .bind(row.try_get::<String, _>("node_id").unwrap_or_default())
            .bind(row.try_get::<String, _>("kind").unwrap_or_default())
            .bind(&payload)
            .bind(&result)
            .bind(&charge)
            .bind(&now)
            .execute(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        }
    }
    Ok((StatusCode::CREATED, Json(run)))
}

#[derive(Deserialize)]
pub struct EstimateQuery {
    pub seconds: Option<u64>,
    pub mode: Option<String>,
}

/// AP-U4: `estimate = clip_price × shot_count + tts + assemble (+ subtitle 0)`.
pub async fn estimate_oneclick(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(query): Query<EstimateQuery>,
) -> ApiResult<impl IntoResponse> {
    crate::auth::current_user(&state, &headers).await?;
    let seconds = query.seconds.unwrap_or(30).clamp(5, 300);
    let mode = query.mode.unwrap_or_else(|| "stock".to_string());
    let shots = shot_count_for(seconds) as i128;
    let clip_price = if mode == "ai_image" {
        crate::engine::step_price_nano(&state, "image").await
    } else {
        crate::engine::step_price_nano(&state, "material").await
    };
    let tts = crate::engine::step_price_nano(&state, "tts").await;
    let assemble = crate::engine::step_price_nano(&state, "assemble").await;
    let total = clip_price * shots + tts + assemble;
    Ok(Json(json!({
        "seconds": seconds,
        "shots": shots,
        "breakdown": {
            "clips": (clip_price * shots).to_string(),
            "tts": tts.to_string(),
            "assemble": assemble.to_string(),
        },
        "total_nano_usd": total.to_string(),
    })))
}

pub async fn list(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let user = crate::auth::current_user(&state, &headers).await?;
    let rows = sqlx::query(
        "SELECT id, user_id, project_id, kind, status, error, params_json, created_at, \
         updated_at, finished_at FROM runs WHERE user_id = $1 \
         ORDER BY created_at DESC LIMIT 100",
    )
    .bind(&user.id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let mut runs = Vec::new();
    for row in rows {
        let mut run = map_run_json(&row);
        let run_id: String = row.try_get("id").unwrap_or_default();
        let spend: Option<String> = sqlx::query(
            "SELECT SUM(CAST(charge_nano_usd AS INTEGER)) FROM steps WHERE run_id = $1 \
             AND charge_nano_usd IS NOT NULL AND refund_nano_usd IS NULL",
        )
        .bind(&run_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
        .and_then(|row| row.try_get::<Option<i64>, _>(0).ok().flatten())
        .map(|sum| sum.to_string());
        run["spend_nano_usd"] = json!(spend);
        runs.push(run);
    }
    Ok(Json(json!({ "runs": runs })))
}

fn map_run_json(row: &sqlx::sqlite::SqliteRow) -> Value {
    let params_raw: Option<String> = row.try_get("params_json").ok().flatten();
    json!({
        "id": row.try_get::<String, _>("id").unwrap_or_default(),
        "project_id": row.try_get::<Option<String>, _>("project_id").ok().flatten(),
        "kind": row.try_get::<String, _>("kind").unwrap_or_default(),
        "status": row.try_get::<String, _>("status").unwrap_or_default(),
        "error": row.try_get::<Option<String>, _>("error").ok().flatten(),
        "params": params_raw
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .unwrap_or_else(|| json!({})),
        "created_at": row.try_get::<String, _>("created_at").unwrap_or_default(),
        "updated_at": row.try_get::<String, _>("updated_at").unwrap_or_default(),
        "finished_at": row.try_get::<Option<String>, _>("finished_at").ok().flatten(),
    })
}

pub async fn get_run(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let user = crate::auth::current_user(&state, &headers).await?;
    let row = sqlx::query(
        "SELECT id, project_id, kind, status, error, params_json, created_at, updated_at, \
         finished_at FROM runs WHERE id = $1 AND user_id = $2",
    )
    .bind(&id)
    .bind(&user.id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::not_found("run"))?;
    let mut run = map_run_json(&row);
    let steps = crate::engine::load_run_steps(&state, &id)
        .await
        .map_err(ApiError::internal)?;
    run["steps"] = json!(steps.iter().map(|s| s.to_json()).collect::<Vec<_>>());
    // The run's deliverable: the newest video asset bound to an assemble step.
    let output = sqlx::query(
        "SELECT id, kind, mime_type, bytes, created_at, meta_json FROM assets \
         WHERE run_id = $1 AND kind = 'video' ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    run["output"] = match output {
        Some(row) => json!({
            "asset_id": row.try_get::<String, _>("id").unwrap_or_default(),
            "mime_type": row.try_get::<String, _>("mime_type").unwrap_or_default(),
            "bytes": row.try_get::<i64, _>("bytes").unwrap_or(0),
        }),
        None => Value::Null,
    };
    Ok(Json(run))
}

pub async fn cancel_run(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let user = crate::auth::current_user(&state, &headers).await?;
    let row = sqlx::query(
        "UPDATE runs SET status = 'canceled', updated_at = $1, finished_at = $1 \
         WHERE id = $2 AND user_id = $3 AND status IN ('queued', 'running') \
         RETURNING id",
    )
    .bind(crate::now_rfc3339())
    .bind(&id)
    .bind(&user.id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::not_found("run"))?;
    let _ = row;
    // Pending steps cancel immediately; running steps cancel with refund via
    // finish_step, and executor tasks observe the run status.
    let steps = crate::engine::load_run_steps(&state, &id)
        .await
        .map_err(ApiError::internal)?;
    for step in steps.iter().filter(|step| step.status == "pending") {
        let _ = sqlx::query(
            "UPDATE steps SET status = 'canceled', error = 'run canceled', \
             updated_at = $1, finished_at = $1 WHERE id = $2 AND status = 'pending'",
        )
        .bind(crate::now_rfc3339())
        .bind(&step.id)
        .execute(&state.db)
        .await;
    }
    for step in steps.iter().filter(|step| step.status == "running") {
        let _ = crate::engine::finish_step(&state, step, "canceled", None, Some("run canceled"))
            .await;
    }
    let _ = sqlx::query(
        "UPDATE jobs SET status = 'failed', error = 'run canceled', updated_at = $1 \
         WHERE run_id = $2 AND status IN ('queued', 'claimed')",
    )
    .bind(crate::now_rfc3339())
    .bind(&id)
    .execute(&state.db)
    .await;
    if let Ok(Some(updated)) = crate::engine::load_run(&state, &id).await {
        state.publish(&user.id, "run_update", updated.to_json());
    }
    Ok(Json(json!({ "ok": true })))
}
