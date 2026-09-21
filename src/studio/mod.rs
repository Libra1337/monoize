//! Studio canvas creation workbench (studio-workflow.spec.md).
//!
//! The engine owns run/step state transitions, billing, and upstream polling;
//! handlers own the dashboard/admin/public surfaces; agent.rs owns the LLM
//! function-calling loop that mutates project graphs server-side.

pub mod agent;
pub mod engine;
pub mod handlers;
pub mod llm;
pub mod store;
pub mod templates;
pub mod upstream;

#[cfg(test)]
mod tests;

use crate::db::DbPool;
use crate::monoize_routing::MonoizeRoutingStore;
use crate::node_config::HttpClients;
use crate::settings::SettingsStore;
use crate::users::UserStore;
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// Bounded fan-out channel (ST-X2): capacity 256, drop-oldest; clients
/// recover via SWR revalidation on `resync`.
pub type StudioEventSender = tokio::sync::broadcast::Sender<StudioEvent>;

#[derive(Clone, Debug, Serialize)]
pub struct StudioEvent {
    pub user_id: String,
    pub kind: String,
    pub payload: Value,
}

pub struct StudioState {
    pub db: DbPool,
    pub user_store: UserStore,
    pub monoize_store: MonoizeRoutingStore,
    pub billing_rate_store: crate::billing_rate_store::BillingRateStore,
    pub settings_store: SettingsStore,
    pub http_clients: HttpClients,
    pub events: StudioEventSender,
    pub shutdown: Arc<AtomicBool>,
}

impl StudioState {
    pub fn publish(&self, user_id: &str, kind: &str, payload: Value) {
        let _ = self.events.send(StudioEvent {
            user_id: user_id.to_string(),
            kind: kind.to_string(),
            payload,
        });
    }
}

fn positive_env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

// ST-S4 timeouts (ms).
pub fn llm_timeout_ms() -> u64 {
    positive_env_u64("MONOIZE_STUDIO_LLM_TIMEOUT_MS", 60_000)
}
pub fn image_timeout_ms() -> u64 {
    positive_env_u64("MONOIZE_STUDIO_IMAGE_TIMEOUT_MS", 300_000)
}
pub fn video_timeout_ms() -> u64 {
    positive_env_u64("MONOIZE_STUDIO_VIDEO_TIMEOUT_MS", 1_800_000)
}
// ST-S6 admission limits.
pub fn user_active_runs() -> i64 {
    positive_env_u64("MONOIZE_STUDIO_USER_ACTIVE_RUNS", 2) as i64
}
pub fn global_active_runs() -> i64 {
    positive_env_u64("MONOIZE_STUDIO_GLOBAL_ACTIVE_RUNS", 16) as i64
}
// ST-E5 transfer caps.
pub fn upload_max_bytes() -> usize {
    positive_env_u64("MONOIZE_STUDIO_UPLOAD_MAX_BYTES", 20_971_520) as usize
}
pub fn asset_max_bytes() -> usize {
    positive_env_u64("MONOIZE_STUDIO_ASSET_MAX_BYTES", 209_715_200) as usize
}

/// ST-S5: per-step poll backoff 3 s growing 1.5× capped at 15 s.
pub fn poll_backoff_secs(attempts: i64) -> u64 {
    let base = 3.0_f64 * 1.5_f64.powi(attempts.clamp(0, 10) as i32);
    (base as u64).clamp(3, 15)
}

/// Admin-editable studio settings persisted through the settings KV store.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct StudioSettings {
    #[serde(default = "default_agent_model")]
    pub agent_model: String,
    #[serde(default = "default_video_upstream")]
    pub default_video_upstream: String,
    #[serde(default)]
    pub image_model: String,
}

fn default_agent_model() -> String {
    std::env::var("MONOIZE_STUDIO_AGENT_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string())
}
fn default_video_upstream() -> String {
    "openai_video".to_string()
}

impl Default for StudioSettings {
    fn default() -> Self {
        serde_json::from_str("{}").unwrap_or(Self {
            agent_model: default_agent_model(),
            default_video_upstream: default_video_upstream(),
            image_model: String::new(),
        })
    }
}

impl StudioSettings {
    pub const KV_KEY: &'static str = "studio_settings";

    pub async fn load(settings: &SettingsStore) -> Self {
        settings
            .get(Self::KV_KEY)
            .await
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub async fn save(settings: &SettingsStore, next: &Self) -> Result<(), String> {
        let raw = serde_json::to_string(next).map_err(|e| e.to_string())?;
        settings.set(Self::KV_KEY, &raw).await
    }
}
