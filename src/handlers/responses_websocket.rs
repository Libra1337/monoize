use super::*;
use axum::extract::State;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures_util::StreamExt;
use serde_json::{Map, Value, json};

const DEFAULT_MESSAGE_MAX_BYTES: usize = 50 * 1024 * 1024;
const DEFAULT_CONNECTION_MAX_BYTES: usize = 100 * 1024 * 1024;
const DEFAULT_MAX_TURNS: usize = 128;
const DEFAULT_HISTORY_MAX_ITEMS: usize = 4_096;
const DEFAULT_HISTORY_MAX_BYTES: usize = 32 * 1024 * 1024;
const DEFAULT_KEEPALIVE_MS: u64 = 15_000;

#[derive(Clone, Copy, Debug)]
struct ResponsesWebsocketLimits {
    message_max_bytes: usize,
    connection_max_bytes: usize,
    max_turns: usize,
    history_max_items: usize,
    history_max_bytes: usize,
    sse_frame_max_bytes: usize,
    sse_buffer_max_bytes: usize,
    keepalive_ms: u64,
}

impl Default for ResponsesWebsocketLimits {
    fn default() -> Self {
        Self {
            message_max_bytes: DEFAULT_MESSAGE_MAX_BYTES,
            connection_max_bytes: DEFAULT_CONNECTION_MAX_BYTES,
            max_turns: DEFAULT_MAX_TURNS,
            history_max_items: DEFAULT_HISTORY_MAX_ITEMS,
            history_max_bytes: DEFAULT_HISTORY_MAX_BYTES,
            sse_frame_max_bytes: DEFAULT_MESSAGE_MAX_BYTES,
            sse_buffer_max_bytes: DEFAULT_MESSAGE_MAX_BYTES,
            keepalive_ms: DEFAULT_KEEPALIVE_MS,
        }
    }
}

impl ResponsesWebsocketLimits {
    fn from_env() -> Self {
        let defaults = Self::default();
        Self {
            message_max_bytes: positive_env(
                "MONOIZE_RESPONSES_WS_MESSAGE_MAX_BYTES",
                defaults.message_max_bytes,
            ),
            connection_max_bytes: positive_env(
                "MONOIZE_RESPONSES_WS_CONNECTION_MAX_BYTES",
                defaults.connection_max_bytes,
            ),
            max_turns: positive_env("MONOIZE_RESPONSES_WS_MAX_TURNS", defaults.max_turns),
            history_max_items: positive_env(
                "MONOIZE_RESPONSES_WS_HISTORY_MAX_ITEMS",
                defaults.history_max_items,
            ),
            history_max_bytes: positive_env(
                "MONOIZE_RESPONSES_WS_HISTORY_MAX_BYTES",
                defaults.history_max_bytes,
            ),
            sse_frame_max_bytes: positive_env(
                "MONOIZE_RESPONSES_WS_SSE_FRAME_MAX_BYTES",
                defaults.sse_frame_max_bytes,
            ),
            sse_buffer_max_bytes: positive_env(
                "MONOIZE_RESPONSES_WS_SSE_BUFFER_MAX_BYTES",
                defaults.sse_buffer_max_bytes,
            ),
            keepalive_ms: positive_env_u64(
                "MONOIZE_RESPONSES_WS_KEEPALIVE_MS",
                defaults.keepalive_ms,
            ),
        }
    }
}

#[derive(Default)]
struct ResponsesWebsocketSession {
    last_request: Option<Map<String, Value>>,
    last_response_id: Option<String>,
    last_response_output: Vec<Value>,
}

struct CompletedResponse {
    id: String,
    output: Vec<Value>,
    retainable: bool,
}

#[derive(Debug)]
struct WebsocketEventError {
    status: u16,
    code: &'static str,
    message: String,
    param: Option<&'static str>,
}

impl WebsocketEventError {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST.as_u16(),
            code: "invalid_websocket_event",
            message: message.into(),
            param: None,
        }
    }

    fn previous_response_not_found() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST.as_u16(),
            code: "previous_response_not_found",
            message: "the previous response is not available on this WebSocket connection"
                .to_string(),
            param: Some("previous_response_id"),
        }
    }

    // WS15: the code is fixed by the Codex client, which retries on a fresh connection
    // for exactly `websocket_connection_limit_reached` and treats any other code carried
    // with a non-2xx status as a non-retryable transport error.
    fn connection_limit() -> Self {
        Self {
            status: StatusCode::PAYLOAD_TOO_LARGE.as_u16(),
            code: "websocket_connection_limit_reached",
            message: "WebSocket connection resource limit reached. Create a new WebSocket \
                      connection to continue."
                .to_string(),
            param: None,
        }
    }

    // WS19: the bridge's own terminal guarantee, independent of the stream encoders.
    fn stream_incomplete() -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY.as_u16(),
            code: "upstream_stream_incomplete",
            message: "upstream stream ended before a terminal event".to_string(),
            param: None,
        }
    }

    fn history_limit() -> Self {
        Self {
            status: StatusCode::PAYLOAD_TOO_LARGE.as_u16(),
            code: "websocket_history_limit_exceeded",
            message: "WebSocket continuation history limit exceeded".to_string(),
            param: Some("input"),
        }
    }

    fn parser_limit() -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY.as_u16(),
            code: "websocket_sse_limit_exceeded",
            message: "upstream SSE frame exceeded the WebSocket bridge limit".to_string(),
            param: None,
        }
    }
}

pub async fn responses_websocket(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> AppResult<Response> {
    auth_tenant(&headers, &state).await?;
    let limits = ResponsesWebsocketLimits::from_env();
    Ok(ws
        .max_message_size(limits.message_max_bytes)
        .max_frame_size(limits.message_max_bytes)
        .on_upgrade(move |socket| serve_responses_websocket(socket, state, headers, limits))
        .into_response())
}

async fn serve_responses_websocket(
    mut socket: WebSocket,
    state: AppState,
    headers: HeaderMap,
    limits: ResponsesWebsocketLimits,
) {
    let mut session = ResponsesWebsocketSession::default();
    let mut accepted_turns = 0_usize;
    let mut inbound_bytes = 0_usize;

    while let Some(message) = socket.next().await {
        let message = match message {
            Ok(message) => message,
            Err(err) => {
                tracing::debug!(error = %err, "Responses WebSocket receive failed");
                break;
            }
        };

        match message {
            Message::Text(text) => {
                // A turn can buffer client messages that arrived while it was streaming
                // (WS4). Serve them in arrival order before returning to the socket, so a
                // client that pipelines its turns is answered in order.
                let mut queue = std::collections::VecDeque::from([text]);
                let mut closed = false;
                while let Some(text) = queue.pop_front() {
                    inbound_bytes = inbound_bytes.saturating_add(text.len());
                    if accepted_turns >= limits.max_turns
                        || inbound_bytes > limits.connection_max_bytes
                    {
                        let _ =
                            send_event_error(&mut socket, WebsocketEventError::connection_limit())
                                .await;
                        closed = true;
                        break;
                    }
                    accepted_turns = accepted_turns.saturating_add(1);
                    let outcome = handle_client_text(
                        &mut socket,
                        &mut session,
                        &state,
                        &headers,
                        text.as_str(),
                        limits,
                    )
                    .await;
                    for deferred in outcome.deferred {
                        queue.push_back(deferred);
                    }
                    if !outcome.keep_open {
                        closed = true;
                        break;
                    }
                }
                if closed {
                    break;
                }
            }
            Message::Binary(_) => {
                if !send_event_error(
                    &mut socket,
                    WebsocketEventError::invalid("binary WebSocket messages are not supported"),
                )
                .await
                {
                    break;
                }
            }
            Message::Ping(payload) => {
                if socket.send(Message::Pong(payload)).await.is_err() {
                    break;
                }
            }
            Message::Pong(_) => {}
            Message::Close(_) => break,
        }
    }
}

/// Result of serving one client text message: whether the connection stays open, and any
/// client messages that arrived while the turn was streaming and still need serving.
struct TurnOutcome {
    keep_open: bool,
    deferred: Vec<Utf8Bytes>,
}

impl TurnOutcome {
    fn from_send(keep_open: bool) -> Self {
        Self {
            keep_open,
            deferred: Vec::new(),
        }
    }
}

async fn handle_client_text(
    socket: &mut WebSocket,
    session: &mut ResponsesWebsocketSession,
    state: &AppState,
    headers: &HeaderMap,
    text: &str,
    limits: ResponsesWebsocketLimits,
) -> TurnOutcome {
    let value = match serde_json::from_str::<Value>(text) {
        Ok(value) => value,
        Err(err) => {
            return TurnOutcome::from_send(
                send_event_error(
                    socket,
                    WebsocketEventError::invalid(format!("invalid JSON: {err}")),
                )
                .await,
            );
        }
    };
    let Some(mut event) = value.as_object().cloned() else {
        return TurnOutcome::from_send(
            send_event_error(
                socket,
                WebsocketEventError::invalid("WebSocket event must be a JSON object"),
            )
            .await,
        );
    };
    let Some(event_type) = event.get("type").and_then(Value::as_str) else {
        return TurnOutcome::from_send(
            send_event_error(
                socket,
                WebsocketEventError::invalid("WebSocket event is missing type"),
            )
            .await,
        );
    };

    let warmup = event_type == "response.create"
        && event.get("generate").and_then(Value::as_bool) == Some(false);
    let prepared = match event_type {
        "response.create" => prepare_response_create(&mut event, session, limits),
        "response.append" => prepare_response_append(&event, session, limits),
        _ => Err(WebsocketEventError::invalid(format!(
            "unsupported WebSocket event type '{event_type}'"
        ))),
    };
    let request = match prepared {
        Ok(request) => request,
        Err(err) => return TurnOutcome::from_send(send_event_error(socket, err).await),
    };

    if request.get("background").and_then(Value::as_bool) == Some(true) {
        return TurnOutcome::from_send(
            send_event_error(
                socket,
                WebsocketEventError {
                    status: StatusCode::BAD_REQUEST.as_u16(),
                    code: "background_not_supported",
                    message: "background not supported".to_string(),
                    param: Some("background"),
                },
            )
            .await,
        );
    }

    if warmup {
        return TurnOutcome::from_send(send_warmup(socket, session, request, limits).await);
    }

    let mut request_headers = headers.clone();
    request_headers.insert(
        axum::http::header::HeaderName::from_static("x-request-id"),
        axum::http::HeaderValue::from_str(&uuid::Uuid::new_v4().to_string())
            .expect("UUID request id is a valid header value"),
    );
    let response = match super::create_response(
        State(state.clone()),
        request_headers,
        axum::Json(Value::Object(request.clone())),
    )
    .await
    {
        Ok(response) => response,
        Err(err) => return TurnOutcome::from_send(send_app_error(socket, err).await),
    };

    let outcome = match forward_sse_body_as_websocket(socket, response, limits).await {
        Ok(outcome) => outcome,
        Err(err) => {
            return TurnOutcome {
                keep_open: send_event_error(socket, err).await,
                deferred: Vec::new(),
            };
        }
    };
    let ForwardOutcome { turn, deferred } = outcome;
    let completed = match turn {
        ForwardedTurn::Terminal(completed) => completed,
        // WS19: the SSE body ended with no terminal event. The client cannot distinguish a
        // truncated turn from a successful one without a terminal frame, so supply one here.
        ForwardedTurn::MissingTerminal => {
            tracing::warn!(
                "Responses WebSocket turn ended without a terminal event; sending {}",
                WebsocketEventError::stream_incomplete().code
            );
            return TurnOutcome {
                keep_open: send_event_error(socket, WebsocketEventError::stream_incomplete()).await,
                deferred,
            };
        }
        // WS12b: no continuation state is retained for a turn the client abandoned.
        ForwardedTurn::ClientClosed => {
            return TurnOutcome {
                keep_open: false,
                deferred: Vec::new(),
            };
        }
    };
    if let Some(completed) = completed {
        if completed.retainable && request_with_output_fits(&request, &completed.output, limits) {
            session.last_request = Some(request);
            session.last_response_id = Some(completed.id);
            session.last_response_output = completed.output;
        } else {
            *session = ResponsesWebsocketSession::default();
        }
    }
    TurnOutcome {
        keep_open: true,
        deferred,
    }
}

fn prepare_response_create(
    event: &mut Map<String, Value>,
    session: &ResponsesWebsocketSession,
    limits: ResponsesWebsocketLimits,
) -> Result<Map<String, Value>, WebsocketEventError> {
    let previous_response_id = event
        .get("previous_response_id")
        .filter(|value| !value.is_null())
        .and_then(Value::as_str)
        .map(str::to_string);
    let incoming_input = input_items(event.get("input"))?;

    let mut request = if let Some(previous_response_id) = previous_response_id {
        if session.last_response_id.as_deref() != Some(previous_response_id.as_str()) {
            return Err(WebsocketEventError::previous_response_not_found());
        }
        let mut request = session
            .last_request
            .clone()
            .ok_or_else(WebsocketEventError::previous_response_not_found)?;
        overlay_response_create_fields(&mut request, event);
        request.insert(
            "input".to_string(),
            Value::Array(continued_input(session, incoming_input, limits)?),
        );
        request
    } else {
        let mut request = event.clone();
        request.insert("input".to_string(), Value::Array(incoming_input));
        request
    };

    normalize_generated_request(&mut request);
    ensure_request_history_fits(&request, limits)?;
    Ok(request)
}

fn prepare_response_append(
    event: &Map<String, Value>,
    session: &ResponsesWebsocketSession,
    limits: ResponsesWebsocketLimits,
) -> Result<Map<String, Value>, WebsocketEventError> {
    if !event.contains_key("input") {
        return Err(WebsocketEventError::invalid(
            "response.append is missing input",
        ));
    }
    let incoming_input = input_items(event.get("input"))?;
    let mut request = session
        .last_request
        .clone()
        .ok_or_else(WebsocketEventError::previous_response_not_found)?;
    request.insert(
        "input".to_string(),
        Value::Array(continued_input(session, incoming_input, limits)?),
    );
    normalize_generated_request(&mut request);
    ensure_request_history_fits(&request, limits)?;
    Ok(request)
}

fn continued_input(
    session: &ResponsesWebsocketSession,
    incoming_input: Vec<Value>,
    limits: ResponsesWebsocketLimits,
) -> Result<Vec<Value>, WebsocketEventError> {
    let request = session
        .last_request
        .as_ref()
        .ok_or_else(WebsocketEventError::previous_response_not_found)?;
    let mut input = input_items(request.get("input"))?;
    let total_items = input
        .len()
        .saturating_add(session.last_response_output.len())
        .saturating_add(incoming_input.len());
    if total_items > limits.history_max_items {
        return Err(WebsocketEventError::history_limit());
    }
    input.extend(session.last_response_output.iter().cloned());
    input.extend(incoming_input);
    if serialized_len(&input) > limits.history_max_bytes {
        return Err(WebsocketEventError::history_limit());
    }
    Ok(input)
}

fn ensure_request_history_fits(
    request: &Map<String, Value>,
    limits: ResponsesWebsocketLimits,
) -> Result<(), WebsocketEventError> {
    let items = request
        .get("input")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if items > limits.history_max_items || serialized_len(request) > limits.history_max_bytes {
        return Err(WebsocketEventError::history_limit());
    }
    Ok(())
}

fn request_with_output_fits(
    request: &Map<String, Value>,
    output: &[Value],
    limits: ResponsesWebsocketLimits,
) -> bool {
    let request_items = request
        .get("input")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    request_items.saturating_add(output.len()) <= limits.history_max_items
        && serialized_len(request).saturating_add(serialized_len(output))
            <= limits.history_max_bytes
}

fn input_items(input: Option<&Value>) -> Result<Vec<Value>, WebsocketEventError> {
    match input {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => Ok(items.clone()),
        Some(Value::String(_)) | Some(Value::Object(_)) => Ok(vec![input.cloned().unwrap()]),
        Some(_) => Err(WebsocketEventError::invalid(
            "Responses input must be a string, object, or array",
        )),
    }
}

fn overlay_response_create_fields(request: &mut Map<String, Value>, event: &Map<String, Value>) {
    for (key, value) in event {
        if !matches!(
            key.as_str(),
            "type" | "input" | "previous_response_id" | "generate" | "client_metadata"
        ) {
            request.insert(key.clone(), value.clone());
        }
    }
}

fn normalize_generated_request(request: &mut Map<String, Value>) {
    for key in [
        "type",
        "generate",
        "client_metadata",
        "previous_response_id",
    ] {
        request.remove(key);
    }
    request.insert("stream".to_string(), Value::Bool(true));
}

async fn send_warmup(
    socket: &mut WebSocket,
    session: &mut ResponsesWebsocketSession,
    request: Map<String, Value>,
    limits: ResponsesWebsocketLimits,
) -> bool {
    let Some(model) = request.get("model").and_then(Value::as_str) else {
        return send_event_error(
            socket,
            WebsocketEventError::invalid("response.create is missing model"),
        )
        .await;
    };
    let response_id = format!("resp_monoize_ws_{}", uuid::Uuid::new_v4().simple());
    let created_at = chrono::Utc::now().timestamp();
    let created_response = warmup_response(&response_id, model, created_at, "in_progress");
    let completed_response = warmup_response(&response_id, model, created_at, "completed");
    let created = json!({
        "type": "response.created",
        "sequence_number": 1,
        "response": created_response,
    });
    let completed = json!({
        "type": "response.completed",
        "sequence_number": 2,
        "response": completed_response,
    });

    if !send_json(socket, created).await || !send_json(socket, completed).await {
        return false;
    }
    if request_with_output_fits(&request, &[], limits) {
        session.last_request = Some(request);
        session.last_response_id = Some(response_id);
        session.last_response_output.clear();
    } else {
        *session = ResponsesWebsocketSession::default();
    }
    true
}

fn warmup_response(id: &str, model: &str, created_at: i64, status: &str) -> Value {
    json!({
        "id": id,
        "object": "response",
        "created_at": created_at,
        "completed_at": (status == "completed").then_some(created_at),
        "model": model,
        "status": status,
        "output": [],
        "incomplete_details": null,
        "previous_response_id": null,
        "instructions": null,
        "error": null,
        "tools": [],
        "tool_choice": "auto",
        "truncation": "auto",
        "parallel_tool_calls": true,
        "text": { "format": { "type": "text" } },
        "top_p": 1.0,
        "presence_penalty": 0,
        "frequency_penalty": 0,
        "top_logprobs": 0,
        "temperature": 1.0,
        "reasoning": null,
        "max_output_tokens": null,
        "max_tool_calls": null,
        "store": false,
        "background": false,
        "metadata": {},
        "safety_identifier": null,
        "prompt_cache_key": null,
        "usage": null,
        "user": null,
    })
}

/// Outcome of forwarding one generated turn's SSE body over the WebSocket.
enum ForwardedTurn {
    /// A Responses terminal event was forwarded. Carries WS7 continuation state when
    /// the terminal was `response.completed`.
    Terminal(Option<CompletedResponse>),
    /// The SSE body ended with no terminal event. WS19 requires the bridge to supply one.
    MissingTerminal,
    /// The client closed the connection mid-turn (WS12a).
    ClientClosed,
}

/// One turn's forwarding result, plus any client text messages that arrived while the turn
/// was still streaming. WS4 lets a client send its next request without waiting for the
/// current turn's terminal, so those messages MUST be carried to the receive loop rather
/// than dropped; dropping one strands a client that pipelines its turns.
struct ForwardOutcome {
    turn: ForwardedTurn,
    deferred: Vec<Utf8Bytes>,
}

async fn forward_sse_body_as_websocket(
    socket: &mut WebSocket,
    response: Response,
    limits: ResponsesWebsocketLimits,
) -> Result<ForwardOutcome, WebsocketEventError> {
    let mut stream = response.into_body().into_data_stream();
    let mut buffer = Vec::new();
    let mut completed = None;
    let mut terminal_seen = false;
    let mut deferred: Vec<Utf8Bytes> = Vec::new();
    let keepalive = std::time::Duration::from_millis(limits.keepalive_ms.max(1));
    // WS18 measures the interval from the last outbound frame, so the deadline is tracked
    // explicitly rather than as a per-iteration timer. An SSE keep-alive comment arrives on
    // the same 15s cadence and produces no outbound frame; a timer rebuilt each iteration
    // would be reset by it and never fire.
    let mut next_keepalive = tokio::time::Instant::now() + keepalive;

    loop {
        // WS12a / WS18: the turn's upstream is silent for its whole reasoning phase. Read
        // inbound client messages concurrently so a Ping is answered and a Close is acted
        // on, and treat a lapsed keep-alive interval as a third event source. `socket.next()`
        // is what flushes the implementation's queued automatic Pong; a turn that only
        // awaits the SSE body never writes, so the Pong would never reach the client.
        let chunk = tokio::select! {
            biased;
            inbound = socket.next() => {
                match inbound {
                    // A Ping is answered by the implementation as a side effect of this read.
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                    Some(Ok(Message::Close(_))) | None => {
                        return Ok(ForwardOutcome {
                            turn: ForwardedTurn::ClientClosed,
                            deferred,
                        });
                    }
                    Some(Err(err)) => {
                        tracing::debug!(error = %err, "Responses WebSocket receive failed mid-turn");
                        return Ok(ForwardOutcome {
                            turn: ForwardedTurn::ClientClosed,
                            deferred,
                        });
                    }
                    // WS4 runs one generation at a time, so this request waits for the
                    // current turn's terminal instead of being served now.
                    Some(Ok(Message::Text(text))) => {
                        deferred.push(text);
                        continue;
                    }
                    Some(Ok(Message::Binary(_))) => continue,
                }
            }
            chunk = stream.next() => chunk,
            _ = tokio::time::sleep_until(next_keepalive) => {
                if socket.send(Message::Ping(Vec::new().into())).await.is_err() {
                    return Err(WebsocketEventError::invalid("WebSocket send failed"));
                }
                next_keepalive = tokio::time::Instant::now() + keepalive;
                continue;
            }
        };

        let Some(chunk) = chunk else { break };
        let chunk = chunk.map_err(|err| {
            tracing::debug!(error = %err, "Responses WebSocket SSE bridge read failed");
            WebsocketEventError::invalid("upstream SSE body read failed")
        })?;
        if buffer.len().saturating_add(chunk.len()) > limits.sse_buffer_max_bytes {
            return Err(WebsocketEventError::parser_limit());
        }
        buffer.extend_from_slice(&chunk);
        for data in drain_sse_data(&mut buffer, false, limits.sse_frame_max_bytes)? {
            if forward_sse_event(socket, &data, limits, &mut completed, &mut terminal_seen).await? {
                next_keepalive = tokio::time::Instant::now() + keepalive;
            }
        }
    }

    for data in drain_sse_data(&mut buffer, true, limits.sse_frame_max_bytes)? {
        forward_sse_event(socket, &data, limits, &mut completed, &mut terminal_seen).await?;
    }
    Ok(ForwardOutcome {
        turn: if terminal_seen {
            ForwardedTurn::Terminal(completed)
        } else {
            ForwardedTurn::MissingTerminal
        },
        deferred,
    })
}

/// Forwards one SSE payload as a WebSocket text message. Returns whether an outbound frame
/// was actually sent, which WS18 measures its keep-alive interval from.
async fn forward_sse_event(
    socket: &mut WebSocket,
    data: &str,
    limits: ResponsesWebsocketLimits,
    completed: &mut Option<CompletedResponse>,
    terminal_seen: &mut bool,
) -> Result<bool, WebsocketEventError> {
    // WS4: `[DONE]` is an SSE sentinel, not a Responses event.
    if data == "[DONE]" {
        return Ok(false);
    }
    if let Some(terminal) = completed_response_from_event(data, limits) {
        *completed = Some(terminal);
    }
    if event_is_terminal(data) {
        *terminal_seen = true;
    }
    if socket
        .send(Message::Text(data.to_owned().into()))
        .await
        .is_err()
    {
        return Err(WebsocketEventError::invalid("WebSocket send failed"));
    }
    Ok(true)
}

fn event_is_terminal(data: &str) -> bool {
    serde_json::from_str::<Value>(data)
        .ok()
        .as_ref()
        .and_then(|event| event.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|event_type| {
            matches!(
                event_type,
                "response.completed"
                    | "response.incomplete"
                    | "response.failed"
                    | "response.cancelled"
                    | "error"
            )
        })
}

fn drain_sse_data(
    buffer: &mut Vec<u8>,
    eof: bool,
    max_frame_bytes: usize,
) -> Result<Vec<String>, WebsocketEventError> {
    let mut frames = Vec::new();
    loop {
        let boundary = buffer.windows(2).position(|window| window == b"\n\n");
        let Some(boundary) = boundary else {
            break;
        };
        if boundary.saturating_add(2) > max_frame_bytes {
            return Err(WebsocketEventError::parser_limit());
        }
        let frame = buffer.drain(..boundary + 2).collect::<Vec<_>>();
        if let Some(data) = parse_sse_data_frame(&frame) {
            frames.push(data);
        }
    }
    if buffer.len() > max_frame_bytes {
        return Err(WebsocketEventError::parser_limit());
    }
    if eof && !buffer.is_empty() {
        let frame = std::mem::take(buffer);
        if let Some(data) = parse_sse_data_frame(&frame) {
            frames.push(data);
        }
    }
    Ok(frames)
}

fn parse_sse_data_frame(frame: &[u8]) -> Option<String> {
    let frame = String::from_utf8_lossy(frame);
    let data = frame
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|line| line.strip_prefix(' ').unwrap_or(line))
        .collect::<Vec<_>>();
    (!data.is_empty()).then(|| data.join("\n"))
}

fn completed_response_from_event(
    data: &str,
    limits: ResponsesWebsocketLimits,
) -> Option<CompletedResponse> {
    let event: Value = serde_json::from_str(data).ok()?;
    if event.get("type").and_then(Value::as_str) != Some("response.completed") {
        return None;
    }
    let response = event.get("response")?;
    let id = response.get("id")?.as_str()?.to_string();
    let output = response
        .get("output")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let retainable = output.len() <= limits.history_max_items
        && serialized_len(&output) <= limits.history_max_bytes;
    Some(CompletedResponse {
        id,
        output: if retainable { output } else { Vec::new() },
        retainable,
    })
}

fn serialized_len(value: &(impl serde::Serialize + ?Sized)) -> usize {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX)
}

fn positive_env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn positive_env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

async fn send_app_error(socket: &mut WebSocket, err: AppError) -> bool {
    let status = err.upstream_status.unwrap_or(err.status.as_u16());
    let code = err
        .upstream_code
        .as_deref()
        .unwrap_or(err.code.as_str())
        .to_string();
    let error_type = err
        .upstream_type
        .as_deref()
        .unwrap_or(err.error_type.as_str())
        .to_string();
    let param = err.upstream_param.as_ref().or(err.param.as_ref()).cloned();
    send_json(
        socket,
        json!({
            "type": "error",
            "status": status,
            "sequence_number": 0,
            "error": {
                "type": error_type,
                "code": code,
                "message": err.message,
                "param": param,
            }
        }),
    )
    .await
}

async fn send_event_error(socket: &mut WebSocket, err: WebsocketEventError) -> bool {
    send_json(
        socket,
        json!({
            "type": "error",
            "status": err.status,
            "sequence_number": 0,
            "error": {
                "type": "invalid_request_error",
                "code": err.code,
                "message": err.message,
                "param": err.param,
            }
        }),
    )
    .await
}

async fn send_json(socket: &mut WebSocket, value: Value) -> bool {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_bridge_extracts_json_and_ignores_comments() {
        let mut buffer =
            b": heartbeat\n\nevent: response.created\ndata: {\"type\":\"response.created\"}\n\n"
                .to_vec();
        assert_eq!(
            drain_sse_data(&mut buffer, false, 1024).unwrap(),
            vec![r#"{"type":"response.created"}"#.to_string()]
        );
        assert!(buffer.is_empty());
    }

    #[test]
    fn v2_continuation_reconstructs_full_input() {
        let mut session = ResponsesWebsocketSession {
            last_request: Some(
                json!({ "model": "mock", "input": [{"type":"message","role":"user"}] })
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
            last_response_id: Some("resp_1".to_string()),
            last_response_output: vec![json!({"type":"function_call","call_id":"call_1"})],
        };
        let mut event = json!({
            "type": "response.create",
            "model": "mock",
            "previous_response_id": "resp_1",
            "input": [{"type":"function_call_output","call_id":"call_1","output":"ok"}]
        })
        .as_object()
        .unwrap()
        .clone();

        let request =
            prepare_response_create(&mut event, &session, ResponsesWebsocketLimits::default())
                .unwrap();
        let input = request.get("input").and_then(Value::as_array).unwrap();
        assert_eq!(input.len(), 3);
        assert!(!request.contains_key("previous_response_id"));
        assert_eq!(request.get("stream"), Some(&json!(true)));

        session.last_response_id = Some("different".to_string());
        assert!(
            prepare_response_create(&mut event, &session, ResponsesWebsocketLimits::default(),)
                .is_err()
        );
    }

    #[test]
    fn continuation_rejects_history_item_limit_before_extension() {
        let session = ResponsesWebsocketSession {
            last_request: Some(json!({"input": [1, 2]}).as_object().unwrap().clone()),
            last_response_id: Some("resp_1".to_string()),
            last_response_output: vec![json!(3)],
        };
        let limits = ResponsesWebsocketLimits {
            history_max_items: 3,
            ..ResponsesWebsocketLimits::default()
        };
        assert!(continued_input(&session, vec![json!(4)], limits).is_err());
    }

    #[test]
    fn sse_parser_rejects_unterminated_oversized_frame() {
        let mut buffer = vec![b'x'; 9];
        assert!(drain_sse_data(&mut buffer, false, 8).is_err());
    }
}
