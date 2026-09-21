# OpenAI Video Upstream Provider Specification

## 0. Status

- **Subsystem:** `openai_video` upstream channel type for Sora-2-compatible video generation APIs.
- **Scope:** `src/studio/video_upstream.rs` (openai_video arm), provider type registration, `ProviderDialog` channel-type option.
- **Dependency:** `studio-workflow.spec.md` (ST-E2, ST-E3), `monoize-upstream-routing.spec.md` (eligibility), `upstream-error-sanitization.spec.md`.

## 1. Provider type

OV-1. `MonoizeProviderType::OpenaiVideo` and `config::ProviderType::OpenaiVideo` are added. Channel configuration: `base_url` (root hosting `/videos`), `api_key` (Bearer). The type MUST NOT be selectable for chat/embedding routing (ST-E4).

## 2. Submit

OV-2. `POST {base}/videos` with JSON `{"model": string, "prompt": string, "seconds": string?, "size": string?, "input_reference": data-URL?}` (omitting empty optional fields). The gateway MUST percent-encode nothing in the path; the path is fixed.

OV-3. A 2xx response with a non-empty `id` (fields `id`, `status?`, `progress?`) marks acceptance. HTTP 429/5xx/network failure is submit-failure (retry per ST-E3). HTTP 4xx is a hard failure surfaced as `upstream_rejected`.

OV-4. The gateway stores the upstream job id and the raw submit response for the step `result_json`.

## 3. Poll

OV-P1. `GET {base}/videos/{id}` at the ST-S5 cadence. Response fields read: `status`, `progress` (0–100 integer), `error`/`error.message`.

OV-P2. Status normalization (case-insensitive):

| Upstream status | Step state |
|---|---|
| `queued`, `pending`, `in_progress`, `processing` | running (progress forwarded) |
| `completed`, `succeeded`, `success` | succeeded |
| `failed`, `error`, `cancelled`, `canceled` | failed / canceled |

Unknown statuses MUST be treated as `running` and logged once per step.

OV-P3. Terminal responses SHOULD embed output metadata. The gateway extracts content via the content endpoint (OV-C1) rather than trusting inline URLs, but MUST record any inline `output[].url` as a fallback remote ref.

## 4. Cancel and content

OV-C1. `GET {base}/videos/{id}/content` streams the media (expected `video/mp4`; any `video/*` or `application/octet-stream` accepted). Streaming follows ST-E5 (64 KiB chunks, size cap).

OV-C2. `POST {base}/videos/{id}/cancel` is best-effort (ST-S7). 404/409 on cancel MUST NOT fail the local cancellation.

OV-C3. An `Authorization: Bearer <api_key>` header applies to every call. Response bodies MUST pass through upstream error sanitization before reaching users.

OV-C4. Poll timeout is the step timeout (ST-S4 video default 1800 s). Timeout → step `failed` + refund.
