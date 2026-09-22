use axum::body::Body;
use axum::http::header::AUTHORIZATION;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use monoize::app::{RuntimeConfig, build_app, load_state_with_runtime};
use monoize::users::UserRole;
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};
use tower::ServiceExt;

/// RL-API15: active health-probe rows MUST NOT appear in the dashboard request-log
/// list for any role, and the admin `username` filter must not re-admit them.
#[tokio::test]
async fn request_log_list_excludes_active_probe_rows_for_every_role() {
    let mut state = load_state_with_runtime(RuntimeConfig {
        listen: "127.0.0.1:0".to_string(),
        metrics_path: "/metrics".to_string(),
        database_dsn: "sqlite::memory:".to_string(),
        request_log_spool_dir: None,
        node: monoize::node_config::NodeSettings::primary_default(),
    })
    .await
    .expect("load state");

    let admin = state
        .user_store
        .create_user("probe_admin", "password123", UserRole::SuperAdmin, None)
        .await
        .expect("create admin");
    let member = state
        .user_store
        .create_user("probe_member", "password123", UserRole::User, None)
        .await
        .expect("create member");

    let now_ms = chrono::Utc::now().timestamp_millis();
    for (id, user_id, kind, model) in [
        (
            "log-customer",
            member.id.as_str(),
            None::<&str>,
            "customer-model",
        ),
        ("log-probe", admin.id.as_str(), Some("active_probe_connectivity"), "probe-model"),
    ] {
        state
            .db_pool
            .write()
            .await
            .execute(sea_orm::Statement::from_sql_and_values(
                state.db_pool.read().get_database_backend(),
                "INSERT INTO request_logs (id, user_id, model, input_tokens, cache_read_tokens, \
                 output_tokens, charge_nano_usd, created_at, created_at_unix_ms, is_stream, status, request_kind) \
                 VALUES ($1, $2, $3, 1, 0, 1, '0', $4, $5, 0, 'success', $6)",
                [
                    id.into(),
                    user_id.into(),
                    model.into(),
                    chrono::Utc::now().to_rfc3339().into(),
                    now_ms.into(),
                    kind.into(),
                ],
            ))
            .await
            .expect("seed request log row");
    }

    let admin_session = state
        .user_store
        .create_session(&admin.id, 7)
        .await
        .expect("admin session");
    let member_session = state
        .user_store
        .create_session(&member.id, 7)
        .await
        .expect("member session");
    let router = build_app(state.clone());

    // Super admin without filters: only the customer row appears.
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/dashboard/request-logs?limit=50")
                .header(AUTHORIZATION, format!("Bearer {}", admin_session.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let models: Vec<&str> = body["data"]
        .as_array()
        .expect("data array")
        .iter()
        .map(|row| row["model"].as_str().expect("model"))
        .collect();
    assert!(models.contains(&"customer-model"), "customer row missing: {models:?}");
    assert!(!models.contains(&"probe-model"), "probe row leaked to list: {models:?}");

    // Admin username filter on the admin's own name must not re-admit the probe row.
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/dashboard/request-logs?username=probe_admin")
                .header(AUTHORIZATION, format!("Bearer {}", admin_session.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["total"], json!(0), "probe row matched the username filter");

    // Member sees only its own customer row.
    let resp = router
        .oneshot(
            Request::builder()
                .uri("/api/dashboard/request-logs?limit=50")
                .header(AUTHORIZATION, format!("Bearer {}", member_session.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["total"], json!(1));
}
