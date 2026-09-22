# fal.ai Video Upstream Provider Specification

## 0. Status

- **Subsystem:** `fal_video` upstream channel type for the fal.ai queue API.
- **Scope:** `src/studio/video_upstream.rs` (fal arm), provider type registration, `ProviderDialog` channel-type option.
- **Dependency:** `apeiron-video-studio.spec.md` (AP-E7 — dispatch now lives in the standalone Apeiron service), `upstream-error-sanitization.spec.md`. The platform keeps the channel type registered as a dormant kind (SB-13).

## 1. Provider type

FV-1. `MonoizeProviderType::FalVideo` and `config::ProviderType::FalVideo` are added. Channel configuration: `base_url` (default `https://queue.fal.run`), `api_key` (used verbatim as `Authorization: Key <api_key>`). The type MUST NOT be selectable for chat/embedding routing (ST-E4).

FV-2. The model key stored in the channel's model map is appended to the base URL as a single percent-encoded path segment: `POST {base}/{model}`.

## 2. Submit

FV-S1. `POST {base}/{model}` with the JSON input `{"prompt": string, "image_url": string?, "duration?": string, "aspect_ratio"?/"resolution"?: string}` (omit empty). A 2xx response with `request_id` marks acceptance; the gateway records `status_url`, `response_url`, `cancel_url?` when present.

FV-S2. Submit failure classification follows OV-3.

## 3. Poll and result

FV-P1. `GET {status_url}` at the ST-S5 cadence. `status` values `QUEUED`/`IN_PROGRESS` → running (`queue_position` forwarded as progress hint); `COMPLETED` → fetch result; any other value → failed.

FV-P2. On `COMPLETED`, `GET {response_url}` once. The video URL is extracted from the first present path: `output.video.url`, `output.url`, `output.videos[0].url`, `output[0].url` (string or `{url}`). The ref is stored as the asset remote ref; inline URLs MUST NOT be exposed to end users (ST-E5 proxy).

## 4. Cancel and content

FV-C1. If the submit response included `cancel_url`, cancellation POSTs it best-effort (ST-S7); otherwise cancellation is local-only and the step stops polling.

FV-C2. Content streams from the extracted media URL with `Authorization: Key <api_key>` where the host is the fal domain; third-party CDN hosts (e.g. `fal.media`) are fetched without credentials. SSRF private-address guards apply to every media fetch.

FV-C3. Poll timeout equals the step timeout (ST-S4 video default 1800 s). Timeout → step `failed` + refund.
