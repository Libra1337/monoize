//! AP-AG2: conversational graph agent — an LLM tool loop that edits the
//! project graph server-side. Every mutation bumps the version and
//! broadcasts `graph_patch` so the canvas reloads live.

use crate::error::{ApiError, ApiResult};
use crate::state::SharedState;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;


/// Insert into a node's params object (Value is an object per AP-G3).
fn set_param_value(node: &mut crate::graph::Node, key: &str, value: Value) {
    if let Some(map) = node.params.as_object_mut() {
        map.insert(key.to_string(), value);
    }
}

#[derive(Deserialize)]
pub struct AgentBody {
    pub message: String,
}

const SYSTEM_PROMPT: &str = "You are Apeiron's video workflow agent. You edit a node graph \
that produces videos. Available nodes: script (LLM text), storyboard (splits text into shots), \
image (text-to-image), video (text-to-video), tts (voice), subtitle (SRT), material (stock clip), \
assemble (final film). Use the tools to build or adjust the graph for the user's request. \
Call run_graph only when the user asks to run/generate. Reply concisely in the user's language.";

#[allow(dead_code)]
fn tools_schema() -> Value {
    json!([
        {
            "type": "function",
            "function": {
                "name": "update_script",
                "description": "Set the script node's prompt and optional system instruction",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "prompt": { "type": "string" },
                        "system": { "type": "string" }
                    },
                    "required": ["prompt"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "create_shots",
                "description": "Ensure a storyboard node exists that splits the script into N shots",
                "parameters": {
                    "type": "object",
                    "properties": { "count": { "type": "integer", "minimum": 1, "maximum": 24 } },
                    "required": []
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "update_shot",
                "description": "Edit one storyboard shot (applies as a review override)",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "index": { "type": "integer", "minimum": 0 },
                        "description": { "type": "string" },
                        "keywords": { "type": "string" }
                    },
                    "required": ["index"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "set_param",
                "description": "Set one parameter on any node (e.g. size, seconds, voice)",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "node_id": { "type": "string" },
                        "key": { "type": "string" },
                        "value": {}
                    },
                    "required": ["node_id", "key", "value"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "run_graph",
                "description": "Start a run of the whole graph",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        }
    ])
}

async fn load_project(
    state: &SharedState,
    project_id: &str,
    user_id: &str,
) -> ApiResult<(i64, crate::graph::Graph)> {
    let row = sqlx::query(
        "SELECT version, graph_json FROM projects WHERE id = $1 AND user_id = $2",
    )
    .bind(project_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::not_found("project"))?;
    let version: i64 = row.try_get("version").unwrap_or(1);
    let raw: String = row.try_get("graph_json").unwrap_or_default();
    let graph = crate::graph::Graph::parse(&raw).map_err(ApiError::internal)?;
    Ok((version, graph))
}

async fn save_graph(
    state: &SharedState,
    project_id: &str,
    version: i64,
    graph: &crate::graph::Graph,
) -> Result<(), String> {
    let raw = serde_json::to_string(graph).map_err(|e| e.to_string())?;
    let now = crate::now_rfc3339();
    sqlx::query(
        "UPDATE projects SET graph_json = $1, version = $2, updated_at = $3 \
         WHERE id = $4 AND version = $2 - 1",
    )
    .bind(&raw)
    .bind(version)
    .bind(&now)
    .bind(project_id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn submit(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(body): Json<AgentBody>,
) -> ApiResult<impl IntoResponse> {
    let user = crate::auth::current_user(&state, &headers).await?;
    if body.message.trim().is_empty() {
        return Err(ApiError::bad_request("invalid_params", "message is required"));
    }
    let providers = crate::upstream::list_providers(&state, "llm")
        .await
        .map_err(ApiError::internal)?;
    let provider = providers
        .into_iter()
        .next()
        .ok_or_else(|| ApiError::bad_request("no_provider", "no enabled llm provider"))?;

    let (mut version, mut graph) = load_project(&state, &project_id, &user.id).await?;
    let mut applied: Vec<Value> = Vec::new();
    let mut messages = vec![
        json!({ "role": "system", "content": SYSTEM_PROMPT }),
    ];
    // Compact graph summary keeps the loop grounded.
    let summary: Vec<String> = graph
        .nodes
        .iter()
        .map(|node| format!("{}:{}", node.id, node.kind))
        .collect();
    messages.push(json!({
        "role": "system",
        "content": format!("Current graph nodes: [{}]. Edges: {}.", summary.join(", "), graph.edges.len())
    }));
    messages.push(json!({ "role": "user", "content": body.message }));

    let timeout = crate::http_timeout("script", &state.cfg);
    let mut assistant_text = String::new();
    for _round in 0..12 {
        let output = crate::upstream::llm_complete(
            &state,
            &provider,
            "",
            &messages
                .iter()
                .map(|message| message.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
            timeout,
        )
        .await;
        // The generic adapter takes a single prompt; tool calling needs the
        // raw chat format, so fall back to a JSON protocol prompt.
        let text = match output {
            Ok(output) => output.text,
            Err(error) => {
                // Try the JSON-action protocol instead.
                let protocol = format!(
                    "{}\n\nReply with ONLY one JSON object: {{\"action\": \"<tool>|reply\", \"args\": {{...}}, \"message\": \"short user-facing text\"}}. Tools: update_script(prompt, system?), create_shots(count?), update_shot(index, description?, keywords?), set_param(node_id, key, value), run_graph().",
                    body.message
                );
                match crate::upstream::llm_complete(&state, &provider, SYSTEM_PROMPT, &protocol, timeout).await {
                    Ok(fallback) => fallback.text,
                    Err(_) => return Err(ApiError::internal(error)),
                }
            }
        };
        let action = parse_action(&text);
        let Some(action) = action else {
            assistant_text = text.trim().to_string();
            break;
        };
        let tool = action.get("action").and_then(Value::as_str).unwrap_or("reply");
        let args = action.get("args").cloned().unwrap_or(json!({}));
        if let Some(text) = action.get("message").and_then(Value::as_str) {
            assistant_text = text.to_string();
        }
        match tool {
            "update_script" => {
                let prompt = args.get("prompt").and_then(Value::as_str).unwrap_or("");
                let system = args.get("system").and_then(Value::as_str);
                if let Some(node) = graph.nodes.iter_mut().find(|node| node.kind == "script") {
                    set_param_value(node, "prompt", json!(prompt));
                    if let Some(system) = system {
                        set_param_value(node, "system", json!(system));
                    }
                    version += 1;
                    save_graph(&state, &project_id, version, &graph).await.map_err(ApiError::internal)?;
                    applied.push(json!({ "tool": "update_script" }));
                    state.publish(&user.id, "graph_patch", json!({ "project_id": project_id, "version": version }));
                }
            }
            "create_shots" => {
                let count = args.get("count").and_then(Value::as_u64).unwrap_or(6);
                if let Some(node) = graph.nodes.iter_mut().find(|node| node.kind == "storyboard") {
                    set_param_value(node, "count", json!(count));
                    version += 1;
                    save_graph(&state, &project_id, version, &graph).await.map_err(ApiError::internal)?;
                    applied.push(json!({ "tool": "create_shots" }));
                    state.publish(&user.id, "graph_patch", json!({ "project_id": project_id, "version": version }));
                }
            }
            "update_shot" => {
                let index = args.get("index").and_then(Value::as_i64).unwrap_or(0);
                if let Some(node) = graph.nodes.iter_mut().find(|node| node.kind == "storyboard") {
                    if !node.params.get("shot_overrides").is_some_and(Value::is_object) {
                        set_param_value(node, "shot_overrides", json!({}));
                    }
                    let overrides = node.params.get_mut("shot_overrides").expect("just set");
                    if let Some(map) = overrides.as_object_mut() {
                        let mut edit = map.get(&index.to_string()).cloned().unwrap_or(json!({}));
                        if let Some(description) = args.get("description").and_then(Value::as_str) {
                            edit["description"] = json!(description);
                        }
                        if let Some(keywords) = args.get("keywords").and_then(Value::as_str) {
                            edit["keywords"] = json!(keywords);
                        }
                        map.insert(index.to_string(), edit);
                    }
                    version += 1;
                    save_graph(&state, &project_id, version, &graph).await.map_err(ApiError::internal)?;
                    applied.push(json!({ "tool": "update_shot", "index": index }));
                    state.publish(&user.id, "graph_patch", json!({ "project_id": project_id, "version": version }));
                }
            }
            "set_param" => {
                let node_id = args.get("node_id").and_then(Value::as_str).unwrap_or("");
                let key = args.get("key").and_then(Value::as_str).unwrap_or("");
                let value = args.get("value").cloned().unwrap_or(Value::Null);
                if !node_id.is_empty() && !key.is_empty() {
                    if let Some(node) = graph.nodes.iter_mut().find(|node| node.id == node_id) {
                        set_param_value(node, key, value);
                        version += 1;
                        save_graph(&state, &project_id, version, &graph).await.map_err(ApiError::internal)?;
                        applied.push(json!({ "tool": "set_param", "node_id": node_id, "key": key }));
                        state.publish(&user.id, "graph_patch", json!({ "project_id": project_id, "version": version }));
                    }
                }
            }
            "run_graph" => {
                let run = super::projects::create_run(
                    &state,
                    &user.id,
                    Some(&project_id),
                    "graph",
                    json!({ "root": "final" }),
                )
                .await?;
                applied.push(json!({ "tool": "run_graph", "run_id": run.get("id") }));
                break;
            }
            _ => {
                assistant_text = action
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                break;
            }
        }
        messages.push(json!({ "role": "assistant", "content": text }));
        messages.push(json!({
            "role": "user",
            "content": "Continue with the next tool call or reply with action \"reply\" when done."
        }));
    }

    Ok((
        StatusCode::OK,
        Json(json!({ "message": assistant_text, "applied": applied, "version": version })),
    ))
}

fn parse_action(text: &str) -> Option<Value> {
    let cleaned = text.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
    let start = cleaned.find('{')?;
    let end = cleaned.rfind('}')?;
    serde_json::from_str(&cleaned[start..=end]).ok()
}
