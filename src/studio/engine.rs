//! ST-S1..S7: the background engine. One scheduler task; dispatches pending
//! steps under admission limits, polls running upstream jobs with backoff,
//! settles billing, mutates graphs, and publishes SSE events.

use crate::monoize_routing::MonoizeProviderType;
use crate::studio::StudioState;
use crate::studio::agent;
use crate::studio::store::{self, RunRow, StepRow};
use crate::studio::upstream::{self, ChannelChoice, PollOutcome, UpstreamError};
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

pub fn spawn(state: Arc<StudioState>) {
    tokio::spawn(async move {
        let mut tick: u64 = 0;
        loop {
            if state.shutdown.load(std::sync::atomic::Ordering::Acquire) {
                break;
            }
            if let Err(error) = run_tick(&state).await {
                tracing::warn!(%error, "studio engine tick failed");
            }
            tick = tick.wrapping_add(1);
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        let _ = tick;
    });
}

async fn run_tick(state: &Arc<StudioState>) -> Result<(), String> {
    drive_agent_runs(state).await?;
    dispatch_pending(state).await?;
    poll_running(state).await?;
    finalize_runs(state).await?;
    Ok(())
}

/// Agent runs are driven inline (ST-A3): their LLM turns execute in the tick
/// path so step records and billing stay centralized.
async fn drive_agent_runs(state: &Arc<StudioState>) -> Result<(), String> {
    for run in store::list_runs(&state.db, None, 64).await? {
        if state.shutdown.load(std::sync::atomic::Ordering::Acquire) {
            break;
        }
        if run.status != "queued" || run.kind != "agent" {
            continue;
        }
        if !admit_run(state, &run).await? {
            continue;
        }
        // The agent message is attached to the first queued llm step payload.
        let steps = store::list_steps_for_run(&state.db, &run.id).await?;
        let user_message = steps
            .iter()
            .find(|s| s.kind == "llm" && s.status == "pending")
            .and_then(|s| {
                serde_json::from_str::<Value>(&s.payload_json)
                    .ok()
                    .and_then(|p| {
                        p.get("user_message")
                            .and_then(Value::as_str)
                            .map(String::from)
                    })
            })
            .unwrap_or_default();
        if user_message.is_empty() {
            let _ = store::update_run_status(
                &state.db,
                &run.id,
                "failed",
                Some("agent run missing user message"),
            )
            .await;
            continue;
        }
        let _ = store::update_run_status(&state.db, &run.id, "running", None).await;
        state.publish(
            &run.user_id,
            "run_update",
            json!({"run_id": run.id, "status": "running"}),
        );
        match agent::drive_agent(state, &run, &user_message).await {
            Ok(outcome) => {
                let _ = store::update_run_status(&state.db, &run.id, "running", None).await;
                state.publish(
                    &run.user_id,
                    "agent_delta",
                    json!({"run_id": run.id, "text": outcome.final_message}),
                );
                let _ = store::update_run_status(&state.db, &run.id, "succeeded", None).await;
            }
            Err(error) => {
                let _ = store::update_run_status(&state.db, &run.id, "failed", Some(&error)).await;
            }
        }
        state.publish(
            &run.user_id,
            "run_update",
            run_event(&run.id, &store::get_run(&state.db, &run.id).await?),
        );
    }
    Ok(())
}

fn run_event(run_id: &str, run: &Option<RunRow>) -> Value {
    match run {
        Some(row) => json!({"run_id": run_id, "status": row.status, "error": row.error}),
        None => json!({"run_id": run_id}),
    }
}

async fn admit_run(state: &Arc<StudioState>, _run: &RunRow) -> Result<bool, String> {
    let global = store::active_run_counts(&state.db, None).await?;
    if global > crate::studio::global_active_runs() {
        return Ok(false);
    }
    Ok(true)
}

async fn dispatch_pending(state: &Arc<StudioState>) -> Result<(), String> {
    let global = store::active_run_counts(&state.db, None).await?;
    if global >= crate::studio::global_active_runs() {
        return Ok(());
    }
    for step in store::list_steps_in_status(&state.db, "pending", 16).await? {
        if state.shutdown.load(std::sync::atomic::Ordering::Acquire) {
            break;
        }
        let Some(run) = store::get_run(&state.db, &step.run_id).await? else {
            continue;
        };
        if run.status == "queued" {
            let _ = store::update_run_status(&state.db, &run.id, "running", None).await;
        }
        if run.status != "running" {
            continue;
        }
        if !per_user_admission(state, &run).await? {
            continue;
        }
        let payload: Value = serde_json::from_str(&step.payload_json).unwrap_or(json!({}));
        match step.kind.as_str() {
            "llm" => {
                // agent-turn steps are driven by drive_agent_runs; skip them here
                continue;
            }
            "image" => dispatch_generation(state, &run, &step, &payload, false).await?,
            "video" => dispatch_generation(state, &run, &step, &payload, true).await?,
            _ => {
                let _ = store::transition_step(
                    &state.db,
                    &step.id,
                    &["pending"],
                    "skipped",
                    Some("unknown step kind"),
                    None,
                )
                .await;
            }
        }
    }
    Ok(())
}

async fn per_user_admission(state: &Arc<StudioState>, run: &RunRow) -> Result<bool, String> {
    let count = store::active_run_counts(&state.db, Some(&run.user_id)).await?;
    Ok(count <= crate::studio::user_active_runs())
}

fn provider_type_for(step: &StepRow, payload: &Value) -> Option<MonoizeProviderType> {
    match step.kind.as_str() {
        "image" => Some(MonoizeProviderType::OpenaiImage),
        "video" => match payload.get("upstream_kind").and_then(Value::as_str) {
            Some("openai_video") => Some(MonoizeProviderType::OpenaiVideo),
            Some("fal_video") => Some(MonoizeProviderType::FalVideo),
            Some("replicate") => Some(MonoizeProviderType::Replicate),
            _ => None,
        },
        _ => None,
    }
}

fn model_for(step: &StepRow, payload: &Value) -> String {
    payload
        .get("model")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
        .map(String::from)
        .unwrap_or_else(|| match step.kind.as_str() {
            "image" => {
                std::env::var("MONOIZE_STUDIO_IMAGE_MODEL").unwrap_or_else(|_| "gpt-image-2".into())
            }
            _ => std::env::var("MONOIZE_STUDIO_VIDEO_MODEL").unwrap_or_else(|_| "sora-2".into()),
        })
}

/// ST-E3: bounded submit retry across the ordered candidate list.
async fn dispatch_generation(
    state: &Arc<StudioState>,
    run: &RunRow,
    step: &StepRow,
    payload: &Value,
    video: bool,
) -> Result<(), String> {
    let Some(wanted) = provider_type_for(step, payload) else {
        let _ = store::transition_step(
            &state.db,
            &step.id,
            &["pending"],
            "failed",
            Some("no upstream kind configured"),
            None,
        )
        .await;
        return Ok(());
    };
    let model = model_for(step, payload);
    let prompt = payload
        .get("prompt")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if prompt.trim().is_empty() {
        let _ = store::transition_step(
            &state.db,
            &step.id,
            &["pending"],
            "failed",
            Some("empty prompt"),
            None,
        )
        .await;
        return Ok(());
    }
    let providers = state
        .monoize_store
        .list_providers()
        .await
        .unwrap_or_default();
    let candidates =
        upstream::select_channels(&state.monoize_store, &providers, wanted, &model, None);
    if candidates.is_empty() {
        let _ = store::transition_step(
            &state.db,
            &step.id,
            &["pending"],
            "failed",
            Some("no eligible channel for model"),
            None,
        )
        .await;
        return Ok(());
    }
    let mut submitted: Option<(ChannelChoice, Value)> = None;
    let mut last_error = String::new();
    for choice in candidates.iter().take(2) {
        let outcome = if video {
            upstream::submit_video_job(
                state,
                choice,
                &prompt,
                payload.get("seconds").and_then(Value::as_str),
                payload.get("size").and_then(Value::as_str),
                payload.get("image").and_then(Value::as_str),
            )
            .await
            .map(|submit| {
                (
                    json!({"remote": submit.remote_ref, "provider_id": choice.provider_id,
                           "base_url": choice.base_url, "api_key": choice.api_key,
                           "provider_type": choice.provider_type.as_str(),
                           "progress": submit.progress_hint}),
                    choice.multiplier,
                )
            })
        } else {
            upstream::submit_image_job(
                state,
                choice,
                &prompt,
                payload.get("size").and_then(Value::as_str),
            )
            .await
            .map(|image_ref| {
                (
                    json!({"remote": image_ref, "provider_id": choice.provider_id,
                           "base_url": choice.base_url, "api_key": choice.api_key,
                           "provider_type": choice.provider_type.as_str()}),
                    choice.multiplier,
                )
            })
        };
        match outcome {
            Ok((result_state, multiplier)) => {
                submitted = Some((choice.clone(), {
                    let mut merged = result_state.clone();
                    merged["multiplier"] = json!(multiplier);
                    merged
                }));
                break;
            }
            Err(UpstreamError::SubmitRejected(message)) => {
                last_error = message;
                break; // hard failure, no channel walk
            }
            Err(UpstreamError::Transport(message)) => {
                last_error = message;
                continue;
            }
        }
    }
    let Some((choice, result_state)) = submitted else {
        let _ = store::transition_step(
            &state.db,
            &step.id,
            &["pending"],
            "failed",
            Some(&format!("submit failed: {last_error}")),
            None,
        )
        .await;
        return Ok(());
    };

    // MB-ST1: pre-charge at dispatch. Image jobs are synchronous, so their
    // charge settles immediately in the same transition.
    let base_price = payload
        .get("price_nano")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let multiplier = result_state
        .get("multiplier")
        .and_then(Value::as_f64)
        .unwrap_or(1.0);
    let price = ((base_price as f64) * multiplier).ceil() as i64;
    if price > 0 {
        let reason = if video {
            "studio_video_charge"
        } else {
            "studio_image_charge"
        };
        let meta = json!({"request_id": format!("studio_step_{}", step.id)});
        if let Err(error) = state
            .user_store
            .studio_charge_balance(&run.user_id, price as i128, reason, &meta)
            .await
        {
            let _ = store::transition_step(
                &state.db,
                &step.id,
                &["pending"],
                "failed",
                Some(&error),
                None,
            )
            .await;
            return Ok(());
        }
        let _ = store::add_step_charge(&state.db, &step.id, price).await;
    }

    // Image submissions carry the finished payload already (synchronous).
    if !video {
        let image_ref = result_state.get("remote").cloned().unwrap_or(Value::Null);
        finish_generation_step(state, run, step, &image_ref, price).await?;
        let _ = choice;
        return Ok(());
    }

    let merged_payload = {
        let mut merged = payload.clone();
        merged["dispatch"] = result_state;
        serde_json::to_string(&merged).map_err(|e| e.to_string())?
    };
    // pending -> running with the dispatch state attached.
    state
        .db
        .write()
        .await
        .execute(state.db.stmt(
            "UPDATE studio_steps SET status = 'running', payload_json = $1, updated_at = $2, attempts = attempts + 1 WHERE id = $3 AND status = 'pending'",
            vec![merged_payload.into(), chrono::Utc::now().to_rfc3339().into(), step.id.clone().into()],
        ))
        .await
        .map_err(|e| e.to_string())?;
    state.publish(
        &run.user_id,
        "step_update",
        json!({"step_id": step.id, "status": "running", "kind": step.kind}),
    );
    Ok(())
}

/// Shared success path: create the asset row, patch the graph node, publish.
async fn finish_generation_step(
    state: &Arc<StudioState>,
    run: &RunRow,
    step: &StepRow,
    remote_ref: &Value,
    charge: i64,
) -> Result<(), String> {
    let kind = if step.kind == "video" {
        "video"
    } else {
        "image"
    };
    let asset_id = Uuid::new_v4().to_string();
    let mime = if kind == "video" {
        "video/mp4".to_string()
    } else {
        remote_ref
            .get("b64_json")
            .and_then(Value::as_str)
            .map(|_| "image/png".to_string())
            .unwrap_or_else(|| "image/jpeg".to_string())
    };
    store::insert_asset(
        &state.db,
        &asset_id,
        &run.user_id,
        &run.id,
        &step.id,
        kind,
        remote_ref
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("openai_image"),
        remote_ref.get("provider_id").and_then(Value::as_str),
        &remote_ref.to_string(),
        &mime,
    )
    .await?;
    let result = json!({"asset_id": asset_id, "charge_nano_usd": charge});
    let _ = store::transition_step(
        &state.db,
        &step.id,
        &["running", "pending"],
        "succeeded",
        None,
        Some(&result.to_string()),
    )
    .await;
    state.publish(
        &run.user_id,
        "step_update",
        json!({"step_id": step.id, "status": "succeeded", "kind": kind, "asset_id": asset_id}),
    );
    if let (Some(project_id), Some(node_id)) = (run.project_id.as_deref(), step.node_id.as_deref())
    {
        if let Some(mut project) = store::get_project(&state.db, &run.user_id, project_id).await? {
            let mut graph: Value =
                serde_json::from_str(&project.graph_json).unwrap_or_else(|_| agent::empty_graph());
            let mut patch = json!({"status": "succeeded", "asset_id": asset_id});
            if kind == "image"
                && let Some(b64) = remote_ref.get("b64_json").and_then(Value::as_str)
            {
                patch["b64"] = json!(b64);
            }
            store::graph_mutate_node(&mut graph, node_id, &patch);
            let _ = store::engine_save_graph(&state.db, project_id, &graph.to_string()).await;
            state.publish(
                &run.user_id,
                "graph_patch",
                json!({"project_id": project_id, "node_id": node_id, "patch": patch}),
            );
            let _ = &mut project;
        }
    }
    Ok(())
}

async fn poll_running(state: &Arc<StudioState>) -> Result<(), String> {
    for step in store::list_steps_in_status(&state.db, "running", 32).await? {
        if state.shutdown.load(std::sync::atomic::Ordering::Acquire) {
            break;
        }
        let payload: Value = serde_json::from_str(&step.payload_json).unwrap_or(json!({}));
        let Some(dispatch) = payload.get("dispatch").cloned() else {
            continue;
        };
        // Backoff gate (ST-S5): poll when updated_at + backoff has elapsed.
        let updated = chrono::DateTime::parse_from_rfc3339(&step.updated_at)
            .map(|t| t.with_timezone(&chrono::Utc).timestamp())
            .unwrap_or(0);
        let now = chrono::Utc::now().timestamp();
        if now < updated + crate::studio::poll_backoff_secs(step.attempts.max(1)) as i64 {
            // ST-S4 timeout check still applies between polls.
            let created = chrono::DateTime::parse_from_rfc3339(&step.created_at)
                .map(|t| t.with_timezone(&chrono::Utc).timestamp())
                .unwrap_or(now);
            let timeout = crate::studio::video_timeout_ms() / 1000;
            if now > created + timeout as i64 {
                if let Some(run) = store::get_run(&state.db, &step.run_id).await? {
                    refund_step(state, &run, &step).await;
                }
                let _ = store::transition_step(
                    &state.db,
                    &step.id,
                    &["running"],
                    "failed",
                    Some("video step timed out"),
                    None,
                )
                .await;
            }
            continue;
        }
        let Some(run) = store::get_run(&state.db, &step.run_id).await? else {
            continue;
        };
        let choice = ChannelChoice {
            provider_id: dispatch
                .get("provider_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            provider_name: String::new(),
            multiplier: dispatch
                .get("multiplier")
                .and_then(Value::as_f64)
                .unwrap_or(1.0),
            base_url: dispatch
                .get("base_url")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            api_key: dispatch
                .get("api_key")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            provider_type: crate::monoize_routing::MonoizeProviderType::from_str(
                dispatch
                    .get("provider_type")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            )
            .unwrap_or(MonoizeProviderType::OpenaiVideo),
            upstream_model: String::new(),
            proxy_url: None,
        };
        let remote_ref = dispatch.get("remote").cloned().unwrap_or(Value::Null);
        match upstream::poll_video_job(state, &choice, &remote_ref).await {
            Ok(PollOutcome::Running { progress }) => {
                let _ = store::transition_step(
                    &state.db,
                    &step.id,
                    &["running"],
                    "running",
                    None,
                    None,
                )
                .await;
                if let Some(progress) = progress {
                    state.publish(
                        &run.user_id,
                        "step_update",
                        json!({"step_id": step.id, "status": "running", "progress": progress}),
                    );
                }
            }
            Ok(PollOutcome::Succeeded { media_url, mime }) => {
                let mut remote = remote_ref.clone();
                if let Some(media_url) = media_url {
                    remote["media_url"] = json!(media_url);
                }
                remote["mime"] = json!(mime);
                let charge = step.charge_nano_usd;
                finish_generation_step(state, &run, &step, &remote, charge).await?;
            }
            Ok(PollOutcome::Failed(message)) => {
                let canceled = message == "canceled";
                refund_step(state, &run, &step).await;
                let to = if canceled { "canceled" } else { "failed" };
                let _ = store::transition_step(
                    &state.db,
                    &step.id,
                    &["running"],
                    to,
                    Some(&message),
                    None,
                )
                .await;
            }
            Err(error) => {
                // Transport blips keep polling; hard rejections fail the step.
                if matches!(error, UpstreamError::SubmitRejected(_)) {
                    refund_step(state, &run, &step).await;
                    let _ = store::transition_step(
                        &state.db,
                        &step.id,
                        &["running"],
                        "failed",
                        Some(&error.message()),
                        None,
                    )
                    .await;
                } else {
                    let _ = store::transition_step(
                        &state.db,
                        &step.id,
                        &["running"],
                        "running",
                        None,
                        None,
                    )
                    .await;
                }
            }
        }
    }
    Ok(())
}

/// MB-ST3: full idempotent refund for failed/canceled steps.
async fn refund_step(state: &Arc<StudioState>, run: &RunRow, step: &StepRow) {
    if step.charge_nano_usd <= 0 || step.refund_nano_usd > 0 {
        return;
    }
    let reason = if step.kind == "video" {
        "studio_video_refund"
    } else {
        "studio_image_refund"
    };
    let meta = json!({"request_id": format!("studio_step_{}", step.id)});
    if let Err(error) = state
        .user_store
        .studio_refund_balance(&run.user_id, step.charge_nano_usd as i128, reason, &meta)
        .await
    {
        tracing::warn!(%error, step_id = %step.id, "studio refund failed");
        return;
    }
    let _ = store::mark_step_refunded(&state.db, &step.id, step.charge_nano_usd).await;
}

/// ST-S3 + ST-P3: terminal recompute and the pipeline frames barrier.
async fn finalize_runs(state: &Arc<StudioState>) -> Result<(), String> {
    for run in store::list_runs(&state.db, None, 64).await? {
        if run.status != "running" {
            continue;
        }
        let steps = store::list_steps_for_run(&state.db, &run.id).await?;
        if steps.is_empty() {
            continue;
        }
        if steps
            .iter()
            .any(|s| matches!(s.status.as_str(), "pending" | "running"))
        {
            continue;
        }
        // ST-P5: advance the frames barrier before declaring the run done.
        let created_videos = agent::advance_pipeline_barriers(state, &run).await;
        let steps = store::list_steps_for_run(&state.db, &run.id).await?;
        if steps
            .iter()
            .any(|s| matches!(s.status.as_str(), "pending" | "running"))
        {
            let _ = created_videos;
            continue;
        }
        let failed = steps.iter().any(|s| s.status == "failed");
        let succeeded = steps.iter().any(|s| s.status == "succeeded");
        let status = if failed && succeeded {
            "partial"
        } else if failed {
            "failed"
        } else {
            "succeeded"
        };
        let error = if failed {
            steps
                .iter()
                .find(|s| s.status == "failed")
                .and_then(|s| s.error.clone())
        } else {
            None
        };
        let _ = store::update_run_status(&state.db, &run.id, status, error.as_deref()).await;
        state.publish(
            &run.user_id,
            "run_update",
            json!({"run_id": run.id, "status": status}),
        );
    }
    Ok(())
}

/// ST-S7: cancel a run — pending steps cancel locally; running video steps
/// fire the upstream cancel best-effort; refunds follow.
pub async fn cancel_run(state: &Arc<StudioState>, run: &RunRow) -> Result<(), String> {
    for step in store::list_steps_for_run(&state.db, &run.id).await? {
        match step.status.as_str() {
            "pending" => {
                let _ = store::transition_step(
                    &state.db,
                    &step.id,
                    &["pending"],
                    "canceled",
                    None,
                    None,
                )
                .await;
            }
            "running" if step.kind == "video" => {
                let payload: Value = serde_json::from_str(&step.payload_json).unwrap_or(json!({}));
                if let Some(dispatch) = payload.get("dispatch") {
                    let choice = ChannelChoice {
                        provider_id: String::new(),
                        provider_name: String::new(),
                        multiplier: 1.0,
                        base_url: dispatch
                            .get("base_url")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        api_key: dispatch
                            .get("api_key")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        provider_type: MonoizeProviderType::from_str(
                            dispatch
                                .get("provider_type")
                                .and_then(Value::as_str)
                                .unwrap_or(""),
                        )
                        .unwrap_or(MonoizeProviderType::OpenaiVideo),
                        upstream_model: String::new(),
                        proxy_url: None,
                    };
                    upstream::cancel_video_job(
                        state,
                        &choice,
                        &dispatch.get("remote").cloned().unwrap_or(Value::Null),
                    )
                    .await;
                }
                refund_step(state, run, &step).await;
                let _ = store::transition_step(
                    &state.db,
                    &step.id,
                    &["running"],
                    "canceled",
                    None,
                    None,
                )
                .await;
            }
            _ => {}
        }
    }
    let _ = store::update_run_status(&state.db, &run.id, "canceled", None).await;
    state.publish(
        &run.user_id,
        "run_update",
        json!({"run_id": run.id, "status": "canceled"}),
    );
    Ok(())
}
