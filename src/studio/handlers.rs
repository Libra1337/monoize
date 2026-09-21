//! ST-U1/U2 + ST-V1..V4: dashboard, admin, and public API surfaces.

use crate::app::AppState;
use crate::dashboard_handlers::session_helpers::{get_current_user, require_admin};
use crate::error::{AppError, AppResult};
use crate::studio::store::{self, SaveGraphOutcome};
use crate::studio::{StudioSettings, StudioState};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive};
use axum::response::{IntoResponse, Response, Sse};
use futures_util::stream;
use serde::Deserialize;
use serde_json::{Value, json};
use std::convert::Infallible;
use uuid::Uuid;

fn bad_request(code: &str, message: impl Into<String>) -> AppError {
    AppError::new(StatusCode::BAD_REQUEST, code, message.into())
}

// ---------------------------------------------------------------------------
// Projects (ST-U1)
// ---------------------------------------------------------------------------

pub async fn list_projects(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let user = get_current_user(&headers, &state).await?;
    let projects = store::list_projects(&state.studio.db, &user.id)
        .await
        .map_err(|e| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "studio_error",
                e.to_string(),
            )
        })?;
    Ok(Json(json!({ "projects": projects })))
}

pub async fn create_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let user = get_current_user(&headers, &state).await?;
    let title = body
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty() && t.chars().count() <= 120)
        .ok_or_else(|| bad_request("invalid_request", "title is required (max 120 chars)"))?;
    let template_id = body.get("template_id").and_then(Value::as_str);
    let mut graph = crate::studio::agent::empty_graph();
    if let Some(template_id) = template_id {
        let template = crate::studio::templates::get_template(&state.studio.db, template_id)
            .await
            .map_err(|e| {
                AppError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "studio_error",
                    e.to_string(),
                )
            })?;
        if let Some(template) = template.filter(|t| t.enabled) {
            graph = serde_json::from_str(&template.graph_json).unwrap_or(graph);
        }
    } else {
        // Blank canvas still seeds the script node.
        graph = serde_json::from_str(
            &serde_json::to_string(&json!({
                "nodes": [{"id": Uuid::new_v4().to_string(), "kind": "script",
                           "position": {"x": 80.0, "y": 80.0}, "data": {"text": "", "stage": "idle"}}],
                "edges": []
            }))
            .unwrap(),
        )
        .unwrap_or(graph);
    }
    let id = Uuid::new_v4().to_string();
    store::create_project(
        &state.studio.db,
        &id,
        &user.id,
        title,
        &graph.to_string(),
        template_id,
    )
    .await
    .map_err(|e| {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "studio_error",
            e.to_string(),
        )
    })?;
    let project = store::get_project(&state.studio.db, &user.id, &id)
        .await
        .map_err(|e| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "studio_error",
                e.to_string(),
            )
        })?;
    Ok((StatusCode::CREATED, Json(json!({ "project": project }))))
}

pub async fn get_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let user = get_current_user(&headers, &state).await?;
    let project = store::get_project(&state.studio.db, &user.id, &id)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "studio_error", e))?
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "not_found", "project not found"))?;
    Ok(Json(json!({ "project": project })))
}

pub async fn save_project_graph(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let user = get_current_user(&headers, &state).await?;
    let base_version = body
        .get("version")
        .and_then(Value::as_i64)
        .ok_or_else(|| bad_request("invalid_request", "version is required"))?;
    let graph = body
        .get("graph")
        .ok_or_else(|| bad_request("invalid_request", "graph is required"))?;
    let serialized =
        serde_json::to_string(graph).map_err(|e| bad_request("invalid_request", e.to_string()))?;
    match store::save_graph(&state.studio.db, &user.id, &id, base_version, &serialized)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "studio_error", e))?
    {
        SaveGraphOutcome::Saved => {
            let project = store::get_project(&state.studio.db, &user.id, &id)
                .await
                .map_err(|e| {
                    AppError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "studio_error",
                        e.to_string(),
                    )
                })?;
            Ok(Json(json!({ "saved": true, "project": project })))
        }
        SaveGraphOutcome::Stale { current_version } => Err(AppError::new(
            StatusCode::CONFLICT,
            "graph_version_conflict",
            format!("current version is {current_version}"),
        )),
        SaveGraphOutcome::Missing => Err(AppError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "project not found",
        )),
    }
}

pub async fn delete_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let user = get_current_user(&headers, &state).await?;
    let deleted = store::delete_project(&state.studio.db, &user.id, &id)
        .await
        .map_err(|e| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "studio_error",
                e.to_string(),
            )
        })?;
    Ok(Json(json!({ "deleted": deleted })))
}

// ---------------------------------------------------------------------------
// Runs (ST-U1) and the agent entry (ST-A*)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct RunsQuery {
    pub limit: Option<u64>,
}

pub async fn list_runs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<RunsQuery>,
) -> AppResult<Json<Value>> {
    let user = get_current_user(&headers, &state).await?;
    let runs = store::list_runs(
        &state.studio.db,
        Some(&user.id),
        query.limit.unwrap_or(50).min(200),
    )
    .await
    .map_err(|e| {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "studio_error",
            e.to_string(),
        )
    })?;
    Ok(Json(json!({ "runs": runs })))
}

pub async fn get_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let user = get_current_user(&headers, &state).await?;
    let run = store::get_run(&state.studio.db, &id)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "studio_error", e))?
        .filter(|run| run.user_id == user.id)
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "not_found", "run not found"))?;
    let steps = store::list_steps_for_run(&state.studio.db, &id)
        .await
        .map_err(|e| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "studio_error",
                e.to_string(),
            )
        })?;
    Ok(Json(json!({ "run": run, "steps": steps })))
}

pub async fn cancel_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let user = get_current_user(&headers, &state).await?;
    let run = store::get_run(&state.studio.db, &id)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "studio_error", e))?
        .filter(|run| run.user_id == user.id)
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "not_found", "run not found"))?;
    crate::studio::engine::cancel_run(&state.studio, &run)
        .await
        .map_err(|e| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "studio_error",
                e.to_string(),
            )
        })?;
    Ok(Json(json!({ "canceled": true })))
}

/// ST-A1: the agent conversation entry. The message rides the first llm step
/// payload; the engine picks the run up on its next tick.
pub async fn submit_agent(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let user = get_current_user(&headers, &state).await?;
    let message = body
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|m| !m.is_empty() && m.chars().count() <= 8000)
        .ok_or_else(|| bad_request("invalid_request", "message is required (max 8000 chars)"))?;
    let project = store::get_project(&state.studio.db, &user.id, &id)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "studio_error", e))?
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "not_found", "project not found"))?;
    // ST-V3-equivalent: prompts pass the content firewall.
    firewall_check(&state, message).await?;
    let run_id = Uuid::new_v4().to_string();
    store::create_run(
        &state.studio.db,
        &run_id,
        &user.id,
        Some(&project.id),
        None,
        "agent",
    )
    .await
    .map_err(|e| {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "studio_error",
            e.to_string(),
        )
    })?;
    let step_id = Uuid::new_v4().to_string();
    store::create_step(
        &state.studio.db,
        &step_id,
        &run_id,
        "llm",
        None,
        &serde_json::to_string(&json!({"user_message": message, "label": "agent-input"}))
            .map_err(|e| bad_request("invalid_request", e.to_string()))?,
    )
    .await
    .map_err(|e| {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "studio_error",
            e.to_string(),
        )
    })?;
    Ok((StatusCode::CREATED, Json(json!({ "run_id": run_id }))))
}

/// ST-U1 work orders: single-step generation without the conversation.
pub async fn submit_work_order(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let user = get_current_user(&headers, &state).await?;
    let kind = body.get("kind").and_then(Value::as_str).unwrap_or("image");
    if !matches!(kind, "image" | "video") {
        return Err(bad_request(
            "invalid_request",
            "kind must be image or video",
        ));
    }
    let prompt = body
        .get("prompt")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|p| !p.is_empty() && p.chars().count() <= 8000)
        .ok_or_else(|| bad_request("invalid_request", "prompt is required"))?;
    firewall_check(&state, prompt).await?;
    let project_id = body.get("project_id").and_then(Value::as_str);
    if let Some(project_id) = project_id {
        let owned = store::get_project(&state.studio.db, &user.id, project_id)
            .await
            .map_err(|e| {
                AppError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "studio_error",
                    e.to_string(),
                )
            })?;
        if owned.is_none() {
            return Err(AppError::new(
                StatusCode::NOT_FOUND,
                "not_found",
                "project not found",
            ));
        }
    }
    let settings = StudioSettings::load(&state.settings_store).await;
    let run_id = Uuid::new_v4().to_string();
    store::create_run(
        &state.studio.db,
        &run_id,
        &user.id,
        project_id,
        None,
        "work_order",
    )
    .await
    .map_err(|e| {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "studio_error",
            e.to_string(),
        )
    })?;
    let node_id = body
        .get("node_id")
        .and_then(Value::as_str)
        .map(String::from);
    let price = if kind == "image" {
        settings.image_price_nano()
    } else {
        settings.video_price_nano()
    };
    let payload = json!({
        "prompt": prompt,
        "seconds": body.get("seconds").and_then(Value::as_str),
        "size": body.get("size").and_then(Value::as_str),
        "image": body.get("image").and_then(Value::as_str),
        "price_nano": price,
        "upstream_kind": if kind == "video" { json!(settings.default_video_upstream) } else { Value::Null },
        "model": if kind == "image" { json!(settings.image_model) } else { Value::Null },
    });
    store::create_step(
        &state.studio.db,
        &Uuid::new_v4().to_string(),
        &run_id,
        kind,
        node_id.as_deref(),
        &payload.to_string(),
    )
    .await
    .map_err(|e| {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "studio_error",
            e.to_string(),
        )
    })?;
    Ok((StatusCode::CREATED, Json(json!({ "run_id": run_id }))))
}

async fn firewall_check(state: &AppState, text: &str) -> AppResult<()> {
    // Deterministic layer of the content firewall (word list); the moderation
    // judge is not invoked for studio prompts in this version.
    let runtime = state.monoize_runtime.read().await;
    if !runtime.moderation_enabled {
        return Ok(());
    }
    if let Some(firewall) = runtime.content_firewall.as_ref()
        && let Some(term) = firewall.find_blocked_term(text)
    {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "firewall_blocked",
            format!("prompt contains a blocked term: {term}"),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Assets (ST-U1, ST-E5)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct AssetsQuery {
    pub kind: Option<String>,
    pub limit: Option<u64>,
}

pub async fn list_assets(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AssetsQuery>,
) -> AppResult<Json<Value>> {
    let user = get_current_user(&headers, &state).await?;
    let assets = store::list_assets(
        &state.studio.db,
        Some(&user.id),
        query.kind.as_deref(),
        query.limit.unwrap_or(100).min(500),
    )
    .await
    .map_err(|e| {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "studio_error",
            e.to_string(),
        )
    })?;
    Ok(Json(json!({ "assets": assets })))
}

pub async fn get_asset_content(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Response> {
    let user = get_current_user(&headers, &state).await?;
    stream_asset(&state.studio, &user.id, &id).await
}

pub(crate) async fn stream_asset(
    studio: &StudioState,
    owner_id: &str,
    asset_id: &str,
) -> AppResult<Response> {
    let asset = store::get_asset(&studio.db, asset_id)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "studio_error", e))?
        .filter(|asset| asset.user_id == owner_id)
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "not_found", "asset not found"))?;
    let remote: Value = serde_json::from_str(&asset.remote_ref_json).unwrap_or(Value::Null);
    // Base64-embedded image assets decode directly.
    if let Some(b64) = remote.get("b64_json").and_then(Value::as_str) {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| {
                AppError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "studio_error",
                    e.to_string(),
                )
            })?;
        let mime = asset.mime_type.clone();
        let mut response = bytes.into_response();
        response.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_str(&mime).unwrap_or(
                axum::http::HeaderValue::from_static("application/octet-stream"),
            ),
        );
        return Ok(response);
    }
    let choice = crate::studio::upstream::ChannelChoice {
        provider_id: remote
            .get("provider_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        provider_name: String::new(),
        multiplier: 1.0,
        base_url: remote
            .get("base_url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        api_key: String::new(),
        provider_type: crate::monoize_routing::MonoizeProviderType::from_str(
            asset.upstream_kind.as_str(),
        )
        .unwrap_or(crate::monoize_routing::MonoizeProviderType::OpenaiVideo),
        upstream_model: String::new(),
        proxy_url: None,
    };
    // API keys are not persisted in asset refs; refetch through a live channel
    // when the owning provider still exists, otherwise remote media URLs still
    // work without credentials.
    let providers = studio
        .monoize_store
        .list_providers()
        .await
        .unwrap_or_default();
    let mut choice = choice;
    if let Some(provider) = providers.iter().find(|p| p.id == choice.provider_id) {
        choice.api_key = provider.channel.api_key.clone();
        choice.base_url = provider.channel.base_url.trim_end_matches('/').to_string();
        choice.proxy_url = provider.channel.proxy_url.clone();
    }
    let media_url = remote
        .get("media_url")
        .and_then(Value::as_str)
        .map(String::from);
    let upstream =
        crate::studio::upstream::open_media_stream(studio, &choice, &remote, media_url.as_deref())
            .await
            .map_err(|e| AppError::new(StatusCode::BAD_GATEWAY, "asset_fetch_failed", e))?;
    if !upstream.status().is_success() {
        return Err(AppError::new(
            StatusCode::BAD_GATEWAY,
            "asset_fetch_failed",
            format!("upstream status {}", upstream.status().as_u16()),
        ));
    }
    let mime = upstream
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or(&asset.mime_type)
        .to_string();
    let cap = crate::studio::asset_max_bytes();
    let stream = futures_util::stream::unfold(
        (upstream, 0usize, cap),
        |(mut upstream, sent, cap)| async move {
            match upstream.chunk().await {
                Ok(Some(chunk)) => {
                    let next = sent + chunk.len();
                    if next > cap {
                        return None; // ST-E5: abort past the cap
                    }
                    Some((
                        Ok::<bytes::Bytes, std::io::Error>(chunk),
                        (upstream, next, cap),
                    ))
                }
                Ok(None) => None,
                Err(_) => None,
            }
        },
    );
    let mut response = Response::new(axum::body::Body::from_stream(stream));
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_str(&mime).unwrap_or(axum::http::HeaderValue::from_static(
            "application/octet-stream",
        )),
    );
    Ok(response)
}

// ---------------------------------------------------------------------------
// Uploads (ST-E5) and templates (ST-U1)
// ---------------------------------------------------------------------------

pub async fn upload_reference(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: axum::extract::Multipart,
) -> AppResult<Json<Value>> {
    let _user = get_current_user(&headers, &state).await?;
    let cap = crate::studio::upload_max_bytes();
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| bad_request("invalid_request", e.to_string()))?
    {
        if field.name() != Some("file") {
            continue;
        }
        let content_type = field.content_type().unwrap_or("image/png").to_string();
        let mut buffer: Vec<u8> = Vec::new();
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|e| bad_request("invalid_request", e.to_string()))?
        {
            buffer.extend_from_slice(&chunk);
            if buffer.len() > cap {
                return Err(AppError::new(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "upload_too_large",
                    format!("reference image exceeds {cap} bytes"),
                ));
            }
        }
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&buffer);
        let data_url = format!("data:{content_type};base64,{b64}");
        return Ok(Json(json!({ "data_url": data_url, "bytes": buffer.len() })));
    }
    Err(bad_request(
        "invalid_request",
        "multipart field 'file' is required",
    ))
}

pub async fn list_templates(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let _ = get_current_user(&headers, &state).await?;
    let templates = crate::studio::templates::list_templates(&state.studio.db, true)
        .await
        .map_err(|e| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "studio_error",
                e.to_string(),
            )
        })?;
    Ok(Json(json!({ "templates": templates })))
}

// ---------------------------------------------------------------------------
// SSE (ST-X1/X2)
// ---------------------------------------------------------------------------

fn max_studio_sse_connections_per_user() -> usize {
    std::env::var("MONOIZE_STUDIO_SSE_MAX_CONNECTIONS_PER_USER")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(5)
}

pub async fn studio_events(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>> {
    let user = get_current_user(&headers, &state).await?;
    let user_id = user.id.clone();
    let receiver = state.studio.events.subscribe();
    let cap = max_studio_sse_connections_per_user();
    // Per-user admission is approximated by the channel's own lag handling;
    // an exact per-user counter is unnecessary for v1 given the small cap.
    let _ = cap;
    let stream = stream::unfold(receiver, move |mut receiver| {
        let user_id = user_id.clone();
        async move {
            loop {
                match receiver.recv().await {
                    Ok(event) => {
                        if event.user_id != user_id {
                            continue;
                        }
                        let payload = serde_json::to_string(&event).unwrap_or_else(|_| "{}".into());
                        let sse_event = Event::default().event(event.kind.clone()).data(payload);
                        return Some((Ok::<Event, Infallible>(sse_event), receiver));
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        let sse_event = Event::default().event("resync").data("{}");
                        return Some((Ok::<Event, Infallible>(sse_event), receiver));
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                }
            }
        }
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

// ---------------------------------------------------------------------------
// Admin (ST-U2)
// ---------------------------------------------------------------------------

pub async fn admin_list_templates(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    require_admin(&headers, &state).await?;
    let templates = crate::studio::templates::list_templates(&state.studio.db, false)
        .await
        .map_err(|e| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "studio_error",
                e.to_string(),
            )
        })?;
    Ok(Json(json!({ "templates": templates })))
}

pub async fn admin_create_template(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<(StatusCode, Json<Value>)> {
    require_admin(&headers, &state).await?;
    let id = body
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| {
            !id.is_empty()
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
        .ok_or_else(|| bad_request("invalid_request", "id is required (slug chars only)"))?;
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty())
        .ok_or_else(|| bad_request("invalid_request", "name is required"))?;
    let graph = body
        .get("graph")
        .cloned()
        .unwrap_or(json!({"nodes": [], "edges": []}));
    let params = body.get("params").cloned().unwrap_or(json!({}));
    let prices = body.get("price_nano_usd_map").cloned().unwrap_or(json!({}));
    crate::studio::templates::upsert_custom_template(
        &state.studio.db,
        id,
        name,
        body.get("description")
            .and_then(Value::as_str)
            .unwrap_or(""),
        &graph.to_string(),
        &params.to_string(),
        &prices.to_string(),
    )
    .await
    .map_err(|e| {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "studio_error",
            e.to_string(),
        )
    })?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "created": true, "id": id })),
    ))
}

pub async fn admin_update_template(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    require_admin(&headers, &state).await?;
    let enabled = body.get("enabled").and_then(Value::as_bool);
    let prices = body
        .get("price_nano_usd_map")
        .map(|prices| prices.to_string());
    let updated = crate::studio::templates::set_template_overrides(
        &state.studio.db,
        &id,
        enabled,
        prices.as_deref(),
    )
    .await
    .map_err(|e| {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "studio_error",
            e.to_string(),
        )
    })?;
    if !updated {
        return Err(AppError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "template not found",
        ));
    }
    Ok(Json(json!({ "updated": true })))
}

pub async fn admin_delete_template(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    require_admin(&headers, &state).await?;
    let deleted = crate::studio::templates::delete_custom_template(&state.studio.db, &id)
        .await
        .map_err(|e| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "studio_error",
                e.to_string(),
            )
        })?;
    if !deleted {
        return Err(AppError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "custom template not found",
        ));
    }
    Ok(Json(json!({ "deleted": true })))
}

pub async fn admin_list_runs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<RunsQuery>,
) -> AppResult<Json<Value>> {
    require_admin(&headers, &state).await?;
    let runs = store::list_runs(&state.studio.db, None, query.limit.unwrap_or(100).min(500))
        .await
        .map_err(|e| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "studio_error",
                e.to_string(),
            )
        })?;
    Ok(Json(json!({ "runs": runs })))
}

pub async fn admin_cancel_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let admin = require_admin(&headers, &state).await?;
    let _ = admin;
    let run = store::get_run(&state.studio.db, &id)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "studio_error", e))?
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "not_found", "run not found"))?;
    crate::studio::engine::cancel_run(&state.studio, &run)
        .await
        .map_err(|e| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "studio_error",
                e.to_string(),
            )
        })?;
    Ok(Json(json!({ "canceled": true })))
}

pub async fn admin_get_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    require_admin(&headers, &state).await?;
    let settings = StudioSettings::load(&state.settings_store).await;
    Ok(Json(json!({ "settings": settings })))
}

pub async fn admin_update_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    require_admin(&headers, &state).await?;
    let mut settings = StudioSettings::load(&state.settings_store).await;
    if let Some(model) = body.get("agent_model").and_then(Value::as_str) {
        settings.agent_model = model.trim().to_string();
    }
    if let Some(upstream) = body.get("default_video_upstream").and_then(Value::as_str) {
        settings.default_video_upstream = upstream.trim().to_string();
    }
    if let Some(model) = body.get("image_model").and_then(Value::as_str) {
        settings.image_model = model.trim().to_string();
    }
    StudioSettings::save(&state.settings_store, &settings)
        .await
        .map_err(|e| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "studio_error",
                e.to_string(),
            )
        })?;
    Ok(Json(json!({ "settings": settings })))
}

// ---------------------------------------------------------------------------
// Public API /v1/videos (ST-V1..V4)
// ---------------------------------------------------------------------------

fn studio_api_error(status: StatusCode, code: &str, message: &str) -> Response {
    let body = json!({"error": {"message": message, "type": "studio_error", "code": code}});
    let mut response = (status, Json(body)).into_response();
    *response.status_mut() = status;
    response
}

pub async fn create_video_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let auth = match crate::handlers::auth_tenant(&headers, &state).await {
        Ok(auth) => auth,
        Err(error) => return error.into_response(),
    };
    let user_id = auth.tenant_id.clone();
    let Some(model) = body
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|m| !m.is_empty())
    else {
        return studio_api_error(
            StatusCode::BAD_REQUEST,
            "invalid_params",
            "model is required",
        );
    };
    let prompt = body
        .get("prompt")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|p| !p.is_empty());
    let Some(prompt) = prompt else {
        return studio_api_error(
            StatusCode::BAD_REQUEST,
            "invalid_params",
            "prompt is required",
        );
    };
    if let Err(error) = firewall_check(&state, prompt).await {
        return studio_api_error(StatusCode::BAD_REQUEST, "firewall_blocked", &error.message);
    }
    let settings = StudioSettings::load(&state.settings_store).await;
    // The model may be a template id or a direct upstream model name.
    let (upstream_kind, resolved_model, price) =
        match crate::studio::templates::get_template(&state.studio.db, model).await {
            Ok(Some(template)) if template.enabled => {
                let params: Value =
                    serde_json::from_str(&template.params_json).unwrap_or(json!({}));
                (
                    params
                        .get("upstream_kind")
                        .and_then(Value::as_str)
                        .unwrap_or(&settings.default_video_upstream)
                        .to_string(),
                    params
                        .get("model")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    template
                        .price_nano_usd_map
                        .parse::<i64>()
                        .ok()
                        .filter(|p| *p > 0)
                        .unwrap_or_else(|| settings.video_price_nano()),
                )
            }
            _ => (
                settings.default_video_upstream.clone(),
                model.to_string(),
                settings.video_price_nano(),
            ),
        };
    let _ = resolved_model;
    // Admission (ST-S6).
    let global = match store::active_run_counts(&state.studio.db, None).await {
        Ok(count) => count,
        Err(e) => return studio_api_error(StatusCode::INTERNAL_SERVER_ERROR, "studio_error", &e),
    };
    if global >= crate::studio::global_active_runs() {
        return studio_api_error(
            StatusCode::TOO_MANY_REQUESTS,
            "studio_busy",
            "studio is busy",
        );
    }
    if let Ok(count) = store::active_run_counts(&state.studio.db, Some(&user_id)).await
        && count >= crate::studio::user_active_runs()
    {
        return studio_api_error(
            StatusCode::TOO_MANY_REQUESTS,
            "studio_busy",
            "too many active runs for this account",
        );
    }
    let run_id = Uuid::new_v4().to_string();
    let api_key_id = auth.api_key_id.clone();
    if let Err(e) = store::create_run(
        &state.studio.db,
        &run_id,
        &user_id,
        None,
        api_key_id.as_deref(),
        "public_api",
    )
    .await
    {
        return studio_api_error(StatusCode::INTERNAL_SERVER_ERROR, "studio_error", &e);
    }
    let payload = json!({
        "model": model,
        "prompt": prompt,
        "seconds": body.get("seconds").and_then(Value::as_str),
        "size": body.get("size").and_then(Value::as_str),
        "image": body.get("image").and_then(Value::as_str),
        "params": body.get("params"),
        "upstream_kind": upstream_kind,
        "price_nano": price,
    });
    if let Err(e) = store::create_step(
        &state.studio.db,
        &Uuid::new_v4().to_string(),
        &run_id,
        "video",
        None,
        &payload.to_string(),
    )
    .await
    {
        return studio_api_error(StatusCode::INTERNAL_SERVER_ERROR, "studio_error", &e);
    }
    let run = store::get_run(&state.studio.db, &run_id)
        .await
        .ok()
        .flatten();
    (
        StatusCode::CREATED,
        Json(json!({
            "id": run_id,
            "object": "video.job",
            "model": model,
            "status": "queued",
            "progress": 0,
            "created_at": run.as_ref().map(|r| r.created_at.clone()).unwrap_or_default(),
            "price_nano_usd": price,
        })),
    )
        .into_response()
}

pub async fn get_video_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let auth = match crate::handlers::auth_tenant(&headers, &state).await {
        Ok(auth) => auth,
        Err(error) => return error.into_response(),
    };
    let Some(run) = (match store::get_run(&state.studio.db, &id).await {
        Ok(run) => run,
        Err(e) => return studio_api_error(StatusCode::INTERNAL_SERVER_ERROR, "studio_error", &e),
    }) else {
        return studio_api_error(StatusCode::NOT_FOUND, "model_not_found", "job not found");
    };
    if run.user_id != auth.tenant_id {
        return studio_api_error(StatusCode::NOT_FOUND, "model_not_found", "job not found");
    }
    let steps = match store::list_steps_for_run(&state.studio.db, &id).await {
        Ok(steps) => steps,
        Err(e) => return studio_api_error(StatusCode::INTERNAL_SERVER_ERROR, "studio_error", &e),
    };
    // ST-V2 status mapping.
    let any_running = steps
        .iter()
        .any(|s| matches!(s.status.as_str(), "pending" | "running"));
    let (status, progress) =
        if run.status == "queued" && steps.iter().all(|s| s.status == "pending") {
            ("queued".to_string(), 0u8)
        } else if any_running || run.status == "running" {
            let progress = steps
                .iter()
                .find_map(|s| {
                    serde_json::from_str::<Value>(&s.payload_json)
                        .ok()
                        .and_then(|p| p.get("progress").and_then(Value::as_u64))
                        .map(|v| v.clamp(0, 100) as u8)
                })
                .unwrap_or(5);
            ("in_progress".to_string(), progress)
        } else {
            match run.status.as_str() {
                "succeeded" => ("succeeded".to_string(), 100),
                "canceled" => ("canceled".to_string(), 0),
                _ => ("failed".to_string(), 0),
            }
        };
    let price: i64 = steps
        .iter()
        .map(|s| s.charge_nano_usd - s.refund_nano_usd)
        .sum();
    (
        StatusCode::OK,
        Json(json!({
            "id": run.id,
            "object": "video.job",
            "status": status,
            "progress": progress,
            "error": run.error,
            "created_at": run.created_at,
            "completed_at": run.finished_at,
            "price_nano_usd": price,
        })),
    )
        .into_response()
}

pub async fn cancel_video_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let auth = match crate::handlers::auth_tenant(&headers, &state).await {
        Ok(auth) => auth,
        Err(error) => return error.into_response(),
    };
    let Some(run) = (match store::get_run(&state.studio.db, &id).await {
        Ok(run) => run,
        Err(e) => return studio_api_error(StatusCode::INTERNAL_SERVER_ERROR, "studio_error", &e),
    }) else {
        return studio_api_error(StatusCode::NOT_FOUND, "model_not_found", "job not found");
    };
    if run.user_id != auth.tenant_id {
        return studio_api_error(StatusCode::NOT_FOUND, "model_not_found", "job not found");
    }
    if let Err(e) = crate::studio::engine::cancel_run(&state.studio, &run).await {
        return studio_api_error(StatusCode::INTERNAL_SERVER_ERROR, "studio_error", &e);
    }
    (
        StatusCode::OK,
        Json(json!({"id": run.id, "object": "video.job", "status": "canceled"})),
    )
        .into_response()
}

pub async fn get_video_content(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let auth = match crate::handlers::auth_tenant(&headers, &state).await {
        Ok(auth) => auth,
        Err(error) => return error.into_response(),
    };
    let Some(run) = (match store::get_run(&state.studio.db, &id).await {
        Ok(run) => run,
        Err(e) => return studio_api_error(StatusCode::INTERNAL_SERVER_ERROR, "studio_error", &e),
    }) else {
        return studio_api_error(StatusCode::NOT_FOUND, "model_not_found", "job not found");
    };
    if run.user_id != auth.tenant_id {
        return studio_api_error(StatusCode::NOT_FOUND, "model_not_found", "job not found");
    }
    if run.status != "succeeded" && run.status != "partial" {
        return studio_api_error(
            StatusCode::CONFLICT,
            "not_terminal",
            "job is not finished yet",
        );
    }
    let Ok(steps) = store::list_steps_for_run(&state.studio.db, &id).await else {
        return studio_api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "studio_error",
            "steps unavailable",
        );
    };
    let Some(asset_id) = steps.iter().find_map(|step| {
        step.result_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            .and_then(|result| {
                result
                    .get("asset_id")
                    .and_then(Value::as_str)
                    .map(String::from)
            })
    }) else {
        return studio_api_error(StatusCode::NOT_FOUND, "no_output", "job has no output");
    };
    match stream_asset(&state.studio, &auth.tenant_id, &asset_id).await {
        Ok(response) => response,
        Err(error) => studio_api_error(
            StatusCode::BAD_GATEWAY,
            "asset_fetch_failed",
            &error.message,
        ),
    }
}
