//! AP-H1: per-user SSE stream of engine events.

use crate::error::{ApiError, ApiResult};
use crate::state::SharedState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use std::convert::Infallible;
use std::sync::Arc;
use tokio_stream::StreamExt;

/// Drops the per-user connection count when the last stream handle goes away.
struct CountGuard {
    state: SharedState,
    user_id: String,
}

impl Drop for CountGuard {
    fn drop(&mut self) {
        let mut counts = self.state.sse_counts.lock().expect("sse count lock");
        if let Some(current) = counts.get_mut(&self.user_id) {
            *current = current.saturating_sub(1);
            if *current == 0 {
                counts.remove(&self.user_id);
            }
        }
    }
}

pub async fn events(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let user = crate::auth::current_user(&state, &headers).await?;
    let user_id = user.id.clone();

    // AP-§11: per-user SSE connection cap.
    {
        let mut counts = state.sse_counts.lock().expect("sse count lock");
        let current = counts.get(&user_id).copied().unwrap_or(0);
        if current >= state.cfg.sse_max_per_user {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "sse_busy",
                "too many event streams for this user",
            ));
        }
        counts.insert(user_id.clone(), current + 1);
    }

    let rx = state.events.subscribe();
    let guard = Arc::new(CountGuard {
        state: state.clone(),
        user_id: user_id.clone(),
    });
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(
        move |item| {
            // Holding the guard inside the closure pins the connection count
            // to the stream's lifetime, not the handler's.
            let _ = &guard;
            match item {
                Ok(event) if event.user_id == user_id => {
                    let payload =
                        serde_json::json!({ "kind": event.kind, "payload": event.payload });
                    Some(Ok::<Event, Infallible>(
                        Event::default().data(payload.to_string()),
                    ))
                }
                _ => None,
            }
        },
    );

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
