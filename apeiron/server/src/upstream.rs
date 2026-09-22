//! AP-E7: upstream provider adapters — LLM chat, OpenAI Images, and the
//! openai_video / fal_video / replicate video job protocols, executed against
//! Apeiron's own provider registry.

use crate::state::SharedState;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{json, Value};
use sqlx::Row;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Provider {
    pub id: String,
    /// Registry columns surfaced for payload building and diagnostics; not
    /// every adapter reads every field.
    #[allow(dead_code)]
    pub kind: String,
    pub upstream_kind: String,
    #[allow(dead_code)]
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub params: Value,
    #[allow(dead_code)]
    pub weight: i64,
}

/// Enabled providers of one kind, weight DESC then insertion order (AP-E7).
pub async fn list_providers(
    state: &SharedState,
    kind: &str,
) -> Result<Vec<Provider>, String> {
    let rows = sqlx::query(
        "SELECT id, kind, upstream_kind, name, base_url, api_key, model, params_json, weight \
         FROM providers WHERE kind = $1 AND enabled = 1 \
         ORDER BY weight DESC, created_at ASC",
    )
    .bind(kind)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|row| Provider {
            id: row.try_get("id").unwrap_or_default(),
            kind: row.try_get("kind").unwrap_or_default(),
            upstream_kind: row.try_get("upstream_kind").unwrap_or_default(),
            name: row.try_get("name").unwrap_or_default(),
            base_url: row
                .try_get::<String, _>("base_url")
                .unwrap_or_default()
                .trim_end_matches('/')
                .to_string(),
            api_key: row.try_get("api_key").unwrap_or_default(),
            model: row.try_get("model").unwrap_or_default(),
            params: row
                .try_get::<String, _>("params_json")
                .ok()
                .and_then(|raw| serde_json::from_str(&raw).ok())
                .unwrap_or_else(|| json!({})),
            weight: row.try_get("weight").unwrap_or(0),
        })
        .collect())
}

fn auth_header(provider: &Provider) -> (&'static str, String) {
    match provider.upstream_kind.as_str() {
        "fal_video" => ("authorization", format!("Key {}", provider.api_key)),
        _ => ("authorization", format!("Bearer {}", provider.api_key)),
    }
}

async fn send_json(
    state: &SharedState,
    method: reqwest::Method,
    url: &str,
    provider: &Provider,
    body: Option<Value>,
    timeout: Duration,
) -> Result<(reqwest::StatusCode, Value), String> {
    let (header, value) = auth_header(provider);
    let mut request = state
        .http
        .request(method, url.to_string())
        .header(header, value)
        .timeout(timeout);
    if let Some(body) = body {
        request = request.header("content-type", "application/json").json(&body);
    }
    let response = request.send().await.map_err(|e| e.to_string())?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("read upstream body: {e}"))?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err("upstream response body too large".into());
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|e| format!("invalid upstream JSON: {e}"))?;
    Ok((status, value))
}

fn failure_detail(status: reqwest::StatusCode, body: &Value) -> String {
    let detail = body
        .pointer("/error/message")
        .and_then(Value::as_str)
        .or_else(|| body.get("detail").and_then(Value::as_str))
        .unwrap_or("");
    format!("upstream {}: {}", status.as_u16(), detail)
}

// ---------------------------------------------------------------------------
// LLM (script / storyboard)
// ---------------------------------------------------------------------------

pub struct LlmOutput {
    pub text: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

pub async fn llm_complete(
    state: &SharedState,
    provider: &Provider,
    system: &str,
    prompt: &str,
    timeout: Duration,
) -> Result<LlmOutput, String> {
    let mut messages = Vec::new();
    if !system.trim().is_empty() {
        messages.push(json!({ "role": "system", "content": system }));
    }
    messages.push(json!({ "role": "user", "content": prompt }));
    let (status, value) = send_json(
        state,
        reqwest::Method::POST,
        &format!("{}/v1/chat/completions", provider.base_url),
        provider,
        Some(json!({ "model": provider.model, "messages": messages })),
        timeout,
    )
    .await?;
    if !status.is_success() {
        return Err(failure_detail(status, &value));
    }
    let text = value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if text.trim().is_empty() {
        return Err("upstream returned an empty completion".into());
    }
    Ok(LlmOutput {
        text,
        input_tokens: value.pointer("/usage/prompt_tokens").and_then(Value::as_u64).unwrap_or(0),
        output_tokens: value
            .pointer("/usage/completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    })
}

// ---------------------------------------------------------------------------
// Image (OpenAI Images shape)
// ---------------------------------------------------------------------------

pub async fn image_generate(
    state: &SharedState,
    provider: &Provider,
    prompt: &str,
    size: &str,
    timeout: Duration,
) -> Result<(Vec<u8>, String, Option<String>), String> {
    let mut body = json!({
        "model": provider.model,
        "prompt": prompt,
        "n": 1,
        "response_format": "b64_json",
    });
    if !size.trim().is_empty() {
        body["size"] = json!(size);
    }
    let (status, value) = send_json(
        state,
        reqwest::Method::POST,
        &format!("{}/v1/images/generations", provider.base_url),
        provider,
        Some(body),
        timeout,
    )
    .await?;
    if !status.is_success() {
        return Err(failure_detail(status, &value));
    }
    let first = value.pointer("/data/0").cloned().unwrap_or(Value::Null);
    if first.is_null() {
        return Err("image response carried no data".into());
    }
    let revised = first
        .get("revised_prompt")
        .and_then(Value::as_str)
        .map(str::to_string);
    if let Some(b64) = first.get("b64_json").and_then(Value::as_str) {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| format!("decode image b64: {e}"))?;
        if bytes.is_empty() {
            return Err("image b64 payload empty".into());
        }
        return Ok((bytes, "image/png".to_string(), revised));
    }
    if let Some(url) = first.get("url").and_then(Value::as_str) {
        let response = state
            .http
            .get(url.to_string())
            .timeout(Duration::from_secs(180))
            .send()
            .await
            .map_err(|e| format!("fetch image url: {e}"))?;
        if !response.status().is_success() {
            return Err(format!("fetch image url: {}", response.status().as_u16()));
        }
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("image/png")
            .split(';')
            .next()
            .unwrap_or("image/png")
            .to_string();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| format!("read image bytes: {e}"))?
            .to_vec();
        if bytes.is_empty() {
            return Err("image url payload empty".into());
        }
        return Ok((bytes, mime, revised));
    }
    Err("image response carried neither b64_json nor url".into())
}

// ---------------------------------------------------------------------------
// Video submit / poll / fetch
// ---------------------------------------------------------------------------

pub async fn video_submit(
    state: &SharedState,
    provider: &Provider,
    prompt: &str,
    seconds: Option<&str>,
    size: Option<&str>,
    image_data_url: Option<&str>,
) -> Result<Value, String> {
    let submit_timeout = Duration::from_secs(30);
    match provider.upstream_kind.as_str() {
        "openai_video" => {
            let mut body = json!({ "model": provider.model, "prompt": prompt });
            if let Some(seconds) = seconds {
                body["seconds"] = json!(seconds);
            }
            if let Some(size) = size {
                body["size"] = json!(size);
            }
            if let Some(image) = image_data_url {
                body["input_reference"] = json!(image);
            }
            let (status, value) = send_json(
                state,
                reqwest::Method::POST,
                &format!("{}/videos", provider.base_url),
                provider,
                Some(body),
                submit_timeout,
            )
            .await?;
            if !status.is_success() {
                return Err(failure_detail(status, &value));
            }
            let id = value
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .ok_or("upstream job id missing")?;
            Ok(json!({
                "kind": "openai_video",
                "provider_id": provider.id,
                "base_url": provider.base_url,
                "job_id": id,
            }))
        }
        "fal_video" => {
            let encoded = utf8_percent_encode(&provider.model, NON_ALPHANUMERIC).to_string();
            let mut body = json!({ "prompt": prompt });
            if let Some(image) = image_data_url {
                body["image_url"] = json!(image);
            }
            if let Some(seconds) = seconds {
                body["duration"] = json!(seconds);
            }
            if let Some(size) = size {
                body["resolution"] = json!(size);
            }
            let (status, value) = send_json(
                state,
                reqwest::Method::POST,
                &format!("{}/{}", provider.base_url, encoded),
                provider,
                Some(body),
                submit_timeout,
            )
            .await?;
            if !status.is_success() {
                return Err(failure_detail(status, &value));
            }
            let id = value
                .get("request_id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .ok_or("fal request_id missing")?;
            Ok(json!({
                "kind": "fal_video",
                "provider_id": provider.id,
                "base_url": provider.base_url,
                "job_id": id,
                "status_url": value.get("status_url").and_then(Value::as_str),
                "response_url": value.get("response_url").and_then(Value::as_str),
            }))
        }
        "replicate" => {
            let url = if let Some((owner, rest)) = provider.model.split_once('/') {
                if rest.contains(':') || provider.model.matches('/').count() > 1 {
                    format!("{}/v1/predictions", provider.base_url)
                } else {
                    let owner = utf8_percent_encode(owner, NON_ALPHANUMERIC).to_string();
                    let name = utf8_percent_encode(rest, NON_ALPHANUMERIC).to_string();
                    format!("{}/v1/models/{owner}/{name}/predictions", provider.base_url)
                }
            } else {
                format!("{}/v1/predictions", provider.base_url)
            };
            let mut input = json!({ "prompt": prompt });
            if let Some(image) = image_data_url {
                input["image"] = json!(image);
            }
            if let Some(seconds) = seconds {
                input["duration"] = json!(seconds);
            }
            let (status, value) = send_json(
                state,
                reqwest::Method::POST,
                &url,
                provider,
                Some(json!({ "input": input })),
                submit_timeout,
            )
            .await?;
            if !status.is_success() {
                return Err(failure_detail(status, &value));
            }
            let id = value
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .ok_or("replicate prediction id missing")?;
            Ok(json!({
                "kind": "replicate",
                "provider_id": provider.id,
                "base_url": provider.base_url,
                "job_id": id,
            }))
        }
        other => Err(format!("provider upstream_kind {other} is not a video executor")),
    }
}

pub enum PollOutcome {
    Running { progress: Option<u8> },
    Succeeded { media_url: Option<String> },
    Failed(String),
}

pub async fn video_poll(
    state: &SharedState,
    provider: &Provider,
    remote_ref: &Value,
) -> Result<PollOutcome, String> {
    let poll_timeout = Duration::from_secs(30);
    let job_id = remote_ref
        .get("job_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let base = remote_ref
        .get("base_url")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .unwrap_or(&provider.base_url)
        .to_string();
    let ref_provider = Provider {
        base_url: base.clone(),
        ..provider.clone()
    };
    match remote_ref.get("kind").and_then(Value::as_str) {
        Some("openai_video") => {
            let (status, value) = send_json(
                state,
                reqwest::Method::GET,
                &format!("{base}/videos/{job_id}"),
                &ref_provider,
                None,
                poll_timeout,
            )
            .await?;
            if !status.is_success() {
                return Err(format!("poll status {}", status.as_u16()));
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
                "completed" | "succeeded" | "success" => {
                    Ok(PollOutcome::Succeeded { media_url: None })
                }
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
                        format!("{base}/{path}")
                    }
                })
                .unwrap_or_else(|| format!("{base}/{job_id}/status"));
            let (status, value) = send_json(
                state,
                reqwest::Method::GET,
                &status_url,
                &ref_provider,
                None,
                poll_timeout,
            )
            .await?;
            if !status.is_success() {
                return Err(format!("poll status {}", status.as_u16()));
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
                                format!("{base}/{path}")
                            }
                        })
                        .unwrap_or_else(|| format!("{base}/{job_id}"));
                    let (status, value) = send_json(
                        state,
                        reqwest::Method::GET,
                        &response_url,
                        &ref_provider,
                        None,
                        poll_timeout,
                    )
                    .await?;
                    if !status.is_success() {
                        return Err(format!("result status {}", status.as_u16()));
                    }
                    match extract_fal_video_url(&value) {
                        Some(url) => {
                            Ok(PollOutcome::Succeeded { media_url: Some(url) })
                        }
                        None => Err("fal result carried no video url".into()),
                    }
                }
                Some("IN_PROGRESS") | Some("QUEUED") => Ok(PollOutcome::Running {
                    progress: value
                        .get("queue_position")
                        .and_then(Value::as_u64)
                        .map(|p| (100u64.saturating_sub(p.min(99))) as u8),
                }),
                _ => Ok(PollOutcome::Failed("fal job failed".into())),
            }
        }
        Some("replicate") => {
            let (status, value) = send_json(
                state,
                reqwest::Method::GET,
                &format!("{base}/v1/predictions/{job_id}"),
                &ref_provider,
                None,
                poll_timeout,
            )
            .await?;
            if !status.is_success() {
                return Err(format!("poll status {}", status.as_u16()));
            }
            match value.get("status").and_then(Value::as_str) {
                Some("succeeded") => match extract_replicate_media(value.get("output")) {
                    Some(url) => Ok(PollOutcome::Succeeded { media_url: Some(url) }),
                    None => Err("replicate output carried no media".into()),
                },
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
        _ => Err("unknown remote ref kind".into()),
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

/// Open the media stream for a succeeded video job; credentials stay off
/// third-party CDN hosts.
pub async fn open_media_stream(
    state: &SharedState,
    provider: &Provider,
    remote_ref: &Value,
    media_url: Option<&str>,
) -> Result<reqwest::Response, String> {
    let cap = Duration::from_secs(300);
    let base = remote_ref
        .get("base_url")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .unwrap_or(&provider.base_url)
        .to_string();
    let ref_provider = Provider {
        base_url: base.clone(),
        ..provider.clone()
    };
    if let Some(media_url) = media_url.filter(|url| url.starts_with("http")) {
        let third_party = !media_url.contains(&base);
        let (header, value) = auth_header(&ref_provider);
        let mut request = state.http.get(media_url.to_string()).timeout(cap);
        if !third_party {
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
        Some("openai_video") => format!("{base}/videos/{job_id}/content"),
        Some("replicate") => format!("{base}/v1/predictions/{job_id}/content"),
        _ => return Err("no media url for this asset".into()),
    };
    let (header, value) = auth_header(&ref_provider);
    state
        .http
        .get(url)
        .header(header, value)
        .timeout(cap)
        .send()
        .await
        .map_err(|e| format!("media fetch failed: {e}"))
}
