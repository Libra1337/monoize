# Studio Bridge Specification

## 0. Status

- **Subsystem:** Platform-side bridge that hands dashboard users over to the standalone
  Apeiron video studio and settles Apeiron usage against platform wallets.
- **Scope:** `src/studio_bridge.rs`, routes `/studio-entry` and `/api/studio-bridge/*`,
  migration `m20260922_000122_studio_bridge_ops`, removal of the embedded studio
  (migration `m20260922_000123_drop_studio_workflow`).
- **Dependency:** `dashboard-session-authentication.spec.md`, `metered-billing.spec.md`,
  `apeiron-video-studio.spec.md` (the consuming service).

## 1. Configuration

SB-1. Environment variables:

| Variable | Meaning | Default |
|---|---|---|
| `MONOIZE_STUDIO_BRIDGE_SECRET` | HMAC-SHA256 key for handoff token minting | unset → bridge disabled |
| `MONOIZE_STUDIO_BRIDGE_SERVICE_TOKEN` | Bearer token accepted on `/api/studio-bridge/*` | unset → service endpoints 404 |
| `MONOIZE_APEIRON_URL` | Base URL of the standalone Apeiron site | `http://127.0.0.1:8090` |

SB-2. When `MONOIZE_STUDIO_BRIDGE_SECRET` is unset, `GET /studio-entry` responds
HTTP 503 with code `bridge_disabled`. When `MONOIZE_STUDIO_BRIDGE_SERVICE_TOKEN`
is unset, every `/api/studio-bridge/*` endpoint responds HTTP 404 with code
`bridge_disabled`. Parsing follows RRB-C1 (positive-integer rule not applicable;
values are opaque strings).

## 2. Handoff token

SB-3. Format: `v1.<payload_b64url>.<sig_hex>` where `payload_b64url` is base64url
(no padding) of `{"sub": <user_id>, "exp": <unix seconds>, "nonce": <uuid v4>}` and
`sig_hex` is the lowercase hex HMAC-SHA256 of `"apeiron-handoff.v1\n" +
payload_b64url` keyed by `MONOIZE_STUDIO_BRIDGE_SECRET`.

SB-4. Lifetime: exactly 120 seconds from mint. A token with `exp <= now` is
rejected with HTTP 410 `expired_token`.

SB-5. Single use is enforced best-effort: the 1024 most recently exchanged nonce
hashes are kept in memory; a replay within that window is rejected with HTTP 409
`replayed_token`. (Multi-instance deployments may not share the window; the 120 s
TTL bounds exposure.)

## 3. Endpoints

SB-6. `GET /studio-entry` — authenticated by dashboard session (cookie
`monoize_session` or `Authorization: Bearer <session token>`). On success responds
HTTP 302 with `Location: {MONOIZE_APEIRON_URL}/handoff?token=<handoff_token>`.
Without a valid session responds HTTP 302 with
`Location: /login?next=%2Fstudio-entry`.

SB-7. `POST /api/studio-bridge/exchange` — service bearer auth. Body
`{"token": string}`. On success responds HTTP 200
`{"user_id", "username", "display_name", "role", "balance_nano_usd": string,
"balance_unlimited": bool}`. Errors: 401 `unauthorized`, 400 `invalid_token`,
410 `expired_token`, 409 `replayed_token`, 404 `user_not_found`. Disabled or
non-enabled users resolve to `user_not_found`.

SB-8. `GET /api/studio-bridge/balance?user_id=<id>` — service bearer auth.
Responds HTTP 200 `{"balance_nano_usd": string, "balance_unlimited": bool}`;
404 `user_not_found` otherwise.

SB-9. `POST /api/studio-bridge/debit` — service bearer auth. Body
`{"user_id": string, "amount_nano_usd": i64 >= 0, "idempotency_key": string (1..=128),
"reason": string, "meta": object}`. Behavior:

1. Insert `(idempotency_key, user_id, "debit", amount, status="pending")` into
   `studio_bridge_ops` (primary key `idempotency_key`).
2. Unique violation → read existing row: `applied` → 200 `{"applied": true,
   "duplicate": true}`; `pending` → 409 `operation_in_progress`; `failed` →
   last stored error (409 `insufficient_balance` or 500 class).
3. Fresh row → call `UserStore::studio_charge_balance(user_id, amount,
   "apeiron_charge", meta.merged.request_id = idempotency_key)`.
   On success mark `applied` → 200 `{"applied": true, "duplicate": false}`.
   On `insufficient_balance` mark `failed`, store `insufficient_balance` →
   HTTP 409 `insufficient_balance`. Other errors mark `failed` → HTTP 500.
   `amount_nano_usd == 0` short-circuits to `{"applied": true}` without a
   ledger row.

SB-10. `POST /api/studio-bridge/refund` — same shape and state machine as SB-9
with kind `refund`, calling `UserStore::studio_refund_balance` with ledger reason
`apeiron_refund`. A refund can never fail with `insufficient_balance`.

SB-11. Ledger contract: bridge debits/credits MUST appear in `billing_ledger`
with kinds `apeiron_charge` / `apeiron_refund` and `meta.request_id` equal to the
caller's `idempotency_key`.

SB-12. Service bearer comparison is SHA-256 digest equality, constant time
(same construction as the replica ingest token).

## 4. Removal of the embedded studio

SB-13. The embedded studio subsystem is removed in the same change:

- `src/studio/*` and `src/studio-templates.catalog.json` are deleted.
- Routes removed: every `/api/dashboard/studio/*`, every
  `/api/dashboard/admin/studio/*`, `/v1/videos*` and the unprefixed `/videos*`
  twins.
- Tables `studio_projects`, `studio_templates`, `studio_runs`, `studio_steps`,
  `studio_assets` and their indexes are dropped by migration
  `m20260922_000123_drop_studio_workflow`. No compatibility aliases remain.
- Frontend: `pages/studio*`, `pages/studio-admin.tsx`, `lib/studio-api.ts` are
  deleted; the `nav.studio` entry becomes an external link opening
  `/studio-entry` in a new tab; the `nav.studioAdmin` entry is removed.
- The `openai_video` / `fal_video` provider channel types remain registered as
  dormant channel kinds (configurable, never chat-routable); their dispatch
  consumers now live in the Apeiron service, which owns its own provider
  registry.

SB-14. The `MONOIZE_STUDIO_*` environment family no longer exists on the
platform; the equivalent knobs are `APEIRON_*` variables defined in
`apeiron-video-studio.spec.md` §11.
