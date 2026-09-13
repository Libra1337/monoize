# Content Firewall Specification

## 0. Scope

- Product name: Monoize.
- Subsystem: request-side prohibited-word firewall for all model-forwarding HTTP
  endpoints (`src/content_firewall.rs`, enforcement in `src/handlers/`).
- Purpose: reject any request whose scanned text contains a configured
  prohibited term (default list: NSFW and child-sexual-abuse zero-tolerance
  terms, bilingual zh/en) before any upstream model call is made. This mirrors
  gateway practice domestic and abroad: open-source gateways (one-api/new-api)
  intercept by keyword and are migrating to Aho-Corasick matching; hosted
  gateways (LiteLLM guardrails, agentgateway) layer operator-defined blocked
  keyword lists under ML classifiers; child-sexual-content is a zero-tolerance
  category for every major provider.
- Non-goals: ML-based classification, image/audio byte scanning, output-side
  (response) moderation, per-caller exemptions.

## 1. Settings

CF-1. Two rows in `system_settings` configure the firewall:

| key | type | default |
|-----|------|---------|
| `moderation_enabled` | bool | `true` |
| `moderation_blocked_words` | text (newline-separated terms) | the built-in list of CF-3 |

CF-2. `moderation_blocked_words` is stored verbatim as received from the admin
API. Canonicalization happens at compile time only (CF-4) and never rewrites
the stored value.

CF-3. The built-in default term list is exactly (one term per line, 18 terms):

```
porn
child erotica
child nude
csam
csem
sexualized minors
underage nude
色情
黄片
成人片
儿童裸照
儿童裸体
未成年裸照
未成年裸体
嫖宿幼女
强奸幼女
猥亵儿童
裸聊
```

CF-4. Term canonicalization: split the raw value on `\n`, trim leading and
trailing Unicode whitespace from each line, drop empty lines, lowercase each
term with Unicode simple case folding (`str::to_lowercase`), drop duplicate
terms preserving first occurrence.

CF-5. The compiled matcher is an Aho-Corasick automaton over the canonicalized
terms. Matching is leftmost substring matching against the Unicode-lowercased
scanned string. Word boundaries are NOT required: a term matches inside any
surrounding text. When the canonicalized term list is empty, no automaton
exists and no scanning occurs.

CF-6. A request is rejected if and only if
`moderation_enabled == true` AND the compiled automaton exists AND at least one
scanned string of the request contains at least one term.

## 2. Scanned text set

CF-7. For `POST /v1/chat/completions` (and its `/chat/completions` alias), the
WebSocket responses path, and `POST /v1/completions` (translated to a chat
completion before dispatch): the scanned strings are, of the decoded
`urp::UrpRequest` input, every `Node::Text.content`, `Node::Refusal.content`,
`Node::Reasoning.content`, `Node::Reasoning.summary`, and
`Node::ToolCall.arguments` value.

CF-8. For `POST /v1/responses` (and its `/responses` alias): the scanned
strings are the same node-derived set as CF-7.

CF-9. For `POST /v1/messages` (and its `/messages` alias): the scanned strings
are the same node-derived set as CF-7.

CF-10. For `POST /v1/responses/compact`: the scanned strings are the same
node-derived set as CF-7 applied to the decoded routing request.

CF-11. For `POST /v1/embeddings`: the scanned strings are the `input` string,
or every string element when `input` is an array of strings.

CF-12. For `POST /v1/images/generations`: the scanned string is the `prompt`
field. For `POST /v1/images/edits`: the scanned string is the `prompt`
multipart field.

CF-13. Binary node payloads (`Node::Image`, `Node::Audio`, `Node::File`
sources) are not decoded and not scanned. Model names, tool names, and JSON
field names outside the CF-7..CF-12 strings are not scanned.

## 3. Enforcement

CF-14. The firewall check runs in every handler of CF-7..CF-12 after tenant
authentication succeeds, after model redirects and the model allowlist check,
and before any of: attempt building, the balance/quota gate, request-log
admission, and any upstream network I/O. A rejected request performs zero
upstream network calls and consumes no billing.

CF-15. A rejection returns HTTP `403 Forbidden` with the standard OpenAI-style
error envelope:

```json
{"error": {"message": "request blocked by content firewall: prohibited term '<term>'", "type": "content_policy_violation", "param": null, "code": "content_blocked"}}
```

where `<term>` is the canonicalized term that matched. On `/v1/messages` the
same status, code, type, and message are wrapped in the Anthropic error
envelope by the existing handler wrapper. On the responses WebSocket the same
fields are delivered through the existing WebSocket error event path.

CF-16. No bypass exists. The firewall applies to every authenticated caller
(API keys, dashboard sessions, playground, internal sources) regardless of
role, including administrators. No per-API-key, per-user, per-role,
per-provider, or per-model exemption exists, and no request field can disable
or weaken the check.

CF-17. Each rejection emits exactly one `tracing::warn` record containing the
authenticated user id (when present), the API key id (when present), and the
matched term, and persists exactly one firewall-event row (CF-20, CF-21).
Moderation rejections do not create request-log rows, matching the existing
pre-forward rejection behavior (for example `model_not_allowed`).

## 4. Runtime propagation

CF-18. The compiled automaton lives in the shared runtime snapshot
(`MonoizeRuntimeConfig`). It is rebuilt: at process start from stored settings,
on the primary after a successful `PUT /api/dashboard/settings` that changes
either setting, and on replicas when the config epoch changes. Updates take
effect on requests received after the rebuild, without restart.

## 5. Admin UI

CF-19. The admin settings page exposes the two settings in one dedicated
category `moderation` (see `system-settings-ui.spec.md` SSU-1): one switch for
`moderation_enabled` and one multi-line textarea for
`moderation_blocked_words` (one term per line).

## 6. Event persistence

CF-20. Every rejection persists exactly one row into the `firewall_events`
table with columns: `id` (uuid text, primary key), `user_id`
(text, null when absent), `username` (text, null when absent), `api_key_id`
(text, null when absent), `api_key_name` (text, null when absent), `endpoint`
(text), `model` (text), `term` (text, canonicalized matched term), `content`
(text), `created_at` (RFC 3339 UTC text), `created_at_unix_ms` (integer).
The event store is deployment-global; row visibility for the CF-24/CF-25
reads is admin-only, so no per-tenant scoping column is needed.

CF-21. `endpoint` is one of: `chat_completions`, `responses`, `messages`,
`responses_compact`, `embeddings`, `images_generations`, `images_edits`. The
WebSocket responses path records `responses`. `content` is the scanned string
that matched, truncated to at most 8000 Unicode scalar values, unchanged
otherwise. The events list API returns `content` in full; the dashboard shows
a truncated summary in the table and the full text in a click-open dialog.

CF-22. A persistence failure MUST NOT change the rejection outcome: the client
still receives the CF-15 response and the failure is logged as a warning.

CF-23. The table retains at most the 50,000 newest rows per process-wide table
state; after each insert the system deletes rows beyond that cap ordered by
`created_at_unix_ms` ascending.

## 7. Firewall dashboard page and APIs

CF-24. `GET /api/dashboard/firewall/stats` (admin session required) returns
JSON: `total`, `last_24h`, `last_7d`, `distinct_users` (all counts over the
whole deployment), `daily` (exactly 14 entries, oldest first,
each `{date: "YYYY-MM-DD", count}` over UTC days ending with the current UTC
day), and `top_terms` (at most 10 `{term, count}` entries, descending count
over all retained rows, ties broken by ascending term). All aggregation is
computed server-side from retained rows.

CF-25. `GET /api/dashboard/firewall/events` (admin session required) returns
`{data: [rows], total, limit, offset}` ordered by `created_at_unix_ms`
descending over the whole deployment, with query parameters
`limit` (default 50, clamped to 1..200), `offset` (default 0), `term`
(optional substring filter), `since_ms` and `until_ms` (optional inclusive
`created_at_unix_ms` bounds). Non-admin callers receive HTTP 403.

CF-26. The dashboard renders an admin-only page at `/dashboard/firewall`
(labeled by i18n key `nav.firewall`) containing, in order: a stat-tile row
(`last_24h`, `last_7d`, `total`, `distinct_users`); a 14-day bar chart of
`daily`; a `top_terms` list; and the events table of CF-25 with a term search
box and a time-range selector (`24h`, `7d`, `30d`, all), paginated through
`limit`/`offset`. Every data region has a skeleton loading state and an empty
state; navigation and data fetching follow SWR per `dashboard-ui-layout.spec.md`.

CF-27. The page and both APIs expose no mutation surface: firewall
configuration changes only through the CF-19 settings category.
