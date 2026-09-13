use axum::body::Body;
use axum::http::header::AUTHORIZATION;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use monoize::app::{RuntimeConfig, build_app, load_state_with_runtime};
use monoize::users::{AccountClass, User, UserRole};
use serde_json::{Value, json};
use sea_orm::ConnectionTrait;
use tower::ServiceExt;

/// UA-25 / DH-16 / DH-17a / UA-42: role-scoped analytics and the per-user cache endpoint.
///
/// The scenario seeds request-log rows directly for three users across two groups, then
/// checks that `scope=group` aggregates only the caller's group, that a member cannot use
/// `scope=group`, and that the per-user cache endpoint is super_admin-only.
#[tokio::test]
async fn analytics_group_scope_and_per_user_cache_follow_roles() {
    let mut state = load_state_with_runtime(RuntimeConfig {
        listen: "127.0.0.1:0".to_string(),
        metrics_path: "/metrics".to_string(),
        database_dsn: "sqlite::memory:".to_string(),
        request_log_spool_dir: None,
        node: monoize::node_config::NodeSettings::primary_default(),
    })
    .await
    .expect("state loads");

    let admin = state
        .user_store
        .create_user("admin_cache_scope", "password", UserRole::Admin, None)
        .await
        .expect("admin created");
    let member = state
        .user_store
        .create_user("member_cache_scope", "password", UserRole::User, None)
        .await
        .expect("member created");
    let super_admin = state
        .user_store
        .create_user("root_cache_scope", "password", UserRole::SuperAdmin, None)
        .await
        .expect("super admin created");

    // The admin and member share a group; the super admin must sit in a different one, so
    // create a second group when the registry only offers the default.
    let groups = state.user_store.list_groups().await.expect("groups listed");
    let shared_group = groups
        .iter()
        .find(|g| g.account_class == AccountClass::Standard)
        .map(|g| g.id.clone())
        .expect("a standard group exists");
    let other_group = state
        .user_store
        .create_group(monoize::users::CreateGroupInput {
            name: "cache-scope-island".to_string(),
            confirm_public_exposure: true,
            description: String::new(),
            user_selectable: false,
            sort_order: 0,
            account_class: AccountClass::Standard,
        })
        .await
        .expect("island group created")
        .id;
    state
        .user_store
        .update_user(&super_admin.id, None, None, None, None, None, None, None, Some(&other_group))
        .await
        .expect("super admin moved into the island group");
    for user in [&admin, &member] {
        state
            .user_store
            .update_user(
                &user.id,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(&shared_group),
            )
            .await
            .expect("user moved into the shared group");
    }
    let admin: User = state
        .user_store
        .get_user_by_id(&admin.id)
        .await
        .expect("reload admin")
        .expect("admin exists");
    let member: User = state
        .user_store
        .get_user_by_id(&member.id)
        .await
        .expect("reload member")
        .expect("member exists");

    let now_ms = chrono::Utc::now().timestamp_millis();
    for (user_id, input, cache_read) in [
        (&admin.id, 600_000i64, 300_000i64),
        (&member.id, 200_000i64, 100_000i64),
        (&super_admin.id, 100_000i64, 0i64),
    ] {
        state
            .db_pool
            .write()
            .await
            .execute(sea_orm::Statement::from_sql_and_values(
                state.db_pool.read().get_database_backend(),
                "INSERT INTO request_logs (id, user_id, model, input_tokens, cache_read_tokens, \
                 output_tokens, charge_nano_usd, created_at, created_at_unix_ms, is_stream, status) \
                 VALUES ($1, $2, 'gpt-cache-test', $3, $4, 0, '0', $5, $6, 0, 'success')",
                [
                    uuid::Uuid::new_v4().to_string().into(),
                    user_id.clone().into(),
                    input.into(),
                    cache_read.into(),
                    chrono::Utc::now().to_rfc3339().into(),
                    now_ms.into(),
                ],
            ))
            .await
            .expect("seed request log row");
    }

    let admin_session = state.user_store.create_session(&admin.id, 7).await.expect("admin session");
    let member_session = state.user_store.create_session(&member.id, 7).await.expect("member session");
    let root_session = state.user_store.create_session(&super_admin.id, 7).await.expect("root session");
    let admin_auth = format!("Bearer {}", admin_session.token);
    let member_auth = format!("Bearer {}", member_session.token);
    let root_auth = format!("Bearer {}", root_session.token);
    let router = build_app(state.clone());

    // DH-17a: scope=group aggregates only rows of users in the caller's group.
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/dashboard/analytics?buckets=1&range_hours=24&scope=group")
                .header(AUTHORIZATION, admin_auth.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["total_input_tokens"], json!("800000"));
    assert_eq!(body["total_cache_read_tokens"], json!("400000"));

    // DH-16: a member sending scope=group is rejected with 400.
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/dashboard/analytics?buckets=1&range_hours=24&scope=group")
                .header(AUTHORIZATION, member_auth.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // UA-42: the per-user cache endpoint is super_admin-only and ranks by input volume.
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/dashboard/usage/cache/users?range_hours=24")
                .header(AUTHORIZATION, admin_auth.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/dashboard/usage/cache/users?range_hours=24")
                .header(AUTHORIZATION, root_auth.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let users = body["users"].as_array().expect("users array");
    assert_eq!(users.len(), 3);
    assert_eq!(users[0]["username"], json!("admin_cache_scope"));
    assert_eq!(users[0]["input_tokens"], json!("600000"));
    assert_eq!(users[0]["cache_read_tokens"], json!("300000"));
    assert_eq!(users[2]["username"], json!("root_cache_scope"));
}
