//! ST-E1/E2/E3: studio execution surfaces — channel selection over the
//! existing provider registry and the openai_video / fal_video / replicate
//! video job protocols plus the OpenAI Images protocol for image steps.

use crate::monoize_routing::{
    MonoizeModelEntry, MonoizeProvider, MonoizeProviderType, MonoizeRoutingStore,
};
use crate::studio::StudioState;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Value, json};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ChannelChoice {
    pub provider_id: String,
    pub provider_name: String,
    pub multiplier: f64,
    pub base_url: String,
    pub api_key: String,
    pub provider_type: MonoizeProviderType,
    pub upstream_model: String,
    pub proxy_url: Option<String>,
}

#[derive(Debug)]
pub enum UpstreamError {
    SubmitRejected(String),
    Transport(String),
}

impl UpstreamError {
    pub fn message(&self) -> String {
        match self {
            Self::SubmitRejected(m) => m.clone(),
            Self::Transport(m) => m.clone(),
        }
    }
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Transport(_))
    }
}

fn provider_multiplier_f64(provider: &MonoizeProvider) -> f64 {
    provider
        .multiplier
        .to_string()
        .parse::<f64>()
        .ok()
        .filter(|value: &f64| value.is_finite() && *value >= 0.0)
        .unwrap_or(1.0)
}

fn upstream_model_of(entry: &MonoizeModelEntry) -> String {
    entry
        .redirect
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string()
}

/// Ordered candidate list for one (kind, model) pair, filtered by group when
/// the caller carries key groups (GR-E7). Weight is ignored in favor of
/// priority order; the submit retry (ST-E3) walks this list top-down.
pub fn select_channels(
    store: &MonoizeRoutingStore,
    providers: &[MonoizeProvider],
    wanted: MonoizeProviderType,
    model: &str,
    allowed_groups: Option<&[String]>,
) -> Vec<ChannelChoice> {
    let mut choices = Vec::new();
    for provider in providers {
        if !provider.enabled {
            continue;
        }
        if let Some(groups) = allowed_groups
            && !groups.is_empty()
            && !groups.iter().any(|g| g == &provider.group_id)
        {
            continue;
        }
        let channel = &provider.channel;
        if !channel.enabled || channel.provider_type != wanted {
            continue;
        }
        let Some(entry) = channel.models.get(model) else {
            continue;
        };
        let upstream_model = {
            let redirected = upstream_model_of(entry);
            if redirected.is_empty() {
                model.to_string()
            } else {
                redirected
            }
        };
        choices.push(ChannelChoice {
            provider_id: provider.id.clone(),
            provider_name: provider.name.clone(),
            multiplier: provider_multiplier_f64(provider),
            base_url: channel.base_url.trim_end_matches('/').to_string(),
            api_key: channel.api_key.clone(),
            provider_type: channel.provider_type,
            upstream_model,
            proxy_url: channel.proxy_url.clone(),
        });
    }
    let _ = store;
    choices
}

fn auth_header(choice: &ChannelChoice) -> (&'static str, String) {
    match choice.provider_type {
        MonoizeProviderType::FalVideo => ("authorization", format!("Key {}", choice.api_key)),
        _ => ("authorization", format!("Bearer {}", choice.api_key)),
    }
}

async fn http_client(
    state: &StudioState,
    proxy_url: Option<&str>,
) -> Result<reqwest::Client, String> {
    state.http_clients.for_channel_proxy(proxy_url)
}

async fn send_json(
    client: &reqwest::Client,
    method: reqwest::Method,
    url: &str,
    choice: &ChannelChoice,
    body: Value,
    timeout_ms: u64,
) -> Result<reqwest::Response, UpstreamError> {
    let (header, value) = auth_header(choice);
    let mut request = client
        .request(method, url.to_string())
        .header(header, value)
        .header("content-type", "application/json")
        .timeout(Duration::from_millis(timeout_ms));
    if let Some(extra) = extra_static_headers(choice) {
        for (name, val) in extra {
            request = request.header(name, val);
        }
    }
    request
        .json(&body)
        .send()
        .await
        .map_err(|e| UpstreamError::Transport(e.to_string()))
}

fn extra_static_headers(_choice: &ChannelChoice) -> Option<Vec<(&'static str, String)>> {
    None
}

async fn read_json(resp: reqwest::Response) -> Result<(reqwest::StatusCode, Value), UpstreamError> {
    let status = resp.status();
    let bytes = crate::bounded_response::read_response_body_with_limit(
        resp,
        crate::studio::asset_max_bytes().min(16 * 1024 * 1024),
    )
    .await
    .map_err(|e| UpstreamError::Transport(e.to_string()))?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|e| UpstreamError::Transport(format!("invalid upstream JSON: {e}")))?;
    Ok((status, value))
}

fn submit_failure(status: reqwest::StatusCode, body: &Value) -> UpstreamError {
    let detail = body
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(Value::as_str)
        .or_else(|| body.get("detail").and_then(Value::as_str))
        .unwrap_or("")
        .to_string();
    UpstreamError::SubmitRejected(format!("upstream {}: {}", status.as_u16(), detail))
}

// ---------------------------------------------------------------------------
// Submit
// ---------------------------------------------------------------------------

pub struct VideoSubmit {
    pub remote_ref: Value,
    pub progress_hint: Option<u8>,
}

pub async fn submit_video_job(
    state: &StudioState,
    choice: &ChannelChoice,
    prompt: &str,
    seconds: Option<&str>,
    size: Option<&str>,
    image: Option<&str>,
) -> Result<VideoSubmit, UpstreamError> {
    let client = http_client(state, choice.proxy_url.as_deref())
        .await
        .map_err(UpstreamError::Transport)?;
    let submit_timeout = 30_000_u64;
    match choice.provider_type {
        MonoizeProviderType::OpenaiVideo => {
            let mut body = json!({"model": choice.upstream_model, "prompt": prompt});
            if let Some(seconds) = seconds {
                body["seconds"] = json!(seconds);
            }
            if let Some(size) = size {
                body["size"] = json!(size);
            }
            if let Some(image) = image {
                body["input_reference"] = json!(image);
            }
            let resp = send_json(
                &client,
                reqwest::Method::POST,
                &format!("{}/videos", choice.base_url),
                choice,
                body,
                submit_timeout,
            )
            .await?;
            let (status, value) = read_json(resp).await?;
            if !status.is_success() {
                return Err(submit_failure(status, &value));
            }
            let id = value
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .ok_or_else(|| UpstreamError::Transport("upstream job id missing".into()))?;
            Ok(VideoSubmit {
                remote_ref: json!({
                    "kind": "openai_video",
                    "provider_id": choice.provider_id,
                    "base_url": choice.base_url,
                    "job_id": id,
                }),
                progress_hint: value
                    .get("progress")
                    .and_then(Value::as_u64)
                    .map(|p| p as u8),
            })
        }
        MonoizeProviderType::FalVideo => {
            let encoded = utf8_percent_encode(&choice.upstream_model, NON_ALPHANUMERIC).to_string();
            let mut body = json!({"prompt": prompt});
            if let Some(image) = image {
                body["image_url"] = json!(image);
            }
            if let Some(seconds) = seconds {
                body["duration"] = json!(seconds);
            }
            if let Some(size) = size {
                body["resolution"] = json!(size);
            }
            let resp = send_json(
                &client,
                reqwest::Method::POST,
                &format!("{}/{}", choice.base_url, encoded),
                choice,
                body,
                submit_timeout,
            )
            .await?;
            let (status, value) = read_json(resp).await?;
            if !status.is_success() {
                return Err(submit_failure(status, &value));
            }
            let id = value
                .get("request_id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .ok_or_else(|| UpstreamError::Transport("fal request_id missing".into()))?;
            Ok(VideoSubmit {
                remote_ref: json!({
                    "kind": "fal_video",
                    "provider_id": choice.provider_id,
                    "base_url": choice.base_url,
                    "job_id": id,
                    "status_url": value.get("status_url").and_then(Value::as_str),
                    "response_url": value.get("response_url").and_then(Value::as_str),
                    "cancel_url": value.get("cancel_url").and_then(Value::as_str),
                }),
                progress_hint: None,
            })
        }
        MonoizeProviderType::Replicate => {
            // Model-key convention from replicate-upstream.spec.md §2.2: the
            // whole key is one encoded segment for the model form.
            let encoded = utf8_percent_encode(&choice.upstream_model, NON_ALPHANUMERIC).to_string();
            let url = if let Some((owner, rest)) = choice.upstream_model.split_once('/') {
                if rest.contains(':') || choice.upstream_model.matches('/').count() > 1 {
                    format!("{}/v1/predictions", choice.base_url)
                } else {
                    let owner = utf8_percent_encode(owner, NON_ALPHANUMERIC).to_string();
                    let name = utf8_percent_encode(rest, NON_ALPHANUMERIC).to_string();
                    format!("{}/v1/models/{owner}/{name}/predictions", choice.base_url)
                }
            } else {
                format!("{}/v1/predictions", choice.base_url)
            };
            let _ = encoded;
            let mut body = json!({"input": {"prompt": prompt}});
            if let Some(image) = image {
                body["input"]["image"] = json!(image);
            }
            if let Some(seconds) = seconds {
                body["input"]["duration"] = json!(seconds);
            }
            let resp = send_json(
                &client,
                reqwest::Method::POST,
                &url,
                choice,
                body,
                submit_timeout,
            )
            .await?;
            let (status, value) = read_json(resp).await?;
            if !status.is_success() {
                return Err(submit_failure(status, &value));
            }
            let id = value
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .ok_or_else(|| {
                    UpstreamError::Transport("replicate prediction id missing".into())
                })?;
            Ok(VideoSubmit {
                remote_ref: json!({
                    "kind": "replicate",
                    "provider_id": choice.provider_id,
                    "base_url": choice.base_url,
                    "job_id": id,
                }),
                progress_hint: None,
            })
        }
        _ => Err(UpstreamError::SubmitRejected(
            "provider type is not a video executor".into(),
        )),
    }
}

// ---------------------------------------------------------------------------
// Poll
// ---------------------------------------------------------------------------

pub enum PollOutcome {
    Running {
        progress: Option<u8>,
    },
    Succeeded {
        media_url: Option<String>,
        mime: String,
    },
    Failed(String),
}

pub async fn poll_video_job(
    state: &StudioState,
    choice: &ChannelChoice,
    remote_ref: &Value,
) -> Result<PollOutcome, UpstreamError> {
    let client = http_client(state, choice.proxy_url.as_deref())
        .await
        .map_err(UpstreamError::Transport)?;
    let job_id = remote_ref
        .get("job_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let poll_timeout = 30_000_u64;
    match remote_ref.get("kind").and_then(Value::as_str) {
        Some("openai_video") => {
            let resp = send_json(
                &client,
                reqwest::Method::GET,
                &format!("{}/videos/{job_id}", choice.base_url),
                choice,
                json!({}),
                poll_timeout,
            )
            .await?;
            let (status, value) = read_json(resp).await?;
            if !status.is_success() {
                return Err(UpstreamError::Transport(format!(
                    "poll status {}",
                    status.as_u16()
                )));
            }
            let upstream_status = value
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            let progress = value
                .get("progress")
                .and_then(Value::as_u64)
                .map(|p| p.clamp(0, 100) as u8);
            match upstream_status.as_str() {
                "completed" | "succeeded" | "success" => Ok(PollOutcome::Succeeded {
                    media_url: None,
                    mime: "video/mp4".into(),
                }),
                "failed" | "error" => Ok(PollOutcome::Failed(
                    value
                        .pointer("/error/message")
                        .or_else(|| value.get("error"))
                        .and_then(Value::as_str)
                        .unwrap_or("upstream video job failed")
                        .to_string(),
                )),
                "cancelled" | "canceled" => Ok(PollOutcome::Failed("canceled".into())),
                _ => Ok(PollOutcome::Running { progress }),
            }
        }
        Some("fal_video") => {
            let status_url = remote_ref
                .get("status_url")
                .and_then(Value::as_str)
                .map(|path| {
                    if path.starts_with("http") {
                        path.to_string()
                    } else {
                        format!("{}/{path}", choice.base_url)
                    }
                })
                .unwrap_or_else(|| format!("{}/{job_id}/status", choice.base_url));
            let resp = send_json(
                &client,
                reqwest::Method::GET,
                &status_url,
                choice,
                json!({}),
                poll_timeout,
            )
            .await?;
            let (status, value) = read_json(resp).await?;
            if !status.is_success() {
                return Err(UpstreamError::Transport(format!(
                    "poll status {}",
                    status.as_u16()
                )));
            }
            match value.get("status").and_then(Value::as_str) {
                Some("COMPLETED") => {
                    let response_url = remote_ref
                        .get("response_url")
                        .and_then(Value::as_str)
                        .map(|path| {
                            if path.starts_with("http") {
                                path.to_string()
                            } else {
                                format!("{}/{path}", choice.base_url)
                            }
                        })
                        .unwrap_or_else(|| format!("{}/{job_id}", choice.base_url));
                    let resp = send_json(
                        &client,
                        reqwest::Method::GET,
                        &response_url,
                        choice,
                        json!({}),
                        poll_timeout,
                    )
                    .await?;
                    let (status, value) = read_json(resp).await?;
                    if !status.is_success() {
                        return Err(UpstreamError::Transport(format!(
                            "result status {}",
                            status.as_u16()
                        )));
                    }
                    let media = extract_fal_video_url(&value);
                    match media {
                        Some(url) => Ok(PollOutcome::Succeeded {
                            media_url: Some(url),
                            mime: "video/mp4".into(),
                        }),
                        None => Ok(PollOutcome::Failed(
                            "fal result carried no video url".into(),
                        )),
                    }
                }
                Some("IN_PROGRESS") | Some("QUEUED") => {
                    let position = value.get("queue_position").and_then(Value::as_u64);
                    Ok(PollOutcome::Running {
                        progress: position.map(|p| (100u64.saturating_sub(p.min(99))) as u8),
                    })
                }
                _ => Ok(PollOutcome::Failed("fal job failed".into())),
            }
        }
        Some("replicate") => {
            let resp = send_json(
                &client,
                reqwest::Method::GET,
                &format!("{}/v1/predictions/{job_id}", choice.base_url),
                choice,
                json!({}),
                poll_timeout,
            )
            .await?;
            let (status, value) = read_json(resp).await?;
            if !status.is_success() {
                return Err(UpstreamError::Transport(format!(
                    "poll status {}",
                    status.as_u16()
                )));
            }
            match value.get("status").and_then(Value::as_str) {
                Some("succeeded") => {
                    let output = value.get("output");
                    let url = extract_replicate_media(output);
                    match url {
                        Some(url) => Ok(PollOutcome::Succeeded {
                            media_url: Some(url),
                            mime: "video/mp4".into(),
                        }),
                        None => Ok(PollOutcome::Failed(
                            "replicate output carried no media".into(),
                        )),
                    }
                }
                Some("failed") | Some("canceled") => Ok(PollOutcome::Failed(
                    value
                        .get("error")
                        .and_then(Value::as_str)
                        .map(String::from)
                        .unwrap_or_else(|| "upstream failed".to_string()),
                )),
                _ => Ok(PollOutcome::Running { progress: None }),
            }
        }
        _ => Err(UpstreamError::SubmitRejected(
            "unknown remote ref kind".into(),
        )),
    }
}

fn extract_fal_video_url(output: &Value) -> Option<String> {
    for pointer in ["/video/url", "/url", "/videos/0/url"] {
        if let Some(url) = output.pointer(pointer).and_then(Value::as_str) {
            return Some(url.to_string());
        }
    }
    output
        .as_array()
        .and_then(|items| items.first())
        .and_then(|item| match item {
            Value::String(s) => Some(s.clone()),
            other => other.get("url").and_then(Value::as_str).map(String::from),
        })
}

fn extract_replicate_media(output: Option<&Value>) -> Option<String> {
    let output = output?;
    match output {
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => items
            .iter()
            .find_map(|item| item.as_str().map(String::from)),
        _ => None,
    }
    .filter(|url| {
        url.contains("replicate.delivery")
            || url.ends_with(".mp4")
            || url.ends_with(".webm")
            || url.ends_with(".mov")
    })
}

// ---------------------------------------------------------------------------
// Cancel
// ---------------------------------------------------------------------------

pub async fn cancel_video_job(state: &StudioState, choice: &ChannelChoice, remote_ref: &Value) {
    let Ok(client) = http_client(state, choice.proxy_url.as_deref()).await else {
        return;
    };
    let job_id = remote_ref
        .get("job_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let url = match remote_ref.get("kind").and_then(Value::as_str) {
        Some("openai_video") => format!("{}/videos/{job_id}/cancel", choice.base_url),
        Some("fal_video") => {
            match remote_ref.get("cancel_url").and_then(Value::as_str) {
                Some(path) if path.starts_with("http") => path.to_string(),
                _ => return, // FV-C1: local-only cancellation without a cancel_url
            }
        }
        Some("replicate") => format!("{}/v1/predictions/{job_id}/cancel", choice.base_url),
        _ => return,
    };
    let _ = send_json(
        &client,
        reqwest::Method::POST,
        &url,
        choice,
        json!({}),
        10_000,
    )
    .await;
}

// ---------------------------------------------------------------------------
// Media streaming (ST-E5)
// ---------------------------------------------------------------------------

/// Resolves the fetch for one asset: either the content endpoint of the owning
/// job or the extracted remote media URL. Never exposes credentials to
/// third-party CDN hosts.
pub async fn open_media_stream(
    state: &StudioState,
    choice: &ChannelChoice,
    remote_ref: &Value,
    media_url: Option<&str>,
) -> Result<reqwest::Response, String> {
    let client = http_client(state, choice.proxy_url.as_deref()).await?;
    let cap = Duration::from_secs(120);
    if let Some(media_url) = media_url.filter(|url| url.starts_with("http")) {
        let third_party = !media_url.contains(&choice.base_url);
        let mut request = client.get(media_url.to_string()).timeout(cap);
        if !third_party {
            let (header, value) = auth_header(choice);
            request = request.header(header, value);
        }
        return request
            .send()
            .await
            .map_err(|e| format!("media fetch failed: {e}"));
    }
    let job_id = remote_ref
        .get("job_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let url = match remote_ref.get("kind").and_then(Value::as_str) {
        Some("openai_video") => format!("{}/videos/{job_id}/content", choice.base_url),
        Some("replicate") => format!("{}/v1/predictions/{job_id}/content", choice.base_url),
        _ => return Err("no media url for this asset".into()),
    };
    let (header, value) = auth_header(choice);
    client
        .get(url)
        .header(header, value)
        .timeout(cap)
        .send()
        .await
        .map_err(|e| format!("media fetch failed: {e}"))
}

// ---------------------------------------------------------------------------
// Image steps (ST-E1)
// ---------------------------------------------------------------------------

pub async fn submit_image_job(
    state: &StudioState,
    choice: &ChannelChoice,
    prompt: &str,
    size: Option<&str>,
) -> Result<Value, UpstreamError> {
    let client = http_client(state, choice.proxy_url.as_deref())
        .await
        .map_err(UpstreamError::Transport)?;
    let mut body = json!({"model": choice.upstream_model, "prompt": prompt, "n": 1});
    if let Some(size) = size {
        body["size"] = json!(size);
    }
    let resp = send_json(
        &client,
        reqwest::Method::POST,
        &format!("{}/v1/images/generations", choice.base_url),
        choice,
        body,
        crate::studio::image_timeout_ms(),
    )
    .await?;
    let (status, value) = read_json(resp).await?;
    if !status.is_success() {
        return Err(submit_failure(status, &value));
    }
    let first = value.pointer("/data/0").cloned().unwrap_or(Value::Null);
    if first.is_null() {
        return Err(UpstreamError::Transport(
            "image response carried no data".into(),
        ));
    }
    Ok(json!({
        "kind": "openai_image",
        "provider_id": choice.provider_id,
        "base_url": choice.base_url,
        "b64_json": first.get("b64_json").and_then(Value::as_str),
        "url": first.get("url").and_then(Value::as_str),
        "revised_prompt": value.pointer("/data/0/revised_prompt").and_then(Value::as_str),
    }))
}
