//! ST-A2/A3/A4/A5 + ST-P1..P5: the agent loop, its tool set, and the script
//! pipeline stages. All graph mutations happen server-side and broadcast
//! `graph_patch` events; every billable call lands as a studio step.

use crate::studio::StudioState;
use crate::studio::llm;
use crate::studio::store::{self, RunRow};
use serde_json::{Value, json};
use uuid::Uuid;

const MAX_TOOL_ITERATIONS: usize = 12;

fn now_string() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub fn empty_graph() -> Value {
    json!({"nodes": [], "edges": [], "version": 0})
}

pub fn graph_summary(graph: &Value) -> String {
    let nodes = graph.get("nodes").and_then(Value::as_array);
    let shots = nodes
        .map(|nodes| {
            nodes
                .iter()
                .filter(|n| n.get("kind").and_then(Value::as_str) == Some("shot"))
                .count()
        })
        .unwrap_or(0);
    format!(
        "canvas currently has {} nodes and {} shot nodes",
        nodes.map(Vec::len).unwrap_or(0),
        shots
    )
}

fn script_node_id(graph: &Value) -> Option<String> {
    graph.get("nodes")?.as_array()?.iter().find_map(|node| {
        (node.get("kind").and_then(Value::as_str) == Some("script"))
            .then(|| node.get("id").and_then(Value::as_str).map(String::from))
            .flatten()
    })
}

fn shot_nodes(graph: &Value) -> Vec<Value> {
    graph
        .get("nodes")
        .and_then(Value::as_array)
        .map(|nodes| {
            nodes
                .iter()
                .filter(|n| n.get("kind").and_then(Value::as_str) == Some("shot"))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn ensure_script_node(graph: &mut Value) -> String {
    if let Some(id) = script_node_id(graph) {
        return id;
    }
    let id = Uuid::new_v4().to_string();
    let (x, y) = store::graph_next_node_position(graph, 0);
    if let Some(nodes) = graph.get_mut("nodes").and_then(Value::as_array_mut) {
        nodes.push(json!({
            "id": id, "kind": "script", "position": {"x": x, "y": y},
            "data": {"text": "", "stage": "idle"}
        }));
    }
    id
}

/// Replace all shot nodes (and their contains edges) from a structured list.
fn write_shots(graph: &mut Value, script_id: &str, shots: &[Value]) {
    let shot_ids: Vec<String> = graph
        .get("nodes")
        .and_then(Value::as_array)
        .map(|nodes| {
            nodes
                .iter()
                .filter(|n| n.get("kind").and_then(Value::as_str) == Some("shot"))
                .filter_map(|n| n.get("id").and_then(Value::as_str).map(String::from))
                .collect()
        })
        .unwrap_or_default();
    if let Some(nodes) = graph.get_mut("nodes").and_then(Value::as_array_mut) {
        nodes.retain(|n| n.get("kind").and_then(Value::as_str) != Some("shot"));
    }
    if let Some(edges) = graph.get_mut("edges").and_then(Value::as_array_mut) {
        edges.retain(|e| {
            e.get("kind").and_then(Value::as_str) != Some("contains")
                && !shot_ids
                    .iter()
                    .any(|id| e.get("target").and_then(Value::as_str) == Some(id.as_str()))
        });
    }
    for (index, shot) in shots.iter().enumerate() {
        let id = Uuid::new_v4().to_string();
        let (x, y) = store::graph_next_node_position(graph, index + 1);
        if let Some(nodes) = graph.get_mut("nodes").and_then(Value::as_array_mut) {
            nodes.push(json!({
                "id": id, "kind": "shot", "position": {"x": x, "y": y},
                "data": {
                    "index": index + 1,
                    "description": shot.get("description").and_then(Value::as_str).unwrap_or(""),
                    "camera": shot.get("camera").and_then(Value::as_str).unwrap_or(""),
                    "duration_sec": shot.get("duration_sec").and_then(Value::as_i64).unwrap_or(4),
                    "dialogue": shot.get("dialogue").and_then(Value::as_str).unwrap_or(""),
                    "prompt": shot.get("prompt").and_then(Value::as_str).unwrap_or(""),
                    "image_node": Value::Null,
                    "video_node": Value::Null
                }
            }));
        }
        if let Some(edges) = graph.get_mut("edges").and_then(Value::as_array_mut) {
            edges.push(json!({
                "id": Uuid::new_v4().to_string(), "source": script_id, "target": id, "kind": "contains"
            }));
        }
    }
}

fn set_node_status(graph: &mut Value, node_id: &str, status: &str) {
    store::graph_mutate_node(graph, node_id, &json!({"status": status}));
}

// ---------------------------------------------------------------------------
// LLM step recording (each call is a billable step, MB-ST2)
// ---------------------------------------------------------------------------

async fn record_llm_step(
    state: &StudioState,
    run: &RunRow,
    label: &str,
    messages: &[Value],
    tools: Option<&Value>,
) -> Result<llm::LlmResult, String> {
    let settings = crate::studio::StudioSettings::load(&state.settings_store).await;
    let model = settings.agent_model.clone();
    let step_id = Uuid::new_v4().to_string();
    store::create_step(
        &state.db,
        &step_id,
        &run.id,
        "llm",
        None,
        &serde_json::to_string(&json!({"label": label, "model": model, "messages": messages}))
            .map_err(|e| e.to_string())?,
    )
    .await?;
    let _ = store::transition_step(&state.db, &step_id, &["pending"], "running", None, None).await;
    state.publish(
        &run.user_id,
        "step_update",
        json!({"step_id": step_id, "status": "running", "kind": "llm"}),
    );
    match llm::complete(state, &model, messages, tools).await {
        Ok((result, choice)) => {
            let charge = llm::token_charge_nano(
                state,
                &model,
                None,
                result.input_tokens,
                result.output_tokens,
            )
            .await;
            if charge > 0 {
                let meta = json!({"request_id": format!("studio_step_{step_id}"), "model": model, "label": label});
                if let Err(error) = state
                    .user_store
                    .studio_charge_balance(&run.user_id, charge, "studio_llm_charge", &meta)
                    .await
                {
                    store::transition_step(
                        &state.db,
                        &step_id,
                        &["running"],
                        "failed",
                        Some(&error),
                        None,
                    )
                    .await?;
                    return Err(error);
                }
                let _ = store::add_step_charge(&state.db, &step_id, charge as i64).await;
            }
            let _ = store::transition_step(
                &state.db,
                &step_id,
                &["running"],
                "succeeded",
                None,
                Some(
                    &serde_json::to_string(&json!({
                        "content": result.content,
                        "tool_calls": result.tool_calls,
                        "input_tokens": result.input_tokens,
                        "output_tokens": result.output_tokens,
                        "pricing_profile": Value::Null,
                    }))
                    .map_err(|e| e.to_string())?,
                ),
            )
            .await;
            let _ = choice;
            Ok(result)
        }
        Err(error) => {
            let message = error.message();
            let _ = store::transition_step(
                &state.db,
                &step_id,
                &["running"],
                "failed",
                Some(&message),
                None,
            )
            .await;
            Err(message)
        }
    }
}

// ---------------------------------------------------------------------------
// Generation step creation (image / video)
// ---------------------------------------------------------------------------

/// Prices come from the step payload's `price_nano` (template or node price)
/// times the selected channel multiplier at dispatch time (MB-ST1).
pub struct GenRequest {
    pub prompt: String,
    pub seconds: Option<String>,
    pub size: Option<String>,
    pub image: Option<String>,
    pub node_id: Option<String>,
    pub price_nano: i64,
}

pub async fn create_image_step(state: &StudioState, run: &RunRow, request: &GenRequest) -> String {
    let step_id = Uuid::new_v4().to_string();
    let settings = crate::studio::StudioSettings::load(&state.settings_store).await;
    let payload = json!({
        "model": settings.image_model,
        "prompt": request.prompt,
        "size": request.size,
        "barrier": "frames",
        "price_nano": request.price_nano,
    });
    let _ = store::create_step(
        &state.db,
        &step_id,
        &run.id,
        "image",
        request.node_id.as_deref(),
        &payload.to_string(),
    )
    .await;
    step_id
}

pub async fn create_video_step(state: &StudioState, run: &RunRow, request: &GenRequest) -> String {
    let step_id = Uuid::new_v4().to_string();
    let settings = crate::studio::StudioSettings::load(&state.settings_store).await;
    let payload = json!({
        "upstream_kind": settings.default_video_upstream,
        "model": settings.video_model_hint(),
        "prompt": request.prompt,
        "seconds": request.seconds,
        "size": request.size,
        "image": request.image,
        "barrier": "videos",
        "price_nano": request.price_nano,
    });
    let _ = store::create_step(
        &state.db,
        &step_id,
        &run.id,
        "video",
        request.node_id.as_deref(),
        &payload.to_string(),
    )
    .await;
    step_id
}

impl crate::studio::StudioSettings {
    pub fn video_model_hint(&self) -> String {
        std::env::var("MONOIZE_STUDIO_VIDEO_MODEL").unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Agent tools
// ---------------------------------------------------------------------------

fn tools_schema() -> Value {
    serde_json::from_str(TOOLS_JSON).unwrap_or(Value::Array(Vec::new()))
}

const TOOLS_JSON: &str = r#"[
  {"type": "function", "function": {
    "name": "update_script",
    "description": "Write or replace the screenplay text on the script node.",
    "parameters": {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}}},
  {"type": "function", "function": {
    "name": "create_shots",
    "description": "Replace the shot list (storyboard) derived from the script.",
    "parameters": {"type": "object", "properties": {
      "shots": {"type": "array", "items": {"type": "object", "properties": {
        "description": {"type": "string"}, "camera": {"type": "string"},
        "duration_sec": {"type": "integer"}, "dialogue": {"type": "string"}},
        "required": ["description"]}}}, "required": ["shots"]}}},
  {"type": "function", "function": {
    "name": "generate_images",
    "description": "Create first-frame image generations for shots (or a standalone prompt).",
    "parameters": {"type": "object", "properties": {
      "shot_ids": {"type": "array", "items": {"type": "string"}},
      "prompt": {"type": "string"}}}}},
  {"type": "function", "function": {
    "name": "generate_videos",
    "description": "Create per-shot videos from ready first frames.",
    "parameters": {"type": "object", "properties": {
      "shot_ids": {"type": "array", "items": {"type": "string"}}}}},
  {"type": "function", "function": {
    "name": "run_pipeline",
    "description": "Run the full six-stage pipeline: draft, split shots, synthesize prompts, frames, then videos.",
    "parameters": {"type": "object", "properties": {"idea": {"type": "string"}}, "required": ["idea"]}}}
]"#;

pub struct AgentOutcome {
    pub final_message: String,
    pub graph: Value,
}

pub async fn drive_agent(
    state: &StudioState,
    run: &RunRow,
    user_message: &str,
) -> Result<AgentOutcome, String> {
    let mut graph = match run.project_id.as_deref() {
        Some(project_id) => store::get_project(&state.db, &run.user_id, project_id)
            .await?
            .map(|p| serde_json::from_str(&p.graph_json).unwrap_or_else(|_| empty_graph()))
            .unwrap_or_else(empty_graph),
        None => empty_graph(),
    };
    let system = format!(
        "You are the creation agent of an AI film studio running on a relay gateway. \
You operate an infinite canvas: a script node hosts the screenplay, shot nodes form the storyboard, \
image nodes are first frames, video nodes are final clips. {summary}\n\
Work through the provided tools. Prefer run_pipeline for full short-film requests; \
use finer tools when the user asks for a partial change. Answer in the user's language.",
        summary = graph_summary(&graph)
    );
    let mut messages = vec![
        json!({"role": "system", "content": system}),
        json!({"role": "user", "content": user_message}),
    ];
    let mut final_message = String::new();
    for _ in 0..MAX_TOOL_ITERATIONS {
        let result =
            record_llm_step(state, run, "agent-turn", &messages, Some(&tools_schema())).await?;
        if result.tool_calls.is_empty() {
            final_message = result.content;
            break;
        }
        messages.push(json!({"role": "assistant", "content": result.content, "tool_calls": result.tool_calls}));
        for call in &result.tool_calls {
            let name = call
                .pointer("/function/name")
                .and_then(Value::as_str)
                .unwrap_or("");
            let raw_args = call
                .pointer("/function/arguments")
                .and_then(Value::as_str)
                .unwrap_or("{}");
            let args: Value = serde_json::from_str(raw_args).unwrap_or(json!({}));
            let tool_result = execute_tool(state, run, &mut graph, name, &args).await;
            state.publish(
                &run.user_id,
                "agent_tool",
                json!({"run_id": run.id, "tool": name, "digest": truncate(&args.to_string(), 200), "result": truncate(&tool_result, 400)}),
            );
            messages.push(json!({"role": "tool", "tool_call_id": call.get("id").and_then(Value::as_str).unwrap_or(""), "content": tool_result}));
        }
    }
    if final_message.is_empty() {
        final_message = "Agent reached the tool-iteration limit.".to_string();
    }
    Ok(AgentOutcome {
        final_message,
        graph,
    })
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let cut: String = text.chars().take(max).collect();
        format!("{cut}…")
    }
}

async fn persist_graph(state: &StudioState, run: &RunRow, graph: &Value) {
    if let Some(project_id) = run.project_id.as_deref() {
        if let Ok(serialized) = serde_json::to_string(graph) {
            let _ = store::engine_save_graph(&state.db, project_id, &serialized).await;
            state.publish(
                &run.user_id,
                "graph_patch",
                json!({"project_id": project_id, "version": graph.get("version").and_then(Value::as_i64).unwrap_or(0), "patch": {}}),
            );
        }
    }
}

async fn execute_tool(
    state: &StudioState,
    run: &RunRow,
    graph: &mut Value,
    name: &str,
    args: &Value,
) -> String {
    match name {
        "update_script" => {
            let text = args.get("text").and_then(Value::as_str).unwrap_or("");
            let script_id = ensure_script_node(graph);
            store::graph_mutate_node(
                graph,
                &script_id,
                &json!({"text": text, "stage": "drafted", "updated_at": now_string()}),
            );
            persist_graph(state, run, graph).await;
            format!("script updated ({} chars)", text.chars().count())
        }
        "create_shots" => {
            let shots = args
                .get("shots")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let script_id = ensure_script_node(graph);
            write_shots(graph, &script_id, &shots);
            store::graph_mutate_node(graph, &script_id, &json!({"stage": "split"}));
            persist_graph(state, run, graph).await;
            format!("created {} shots", shots.len())
        }
        "generate_images" => {
            let settings = crate::studio::StudioSettings::load(&state.settings_store).await;
            let mut created = 0usize;
            if let Some(prompt) = args.get("prompt").and_then(Value::as_str) {
                let node_id = Uuid::new_v4().to_string();
                let (x, y) = store::graph_next_node_position(graph, 5);
                if let Some(nodes) = graph.get_mut("nodes").and_then(Value::as_array_mut) {
                    nodes.push(
                        json!({"id": node_id, "kind": "image", "position": {"x": x, "y": y},
                        "data": {"prompt": prompt, "status": "queued", "asset_id": Value::Null}}),
                    );
                }
                create_image_step(
                    state,
                    run,
                    &GenRequest {
                        prompt: prompt.to_string(),
                        seconds: None,
                        size: None,
                        image: None,
                        node_id: Some(node_id),
                        price_nano: settings.image_price_nano(),
                    },
                )
                .await;
                created += 1;
            }
            for shot in shot_nodes(graph) {
                let id = shot
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if !shot_selected(args, &id) {
                    continue;
                }
                let prompt = shot
                    .pointer("/data/prompt")
                    .or_else(|| shot.pointer("/data/description"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if prompt.is_empty() {
                    continue;
                }
                let node_id = Uuid::new_v4().to_string();
                let (x, y) = store::graph_next_node_position(graph, 9);
                if let Some(nodes) = graph.get_mut("nodes").and_then(Value::as_array_mut) {
                    nodes.push(json!({"id": node_id, "kind": "image", "position": {"x": x, "y": y},
                        "data": {"prompt": prompt, "status": "queued", "asset_id": Value::Null, "shot": id}}));
                }
                if let Some(edges) = graph.get_mut("edges").and_then(Value::as_array_mut) {
                    edges.push(json!({"id": Uuid::new_v4().to_string(), "source": node_id, "target": id, "kind": "consistency"}));
                }
                store::graph_mutate_node(graph, &id, &json!({"image_node": node_id}));
                create_image_step(
                    state,
                    run,
                    &GenRequest {
                        prompt: prompt.to_string(),
                        seconds: None,
                        size: None,
                        image: None,
                        node_id: Some(node_id),
                        price_nano: settings.image_price_nano(),
                    },
                )
                .await;
                created += 1;
            }
            persist_graph(state, run, graph).await;
            format!("queued {created} image steps")
        }
        "generate_videos" => {
            let settings = crate::studio::StudioSettings::load(&state.settings_store).await;
            let mut created = 0usize;
            let mut skipped = 0usize;
            for shot in shot_nodes(graph) {
                let id = shot
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if !shot_selected(args, &id) {
                    continue;
                }
                let image = ready_frame_data_url(graph, &shot);
                let prompt = shot
                    .pointer("/data/prompt")
                    .or_else(|| shot.pointer("/data/description"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if image.is_none() && prompt.is_empty() {
                    skipped += 1;
                    continue;
                }
                let node_id = Uuid::new_v4().to_string();
                let (x, y) = store::graph_next_node_position(graph, 13);
                if let Some(nodes) = graph.get_mut("nodes").and_then(Value::as_array_mut) {
                    nodes.push(json!({"id": node_id, "kind": "video", "position": {"x": x, "y": y},
                        "data": {"prompt": prompt, "status": "queued", "asset_id": Value::Null, "shot": id}}));
                }
                store::graph_mutate_node(graph, &id, &json!({"video_node": node_id}));
                create_video_step(
                    state,
                    run,
                    &GenRequest {
                        prompt: prompt.to_string(),
                        seconds: shot
                            .pointer("/data/duration_sec")
                            .and_then(Value::as_i64)
                            .map(|s| s.to_string()),
                        size: None,
                        image,
                        node_id: Some(node_id),
                        price_nano: settings.video_price_nano(),
                    },
                )
                .await;
                created += 1;
            }
            persist_graph(state, run, graph).await;
            format!("queued {created} video steps ({skipped} skipped: no frame or prompt)")
        }
        "run_pipeline" => {
            let idea = args.get("idea").and_then(Value::as_str).unwrap_or("");
            run_pipeline(state, run, graph, idea).await
        }
        _ => format!("unknown tool {name}"),
    }
}

fn shot_selected(args: &Value, shot_node_id: &str) -> bool {
    match args.get("shot_ids").and_then(Value::as_array) {
        Some(ids) if !ids.is_empty() => ids
            .iter()
            .filter_map(Value::as_str)
            .any(|id| id == shot_node_id),
        _ => true,
    }
}

/// The frame's b64 payload is attached to the image node data by the engine
/// when the asset lands; videos read it from there.
fn ready_frame_data_url(graph: &Value, shot: &Value) -> Option<String> {
    let image_node = shot.pointer("/data/image_node").and_then(Value::as_str)?;
    graph
        .get("nodes")
        .and_then(Value::as_array)?
        .iter()
        .find(|n| n.get("id").and_then(Value::as_str) == Some(image_node))
        .and_then(|n| n.pointer("/data/b64"))
        .and_then(Value::as_str)
        .map(|b64| format!("data:image/png;base64,{b64}"))
}

/// ST-P1..P5: six-stage pipeline with per-shot independence. Stage 6 is
/// created by the engine's frames barrier, not here.
async fn run_pipeline(state: &StudioState, run: &RunRow, graph: &mut Value, idea: &str) -> String {
    // Stage 1: draft.
    let draft = record_llm_step(
        state,
        run,
        "pipeline-draft",
        &[json!({"role": "user", "content": format!(
            "Write a concise screenplay (max 12 shots) for this idea. Plain text, no markup:\n{idea}")})],
        None,
    )
    .await;
    let draft_text = match draft {
        Ok(result) if !result.content.trim().is_empty() => result.content,
        Ok(_) => idea.to_string(),
        Err(error) => return format!("pipeline failed at draft: {error}"),
    };
    let script_id = ensure_script_node(graph);
    store::graph_mutate_node(
        graph,
        &script_id,
        &json!({"text": draft_text, "stage": "drafted"}),
    );

    // Stage 2: split into shots.
    let split = record_llm_step(
        state,
        run,
        "pipeline-split",
        &[json!({"role": "user", "content": format!(
            "Split this screenplay into shots. Return ONLY a JSON array of objects {{\"description\",\"camera\",\"duration_sec\",\"dialogue\"}}.\n\n{draft_text}")})],
        None,
    )
    .await;
    let shots_value: Vec<Value> = match split {
        Ok(result) => extract_json_array(&result.content).unwrap_or_default(),
        Err(error) => return format!("pipeline failed at split: {error}"),
    };
    write_shots(graph, &script_id, &shots_value);
    store::graph_mutate_node(graph, &script_id, &json!({"stage": "split"}));

    // Stage 3: synthesize per-shot prompts.
    let shots_inline = serde_json::to_string(&shots_value).unwrap_or_default();
    let synthesize = record_llm_step(
        state,
        run,
        "pipeline-synthesize",
        &[json!({"role": "user", "content": format!(
            "For each shot write a vivid image-generation prompt (subject, action, lighting, style; keep characters consistent). \
Return ONLY a JSON array of strings aligned with the input order.\nShots:\n{shots_inline}\n\nScreenplay:\n{draft_text}")})],
        None,
    )
    .await;
    if let Ok(result) = synthesize {
        if let Some(prompts) = extract_json_array(&result.content) {
            for (index, prompt) in prompts.iter().enumerate() {
                let text = prompt.as_str().unwrap_or("");
                if let Some(shot) = shot_nodes(graph).get(index) {
                    let id = shot
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    store::graph_mutate_node(graph, &id, &json!({"prompt": text}));
                }
            }
        }
    }
    store::graph_mutate_node(graph, &script_id, &json!({"stage": "synthesized"}));

    // Stages 4+5: frames barrier (engine advances to videos when all frames land).
    let settings = crate::studio::StudioSettings::load(&state.settings_store).await;
    for shot in shot_nodes(graph) {
        let id = shot
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let prompt = shot
            .pointer("/data/prompt")
            .or_else(|| shot.pointer("/data/description"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if prompt.is_empty() {
            continue;
        }
        let node_id = Uuid::new_v4().to_string();
        let (x, y) = store::graph_next_node_position(graph, 9);
        if let Some(nodes) = graph.get_mut("nodes").and_then(Value::as_array_mut) {
            nodes.push(json!({"id": node_id, "kind": "image", "position": {"x": x, "y": y},
                "data": {"prompt": prompt, "status": "queued", "asset_id": Value::Null, "shot": id}}));
        }
        if let Some(edges) = graph.get_mut("edges").and_then(Value::as_array_mut) {
            edges.push(json!({"id": Uuid::new_v4().to_string(), "source": node_id, "target": id, "kind": "consistency"}));
        }
        store::graph_mutate_node(graph, &id, &json!({"image_node": node_id}));
        create_image_step(
            state,
            run,
            &GenRequest {
                prompt: prompt.to_string(),
                seconds: None,
                size: None,
                image: None,
                node_id: Some(node_id),
                price_nano: settings.image_price_nano(),
            },
        )
        .await;
    }
    store::graph_mutate_node(graph, &script_id, &json!({"stage": "frames"}));
    persist_graph(state, run, graph).await;
    format!(
        "pipeline: drafted, split into {} shots, prompts synthesized, frames queued",
        shots_value.len()
    )
}

fn extract_json_array(text: &str) -> Option<Vec<Value>> {
    let start = text.find('[')?;
    let end = text.rfind(']')?;
    let slice = &text[start..=end];
    serde_json::from_str(slice).ok()
}

/// ST-P5 barrier: when every frames-barrier image step of a pipeline run is
/// terminal, spawn the videos barrier. Returns the number of video steps made.
pub async fn advance_pipeline_barriers(state: &StudioState, run: &RunRow) -> usize {
    let Ok(steps) = store::list_steps_for_run(&state.db, &run.id).await else {
        return 0;
    };
    let frames: Vec<&crate::studio::store::StepRow> = steps
        .iter()
        .filter(|s| s.kind == "image" && step_barrier(s) == "frames")
        .collect();
    if frames.is_empty() || !frames.iter().all(|s| s.status == "succeeded") {
        return 0;
    }
    if steps.iter().any(|s| s.kind == "video") {
        return 0;
    }
    let graph = match run.project_id.as_deref() {
        Some(project_id) => store::get_project(&state.db, &run.user_id, project_id)
            .await
            .ok()
            .flatten()
            .map(|p| {
                serde_json::from_str::<Value>(&p.graph_json).unwrap_or_else(|_| empty_graph())
            }),
        None => None,
    };
    let Some(mut graph) = graph else {
        return 0;
    };
    let settings = crate::studio::StudioSettings::load(&state.settings_store).await;
    let mut created = 0usize;
    for shot in shot_nodes(&graph) {
        let id = shot
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let prompt = shot
            .pointer("/data/prompt")
            .or_else(|| shot.pointer("/data/description"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let image = ready_frame_data_url(&graph, &shot);
        let node_id = Uuid::new_v4().to_string();
        let (x, y) = store::graph_next_node_position(&graph, 13);
        if let Some(nodes) = graph.get_mut("nodes").and_then(Value::as_array_mut) {
            nodes.push(json!({"id": node_id, "kind": "video", "position": {"x": x, "y": y},
                "data": {"prompt": prompt, "status": "queued", "asset_id": Value::Null, "shot": id}}));
        }
        store::graph_mutate_node(&mut graph, &id, &json!({"video_node": node_id}));
        create_video_step(
            state,
            run,
            &GenRequest {
                prompt: prompt.to_string(),
                seconds: shot
                    .pointer("/data/duration_sec")
                    .and_then(Value::as_i64)
                    .map(|s| s.to_string()),
                size: None,
                image,
                node_id: Some(node_id),
                price_nano: settings.video_price_nano(),
            },
        )
        .await;
        created += 1;
    }
    if let Some(script_id) = script_node_id(&graph) {
        set_node_status(&mut graph, &script_id, "videos");
        store::graph_mutate_node(&mut graph, &script_id, &json!({"stage": "videos"}));
    }
    persist_graph(state, run, &graph).await;
    created
}

fn step_barrier(step: &crate::studio::store::StepRow) -> String {
    serde_json::from_str::<Value>(&step.payload_json)
        .ok()
        .and_then(|payload| {
            payload
                .get("barrier")
                .and_then(Value::as_str)
                .map(String::from)
        })
        .unwrap_or_default()
}

impl crate::studio::StudioSettings {
    pub fn image_price_nano(&self) -> i64 {
        std::env::var("MONOIZE_STUDIO_IMAGE_PRICE_NANO")
            .ok()
            .and_then(|raw| raw.parse::<i64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(4_000_000) // $0.04 per image run by default
    }
    pub fn video_price_nano(&self) -> i64 {
        std::env::var("MONOIZE_STUDIO_VIDEO_PRICE_NANO")
            .ok()
            .and_then(|raw| raw.parse::<i64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(500_000_000) // $0.50 per video run by default
    }
}
