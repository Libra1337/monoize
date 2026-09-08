async fn start_test_cap_verifier() -> monoize::captcha::CapVerifier {
    async fn siteverify(Json(body): Json<Value>) -> Json<Value> {
        Json(json!({
            "success": body["secret"] == json!("test-cap-secret")
                && body["response"] == json!("test-captcha-token")
        }))
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test Cap server");
    let address = listener.local_addr().expect("test Cap server address");
    tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/site-key/siteverify", post(siteverify)),
        )
        .await
        .expect("serve test Cap endpoint");
    });
    monoize::captcha::CapVerifier::configured(
        &format!("http://{address}/site-key/"),
        "test-cap-secret".to_string(),
    )
    .expect("configure test Cap verifier")
}
