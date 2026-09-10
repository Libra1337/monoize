use super::*;
use base64::Engine as _;

fn last_captured_body(ctx: &TestContext, endpoint: &str) -> Value {
    ctx.captured_bodies
        .lock()
        .expect("captured bodies lock")
        .iter()
        .rev()
        .find(|(name, _)| name == endpoint)
        .map(|(_, body)| body.clone())
        .unwrap_or_else(|| panic!("missing captured upstream body for {endpoint}"))
}

#[tokio::test]
async fn direct_request_matrix_captures_all_openai_anthropic_routes_nonstream_and_stream() {
    let ctx = setup().await;
    let downstreams = ["responses", "chat", "messages"];
    let upstreams = [
        ("responses", "gpt-5-mini"),
        ("chat", "gpt-5-mini-chat"),
        ("messages", "gpt-5-mini-msg"),
    ];

    for stream in [false, true] {
        for downstream in downstreams {
            for (upstream, model) in upstreams {
                let marker = format!("grid-{downstream}-{upstream}-{stream}");
                let (path, body) = match downstream {
                    "responses" => (
                        "/v1/responses",
                        json!({
                            "model": model,
                            "input": marker.clone(),
                            "max_output_tokens": 64,
                            "stream": stream
                        }),
                    ),
                    "chat" => (
                        "/v1/chat/completions",
                        json!({
                            "model": model,
                            "messages": [{ "role": "user", "content": marker.clone() }],
                            "max_completion_tokens": 64,
                            "stream": stream
                        }),
                    ),
                    "messages" => (
                        "/v1/messages",
                        json!({
                            "model": model,
                            "max_tokens": 64,
                            "messages": [{ "role": "user", "content": marker.clone() }],
                            "stream": stream
                        }),
                    ),
                    _ => unreachable!(),
                };

                let (status, response) = json_post(&ctx, path, body).await;
                assert_eq!(
                    status,
                    StatusCode::OK,
                    "{downstream}->{upstream} stream={stream}: {response}"
                );
                if stream {
                    match downstream {
                        "responses" => {
                            assert!(response.contains("event: response.completed"), "{response}");
                            assert!(response.contains("data: [DONE]"), "{response}");
                        }
                        "chat" => {
                            assert!(response.contains("data: [DONE]"), "{response}");
                            assert!(!response.contains("event:"), "{response}");
                        }
                        "messages" => {
                            assert!(response.contains("event: message_start"), "{response}");
                            assert!(response.contains("event: message_stop"), "{response}");
                            assert!(!response.contains("data: [DONE]"), "{response}");
                        }
                        _ => unreachable!(),
                    }
                } else {
                    let decoded: Value = serde_json::from_str(&response).unwrap();
                    match downstream {
                        "responses" => {
                            assert!(decoded.get("output").and_then(Value::as_array).is_some());
                        }
                        "chat" => {
                            assert!(decoded.get("choices").and_then(Value::as_array).is_some());
                        }
                        "messages" => {
                            assert_eq!(decoded["type"], json!("message"));
                            assert!(decoded.get("content").and_then(Value::as_array).is_some());
                        }
                        _ => unreachable!(),
                    }
                }
                let captured = last_captured_body(&ctx, upstream);
                assert_eq!(captured["stream"], json!(stream));
                assert!(
                    serde_json::to_string(&captured).unwrap().contains(&marker),
                    "{downstream}->{upstream} lost semantic input: {captured}"
                );
                match upstream {
                    "responses" => {
                        assert!(captured.get("input").is_some(), "{captured}");
                        assert!(captured.get("messages").is_none(), "{captured}");
                    }
                    "chat" => {
                        assert!(captured.get("messages").is_some(), "{captured}");
                        assert!(captured.get("input").is_none(), "{captured}");
                    }
                    "messages" => {
                        assert!(captured.get("messages").is_some(), "{captured}");
                        assert!(captured.get("input").is_none(), "{captured}");
                        assert_eq!(captured["max_tokens"], json!(64));
                    }
                    _ => unreachable!(),
                }
            }
        }
    }
}

mod responses_reasoning {
    use super::*;
    include!("adapters_nonstream/responses_reasoning.rs");
}

mod images_and_chat {
    use super::*;
    include!("adapters_nonstream/images_and_chat.rs");
}

mod messages_basic {
    use super::*;
    include!("adapters_nonstream/messages_basic.rs");
}

mod tools_envelope {
    use super::*;
    include!("adapters_nonstream/tools_envelope.rs");
}

mod reasoning_tools {
    use super::*;
    include!("adapters_nonstream/reasoning_tools.rs");
}

mod messages_native {
    use super::*;
    include!("adapters_nonstream/messages_native.rs");
}

mod native_responses {
    use super::*;
    include!("adapters_nonstream/native_responses.rs");
}

mod messages_reasoning {
    use super::*;
    include!("adapters_nonstream/messages_reasoning.rs");
}

mod request_controls {
    use super::*;
    include!("adapters_nonstream/request_controls.rs");
}

mod controls_tools_matrix {
    use super::*;
    include!("adapters_nonstream/controls_tools_matrix.rs");
}

#[tokio::test]
async fn chat_nonstream_usage_preserves_nested_unknown_details() {
    let ctx = setup().await;
    let (status, body) = json_post(
        &ctx,
        "/v1/chat/completions",
        json!({
            "model": "gpt-5-mini-chat",
            "messages": [{ "role": "user", "content": "nested chat usage" }],
            "stream_mode": "nested_usage_details"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {body}");

    let response: Value = serde_json::from_str(&body).expect("chat response JSON");
    assert_eq!(
        response["usage"]["prompt_tokens_details"],
        json!({
            "cached_tokens": 0,
            "cache_write_tokens": 0,
            "cache_creation_tokens": 0,
            "tool_prompt_tokens": 0,
            "vendor_prompt_detail": { "kind": "warm" }
        })
    );
    assert_eq!(
        response["usage"]["completion_tokens_details"],
        json!({
            "reasoning_tokens": 0,
            "accepted_prediction_tokens": 0,
            "rejected_prediction_tokens": 0,
            "vendor_completion_detail": [1, 2]
        })
    );
    assert!(response["usage"].get("vendor_prompt_detail").is_none());
    assert!(response["usage"].get("vendor_completion_detail").is_none());
}

#[tokio::test]
async fn responses_nonstream_usage_preserves_nested_unknown_details() {
    let ctx = setup().await;
    let (status, body) = json_post(
        &ctx,
        "/v1/responses",
        json!({
            "model": "gpt-5-mini",
            "input": "nested Responses usage",
            "stream_mode": "nested_usage_details"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {body}");

    let response: Value = serde_json::from_str(&body).expect("Responses response JSON");
    assert_eq!(
        response["usage"]["input_tokens_details"],
        json!({
            "cached_tokens": 0,
            "cache_write_tokens": 0,
            "cache_creation_tokens": 0,
            "tool_prompt_tokens": 0,
            "vendor_input_detail": { "kind": "warm" }
        })
    );
    assert_eq!(
        response["usage"]["output_tokens_details"],
        json!({
            "reasoning_tokens": 0,
            "accepted_prediction_tokens": 0,
            "rejected_prediction_tokens": 0,
            "vendor_output_detail": [3, 4]
        })
    );
    assert!(response["usage"].get("vendor_input_detail").is_none());
    assert!(response["usage"].get("vendor_output_detail").is_none());
}

#[tokio::test]
async fn channel_extra_headers_are_sent_to_upstream() {
    let ctx = setup().await;

    // Build a dedicated provider whose channel carries a static affinity header.
    let models = HashMap::from([(
        "cf-affinity-model".to_string(),
        monoize::monoize_routing::MonoizeModelEntry {
            redirect: None,
            pricing_profile_mode: Default::default(),
            pricing_profile_override: None,
            multiplier_override: Some(monoize::exact_decimal::Multiplier::ONE),
        },
    )]);
    let upstream_addr = {
        // The mock upstream address is embedded in existing channels' base_url.
        let providers = ctx.state.monoize_store.list_providers().await.unwrap();
        providers
            .iter()
            .map(|provider| provider.channel.base_url.clone())
            .next()
            .expect("at least one channel")
    };
    ctx.state
        .monoize_store
        .create_provider(monoize::monoize_routing::CreateMonoizeProviderInput {
            confirm_public_exposure: true,
            pricing_profile: Some("default".to_string()),
            multiplier: Default::default(),
            name: "up-cf-affinity".to_string(),
            api_type_overrides: Vec::new(),
            group_id: String::new(),
            channel: monoize::monoize_routing::CreateMonoizeChannelInput {
                name: "cf-affinity-channel".to_string(),
                provider_type: monoize::monoize_routing::MonoizeProviderType::ChatCompletion,
                base_url: upstream_addr,
                api_key: Some("upstream-key".to_string()),
                enabled: true,
                allow_missing_usage: false,
                passive_failure_count_threshold_override: None,
                passive_cooldown_seconds_override: None,
                passive_window_seconds_override: None,
                passive_rate_limit_cooldown_seconds_override: None,
                models,
                active_probe_enabled_override: None,
                active_probe_interval_seconds_override: None,
                active_probe_success_threshold_override: None,
                active_probe_model_override: None,
                affinity_enabled_override: None,
                affinity_idle_ttl_seconds_override: None,
                affinity_failback_mode_override: None,
                affinity_failback_delay_seconds_override: None,
                proxy_url: None,
                extra_headers: Some(std::collections::BTreeMap::from([(
                    "x-session-affinity".to_string(),
                    "ses_e2e_001".to_string(),
                )])),
                session_affinity_auto: Some(true),
            },
            channel_max_retries: 0,
            channel_retry_interval_ms: 0,
            circuit_breaker_enabled: false,
            per_model_circuit_break: false,
            transforms: Vec::new(),
            active_probe_enabled_override: None,
            active_probe_interval_seconds_override: None,
            active_probe_success_threshold_override: None,
            active_probe_model_override: None,
            request_timeout_ms_override: None,
            extra_fields_whitelist: None,
            strip_cross_protocol_nested_extra: None,
            enabled: true,
            priority: None,
        })
        .await
        .expect("create provider");
    seed_test_model_pricing(&ctx.state, &["cf-affinity-model"]).await;

    let (status, body) = json_post(
        &ctx,
        "/v1/chat/completions",
        json!({
            "model": "cf-affinity-model",
            "messages": [{ "role": "user", "content": "affinity header check" }],
            "max_completion_tokens": 32
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let headers = ctx.captured_headers.lock().expect("captured headers lock");
    assert!(
        headers
            .iter()
            .any(|(name, value)| name == "x-session-affinity" && value == "ses_e2e_001"),
        "x-session-affinity must reach the upstream, got {headers:?}"
    );
}

#[tokio::test]
async fn auto_session_affinity_is_stable_per_conversation_and_distinct_across_sessions() {
    let ctx = setup().await;
    let upstream_addr = {
        let providers = ctx.state.monoize_store.list_providers().await.unwrap();
        providers
            .iter()
            .map(|provider| provider.channel.base_url.clone())
            .next()
            .expect("at least one channel")
    };
    ctx.state
        .monoize_store
        .create_provider(monoize::monoize_routing::CreateMonoizeProviderInput {
            confirm_public_exposure: true,
            pricing_profile: Some("default".to_string()),
            multiplier: Default::default(),
            name: "up-cf-auto-affinity".to_string(),
            api_type_overrides: Vec::new(),
            group_id: String::new(),
            channel: monoize::monoize_routing::CreateMonoizeChannelInput {
                name: "cf-auto-channel".to_string(),
                provider_type: monoize::monoize_routing::MonoizeProviderType::ChatCompletion,
                base_url: upstream_addr,
                api_key: Some("upstream-key".to_string()),
                enabled: true,
                allow_missing_usage: false,
                passive_failure_count_threshold_override: None,
                passive_cooldown_seconds_override: None,
                passive_window_seconds_override: None,
                passive_rate_limit_cooldown_seconds_override: None,
                models: HashMap::from([(
                    "cf-auto-model".to_string(),
                    monoize::monoize_routing::MonoizeModelEntry {
                        redirect: None,
                        pricing_profile_mode: Default::default(),
                        pricing_profile_override: None,
                        multiplier_override: Some(monoize::exact_decimal::Multiplier::ONE),
                    },
                )]),
                active_probe_enabled_override: None,
                active_probe_interval_seconds_override: None,
                active_probe_success_threshold_override: None,
                active_probe_model_override: None,
                affinity_enabled_override: None,
                affinity_idle_ttl_seconds_override: None,
                affinity_failback_mode_override: None,
                affinity_failback_delay_seconds_override: None,
                proxy_url: None,
                extra_headers: None,
                session_affinity_auto: Some(true),
            },
            channel_max_retries: 0,
            channel_retry_interval_ms: 0,
            circuit_breaker_enabled: false,
            per_model_circuit_break: false,
            transforms: Vec::new(),
            active_probe_enabled_override: None,
            active_probe_interval_seconds_override: None,
            active_probe_success_threshold_override: None,
            active_probe_model_override: None,
            request_timeout_ms_override: None,
            extra_fields_whitelist: None,
            strip_cross_protocol_nested_extra: None,
            enabled: true,
            priority: None,
        })
        .await
        .expect("create provider");
    seed_test_model_pricing(&ctx.state, &["cf-auto-model"]).await;

    let turn_one = serde_json::json!({
        "model": "cf-auto-model",
        "messages": [
            { "role": "system", "content": "shared system prompt" },
            { "role": "user", "content": "session A question" }
        ],
        "max_completion_tokens": 32
    });
    let mut turn_two = turn_one.clone();
    turn_two["messages"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({ "role": "assistant", "content": "partial answer" }));

    for body in [&turn_one, &turn_two] {
        let (status, resp) = json_post(&ctx, "/v1/chat/completions", body.clone()).await;
        assert_eq!(status, StatusCode::OK, "{resp}");
    }

    // Same conversation grown by one turn: identical affinity value twice.
    let affinities: Vec<String> = {
        let headers = ctx.captured_headers.lock().expect("captured headers lock");
        headers
            .iter()
            .filter(|(name, _)| name == "x-session-affinity")
            .map(|(_, value)| value.clone())
            .collect()
    };
    assert_eq!(affinities.len(), 2, "{affinities:?}");
    assert_eq!(affinities[0], affinities[1]);
    assert!(affinities[0].starts_with("mono-"), "{affinities:?}");

    // Distinct conversation head derives a distinct affinity.
    let other = serde_json::json!({
        "model": "cf-auto-model",
        "messages": [
            { "role": "system", "content": "shared system prompt" },
            { "role": "user", "content": "session B question" }
        ],
        "max_completion_tokens": 32
    });
    let (status, resp) = json_post(&ctx, "/v1/chat/completions", other).await;
    assert_eq!(status, StatusCode::OK, "{resp}");
    let third = {
        let headers = ctx.captured_headers.lock().expect("captured headers lock");
        headers
            .iter()
            .rev()
            .find(|(name, _)| name == "x-session-affinity")
            .map(|(_, value)| value.to_string())
            .unwrap()
    };
    assert_ne!(third, affinities[0]);

    ctx.captured_headers
        .lock()
        .expect("captured headers lock")
        .clear();
    let mut with_calc = turn_one.clone();
    with_calc["tools"] = json!([{
        "type": "function",
        "function": { "name": "calc", "parameters": { "type": "object" } }
    }]);
    let mut with_search = with_calc.clone();
    with_search["tools"] = json!([
        {
            "type": "function",
            "function": { "name": "calc", "parameters": { "type": "object" } }
        },
        {
            "type": "function",
            "function": { "name": "search", "parameters": { "type": "object" } }
        }
    ]);
    with_search["messages"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "role": "assistant", "content": "working" }));
    for body in [&with_calc, &with_search] {
        let (status, resp) = json_post(&ctx, "/v1/chat/completions", body.clone()).await;
        assert_eq!(status, StatusCode::OK, "{resp}");
    }
    let tool_affinities: Vec<String> = {
        let headers = ctx.captured_headers.lock().expect("captured headers lock");
        headers
            .iter()
            .filter(|(name, _)| name == "x-session-affinity")
            .map(|(_, value)| value.clone())
            .collect()
    };
    assert_eq!(tool_affinities.len(), 2, "{tool_affinities:?}");
    assert_eq!(tool_affinities[0], tool_affinities[1]);
    assert_eq!(tool_affinities[0], affinities[0]);

    ctx.captured_headers
        .lock()
        .expect("captured headers lock")
        .clear();
    let mut body_session = turn_one.clone();
    body_session["session_id"] = json!("019ffeb5-e6ed-7180-89b6-df6e938625a6");
    let mut body_session_later = body_session.clone();
    body_session_later["messages"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "role": "assistant", "content": "working" }));
    body_session_later["tools"] = json!([{
        "type": "function",
        "function": { "name": "other_tool", "parameters": { "type": "object" } }
    }]);
    for body in [&body_session, &body_session_later] {
        let (status, resp) = json_post(&ctx, "/v1/chat/completions", body.clone()).await;
        assert_eq!(status, StatusCode::OK, "{resp}");
    }
    let body_ids: Vec<String> = {
        let headers = ctx.captured_headers.lock().expect("captured headers lock");
        headers
            .iter()
            .filter(|(name, _)| name == "x-session-affinity")
            .map(|(_, value)| value.clone())
            .collect()
    };
    assert_eq!(
        body_ids,
        vec![
            "019ffeb5-e6ed-7180-89b6-df6e938625a6".to_string(),
            "019ffeb5-e6ed-7180-89b6-df6e938625a6".to_string()
        ]
    );
}

/// §2.2.2: the legacy endpoint must reach the same upstream as the chat endpoint and return
/// the legacy shape.
///
/// The prompt is asserted to arrive upstream as a chat message, which is what proves the
/// translation happened rather than the request taking some separate path.
#[tokio::test]
async fn legacy_completions_translates_to_chat_and_back() {
    let ctx = setup().await;

    let (status, response) = json_post(
        &ctx,
        "/v1/completions",
        json!({
            "model": "gpt-5-mini-chat",
            "prompt": "legacy-prompt-marker",
            "max_tokens": 64
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");

    let upstream = last_captured_body(&ctx, "chat");
    assert_eq!(
        upstream["messages"][0]["content"], json!("legacy-prompt-marker"),
        "the prompt must reach upstream as a chat message: {upstream}"
    );
    assert!(
        upstream.get("prompt").is_none(),
        "the legacy field must not be forwarded: {upstream}"
    );

    let body: Value = serde_json::from_str(&response).expect("JSON response");
    assert_eq!(body["object"], json!("text_completion"), "{body}");
    assert!(
        body["choices"][0]["text"].is_string(),
        "a legacy choice carries text, not a message: {body}"
    );
    assert!(body["choices"][0].get("message").is_none(), "{body}");
    assert!(body["usage"]["total_tokens"].is_number(), "{body}");
}

/// The streamed form must carry text deltas and terminate, so a legacy client that streams
/// sees the same frame sequence it would from OpenAI.
#[tokio::test]
async fn legacy_completions_streams_text_frames() {
    let ctx = setup().await;

    let (status, response) = json_post(
        &ctx,
        "/v1/completions",
        json!({
            "model": "gpt-5-mini-chat",
            "prompt": "legacy-stream-marker",
            "max_tokens": 64,
            "stream": true
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert!(response.contains("\"object\":\"text_completion\""), "{response}");
    assert!(response.contains("\"text\""), "{response}");
    assert!(!response.contains("\"delta\""), "no chat delta may leak: {response}");
    assert!(response.contains("data: [DONE]"), "{response}");
}

/// LC2 and LC4 must be refused before any upstream call, so an unsupported request costs
/// nothing and returns a reason.
#[tokio::test]
async fn legacy_completions_refuses_what_it_cannot_express() {
    let ctx = setup().await;

    for body in [
        json!({ "model": "gpt-5-mini-chat", "prompt": ["a", "b"] }),
        json!({ "model": "gpt-5-mini-chat", "prompt": [1, 2, 3] }),
        json!({ "model": "gpt-5-mini-chat" }),
        json!({ "model": "gpt-5-mini-chat", "prompt": "x", "echo": true }),
        json!({ "model": "gpt-5-mini-chat", "prompt": "x", "best_of": 2 }),
    ] {
        let (status, response) = json_post(&ctx, "/v1/completions", body.clone()).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "must refuse {body}: {response}"
        );
    }
}

async fn install_prefix_stabilize_rule(ctx: &TestContext, config: Value) {
    let provider = ctx
        .state
        .monoize_store
        .list_providers()
        .await
        .expect("list providers")
        .into_iter()
        .find(|provider| provider.name == "up-chat")
        .expect("chat_completion provider");

    ctx.state
        .monoize_store
        .update_provider(
            &provider.id,
            monoize::monoize_routing::UpdateMonoizeProviderInput {
                confirm_public_exposure: false,
                name: None,
                channel: None,
                pricing_profile: None,
                multiplier: None,
                channel_max_retries: None,
                channel_retry_interval_ms: None,
                circuit_breaker_enabled: None,
                per_model_circuit_break: None,
                transforms: Some(vec![monoize::transforms::TransformRuleConfig {
                    transform: "cache_prefix_stabilize".to_string(),
                    enabled: true,
                    models: None,
                    phase: monoize::transforms::Phase::Request,
                    config,
                }]),
                active_probe_enabled_override: None,
                api_type_overrides: None,
                active_probe_interval_seconds_override: None,
                active_probe_success_threshold_override: None,
                active_probe_model_override: None,
                request_timeout_ms_override: None,
                extra_fields_whitelist: None,
                strip_cross_protocol_nested_extra: None,
                group_id: None,
                enabled: None,
                priority: None,
            },
        )
        .await
        .expect("install cache_prefix_stabilize rule");
}

/// The agent shape that motivates ACPS-1: a stable instruction prefix interrupted early by a
/// block that changes on every request.
const AGENT_SYSTEM_PROMPT: &str = "You are a coding agent.\n\
<env>\nWorking directory: /workspace\nToday's date: 2026-09-10\n</env>\n\
## Rules\nPrefer the existing convention.";

/// ACPS-15: the volatile block must reach the upstream behind the stable text, so the stable
/// text becomes a prefix the upstream can cache. Asserting on the body the upstream received
/// is what proves the rewrite survives decode, transform, and encode rather than only holding
/// inside the transform's own unit test.
#[tokio::test]
async fn prefix_stabilize_moves_the_agent_env_block_behind_the_stable_system_text() {
    let ctx = setup().await;
    install_prefix_stabilize_rule(&ctx, json!({})).await;

    let (status, response) = json_post(
        &ctx,
        "/v1/chat/completions",
        json!({
            "model": "gpt-5-mini-chat",
            "messages": [
                { "role": "system", "content": AGENT_SYSTEM_PROMPT },
                { "role": "user", "content": "which rule applies?" }
            ]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");

    let upstream = last_captured_body(&ctx, "chat");
    assert_eq!(
        upstream["messages"][0]["content"],
        json!(
            "You are a coding agent.\n## Rules\nPrefer the existing convention.\n\
<env>\nWorking directory: /workspace\nToday's date: 2026-09-10\n</env>"
        ),
        "the volatile block must arrive after the stable text: {upstream}"
    );
    assert_eq!(
        upstream["messages"][1]["content"],
        json!("which rule applies?"),
        "{upstream}"
    );
}

/// ACPS-16: relocation is a reordering. No line may be dropped, added, or edited, because the
/// model must still read everything the caller sent.
#[tokio::test]
async fn prefix_stabilize_preserves_every_line_of_the_system_prompt() {
    let ctx = setup().await;
    install_prefix_stabilize_rule(&ctx, json!({})).await;

    let (status, response) = json_post(
        &ctx,
        "/v1/chat/completions",
        json!({
            "model": "gpt-5-mini-chat",
            "messages": [
                { "role": "system", "content": AGENT_SYSTEM_PROMPT },
                { "role": "user", "content": "go" }
            ]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");

    let upstream = last_captured_body(&ctx, "chat");
    let delivered = upstream["messages"][0]["content"]
        .as_str()
        .expect("system content")
        .to_string();
    let mut sent_lines: Vec<&str> = AGENT_SYSTEM_PROMPT.lines().collect();
    let mut delivered_lines: Vec<&str> = delivered.lines().collect();
    sent_lines.sort_unstable();
    delivered_lines.sort_unstable();
    assert_eq!(sent_lines, delivered_lines, "{upstream}");
}

/// ACPS-12: the leading system run is the only region this transform may read or write. A user
/// message that happens to contain the same delimiters is the caller's own content.
#[tokio::test]
async fn prefix_stabilize_leaves_user_content_containing_the_delimiters_alone() {
    let ctx = setup().await;
    install_prefix_stabilize_rule(&ctx, json!({})).await;

    let user_text = "<env>\nmy own notes\n</env>";
    let (status, response) = json_post(
        &ctx,
        "/v1/chat/completions",
        json!({
            "model": "gpt-5-mini-chat",
            "messages": [
                { "role": "system", "content": "Stable instructions." },
                { "role": "user", "content": user_text }
            ]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");

    let upstream = last_captured_body(&ctx, "chat");
    assert_eq!(upstream["messages"][0]["content"], json!("Stable instructions."), "{upstream}");
    assert_eq!(upstream["messages"][1]["content"], json!(user_text), "{upstream}");
}

/// ACPS-6: an Anthropic upstream caches at a whole-node breakpoint, so the transform must not
/// reorder anything there. The same rule on the same request must leave the prompt untouched.
#[tokio::test]
async fn prefix_stabilize_does_not_rewrite_a_messages_upstream() {
    let ctx = setup().await;
    let provider = ctx
        .state
        .monoize_store
        .list_providers()
        .await
        .expect("list providers")
        .into_iter()
        .find(|provider| provider.name == "up-msg")
        .expect("messages provider");
    ctx.state
        .monoize_store
        .update_provider(
            &provider.id,
            monoize::monoize_routing::UpdateMonoizeProviderInput {
                confirm_public_exposure: false,
                name: None,
                channel: None,
                pricing_profile: None,
                multiplier: None,
                channel_max_retries: None,
                channel_retry_interval_ms: None,
                circuit_breaker_enabled: None,
                per_model_circuit_break: None,
                transforms: Some(vec![monoize::transforms::TransformRuleConfig {
                    transform: "cache_prefix_stabilize".to_string(),
                    enabled: true,
                    models: None,
                    phase: monoize::transforms::Phase::Request,
                    config: json!({}),
                }]),
                active_probe_enabled_override: None,
                api_type_overrides: None,
                active_probe_interval_seconds_override: None,
                active_probe_success_threshold_override: None,
                active_probe_model_override: None,
                request_timeout_ms_override: None,
                extra_fields_whitelist: None,
                strip_cross_protocol_nested_extra: None,
                group_id: None,
                enabled: None,
                priority: None,
            },
        )
        .await
        .expect("install rule on the messages provider");

    let (status, response) = json_post(
        &ctx,
        "/v1/chat/completions",
        json!({
            "model": "gpt-5-mini-msg",
            "messages": [
                { "role": "system", "content": AGENT_SYSTEM_PROMPT },
                { "role": "user", "content": "go" }
            ]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");

    let upstream = last_captured_body(&ctx, "messages");
    let delivered = serde_json::to_string(&upstream["system"]).expect("system json");
    assert!(
        delivered.contains("You are a coding agent.\\n<env>"),
        "the Anthropic upstream must receive the original order: {upstream}"
    );
}
