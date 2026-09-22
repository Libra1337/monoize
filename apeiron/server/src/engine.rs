//! AP-E1..E8: the run engine — step materialization, dispatch, upstream
//! execution, worker-job enqueueing, refunds, timeouts, finalization.

use crate::graph::{self, Graph, Node};
use crate::state::SharedState;
use serde_json::{json, Value};
use sqlx::Row;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct RunRow {
    pub id: String,
    pub user_id: String,
    pub project_id: Option<String>,
    pub kind: String,
    pub status: String,
    pub error: Option<String>,
    pub params: Value,
    pub created_at: String,
    pub updated_at: String,
    pub finished_at: Option<String>,
}

impl RunRow {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "user_id": self.user_id,
            "project_id": self.project_id,
            "kind": self.kind,
            "status": self.status,
            "error": self.error,
            "params": self.params,
            "created_at": self.created_at,
            "updated_at": self.updated_at,
            "finished_at": self.finished_at,
        })
    }
}

#[derive(Clone, Debug)]
pub struct StepRow {
    pub id: String,
    pub run_id: String,
    pub node_id: String,
    pub kind: String,
    pub status: String,
    pub payload: Value,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub attempts: i64,
    pub charge_nano_usd: Option<String>,
    pub refund_nano_usd: Option<String>,
    /// Upstream bookkeeping for video polls; `upstream_ref` never leaves the
    /// server because it embeds provider base URLs.
    #[allow(dead_code)]
    pub upstream_kind: Option<String>,
    #[allow(dead_code)]
    pub upstream_ref: Option<Value>,
    pub updated_at: String,
}

impl StepRow {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "run_id": self.run_id,
            "node_id": self.node_id,
            "kind": self.kind,
            "status": self.status,
            "payload": self.payload,
            "result": self.result,
            "error": self.error,
            "attempts": self.attempts,
            "charge_nano_usd": self.charge_nano_usd,
            "refund_nano_usd": self.refund_nano_usd,
            "updated_at": self.updated_at,
        })
    }
}

// ---------------------------------------------------------------------------
// Settings / prices
// ---------------------------------------------------------------------------

pub async fn get_setting(
    state: &SharedState,
    key: &str,
    default: &str,
) -> String {
    sqlx::query("SELECT value FROM settings WHERE key = $1")
        .bind(key)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
        .and_then(|row| row.try_get::<String, _>("value").ok())
        .unwrap_or_else(|| default.to_string())
}

pub async fn set_setting(state: &SharedState, key: &str, value: &str) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO settings (key, value, updated_at) VALUES ($1, $2, $3) \
         ON CONFLICT (key) DO UPDATE SET value = excluded.value, \
         updated_at = excluded.updated_at",
    )
    .bind(key)
    .bind(value)
    .bind(crate::now_rfc3339())
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// AP-B1 price table (nano-USD).
pub async fn step_price_nano(state: &SharedState, kind: &str) -> i128 {
    let (key, default) = match kind {
        "image" => ("price_image_nano", "4000000"),
        "video" => ("price_video_nano", "500000000"),
        "tts" => ("price_tts_nano", "20000000"),
        "assemble" => ("price_assemble_nano", "10000000"),
        "material" => ("price_material_nano", "0"),
        "subtitle" => ("price_subtitle_nano", "0"),
        _ => ("", "0"),
    };
    if key.is_empty() {
        return 0;
    }
    let raw = get_setting(state, key, default).await;
    crate::parse_nano(&raw).max(0)
}

pub fn llm_kind(step_kind: &str) -> bool {
    matches!(step_kind, "script" | "storyboard")
}

// ---------------------------------------------------------------------------
// Row accessors
// ---------------------------------------------------------------------------

pub async fn load_run(state: &SharedState, run_id: &str) -> Result<Option<RunRow>, String> {
    let row = sqlx::query(
        "SELECT id, user_id, project_id, kind, status, error, params_json, created_at, \
         updated_at, finished_at FROM runs WHERE id = $1",
    )
    .bind(run_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.map(map_run))
}

fn map_run(row: sqlx::sqlite::SqliteRow) -> RunRow {
    RunRow {
        id: row.try_get("id").unwrap_or_default(),
        user_id: row.try_get("user_id").unwrap_or_default(),
        project_id: row.try_get("project_id").ok().flatten(),
        kind: row.try_get("kind").unwrap_or_default(),
        status: row.try_get("status").unwrap_or_default(),
        error: row.try_get("error").ok().flatten(),
        params: row
            .try_get::<String, _>("params_json")
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_else(|| json!({})),
        created_at: row.try_get("created_at").unwrap_or_default(),
        updated_at: row.try_get("updated_at").unwrap_or_default(),
        finished_at: row.try_get("finished_at").ok().flatten(),
    }
}

pub async fn load_run_steps(state: &SharedState, run_id: &str) -> Result<Vec<StepRow>, String> {
    let rows = sqlx::query(
        "SELECT id, run_id, node_id, kind, status, payload_json, result_json, error, attempts, \
         charge_nano_usd, refund_nano_usd, upstream_kind, upstream_ref, updated_at \
         FROM steps WHERE run_id = $1 ORDER BY created_at ASC",
    )
    .bind(run_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(map_step).collect())
}

fn map_step(row: sqlx::sqlite::SqliteRow) -> StepRow {
    StepRow {
        id: row.try_get("id").unwrap_or_default(),
        run_id: row.try_get("run_id").unwrap_or_default(),
        node_id: row.try_get("node_id").unwrap_or_default(),
        kind: row.try_get("kind").unwrap_or_default(),
        status: row.try_get("status").unwrap_or_default(),
        payload: row
            .try_get::<String, _>("payload_json")
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_else(|| json!({})),
        result: row
            .try_get::<Option<String>, _>("result_json")
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str(&raw).ok()),
        error: row.try_get("error").ok().flatten(),
        attempts: row.try_get("attempts").unwrap_or(0),
        charge_nano_usd: row.try_get("charge_nano_usd").ok().flatten(),
        refund_nano_usd: row.try_get("refund_nano_usd").ok().flatten(),
        upstream_kind: row.try_get("upstream_kind").ok().flatten(),
        upstream_ref: row
            .try_get::<Option<String>, _>("upstream_ref")
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str(&raw).ok()),
        updated_at: row.try_get("updated_at").unwrap_or_default(),
    }
}

pub async fn load_graph_for_run(
    state: &SharedState,
    run: &RunRow,
) -> Result<Graph, String> {
    let Some(project_id) = &run.project_id else {
        return Err("run has no project".into());
    };
    let row = sqlx::query("SELECT graph_json FROM projects WHERE id = $1")
        .bind(project_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "project not found".to_string())?;
    let raw: String = row.try_get("graph_json").unwrap_or_default();
    Graph::parse(&raw)
}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

pub async fn spawn(state: SharedState) {
    tracing::info!("apeiron engine started");
    loop {
        if state.shutdown.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        if let Err(error) = tick(&state).await {
            tracing::warn!(%error, "engine tick failed");
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn tick(state: &SharedState) -> Result<(), String> {
    reclaim_stale_jobs(state).await?;
    let rows = sqlx::query(
        "SELECT id, user_id, project_id, kind, status, error, params_json, created_at, \
         updated_at, finished_at FROM runs WHERE status IN ('queued', 'running')",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    for row in rows {
        let run = map_run(row);
        if let Err(error) = process_run(state, &run).await {
            tracing::warn!(run = %run.id, %error, "process run failed");
            fail_run(state, &run, &error).await;
        }
    }
    Ok(())
}

pub async fn fail_run(state: &SharedState, run: &RunRow, error: &str) {
    let now = crate::now_rfc3339();
    let _ = sqlx::query(
        "UPDATE runs SET status = 'failed', error = $1, updated_at = $2, finished_at = $2 \
         WHERE id = $3 AND status IN ('queued', 'running')",
    )
    .bind(crate::short_error(error))
    .bind(&now)
    .bind(&run.id)
    .execute(&state.db)
    .await;
    if let Ok(steps) = load_run_steps(state, &run.id).await {
        for step in steps.iter().filter(|s| s.status == "pending") {
            let _ = finish_step(state, step, "canceled", None, Some("run aborted")).await;
        }
        for step in steps.iter().filter(|s| s.status == "running") {
            let _ = finish_step(state, step, "failed", None, Some("run aborted")).await;
        }
    }
    if let Ok(Some(updated)) = load_run(state, &run.id).await {
        state.publish(&run.user_id, "run_update", updated.to_json());
    }
}

/// AP-W3: jobs whose worker missed two heartbeat windows return to the queue
/// once, then fail.
async fn reclaim_stale_jobs(state: &SharedState) -> Result<(), String> {
    let cutoff = (chrono::Utc::now() - chrono::Duration::seconds(120))
        .to_rfc3339();
    let rows = sqlx::query(
        "SELECT id, step_id, run_id, attempts FROM jobs \
         WHERE status IN ('claimed', 'running') AND \
         (heartbeat_at IS NULL OR heartbeat_at < $1)",
    )
    .bind(&cutoff)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    for row in rows {
        let job_id: String = row.try_get("id").unwrap_or_default();
        let attempts: i64 = row.try_get("attempts").unwrap_or(0);
        if attempts < 2 {
            let _ = sqlx::query(
                "UPDATE jobs SET status = 'queued', worker_id = NULL, updated_at = $1 \
                 WHERE id = $2",
            )
            .bind(crate::now_rfc3339())
            .bind(&job_id)
            .execute(&state.db)
            .await;
        } else {
            let _ = sqlx::query(
                "UPDATE jobs SET status = 'failed', error = 'worker_lease_lost', \
                 updated_at = $1 WHERE id = $2",
            )
            .bind(crate::now_rfc3339())
            .bind(&job_id)
            .execute(&state.db)
            .await;
            let step_id: Option<String> = row.try_get("step_id").ok().flatten();
            if let Some(step_id) = step_id {
                if let Ok(Some(step)) = load_step(state, &step_id).await {
                    let _ =
                        finish_step(state, &step, "failed", None, Some("worker_lease_lost"))
                            .await;
                }
            }
            let run_id: String = row.try_get("run_id").unwrap_or_default();
            finalize_run(state, &run_id).await;
        }
    }
    Ok(())
}

pub async fn load_step(state: &SharedState, step_id: &str) -> Result<Option<StepRow>, String> {
    let row = sqlx::query(
        "SELECT id, run_id, node_id, kind, status, payload_json, result_json, error, attempts, \
         charge_nano_usd, refund_nano_usd, upstream_kind, upstream_ref, updated_at \
         FROM steps WHERE id = $1",
    )
    .bind(step_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.map(map_step))
}

async fn process_run(state: &SharedState, run: &RunRow) -> Result<(), String> {
    if run.status == "queued" {
        let _ = sqlx::query(
            "UPDATE runs SET status = 'running', updated_at = $1 WHERE id = $2 AND status = 'queued'",
        )
        .bind(crate::now_rfc3339())
        .bind(&run.id)
        .execute(&state.db)
        .await;
        state.publish(&run.user_id, "run_update", run.to_json());
    }
    let graph = match load_graph_for_run(state, run).await {
        Ok(graph) => graph,
        Err(error) => {
            fail_run(state, run, &error).await;
            return Ok(());
        }
    };
    let root = run.params.get("root").and_then(Value::as_str);
    let reachable = graph::reachable_nodes(&graph, root);
    let order: Vec<&Node> = graph::topo_order(&graph)
        .into_iter()
        .filter(|node| reachable.contains(&node.id))
        .collect();

    let all_steps = load_run_steps(state, &run.id).await?;
    let mut by_node: HashMap<String, Vec<StepRow>> = HashMap::new();
    for step in &all_steps {
        by_node.entry(step.node_id.clone()).or_default().push(step.clone());
    }

    let mut progressed = false;
    for node in &order {
        let existing = by_node.get(&node.id).cloned().unwrap_or_default();
        if existing.is_empty() {
            // Materialize or skip depending on upstream outcomes (AP-E2/E3).
            let upstream = graph::upstream_nodes(&graph, &node.id);
            let upstream_steps: Vec<StepRow> = upstream
                .iter()
                .flat_map(|id| by_node.get(id).cloned().unwrap_or_default())
                .collect();
            if upstream_steps.is_empty()
                || upstream_steps
                    .iter()
                    .all(|step| step.status == "succeeded")
            {
                create_steps_for_node(state, run, &graph, node, &by_node).await?;
                progressed = true;
            } else if upstream_steps
                .iter()
                .all(|step| matches!(step.status.as_str(), "failed" | "canceled" | "skipped"))
            {
                insert_step(state, run, node, json!({ "skipped": true }), "skipped").await?;
                progressed = true;
            }
        } else {
            // Dispatch every pending step whose node dependencies succeeded.
            let upstream = graph::upstream_nodes(&graph, &node.id);
            let upstream_ok = upstream.iter().all(|id| {
                by_node
                    .get(id)
                    .is_some_and(|steps| steps.iter().all(|step| step.status == "succeeded"))
            });
            if upstream_ok {
                let mut refreshed = Vec::new();
                if let Ok(steps) = load_run_steps(state, &run.id).await {
                    refreshed = steps.into_iter().filter(|s| s.node_id == node.id).collect();
                }
                for step in refreshed.iter().filter(|step| step.status == "pending") {
                    if dispatch_step(state, run, &graph, node, step, &by_node).await? {
                        progressed = true;
                    }
                }
                by_node.insert(node.id.clone(), refreshed);
            }
        }
        // Refresh node step cache after mutations.
        if progressed {
            if let Ok(steps) = load_run_steps(state, &run.id).await {
                let node_steps: Vec<StepRow> =
                    steps.into_iter().filter(|s| s.node_id == node.id).collect();
                if !node_steps.is_empty() {
                    by_node.insert(node.id.clone(), node_steps);
                }
            }
        }
    }

    if !progressed {
        finalize_run(state, &run.id).await;
    }
    Ok(())
}

async fn insert_step(
    state: &SharedState,
    run: &RunRow,
    node: &Node,
    payload: Value,
    status: &str,
) -> Result<StepRow, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::now_rfc3339();
    sqlx::query(
        "INSERT INTO steps (id, run_id, node_id, kind, status, payload_json, created_at, \
         updated_at, finished_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $7, $7)",
    )
    .bind(&id)
    .bind(&run.id)
    .bind(&node.id)
    .bind(&node.kind)
    .bind(status)
    .bind(payload.to_string())
    .bind(&now)
    .execute(&state.db)
    .await
    .map_err(|e| format!("insert step: {e}"))?;
    let step = StepRow {
        id,
        run_id: run.id.clone(),
        node_id: node.id.clone(),
        kind: node.kind.clone(),
        status: status.to_string(),
        payload,
        result: None,
        error: None,
        attempts: 0,
        charge_nano_usd: None,
        refund_nano_usd: None,
        upstream_kind: None,
        upstream_ref: None,
        updated_at: now,
    };
    if status != "skipped" {
        state.publish(&run.user_id, "step_update", step.to_json());
    }
    Ok(step)
}

async fn create_steps_for_node(
    state: &SharedState,
    run: &RunRow,
    graph: &Graph,
    node: &Node,
    by_node: &HashMap<String, Vec<StepRow>>,
) -> Result<(), String> {
    if graph::fan_out_kind(&node.kind) && graph::fed_by_storyboard(graph, node) {
        // AP-E2: one step per shot, created from the storyboard result.
        let source = graph::storyboard_source(graph, node).unwrap_or_default();
        let shots = by_node
            .get(&source)
            .and_then(|steps| steps.first())
            .and_then(|step| step.result.as_ref())
            .and_then(|result| result.get("shots"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for (index, shot) in shots.iter().enumerate() {
            insert_step(
                state,
                run,
                node,
                json!({ "shot": shot, "shot_index": index }),
                "pending",
            )
            .await?;
        }
        if shots.is_empty() {
            // A storyboard with zero parsed shots yields no executable work.
            insert_step(state, run, node, json!({ "skipped": true }), "skipped").await?;
        }
    } else {
        insert_step(state, run, node, json!({}), "pending").await?;
    }
    Ok(())
}

/// Resolve the inputs a step needs from producer step results.
struct ResolvedInputs {
    text: Option<String>,
    image_asset_id: Option<String>,
    clip_assets: Vec<Value>,
    audio_asset_id: Option<String>,
    subtitle_asset_id: Option<String>,
}

async fn resolve_inputs(
    state: &SharedState,
    graph: &Graph,
    node: &Node,
    step: &StepRow,
    by_node: &HashMap<String, Vec<StepRow>>,
) -> Result<ResolvedInputs, String> {
    let mut inputs = ResolvedInputs {
        text: None,
        image_asset_id: None,
        clip_assets: Vec::new(),
        audio_asset_id: None,
        subtitle_asset_id: None,
    };
    for edge in graph.edges.iter().filter(|e| e.target == node.id) {
        let producer_kind = graph.kind_of(&edge.source).unwrap_or("");
        let steps = by_node.get(&edge.source).cloned().unwrap_or_default();
        let results: Vec<Value> = steps
            .iter()
            .filter_map(|step| step.result.clone())
            .collect();
        match edge.targetPort.as_str() {
            "text" => {
                if producer_kind == "storyboard" {
                    if node.kind == "tts" || node.kind == "subtitle" {
                        // Full narration: join per-shot narrations (AP-G2 tts).
                        let joiner = node
                            .params
                            .get("joiner")
                            .and_then(Value::as_str)
                            .unwrap_or("\n\n");
                        let narrations: Vec<String> = steps
                            .iter()
                            .filter_map(|s| s.result.as_ref())
                            .filter_map(|r| r.get("shots"))
                            .filter_map(Value::as_array)
                            .flat_map(|shots| shots.iter().map(|shot| shot.to_string()))
                            .collect();
                        let mut parts = Vec::new();
                        for raw in narrations {
                            if let Ok(shot) = serde_json::from_str::<Value>(&raw) {
                                parts.push(
                                    shot
                                        .get("narration")
                                        .and_then(Value::as_str)
                                        .unwrap_or("")
                                        .to_string(),
                                );
                            }
                        }
                        let joined = parts
                            .into_iter()
                            .filter(|part| !part.trim().is_empty())
                            .collect::<Vec<_>>()
                            .join(joiner);
                        inputs.text = Some(joined);
                    } else if let Some(shot) = step.payload.get("shot") {
                        // Fan-out consumers: the step's own shot drives the prompt.
                        let base = shot
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let keywords = shot
                            .get("keywords")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        inputs.text = Some(if keywords.is_empty() {
                            base.to_string()
                        } else {
                            format!("{base}. {keywords}")
                        });
                    }
                } else {
                    let text = results
                        .iter()
                        .filter_map(|r| r.get("text"))
                        .filter_map(Value::as_str)
                        .next()
                        .unwrap_or("")
                        .to_string();
                    inputs.text = Some(text);
                }
            }
            "image" => {
                inputs.image_asset_id = results
                    .iter()
                    .filter_map(|r| r.get("asset_id").and_then(Value::as_str))
                    .next()
                    .map(str::to_string);
            }
            "audio" => {
                inputs.audio_asset_id = results
                    .iter()
                    .filter_map(|r| r.get("asset_id").and_then(Value::as_str))
                    .next()
                    .map(str::to_string);
            }
            "subtitle" => {
                inputs.subtitle_asset_id = results
                    .iter()
                    .filter_map(|r| r.get("asset_id").and_then(Value::as_str))
                    .next()
                    .map(str::to_string);
            }
            "clips" => {
                let _ = state;
                for result in results {
                    if let Some(asset_id) = result.get("asset_id").and_then(Value::as_str) {
                        inputs.clip_assets.push(json!({
                            "asset_id": asset_id,
                            "producer_kind": producer_kind,
                            "shot_index": result.get("shot_index").and_then(Value::as_i64),
                            "duration_secs": result.get("duration_secs").and_then(Value::as_f64),
                        }));
                    }
                }
            }
            _ => {}
        }
    }
    inputs
        .clip_assets
        .sort_by_key(|clip| clip.get("shot_index").and_then(Value::as_i64).unwrap_or(0));
    Ok(inputs)
}

/// AP-E2 dispatch: CAS pending -> running, bill, and spawn the executor.
/// Returns true when the step actually left `pending`.
async fn dispatch_step(
    state: &SharedState,
    run: &RunRow,
    graph: &Graph,
    node: &Node,
    step: &StepRow,
    by_node: &HashMap<String, Vec<StepRow>>,
) -> Result<bool, String> {
    let claimed = sqlx::query(
        "UPDATE steps SET status = 'running', updated_at = $1 WHERE id = $2 AND status = 'pending'",
    )
    .bind(crate::now_rfc3339())
    .bind(&step.id)
    .execute(&state.db)
    .await
    .map_err(|e| format!("claim step: {e}"))?;
    if claimed.rows_affected() == 0 {
        return Ok(false);
    }
    let mut claimed_step = step.clone();
    claimed_step.status = "running".to_string();
    state
        .publish(&run.user_id, "step_update", claimed_step.to_json());

    // AP-B2 pre-charge for fixed-price kinds.
    if !llm_kind(&step.kind) {
        let price = step_price_nano(state, &step.kind).await;
        if price > 0 {
            match charge_step(state, run, step, price).await {
                Ok(()) => {}
                Err(error) => {
                    let _ = finish_step(state, step, "failed", None, Some(&error)).await;
                    finalize_run(state, &run.id).await;
                    return Ok(true);
                }
            }
        }
    }

    let inputs = match resolve_inputs(state, graph, node, step, by_node).await {
        Ok(inputs) => inputs,
        Err(error) => {
            let _ = finish_step(state, step, "failed", None, Some(&error)).await;
            finalize_run(state, &run.id).await;
            return Ok(true);
        }
    };

    let task_state = state.clone();
    let task_run = run.clone();
    let task_step = claimed_step;
    let task_node = node.clone();
    let task_inputs = inputs;
    let graph_snapshot = graph.clone();
    tokio::spawn(async move {
        execute_step(task_state, task_run, graph_snapshot, task_node, task_step, task_inputs)
            .await;
    });
    Ok(true)
}

/// AP-B3: pre-charge with a per-step idempotency key; the charge amount is
/// persisted on the step so refunds settle exactly once.
pub async fn charge_step(
    state: &SharedState,
    run: &RunRow,
    step: &StepRow,
    amount_nano: i128,
) -> Result<(), String> {
    if amount_nano <= 0 {
        return Ok(());
    }
    let bridge = crate::bridge::BridgeClient::new(
        &state.http,
        &state.cfg.bridge_url,
        state.cfg.bridge_service_token.as_deref(),
    );
    let key = format!("apeiron_step_{}", step.id);
    bridge
        .settle("debit", &run.user_id, amount_nano, &key, json!({ "step": step.id }))
        .await
        .map_err(|error| match error {
            crate::bridge::BridgeError::Insufficient => "insufficient_balance".to_string(),
            other => format!("bridge_unavailable: {}", other.message()),
        })?;
    let _ = sqlx::query(
        "UPDATE steps SET charge_nano_usd = $1, updated_at = $2 WHERE id = $3",
    )
    .bind(amount_nano.to_string())
    .bind(crate::now_rfc3339())
    .bind(&step.id)
    .execute(&state.db)
    .await;
    refresh_mirror_balance(state, &run.user_id).await;
    Ok(())
}

/// AP-B3 settlement for token-metered LLM steps.
async fn settle_llm_step(
    state: &SharedState,
    run: &RunRow,
    step: &StepRow,
    provider: &crate::upstream::Provider,
    input_tokens: u64,
    output_tokens: u64,
) -> Result<(), String> {
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
    let raw_cost = input_tokens as i128 * in_rate + output_tokens as i128 * out_rate;
    let cost = ((raw_cost + 999_999) / 1_000_000).max(0);
    if cost <= 0 {
        return Ok(());
    }
    let bridge = crate::bridge::BridgeClient::new(
        &state.http,
        &state.cfg.bridge_url,
        state.cfg.bridge_service_token.as_deref(),
    );
    let key = format!("apeiron_step_llm_{}", step.id);
    bridge
        .settle("debit", &run.user_id, cost, &key, json!({ "step": step.id }))
        .await
        .map_err(|error| match error {
            crate::bridge::BridgeError::Insufficient => "insufficient_balance".to_string(),
            other => format!("bridge_unavailable: {}", other.message()),
        })?;
    let _ = sqlx::query(
        "UPDATE steps SET charge_nano_usd = $1, updated_at = $2 WHERE id = $3",
    )
    .bind(cost.to_string())
    .bind(crate::now_rfc3339())
    .bind(&step.id)
    .execute(&state.db)
    .await;
    refresh_mirror_balance(state, &run.user_id).await;
    Ok(())
}

/// AP-B5.
pub async fn refresh_mirror_balance(state: &SharedState, user_id: &str) {
    let bridge = crate::bridge::BridgeClient::new(
        &state.http,
        &state.cfg.bridge_url,
        state.cfg.bridge_service_token.as_deref(),
    );
    if let Ok((balance, unlimited)) = bridge.balance(user_id).await {
        let _ = sqlx::query(
            "UPDATE users SET balance_nano_usd = $1, balance_unlimited = $2, \
             balance_synced_at = $3 WHERE id = $4",
        )
        .bind(&balance)
        .bind(unlimited as i64)
        .bind(crate::now_rfc3339())
        .bind(user_id)
        .execute(&state.db)
        .await;
        state.publish(user_id, "balance_update", json!({ "balance_nano_usd": balance }));
    }
}

/// Terminal transition with refund bookkeeping (AP-B3). CAS on `running` keeps
/// late executor completions from overwriting finalized steps.
pub async fn finish_step(
    state: &SharedState,
    step: &StepRow,
    status: &str,
    result: Option<Value>,
    error: Option<&str>,
) -> Result<StepRow, String> {
    let now = crate::now_rfc3339();
    let outcome = sqlx::query(
        "UPDATE steps SET status = $1, result_json = $2, error = $3, updated_at = $4, \
         finished_at = $4 WHERE id = $5 AND status = 'running'",
    )
    .bind(status)
    .bind(result.as_ref().map(|value| value.to_string()))
    .bind(error.map(crate::short_error))
    .bind(&now)
    .bind(&step.id)
    .execute(&state.db)
    .await
    .map_err(|e| format!("finish step: {e}"))?;
    if outcome.rows_affected() == 0 {
        if let Some(existing) = load_step(state, &step.id).await? {
            return Ok(existing);
        }
        return Err("step vanished".into());
    }
    let mut finished = step.clone();
    finished.status = status.to_string();
    finished.result = result;
    finished.error = error.map(str::to_string);

    // Full refund exactly once for failed/canceled steps that were charged.
    if matches!(status, "failed" | "canceled") {
        let charge = finished
            .charge_nano_usd
            .as_deref()
            .map(crate::parse_nano)
            .unwrap_or(0);
        if charge > 0 {
            if finished.refund_nano_usd.is_none() {
                if let Some(run) = load_run(state, &step.run_id).await? {
                    let bridge = crate::bridge::BridgeClient::new(
                        &state.http,
                        &state.cfg.platform_url,
                        state.cfg.bridge_service_token.as_deref(),
                    );
                    let key = format!("apeiron_refund_{}", step.id);
                    if bridge
                        .settle("refund", &run.user_id, charge, &key, json!({ "step": step.id }))
                        .await
                        .is_ok()
                    {
                        let _ = sqlx::query(
                            "UPDATE steps SET refund_nano_usd = $1, updated_at = $2 WHERE id = $3",
                        )
                        .bind(charge.to_string())
                        .bind(crate::now_rfc3339())
                        .bind(&step.id)
                        .execute(&state.db)
                        .await;
                        finished.refund_nano_usd = Some(charge.to_string());
                        refresh_mirror_balance(state, &run.user_id).await;
                    }
                }
            }
        }
    }

    if let Some(run) = load_run(state, &step.run_id).await? {
        state.publish(&run.user_id, "step_update", finished.to_json());
    }
    Ok(finished)
}

/// AP-E1 finalization: no active steps and nothing left to materialize.
pub async fn finalize_run(state: &SharedState, run_id: &str) {
    let run = match load_run(state, run_id).await {
        Ok(Some(run)) => run,
        _ => return,
    };
    if !matches!(run.status.as_str(), "queued" | "running") {
        return;
    }
    let steps = match load_run_steps(state, run_id).await {
        Ok(steps) => steps,
        Err(_) => return,
    };
    if steps
        .iter()
        .any(|step| matches!(step.status.as_str(), "pending" | "running"))
    {
        return;
    }
    // A run with zero steps finalizes immediately as succeeded.
    let final_status = if steps.is_empty() {
        "succeeded"
    } else if steps.iter().any(|s| s.status == "canceled") && run.status == "canceled" {
        "canceled"
    } else if steps.iter().any(|s| s.status == "canceled") {
        "partial"
    } else if steps.iter().any(|s| s.status == "failed") {
        if steps.iter().any(|s| s.status == "succeeded") {
            "partial"
        } else {
            "failed"
        }
    } else {
        "succeeded"
    };
    let now = crate::now_rfc3339();
    let _ = sqlx::query(
        "UPDATE runs SET status = $1, updated_at = $2, finished_at = $2 WHERE id = $3 \
         AND status IN ('queued', 'running')",
    )
    .bind(final_status)
    .bind(&now)
    .bind(run_id)
    .execute(&state.db)
    .await;
    if let Ok(Some(updated)) = load_run(state, run_id).await {
        state.publish(&run.user_id, "run_update", updated.to_json());
    }
}

// ---------------------------------------------------------------------------
// Executors
// ---------------------------------------------------------------------------

async fn execute_step(
    state: SharedState,
    run: RunRow,
    graph: Graph,
    node: Node,
    step: StepRow,
    inputs: ResolvedInputs,
) {
    let outcome = match step.kind.as_str() {
        "script" | "storyboard" => run_llm_step(state.clone(), &run, &node, &step, &inputs).await,
        "image" => run_image_step(state.clone(), &run, &step, &node, &inputs).await,
        "video" => run_video_step(state.clone(), &run, &graph, &node, &step, &inputs).await,
        "tts" | "material" | "subtitle" | "assemble" => {
            enqueue_worker_job(state.clone(), &run, &node, &step, &inputs).await
        }
        _ => Err(format!("node kind {} is not executable", node.kind)),
    };
    match outcome {
        Ok(()) => {}
        Err(error) => {
            let _ = finish_step(&state, &step, "failed", None, Some(&error)).await;
        }
    }
    finalize_run(&state, &run.id).await;
}

fn storyboard_system_prompt(count: usize) -> String {
    format!(
        "You are a video director. Split the provided script into exactly {count} shots. \
         Reply with ONLY a JSON array, no markdown fences, no commentary. Each element: \
         {{\"key\": string, \"description\": string (visual description in English for image generation), \
         \"narration\": string (voice-over sentence), \"keywords\": string (2-4 stock footage search keywords in English), \
         \"duration_secs\": number (2-8)}}."
    )
}

pub fn parse_shots(text: &str, count: usize) -> Vec<Value> {
    let cleaned = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let slice = match (cleaned.find('['), cleaned.rfind(']')) {
        (Some(start), Some(end)) if end > start => &cleaned[start..=end],
        _ => return Vec::new(),
    };
    let Ok(Value::Array(raw)) = serde_json::from_str::<Value>(slice) else {
        return Vec::new();
    };
    let count = count.clamp(1, 24).max(1);
    raw.into_iter()
        .take(count)
        .enumerate()
        .map(|(index, shot)| {
            let get_str = |field: &str| {
                shot.get(field)
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_default()
            };
            let key = get_str("key");
            let key = if key.is_empty() { format!("shot-{index}") } else { key };
            json!({
                "key": key,
                "description": get_str("description"),
                "narration": get_str("narration"),
                "keywords": get_str("keywords"),
                "duration_secs": shot.get("duration_secs").and_then(Value::as_f64)
                    .filter(|secs| (1.0..=30.0).contains(secs))
                    .unwrap_or(5.0),
            })
        })
        .collect()
}

async fn run_llm_step(
    state: SharedState,
    run: &RunRow,
    node: &Node,
    step: &StepRow,
    inputs: &ResolvedInputs,
) -> Result<(), String> {
    let providers = crate::upstream::list_providers(&state, "llm").await?;
    if providers.is_empty() {
        return Err("no_provider: no enabled llm provider".into());
    }
    let provider = providers.into_iter().next().unwrap();
    let timeout = crate::http_timeout(&step.kind, &state.cfg);
    let prompt = inputs.text.clone().unwrap_or_else(|| {
        node.params
            .get("prompt")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    });
    if prompt.trim().is_empty() {
        return Err("invalid_params: the script prompt is empty".into());
    }
    let system = if step.kind == "storyboard" {
        let count = node
            .params
            .get("count")
            .and_then(Value::as_u64)
            .unwrap_or(6) as usize;
        storyboard_system_prompt(count)
    } else {
        node.params
            .get("system")
            .and_then(Value::as_str)
            .unwrap_or("You are a professional video script writer.")
            .to_string()
    };
    let output = crate::upstream::llm_complete(&state, &provider, &system, &prompt, timeout)
        .await
        .map_err(|e| format!("llm upstream: {e}"))?;
    let result = if step.kind == "storyboard" {
        let count = node
            .params
            .get("count")
            .and_then(Value::as_u64)
            .unwrap_or(6) as usize;
        let shots = parse_shots(&output.text, count);
        if shots.is_empty() {
            return Err("storyboard produced no parseable shots".into());
        }
        json!({ "text": output.text, "shots": shots })
    } else {
        json!({ "text": output.text })
    };
    // AP-B2: LLM settles by usage after completion.
    if let Err(error) =
        settle_llm_step(&state, run, step, &provider, output.input_tokens, output.output_tokens)
            .await
    {
        return Err(error);
    }
    finish_step(&state, step, "succeeded", Some(result), None).await?;
    Ok(())
}

async fn run_image_step(
    state: SharedState,
    run: &RunRow,
    step: &StepRow,
    node: &Node,
    inputs: &ResolvedInputs,
) -> Result<(), String> {
    let providers = crate::upstream::list_providers(&state, "image").await?;
    if providers.is_empty() {
        return Err("no_provider: no enabled image provider".into());
    }
    let prompt = inputs.text.clone().unwrap_or_else(|| {
        node.params
            .get("prompt")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    });
    if prompt.trim().is_empty() {
        return Err("invalid_params: the image prompt is empty".into());
    }
    let style = node.params.get("style").and_then(Value::as_str).unwrap_or("");
    let prompt = if style.is_empty() {
        prompt
    } else {
        format!("{prompt}. Style: {style}")
    };
    let size = node
        .params
        .get("size")
        .and_then(Value::as_str)
        .unwrap_or("1280x720");
    let timeout = crate::http_timeout("image", &state.cfg);
    let mut last_error = String::new();
    for provider in providers.iter().take(2) {
        match crate::upstream::image_generate(&state, provider, &prompt, size, timeout).await {
            Ok((bytes, mime, revised)) => {
                let mut meta = json!({ "provider": provider.id, "model": provider.model });
                if let Some(revised) = revised {
                    meta["revised_prompt"] = json!(revised);
                }
                let asset = crate::assets::store_asset(
                    &state,
                    &run.user_id,
                    Some(&run.id),
                    Some(&step.id),
                    "image",
                    &mime,
                    &bytes,
                    meta,
                )
                .await?;
                let shot_index = step.payload.get("shot_index").and_then(Value::as_i64);
                let duration_secs = step
                    .payload
                    .pointer("/shot/duration_secs")
                    .and_then(Value::as_f64);
                let mut result = json!({ "asset_id": asset.id, "mime_type": mime });
                if let Some(index) = shot_index {
                    result["shot_index"] = json!(index);
                }
                if let Some(duration) = duration_secs {
                    result["duration_secs"] = json!(duration);
                }
                finish_step(&state, step, "succeeded", Some(result), None).await?;
                return Ok(());
            }
            Err(error) => {
                last_error = error;
                tracing::warn!(step = %step.id, %last_error, "image provider attempt failed");
            }
        }
    }
    Err(format!("image upstream: {last_error}"))
}

async fn run_video_step(
    state: SharedState,
    run: &RunRow,
    graph: &Graph,
    node: &Node,
    step: &StepRow,
    inputs: &ResolvedInputs,
) -> Result<(), String> {
    let providers = crate::upstream::list_providers(&state, "video").await?;
    if providers.is_empty() {
        return Err("no_provider: no enabled video provider".into());
    }
    let prompt = inputs.text.clone().unwrap_or_else(|| {
        node.params
            .get("prompt")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    });
    if prompt.trim().is_empty() {
        return Err("invalid_params: the video prompt is empty".into());
    }
    let seconds = node.params.get("seconds").and_then(Value::as_str);
    let size = node.params.get("size").and_then(Value::as_str);

    // Optional first-frame image as a data URL.
    let mut image_data_url: Option<String> = None;
    if let Some(asset_id) = &inputs.image_asset_id {
        if let Ok(Some(asset)) = crate::assets::get_asset(&state, asset_id).await {
            if let Ok(bytes) = crate::assets::read_asset_bytes(&state, &asset).await {
                use base64::Engine;
                image_data_url = Some(format!(
                    "data:{};base64,{}",
                    asset.mime_type,
                    base64::engine::general_purpose::STANDARD.encode(&bytes)
                ));
            }
        }
    }

    // AP-E7: at most 2 submit attempts across ordered providers.
    let mut submitted: Option<(crate::upstream::Provider, Value)> = None;
    let mut last_error = String::new();
    for provider in providers.iter().take(2) {
        match crate::upstream::video_submit(
            &state,
            provider,
            &prompt,
            seconds,
            size,
            image_data_url.as_deref(),
        )
        .await
        {
            Ok(remote_ref) => {
                submitted = Some((provider.clone(), remote_ref));
                break;
            }
            Err(error) => {
                last_error = error;
                tracing::warn!(step = %step.id, %last_error, "video submit attempt failed");
            }
        }
    }
    let Some((provider, remote_ref)) = submitted else {
        return Err(format!("video submit: {last_error}"));
    };
    let _ = sqlx::query(
        "UPDATE steps SET upstream_kind = $1, upstream_ref = $2, attempts = attempts + 1, \
         updated_at = $3 WHERE id = $4",
    )
    .bind(&provider.upstream_kind)
    .bind(remote_ref.to_string())
    .bind(crate::now_rfc3339())
    .bind(&step.id)
    .execute(&state.db)
    .await;

    // Poll with backoff until terminal, timed out, or the run cancels.
    let deadline =
        tokio::time::Instant::now() + crate::http_timeout("video", &state.cfg);
    let mut attempts: i64 = 0;
    loop {
        if state.shutdown.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("server shutting down".into());
        }
        if let Ok(Some(current_run)) = load_run(&state, &run.id).await {
            if current_run.status == "canceled" {
                let _ = finish_step(&state, step, "canceled", None, Some("run canceled")).await;
                return Ok(());
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("video step timed out".into());
        }
        match crate::upstream::video_poll(&state, &provider, &remote_ref).await {
            Ok(crate::upstream::PollOutcome::Succeeded { media_url }) => {
                let response = crate::upstream::open_media_stream(
                    &state,
                    &provider,
                    &remote_ref,
                    media_url.as_deref(),
                )
                .await?;
                let asset = crate::assets::store_asset_from_response(
                    &state,
                    &run.user_id,
                    Some(&run.id),
                    Some(&step.id),
                    "video",
                    "video/mp4",
                    response,
                    json!({ "provider": provider.id, "model": provider.model }),
                )
                .await?;
                let shot_index = step.payload.get("shot_index").and_then(Value::as_i64);
                let mut result = json!({ "asset_id": asset.id, "mime_type": asset.mime_type });
                if let Some(index) = shot_index {
                    result["shot_index"] = json!(index);
                }
                finish_step(&state, step, "succeeded", Some(result), None).await?;
                return Ok(());
            }
            Ok(crate::upstream::PollOutcome::Failed(reason)) => {
                return Err(format!("video upstream failed: {reason}"));
            }
            Ok(crate::upstream::PollOutcome::Running { progress }) => {
                let _ = sqlx::query(
                    "UPDATE steps SET attempts = $1, updated_at = $2 WHERE id = $3",
                )
                .bind(attempts)
                .bind(crate::now_rfc3339())
                .bind(&step.id)
                .execute(&state.db)
                .await;
                if let Some(progress) = progress {
                    state.publish(
                        &run.user_id,
                        "step_progress",
                        json!({ "step_id": step.id, "progress": progress }),
                    );
                }
            }
            Err(error) => {
                tracing::warn!(step = %step.id, %error, "video poll error");
            }
        }
        let backoff = crate::poll_backoff_secs(attempts);
        attempts += 1;
        let _ = graph; // graph kept for signature symmetry with future node kinds
        tokio::time::sleep(Duration::from_secs(backoff)).await;
    }
}

async fn enqueue_worker_job(
    state: SharedState,
    run: &RunRow,
    node: &Node,
    step: &StepRow,
    inputs: &ResolvedInputs,
) -> Result<(), String> {
    let payload = build_job_payload(&state, node, step, inputs).await?;
    let job_id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO jobs (id, step_id, run_id, kind, status, payload_json, created_at, \
         updated_at) VALUES ($1, $2, $3, $4, 'queued', $5, $6, $6)",
    )
    .bind(&job_id)
    .bind(&step.id)
    .bind(&run.id)
    .bind(&step.kind)
    .bind(payload.to_string())
    .bind(crate::now_rfc3339())
    .execute(&state.db)
    .await
    .map_err(|e| format!("insert job: {e}"))?;
    state.publish(
        &run.user_id,
        "job_update",
        json!({ "id": job_id, "step_id": step.id, "kind": step.kind, "status": "queued" }),
    );
    Ok(())
}

async fn build_job_payload(
    state: &SharedState,
    node: &Node,
    step: &StepRow,
    inputs: &ResolvedInputs,
) -> Result<Value, String> {
    match step.kind.as_str() {
        "tts" => {
            let providers = crate::upstream::list_providers(state, "tts").await?;
            let provider =
                providers.into_iter().next().ok_or("no_provider: no enabled tts provider")?;
            let text = inputs
                .text
                .clone()
                .or_else(|| {
                    node.params
                        .get("text")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_default();
            if text.trim().is_empty() {
                return Err("invalid_params: the tts text is empty".into());
            }
            Ok(json!({
                "text": text,
                "voice": node.params.get("voice").and_then(Value::as_str).unwrap_or("alloy"),
                "speed": node.params.get("speed").and_then(Value::as_f64).unwrap_or(1.0),
                "provider": {
                    "base_url": provider.base_url,
                    "api_key": provider.api_key,
                    "model": provider.model,
                }
            }))
        }
        "material" => {
            let query = step
                .payload
                .pointer("/shot/keywords")
                .and_then(Value::as_str)
                .filter(|keywords| !keywords.trim().is_empty())
                .map(str::to_string)
                .or_else(|| {
                    inputs.text
                        .clone()
                        .or_else(|| {
                            node.params
                                .get("query")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                        })
                })
                .unwrap_or_default();
            let url = node.params.get("url").and_then(Value::as_str).unwrap_or("");
            if url.is_empty() && query.trim().is_empty() {
                return Err("invalid_params: the material query is empty".into());
            }
            let mut payload = json!({
                "query": query,
                "shot_index": step.payload.get("shot_index"),
                "duration_secs": step.payload.pointer("/shot/duration_secs"),
            });
            if !url.is_empty() {
                payload["url"] = json!(url);
            } else {
                let providers = crate::upstream::list_providers(state, "material").await?;
                let provider = providers
                    .into_iter()
                    .next()
                    .ok_or("no_provider: no enabled material provider")?;
                let source = if provider.upstream_kind == "pexels" {
                    json!("pexels")
                } else {
                    json!(provider.upstream_kind)
                };
                payload["provider"] = json!({
                    "source": source,
                    "api_key": provider.api_key,
                });
            }
            Ok(payload)
        }
        "subtitle" => {
            let text = inputs.text.clone().unwrap_or_default();
            if text.trim().is_empty() {
                return Err("invalid_params: the subtitle text is empty".into());
            }
            let lines: Vec<Value> = split_lines(&text)
                .into_iter()
                .map(|line| json!({ "text": line.text, "weight": line.weight }))
                .collect();
            Ok(json!({
                "lines": lines,
                "audio_asset_id": inputs.audio_asset_id,
            }))
        }
        "assemble" => {
            if inputs.clip_assets.is_empty() {
                return Err("invalid_params: assembly needs at least one clip".into());
            }
            let mut clips = Vec::new();
            for clip in &inputs.clip_assets {
                let asset_id = clip.get("asset_id").and_then(Value::as_str).unwrap_or("");
                let producer = clip.get("producer_kind").and_then(Value::as_str).unwrap_or("video");
                let duration = step
                    .payload
                    .pointer("/shot/duration_secs")
                    .and_then(Value::as_f64)
                    .or_else(|| clip.get("duration_secs").and_then(Value::as_f64))
                    .unwrap_or(4.0)
                    .clamp(1.0, 30.0);
                clips.push(json!({
                    "asset_id": asset_id,
                    "kind": if producer == "image" { "image" } else { "video" },
                    "duration_secs": if producer == "image" { duration } else { 0.0 },
                }));
            }
            Ok(json!({
                "clips": clips,
                "audio_asset_id": inputs.audio_asset_id,
                "subtitle_asset_id": inputs.subtitle_asset_id,
                "resolution": node.params.get("resolution").and_then(Value::as_str).unwrap_or("1280x720"),
                "fps": node.params.get("fps").and_then(Value::as_u64).unwrap_or(30),
            }))
        }
        other => Err(format!("no worker job for step kind {other}")),
    }
}

pub struct SubtitleLine {
    pub text: String,
    pub weight: f64,
}

/// Sentence split with character weights; lines cap at ~40 chars per cue.
pub fn split_lines(text: &str) -> Vec<SubtitleLine> {
    let mut lines = Vec::new();
    let mut buffer = String::new();
    for ch in text.chars() {
        buffer.push(ch);
        let boundary = matches!(ch, '。' | '！' | '？' | '.' | '!' | '?' | '\n' | ';');
        if boundary && buffer.trim().chars().count() >= 2 {
            let trimmed = buffer.trim();
            let mut piece = trimmed.to_string();
            while piece.chars().count() > 40 {
                let head: String = piece.chars().take(40).collect();
                lines.push(SubtitleLine {
                    text: head,
                    weight: 40.0,
                });
                piece = piece.chars().skip(40).collect();
            }
            let count = piece.chars().count() as f64;
            if count > 0.0 {
                lines.push(SubtitleLine { text: piece, weight: count });
            }
            buffer.clear();
        }
    }
    let remainder = buffer.trim();
    if !remainder.is_empty() {
        let mut piece = remainder.to_string();
        while piece.chars().count() > 40 {
            let head: String = piece.chars().take(40).collect();
            lines.push(SubtitleLine { text: head, weight: 40.0 });
            piece = piece.chars().skip(40).collect();
        }
        let count = piece.chars().count() as f64;
        if count > 0.0 {
            lines.push(SubtitleLine { text: piece, weight: count });
        }
    }
    lines
}
