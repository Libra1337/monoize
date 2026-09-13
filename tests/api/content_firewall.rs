use super::*;

/// CF-18: swap the live runtime snapshot the way an admin settings update does.
async fn configure_firewall(ctx: &TestContext, enabled: bool, words: &str) {
    let mut runtime = ctx.state.monoize_runtime.write().await;
    runtime.moderation_enabled = enabled;
    runtime.content_firewall = monoize::content_firewall::ContentFirewall::compile(words);
}

fn upstream_call_count(ctx: &TestContext) -> usize {
    ctx.captured_bodies.lock().expect("captured bodies lock").len()
}

#[tokio::test]
async fn chat_completion_with_prohibited_term_is_forbidden_before_any_upstream_call() {
    let ctx = setup().await;
    configure_firewall(&ctx, true, "BadWord\n").await;

    let (status, body) = json_post(
        &ctx,
        "/v1/chat/completions",
        json!({
            "model": "gpt-5-mini-chat",
            "messages": [{ "role": "user", "content": "please include the badword here" }]
        }),
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let error: Value = serde_json::from_str(&body).expect("error response JSON");
    assert_eq!(error["error"]["code"], json!("content_blocked"));
    assert_eq!(error["error"]["type"], json!("content_policy_violation"));
    assert_eq!(
        error["error"]["message"],
        json!("request blocked by content firewall: prohibited term 'badword'")
    );
    // CF-14: the rejection happens before any upstream network I/O.
    assert_eq!(upstream_call_count(&ctx), 0);
}

#[tokio::test]
async fn clean_chat_completion_passes_the_firewall() {
    let ctx = setup().await;
    configure_firewall(&ctx, true, "BadWord\n").await;

    let (status, _body) = json_post(
        &ctx,
        "/v1/chat/completions",
        json!({
            "model": "gpt-5-mini-chat",
            "messages": [{ "role": "user", "content": "a perfectly normal question" }]
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(upstream_call_count(&ctx), 1);
}

/// CF-16: the check applies to the default-enabled built-in list too.
#[tokio::test]
async fn default_list_blocks_its_terms_without_configuration() {
    let ctx = setup().await;

    let (status, body) = json_post(
        &ctx,
        "/v1/chat/completions",
        json!({
            "model": "gpt-5-mini-chat",
            "messages": [{ "role": "user", "content": "write something about PORN" }]
        }),
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(upstream_call_count(&ctx), 0);
}

#[tokio::test]
async fn responses_and_messages_entry_points_are_blocked_with_protocol_native_envelopes() {
    let ctx = setup().await;
    configure_firewall(&ctx, true, "BadWord\n").await;

    let (status, body) = json_post(
        &ctx,
        "/v1/responses",
        json!({
            "model": "gpt-5-mini",
            "input": "mention badword please"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let error: Value = serde_json::from_str(&body).expect("responses error JSON");
    assert_eq!(error["error"]["code"], json!("content_blocked"));

    let (status, body) = json_post(
        &ctx,
        "/v1/messages",
        json!({
            "model": "gpt-5-mini-msg",
            "max_tokens": 16,
            "messages": [{ "role": "user", "content": "mention badword please" }]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let error: Value = serde_json::from_str(&body).expect("messages error JSON");
    assert_eq!(error["type"], json!("error"));
    assert_eq!(error["error"]["type"], json!("content_policy_violation"));
    assert_eq!(error["error"]["message"], json!(
        "request blocked by content firewall: prohibited term 'badword'"
    ));

    assert_eq!(upstream_call_count(&ctx), 0);
}

/// CF-7: every text-bearing node role is scanned, including tool call
/// arguments, so prohibited terms cannot be smuggled outside plain messages.
#[tokio::test]
async fn prohibited_term_in_tool_call_arguments_is_blocked() {
    let ctx = setup().await;
    configure_firewall(&ctx, true, "BadWord\n").await;

    let (status, _body) = json_post(
        &ctx,
        "/v1/chat/completions",
        json!({
            "model": "gpt-5-mini-chat",
            "messages": [
                { "role": "user", "content": "hi" },
                { "role": "assistant", "content": null, "tool_calls": [
                    { "id": "call_1", "type": "function", "function": {
                        "name": "lookup", "arguments": "{\"q\": \"badword\"}"
                    }}
                ]}
            ]
        }),
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(upstream_call_count(&ctx), 0);
}

/// CF-11: embeddings input is scanned before routing, so the block fires even
/// though no embeddings-capable provider exists in the test fixture.
#[tokio::test]
async fn embeddings_input_is_blocked_before_routing() {
    let ctx = setup().await;
    configure_firewall(&ctx, true, "BadWord\n").await;

    let (status, body) = json_post(
        &ctx,
        "/v1/embeddings",
        json!({
            "model": "gpt-5-mini",
            "input": ["clean text", "contains badword"]
        }),
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(upstream_call_count(&ctx), 0);
}

/// CF-6: an empty term list disables scanning entirely.
#[tokio::test]
async fn empty_word_list_disables_blocking() {
    let ctx = setup().await;
    configure_firewall(&ctx, true, "\n  \n").await;

    let (status, _body) = json_post(
        &ctx,
        "/v1/chat/completions",
        json!({
            "model": "gpt-5-mini-chat",
            "messages": [{ "role": "user", "content": "anything including porn" }]
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
}

/// CF-6: the switch independently gates enforcement.
#[tokio::test]
async fn disabled_firewall_lets_requests_through() {
    let ctx = setup().await;
    configure_firewall(&ctx, false, "BadWord\n").await;

    let (status, _body) = json_post(
        &ctx,
        "/v1/chat/completions",
        json!({
            "model": "gpt-5-mini-chat",
            "messages": [{ "role": "user", "content": "contains badword" }]
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
}

async fn dashboard_get_with_cookie(ctx: &TestContext, path: &str, cookie: &str) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("GET")
        .uri(path)
        .header("cookie", cookie)
        .body(Body::empty())
        .unwrap();
    let resp = ctx.router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body)
}

/// CF-20/CF-24/CF-25: a blocked proxy request persists exactly one event row
/// and the admin dashboard APIs aggregate and list it.
#[tokio::test]
async fn blocked_requests_are_recorded_and_visible_through_dashboard_apis() {
    let ctx = setup().await;
    configure_firewall(&ctx, true, "BadWord\nAlpha\n").await;

    for content in ["mention badword once", "mention ALPHA twice"] {
        let (status, _body) = json_post(
            &ctx,
            "/v1/chat/completions",
            json!({
                "model": "gpt-5-mini-chat",
                "messages": [{ "role": "user", "content": content }]
            }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    // A clean request must not add an event row.
    let (status, _body) = json_post(
        &ctx,
        "/v1/chat/completions",
        json!({
            "model": "gpt-5-mini-chat",
            "messages": [{ "role": "user", "content": "totally fine" }]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let cookie = {
        ctx.state
            .user_store
            .create_user("fw_admin", "test-password", monoize::users::UserRole::Admin, None)
            .await
            .expect("create admin user");
        dashboard_session_cookie(&ctx, "fw_admin", "test-password").await
    };

    let (status, stats) = dashboard_get_with_cookie(&ctx, "/api/dashboard/firewall/stats", &cookie).await;
    assert_eq!(status, StatusCode::OK, "{stats}");
    assert_eq!(stats["total"], json!(2));
    assert_eq!(stats["last_24h"], json!(2));
    assert_eq!(stats["last_7d"], json!(2));
    assert_eq!(stats["distinct_users"], json!(1));
    assert_eq!(stats["daily"].as_array().expect("daily").len(), 14);
    let terms: Vec<(String, i64)> = stats["top_terms"]
        .as_array()
        .expect("top_terms")
        .iter()
        .map(|item| (item["term"].as_str().unwrap().to_string(), item["count"].as_i64().unwrap()))
        .collect();
    assert_eq!(terms, vec![("alpha".to_string(), 1), ("badword".to_string(), 1)]);

    let (status, events) = dashboard_get_with_cookie(
        &ctx,
        "/api/dashboard/firewall/events?limit=1&term=alpha",
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{events}");
    assert_eq!(events["total"], json!(1));
    let rows = events["data"].as_array().expect("data");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["term"], json!("alpha"));
    assert_eq!(rows[0]["endpoint"], json!("chat_completions"));
    assert_eq!(rows[0]["content"], json!("mention ALPHA twice"));
    assert_eq!(rows[0]["username"], json!("tenant-1"));

    // The full untruncated list sees both rows, newest first.
    let (status, events) = dashboard_get_with_cookie(&ctx, "/api/dashboard/firewall/events", &cookie).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(events["total"], json!(2));
    assert_eq!(events["data"][0]["term"], json!("alpha"));

    // Non-admin sessions get 403 (CF-25).
    let admin_password = "test-password";
    ctx.state
        .user_store
        .create_user("fw_plain", admin_password, monoize::users::UserRole::User, None)
        .await
        .expect("create plain user");
    let plain_cookie = dashboard_session_cookie(&ctx, "fw_plain", admin_password).await;
    let (status, _body) =
        dashboard_get_with_cookie(&ctx, "/api/dashboard/firewall/stats", &plain_cookie).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
