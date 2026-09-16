# Upstream Error Sanitization Specification

## 0. Status

- **Purpose:** Define exactly which parts of an upstream failure are exposed to downstream API clients, which parts are persisted in request logs, which parts each dashboard viewer role may read back, and which parts exist only in the server log.
- **Scope:** Applies to every forwarding endpoint (`/v1/responses`, `/v1/chat/completions`, `/v1/messages`, the Image API, compact, and the Responses WebSocket) for errors derived from upstream attempts, to the request-log persistence of those errors, to the dashboard request-log read paths that return those errors, and to mid-stream downstream error frames.
- **Disclosure tiers.** There are exactly three disclosure tiers:
  1. **Client tier** — downstream API responses and mid-stream frames: masked or generic text only (sections 2, 4, 6).
  2. **Admin tier** — persisted request-log fields as read back by a dashboard user whose role satisfies `request-logs.spec.md` RL-API1 admin access (`admin` or `super_admin`): the full raw upstream detail, bounded only by `TRUNC` (sections 3, 5, 8).
  3. **Non-admin dashboard tier** — persisted request-log fields as read back by any other dashboard user: `MASK` applied at read time (section 8).
  The server tracing log additionally carries the full unbounded raw detail (SAN-3).
- **Reference alignment:** The client tier follows the New API reference implementation (`QuantumNous/new-api`): every client-facing relay error message is masked (`kitutil.MaskSensitiveInfo`), transport failures are replaced by one fixed generic message, and an unparseable upstream error body is never echoed to a client. Monoize deliberately deviates from New API for persisted error-log text: Monoize persists the truncated raw detail so administrators can read the complete upstream error, and enforces masking for non-admin viewers at read time instead of at write time.
- **Relation to other specs:** `monoize-upstream-routing.spec.md` RTA-8/RTA-8a define when the exhausted-routing error is returned; this spec defines its text. `request-logs.spec.md` RL17 defines which attempt fields are persisted and RL-API14 mirrors the read-time disclosure rule; this spec defines the error-text content of those fields. `unified_responses_proxy.spec.md` FP4e renders the client message defined here.

## 1. Definitions

SAN-D1. `MASK(text)` is a pure function on strings. It applies the following four rewrites, in this order, to every non-overlapping match, scanning left to right:

1. **URL masking.** Every substring matching the regex `(http|https)://[^\s/$.?#].[^\s]*` is parsed as a URL. If parsing fails, the substring is kept unchanged. If parsing succeeds, the substring is replaced by `{scheme}://{MASKHOST(host)}{port?}{path'}{query'}` where:
   - `MASKHOST(host)` splits `host` on `.`; if it has fewer than 2 labels the result is `***`; otherwise the result is `***.` followed by the preserved tail. The preserved tail is the last two labels when the last label has length 2 and the second-to-last label has length <= 3 (country-code TLD heuristic, e.g. `co.uk`), otherwise only the last label.
   - `port?` is `:{port}` when the URL carries an explicit port, otherwise empty.
   - `path'`: when the URL path is empty or `/`, `path'` equals the path unchanged; otherwise every `/`-separated non-empty path segment is replaced by `***` and segments are re-joined with `/` after a leading `/`.
   - `query'`: when the URL has no query, `query'` is empty; otherwise every `key=value` pair is replaced by `key=***`, pairs re-joined with `&` after a leading `?`.
2. **Bare domain masking.** Every remaining substring matching `\b(?:[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?\.)+[a-zA-Z]{2,}\b` is replaced by `N` copies of `***.` followed by the preserved tail as in `MASKHOST`, where `N = label_count - tail_label_count`, with a minimum of one `***.` prefix.
3. **IPv4 masking.** Every substring matching `\b(?:\d{1,3}\.){3}\d{1,3}\b` is replaced by `***.***.***.***`.
4. **API-key masking.** Every substring matching `(['"]?)api_key:([^\s'"]+)(['"]?)` is replaced by `{quote}api_key:***{quote}`.

SAN-D2. `DETAIL_LIMIT = 2048`. `TRUNC(text)` equals `text` when `text` contains at most `DETAIL_LIMIT` Unicode scalar values; otherwise it equals the first `DETAIL_LIMIT` scalar values of `text` followed by the literal suffix `... (truncated)`.

SAN-D2a. `QUOTA(error)` is a predicate over the combined text of an upstream error's
`message`, `code`, `type`, and `param` string fields, lowercased and matched on word
boundaries. It holds when any of these exact signals appears:
`insufficient_quota`, `quota_exceeded`, `quota exceeded`, `rate_limit_exceeded`,
`rate limit exceeded`, `rate_limit_error`, `too_many_requests`, `429_resource_exhausted`,
`resource_exhausted`, `daily_quota`, `hourly_quota`, `5 hour quota`, `5-hour quota`,
`per_hour_quota`, `monthly_quota`, `usage_limit_reached`, `usage limit reached`,
`usage_limit_exceeded`, `billing_limit_reached`, `billing_hard_limit_reached`,
`current_quota`, `over quota`, `exceeded your current quota`, `org_monthly_spend_limit`,
`spend_limit_reached`, `credits_exhausted`, `credit_balance_too_low`. `QUOTA` is
model-agnostic and protocol-agnostic: it applies to every upstream provider type and every
downstream protocol. `GENERIC_QUOTA_TEXT` is the fixed string
`upstream provider quota exceeded; please retry later or contact the operator`.
The downstream-facing error `message` for any error where `QUOTA` holds MUST be exactly
`GENERIC_QUOTA_TEXT` (when `monoize_mask_sensitive_info = true`). Numbers inside the raw
text, window sizes (`5 hour`), account identifiers, and provider-specific wording MUST NOT
appear downstream. `internal_message` and every persisted request-log field keep the raw
`TRUNC`-bounded detail unchanged (SAN-2 tier), and the server tracing log keeps the
unbounded raw detail (SAN-3). When `monoize_mask_sensitive_info = false`, `QUOTA` still
applies: quota wording is replaced by `TRUNC(raw message)` under the same SAN-CFG5
deviation style used for other messages, because the quota text is itself the sensitive
surface (window sizes, plan tiers, operator account state), not merely a URL or key.

SAN-D3. Every `UpstreamCallError` carries a `source` classification with exactly these values:

- `transport`: the upstream HTTP request could not be sent or its response body could not be read (connection, TLS, DNS, timeout, and body-read failures).
- `structured_body`: the upstream returned a non-2xx status and the response body parsed as JSON containing a non-empty `error.message` string.
- `unparsed_body`: the upstream returned a non-2xx status and a non-empty body without a parseable `error.message`.
- `empty_body`: the upstream returned a non-2xx status and an empty body.
- `internal`: a Monoize-generated diagnostic (missing provider configuration, request-encoding failure, or JSON-decoding failure of a 2xx body).

Constructors default `source` to `transport` for network-kind errors and `internal` for HTTP-kind errors; the non-2xx response path sets `structured_body`, `unparsed_body`, or `empty_body` explicitly.

## 2. Per-attempt error conversion

SAN-1. When a failed upstream attempt is converted to an `AppError` (`upstream_error_to_app`), with `STATUS` being the upstream HTTP status or `502 Bad Gateway` when absent, the client-facing `message` MUST be exactly:

- `source = transport`: the fixed string `failed to request upstream`.
- `source = unparsed_body` or `empty_body`: `upstream status {STATUS}` (the Display form of the status, e.g. `upstream status 502 Bad Gateway`), with no body content.
- `source = structured_body` or `internal`: `upstream status {STATUS}: ` followed by `MASK(raw message)`.

SAN-2. The same `AppError` MUST set `internal_message` to `upstream status {STATUS}: ` followed by `TRUNC(raw message)`. `MASK` MUST NOT be applied to `internal_message`; it is the admin-tier detail and its read-time disclosure is governed by section 8.

SAN-2a. When `QUOTA` holds for the upstream error, the client-facing `message` produced by
SAN-1 MUST be replaced by `GENERIC_QUOTA_TEXT` regardless of `err.source`, and the
`upstream_code`/`upstream_type`/`upstream_param` diagnostic fields keep their raw values
unchanged (SAN-12). The exhausted-routing composition (SAN-6/SAN-7) MUST apply `QUOTA` to
the last recorded attempt's `client_error` before composing the downstream message, so the
`Last error:` tail of an all-attempts-failed message is `GENERIC_QUOTA_TEXT` when the last
attempt was a quota failure.

SAN-3. Before the conversion in SAN-1, the raw unmasked detail (including transport error text with the full upstream URL and the raw unparsed error body) MUST be written to the server log (tracing, `warn` level) without truncation. The raw unmasked detail MUST NOT appear in any downstream response body or mid-stream frame. Persisted request-log fields carry the `TRUNC`-bounded raw detail per SAN-2, SAN-5, SAN-9, and SAN-10; disclosure of those fields to dashboard viewers is governed by section 8. Request-capture dump files (`request-capture-dumps.spec.md`) are server-local operator artifacts and are exempt.

SAN-4. When a 2xx upstream response embeds a Chat Completions error object (`embedded_chat_completion_error_to_app`), the resulting `AppError.message` MUST be `MASK` of the embedded message, and `AppError.internal_message` MUST be `TRUNC` of the raw embedded message.

SAN-4a. When `QUOTA` holds for the embedded error object, `AppError.message` MUST be
`GENERIC_QUOTA_TEXT` instead of the SAN-4 masked form.

## 3. Attempt recording

SAN-5. Each recorded failed attempt (`TriedProvider`) MUST carry two error strings:

- `error`: the persisted internal detail. It MUST equal `AppError.internal_message` when set, otherwise `TRUNC(AppError.message)`. `MASK` MUST NOT be applied to `error` at write time; read-time disclosure is governed by section 8.
- `client_error`: the client-facing text, equal to `MASK(AppError.message)`. `client_error` MUST NOT be serialized into `tried_providers_json`.

`MASK` is applied unconditionally to `client_error` because attempt failures can also originate from response-decoding `AppError`s that do not pass through SAN-1; masking is idempotent, so re-masking an already-sanitized client message is a fixed point.

## 4. Exhausted-routing downstream error

SAN-6. The downstream `message` of the exhausted-routing error (`monoize-upstream-routing.spec.md` RTA-8) MUST be exactly:

- zero recorded attempts and no Provider serves the model for the caller's account class:
  `Model not found: {model}`.
- zero recorded attempts and at least one Provider serves the model for the caller's account
  class: `No available upstream provider for model: {model}`.
- one or more recorded attempts: `All upstream attempts failed for model: {model}. Last error: {client_error of the last recorded attempt}`.

Neither zero-attempt message may name a Provider, a Channel, or a Group. `Model not found`
discloses only that the model string the caller sent is not routable for that caller, which
the caller already knows.

The downstream message MUST NOT contain the attempt count, provider identifiers, channel identifiers, or upstream URLs.

SAN-7. The exhausted-routing error MUST set `internal_message` to:

- zero recorded attempts: equal to the downstream message.
- one or more recorded attempts: `All {n} upstream attempt(s) failed for model: {model}. Last error: {error of the last recorded attempt}`, where `n` is the recorded attempt count and `{error}` is the unmasked internal detail per SAN-5. Consequently the admin-tier request-log message includes the full last-attempt detail.

SAN-8. Under the RTA-8a exception (`upstream_code = "thinking_signature_invalid"`), the downstream message MUST equal the last recorded attempt's `client_error` and `internal_message` MUST equal the last recorded attempt's `error`.

## 5. Request-log persistence

SAN-9. A terminal error request-log row MUST persist `error_message = AppError.internal_message` when set, otherwise `AppError.message`. The streaming terminal-error log path MUST apply the same rule. Consequently a persisted `error_message` MAY contain raw upstream URLs, bare domains, IPv4 addresses, and `api_key:` values, bounded only by `TRUNC`. The stored value is the admin-tier text; disclosure to dashboard viewers is governed by section 8.

SAN-10. Each `tried_providers_json` entry's `error` field MUST equal the attempt's `error` string per SAN-5 (unmasked, truncated internal detail).

## 6. Mid-stream error frames

SAN-11. When a downstream stream encoder renders a `UrpStreamEvent::Error` into a downstream frame (Chat Completions terminal error `data:` frame, Anthropic Messages `error` event, or Responses `response.failed` payload), the rendered `message` string MUST be `MASK(event message)`. This rule covers decoder-origin mid-stream failures whose text never passes through SAN-1.

SAN-11a. When `QUOTA` holds for the mid-stream error (evaluated over the same message,
code, and extra-body `error` object fields), the rendered downstream `message` MUST be
`GENERIC_QUOTA_TEXT`, and the rendered frame MUST NOT replay the original upstream error
object: any nested `error` object carried in `extra_body` is dropped for the client frame
(because it contains the raw quota text), while the persisted request-log terminal error
keeps the raw `TRUNC`-bounded message unchanged. Like SAN-11, this rule is
protocol-agnostic and applies to every model.

## 7. Diagnostic fields

SAN-12. The structured diagnostic field `upstream_status` remains exposed downstream unchanged, as required by RTA-8. It is an HTTP status code: a bounded integer that cannot carry infrastructure text.

SAN-12a. The fields `upstream_code`, `upstream_type`, and `upstream_param` are exposed downstream only when they satisfy `ENUMSHAPE`. A field that fails `ENUMSHAPE` MUST be omitted from the downstream body. The persisted request-log value is unaffected; section 8 governs its disclosure.

`ENUMSHAPE(s)` holds when `s` is at most 64 bytes, is non-empty, and every character is an ASCII alphanumeric, `_`, `-`, or `.`.

Rationale: these fields are documented as enumerated metadata, but the value is chosen by the upstream, not by Monoize. An upstream that places a sentence, a URL, or a model identifier in its `code` field would otherwise publish it verbatim. `ENUMSHAPE` admits every conforming enumerated value (`insufficient_quota`, `model_not_found`, `rate_limit_exceeded`) and rejects free-form text, because a space, `/`, `:`, or `@` cannot appear in a conforming value.

## 8. Read-time disclosure of persisted error detail

The request-log read surfaces are `GET /api/dashboard/request-logs` (REST list) and `GET /api/dashboard/request-logs/stream` (SSE). Both surfaces serialize `error_message` (as `error.message`) and `tried_providers[].error`.

SAN-13. When the authenticated dashboard caller's role satisfies the RL-API1 admin predicate (`admin` or `super_admin`), both read surfaces MUST return `error.message` and every `tried_providers[].error` exactly as stored: the full raw detail bounded only by `TRUNC`, with no `MASK` applied.

SAN-14. For every other authenticated dashboard caller, both read surfaces MUST replace, before serialization, `error.message` with `MASK(stored error_message)` and each `tried_providers[].error` with `MASK(stored error)`. The replacement applies to REST list rows, to the initial SSE pending batch, and to every live SSE `log_batch` row. The stored row MUST NOT be modified.

SAN-15. SAN-14 operates on the stored text, which is `TRUNC`-bounded at write time. Because `MASK` is idempotent, applying SAN-14 to a historical row whose stored text was masked at write time (rows persisted before this policy) yields the stored text unchanged.

SAN-16. No non-dashboard API may return persisted request-log error text. The forwarding endpoints return only the client-tier messages defined in sections 2, 4, and 6.

## 9. System setting: mask sensitive info

SAN-CFG1. System settings MUST include `monoize_mask_sensitive_info: boolean`.

SAN-CFG2. The default value of `monoize_mask_sensitive_info` MUST be `true`.

SAN-CFG3. `monoize_runtime` MUST publish `mask_sensitive_info` equal to the committed `monoize_mask_sensitive_info`. Forwarding and dashboard request-log read paths that apply `MASK` MUST read this runtime value and MUST NOT query `system_settings` per request.

SAN-CFG4. When `mask_sensitive_info` is `true`, sections 2, 3 (`client_error`), 4, 6, and 8 apply unchanged.

SAN-CFG5. When `mask_sensitive_info` is `false`:

1. Every call site that would apply `MASK` (SAN-1 structured/internal client text, SAN-4, SAN-5 `client_error`, SAN-11, SAN-14) MUST leave the text unchanged (identity).
2. SAN-1 transport client message MUST be `upstream status {STATUS}: ` followed by `TRUNC(raw message)` instead of the fixed string `failed to request upstream`.
3. SAN-1 `unparsed_body` client message MUST be `upstream status {STATUS}: ` followed by `TRUNC(raw message)` instead of status-only text.
4. SAN-1 `empty_body` client message remains `upstream status {STATUS}`.
5. SAN-14 MUST NOT run: non-admin dashboard callers receive the stored admin-tier text verbatim.
6. SAN-2, SAN-5 `error`, SAN-7, SAN-9, and SAN-10 remain unchanged (persisted detail stays unmasked and `TRUNC`-bounded).
7. SAN-3 server tracing remains unchanged.

SAN-CFG6. Changing `monoize_mask_sensitive_info` via `PUT /api/dashboard/settings` MUST take effect for subsequent forwarding and dashboard reads after the settings transaction publishes `monoize_runtime` (DB23b). Already-persisted request-log rows are not rewritten.

## 10. Deployment-identity redaction (client tier)

The rules in sections 2, 4, and 6 bound *how* upstream error text is rewritten. This section
bounds *what a client may learn* regardless of that rewriting, and takes precedence over
them where they disagree.

SAN-D3. The **deployment identity set** `IDENTITY` of a request attempt is the set of
non-empty strings:

1. the upstream base URL, and its host component taken alone;
2. the upstream API key;
3. the upstream model identifier (`attempt.upstream_model`), including any Monoize-specific
   suffix such as `[1m]`;
4. the Group name and Group identifier;
5. the Provider name and Provider identifier;
6. the Channel name and Channel identifier;
7. the pricing profile name.

`IDENTITY` is known to Monoize at error-assembly time; it is not inferred from the error
text.

SAN-15. No byte of any `IDENTITY` member MAY appear in any client-tier surface. The
client-tier surfaces are exactly: a non-stream JSON error body, a mid-stream error frame, a
mid-stream terminal frame carrying an error object, and a WebSocket error event. This holds
for every downstream protocol, every endpoint in scope, and every value of
`monoize_mask_sensitive_info`.

SAN-15a. SAN-15 is not subject to `monoize_mask_sensitive_info`. That setting governs
`MASK` only. Disabling it MUST NOT expose an `IDENTITY` member to a client.

Rationale: `MASK` is a blacklist of four syntactic patterns. A blacklist cannot bound what a
client learns, because an identifier need not match any pattern: a model name, a Group name,
and a bare hostname without a dot each pass `MASK` unchanged. SAN-16 and SAN-17 replace the
blacklist with a whitelist at the client boundary and keep exact erasure as a second layer.

### 10.1 Whitelist: client-visible message text

SAN-16. The client-visible `message` of an upstream-derived error MUST be a string authored
by Monoize. Upstream free-form text MUST NOT be forwarded into it. The message is selected
from the fixed catalogue below by the error's classification, and the selected string is
constant: it interpolates no upstream value.

| classification | client-visible `message` |
| --- | --- |
| `QUOTA` holds (SAN-D2a) | `GENERIC_QUOTA_TEXT` |
| upstream status `400` | `the upstream provider rejected the request as invalid` |
| upstream status `401` or `403` | `the upstream provider refused the request` |
| upstream status `404` | `the upstream provider does not serve this model` |
| upstream status `408`, or a transport timeout | `the upstream provider did not respond in time` |
| upstream status `413` | `the request is too large for the upstream provider` |
| upstream status `422` | `the upstream provider rejected a request parameter` |
| upstream status `429` without `QUOTA` | `the upstream provider is rate limiting this request` |
| upstream status `5xx` | `the upstream provider reported an internal error` |
| a transport failure with no status | `the upstream provider could not be reached` |
| a content-firewall rejection | the firewall's own rule text (SAN-18) |
| none of the above | `the upstream provider returned an error` |

SAN-16a. `internal_message` MUST retain the `TRUNC`-bounded raw upstream detail exactly as
SAN-2 specifies. SAN-16 changes only the client-visible string. Every persisted field,
every admin-tier read, and the server tracing log are unchanged, so no diagnostic capability
is lost to an operator.

SAN-16b. The exhausted-routing message (SAN-6) MUST NOT name the requested model. Its
client-visible text MUST be exactly `no upstream provider could serve this request`. The
attempt count, provider identifiers, channel identifiers, and upstream URLs were already
excluded by SAN-6; the model name is excluded because a client that submitted a Monoize
model alias would otherwise learn the alias it resolves to when that alias appears in the
routing text.

### 10.2 Second layer: exact erasure

SAN-17. Before serialization, every client-tier string MUST have each `IDENTITY` member
replaced by `[redacted]`, matched as an exact case-insensitive substring, longest member
first. This runs after SAN-16 and after `MASK`.

SAN-17a. SAN-17 is a defence-in-depth layer, not the primary guarantee. Under SAN-16 a
correct implementation produces no `IDENTITY` occurrence for SAN-17 to remove. SAN-17 exists
so that a future code path that forwards upstream text without applying SAN-16 cannot
publish an `IDENTITY` member.

SAN-17b. SAN-17 MUST NOT apply to persisted request-log fields, to admin-tier reads, or to
the server tracing log.

### 10.3 Permitted rejection reasons

SAN-18. A rejection reason authored by Monoize itself MAY be exposed to a client in full,
including a rule identifier, a rule name, and a matched-category label. The content firewall
is such a source: its rejection text is Monoize-authored and names no upstream.

SAN-16c. SAN-16 supersedes the SAN-8 text carve-out. Under the RTA-8a exception
(`upstream_code = "thinking_signature_invalid"`), the client-visible `message` is the SAN-16
catalogue entry for the attempt's status, not the attempt's own text. `internal_message`
still equals the last attempt's `error`, so SAN-8's operator-facing half is unchanged. A
client keys its retry on the `thinking_signature_invalid` code, which SAN-12a admits.

SAN-18a. SAN-18 does not weaken SAN-15. A Monoize-authored rejection reason is still subject
to SAN-17, so a rule name that happens to contain a Group name is redacted.

### 10.4 Verification obligation

SAN-19. The test suite MUST contain, for each downstream protocol, a case in which the
upstream error text embeds every `IDENTITY` member and the assertion is that no member
appears in the client-visible bytes. The case MUST run with `monoize_mask_sensitive_info`
both enabled and disabled.
