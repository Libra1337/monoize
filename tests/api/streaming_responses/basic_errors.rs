
#[tokio::test]
async fn responses_streaming_returns_sse_before_delayed_upstream_headers() {
    let ctx = setup().await;
    let req = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, ctx.auth_header.clone())
        .body(Body::from(
            json!({
                "model": "gpt-5-mini",
                "input": "delayed upstream headers",
                "stream": true,
                "force_upstream_delay_ms": 800
            })
            .to_string(),
        ))
        .unwrap();

    let response = tokio::time::timeout(
        Duration::from_millis(200),
        ctx.router.clone().oneshot(req),
    )
    .await
    .expect("streaming handler must return before upstream headers")
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let text = String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes())
        .to_string();
    assert!(text.contains("event: response.completed"), "{text}");
}

#[tokio::test]
async fn responses_streaming_completed_preserves_service_tier() {
    let ctx = setup().await;
    let req = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, ctx.auth_header.clone())
        .body(Body::from(
            json!({
                "model":"gpt-5-mini",
                "input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"stream"}]}],
                "stream": true,
                "stream_mode": "message_then_tool_then_completed",
                "tools": [{ "type": "function", "name": "tool_a", "parameters": { "type": "object", "additionalProperties": true } }],
                "service_tier": "priority"
            })
            .to_string(),
        ))
        .unwrap();
    let resp = ctx.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).to_string();
    let frames = parse_responses_sse_json(&text);
    let completed = frames
        .iter()
        .find(|(event, _)| event == "response.completed")
        .map(|(_, payload)| payload)
        .expect("response.completed frame");
    assert_eq!(
        completed["response"]["service_tier"].as_str(),
        Some("priority")
    );
}

#[tokio::test]
async fn responses_streaming_done_without_terminal_event_is_failure() {
    let ctx = setup().await;
    let req = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, ctx.auth_header.clone())
        .body(Body::from(
            json!({
                "model": "gpt-5-mini",
                "input": "stream",
                "stream": true,
                "stream_mode": "missing_terminal"
            })
            .to_string(),
        ))
        .unwrap();

    let resp = ctx.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let text = String::from_utf8_lossy(&resp.into_body().collect().await.unwrap().to_bytes())
        .to_string();
    let frames = parse_responses_sse_json(&text);

    assert!(!frames.iter().any(|(event, _)| event == "response.completed"));
    let failed = frames
        .iter()
        .find(|(event, _)| event == "response.failed")
        .map(|(_, payload)| payload)
        .expect("response.failed frame");
    assert_eq!(
        failed["response"]["error"]["code"].as_str(),
        Some("responses_stream_missing_terminal")
    );
    assert_eq!(count_done_sentinels(&text), 1);
}

#[tokio::test]
async fn responses_streaming_incomplete_terminal_remains_incomplete() {
    let ctx = setup().await;
    let req = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, ctx.auth_header.clone())
        .body(Body::from(
            json!({
                "model": "gpt-5-mini",
                "input": "stream",
                "stream": true,
                "stream_mode": "incomplete_terminal"
            })
            .to_string(),
        ))
        .unwrap();

    let resp = ctx.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let text = String::from_utf8_lossy(&resp.into_body().collect().await.unwrap().to_bytes())
        .to_string();
    let frames = parse_responses_sse_json(&text);

    assert!(!frames.iter().any(|(event, _)| event == "response.completed"));
    assert!(!frames.iter().any(|(event, _)| event == "response.failed"));
    let incomplete = frames
        .iter()
        .find(|(event, _)| event == "response.incomplete")
        .map(|(_, payload)| payload)
        .expect("response.incomplete frame");
    assert_eq!(incomplete["response"]["status"], json!("incomplete"));
    assert_eq!(
        incomplete["response"]["incomplete_details"]["reason"],
        json!("max_output_tokens")
    );
    assert_eq!(count_done_sentinels(&text), 1);
}

#[tokio::test]
async fn responses_streaming_image_generation_completed_emits_native_top_level_item() {
    let ctx = setup().await;
    let req = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, ctx.auth_header.clone())
        .body(Body::from(
            json!({
                "model":"gpt-5-mini",
                "input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"generate image"}]}],
                "tools":[{"type":"image_generation","output_format":"png"}],
                "stream": true,
                "stream_mode": "image_generation_completed"
            })
            .to_string(),
        ))
        .unwrap();
    let resp = ctx.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).to_string();
    let frames = parse_responses_sse_json(&text);

    assert!(!frames.iter().any(|(event, payload)| {
        event == "response.content_part.done"
            && payload["part"]["type"].as_str() == Some("output_image")
    }));
    assert!(frames.iter().any(|(event, payload)| {
        event == "response.output_item.done"
            && payload["item"]["type"].as_str() == Some("image_generation_call")
            && payload["item"]["result"]
                .as_str()
                .is_some_and(|result| !result.is_empty())
    }), "{text}");
    assert!(
        frames.iter().any(|(event, payload)| {
            event == "response.completed"
                && payload["response"]["output"]
                    .as_array()
                    .is_some_and(|output| {
                        output.iter().any(|item| {
                            item["type"].as_str() == Some("image_generation_call")
                                && item["id"].as_str() == Some("ig_mock")
                                && item["output_format"].as_str() == Some("png")
                                && item["result"]
                                    .as_str()
                                    .is_some_and(|result| !result.is_empty())
                        })
                    })
        }),
        "{text}"
    );
    assert!(!text.contains("output_image"), "{text}");
}

#[tokio::test]
async fn responses_streaming_completed_snapshot_preserves_native_image_generation_item() {
    let ctx = setup().await;
    let req = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, ctx.auth_header.clone())
        .body(Body::from(
            json!({
                "model":"gpt-5-mini",
                "input":"generate image",
                "tools":[{"type":"image_generation","output_format":"webp"}],
                "stream": true,
                "stream_mode": "image_generation_completed_snapshot_only"
            })
            .to_string(),
        ))
        .unwrap();
    let resp = ctx.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).to_string();
    let frames = parse_responses_sse_json(&text);

    assert!(
        frames.iter().any(|(event, payload)| {
            event == "response.completed"
                && payload["response"]["output"]
                    .as_array()
                    .is_some_and(|output| {
                        output.iter().any(|item| {
                            item["type"].as_str() == Some("image_generation_call")
                                && item["id"].as_str() == Some("ig_mock")
                                && item["output_format"].as_str() == Some("webp")
                                && item["result"]
                                    .as_str()
                                    .is_some_and(|data| !data.is_empty())
                        })
                    })
        }),
        "{text}"
    );
    assert!(!text.contains("output_image"), "{text}");
}

#[tokio::test]
async fn responses_streaming_reconstructs_phase_from_output_item_done() {
    let ctx = setup().await;
    let req = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, ctx.auth_header.clone())
        .body(Body::from(
            json!({
                "model":"gpt-5-mini",
                "input":"stream",
                "stream": true,
                "stream_mode": "item_done_only",
                "message_phase": "commentary"
            })
            .to_string(),
        ))
        .unwrap();
    let resp = ctx.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).to_string();
    assert!(text.contains("event: response.output_text.delta"));
    assert!(text.contains("\"phase\":\"commentary\""));
}

#[tokio::test]
async fn responses_streaming_consumes_next_envelope_extra_exactly_once() {
    let ctx = setup().await;
    let req = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, ctx.auth_header.clone())
        .body(Body::from(
            json!({
                "model":"gpt-5-mini",
                "stream": true,
                "input": [
                    {
                        "type": "message",
                        "role": "user",
                        "content": [{ "type": "input_text", "text": "first" }],
                        "first_only": "A"
                    },
                    {
                        "type": "message",
                        "role": "assistant",
                        "content": [{ "type": "output_text", "text": "second" }]
                    }
                ]
            })
            .to_string(),
        ))
        .unwrap();
    let resp = ctx.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).to_string();
    let frames = parse_responses_sse_json(&text);

    let added_items: Vec<&Value> = frames
        .iter()
        .filter(|(event, _)| event == "response.output_item.added")
        .map(|(_, payload)| &payload["item"])
        .collect();
    assert!(
        !added_items.is_empty(),
        "expected at least one visible output item: {text}"
    );
    assert_eq!(
        added_items[0]["first_only"],
        json!("A"),
        "control-node metadata must land on the next output item envelope: {text}"
    );
    for item in added_items.iter().skip(1) {
        assert!(
            item.get("first_only").is_none(),
            "control-node metadata must be consumed exactly once: {text}"
        );
    }
    assert!(
        added_items
            .iter()
            .all(|item| item["type"].as_str() != Some("next_downstream_envelope_extra")),
        "control node must not surface as a visible Responses item: {text}"
    );

    let completed = frames
        .iter()
        .find(|(event, _)| event == "response.completed")
        .map(|(_, payload)| payload)
        .expect("response.completed frame");
    let output = completed["response"]["output"]
        .as_array()
        .expect("completed response output array");
    assert_eq!(output[0]["first_only"], json!("A"));
    for item in output.iter().skip(1) {
        assert!(item.get("first_only").is_none());
    }
}

#[tokio::test]
async fn responses_streaming_includes_tool_calls_and_reasoning_when_upstream_is_chat() {
    let ctx = setup().await;
    let req = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, ctx.auth_header.clone())
        .body(Body::from(
            json!({
                "model":"gpt-5-mini-chat",
                "input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"stream tool"}]}],
                "tools":[{ "type":"function","function":{ "name":"tool_a","parameters":{ "type":"object","additionalProperties":true }}}],
                "parallel_tool_calls": true,
                "stream": true
            })
            .to_string(),
        ))
        .unwrap();
    let resp = ctx.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).to_string();
    assert!(text.contains("event: response.function_call_arguments.delta"));
    assert!(text.contains("event: response.reasoning_summary_text.delta"));
}

#[tokio::test]
async fn responses_streaming_reencodes_greedy_merged_items_with_canonical_sse_boundaries() {
    let ctx = setup().await;
    let req = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, ctx.auth_header.clone())
        .body(Body::from(
            json!({
                "model":"gpt-5-mini",
                "input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"stream tool"}]}],
                "tools":[{ "type":"function","name":"tool_a","parameters":{ "type":"object","additionalProperties":true }}],
                "stream": true,
                "stream_mode": "reasoning_text_tool"
            })
            .to_string(),
        ))
        .unwrap();
    let resp = ctx.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).to_string();

    assert!(text.contains("event: response.output_item.added"));
    assert!(text.contains("\"output_index\":0"));
    assert!(text.contains("\"output_index\":1"));
    assert!(text.contains("\"output_index\":2"));
    assert!(text.contains("\"type\":\"reasoning\""));
    assert!(text.contains("\"type\":\"message\""));
    assert!(text.contains("\"type\":\"function_call\""));
    assert!(text.contains("\"phase\":\"analysis\""));
    let frames = parse_responses_sse_json(&text);
    let reasoning_added = frames
        .iter()
        .find(|(event, payload)| {
            event == "response.output_item.added"
                && payload["item"]["type"].as_str() == Some("reasoning")
        })
        .expect("reasoning output_item.added");
    let added_envelope = monoize::urp::parse_reasoning_envelope(
        &reasoning_added.1["item"]["encrypted_content"],
    )
    .expect("added reasoning envelope");
    assert_eq!(added_envelope.item_id.as_deref(), Some("rs_mock"));
    assert_eq!(added_envelope.payload, json!("added_snapshot_sig"));
    assert!(text.contains("event: response.content_part.added"));
    assert!(text.contains(
        "\"part\":{\"annotations\":[],\"logprobs\":[],\"text\":\"\",\"type\":\"output_text\"}"
    ));
    assert!(!text.contains("\"part\":{\"text\":\"\",\"type\":\"reasoning\"}"));
    assert!(!text.contains("event: response.content_part.added\ndata: {\"content_index\":2"));
}

#[tokio::test]
async fn responses_streaming_preserves_distinct_encrypted_reasoning_snapshots() {
    let ctx = setup().await;
    let (status, text) = json_post(
        &ctx,
        "/v1/responses",
        json!({
            "model": "gpt-5-mini",
            "input": "stream tool",
            "tools": [{
                "type": "function",
                "name": "tool_a",
                "parameters": { "type": "object", "additionalProperties": true }
            }],
            "stream": true,
            "stream_mode": "reasoning_text_tool"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");

    let frames = parse_responses_sse_json(&text);
    let reasoning_added = frames
        .iter()
        .find(|(event, payload)| {
            event == "response.output_item.added"
                && payload["item"]["type"].as_str() == Some("reasoning")
        })
        .expect("reasoning output_item.added");
    let added_envelope = monoize::urp::parse_reasoning_envelope(
        &reasoning_added.1["item"]["encrypted_content"],
    )
    .expect("added reasoning envelope");
    assert_eq!(reasoning_added.1["item"]["status"], json!("in_progress"));
    assert_eq!(added_envelope.item_id.as_deref(), Some("rs_mock"));
    assert_eq!(added_envelope.payload, json!("added_snapshot_sig"));

    let reasoning_done = frames
        .iter()
        .find(|(event, payload)| {
            event == "response.output_item.done"
                && payload["item"]["type"].as_str() == Some("reasoning")
        })
        .expect("reasoning output_item.done");
    let done_envelope = monoize::urp::parse_reasoning_envelope(
        &reasoning_done.1["item"]["encrypted_content"],
    )
    .expect("completed reasoning envelope");
    assert_eq!(reasoning_done.1["item"]["status"], json!("completed"));
    assert_eq!(done_envelope.item_id.as_deref(), Some("rs_mock"));
    assert_eq!(done_envelope.payload, json!("mock_sig"));
    assert_ne!(
        reasoning_added.1["item"]["encrypted_content"],
        reasoning_done.1["item"]["encrypted_content"],
        "added and done snapshots must remain separate complete values"
    );

    let completed = frames
        .iter()
        .find(|(event, _)| event == "response.completed")
        .expect("response.completed");
    let terminal_reasoning = completed.1["response"]["output"]
        .as_array()
        .expect("terminal output")
        .iter()
        .find(|item| item["type"].as_str() == Some("reasoning"))
        .expect("terminal reasoning item");
    let terminal_envelope =
        monoize::urp::parse_reasoning_envelope(&terminal_reasoning["encrypted_content"])
            .expect("terminal reasoning envelope");
    assert_eq!(terminal_envelope.item_id.as_deref(), Some("rs_mock"));
    assert_eq!(terminal_envelope.payload, json!("mock_sig"));
    assert_eq!(
        terminal_reasoning["encrypted_content"],
        reasoning_done.1["item"]["encrypted_content"],
        "response.completed must reuse the complete terminal snapshot"
    );
    assert_eq!(
        terminal_reasoning["status"], reasoning_done.1["item"]["status"],
        "response.completed must preserve the done item's terminal status"
    );
}
