// CODEX-DUAL-1: a Codex CLI client can reach the same logical model through
// either upstream protocol. The client selects the mode, not the Channel, so both
// directions must produce a complete downstream Responses stream.

use super::*;

#[tokio::test]
async fn responses_downstream_reaches_a_native_responses_upstream() {
    let ctx = setup().await;
    let (upstream_addr, _, captured_bodies) = start_upstream().await;
    let base_url = format!("http://{upstream_addr}");

    // The harness already routes `gpt-5-mini` to its Responses Channel. Add a
    // second logical model on an explicitly native Responses Channel so the
    // assertion does not depend on the shared fixture's provider set.
    create_test_provider(
        &ctx.state,
        "codex-dual-native",
        monoize::monoize_routing::MonoizeProviderType::Responses,
        "gpt-5-mini-native",
        &base_url,
        "upstream-key",
    )
    .await;
    seed_test_model_pricing(&ctx.state, &["gpt-5-mini-native"]).await;

    let req = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, ctx.auth_header.clone())
        .body(Body::from(
            json!({
                "model": "gpt-5-mini-native",
                "input": "codex native mode",
                "stream": true
            })
            .to_string(),
        ))
        .unwrap();
    let resp = ctx.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let text = String::from_utf8_lossy(&resp.into_body().collect().await.unwrap().to_bytes())
        .to_string();

    assert!(text.contains("event: response.created"), "{text}");
    assert!(text.contains("event: response.output_item.added"), "{text}");
    assert!(text.contains("event: response.completed"), "{text}");
    assert!(text.contains("data: [DONE]"), "{text}");

    let captured = captured_bodies.lock().unwrap();
    let (endpoint, _) = captured.last().expect("captured upstream body");
    assert_eq!(
        endpoint, "responses",
        "a Responses Channel must receive the Responses upstream shape"
    );
}

#[tokio::test]
async fn responses_downstream_reaches_a_chat_upstream_through_the_compatible_override() {
    let ctx = setup().await;
    let (upstream_addr, _, captured_bodies) = start_upstream().await;
    let base_url = format!("http://{upstream_addr}");

    // One Channel whose native protocol is Chat Completions, overridden to
    // Responses for this model. This is the shape a Codex CLI compatible-mode
    // deployment uses: the downstream client keeps speaking Responses while the
    // upstream speaks Chat Completions.
    let mut models = HashMap::new();
    models.insert(
        "gpt-5-mini-compat".to_string(),
        monoize::monoize_routing::MonoizeModelEntry {
            redirect: None,
            pricing_profile_mode: Default::default(),
            pricing_profile_override: None,
            multiplier_override: Some(monoize::exact_decimal::Multiplier::ONE),
        },
    );
    seed_test_model_pricing(&ctx.state, &["gpt-5-mini-compat"]).await;
    ctx.state
        .monoize_store
        .create_provider(monoize::monoize_routing::CreateMonoizeProviderInput {
            confirm_public_exposure: true,
            pricing_profile: Some("openai".to_string()),
            multiplier: Default::default(),
            name: "codex-dual-compat".to_string(),
            api_type_overrides: vec![monoize::monoize_routing::ApiTypeOverride {
                pattern: "gpt-5-mini-compat".to_string(),
                api_type: monoize::monoize_routing::MonoizeProviderType::Responses,
            }],
            group_id: String::new(),
            channel: monoize::monoize_routing::CreateMonoizeChannelInput {
                name: "codex-dual-compat-channel".to_string(),
                provider_type: monoize::monoize_routing::MonoizeProviderType::ChatCompletion,
                base_url,
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
                extra_headers: None,
                session_affinity_auto: None,
            },
            channel_max_retries: 0,
            channel_retry_interval_ms: 0,
            circuit_breaker_enabled: true,
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
        .expect("compat Provider creates");

    let req = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, ctx.auth_header.clone())
        .body(Body::from(
            json!({
                "model": "gpt-5-mini-compat",
                "input": "codex compatible mode",
                "stream": true
            })
            .to_string(),
        ))
        .unwrap();
    let resp = ctx.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let text = String::from_utf8_lossy(&resp.into_body().collect().await.unwrap().to_bytes())
        .to_string();

    assert!(text.contains("event: response.created"), "{text}");
    assert!(text.contains("event: response.output_item.added"), "{text}");
    assert!(text.contains("event: response.completed"), "{text}");
    assert!(text.contains("data: [DONE]"), "{text}");

    let captured = captured_bodies.lock().unwrap();
    let (endpoint, upstream_body) = captured.last().expect("captured upstream body");
    assert_eq!(
        endpoint, "responses",
        "the compat override must select the Responses upstream shape; body={upstream_body}"
    );
}

#[tokio::test]
async fn chat_downstream_reaches_a_responses_upstream_through_the_compatible_override() {
    let ctx = setup().await;
    let (upstream_addr, _, captured_bodies) = start_upstream().await;
    let base_url = format!("http://{upstream_addr}");

    // The reverse direction: the downstream client speaks Chat Completions while
    // the Channel speaks Responses.
    let mut models = HashMap::new();
    models.insert(
        "gpt-5-mini-native-compat".to_string(),
        monoize::monoize_routing::MonoizeModelEntry {
            redirect: None,
            pricing_profile_mode: Default::default(),
            pricing_profile_override: None,
            multiplier_override: Some(monoize::exact_decimal::Multiplier::ONE),
        },
    );
    seed_test_model_pricing(&ctx.state, &["gpt-5-mini-native-compat"]).await;
    ctx.state
        .monoize_store
        .create_provider(monoize::monoize_routing::CreateMonoizeProviderInput {
            confirm_public_exposure: true,
            pricing_profile: Some("openai".to_string()),
            multiplier: Default::default(),
            name: "codex-dual-native-compat".to_string(),
            api_type_overrides: vec![monoize::monoize_routing::ApiTypeOverride {
                pattern: "gpt-5-mini-native-compat".to_string(),
                api_type: monoize::monoize_routing::MonoizeProviderType::ChatCompletion,
            }],
            group_id: String::new(),
            channel: monoize::monoize_routing::CreateMonoizeChannelInput {
                name: "codex-dual-native-compat-channel".to_string(),
                provider_type: monoize::monoize_routing::MonoizeProviderType::Responses,
                base_url,
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
                extra_headers: None,
                session_affinity_auto: None,
            },
            channel_max_retries: 0,
            channel_retry_interval_ms: 0,
            circuit_breaker_enabled: true,
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
        .expect("compat Provider creates");

    let req = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, ctx.auth_header.clone())
        .body(Body::from(
            json!({
                "model": "gpt-5-mini-native-compat",
                "messages": [{ "role": "user", "content": "codex chat mode" }],
                "stream": true
            })
            .to_string(),
        ))
        .unwrap();
    let resp = ctx.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let text = String::from_utf8_lossy(&resp.into_body().collect().await.unwrap().to_bytes())
        .to_string();

    assert!(text.contains("chat.completion.chunk"), "{text}");
    assert!(text.contains("data: [DONE]"), "{text}");

    let captured = captured_bodies.lock().unwrap();
    let (endpoint, upstream_body) = captured.last().expect("captured upstream body");
    assert_eq!(
        endpoint, "chat",
        "the Chat Completions override must select the Chat upstream shape; body={upstream_body}"
    );
}

#[tokio::test]
async fn codex_usage_endpoint_answers_both_polling_shapes() {
    let ctx = setup().await;

    for path in ["/api/codex/usage", "/user/balance"] {
        let req = Request::builder()
            .method("GET")
            .uri(path)
            .header(AUTHORIZATION, ctx.auth_header.clone())
            .body(Body::empty())
            .unwrap();
        let resp = ctx.router.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "path={path}");
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&bytes).expect("usage JSON");
        assert!(
            body.is_object(),
            "usage response must be a JSON object: path={path} body={body}"
        );
    }
}

#[tokio::test]
async fn repeated_usage_polls_stay_consistent_within_the_cache_window() {
    let ctx = setup().await;

    let mut observed = Vec::new();
    for _ in 0..5 {
        let req = Request::builder()
            .method("GET")
            .uri("/api/codex/usage")
            .header(AUTHORIZATION, ctx.auth_header.clone())
            .body(Body::empty())
            .unwrap();
        let resp = ctx.router.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        observed.push(serde_json::from_slice::<Value>(&bytes).expect("usage JSON"));
    }

    let first = observed.first().expect("at least one poll");
    for value in observed.iter().skip(1) {
        assert_eq!(
            value["rate_limit"]["allowed"], first["rate_limit"]["allowed"],
            "the display balance must not oscillate between polls"
        );
    }
}
