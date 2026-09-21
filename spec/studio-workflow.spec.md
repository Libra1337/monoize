# Studio Creation Workflow Specification

## 0. Status

- **Subsystem:** Studio canvas creation workbench (image + video generation with agent orchestration).
- **Scope:** `src/studio/*`, studio dashboard/admin handlers, public `/v1/videos` API, frontend `/dashboard/studio` pages, studio background engine.
- **Dependency:** `api-key-authentication.spec.md`, `metered-billing.spec.md`, `monoize-upstream-routing.spec.md` (channel eligibility), `image-api-proxy.spec.md` (image semantics), `openai-video-upstream.spec.md`, `fal-video-upstream.spec.md`, `replicate-upstream.spec.md` §10, `dashboard-session-authentication.spec.md`, `runtime-resource-bounds.spec.md`.

## 1. Terminology

- **Project:** a user-owned canvas document. Identified by `project_id` (UUID). Contains a **Graph**.
- **Graph:** JSON document `{ "nodes": [...], "edges": [...], "version": n }`. `version` is a monotonic per-project counter incremented on every accepted mutation.
- **Node kinds:** `script`, `shot`, `image`, `video`, `note`. Node shape: `{ "id": string, "kind": string, "position": {"x": f64, "y": f64}, "data": object }`.
- **Edge kinds:** `contains` (script→shot), `consistency` (image→shot, reference sheet), `image_to_video` (image→video), `probe` (video→video, extrapolation clip).
- **Run:** one orchestrated execution (agent pipeline, script pipeline, work order, or public API job). Identified by `run_id` (UUID).
- **Step:** one billable unit inside a run. Kinds: `llm`, `image`, `video`.
- **Template:** a reusable project graph + parameter preset + price declaration. Sources: builtin catalog (`src/studio-templates.catalog.json`) and admin-defined.
- **Asset:** a generated output registered for proxy serving. Kinds: `image`, `video`.
- **Work order:** a single-step run created from the canvas toolbar without the agent conversation.

## 2. Data model

ST-D1. Tables (migration `m20260922_000121_studio_workflow`):

| Table | Key columns |
|---|---|
| `studio_projects` | `id` PK, `user_id`, `title`, `graph_json` TEXT, `version` INT, `template_id` NULL, `created_at`, `updated_at` |
| `studio_templates` | `id` TEXT PK, `source` (`builtin`\|`custom`), `name`, `description`, `graph_json` TEXT, `params_json` TEXT, `price_nano_usd_map` TEXT (JSON: step-kind → nano-USD), `enabled` BOOL, `created_at`, `updated_at` |
| `studio_runs` | `id` PK, `user_id`, `project_id` NULL, `api_key_id` NULL, `kind` (`agent`\|`pipeline`\|`work_order`\|`public_api`), `status`, `error` NULL, `created_at`, `updated_at`, `finished_at` NULL |
| `studio_steps` | `id` PK, `run_id` FK, `kind`, `status`, `node_id` NULL, `payload_json` TEXT (resolved inputs), `result_json` NULL, `error` NULL, `attempts` INT, `charge_nano_usd` INT, `refund_nano_usd` INT, `created_at`, `updated_at`, `finished_at` NULL |
| `studio_assets` | `id` PK, `user_id`, `run_id` FK, `step_id` FK, `kind`, `upstream_kind`, `provider_id` NULL, `remote_ref_json` TEXT, `mime_type`, `bytes` NULL, `status`, `created_at` |

ST-D2. `studio_projects.version` MUST start at 1 on creation and increment by exactly 1 per accepted graph mutation. A save request carrying `version <= current` MUST be rejected with HTTP 409 and the current graph in the response body.

ST-D3. Builtin templates MUST be upserted at startup: for an existing `id`, graph/params follow the catalog revision while admin-set `enabled` and price overrides are preserved. Custom templates are never overwritten by the catalog.

## 3. Run and step state machine

ST-S1. Run statuses: `queued`, `running`, `succeeded`, `failed`, `canceled`, `partial`. Step statuses: `pending`, `running`, `succeeded`, `failed`, `canceled`, `skipped`.

ST-S2. Legal step transitions: `pending→running→{succeeded|failed|canceled}`, `pending→canceled`, `pending→skipped`. Any other transition MUST be rejected by the engine.

ST-S3. Run terminal semantics: a run is `succeeded` iff all non-skipped steps succeeded; `failed` iff any step failed and none were resumable-pending; `partial` iff the run is a pipeline whose shots are independently terminal with mixed outcomes (per-shot independence, see §5); `canceled` by user action.

ST-S4. Step timeouts, overridable per template via `params_json`: `llm` 60 s, `image` 300 s, `video` 1800 s. Environment overrides: `MONOIZE_STUDIO_LLM_TIMEOUT_MS`, `MONOIZE_STUDIO_IMAGE_TIMEOUT_MS`, `MONOIZE_STUDIO_VIDEO_TIMEOUT_MS` (positive integers, RRB-C1 parsing).

ST-S5. The background engine MUST poll upstream only for steps in `running` state, with per-step backoff 3 s growing by 1.5× up to 15 s. With zero active steps the engine MUST issue zero upstream requests and zero timers beyond one 1 s scheduler tick.

ST-S6. Concurrency admission: a user MAY hold at most `MONOIZE_STUDIO_USER_ACTIVE_RUNS` (default 2) runs in non-terminal state; the process MAY hold at most `MONOIZE_STUDIO_GLOBAL_ACTIVE_RUNS` (default 16). Exceeding either MUST reject submission with HTTP 429 `studio_busy`.

ST-S7. Cancellation: for `pending` steps the engine marks `canceled` locally. For `running` video steps the engine MUST call the upstream cancel path (OV-C4 / FV-C4 / §10 of `replicate-upstream`) on a best-effort basis and MUST NOT wait for upstream confirmation.

## 4. Execution surfaces and channel selection

ST-E1. Image steps reuse the OpenAI Images request shape (`POST {base}/v1/images/generations`, `POST {base}/v1/images/edits`) against providers of channel type `openai_image` selected by existing channel eligibility (enabled, model present in the channel model map, weight > 0, breaker state).

ST-E2. Video steps dispatch by the node's declared upstream kind: `openai_video` per `openai-video-upstream.spec.md`, `fal_video` per `fal-video-upstream.spec.md`, `replicate` per `replicate-upstream.spec.md` §10.

ST-E3. Submit-time failure (HTTP 429/5xx/network) MAY retry on the next eligible channel, bounded at 2 attempts total. After an upstream accepts a job (an upstream job id exists), a later failure MUST NOT be resubmitted automatically; the step fails and refunds.

ST-E4. `openai_video` and `fal_video` provider types MUST NOT appear as candidates for chat/embedding routing (the URP waterfall). They are addressable only by studio steps and `/v1/videos`.

ST-E5. Output media MUST be served through gateway streaming proxy in 64 KiB chunks. The engine MUST NOT buffer an entire asset in memory. Single-asset streaming MUST abort with HTTP 502 `asset_too_large` after `MONOIZE_STUDIO_ASSET_MAX_BYTES` (default 209715200) streamed bytes. Uploads (reference images) MUST be capped at `MONOIZE_STUDIO_UPLOAD_MAX_BYTES` (default 20971520) and streamed to upstream.

## 5. Script pipeline (six stages)

ST-P1. The `script` node hosts stages, each materializing graph nodes:

| Stage | Action | Step kinds |
|---|---|---|
| 1 draft | LLM generates screenplay draft into `script.data.text` | llm |
| 2 split | LLM splits draft into shots; creates `shot` nodes + `contains` edges | llm |
| 3 synthesize | LLM writes per-shot prompts from draft + sheet descriptions into `shot.data.prompt` | llm |
| 4 sheets | batch reference images (character/scene/prop) per sheet entries | image |
| 5 frames | batch first-frame images per shot; links `image` nodes with `consistency` edges | image |
| 6 videos | batch per-shot videos from first frames via `image_to_video` | video |

ST-P2. Stages 1–3 operate on editable text: the user MAY edit `script.data.text` or any `shot.data` field between stages and MAY re-run any stage; re-running a stage MUST NOT reset unrelated nodes.

ST-P3. Stages 4–6 are per-shot independent: a failed shot records its step `failed` without blocking siblings; the pipeline run ends `partial` if any shot failed and any succeeded.

ST-P4. Stage 6 MUST run shots serially (concurrency 1) unless the template sets `video_parallelism` (max 3).

ST-P5. Each stage execution MUST present an aggregated cost estimate to the dashboard before dispatch when the stage creates billable steps; the estimate uses the same price function as §6.

## 6. Billing

ST-B1. Price function for `image`/`video` steps: `effective_nano = ceil(base_nano × M)` where `base_nano` is the template/node declared price for the step kind and `M` is the selected channel's provider `multiplier` (default 1.0 when absent or unparseable).

ST-B2. The wallet MUST be charged when a step transitions `pending→running` (pre-charge). Insufficient balance MUST fail the step before dispatch with `insufficient_balance` and no charge.

ST-B3. A step ending `failed` or `canceled` MUST be refunded in full. Refunds MUST be idempotent per step (a step refunds at most once). Ledger reasons: `studio_llm_charge`, `studio_image_charge`, `studio_video_charge`, `studio_*_refund` with `meta.request_id = "studio_step_<step_id>"`.

ST-B4. `llm` steps are charged by actual token usage: `cost = input_tokens × in_rate + output_tokens × out_rate` from the rate snapshot resolved for `studio_agent_model` at call time, converted to nano-USD. Pre-charge for llm steps is 0; settlement occurs on completion.

ST-B5. Public API (`/v1/videos`) jobs charge the key owner's wallet with the same price function; no key window limits apply in this version (future work, see §12).

## 7. Agent protocol

ST-A1. The agent conversation runs server-side: an LLM step loop with function calling against the user's `studio_agent_model` (system setting, default from the catalog's cheapest chat model present in any enabled channel).

ST-A2. Tools exposed to the model (JSON-schema declared):

| Tool | Effect |
|---|---|
| `update_script` | sets `script.data.text` |
| `create_shots` | replaces shot nodes from a structured list |
| `update_shot` | edits one shot's fields |
| `generate_images` | creates image steps for shot ids or a standalone prompt |
| `generate_videos` | creates video steps for shot ids |
| `run_pipeline` | executes stages 1–6 in order with default settings |

ST-A3. The loop MUST stop after 12 tool iterations or when the model returns a final message without tool calls. Every tool call MUST mutate the graph server-side and broadcast a `graph_patch` event (§9).

ST-A4. The system prompt MUST include a compact graph summary (node kinds, shot count, per-shot one-line status). The agent MUST NOT be able to mutate nodes outside its project or bypass step billing.

ST-A5. Agent text streams to the dashboard via SSE token deltas; tool invocations surface as `agent_tool` events with tool name and argument digest.

## 8. Public API `/v1/videos`

ST-V1. Endpoints (mounted with and without the `/v1` prefix, per gateway convention), authenticated by API key (`auth_tenant`):

- `POST /v1/videos` — body `{"model": string, "prompt": string, "seconds": string?, "size": string?, "image": string? (URL or base64 data URL), "params": object?}`. `model` MUST resolve to a template id or a template-declared upstream model name. Creates a `public_api` run with one `video` step. Returns HTTP 201 with the run object.
- `GET /v1/videos/{id}` — returns `{"id", "object": "video.job", "model", "status": "queued"|"in_progress"|"succeeded"|"failed"|"canceled", "progress": 0-100, "error": string?, "created_at", "completed_at"?, "price_nano_usd"}`.
- `POST /v1/videos/{id}/cancel` — best-effort cancel; returns the run object.
- `GET /v1/videos/{id}/content` — streams the first video asset (HTTP 200, upstream MIME); 404 `no_output` if not succeeded; 409 `not_terminal` if still active.

ST-V2. Status mapping: run `queued` → `queued`; any `running` step → `in_progress` (progress = step progress when the upstream reports one, else elapsed/timeout heuristic); `succeeded` → `succeeded`; `failed`/`partial` → `failed`; `canceled` → `canceled`.

ST-V3. The submit endpoint MUST apply the content firewall to `prompt` and reject with the existing firewall error semantics.

ST-V4. Errors use `{"error": {"message", "type": "studio_error", "code"}}` with codes: `model_not_found`, `invalid_params`, `insufficient_balance`, `studio_busy`, `firewall_blocked`.

## 9. SSE events

ST-X1. Endpoint `GET /api/dashboard/studio/events` (session auth) streams events scoped to the current user: `graph_patch` `{project_id, version, patch}`, `run_update` `{run}`, `step_update` `{step}`, `agent_tool` `{run_id, tool, digest}`, `agent_delta` `{run_id, text}`.

ST-X2. The broadcast channel MUST be bounded (capacity 256, drop-oldest) and per-user connection cap applies (reuse the request-logs SSE pattern, default 5, `MONOIZE_STUDIO_SSE_MAX_CONNECTIONS_PER_USER`).

## 10. Dashboard and admin surface

ST-U1. Session-authenticated dashboard endpoints under `/api/dashboard/studio`: projects CRUD + graph save (versioned), runs list/detail, run cancel, step retry (creates a new step for the same node), assets list, reference upload, `events` SSE, templates list, agent conversation submit.

ST-U2. Admin endpoints under `/api/dashboard/admin/studio` (`require_admin`): template create/update/delete (custom only delete), price/enable overrides, all-users runs monitor + cancel, settings (`studio_agent_model`, stage prompt templates, default video upstream kind).

ST-U3. Frontend pages: `/dashboard/studio` (project list), `/dashboard/studio/p/:id` (canvas workbench), `/dashboard/studio/assets`, `/dashboard/studio-admin`. Navigation entries `nav.studio` (user set) and `nav.studioAdmin` (admin set). UI MUST follow `frontend-design-system.spec.md` tokens (no new colors or fonts).

## 11. Resource bounds registration

ST-R1. New environment limits registered in `runtime-resource-bounds.spec.md` §N: the `MONOIZE_STUDIO_*` family defined in this spec. All parse per RRB-C1.

## 12. Future work (non-normative)

Go `studio-bridge` + ComfyUI executor (Phase 2), canvas pixel editing, timeline export/BGM, external MCP endpoint, key window-limit integration for `/v1/videos`, B2 catalog snapshot optimization.
