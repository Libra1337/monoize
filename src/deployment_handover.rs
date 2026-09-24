use crate::app::AppState;
use axum::body::Body;
use axum::extract::{OriginalUri, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header::AUTHORIZATION};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tokio::sync::{Notify, OwnedRwLockReadGuard, RwLock};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Forwarding,
    Paused,
    Local,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Forwarding => "forwarding",
            Self::Paused => "paused",
            Self::Local => "local",
        }
    }
}

pub struct DeploymentHandover {
    origin: String,
    token_digest: [u8; 32],
    client: reqwest::Client,
    mode: Arc<RwLock<Mode>>,
    changed: Notify,
}

impl DeploymentHandover {
    pub fn from_env(is_primary: bool, listen: &str) -> Result<Option<Self>, String> {
        fn variable(name: &str) -> Result<Option<String>, String> {
            match std::env::var(name) {
                Ok(value) => Ok(Some(value)),
                Err(std::env::VarError::NotPresent) => Ok(None),
                Err(_) => Err(format!("{name} must be Unicode")),
            }
        }
        Self::from_values(
            variable("MONOIZE_DEPLOYMENT_PREVIOUS_URL")?.as_deref(),
            variable("MONOIZE_DEPLOYMENT_CONTROL_TOKEN")?.as_deref(),
            is_primary,
            variable("MONOIZE_BOOT_STANDBY_LEASE")?.as_deref() == Some("1"),
            listen,
        )
    }

    fn from_values(
        previous: Option<&str>,
        token: Option<&str>,
        is_primary: bool,
        standby: bool,
        listen: &str,
    ) -> Result<Option<Self>, String> {
        let (previous, token) = match (previous, token) {
            (None, None) => return Ok(None),
            (Some(previous), Some(token)) => (previous, token),
            _ => {
                return Err(
                    "deployment previous URL and control token must be configured together".into(),
                );
            }
        };
        if !is_primary || !standby || token.chars().count() < 32 {
            return Err("deployment forwarding requires a standby Primary and a control token of at least 32 characters".into());
        }
        let url = url::Url::parse(previous).map_err(|_| "invalid deployment previous URL")?;
        let raw_uri: axum::http::Uri = previous
            .parse()
            .map_err(|_| "invalid deployment previous origin")?;
        let raw_host = raw_uri
            .host()
            .unwrap_or("")
            .trim_start_matches('[')
            .trim_end_matches(']');
        let raw_loopback = raw_host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
        let raw_origin_path = raw_uri
            .path_and_query()
            .map(|value| value.as_str())
            .unwrap_or("/");
        let loopback = match url.host() {
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            _ => false,
        };
        let listen: std::net::SocketAddr =
            listen.parse().map_err(|_| "invalid deployment listener")?;
        if url.scheme() != "http"
            || !loopback
            || !raw_loopback
            || raw_origin_path != "/"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || url.port_or_known_default() == Some(listen.port())
        {
            return Err(
                "deployment previous URL must be an HTTP loopback origin on a different port"
                    .into(),
            );
        }
        crate::node_config::ensure_rustls_crypto_provider()?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .pool_max_idle_per_host(0)
            .build()
            .map_err(|error| format!("deployment HTTP client: {error}"))?;
        Ok(Some(Self {
            origin: url.origin().ascii_serialization(),
            token_digest: Sha256::digest(format!("Bearer {token}").as_bytes()).into(),
            client,
            mode: Arc::new(RwLock::new(Mode::Forwarding)),
            changed: Notify::new(),
        }))
    }

    async fn admit(&self) -> Option<OwnedRwLockReadGuard<Mode>> {
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let guard = self.mode.clone().read_owned().await;
            match *guard {
                Mode::Forwarding => return Some(guard),
                Mode::Local => return None,
                Mode::Paused => drop(guard),
            }
            notified.await;
        }
    }

    async fn pause(&self) -> Result<(), ()> {
        let mut mode = self.mode.write().await;
        if *mode == Mode::Local {
            return Err(());
        }
        *mode = Mode::Paused;
        Ok(())
    }

    async fn resume_with_lease_slot(
        &self,
        slot: &crate::store_billing::availability::StorePrimaryLeaseSlot,
    ) -> Result<(), ()> {
        let mut mode = self.mode.write().await;
        if *mode == Mode::Local || slot.get().await.is_some() {
            return Err(());
        }
        *mode = Mode::Forwarding;
        self.changed.notify_waiters();
        Ok(())
    }

    pub async fn activate_local(&self) {
        let mut mode = self.mode.write().await;
        *mode = Mode::Local;
        self.changed.notify_waiters();
    }

    fn authorized(&self, headers: &HeaderMap) -> bool {
        let mut values = headers.get_all(AUTHORIZATION).iter();
        let Some(value) = values.next() else {
            return false;
        };
        if values.next().is_some() {
            return false;
        }
        let digest: [u8; 32] = Sha256::digest(value.as_bytes()).into();
        bool::from(digest.ct_eq(&self.token_digest))
    }

    async fn forward(&self, request: Request, guard: OwnedRwLockReadGuard<Mode>) -> Response {
        let uri = request
            .extensions()
            .get::<OriginalUri>()
            .map(|original| &original.0)
            .unwrap_or(request.uri());
        let path = uri
            .path_and_query()
            .map(|value| value.as_str())
            .unwrap_or("/");
        let Ok(url) = url::Url::parse(&format!("{}{path}", self.origin)) else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        if &url[url::Position::BeforePath..] != path {
            return StatusCode::BAD_REQUEST.into_response();
        }
        let canonical = crate::client_ip::canonical_client_ip_from_headers(request.headers());
        let (parts, body) = request.into_parts();
        let mut headers = parts.headers;
        strip_hop_headers(&mut headers);
        let identity_names: Vec<_> = headers
            .keys()
            .filter(|name| {
                let name = name.as_str();
                name == "forwarded"
                    || name.starts_with("x-forwarded-")
                    || name == "x-real-ip"
                    || name == "x-monoize-client-ip"
            })
            .cloned()
            .collect();
        for name in identity_names {
            headers.remove(name);
        }
        if let Some(canonical) = canonical {
            headers.insert(
                "x-forwarded-for",
                HeaderValue::from_str(&canonical.to_string())
                    .expect("IP address is a header value"),
            );
        }
        let response = match self
            .client
            .request(parts.method, url)
            .headers(headers)
            .body(reqwest::Body::wrap_stream(body.into_data_stream()))
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!(%error, "deployment forwarding failed without retry");
                return StatusCode::BAD_GATEWAY.into_response();
            }
        };
        let status = response.status();
        let mut headers = response.headers().clone();
        strip_hop_headers(&mut headers);
        // The response stream, not its headers, owns the admission. Releasing
        // on the first error also covers consumers that stop polling afterward.
        let stream = futures_util::stream::unfold(
            (response.bytes_stream(), Some(guard)),
            |(mut stream, mut guard)| async move {
                match stream.next().await {
                    Some(item) => {
                        if item.is_err() {
                            guard.take();
                        }
                        Some((item, (stream, guard)))
                    }
                    None => None,
                }
            },
        );
        let mut response = Response::new(Body::from_stream(stream));
        *response.status_mut() = status;
        *response.headers_mut() = headers;
        response
    }
}

fn strip_hop_headers(headers: &mut HeaderMap) {
    let named: Vec<String> = headers
        .get_all("connection")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(',').map(|name| name.trim().to_owned()))
        .collect();
    for name in named {
        headers.remove(name);
    }
    for name in [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    ] {
        headers.remove(name);
    }
}

pub async fn forwarding_middleware(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    if let Some(deployment) = state.deployment_handover.as_ref() {
        if let Some(guard) = deployment.admit().await {
            return deployment.forward(request, guard).await;
        }
    }
    next.run(request).await
}

pub fn control_router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/internal/deployment/pause", post(pause_control))
        .route("/internal/deployment/resume", post(resume_control))
        .route("/internal/deployment/status", get(status_control))
}

async fn control(state: AppState, headers: HeaderMap, action: &str) -> Response {
    let Some(deployment) = state.deployment_handover.as_ref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let status = if !deployment.authorized(&headers) {
        StatusCode::UNAUTHORIZED
    } else {
        let result = match action {
            "pause" => deployment.pause().await,
            "resume" => {
                deployment
                    .resume_with_lease_slot(&state.store_primary_lease)
                    .await
            }
            _ => Ok(()),
        };
        if result.is_ok() {
            StatusCode::OK
        } else {
            StatusCode::CONFLICT
        }
    };
    let mut response = if status == StatusCode::UNAUTHORIZED {
        status.into_response()
    } else {
        let mode = deployment.mode.read().await.as_str();
        let lease_owned = state.validate_store_primary_lease().await.is_ok();
        (
            status,
            axum::Json(serde_json::json!({ "mode": mode, "lease_owned": lease_owned })),
        )
            .into_response()
    };
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response
}

async fn pause_control(State(state): State<AppState>, headers: HeaderMap) -> Response {
    control(state, headers, "pause").await
}
async fn resume_control(State(state): State<AppState>, headers: HeaderMap) -> Response {
    control(state, headers, "resume").await
}
async fn status_control(State(state): State<AppState>, headers: HeaderMap) -> Response {
    control(state, headers, "status").await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::to_bytes, routing::any};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::oneshot;

    const TOKEN: &str = "deployment-test-secret-at-least-32-characters";

    fn handover(origin: &str) -> Arc<DeploymentHandover> {
        Arc::new(
            DeploymentHandover::from_values(Some(origin), Some(TOKEN), true, true, "127.0.0.1:1")
                .unwrap()
                .unwrap(),
        )
    }

    async fn server(router: Router) -> (String, tokio::task::JoinHandle<()>) {
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", socket.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(socket, router).await.unwrap() });
        (origin, task)
    }

    #[test]
    fn configuration_rejects_nonlocal_or_ambiguous_forwarding() {
        for origin in [
            "https://127.0.0.1:8080",
            "http://localhost:8080",
            "http://192.0.2.1:8080",
            "http://user@127.0.0.1:8080",
            "http://127.0.0.1:8080/path",
            "http://127.1:8080",
            "http://127.0.0.1:8080/path/..",
            "http://127.0.0.1:8080/?a=1",
            "http://127.0.0.1:8080/#fragment",
            "http://127.0.0.1:1",
        ] {
            assert!(
                DeploymentHandover::from_values(
                    Some(origin),
                    Some(TOKEN),
                    true,
                    true,
                    "127.0.0.1:1"
                )
                .is_err(),
                "{origin}"
            );
        }
        assert!(
            DeploymentHandover::from_values(None, Some(TOKEN), true, true, "127.0.0.1:1").is_err()
        );
        assert!(
            DeploymentHandover::from_values(
                Some("http://127.0.0.1:8080"),
                Some("short"),
                true,
                true,
                "127.0.0.1:1"
            )
            .is_err()
        );
        assert!(
            DeploymentHandover::from_values(
                Some("http://127.0.0.1:8080"),
                Some(TOKEN),
                false,
                true,
                "127.0.0.1:1"
            )
            .is_err()
        );
        assert!(
            DeploymentHandover::from_values(
                Some("http://127.0.0.1:8080"),
                Some(TOKEN),
                true,
                false,
                "127.0.0.1:1"
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn forwarding_preserves_payload_status_and_only_canonical_client_identity() {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let app = Router::new().fallback(any(move |request: Request| {
            let seen = seen.clone();
            async move {
                seen.fetch_add(1, Ordering::SeqCst);
                assert_eq!(request.method(), "POST");
                assert_eq!(
                    request.uri(),
                    "/api/store/callbacks/channel?sign=a%2Fb&x=1&x=2"
                );
                assert_eq!(request.headers()["authorization"], "Bearer user-token");
                assert_eq!(request.headers()["origin"], "https://lynshen.org");
                assert_eq!(request.headers()["x-forwarded-for"], "198.51.100.8");
                for name in [
                    "forwarded",
                    "x-forwarded-host",
                    "x-real-ip",
                    "x-monoize-client-ip",
                    "x-hop",
                ] {
                    assert!(!request.headers().contains_key(name), "{name}");
                }
                assert_eq!(
                    to_bytes(request.into_body(), 1024).await.unwrap().as_ref(),
                    b"signed\0\xffbody"
                );
                Response::builder()
                    .status(429)
                    .header("set-cookie", "a=1")
                    .header("set-cookie", "b=2")
                    .header("connection", "x-hop")
                    .header("x-hop", "remove")
                    .body(Body::from("unchanged error body"))
                    .unwrap()
            }
        }));
        let (origin, task) = server(app).await;
        let deployment = handover(&origin);
        let mut request = Request::builder()
            .method("POST")
            .uri("/store/callbacks/channel?sign=a%2Fb&x=1&x=2")
            .header("authorization", "Bearer user-token")
            .header("origin", "https://lynshen.org")
            .header("x-monoize-client-ip", "198.51.100.8")
            .header("x-forwarded-for", "spoof")
            .header("x-forwarded-host", "spoof")
            .header("forwarded", "for=spoof")
            .header("x-real-ip", "spoof")
            .header("connection", "x-hop")
            .header("x-hop", "remove")
            .body(Body::from(b"signed\0\xffbody".as_slice()))
            .unwrap();
        request.extensions_mut().insert(OriginalUri(
            "/api/store/callbacks/channel?sign=a%2Fb&x=1&x=2"
                .parse()
                .unwrap(),
        ));
        let guard = deployment.admit().await.unwrap();
        let response = deployment.forward(request, guard).await;
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers().get_all("set-cookie").iter().count(), 2);
        assert!(!response.headers().contains_key("x-hop"));
        assert_eq!(
            to_bytes(response.into_body(), 1024).await.unwrap(),
            "unchanged error body"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        task.abort();
    }

    #[tokio::test]
    async fn redirect_response_is_returned_without_following_it() {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let (origin, task) = server(Router::new().fallback(any(move || {
            let seen = seen.clone();
            async move {
                seen.fetch_add(1, Ordering::SeqCst);
                Response::builder()
                    .status(307)
                    .header("location", "/again")
                    .body(Body::empty())
                    .unwrap()
            }
        })))
        .await;
        let deployment = handover(&origin);
        let response = deployment
            .forward(
                Request::new(Body::empty()),
                deployment.admit().await.unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        task.abort();
    }

    #[tokio::test]
    async fn pause_waits_for_forwarded_body_and_local_transition_releases_queue() {
        let (release, released) = oneshot::channel::<()>();
        let receiver = Arc::new(tokio::sync::Mutex::new(Some(released)));
        let (origin, task) = server(Router::new().fallback(any(move || {
            let receiver = receiver.clone();
            async move {
                let released = receiver.lock().await.take().unwrap();
                Body::from_stream(futures_util::stream::once(async move {
                    released.await.unwrap();
                    Ok::<_, std::io::Error>(bytes::Bytes::from_static(b"done"))
                }))
            }
        })))
        .await;
        let deployment = handover(&origin);
        let response = deployment
            .forward(
                Request::new(Body::empty()),
                deployment.admit().await.unwrap(),
            )
            .await;
        let pause = tokio::spawn({
            let deployment = deployment.clone();
            async move { deployment.pause().await }
        });
        tokio::task::yield_now().await;
        assert!(
            !pause.is_finished(),
            "response headers do not release the forwarding admission"
        );
        release.send(()).unwrap();
        assert_eq!(to_bytes(response.into_body(), 1024).await.unwrap(), "done");
        pause.await.unwrap().unwrap();
        let queued = tokio::spawn({
            let deployment = deployment.clone();
            async move { deployment.admit().await }
        });
        tokio::task::yield_now().await;
        assert!(!queued.is_finished());
        deployment.activate_local().await;
        assert!(queued.await.unwrap().is_none());
        assert!(
            deployment
                .resume_with_lease_slot(
                    &crate::store_billing::availability::StorePrimaryLeaseSlot::empty()
                )
                .await
                .is_err()
        );
        assert!(deployment.pause().await.is_err());
        task.abort();
    }

    #[tokio::test]
    async fn dropping_body_releases_pause_and_resume_reopens_forwarding() {
        let (origin, task) = server(Router::new().fallback(any(|| async {
            Body::from_stream(futures_util::stream::pending::<
                Result<bytes::Bytes, std::io::Error>,
            >())
        })))
        .await;
        let deployment = handover(&origin);
        let response = deployment
            .forward(
                Request::new(Body::empty()),
                deployment.admit().await.unwrap(),
            )
            .await;
        drop(response);
        tokio::time::timeout(std::time::Duration::from_secs(1), deployment.pause())
            .await
            .unwrap()
            .unwrap();
        deployment
            .resume_with_lease_slot(
                &crate::store_billing::availability::StorePrimaryLeaseSlot::empty(),
            )
            .await
            .unwrap();
        assert!(deployment.admit().await.is_some());
        task.abort();
    }

    #[tokio::test]
    async fn transport_failure_is_not_retried() {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let deployment = handover(&format!("http://{}", listener.local_addr().unwrap()));
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let server = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                seen.fetch_add(1, Ordering::SeqCst);
                let mut buffer = [0; 4096];
                let _ = socket.read(&mut buffer).await;
            }
        });
        let response = deployment
            .forward(
                Request::new(Body::empty()),
                deployment.admit().await.unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        tokio::time::timeout(std::time::Duration::from_secs(1), deployment.pause())
            .await
            .unwrap()
            .unwrap();
        server.abort();
    }

    #[tokio::test]
    async fn upstream_body_error_releases_admission() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let deployment = handover(&format!("http://{}", listener.local_addr().unwrap()));
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 4096];
            let _ = socket.read(&mut buffer).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nx")
                .await
                .unwrap();
            socket.shutdown().await.unwrap();
        });
        let response = deployment
            .forward(
                Request::new(Body::empty()),
                deployment.admit().await.unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        let mut body = response.into_body().into_data_stream();
        while let Some(item) = body.next().await {
            if item.is_err() {
                tokio::time::timeout(std::time::Duration::from_secs(1), deployment.pause())
                    .await
                    .unwrap()
                    .unwrap();
                server.await.unwrap();
                return;
            }
        }
        panic!("truncated upstream response must surface a body error");
    }

    #[tokio::test]
    async fn merged_app_forwards_protected_routes_before_guards_and_keeps_inference_local() {
        use axum::extract::ConnectInfo;
        use tower::ServiceExt;
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let (origin, server) = server(Router::new().fallback(any(move |request: Request| {
            let seen = seen.clone();
            async move {
                seen.fetch_add(1, Ordering::SeqCst);
                assert_eq!(request.headers()["x-forwarded-for"], "198.51.100.42");
                assert!(!request.headers().contains_key("x-monoize-client-ip"));
                (StatusCode::IM_A_TEAPOT, request.uri().to_string())
            }
        })))
        .await;
        let mut state =
            crate::app::load_state_with_runtime(crate::app::RuntimeConfig::with_defaults(
                "127.0.0.1:0",
                "/metrics",
                "sqlite::memory:".into(),
            ))
            .await
            .unwrap();
        state.store_lease_handover.store(true, Ordering::Release);
        state
            .store_primary_lease
            .get()
            .await
            .unwrap()
            .release()
            .await
            .unwrap();
        state.store_primary_lease =
            crate::store_billing::availability::StorePrimaryLeaseSlot::empty();
        state.deployment_handover = Some(handover(&origin));
        state.metering_token_digest = Some([0; 32]);
        let app = crate::app::build_app(state.clone());
        for (method, uri) in [
            ("POST", "/api/dashboard/store/orders"),
            ("POST", "/api/store/callbacks/test?signature=a%2Fb"),
            ("GET", "/api/store/callbacks/test?signature=a%2Fb"),
            ("POST", "/internal/replica/admission/issue"),
        ] {
            let mut request = Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .header("x-forwarded-for", "192.0.2.123")
                .header("x-monoize-client-ip", "192.0.2.124")
                .body(Body::from("invalid json; forwarding precedes extraction"))
                .unwrap();
            request.extensions_mut().insert(ConnectInfo(
                "198.51.100.42:1234"
                    .parse::<std::net::SocketAddr>()
                    .unwrap(),
            ));
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::IM_A_TEAPOT, "{uri}");
            assert_eq!(to_bytes(response.into_body(), 4096).await.unwrap(), uri);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        for uri in ["/readyz", "/v1/models"] {
            let response = app
                .clone()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_ne!(response.status(), StatusCode::IM_A_TEAPOT, "{uri}");
        }
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        state.background_shutdown.store(true, Ordering::Release);
        server.abort();
    }

    #[test]
    fn control_authentication_requires_exact_bearer_secret() {
        let deployment = handover("http://127.0.0.1:8080");
        let mut headers = HeaderMap::new();
        assert!(!deployment.authorized(&headers));
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer wrong"));
        assert!(!deployment.authorized(&headers));
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {TOKEN}")).unwrap(),
        );
        assert!(deployment.authorized(&headers));
    }
}
