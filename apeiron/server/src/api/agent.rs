//! AP-AG2: conversational graph agent — a real JSON-action tool loop with
//! conversation history, provider failover, and token billing.

use crate::error::{ApiError, ApiResult};
use crate::state::SharedState;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;

#[derive(Deserialize)]
pub struct AgentBody {
    pub message: String,
    /// Prior conversation turns (role user/assistant), client-supplied.
    #[serde(default)]
    pub history: Vec<HistoryTurn>,
}

#[derive(Deserialize)]
pub struct HistoryTurn {
    pub role: String,
    pub content: String,
}

const TOOL_PROTOCOL: &str = "You operate a video-production node graph. Reply with ONLY one JSON \
object, no markdown fences: {\"action\":\"<tool>|\"reply\", \"args\":{...}, \"message\":\"short \
user-facing text in the user's language\"}. Tools: update_script(prompt, system?), \
create_shots(count?), update_shot(index, description?, keywords?), set_param(node_id, key, \
value), run_graph(). Nodes: script, storyboard, image, video, tts, subtitle, material, assemble. \
Plan multiple steps by chaining actions across turns; the caller feeds your previous action back. \
When the request is complete or needs user input, use action \"reply\".";

fn set_param_value(node: &mut crate::graph::Node, key: &str, value: Value) {
    if let Some(map) = node.params.as_object_mut() {
        map.insert(key.to_string(), value);
    }
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

/// CAS save; returns false when the version moved under us.
async fn save_graph(
    state: &SharedState,
    project_id: &str,
    version: i64,
    graph: &crate::graph::Graph,
) -> Result<bool, String> {
    let raw = serde_json::to_string(graph).map_err(|e| e.to_string())?;
    let now = crate::now_rfc3339();
    let result = sqlx::query(
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
    Ok(result.rows_affected() == 1)
}

struct AgentLlmUsage {
    input_tokens: u64,
    output_tokens: u64,
}

/// One LLM call through the ordered provider list with failover (AP-E7).
async fn call_llm(
    state: &SharedState,
    prompt: &str,
) -> Result<(String, AgentLlmUsage), String> {
    let providers = crate::upstream::list_providers(state, "llm").await?;
    if providers.is_empty() {
        return Err("no_provider: no enabled llm provider".into());
    }
    let timeout = crate::http_timeout("script", &state.cfg);
    let mut last_error = String::new();
    for provider in providers.iter().take(2) {
        match crate::upstream::llm_complete(state, provider, TOOL_PROTOCOL, prompt, timeout).await
        {
            Ok(output) => {
                return Ok((
                    output.text,
                    AgentLlmUsage {
                        input_tokens: output.input_tokens,
                        output_tokens: output.output_tokens,
                    },
                ))
            }
            Err(error) => {
                tracing::warn!(%error, provider = %provider.id, "agent llm attempt failed");
                last_error = error;
            }
        }
    }
    Err(last_error)
}

/// Token-metered bridge settlement for agent turns (AP-AG2 billing).
async fn bill_agent_turn(
    state: &SharedState,
    user_id: &str,
    key_suffix: &str,
    input_tokens: u64,
    output_tokens: u64,
) {
    let providers = crate::upstream::list_providers(state, "llm")
        .await
        .unwrap_or_default();
    let Some(provider) = providers.into_iter().next() else {
        return;
    };
    let in_rate = provider
        .params
        .get("in_price_nano_per_mtok")
        .and_then(Value::as_i64)
        .unwrap_or(0) as i128;
    let out_rate = provider
        .params
        .get("out_price_nano_per_mtok")
        .and_then(Value::as_i64)
        .unwrap_or(0) as i128;
    let raw = input_tokens as i128 * in_rate + output_tokens as i128 * out_rate;
    let cost = ((raw + 999_999) / 1_000_000).max(0);
    if cost <= 0 {
        return;
    }
    let bridge = crate::bridge::BridgeClient::new(
        &state.http,
        &state.cfg.bridge_url,
        state.cfg.bridge_service_token.as_deref(),
    );
    let key = format!("apeiron_step_agent_{key_suffix}");
    if bridge
        .settle("debit", user_id, cost, &key, json!({ "agent": true }))
        .await
        .is_ok()
    {
        crate::engine::refresh_mirror_balance(state, user_id).await;
    }
}

fn parse_action(text: &str) -> Option<Value> {
    let cleaned = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let start = cleaned.find('{')?;
    let end = cleaned.rfind('}')?;
    if end <= start {
        return None;
    }
    let candidate = &cleaned[start..=end];
    let value: Value = serde_json::from_str(candidate).ok()?;
    // Only treat objects carrying an "action" field as tool calls; any other
    // JSON the model emitted is conversational.
    value.get("action").and_then(Value::as_str)?;
    Some(value)
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
    let (mut version, mut graph) = load_project(&state, &project_id, &user.id).await?;
    let mut applied: Vec<Value> = Vec::new();
    let turn_key = uuid::Uuid::new_v4().simple().to_string();
    let mut total_input: u64 = 0;
    let mut total_output: u64 = 0;

    // Conversation transcript: system protocol + graph summary + history.
    let node_summary: Vec<String> = graph
        .nodes
        .iter()
        .map(|node| format!("{}:{}", node.id, node.kind))
        .collect();
    let mut transcript = String::new();
    for turn in body.history.iter().take(20) {
        transcript.push_str(&format!("{}: {}\n", turn.role, turn.content));
    }
    let mut next_input = format!(
        "{transcript}\nGraph nodes: [{}]; edges {}.\nUser: {}",
        node_summary.join(", "),
        graph.edges.len(),
        body.message
    );

    let mut assistant_text = String::new();
    let mut started_run: Option<Value> = None;
    for _round in 0..12 {
        let (text, usage) = call_llm(&state, &next_input)
            .await
            .map_err(ApiError::internal)?;
        total_input += usage.input_tokens;
        total_output += usage.output_tokens;
        let Some(action) = parse_action(&text) else {
            assistant_text = text.trim().to_string();
            break;
        };
        if let Some(message) = action.get("message").and_then(Value::as_str) {
            assistant_text = message.to_string();
        }
        let tool = action.get("action").and_then(Value::as_str).unwrap_or("reply");
        let args = action.get("args").cloned().unwrap_or(json!({}));

        // Reload the graph before each mutation so concurrent edits (the
        // user's autosave) are not clobbered.
        if let Ok((fresh_version, fresh_graph)) =
            load_project(&state, &project_id, &user.id).await
        {
            version = fresh_version;
            graph = fresh_graph;
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
                    if save_graph(&state, &project_id, version, &graph).await.map_err(ApiError::internal)? {
                        applied.push(json!({ "tool": "update_script" }));
                        state.publish(&user.id, "graph_patch", json!({ "project_id": project_id, "version": version }));
                    } else {
                        break;
                    }
                }
            }
            "create_shots" => {
                let count = args.get("count").and_then(Value::as_u64).unwrap_or(6);
                if let Some(node) = graph.nodes.iter_mut().find(|node| node.kind == "storyboard") {
                    set_param_value(node, "count", json!(count));
                    version += 1;
                    if save_graph(&state, &project_id, version, &graph).await.map_err(ApiError::internal)? {
                        applied.push(json!({ "tool": "create_shots", "count": count }));
                        state.publish(&user.id, "graph_patch", json!({ "project_id": project_id, "version": version }));
                    } else {
                        break;
                    }
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
                    if save_graph(&state, &project_id, version, &graph).await.map_err(ApiError::internal)? {
                        applied.push(json!({ "tool": "update_shot", "index": index }));
                        state.publish(&user.id, "graph_patch", json!({ "project_id": project_id, "version": version }));
                    } else {
                        break;
                    }
                }
            }
            "set_param" => {
                let node_id = args.get("node_id").and_then(Value::as_str).unwrap_or("");
                let key = args.get("key").and_then(Value::as_str).unwrap_or("");
                let value = args.get("value").cloned().unwrap_or(Value::Null);
                if node_id.is_empty() || key.is_empty() {
                    break;
                }
                if let Some(node) = graph.nodes.iter_mut().find(|node| node.id == node_id) {
                    set_param_value(node, key, value);
                    version += 1;
                    if save_graph(&state, &project_id, version, &graph).await.map_err(ApiError::internal)? {
                        applied.push(json!({ "tool": "set_param", "node_id": node_id, "key": key }));
                        state.publish(&user.id, "graph_patch", json!({ "project_id": project_id, "version": version }));
                    } else {
                        break;
                    }
                }
            }
            "run_graph" => {
                let run = super::projects::create_run(
                    &state,
                    &user.id,
                    Some(&project_id),
                    "graph",
                    json!({ "root": "auto" }),
                )
                .await?;
                applied.push(json!({ "tool": "run_graph", "run_id": run.get("id") }));
                started_run = run.get("id").cloned();
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
        next_input = format!(
            "Action applied: {tool}. Continue with the next tool call, or use action \"reply\" \
             with a short message when done."
        );
    }

    bill_agent_turn(&state, &user.id, &turn_key, total_input, total_output).await;

    Ok((
        StatusCode::OK,
        Json(json!({
            "message": assistant_text,
            "applied": applied,
            "version": version,
            "run_id": started_run,
        })),
    ))
}
