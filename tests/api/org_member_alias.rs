use axum::body::Body;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, ORIGIN};
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use monoize::app::{RuntimeConfig, build_app, load_state_with_runtime};
use monoize::users::UserRole;
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};
use tower::ServiceExt;

const ORG_ID: &str = "org-alias-test";

async fn put_alias(
    router: &axum::Router,
    token: &str,
    member_id: &str,
    alias: Value,
) -> (StatusCode, Value) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/dashboard/orgs/{ORG_ID}/members/{member_id}/alias"))
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .header(CONTENT_TYPE, "application/json")
                .header(ORIGIN, "https://lynshen.org")
                .body(Body::from(json!({ "alias": alias }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    (status, body)
}

/// ORG-3a: alias set/clear semantics, length validation, and the owner-only
/// rule for setting another member's alias.
#[tokio::test]
async fn org_member_alias_set_clear_and_authorization() {
    let state = load_state_with_runtime(RuntimeConfig {
        listen: "127.0.0.1:0".to_string(),
        metrics_path: "/metrics".to_string(),
        database_dsn: "sqlite::memory:".to_string(),
        request_log_spool_dir: None,
        node: monoize::node_config::NodeSettings::primary_default(),
    })
    .await
    .expect("state loads");

    let owner = state
        .user_store
        .create_user("alias_owner", "password123", UserRole::User, None)
        .await
        .expect("owner creates");
    let member = state
        .user_store
        .create_user("alias_member", "password123", UserRole::User, None)
        .await
        .expect("member creates");
    let outsider = state
        .user_store
        .create_user("alias_outsider", "password123", UserRole::User, None)
        .await
        .expect("outsider creates");

    let now = chrono::Utc::now().to_rfc3339();
    let backend = state.db_pool.read().get_database_backend();
    {
        let write = state.db_pool.write().await;
        // org_detail requires the org wallet user row (users.id = org id, is_org = 1).
        let default_group = state.user_store.default_group_id().await.expect("default group");
        write
            .execute(sea_orm::Statement::from_sql_and_values(
                backend,
                "INSERT INTO users (id, username, password_hash, role, created_at, updated_at, enabled,
                                    balance_nano_usd, balance_unlimited, group_id, is_org)
                 VALUES ($1, $1, '', 'user', $2, $2, 1, '0', 0, $3, 1)",
                [ORG_ID.into(), now.clone().into(), default_group.into()],
            ))
            .await
            .expect("wallet seeds");
        write
            .execute(sea_orm::Statement::from_sql_and_values(
                backend,
                "INSERT INTO orgs (id, owner_user_id, display_name, avatar_emoji, avatar_color,
                                   avatar_image, invite_token, invite_code, invite_created_at, created_at, updated_at)
                 VALUES ($1, $2, 'Alias Org', 'A', '#112233', NULL, 'tok-alias-org', 'ABC123', $3, $3, $3)",
                [ORG_ID.into(), owner.id.clone().into(), now.clone().into()],
            ))
            .await
            .expect("org seeds");
        for (uid, role) in [(&owner.id, "owner"), (&member.id, "member")] {
            write
                .execute(sea_orm::Statement::from_sql_and_values(
                    backend,
                    "INSERT INTO org_members (org_id, user_id, role, joined_at) VALUES ($1, $2, $3, $4)",
                    [ORG_ID.into(), uid.clone().into(), role.into(), now.clone().into()],
                ))
                .await
                .expect("member seeds");
        }
    }

    let owner_session = state.user_store.create_session(&owner.id, 7).await.expect("owner session");
    let member_session = state.user_store.create_session(&member.id, 7).await.expect("member session");
    let outsider_session = state.user_store.create_session(&outsider.id, 7).await.expect("outsider session");
    let router = build_app(state.clone());

    // Owner sets a member alias.
    let (status, body) = put_alias(&router, &owner_session.token, &member.id, json!("大雄")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["alias"], json!("大雄"));
    assert_eq!(body["username"], json!("alias_member"));

    // Detail lists the alias for every member.
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/dashboard/orgs/{ORG_ID}"))
                .header(AUTHORIZATION, format!("Bearer {}", member_session.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&bytes));
    let detail: Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| json!({"raw": String::from_utf8_lossy(&bytes).to_string()}));
    let member_row = detail["members"]
        .as_array()
        .expect("members")
        .iter()
        .find(|row| row["username"] == json!("alias_member"))
        .expect("member row");
    assert_eq!(member_row["alias"], json!("大雄"));

    // Member clears their own alias with a blank string.
    let (status, body) = put_alias(&router, &member_session.token, &member.id, json!("   ")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["alias"], Value::Null);

    // Length validation: 33 characters is rejected.
    let (status, _) = put_alias(&router, &owner_session.token, &member.id, json!("x".repeat(33))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // A plain member cannot set another member's alias.
    let (status, _) = put_alias(&router, &member_session.token, &owner.id, json!("nope")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // A non-member is rejected with 404.
    let (status, _) = put_alias(&router, &outsider_session.token, &member.id, json!("nope")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
