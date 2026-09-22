//! AP-W1..W5: the Go worker's queue surface — claim, heartbeat, result, fail,
//! and input file streaming.

use crate::error::{ApiError, ApiResult};
use crate::state::SharedState;
use axum::body::Body;
use axum::extract::{Multipart, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;

fn worker_authorized(state: &SharedState, headers: &HeaderMap) -> bool {
    let Some(expected) = &state.cfg.worker_token else {
        return false;
    };
    let provided = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| raw.strip_prefix("Bearer "))
        .map(str::trim);
    match provided {
        Some(provided) => crate::sha256_hex(provided) == crate::sha256_hex(expected),
        None => false,
    }
}

fn require_worker(state: &SharedState, headers: &HeaderMap) -> Result<(), ApiError> {
    if worker_authorized(state, headers) {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "invalid worker token",
        ))
    }
}

#[derive(Deserialize)]
pub struct ClaimBody {
    pub worker_id: String,
    #[serde(default)]
    pub kinds: Option<Vec<String>>,
}

/// AP-W2: atomically move one queued job to `running` for this worker.
pub async fn claim(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(body): Json<ClaimBody>,
) -> ApiResult<impl IntoResponse> {
    require_worker(&state, &headers)?;
    if body.worker_id.trim().is_empty() {
        return Err(ApiError::bad_request("invalid_params", "worker_id is required"));
    }
    let candidate = if let Some(kinds) = &body.kinds.filter(|kinds| !kinds.is_empty()) {
        let placeholders: Vec<String> =
            kinds.iter().map(|kind| format!("'{}'", kind.replace('\'', ""))).collect();
        sqlx::query(&format!(
            "SELECT id, kind, payload_json FROM jobs WHERE status = 'queued' \
             AND kind IN ({}) ORDER BY created_at ASC LIMIT 1",
            placeholders.join(",")
        ))
        .fetch_optional(&state.db)
        .await
    } else {
        sqlx::query(
            "SELECT id, kind, payload_json FROM jobs WHERE status = 'queued' \
             ORDER BY created_at ASC LIMIT 1",
        )
        .fetch_optional(&state.db)
        .await
    }
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let Some(candidate) = candidate else {
        return Ok(Json(json!({ "job": Value::Null })));
    };
    let job_id: String = candidate.try_get("id").unwrap_or_default();
    let now = crate::now_rfc3339();
    let claimed = sqlx::query(
        "UPDATE jobs SET status = 'running', worker_id = $1, heartbeat_at = $2, \
         attempts = attempts + 1, updated_at = $2 WHERE id = $3 AND status = 'queued'",
    )
    .bind(&body.worker_id)
    .bind(&now)
    .bind(&job_id)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if claimed.rows_affected() == 0 {
        return Ok(Json(json!({ "job": Value::Null })));
    }
    let kind: String = candidate.try_get("kind").unwrap_or_default();
    let payload: Value = candidate
        .try_get::<String, _>("payload_json")
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_else(|| json!({}));
    let step_id: Option<String> = sqlx::query("SELECT step_id FROM jobs WHERE id = $1")
        .bind(&job_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
        .and_then(|row| row.try_get::<Option<String>, _>("step_id").ok().flatten());
    let run_id: Option<String> = sqlx::query("SELECT run_id FROM jobs WHERE id = $1")
        .bind(&job_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
        .and_then(|row| row.try_get::<Option<String>, _>("run_id").ok().flatten());
    Ok(Json(json!({
        "job": {
            "id": job_id,
            "kind": kind,
            "payload": payload,
            "step_id": step_id,
            "run_id": run_id,
        }
    })))
}

pub async fn heartbeat(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    require_worker(&state, &headers)?;
    let outcome = sqlx::query(
        "UPDATE jobs SET heartbeat_at = $1, status = 'running', updated_at = $1 WHERE id = $2",
    )
    .bind(crate::now_rfc3339())
    .bind(&id)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if outcome.rows_affected() == 0 {
        return Err(ApiError::not_found("job"));
    }
    Ok(Json(json!({ "ok": true })))
}

/// AP-W2: multipart `result` (JSON text) + optional `file`. A file becomes an
/// asset bound to the job's step; subtitle jobs may instead return the SRT in
/// `result.subtitle_srt`.
pub async fn result(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    mut multipart: Multipart,
) -> ApiResult<impl IntoResponse> {
    require_worker(&state, &headers)?;
    let job = sqlx::query(
        "SELECT id, step_id, run_id, kind, status, payload_json FROM jobs WHERE id = $1",
    )
    .bind(&id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::not_found("job"))?;
    let status: String = job.try_get("status").unwrap_or_default();
    if status != "running" {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "job_not_running",
            format!("job status is {status}"),
        ));
    }
    let step_id: String = job
        .try_get::<Option<String>, _>("step_id")
        .ok()
        .flatten()
        .unwrap_or_default();
    let run_id: String = job
        .try_get::<Option<String>, _>("run_id")
        .ok()
        .flatten()
        .unwrap_or_default();
    let kind: String = job.try_get("kind").unwrap_or_default();

    let mut result_json: Option<Value> = None;
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut file_mime: Option<String> = None;
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request("invalid_params", e.to_string()))?
    {
        match field.name() {
            Some("result") => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| ApiError::bad_request("invalid_params", e.to_string()))?;
                result_json = serde_json::from_str(&text).ok();
            }
            Some("file") => {
                file_mime = field.content_type().map(str::to_string);
                let mut buffer = Vec::new();
                while let Some(chunk) = field
                    .chunk()
                    .await
                    .map_err(|e| ApiError::bad_request("invalid_params", e.to_string()))?
                {
                    if (buffer.len() + chunk.len()) as u64 > crate::ASSET_MAX_BYTES {
                        return Err(ApiError::bad_request(
                            "invalid_params",
                            "file exceeds the asset cap",
                        ));
                    }
                    buffer.extend_from_slice(&chunk);
                }
                file_bytes = Some(buffer);
            }
            _ => {}
        }
    }
    let result_json = result_json.unwrap_or_else(|| json!({}));

    // Resolve the owning step + run before storing anything.
    let Some(step) = crate::engine::load_step(&state, &step_id)
        .await
        .map_err(ApiError::internal)?
    else {
        return Err(ApiError::not_found("step"));
    };
    let Some(run) = crate::engine::load_run(&state, &run_id)
        .await
        .map_err(ApiError::internal)?
    else {
        return Err(ApiError::not_found("run"));
    };
    if step.status != "running" {
        // The run was canceled while the worker held the job: drop the output.
        let _ = sqlx::query("UPDATE jobs SET status = 'succeeded', result_json = $1, \
             updated_at = $2 WHERE id = $3")
            .bind(result_json.to_string())
            .bind(crate::now_rfc3339())
            .bind(&id)
            .execute(&state.db)
            .await;
        return Ok(Json(json!({ "ok": true, "discarded": true })));
    }

    // Materialize the asset: uploaded file, or an inline subtitle payload.
    let mut step_result = json!({});
    let asset_kind = match kind.as_str() {
        "tts" => "audio",
        "material" => "video",
        "subtitle" => "subtitle",
        "assemble" => "video",
        other => return Err(ApiError::bad_request("invalid_params", format!("unknown job kind {other}"))),
    };
    if let Some(bytes) = file_bytes.filter(|bytes| !bytes.is_empty()) {
        let mime = file_mime
            .clone()
            .unwrap_or_else(|| match asset_kind {
                "audio" => "audio/mpeg".to_string(),
                "video" => "video/mp4".to_string(),
                "subtitle" => "application/x-subrip".to_string(),
                _ => "application/octet-stream".to_string(),
            });
        let asset = crate::assets::store_asset(
            &state,
            &run.user_id,
            Some(&run.id),
            Some(&step.id),
            asset_kind,
            &mime,
            &bytes,
            json!({ "job": id }),
        )
        .await
        .map_err(ApiError::internal)?;
        step_result["asset_id"] = json!(asset.id);
        step_result["mime_type"] = json!(asset.mime_type);
    } else if let Some(srt) = result_json.get("subtitle_srt").and_then(Value::as_str) {
        let asset = crate::assets::store_asset(
            &state,
            &run.user_id,
            Some(&run.id),
            Some(&step.id),
            "subtitle",
            "application/x-subrip",
            srt.as_bytes(),
            json!({ "job": id }),
        )
        .await
        .map_err(ApiError::internal)?;
        step_result["asset_id"] = json!(asset.id);
    }
    // Carry through worker-reported extras (durations, source urls).
    if let Some(extra) = result_json.get("extra").and_then(Value::as_object) {
        for (key, value) in extra {
            step_result[key] = value.clone();
        }
    }
    if let Some(shot_index) = step.payload.get("shot_index").and_then(Value::as_i64) {
        step_result["shot_index"] = json!(shot_index);
    }

    let now = crate::now_rfc3339();
    sqlx::query(
        "UPDATE jobs SET status = 'succeeded', result_json = $1, updated_at = $2 WHERE id = $3",
    )
    .bind(step_result.to_string())
    .bind(&now)
    .bind(&id)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    state.publish(
        &run.user_id,
        "job_update",
        json!({ "id": id, "kind": kind, "status": "succeeded" }),
    );
    crate::engine::finish_step(&state, &step, "succeeded", Some(step_result), None)
        .await
        .map_err(ApiError::internal)?;
    crate::engine::finalize_run(&state, &run_id).await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct FailBody {
    pub error: String,
}

pub async fn fail(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<FailBody>,
) -> ApiResult<impl IntoResponse> {
    require_worker(&state, &headers)?;
    let row = sqlx::query("SELECT step_id, run_id FROM jobs WHERE id = $1")
        .bind(&id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("job"))?;
    sqlx::query(
        "UPDATE jobs SET status = 'failed', error = $1, updated_at = $2 WHERE id = $3 \
         AND status = 'running'",
    )
    .bind(crate::short_error(&body.error))
    .bind(crate::now_rfc3339())
    .bind(&id)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let step_id: Option<String> = row.try_get("step_id").ok().flatten();
    let run_id: Option<String> = row.try_get("run_id").ok().flatten();
    if let (Some(step_id), Some(run_id)) = (step_id, run_id) {
        if let Ok(Some(step)) = crate::engine::load_step(&state, &step_id).await {
            if step.status == "running" {
                let _ = crate::engine::finish_step(
                    &state,
                    &step,
                    "failed",
                    None,
                    Some(&body.error),
                )
                .await;
            }
        }
        crate::engine::finalize_run(&state, &run_id).await;
    }
    Ok(Json(json!({ "ok": true })))
}

/// AP-W2: worker-scoped input file streaming (assemble clips, audio probes).
pub async fn file(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(asset_id): Path<String>,
) -> ApiResult<Response> {
    require_worker(&state, &headers)?;
    let asset = crate::assets::get_asset(&state, &asset_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("asset"))?;
    let path = std::path::Path::new(&state.cfg.assets_dir).join(&asset.file_path);
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|e| ApiError::internal(format!("open asset: {e}")))?;
    let stream = tokio_util::io::ReaderStream::new(file);
    Ok((
        StatusCode::OK,
        [
            ("content-type", asset.mime_type),
            ("cache-control", "private, no-cache".to_string()),
        ],
        Body::from_stream(stream),
    )
        .into_response())
}
