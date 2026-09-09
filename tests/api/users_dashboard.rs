use axum::Json;
use axum::body::Body;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use monoize::app::{RuntimeConfig, build_app, load_state_with_runtime};
use monoize::users::UserRole;
use serde_json::{Value, json};
use tower::ServiceExt;

struct TestContext {
    router: axum::Router,
    auth_header: String,
}

async fn setup() -> TestContext {
    async fn siteverify(Json(body): Json<Value>) -> Json<Value> {
        Json(json!({
            "success": body["secret"] == json!("test-cap-secret")
                && body["response"] == json!("test-captcha-token")
        }))
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test Cap server");
    let address = listener.local_addr().expect("test Cap address");
    tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/site-key/siteverify", axum::routing::post(siteverify)),
        )
        .await
        .expect("serve test Cap endpoint");
    });

    let mut state = load_state_with_runtime(RuntimeConfig {
        listen: "127.0.0.1:0".to_string(),
        metrics_path: "/metrics".to_string(),
        database_dsn: "sqlite::memory:".to_string(),
        request_log_spool_dir: None,
        node: monoize::node_config::NodeSettings::primary_default(),
    })
    .await
    .expect("state loads");
    state.cap_verifier = monoize::captcha::CapVerifier::configured(
        &format!("http://{address}/site-key/"),
        "test-cap-secret".to_string(),
    )
    .expect("configure test Cap verifier");
    let admin = state
        .user_store
        .create_user("admin_users_dashboard", "password", UserRole::Admin, None)
        .await
        .expect("admin created");
    let session = state
        .user_store
        .create_session(&admin.id, 7)
        .await
        .expect("session created");

    TestContext {
        router: build_app(state),
        auth_header: format!("Bearer {}", session.token),
    }
}

async fn json_request(
    ctx: &TestContext,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(AUTHORIZATION, ctx.auth_header.clone());
    let body = if let Some(body) = body {
        builder = builder.header(CONTENT_TYPE, "application/json");
        Body::from(body.to_string())
    } else {
        Body::empty()
    };
    let resp = ctx
        .router
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}));
    (status, value)
}

/// DL-UM2: the Admin user list must mark sales agents.
///
/// The list groups agents separately from ordinary users, and it can only do that from this
/// flag. `UserResponse::from_user` hardcodes it to false, so a list built through that
/// constructor reports every account as a non-agent and the sales grouping renders empty
/// while the agents sit hidden among the standard users.
#[tokio::test]
async fn the_user_list_marks_sales_agents() {
    let ctx = setup().await;

    let (status, plain) = json_request(
        &ctx,
        Method::POST,
        "/api/dashboard/users",
        Some(json!({ "username": "not_an_agent", "password": "password" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let plain_id = plain["id"].as_str().expect("user id").to_string();

    let (status, created) = json_request(
        &ctx,
        Method::POST,
        "/api/dashboard/store/admin/sales/agents",
        Some(json!({ "discount_bp": 0 })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let agent_id = created["agent"]["user_id"]
        .as_str()
        .expect("agent user id")
        .to_string();

    let (status, users) = json_request(&ctx, Method::GET, "/api/dashboard/users", None).await;
    assert_eq!(status, StatusCode::OK);
    let users = users.as_array().expect("users is an array");

    let agent = users
        .iter()
        .find(|user| user["id"] == json!(agent_id))
        .expect("the agent account is listed");
    assert_eq!(
        agent["is_sales_agent"],
        json!(true),
        "an agent must be marked, or the sales grouping is empty"
    );
    // The agent is a standard-class account: the flag is what separates it, not its class.
    assert_eq!(agent["account_class"], json!("standard"));

    let plain_user = users
        .iter()
        .find(|user| user["id"] == json!(plain_id))
        .expect("the ordinary user is listed");
    assert_eq!(
        plain_user["is_sales_agent"],
        json!(false),
        "an ordinary user must not be grouped as an agent"
    );
}
