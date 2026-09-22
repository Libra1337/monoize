# Apeiron — standalone video studio

Apeiron is the standalone AI video creation website (`spec/apeiron-video-studio.spec.md`).
It is fully independent from the Monoize platform except for two couplings:

1. Users arrive through the platform redirect `GET /studio-entry` (handoff token).
2. Usage settles against the platform wallet through `/api/studio-bridge/*`.

## Components

| Path | Language | Role |
| --- | --- | --- |
| `server/` | Rust (axum + sqlx/SQLite) | SPA host, sessions, graph engine, provider adapters, worker queue, SSE |
| `worker/` | Go | Claimed jobs: TTS, stock material fetch, subtitle build, ffmpeg assembly |
| `web/` | React 19 + Vite + Tailwind | All pages, the node canvas |

## Build

```bash
# web (dist is embedded into the server binary)
cd web && bun install && bun run build

# server
cd server && cargo build --release

# worker (requires ffmpeg + ffprobe on PATH)
cd worker && go build -o apeiron-worker .
```

## Run

```bash
export APEIRON_LISTEN=127.0.0.1:8090
export APEIRON_DATABASE_DSN=sqlite://./data/apeiron.db
export APEIRON_ASSETS_DIR=./data/assets
export APEIRON_PLATFORM_URL=https://<platform-origin>
export APEIRON_BRIDGE_SERVICE_TOKEN=<same as platform MONOIZE_STUDIO_BRIDGE_SERVICE_TOKEN>
export APEIRON_WORKER_TOKEN=<random worker secret>

./target/release/apeiron-server          # terminal 1
APEIRON_SERVER_URL=http://127.0.0.1:8090 \
APEIRON_WORKER_TOKEN=<same> \
./apeiron-worker                          # terminal 2 (ffmpeg host)
```

Platform side env:

```bash
export MONOIZE_STUDIO_BRIDGE_SECRET=<random hex>          # signs handoff tokens
export MONOIZE_STUDIO_BRIDGE_SERVICE_TOKEN=<same as APEIRON_BRIDGE_SERVICE_TOKEN>
export MONOIZE_APEIRON_URL=https://<apeiron-origin>
```

## First-run operator checklist

1. Open Apeiron through the platform (`Video Studio` nav item, or `/studio-entry`).
2. In `Admin → Providers`, add at least: one `llm` provider (OpenAI-compatible),
   one `material` provider (Pexels key) for stock mode, optionally `image`, `video`
   (`openai_video` / `fal_video` / `replicate`), and `tts` (`openai_tts`).
   LLM billing reads `params.in_price_nano_per_mtok` / `out_price_nano_per_mtok`.
3. Review `Admin → Prices` (defaults: image $0.04, video $0.50, tts $0.02,
   assemble $0.01, material/subtitle free).
4. Start the Go worker on a host with `ffmpeg`; `Manager` shows pack and provider state.
5. Try `Create` (one-click film), then `Projects → canvas` for node editing.

## Local development

```bash
cd web && bun run dev          # Vite dev server on :5173, proxies /api → :8090
cd server && cargo run         # API + engine (no embedded SPA needed)
```
