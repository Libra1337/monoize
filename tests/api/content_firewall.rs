use super::*;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

/// CF-18: swap the live runtime snapshot the way an admin settings update does.
async fn configure_firewall(ctx: &TestContext, enabled: bool, words: &str) {
    let mut runtime = ctx.state.monoize_runtime.write().await;
    runtime.moderation_enabled = enabled;
    runtime.content_firewall = monoize::content_firewall::ContentFirewall::compile(words);
}

/// Starts a mock OpenAI-compatible judge endpoint that always answers with the
/// given category, counting the requests it received.
async fn start_judge(category: &'static str) -> (String, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let handler_calls = Arc::clone(&calls);
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(
            move |body: axum::Json<serde_json::Value>| async move {
                handler_calls.fetch_add(1, Ordering::SeqCst);
                assert_eq!(body.0["temperature"], json!(0));
                axum::Json(json!({
                    "choices": [{
                        "message": {"content": format!("{{\"category\":\"{category}\"}}")}
                    }]
                }))
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{address}"), calls)
}

/// Points the runtime judge at `base_url` (CF-18) and optionally enables it.
async fn configure_judge(ctx: &TestContext, base_url: &str, enabled: bool) {
    let mut runtime = ctx.state.monoize_runtime.write().await;
    runtime.moderation_judge = monoize::moderation_judge::JudgeConfig {
        enabled,
        base_url: base_url.to_string(),
        api_key: "test-judge-key".to_string(),
        model: "judge-model".to_string(),
        timeout_ms: 4000,
    };
}

fn upstream_call_count(ctx: &TestContext) -> usize {
    ctx.captured_bodies.lock().expect("captured bodies lock").len()
}

/// CF-31: with the judge disabled the firewall is inert — keyword matches are
/// hints only and must not block.
#[tokio::test]
async fn disabled_judge_lets_keyword_hits_through() {
    let ctx = setup().await;
    configure_firewall(&ctx, true, "BadWord\n").await;

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
    assert_eq!(upstream_call_count(&ctx), 1);
}

/// CF-15/CF-28: a porn verdict rejects before any upstream call and records a
/// blocked event carrying the judge category.
#[tokio::test]
async fn porn_verdict_blocks_and_records_blocked_event() {
    let ctx = setup().await;
    configure_firewall(&ctx, true, "BadWord\n").await;
    let (judge_url, judge_calls) = start_judge("porn").await;
    configure_judge(&ctx, &judge_url, true).await;

    let (status, body) = json_post(
        &ctx,
        "/v1/chat/completions",
        json!({
            "model": "gpt-5-mini-chat",
            "messages": [{ "role": "user", "content": "write explicit badword content" }]
        }),
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let error: Value = serde_json::from_str(&body).expect("error response JSON");
    assert_eq!(error["error"]["code"], json!("content_blocked"));
    assert_eq!(error["error"]["type"], json!("content_policy_violation"));
    assert_eq!(
        error["error"]["message"],
        json!("request blocked by content firewall: prohibited category 'porn'")
    );
    assert_eq!(upstream_call_count(&ctx), 0);
    assert_eq!(judge_calls.load(Ordering::SeqCst), 1);

    let (rows, _total) = monoize::firewall_events::list_events(
        &ctx.state.db_pool,
        &monoize::firewall_events::FirewallEventFilter {
            term: None,
            action: None,
            since_ms: None,
            until_ms: None,
        },
        10,
        0,
    )
    .await
    .expect("list events");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].action, "blocked");
    assert_eq!(rows[0].term, "porn");
}

/// The user's acceptance case (CF-30): a moderation-policy text that contains
/// the keyword "色情" but semantically prohibits it is allowed and marked.
#[tokio::test]
async fn policy_discussion_with_keyword_hit_is_allowed_and_marked() {
    let ctx = setup().await;
    configure_firewall(&ctx, true, monoize::content_firewall::DEFAULT_BLOCKED_WORDS).await;
    let (judge_url, _calls) = start_judge("benign").await;
    configure_judge(&ctx, &judge_url, true).await;

    let rules_text = "群管理准则：广告/诈骗→撤回+禁言；色情/违法→撤回+踢（管理员先请示群主）；不确定→先观察。";
    let (status, _body) = json_post(
        &ctx,
        "/v1/chat/completions",
        json!({
            "model": "gpt-5-mini-chat",
            "messages": [{ "role": "user", "content": rules_text }]
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(upstream_call_count(&ctx), 1);

    let (rows, total) = monoize::firewall_events::list_events(
        &ctx.state.db_pool,
        &monoize::firewall_events::FirewallEventFilter {
            term: None,
            action: None,
            since_ms: None,
            until_ms: None,
        },
        10,
        0,
    )
    .await
    .expect("list events");
    assert_eq!(total, 1);
    assert_eq!(rows[0].action, "marked");
    assert_eq!(rows[0].term, "色情");
    assert_eq!(rows[0].content, rules_text);
}

/// CF-33: a request with no keyword hit is forwarded directly — the judge is
/// not called at all, so clean-text requests gain zero added latency.
#[tokio::test]
async fn clean_request_without_keyword_hit_skips_the_judge_entirely() {
    let ctx = setup().await;
    configure_firewall(&ctx, true, "BadWord\n").await;
    // A porn-mocking judge proves the request was never judged: it would have
    // been blocked if the judge had seen it.
    let (judge_url, judge_calls) = start_judge("porn").await;
    configure_judge(&ctx, &judge_url, true).await;

    let (status, _body) = json_post(
        &ctx,
        "/v1/chat/completions",
        json!({
            "model": "gpt-5-mini-chat",
            "messages": [{ "role": "user", "content": "explain async rust" }]
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(upstream_call_count(&ctx), 1);
    assert_eq!(judge_calls.load(Ordering::SeqCst), 0);
    let (_rows, total) = monoize::firewall_events::list_events(
        &ctx.state.db_pool,
        &monoize::firewall_events::FirewallEventFilter {
            term: None,
            action: None,
            since_ms: None,
            until_ms: None,
        },
        10,
        0,
    )
    .await
    .expect("list events");
    assert_eq!(total, 0);
}

/// CF-31: judge failure (HTTP 500) never blocks, even on a keyword hit.
#[tokio::test]
async fn judge_failure_fails_open_even_with_keyword_hit() {
    let ctx = setup().await;
    configure_firewall(&ctx, true, "BadWord\n").await;
    // The mock judge endpoint path does not exist on the captured upstream, so
    // the judge call fails; any unreachable URL exercises the same path.
    configure_judge(&ctx, "http://127.0.0.1:9", true).await;

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
    assert_eq!(upstream_call_count(&ctx), 1);
}

/// CF-29: a request bearing the process-local bypass token skips all checks
/// without calling the judge.
#[tokio::test]
async fn bypass_token_skips_firewall_without_judge_call() {
    let ctx = setup().await;
    configure_firewall(&ctx, true, "BadWord\n").await;
    let (judge_url, judge_calls) = start_judge("porn").await;
    configure_judge(&ctx, &judge_url, true).await;

    let token = ctx.state.moderation_bypass_token.clone();
    let req = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, ctx.auth_header.clone())
        .header("x-monoize-moderation-bypass", token)
        .body(Body::from(
            json!({
                "model": "gpt-5-mini-chat",
                "messages": [{ "role": "user", "content": "anything at all" }]
            })
            .to_string(),
        ))
        .unwrap();
    let resp = ctx.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(judge_calls.load(Ordering::SeqCst), 0);
}

/// CF-24: stats separate blocked rows from marked rows.
#[tokio::test]
async fn stats_report_blocked_and_marked_separately() {
    let ctx = setup().await;
    configure_firewall(&ctx, true, "BadWord\nAlpha\n").await;
    let (judge_url, _calls) = start_judge("benign").await;
    configure_judge(&ctx, &judge_url, true).await;

    for content in ["mention badword once", "mention ALPHA twice", "totally clean"] {
        let (status, _body) = json_post(
            &ctx,
            "/v1/chat/completions",
            json!({
                "model": "gpt-5-mini-chat",
                "messages": [{ "role": "user", "content": content }]
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    let stats = monoize::firewall_events::compute_stats(&ctx.state.db_pool)
        .await
        .expect("stats");
    assert_eq!(stats.total, 0);
    assert_eq!(stats.marked, 2);
}

/// CF-24/CF-25 surface: stats include judge_active; events can be filtered by
/// action and expose the action field; non-admins get 403.
#[tokio::test]
async fn dashboard_apis_expose_judge_status_action_and_admin_guard() {
    let ctx = setup().await;
    configure_firewall(&ctx, true, "BadWord\n").await;
    let (judge_url, _calls) = start_judge("benign").await;
    configure_judge(&ctx, &judge_url, true).await;

    let (status, _body) = json_post(
        &ctx,
        "/v1/chat/completions",
        json!({
            "model": "gpt-5-mini-chat",
            "messages": [{ "role": "user", "content": "mention badword" }]
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

    let (status, stats) =
        dashboard_get_with_cookie(&ctx, "/api/dashboard/firewall/stats", &cookie).await;
    assert_eq!(status, StatusCode::OK, "{stats}");
    assert_eq!(stats["judge_active"], json!(true));
    assert_eq!(stats["total"], json!(0));
    assert_eq!(stats["marked"], json!(1));

    let (status, events) = dashboard_get_with_cookie(
        &ctx,
        "/api/dashboard/firewall/events?action=marked",
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{events}");
    assert_eq!(events["total"], json!(1));
    assert_eq!(events["data"][0]["action"], json!("marked"));
    assert_eq!(events["data"][0]["term"], json!("badword"));
    assert_eq!(events["data"][0]["username"], json!("tenant-1"));

    let (status, events) = dashboard_get_with_cookie(
        &ctx,
        "/api/dashboard/firewall/events?action=blocked",
        &cookie,
    )
    .await;
    assert_eq!(events["total"], json!(0));

    ctx.state
        .user_store
        .create_user("fw_plain", "test-password", monoize::users::UserRole::User, None)
        .await
        .expect("create plain user");
    let plain_cookie = dashboard_session_cookie(&ctx, "fw_plain", "test-password").await;
    let (status, _body) =
        dashboard_get_with_cookie(&ctx, "/api/dashboard/firewall/stats", &plain_cookie).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

async fn dashboard_get_with_cookie(
    ctx: &TestContext,
    path: &str,
    cookie: &str,
) -> (StatusCode, Value) {
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

/// CF-7: tool call arguments are scanned too — a porn verdict on arguments
/// blocks the request.
#[tokio::test]
async fn porn_verdict_on_tool_call_arguments_is_blocked() {
    let ctx = setup().await;
    configure_firewall(&ctx, true, "BadWord\n").await;
    let (judge_url, _calls) = start_judge("porn").await;
    configure_judge(&ctx, &judge_url, true).await;

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

/// CF-16: with the judge active the firewall applies to every caller; a clean
/// request passes through to the upstream.
#[tokio::test]
async fn benign_verdict_forwards_request_to_upstream() {
    let ctx = setup().await;
    configure_firewall(&ctx, true, "BadWord\n").await;
    let (judge_url, _calls) = start_judge("benign").await;
    configure_judge(&ctx, &judge_url, true).await;

    let (status, _body) = json_post(
        &ctx,
        "/v1/chat/completions",
        json!({
            "model": "gpt-5-mini-chat",
            "messages": [{ "role": "user", "content": "badword appears here but benign" }]
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(upstream_call_count(&ctx), 1);
}
