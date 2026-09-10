//! Legacy `POST /v1/completions` (`spec/unified_responses_proxy.spec.md` §2.2.2).
//!
//! The endpoint is a translation layer, not a forwarding path. A legacy request becomes a
//! chat completion, runs through [`super::create_chat_completions`] unchanged, and the
//! response is rewritten back into the legacy shape. Everything that matters — routing,
//! model redirects, allowlists, billing, request logging, capture — therefore behaves
//! identically to the chat endpoint, because it *is* the chat endpoint.
//!
//! Translating at the HTTP boundary rather than adding a `DownstreamProtocol` variant keeps
//! sixty match sites untouched. The cost is re-encoding the SSE stream, which is acceptable
//! for an endpoint that exists for compatibility.

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::StreamExt;
use serde_json::{Map, Value, json};

use crate::app::AppState;
use crate::error::{AppError, AppResult};

/// Fields a chat completion cannot express (LC4).
const UNSUPPORTED_FIELDS: [&str; 4] = ["suffix", "echo", "best_of", "logprobs"];

pub async fn create_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<Response> {
    let chat_body = to_chat_request(body)?;
    let streaming = chat_body
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let response = super::create_chat_completions(State(state), headers, Json(chat_body)).await?;
    Ok(if streaming {
        rewrite_stream(response)
    } else {
        rewrite_body(response).await?
    })
}

/// Translates a legacy request body into a chat completion body (LC2, LC3, LC4).
fn to_chat_request(body: Value) -> AppResult<Value> {
    let Value::Object(mut object) = body else {
        return Err(invalid("request body must be a JSON object"));
    };

    for field in UNSUPPORTED_FIELDS {
        if object.get(field).is_some_and(|value| !value.is_null()) {
            return Err(invalid(&format!(
                "`{field}` is not supported on /v1/completions; use /v1/chat/completions"
            )));
        }
    }

    let prompt = object
        .remove("prompt")
        .ok_or_else(|| invalid("`prompt` is required"))?;
    let prompt = match prompt {
        Value::String(text) => text,
        // A single-element array is the common client encoding of one prompt. Longer arrays
        // are a batch, and integer arrays are pre-tokenized input; neither survives the
        // translation to one chat completion, so both are refused rather than truncated.
        Value::Array(items) if items.len() == 1 => match items.into_iter().next() {
            Some(Value::String(text)) => text,
            _ => {
                return Err(invalid(
                    "`prompt` must be a string; token-array prompts are not supported",
                ));
            }
        },
        Value::Array(_) => {
            return Err(invalid(
                "`prompt` must be a single string; batched prompts are not supported",
            ));
        }
        _ => return Err(invalid("`prompt` must be a string")),
    };

    object.insert(
        "messages".to_string(),
        json!([{ "role": "user", "content": prompt }]),
    );
    Ok(Value::Object(object))
}

/// Rewrites one non-streaming chat completion into the legacy shape (LC5).
async fn rewrite_body(response: Response) -> AppResult<Response> {
    let (parts, body) = response.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .map_err(|error| {
            AppError::new(
                StatusCode::BAD_GATEWAY,
                "upstream_error",
                format!("could not read upstream response: {error}"),
            )
        })?;

    // An error response is not a completion, so it passes through untouched.
    if !parts.status.is_success() {
        return Ok(Response::from_parts(parts, Body::from(bytes)));
    }

    let Ok(mut value) = serde_json::from_slice::<Value>(&bytes) else {
        return Ok(Response::from_parts(parts, Body::from(bytes)));
    };
    rewrite_completion(&mut value, choice_from_message);
    let rewritten = serde_json::to_vec(&value).map_err(|error| {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            format!("could not encode completion: {error}"),
        )
    })?;
    let mut parts = parts;
    parts.headers.remove(axum::http::header::CONTENT_LENGTH);
    Ok(Response::from_parts(parts, Body::from(rewritten)))
}

/// Rewrites each streamed chunk into the legacy shape (LC6, LC7).
fn rewrite_stream(response: Response) -> Response {
    let (parts, body) = response.into_parts();
    let stream = body.into_data_stream().map(|chunk| {
        chunk.map(|bytes| {
            let Ok(text) = std::str::from_utf8(&bytes) else {
                return bytes;
            };
            axum::body::Bytes::from(rewrite_sse_chunk(text))
        })
    });
    let mut parts = parts;
    parts.headers.remove(axum::http::header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from_stream(stream))
}

/// Rewrites the `data:` payloads in one SSE chunk, leaving framing intact.
///
/// Operates line by line because a single chunk may carry several frames, and event or
/// comment lines must survive unchanged for the framing to stay valid.
fn rewrite_sse_chunk(chunk: &str) -> String {
    let mut out = String::with_capacity(chunk.len());
    for line in chunk.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        let Some(payload) = trimmed.strip_prefix("data:") else {
            out.push_str(line);
            continue;
        };
        let payload = payload.trim_start();
        // LC7: `[DONE]` and any frame that is not a completion object pass through, which
        // includes upstream error frames.
        if payload == "[DONE]" || payload.is_empty() {
            out.push_str(line);
            continue;
        }
        let Ok(mut value) = serde_json::from_str::<Value>(payload) else {
            out.push_str(line);
            continue;
        };
        if value.get("error").is_some() {
            out.push_str(line);
            continue;
        }
        rewrite_completion(&mut value, choice_from_delta);
        let suffix = &line[trimmed.len()..];
        out.push_str("data: ");
        out.push_str(&value.to_string());
        out.push_str(suffix);
    }
    out
}

/// Applies `choice` to every element of `choices` and relabels `object` (LC5, LC6).
fn rewrite_completion(value: &mut Value, choice: fn(&mut Map<String, Value>)) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.insert(
        "object".to_string(),
        Value::String("text_completion".to_string()),
    );
    let Some(choices) = object.get_mut("choices").and_then(Value::as_array_mut) else {
        return;
    };
    for item in choices {
        if let Some(item) = item.as_object_mut() {
            choice(item);
        }
    }
}

fn choice_from_message(choice: &mut Map<String, Value>) {
    let text = choice
        .remove("message")
        .and_then(|message| message.get("content").cloned())
        .and_then(|content| content.as_str().map(str::to_string))
        .unwrap_or_default();
    finish_choice(choice, text);
}

fn choice_from_delta(choice: &mut Map<String, Value>) {
    // LC6: a chunk with no content delta becomes empty text rather than being dropped, so
    // the frame sequence a caller observes matches what the upstream produced.
    let text = choice
        .remove("delta")
        .and_then(|delta| delta.get("content").cloned())
        .and_then(|content| content.as_str().map(str::to_string))
        .unwrap_or_default();
    finish_choice(choice, text);
}

fn finish_choice(choice: &mut Map<String, Value>, text: String) {
    choice.insert("text".to_string(), Value::String(text));
    choice.entry("logprobs").or_insert(Value::Null);
    choice.entry("index").or_insert(json!(0));
}

fn invalid(message: &str) -> AppError {
    AppError::new(StatusCode::BAD_REQUEST, "invalid_request_error", message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_string_prompt_becomes_one_user_message() {
        let chat = to_chat_request(json!({
            "model": "m",
            "prompt": "Hello",
            "max_tokens": 64,
            "temperature": 0.7
        }))
        .expect("translates");
        assert_eq!(chat["messages"], json!([{ "role": "user", "content": "Hello" }]));
        assert!(chat.get("prompt").is_none(), "prompt must not be forwarded");
        // LC3: every other field is forwarded untouched.
        assert_eq!(chat["model"], json!("m"));
        assert_eq!(chat["max_tokens"], json!(64));
        assert_eq!(chat["temperature"], json!(0.7));
    }

    #[test]
    fn a_single_element_array_prompt_is_accepted() {
        let chat = to_chat_request(json!({ "model": "m", "prompt": ["Hi"] })).expect("translates");
        assert_eq!(chat["messages"][0]["content"], json!("Hi"));
    }

    /// LC2: a batch or a token array cannot be one chat completion. Refusing beats returning
    /// output for a prompt the caller did not send.
    #[test]
    fn batched_and_tokenized_prompts_are_refused() {
        for prompt in [json!(["a", "b"]), json!([[1, 2]]), json!([1, 2, 3]), json!(7)] {
            let error = to_chat_request(json!({ "model": "m", "prompt": prompt }))
                .expect_err("must be refused");
            assert_eq!(error.status, StatusCode::BAD_REQUEST);
        }
        let missing = to_chat_request(json!({ "model": "m" })).expect_err("prompt is required");
        assert_eq!(missing.status, StatusCode::BAD_REQUEST);
    }

    /// LC4: silently ignoring these would answer a different question than the one asked.
    #[test]
    fn unsupported_fields_are_refused_by_name() {
        for field in UNSUPPORTED_FIELDS {
            let error = to_chat_request(json!({ "model": "m", "prompt": "x", field: 1 }))
                .expect_err("must be refused");
            assert_eq!(error.status, StatusCode::BAD_REQUEST);
            assert!(error.message.contains(field), "{}", error.message);
        }
        // A null is the same as absent, so it must not trip the check.
        to_chat_request(json!({ "model": "m", "prompt": "x", "suffix": null }))
            .expect("an explicit null is not a value");
    }

    #[test]
    fn a_completion_response_is_rewritten_to_text() {
        let mut value = json!({
            "id": "cmpl-1",
            "object": "chat.completion",
            "created": 1,
            "model": "m",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "Hello there" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5 }
        });
        rewrite_completion(&mut value, choice_from_message);

        assert_eq!(value["object"], json!("text_completion"));
        assert_eq!(value["choices"][0]["text"], json!("Hello there"));
        assert_eq!(value["choices"][0]["logprobs"], Value::Null);
        assert_eq!(value["choices"][0]["finish_reason"], json!("stop"));
        assert!(value["choices"][0].get("message").is_none());
        // LC5: identity and usage are the upstream's and must not be touched.
        assert_eq!(value["id"], json!("cmpl-1"));
        assert_eq!(value["created"], json!(1));
        assert_eq!(value["usage"]["total_tokens"], json!(5));
    }

    #[test]
    fn stream_chunks_are_rewritten_and_framing_is_preserved() {
        let chunk = concat!(
            "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",",
            "\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hi\"},\"finish_reason\":null}]}\n\n",
        );
        let out = rewrite_sse_chunk(chunk);
        assert!(out.starts_with("data: "), "{out}");
        assert!(out.ends_with("\n\n"), "the blank line must survive: {out:?}");
        let payload: Value =
            serde_json::from_str(out.trim_start_matches("data: ").trim()).expect("JSON");
        assert_eq!(payload["object"], json!("text_completion"));
        assert_eq!(payload["choices"][0]["text"], json!("Hi"));
        assert!(payload["choices"][0].get("delta").is_none());
    }

    /// LC6: a role-only opening chunk carries no content, and dropping it would change the
    /// frame sequence the caller sees.
    #[test]
    fn a_chunk_without_content_becomes_empty_text() {
        let chunk = "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"}}]}\n\n";
        let out = rewrite_sse_chunk(chunk);
        let payload: Value =
            serde_json::from_str(out.trim_start_matches("data: ").trim()).expect("JSON");
        assert_eq!(payload["choices"][0]["text"], json!(""));
    }

    /// LC7: an error frame must reach the caller verbatim, not as an empty completion.
    #[test]
    fn terminal_and_error_frames_pass_through_unchanged() {
        assert_eq!(rewrite_sse_chunk("data: [DONE]\n\n"), "data: [DONE]\n\n");

        let error = "data: {\"error\":{\"message\":\"upstream failed\",\"code\":\"bad\"}}\n\n";
        assert_eq!(rewrite_sse_chunk(error), error);

        // Comment and event lines are framing, not payload.
        assert_eq!(rewrite_sse_chunk(": ping\n\n"), ": ping\n\n");
        assert_eq!(rewrite_sse_chunk("event: message\n"), "event: message\n");
    }

    #[test]
    fn several_frames_in_one_chunk_are_each_rewritten() {
        let chunk = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"a\"}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"b\"}}]}\n\n",
            "data: [DONE]\n\n",
        );
        let out = rewrite_sse_chunk(chunk);
        assert_eq!(out.matches("\"text\":\"a\"").count(), 1, "{out}");
        assert_eq!(out.matches("\"text\":\"b\"").count(), 1, "{out}");
        assert!(out.ends_with("data: [DONE]\n\n"), "{out}");
    }
}
