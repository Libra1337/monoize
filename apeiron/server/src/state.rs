//! Shared application state and the SSE event bus.

use crate::config::Config;
use serde::Serialize;
use serde_json::Value;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

pub struct AppState {
    pub db: crate::db::Pool,
    pub http: reqwest::Client,
    pub cfg: Config,
    pub events: broadcast::Sender<Event>,
    pub shutdown: Arc<AtomicBool>,
    pub sse_counts: Mutex<std::collections::HashMap<String, usize>>,
}

pub type SharedState = Arc<AppState>;

#[derive(Clone, Debug, Serialize)]
pub struct Event {
    pub user_id: String,
    pub kind: String,
    pub payload: Value,
}

impl AppState {
    pub fn publish(&self, user_id: &str, kind: &str, payload: Value) {
        let _ = self.events.send(Event {
            user_id: user_id.to_string(),
            kind: kind.to_string(),
            payload,
        });
    }
}
