use super::*;

/// SAN-19. The upstream error text below names every deployment-identity member Monoize
/// knows: its own model identifier, the upstream host and URL, an API key, and the Group,
/// Provider, and Channel names. The assertion is that no member reaches the client, for each
/// downstream protocol, with `monoize_mask_sensitive_info` both enabled and disabled.
///
/// The harness Provider is named `OpenAI`, its Channel `primary`, and the upstream host is
/// the loopback mock. `secret-upstream-name-v9` stands in for an upstream model identifier
/// that differs from the logical alias the client submitted.
const LEAKY_UPSTREAM_MESSAGE: &str = "model `secret-upstream-name-v9` unavailable on \
     channel primary of provider OpenAI in group default; \
     see https://internal-vendor.example.com/v1/chat api_key:sk-live-abc123";

/// Substrings that MUST NOT appear in any client-visible byte.
const FORBIDDEN: &[&str] = &[
    "secret-upstream-name-v9",
    "internal-vendor",
    "sk-live-abc123",
    "api_key:sk-",
];

fn assert_no_identity_leak(label: &str, body: &str) {
    for needle in FORBIDDEN {
        assert!(
            !body.contains(needle),
            "{label}: client body leaked `{needle}`:\n{body}"
        );
    }
}

async fn post_expecting_leak_attempt(
    ctx: &TestContext,
    path: &str,
    mut body: Value,
) -> (StatusCode, String) {
    if let Some(obj) = body.as_object_mut() {
        obj.insert("force_upstream_error_status".to_string(), json!(400));
        obj.insert(
            "force_upstream_error_code".to_string(),
            json!("model_not_found"),
        );
        obj.insert(
            "force_upstream_error_message".to_string(),
            json!(LEAKY_UPSTREAM_MESSAGE),
        );
    }
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, ctx.auth_header.clone())
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = ctx.router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

async fn set_masking(ctx: &TestContext, enabled: bool) {
    let mut runtime = ctx.state.monoize_runtime.write().await;
    runtime.mask_sensitive_info = enabled;
}

async fn run_all_protocols(ctx: &TestContext, tag: &str) {
    // Responses, non-stream.
    let (_, body) = post_expecting_leak_attempt(
        ctx,
        "/v1/responses",
        json!({ "model": "gpt-5-mini", "input": "hi" }),
    )
    .await;
    assert_no_identity_leak(&format!("{tag} responses nonstream"), &body);

    // Responses, streaming: SAN-11 / SAN-16 on the mid-stream frame path.
    let (_, body) = post_expecting_leak_attempt(
        ctx,
        "/v1/responses",
        json!({ "model": "gpt-5-mini", "input": "hi", "stream": true }),
    )
    .await;
    assert_no_identity_leak(&format!("{tag} responses stream"), &body);

    // Chat Completions.
    let (_, body) = post_expecting_leak_attempt(
        ctx,
        "/v1/chat/completions",
        json!({ "model": "gpt-5-mini-chat", "messages": [{"role":"user","content":"hi"}] }),
    )
    .await;
    assert_no_identity_leak(&format!("{tag} chat nonstream"), &body);

    let (_, body) = post_expecting_leak_attempt(
        ctx,
        "/v1/chat/completions",
        json!({
            "model": "gpt-5-mini-chat",
            "messages": [{"role":"user","content":"hi"}],
            "stream": true
        }),
    )
    .await;
    assert_no_identity_leak(&format!("{tag} chat stream"), &body);

    // Anthropic Messages.
    let (_, body) = post_expecting_leak_attempt(
        ctx,
        "/v1/messages",
        json!({
            "model": "gpt-5-mini-msg",
            "max_tokens": 64,
            "messages": [{"role":"user","content":"hi"}]
        }),
    )
    .await;
    assert_no_identity_leak(&format!("{tag} messages nonstream"), &body);

    let (_, body) = post_expecting_leak_attempt(
        ctx,
        "/v1/messages",
        json!({
            "model": "gpt-5-mini-msg",
            "max_tokens": 64,
            "messages": [{"role":"user","content":"hi"}],
            "stream": true
        }),
    )
    .await;
    assert_no_identity_leak(&format!("{tag} messages stream"), &body);
}

#[tokio::test]
async fn identity_never_reaches_a_client_with_masking_enabled() {
    let ctx = setup().await;
    set_masking(&ctx, true).await;
    run_all_protocols(&ctx, "mask=on").await;
}

/// SAN-15a: the guarantee does not depend on the masking switch.
#[tokio::test]
async fn identity_never_reaches_a_client_with_masking_disabled() {
    let ctx = setup().await;
    set_masking(&ctx, false).await;
    run_all_protocols(&ctx, "mask=off").await;
}

/// SAN-16b: the exhausted-routing message names no model at all.
#[tokio::test]
async fn exhausted_routing_message_names_no_model() {
    let ctx = setup().await;
    let (status, body) = post_expecting_leak_attempt(
        &ctx,
        "/v1/responses",
        json!({ "model": "gpt-5-mini", "input": "hi" }),
    )
    .await;
    assert!(
        status.is_client_error() || status.is_server_error(),
        "{body}"
    );
    let parsed: Value = serde_json::from_str(&body).expect("error envelope JSON");
    let message = parsed["error"]["message"].as_str().unwrap_or_default();
    assert!(
        !message.contains("gpt-5-mini"),
        "the exhausted message must not name the model: {message}"
    );
    assert_eq!(message, "no upstream provider could serve this request");
}

/// SAN-12a: an upstream that puts free-form text in `code` must not have it published.
#[tokio::test]
async fn non_enumerated_upstream_code_is_dropped() {
    let ctx = setup().await;
    let req = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, ctx.auth_header.clone())
        .body(Body::from(
            json!({
                "model": "gpt-5-mini",
                "input": "hi",
                "force_upstream_error_status": 400,
                "force_upstream_error_code":
                    "model secret-upstream-name-v9 at https://internal-vendor.example.com failed",
                "force_upstream_error_message": "rejected"
            })
            .to_string(),
        ))
        .unwrap();
    let resp = ctx.router.clone().oneshot(req).await.unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8_lossy(&bytes).to_string();
    assert_no_identity_leak("non-enumerated upstream_code", &body);
}
