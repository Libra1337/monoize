use super::*;

/// MB-M5: a server-native tool usage class with no configured meter rate must be admitted
/// and settle at zero, not rejected before request-log admission. The Codex client enables
/// `web_search` on every request, so a rejecting gate made the gateway serve no traffic.
#[tokio::test]
async fn unpriced_server_tool_class_is_admitted_and_settles_at_zero() {
    let ctx = setup().await;
    let (status, body) = json_post(
        &ctx,
        "/v1/responses",
        json!({
            "model": "gpt-5-mini",
            "input": "hello",
            "tools": [{ "type": "web_search" }]
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        !body.contains("model_pricing_required"),
        "an unpriced server-tool class must not reject the request: {body}"
    );
}

/// SE1c: a pre-stream error on `/v1/responses` must reach the client as a `response.failed`
/// terminal carrying the real code and message. A bare `error` frame alone is discarded by
/// the Codex reader, which then reports a generic truncation instead of the actual cause.
#[tokio::test]
async fn prestream_error_emits_response_failed_terminal_with_real_cause() {
    let ctx = setup().await;
    let req = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, ctx.auth_header.clone())
        .body(Body::from(
            json!({
                "model": "model-that-no-channel-serves",
                "input": "hello",
                "stream": true
            })
            .to_string(),
        ))
        .unwrap();
    let resp = ctx.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "SE1 commits HTTP 200");
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).to_string();

    let frames = parse_sse_frames(&text);
    let error_frame = frames
        .iter()
        .find(|(event, _)| event.as_deref() == Some("error"))
        .map(|(_, data)| serde_json::from_str::<Value>(data).expect("error frame JSON"))
        .expect("SE1 error frame");
    let failed_frame = frames
        .iter()
        .find(|(event, _)| event.as_deref() == Some("response.failed"))
        .map(|(_, data)| serde_json::from_str::<Value>(data).expect("failed frame JSON"))
        .expect("SE1c response.failed frame");

    // The terminal must carry the same cause as the error frame, not a generic message.
    assert_eq!(
        failed_frame["response"]["error"]["code"], error_frame["code"],
        "{text}"
    );
    assert_eq!(
        failed_frame["response"]["error"]["message"], error_frame["message"],
        "{text}"
    );
    assert_eq!(
        failed_frame["response"]["status"],
        json!("failed"),
        "{text}"
    );

    // Frame order per SE1c: error, then response.failed, then [DONE].
    let error_at = text.find("event: error").expect("error frame position");
    let failed_at = text
        .find("event: response.failed")
        .expect("failed frame position");
    assert!(
        error_at < failed_at,
        "error must precede the terminal: {text}"
    );
    assert!(text.trim_end().ends_with("data: [DONE]"), "{text}");
}
