//! AP-§11: environment configuration, parsed once at boot.

#[derive(Clone, Debug)]
pub struct Config {
    pub listen: String,
    pub database_dsn: String,
    pub platform_url: String,
    pub bridge_service_token: Option<String>,
    pub worker_token: Option<String>,
    pub assets_dir: String,
    pub user_active_runs: i64,
    pub global_active_runs: i64,
    pub llm_timeout_ms: u64,
    pub image_timeout_ms: u64,
    pub video_timeout_ms: u64,
    pub tts_timeout_ms: u64,
    pub material_timeout_ms: u64,
    pub assemble_timeout_ms: u64,
    pub sse_max_per_user: usize,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|raw| raw.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_u64(name: &str, default: u64) -> u64 {
    env(name)
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            listen: env("APEIRON_LISTEN").unwrap_or_else(|| "127.0.0.1:8090".to_string()),
            database_dsn: env("APEIRON_DATABASE_DSN")
                .unwrap_or_else(|| "sqlite://./data/apeiron.db".to_string()),
            platform_url: env("APEIRON_PLATFORM_URL")
                .unwrap_or_else(|| "http://127.0.0.1:8080".to_string())
                .trim_end_matches('/')
                .to_string(),
            bridge_service_token: env("APEIRON_BRIDGE_SERVICE_TOKEN"),
            worker_token: env("APEIRON_WORKER_TOKEN"),
            assets_dir: env("APEIRON_ASSETS_DIR").unwrap_or_else(|| "./data/assets".to_string()),
            user_active_runs: env_u64("APEIRON_USER_ACTIVE_RUNS", 2) as i64,
            global_active_runs: env_u64("APEIRON_GLOBAL_ACTIVE_RUNS", 8) as i64,
            llm_timeout_ms: env_u64("APEIRON_LLM_TIMEOUT_MS", 120_000),
            image_timeout_ms: env_u64("APEIRON_IMAGE_TIMEOUT_MS", 300_000),
            video_timeout_ms: env_u64("APEIRON_VIDEO_TIMEOUT_MS", 1_800_000),
            tts_timeout_ms: env_u64("APEIRON_TTS_TIMEOUT_MS", 300_000),
            material_timeout_ms: env_u64("APEIRON_MATERIAL_TIMEOUT_MS", 300_000),
            assemble_timeout_ms: env_u64("APEIRON_ASSEMBLE_TIMEOUT_MS", 1_800_000),
            sse_max_per_user: env_u64("APEIRON_SSE_MAX_CONNECTIONS_PER_USER", 5) as usize,
        }
    }
}
