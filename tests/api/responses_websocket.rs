use super::*;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

type TestWebSocket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

async fn start_downstream(ctx: &TestContext) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind downstream");
    let address = listener.local_addr().expect("downstream address");
    let router = ctx.router.clone();
    let task = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("serve downstream");
    });
    (address, task)
}

async fn connect_responses_websocket(
    address: SocketAddr,
    path: &str,
    authorization: Option<&str>,
    beta: Option<&str>,
) -> Result<TestWebSocket, tokio_tungstenite::tungstenite::Error> {
    let mut request = format!("ws://{address}{path}")
        .into_client_request()
        .expect("websocket request");
    if let Some(authorization) = authorization {
        request.headers_mut().insert(
            AUTHORIZATION,
            authorization.parse().expect("authorization header"),
        );
    }
    if let Some(beta) = beta {
        request
            .headers_mut()
            .insert("openai-beta", beta.parse().expect("OpenAI-Beta header"));
    }
    connect_async(request).await.map(|(socket, _)| socket)
}

async fn send_json(socket: &mut TestWebSocket, value: Value) {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .expect("send websocket request");
}

async fn receive_response(socket: &mut TestWebSocket) -> Vec<Value> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut events = Vec::new();
        loop {
            let message = socket
                .next()
                .await
                .expect("websocket closed before terminal event")
                .expect("websocket receive error");
            let Message::Text(text) = message else {
                continue;
            };
            assert!(!text.starts_with("data:"), "received SSE over websocket");
            let event: Value = serde_json::from_str(text.as_str()).expect("JSON websocket event");
            let event_type = event.get("type").and_then(Value::as_str);
            if event_type == Some("error") {
                panic!("unexpected websocket error: {event}");
            }
            let terminal = matches!(
                event_type,
                Some(
                    "response.completed"
                        | "response.failed"
                        | "response.incomplete"
                        | "response.cancelled"
                )
            );
            events.push(event);
            if terminal {
                break;
            }
        }
        events
    })
    .await
    .expect("websocket response timeout")
}

fn completed_response(events: &[Value]) -> &Value {
    events
        .iter()
        .find(|event| event.get("type").and_then(Value::as_str) == Some("response.completed"))
        .and_then(|event| event.get("response"))
        .expect("response.completed payload")
}

fn captured_responses_bodies(ctx: &TestContext) -> Vec<Value> {
    ctx.captured_bodies
        .lock()
        .expect("captured bodies lock")
        .iter()
        .filter(|(endpoint, _)| endpoint == "responses")
        .map(|(_, body)| body.clone())
        .collect()
}

#[tokio::test]
async fn responses_websocket_v2_warmup_and_continuation_use_http_upstream() {
    let ctx = setup().await;
    let (address, server) = start_downstream(&ctx).await;
    let mut socket = connect_responses_websocket(
        address,
        "/api/v1/responses",
        Some(&ctx.auth_header),
        Some("responses_websockets=2026-02-06"),
    )
    .await
    .expect("connect v2 websocket");

    send_json(
        &mut socket,
        json!({
            "type": "response.create",
            "model": "gpt-5-mini",
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{"type":"input_text","text":"v2 warmup prompt"}]
            }],
            "tools": [],
            "tool_choice": "auto",
            "parallel_tool_calls": true,
            "store": false,
            "stream": true,
            "generate": false,
            "client_metadata": {"thread_id":"thread-test"}
        }),
    )
    .await;
    let warmup_events = receive_response(&mut socket).await;
    let warmup_response = completed_response(&warmup_events);
    let warmup_id = warmup_response["id"]
        .as_str()
        .expect("warmup response id")
        .to_string();
    assert!(warmup_id.starts_with("resp_"));
    assert_eq!(warmup_response["output"], json!([]));
    assert!(captured_responses_bodies(&ctx).is_empty());

    send_json(
        &mut socket,
        json!({
            "type": "response.create",
            "model": "gpt-5-mini",
            "previous_response_id": warmup_id,
            "input": [],
            "tools": [],
            "tool_choice": "auto",
            "parallel_tool_calls": true,
            "store": false,
            "stream": true,
            "client_metadata": {"thread_id":"thread-test"}
        }),
    )
    .await;
    let generated_events = receive_response(&mut socket).await;
    assert_eq!(completed_response(&generated_events)["status"], "completed");

    let bodies = captured_responses_bodies(&ctx);
    assert_eq!(bodies.len(), 1);
    let upstream = &bodies[0];
    assert!(upstream["input"].to_string().contains("v2 warmup prompt"));
    assert_eq!(upstream["stream"], true);
    assert!(upstream.get("previous_response_id").is_none());
    assert!(upstream.get("generate").is_none());
    assert!(upstream.get("client_metadata").is_none());

    socket.close(None).await.expect("close websocket");
    server.abort();
}

#[tokio::test]
async fn responses_websocket_v1_append_reconstructs_prior_output_and_new_input() {
    let ctx = setup().await;
    let (address, server) = start_downstream(&ctx).await;
    let mut socket = connect_responses_websocket(
        address,
        "/v1/responses",
        Some(&ctx.auth_header),
        Some("responses_websockets=2026-02-04"),
    )
    .await
    .expect("connect v1 websocket");

    send_json(
        &mut socket,
        json!({
            "type": "response.create",
            "model": "gpt-5-mini",
            "input": "v1 first input",
            "stream": true
        }),
    )
    .await;
    let first_events = receive_response(&mut socket).await;
    assert_eq!(completed_response(&first_events)["status"], "completed");

    send_json(
        &mut socket,
        json!({
            "type": "response.append",
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{"type":"input_text","text":"v1 second input"}]
            }],
            "client_metadata": {"turn_id":"turn-2"}
        }),
    )
    .await;
    let second_events = receive_response(&mut socket).await;
    assert_eq!(completed_response(&second_events)["status"], "completed");

    let bodies = captured_responses_bodies(&ctx);
    assert_eq!(bodies.len(), 2);
    let second_input = bodies[1]["input"].to_string();
    assert!(second_input.contains("v1 first input"), "{second_input}");
    assert!(second_input.contains("v1 second input"), "{second_input}");
    assert!(
        second_input.contains("assistant"),
        "prior terminal output was not replayed: {second_input}"
    );
    assert!(bodies[1].get("client_metadata").is_none());

    socket.close(None).await.expect("close websocket");
    server.abort();
}

/// Drives one turn against a silent upstream and reports what the socket carried:
/// (pongs received, server Pings received, terminal reached).
async fn observe_silent_turn(
    socket: &mut TestWebSocket,
    upstream_delay_ms: u64,
    client_ping: bool,
    budget: Duration,
) -> (usize, usize, bool) {
    send_json(
        socket,
        json!({
            "type": "response.create",
            "model": "gpt-5-mini",
            "input": "slow",
            "stream": true,
            "force_upstream_delay_ms": upstream_delay_ms
        }),
    )
    .await;
    if client_ping {
        socket
            .send(Message::Ping(vec![7].into()))
            .await
            .expect("send ping");
    }

    let mut pongs = 0usize;
    let mut server_pings = 0usize;
    let mut terminal = false;
    tokio::time::timeout(budget, async {
        while let Some(message) = socket.next().await {
            match message.expect("websocket receive") {
                Message::Pong(_) => pongs += 1,
                Message::Ping(_) => server_pings += 1,
                Message::Text(text) => {
                    let event: Value = serde_json::from_str(text.as_str()).expect("event JSON");
                    if event.get("type").and_then(Value::as_str) == Some("response.completed") {
                        terminal = true;
                        return;
                    }
                }
                Message::Close(frame) => panic!("closed before terminal: {frame:?}"),
                _ => {}
            }
        }
        panic!("socket ended before a terminal event");
    })
    .await
    .expect("turn must terminate");
    (pongs, server_pings, terminal)
}

/// WS12a: a Ping sent mid-turn must be answered before that turn's terminal. The queued
/// automatic Pong only flushes on a socket read, and a turn awaiting a silent upstream
/// performs no reads unless the bridge reads inbound messages concurrently. An intermediary
/// that treats an unanswered Ping as a dead peer closes the connection mid-turn.
#[tokio::test]
async fn responses_websocket_answers_ping_during_upstream_silence() {
    let ctx = setup().await;
    let (address, server) = start_downstream(&ctx).await;
    let mut socket =
        connect_responses_websocket(address, "/v1/responses", Some(&ctx.auth_header), None)
            .await
            .expect("connect websocket");

    let (pongs, _, terminal) =
        observe_silent_turn(&mut socket, 3_000, true, Duration::from_secs(30)).await;

    assert!(terminal);
    assert_eq!(pongs, 1, "the mid-turn Ping must be answered");

    socket.close(None).await.expect("close websocket");
    server.abort();
}

/// WS18: a turn whose upstream is silent must not leave the socket without an outbound frame
/// for longer than the keep-alive interval. Before this, the bridge dropped the downstream
/// SSE comment heartbeats and emitted nothing at all, so an intermediary idle timeout closed
/// the connection mid-turn with no Close frame -- which the Codex client reports as
/// `stream closed before response.completed`.
///
/// The upstream delay exceeds the 15s default interval. Configuring a shorter interval would
/// mean setting a process-global environment variable, which the limits are read from per
/// upgrade and would leak into tests running in parallel.
#[tokio::test]
async fn responses_websocket_sends_keepalive_ping_during_long_upstream_silence() {
    let ctx = setup().await;
    let (address, server) = start_downstream(&ctx).await;
    let mut socket =
        connect_responses_websocket(address, "/v1/responses", Some(&ctx.auth_header), None)
            .await
            .expect("connect websocket");

    let (_, server_pings, terminal) =
        observe_silent_turn(&mut socket, 16_500, false, Duration::from_secs(60)).await;

    assert!(terminal);
    assert!(
        server_pings > 0,
        "a silent turn longer than the keep-alive interval must carry a Ping"
    );

    socket.close(None).await.expect("close websocket");
    server.abort();
}

/// WS4: a client may send its next request before the current turn terminates. The message
/// must be served after that terminal, not dropped. Reading inbound messages concurrently
/// with forwarding (WS12a) makes this reachable, so the bridge buffers them.
#[tokio::test]
async fn responses_websocket_serves_a_request_that_arrives_mid_turn() {
    let ctx = setup().await;
    let (address, server) = start_downstream(&ctx).await;
    let mut socket =
        connect_responses_websocket(address, "/v1/responses", Some(&ctx.auth_header), None)
            .await
            .expect("connect websocket");

    send_json(
        &mut socket,
        json!({
            "type": "response.create",
            "model": "gpt-5-mini",
            "input": "first",
            "stream": true,
            "force_upstream_delay_ms": 1500
        }),
    )
    .await;
    // Pipelined immediately, while the first turn is still waiting on its upstream.
    send_json(
        &mut socket,
        json!({
            "type": "response.create",
            "model": "gpt-5-mini",
            "input": "second",
            "stream": true
        }),
    )
    .await;

    let first = receive_response(&mut socket).await;
    assert_eq!(completed_response(&first)["status"], "completed");
    let second = receive_response(&mut socket).await;
    assert_eq!(
        completed_response(&second)["status"],
        "completed",
        "a request that arrived mid-turn must still be served"
    );

    socket.close(None).await.expect("close websocket");
    server.abort();
}

#[tokio::test]
async fn responses_websocket_rejects_unauthenticated_upgrade() {
    let ctx = setup().await;
    let (address, server) = start_downstream(&ctx).await;
    let error = connect_responses_websocket(address, "/v1/responses", None, None)
        .await
        .expect_err("unauthenticated websocket upgrade must fail");
    let tokio_tungstenite::tungstenite::Error::Http(response) = error else {
        panic!("expected HTTP handshake error");
    };
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    server.abort();
}
