//! AP-H1: project CRUD, versioned graph saves, run creation with admission.

use crate::error::{ApiError, ApiResult};
use crate::state::SharedState;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;

async fn require_user(
    state: &SharedState,
    headers: &HeaderMap,
) -> ApiResult<crate::auth::MirrorUser> {
    crate::auth::current_user(state, headers).await
}

pub async fn list(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let user = require_user(&state, &headers).await?;
    let rows = sqlx::query(
        "SELECT id, title, version, template_id, created_at, updated_at FROM projects \
         WHERE user_id = $1 ORDER BY updated_at DESC LIMIT 200",
    )
    .bind(&user.id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let projects: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            json!({
                "id": row.try_get::<String, _>("id").unwrap_or_default(),
                "title": row.try_get::<String, _>("title").unwrap_or_default(),
                "version": row.try_get::<i64, _>("version").unwrap_or(1),
                "template_id": row.try_get::<Option<String>, _>("template_id").ok().flatten(),
                "created_at": row.try_get::<String, _>("created_at").unwrap_or_default(),
                "updated_at": row.try_get::<String, _>("updated_at").unwrap_or_default(),
            })
        })
        .collect();
    Ok(Json(json!({ "projects": projects })))
}

#[derive(Deserialize)]
pub struct CreateBody {
    pub title: String,
    pub template_id: Option<String>,
}

pub async fn create(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(body): Json<CreateBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let user = require_user(&state, &headers).await?;
    let title = body.title.trim();
    if title.is_empty() || title.chars().count() > 120 {
        return Err(ApiError::bad_request("invalid_params", "title must be 1..=120 characters"));
    }
    let mut graph = json!({
        "nodes": [{
            "id": "script",
            "kind": "script",
            "x": 120.0,
            "y": 200.0,
            "params": {
                "system": "You are a professional video script writer.",
                "prompt": "",
            },
        }],
        "edges": [],
    });
    let mut template_id = None;
    if let Some(id) = &body.template_id {
        let row = sqlx::query(
            "SELECT graph_json FROM templates WHERE id = $1 AND enabled = 1",
        )
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("template"))?;
        let raw: String = row.try_get("graph_json").unwrap_or_default();
        let parsed: Value = serde_json::from_str(&raw)
            .map_err(|e| ApiError::internal(format!("template graph corrupt: {e}")))?;
        crate::graph::Graph::parse(&parsed.to_string())
            .and_then(|g| crate::graph::validate(&g))
            .map_err(|e| ApiError::bad_request("invalid_graph", e))?;
        graph = parsed;
        template_id = Some(id.clone());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::now_rfc3339();
    sqlx::query(
        "INSERT INTO projects (id, user_id, title, graph_json, version, template_id, \
         created_at, updated_at) VALUES ($1, $2, $3, $4, 1, $5, $6, $6)",
    )
    .bind(&id)
    .bind(&user.id)
    .bind(title)
    .bind(graph.to_string())
    .bind(template_id.clone())
    .bind(&now)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "title": title, "version": 1, "template_id": template_id })),
    ))
}

pub async fn get_project(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let user = require_user(&state, &headers).await?;
    let row = sqlx::query(
        "SELECT id, title, graph_json, version, template_id, created_at, updated_at \
         FROM projects WHERE id = $1 AND user_id = $2",
    )
    .bind(&id)
    .bind(&user.id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::not_found("project"))?;
    let graph_raw: String = row.try_get("graph_json").unwrap_or_default();
    let graph: Value = serde_json::from_str(&graph_raw).unwrap_or(json!({}));
    Ok(Json(json!({
        "id": row.try_get::<String, _>("id").unwrap_or_default(),
        "title": row.try_get::<String, _>("title").unwrap_or_default(),
        "graph": graph,
        "version": row.try_get::<i64, _>("version").unwrap_or(1),
        "template_id": row.try_get::<Option<String>, _>("template_id").ok().flatten(),
        "created_at": row.try_get::<String, _>("created_at").unwrap_or_default(),
        "updated_at": row.try_get::<String, _>("updated_at").unwrap_or_default(),
    })))
}

pub async fn delete_project(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let user = require_user(&state, &headers).await?;
    let outcome = sqlx::query("DELETE FROM projects WHERE id = $1 AND user_id = $2")
        .bind(&id)
        .bind(&user.id)
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if outcome.rows_affected() == 0 {
        return Err(ApiError::not_found("project"));
    }
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct SaveGraphBody {
    pub version: i64,
    pub graph: Value,
}

/// AP-D2: the save carries `current + 1`; anything older conflicts.
pub async fn save_graph(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<SaveGraphBody>,
) -> ApiResult<impl IntoResponse> {
    let user = require_user(&state, &headers).await?;
    let row = sqlx::query("SELECT version, graph_json FROM projects WHERE id = $1 AND user_id = $2")
        .bind(&id)
        .bind(&user.id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("project"))?;
    let current: i64 = row.try_get("version").unwrap_or(1);
    if body.version != current + 1 {
        let graph_raw: String = row.try_get("graph_json").unwrap_or_default();
        let graph: Value = serde_json::from_str(&graph_raw).unwrap_or(json!({}));
        return Ok((
            StatusCode::CONFLICT,
            Json(json!({
                "error": {
                    "code": "version_conflict",
                    "message": format!("expected version {}", current + 1),
                },
                "version": current,
                "graph": graph,
            })),
        ));
    }
    let graph =
        crate::graph::Graph::parse(&body.graph.to_string())
            .and_then(|g| crate::graph::validate(&g).map(|_| g))
            .map_err(|e| ApiError::bad_request("invalid_graph", e))?;
    let graph_json = serde_json::to_string(&graph)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let outcome = sqlx::query(
        "UPDATE projects SET graph_json = $1, version = $2, updated_at = $3 \
         WHERE id = $4 AND user_id = $5 AND version = $6",
    )
    .bind(&graph_json)
    .bind(body.version)
    .bind(crate::now_rfc3339())
    .bind(&id)
    .bind(&user.id)
    .bind(current)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if outcome.rows_affected() == 0 {
        return Ok((
            StatusCode::CONFLICT,
            Json(json!({
                "error": { "code": "version_conflict", "message": "concurrent modification" },
            })),
        ));
    }
    Ok((StatusCode::OK, Json(json!({ "version": body.version }))))
}

#[derive(Deserialize)]
pub struct RunBody {
    pub root: Option<String>,
}

pub async fn start_run(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<RunBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let user = require_user(&state, &headers).await?;
    let project = sqlx::query("SELECT id FROM projects WHERE id = $1 AND user_id = $2")
        .bind(&id)
        .bind(&user.id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("project"))?;
    let project_id: String = project.try_get("id").unwrap_or_default();
    if let Some(root) = &body.root {
        let graph = sqlx::query("SELECT graph_json FROM projects WHERE id = $1")
            .bind(&project_id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
        let raw: String = graph.try_get("graph_json").unwrap_or_default();
        let parsed = crate::graph::Graph::parse(&raw)
            .map_err(|e| ApiError::bad_request("invalid_graph", e))?;
        if parsed.node(root).is_none() {
            return Err(ApiError::bad_request("invalid_params", "root node not found"));
        }
    }
    let run = create_run(&state, &user.id, Some(&project_id), "graph", json!({ "root": body.root }))
        .await?;
    Ok((StatusCode::CREATED, Json(run)))
}

/// AP-E5 admission + run insert.
pub async fn create_run(
    state: &SharedState,
    user_id: &str,
    project_id: Option<&str>,
    kind: &str,
    params: Value,
) -> ApiResult<Value> {
    let user_active: i64 = sqlx::query_scalar::<sqlx::sqlite::Sqlite, i64>(
        "SELECT COUNT(*) FROM runs WHERE user_id = $1 AND status IN ('queued', 'running')",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if user_active >= state.cfg.user_active_runs {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "studio_busy",
            "too many active runs for this user",
        ));
    }
    let global_active: i64 = sqlx::query_scalar::<sqlx::sqlite::Sqlite, i64>(
        "SELECT COUNT(*) FROM runs WHERE status IN ('queued', 'running')",
    )
    .fetch_one(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    if global_active >= state.cfg.global_active_runs {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "studio_busy",
            "too many active runs on this instance",
        ));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::now_rfc3339();
    sqlx::query(
        "INSERT INTO runs (id, user_id, project_id, kind, status, params_json, created_at, \
         updated_at) VALUES ($1, $2, $3, $4, 'queued', $5, $6, $6)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(project_id)
    .bind(kind)
    .bind(params.to_string())
    .bind(&now)
    .execute(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(json!({
        "id": id,
        "user_id": user_id,
        "project_id": project_id,
        "kind": kind,
        "status": "queued",
        "params": params,
        "created_at": now,
        "updated_at": now,
    }))
}
