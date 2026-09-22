//! AP-H1: templates, node packs, node catalog, health.

use crate::error::{ApiError, ApiResult};
use crate::state::SharedState;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};
use sqlx::Row;

pub async fn healthz() -> impl IntoResponse {
    Json(json!({ "ok": true }))
}

pub async fn templates(
    State(state): axum::extract::State<SharedState>,
) -> ApiResult<impl IntoResponse> {
    let rows = sqlx::query(
        "SELECT id, source, name, description, graph_json, params_json, sort, created_at, \
         updated_at FROM templates WHERE enabled = 1 ORDER BY sort ASC, created_at ASC",
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
                "graph": serde_json::from_str::<Value>(
                    &row.try_get::<String, _>("graph_json").unwrap_or_default()
                ).unwrap_or(json!({})),
                "params": serde_json::from_str::<Value>(
                    &row.try_get::<String, _>("params_json").unwrap_or_default()
                ).unwrap_or(json!({})),
            })
        })
        .collect();
    Ok(Json(json!({ "templates": templates })))
}

pub async fn node_packs(
    State(state): axum::extract::State<SharedState>,
) -> ApiResult<impl IntoResponse> {
    let rows = sqlx::query(
        "SELECT id, name, version, description, nodes_json, enabled, source FROM node_packs \
         ORDER BY source ASC, id ASC",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let packs: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            json!({
                "id": row.try_get::<String, _>("id").unwrap_or_default(),
                "name": row.try_get::<String, _>("name").unwrap_or_default(),
                "version": row.try_get::<String, _>("version").unwrap_or_default(),
                "description": row.try_get::<String, _>("description").unwrap_or_default(),
                "nodes": serde_json::from_str::<Value>(
                    &row.try_get::<String, _>("nodes_json").unwrap_or_default()
                ).unwrap_or(json!([])),
                "enabled": row.try_get::<i64, _>("enabled").unwrap_or(1) != 0,
                "source": row.try_get::<String, _>("source").unwrap_or_default(),
            })
        })
        .collect();
    Ok(Json(json!({ "packs": packs })))
}

/// Node schema for the canvas palette (AP-G2 mirrored for the SPA).
fn node_schema(kind: &str) -> Value {
    let (name, inputs, output, params) = match kind {
        "note" => ("Note", json!([]), Value::Null, json!(["text"])),
        "script" => ("Script", json!([]), json!(["text"]), json!(["prompt", "system"])),
        "storyboard" => (
            "Storyboard",
            json!(["text"]),
            json!(["shots"]),
            json!(["count"]),
        ),
        "image" => (
            "Image",
            json!(["text"]),
            json!(["image"]),
            json!(["prompt", "size", "style"]),
        ),
        "video" => (
            "Video",
            json!(["text", "image"]),
            json!(["video"]),
            json!(["prompt", "seconds", "size"]),
        ),
        "tts" => (
            "Voice",
            json!(["text"]),
            json!(["audio"]),
            json!(["voice", "speed", "joiner"]),
        ),
        "subtitle" => ("Subtitle", json!(["text", "audio"]), json!(["subtitle"]), json!([])),
        "material" => ("Material", json!(["text"]), json!(["video"]), json!(["query", "url"])),
        "assemble" => (
            "Assemble",
            json!(["clips", "audio", "subtitle"]),
            json!(["video"]),
            json!(["resolution", "fps"]),
        ),
        _ => ("Unknown", json!([]), Value::Null, json!([])),
    };
    json!({ "kind": kind, "name": name, "inputs": inputs, "output": output, "params": params })
}

pub async fn node_catalog() -> impl IntoResponse {
    Json(json!({
        "nodes": crate::graph::NODE_KINDS.iter().map(|kind| node_schema(kind)).collect::<Vec<_>>(),
    }))
}
