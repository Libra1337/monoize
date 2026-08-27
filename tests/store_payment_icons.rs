use axum::body::Body;
use axum::http::header::{AUTHORIZATION, CONTENT_SECURITY_POLICY, CONTENT_TYPE};
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use monoize::app::{AppState, RuntimeConfig, build_app, load_state_with_runtime};
use monoize::users::UserRole;
use tower::ServiceExt;

async fn test_state() -> AppState {
    load_state_with_runtime(RuntimeConfig::with_defaults(
        "127.0.0.1:0",
        "/metrics",
        "sqlite::memory:".to_string(),
    ))
    .await
    .expect("test state loads")
}

async fn session(state: &AppState, username: &str, role: UserRole) -> String {
    let user = state
        .user_store
        .create_user(username, "password123", role, None)
        .await
        .expect("user creates");
    state
        .user_store
        .create_session(&user.id, 7)
        .await
        .expect("session creates")
        .token
}

fn multipart_file(content: &[u8], declared_type: &str) -> (String, Vec<u8>) {
    let boundary = "monoize-icon-boundary";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"icon.bin\"\r\nContent-Type: {declared_type}\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(content);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

fn multipart_fields(fields: &[(&str, &[u8])]) -> (String, Vec<u8>) {
    let boundary = "monoize-icon-boundary";
    let mut body = Vec::new();
    for (name, content) in fields {
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(content);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

async fn upload_fields(
    router: &axum::Router,
    token: &str,
    fields: &[(&str, &[u8])],
) -> axum::response::Response {
    let (content_type, body) = multipart_fields(fields);
    router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/dashboard/store/admin/icons")
                .header(CONTENT_TYPE, content_type)
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(body))
                .expect("request builds"),
        )
        .await
        .expect("request completes")
}

async fn assert_invalid_icon(response: axum::response::Response) {
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload: serde_json::Value = serde_json::from_slice(
        &response
            .into_body()
            .collect()
            .await
            .expect("read error body")
            .to_bytes(),
    )
    .expect("error JSON");
    assert_eq!(payload["error"]["code"], "invalid_icon");
}

async fn upload(
    router: &axum::Router,
    token: Option<&str>,
    content: &[u8],
    declared_type: &str,
) -> axum::response::Response {
    let (content_type, body) = multipart_file(content, declared_type);
    let mut request = Request::builder()
        .method(Method::POST)
        .uri("/api/dashboard/store/admin/icons")
        .header(CONTENT_TYPE, content_type);
    if let Some(token) = token {
        request = request.header(AUTHORIZATION, format!("Bearer {token}"));
    }
    router
        .clone()
        .oneshot(request.body(Body::from(body)).expect("request builds"))
        .await
        .expect("request completes")
}

#[tokio::test]
async fn icon_upload_requires_admin_and_validates_bytes_and_size() {
    let state = test_state().await;
    let admin = session(&state, "icon_admin", UserRole::Admin).await;
    let user = session(&state, "icon_user", UserRole::User).await;
    let router = build_app(state);
    let png = b"\x89PNG\r\n\x1a\nexact";

    assert_eq!(
        upload(&router, Some(&user), png, "image/png")
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_invalid_icon(upload(&router, Some(&admin), b"not an image", "image/png").await).await;
    assert_invalid_icon(
        upload(
            &router,
            Some(&admin),
            &vec![0_u8; 2 * 1024 * 1024 + 1],
            "image/png",
        )
        .await,
    )
    .await;
    assert_invalid_icon(upload_fields(&router, &admin, &[]).await).await;
    assert_invalid_icon(upload_fields(&router, &admin, &[("other", png)]).await).await;
    assert_invalid_icon(upload_fields(&router, &admin, &[("file", png), ("file", png)]).await)
        .await;
}

#[tokio::test]
async fn authenticated_icon_get_round_trips_exact_bytes_and_security_headers() {
    let state = test_state().await;
    let admin = session(&state, "roundtrip_admin", UserRole::Admin).await;
    let user = session(&state, "roundtrip_user", UserRole::User).await;
    let router = build_app(state);
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"><path d=\"M0 0\"/></svg>";

    let response = upload(&router, Some(&admin), svg, "application/octet-stream").await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let payload: serde_json::Value = serde_json::from_slice(
        &response
            .into_body()
            .collect()
            .await
            .expect("read upload body")
            .to_bytes(),
    )
    .expect("upload JSON");
    let url = payload["url"].as_str().expect("icon URL");
    assert!(url.starts_with("/api/dashboard/store/icons/"));

    let anonymous = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(url)
                .body(Body::empty())
                .expect("anonymous request builds"),
        )
        .await
        .expect("anonymous request completes");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    let missing = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/dashboard/store/icons/missing")
                .header(AUTHORIZATION, format!("Bearer {user}"))
                .body(Body::empty())
                .expect("missing icon request builds"),
        )
        .await
        .expect("missing icon request completes");
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(url)
                .header(AUTHORIZATION, format!("Bearer {user}"))
                .body(Body::empty())
                .expect("GET request builds"),
        )
        .await
        .expect("GET request completes");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], "image/svg+xml");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert!(
        response.headers()[CONTENT_SECURITY_POLICY]
            .to_str()
            .expect("CSP header text")
            .contains("sandbox")
    );
    assert_eq!(
        response
            .into_body()
            .collect()
            .await
            .expect("read icon body")
            .to_bytes()
            .as_ref(),
        svg
    );
}
