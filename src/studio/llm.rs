//! ST-A1 / MB-ST2: internal LLM steps — a direct chat-completions call through
//! the existing channel registry, charged by token usage at the model's rates.

use crate::monoize_routing::MonoizeProviderType;
use crate::studio::StudioState;
use crate::studio::upstream::{ChannelChoice, UpstreamError};
use serde_json::{Value, json};
use std::time::Duration;

pub struct LlmResult {
    pub content: String,
    pub tool_calls: Vec<Value>,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

pub async fn chat_candidates(state: &StudioState, model: &str) -> Vec<ChannelChoice> {
    let providers = state
        .monoize_store
        .list_providers()
        .await
        .unwrap_or_default();
    crate::studio::upstream::select_channels(
        &state.monoize_store,
        &providers,
        MonoizeProviderType::ChatCompletion,
        model,
        None,
    )
}

pub async fn complete(
    state: &StudioState,
    model: &str,
    messages: &[Value],
    tools: Option<&Value>,
) -> Result<(LlmResult, ChannelChoice), UpstreamError> {
    let candidates = chat_candidates(state, model).await;
    let mut last_error = UpstreamError::SubmitRejected("no eligible chat channel".into());
    for choice in &candidates {
        match complete_on(state, choice, model, messages, tools).await {
            Ok(result) => return Ok((result, choice.clone())),
            Err(error) if error.is_retryable() => {
                last_error = error;
                continue;
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_error)
}

async fn complete_on(
    state: &StudioState,
    choice: &ChannelChoice,
    _model: &str,
    messages: &[Value],
    tools: Option<&Value>,
) -> Result<LlmResult, UpstreamError> {
    let client = state
        .http_clients
        .for_channel_proxy(choice.proxy_url.as_deref())
        .map_err(UpstreamError::Transport)?;
    let mut body = json!({"model": choice.upstream_model, "messages": messages, "stream": false});
    if let Some(tools) = tools {
        body["tools"] = tools.clone();
        body["tool_choice"] = json!("auto");
    }
    let resp = client
        .post(format!("{}/v1/chat/completions", choice.base_url))
        .header("authorization", format!("Bearer {}", choice.api_key))
        .header("content-type", "application/json")
        .timeout(Duration::from_millis(crate::studio::llm_timeout_ms()))
        .json(&body)
        .send()
        .await
        .map_err(|e| UpstreamError::Transport(e.to_string()))?;
    let status = resp.status();
    let bytes = crate::bounded_response::read_response_body_with_limit(resp, 16 * 1024 * 1024)
        .await
        .map_err(|e| UpstreamError::Transport(e.to_string()))?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|e| UpstreamError::Transport(format!("invalid chat JSON: {e}")))?;
    if !status.is_success() {
        let detail = value
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("");
        return Err(UpstreamError::SubmitRejected(format!(
            "upstream {}: {}",
            status.as_u16(),
            detail
        )));
    }
    let message = value
        .pointer("/choices/0/message")
        .cloned()
        .unwrap_or(Value::Null);
    let tool_calls = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(LlmResult {
        content: message
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        tool_calls,
        input_tokens: value
            .pointer("/usage/prompt_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        output_tokens: value
            .pointer("/usage/completion_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0),
    })
}

/// MB-ST2: token charge in nano-USD from the model's rates. Rates are matched
/// through the channel's pricing profile; a model without rates bills 0 and
/// the usage still lands in the step result for auditing.
pub async fn token_charge_nano(
    state: &StudioState,
    model: &str,
    pricing_profile: Option<&str>,
    input_tokens: i64,
    output_tokens: i64,
) -> i128 {
    if input_tokens <= 0 && output_tokens <= 0 {
        return 0;
    }
    let profile = pricing_profile.unwrap_or("");
    let rates = state
        .billing_rate_store
        .list_matching_rates(profile, Some("chat_completion"), model)
        .await
        .unwrap_or_default();
    let rate_for = |usage_class: &str| -> Option<i128> {
        rates
            .iter()
            .filter(|rate| rate.rate_kind == "token")
            .filter(|rate| {
                rate.usage_class == usage_class
                    || rate.usage_class == "output" && usage_class == "output"
            })
            .filter_map(|rate| rate.unit_price_nano.parse::<i128>().ok())
            .max()
    };
    let input_rate = rate_for("input_uncached").or_else(|| rate_for("input"));
    let output_rate = rate_for("output");
    let input = input_rate.unwrap_or(0) * input_tokens.max(0) as i128;
    let output = output_rate.unwrap_or(0) * output_tokens.max(0) as i128;
    input.saturating_add(output)
}
