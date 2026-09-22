//! Apeiron server entry point: config, database, seed, engine, HTTP serve.

mod api;
mod assets;
mod auth;
mod bridge;
mod config;
mod db;
mod engine;
mod error;
mod frontend;
mod graph;
mod seed;
mod state;
mod upstream;

use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), String> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cfg = config::Config::from_env();
    let db = db::connect(&cfg.database_dsn).await?;
    db::migrate(&db).await?;
    seed::run(&db).await?;

    let (events, _) = tokio::sync::broadcast::channel(256);
    let state = Arc::new(state::AppState {
        db,
        http: reqwest::Client::new(),
        cfg,
        events,
        shutdown: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        sse_counts: std::sync::Mutex::new(std::collections::HashMap::new()),
    });

    let engine_state = state.clone();
    tokio::spawn(async move {
        engine::spawn(engine_state).await;
    });

    let app = api::router(state.clone());
    let listener = tokio::net::TcpListener::bind(&state.cfg.listen)
        .await
        .map_err(|e| format!("bind {}: {e}", state.cfg.listen))?;
    tracing::info!("apeiron listening on {}", state.cfg.listen);

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            state
                .shutdown
                .store(true, std::sync::atomic::Ordering::Relaxed);
        })
        .await
        .map_err(|e| format!("serve: {e}"))?;
    Ok(())
}

pub(crate) fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub(crate) fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub(crate) fn sha256_hex(input: &str) -> String {
    use sha2::Digest;
    hex_lower(&sha2::Sha256::digest(input.as_bytes()))
}

pub(crate) fn poll_backoff_secs(attempts: i64) -> u64 {
    let base = 3.0_f64 * 1.5_f64.powi(attempts.clamp(0, 10) as i32);
    (base as u64).clamp(3, 15)
}

pub(crate) fn parse_nano(raw: &str) -> i128 {
    raw.trim().parse::<i128>().unwrap_or(0)
}

pub(crate) fn short_error(error: &str) -> String {
    error.chars().take(500).collect()
}

pub(crate) const ASSET_MAX_BYTES: u64 = 209_715_200;

pub(crate) fn http_timeout(kind: &str, cfg: &config::Config) -> Duration {
    let ms = match kind {
        "script" | "storyboard" => cfg.llm_timeout_ms,
        "image" => cfg.image_timeout_ms,
        "video" => cfg.video_timeout_ms,
        "tts" => cfg.tts_timeout_ms,
        "material" => cfg.material_timeout_ms,
        "subtitle" => 60_000,
        "assemble" => cfg.assemble_timeout_ms,
        _ => 60_000,
    };
    Duration::from_millis(ms.max(1_000))
}
