use axum::body::Body;
use axum::http::header::AUTHORIZATION;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use monoize::app::{RuntimeConfig, build_app, load_state_with_runtime};
use monoize::users::UserRole;
use sea_orm::ConnectionTrait;
use serde_json::Value;
use tower::ServiceExt;

async fn get_json(router: &axum::Router, path: &str, session_token: &str) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(path)
                .header(AUTHORIZATION, format!("Bearer {session_token}"))
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("request completes");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body reads")
        .to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

#[tokio::test]
async fn api_key_analytics_is_key_scoped_exact_and_owner_protected() {
    let state = load_state_with_runtime(RuntimeConfig::with_defaults(
        "127.0.0.1:0",
        "/metrics",
        "sqlite::memory:".to_string(),
    ))
    .await
    .expect("state loads");
    let owner = state
        .user_store
        .create_user("analytics_owner", "password123", UserRole::User, None)
        .await
        .expect("owner creates");
    let stranger = state
        .user_store
        .create_user("analytics_stranger", "password123", UserRole::User, None)
        .await
        .expect("stranger creates");
    let (key, _) = state
        .user_store
        .create_api_key(&owner.id, "production", None)
        .await
        .expect("key creates");
    let (other_key, _) = state
        .user_store
        .create_api_key(&owner.id, "excluded", None)
        .await
        .expect("other key creates");
    let now = chrono::Utc::now();
    for (id, api_key_id, model, input, cache, output, charge) in [
        (
            "analytics-1",
            key.id.as_str(),
            "model-b",
            100_i64,
            40_i64,
            20_i64,
            "17",
        ),
        (
            "analytics-2",
            key.id.as_str(),
            "model-a",
            200_i64,
            80_i64,
            30_i64,
            "23",
        ),
        (
            "analytics-3",
            other_key.id.as_str(),
            "excluded",
            999_i64,
            999_i64,
            999_i64,
            "999",
        ),
    ] {
        state
            .db_pool
            .write()
            .await
            .execute(state.db_pool.stmt(
                "INSERT INTO request_logs (id, user_id, api_key_id, model, is_stream, input_tokens, cache_read_tokens, output_tokens, charge_nano_usd, status, created_at, created_at_unix_ms) VALUES ($1, $2, $3, $4, 0, $5, $6, $7, $8, 'success', $9, $10)",
                vec![
                    id.into(),
                    owner.id.clone().into(),
                    api_key_id.into(),
                    model.into(),
                    input.into(),
                    cache.into(),
                    output.into(),
                    charge.into(),
                    now.to_rfc3339().into(),
                    now.timestamp_millis().into(),
                ],
            ))
            .await
            .expect("request log inserts");
    }
    let owner_session = state
        .user_store
        .create_session(&owner.id, 7)
        .await
        .expect("owner session creates");
    let stranger_session = state
        .user_store
        .create_session(&stranger.id, 7)
        .await
        .expect("stranger session creates");
    let router = build_app(state);

    let path = format!("/api/dashboard/tokens/{}/analytics?range=24h", key.id);
    let (status, body) = get_json(&router, &path, &owner_session.token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["range"], "24h");
    assert_eq!(body["total_input_tokens"], "300");
    assert_eq!(body["total_cache_read_tokens"], "120");
    assert_eq!(body["total_output_tokens"], "50");
    assert_eq!(body["total_tokens"], "350");
    assert_eq!(body["request_count"], 2);
    assert_eq!(body["consumed_coin_nano"], "40");
    assert_eq!(body["balance_mode"], "wallet");
    assert!(body["independent_balance_nano"].is_null());
    assert_eq!(body["models"][0]["model"], "model-a");
    assert_eq!(body["models"][0]["total_tokens"], "230");
    assert_eq!(body["models"][1]["model"], "model-b");
    assert_eq!(body["trend"].as_array().expect("trend array").len(), 24);

    let (status, _) = get_json(&router, &path, &stranger_session.token).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn api_key_analytics_rejects_unknown_range() {
    let state = load_state_with_runtime(RuntimeConfig::with_defaults(
        "127.0.0.1:0",
        "/metrics",
        "sqlite::memory:".to_string(),
    ))
    .await
    .expect("state loads");
    let user = state
        .user_store
        .create_user("analytics_range", "password123", UserRole::User, None)
        .await
        .expect("user creates");
    let (key, _) = state
        .user_store
        .create_api_key(&user.id, "range", None)
        .await
        .expect("key creates");
    let session = state
        .user_store
        .create_session(&user.id, 7)
        .await
        .expect("session creates");
    let router = build_app(state);
    let path = format!("/api/dashboard/tokens/{}/analytics?range=year", key.id);
    let (status, _) = get_json(&router, &path, &session.token).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
