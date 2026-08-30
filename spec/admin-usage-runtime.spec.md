# Admin Usage and Runtime Pages Specification

## 1. Scope

This specification defines usage ranking and runtime status pages under the
authenticated dashboard. Usage data is privacy-filtered for ordinary users.

## 2. Access

UR-1. `GET /api/dashboard/admin/usage-ranking` MUST require an authenticated
session. Administrator sessions receive full user identifiers, usernames, charges,
and per-user model rows. A non-administrator response MUST omit every user charge.
It MUST include non-charge model rows so each ranked row remains selectable.

UR-2. The frontend routes `/dashboard/admin/usage` and `/dashboard/admin/runtime`
MUST render an unauthorized state for non-admin sessions and MUST NOT request an
administrator endpoint for those sessions.

UR-2a. The frontend route `/dashboard/admin` MUST satisfy UR-2. The dashboard
navigation MUST omit all three administrator routes for a non-admin session.

UR-2b. The frontend route `/dashboard/usage-ranking` MUST render the usage ranking
for every enabled authenticated role. The ordinary-user navigation MUST contain a
link to this route. `/dashboard/admin/usage` MAY remain as an administrator-only
compatibility route, but it MUST NOT be the only usage-ranking entry point.

## 3. Usage ranking endpoint

UR-3. The endpoint MUST aggregate request logs from the preceding 24 hours ending
at the request time. It MUST aggregate in SQL and MUST NOT load raw request-log
rows into application memory.

UR-4. The response MUST contain `time_from`, `time_to`, `total_tokens`,
`total_input_tokens`, `total_cache_read_tokens`, `total_output_tokens`,
`total_calls`, `total_cost_nano_usd`, and `users`. These totals MUST cover every
request log with a non-null user id in the window, including users outside the
top-20 ranking.

UR-5. `users` MUST contain at most 20 users ordered by total Tokens descending,
call count descending, then user id ascending. `input_tokens` is an inclusive input
total and `cache_read_tokens` is a detail of that total. Total Tokens MUST equal input
Tokens plus output Tokens; cache-read Tokens MUST NOT be added again. Each user object MUST contain
`user_id`, `username`, `call_count`, `cost_nano_usd`, `input_tokens`,
`cache_read_tokens`, `output_tokens`, and `models`.

UR-6. Each `models` item MUST contain `model`, `call_count`, `cost_nano_usd`,
`input_tokens`, `cache_read_tokens`, and `output_tokens`. Model rows MUST be
ordered by input Tokens plus output Tokens descending, call count descending, then
model name using UTF-8 byte order. Cache-read Tokens MUST NOT be added again.

UR-7. Token and charge totals MUST be serialized as canonical decimal strings.
Negative or out-of-range stored aggregates MUST return an internal error.

UR-7a. The usage page MUST refresh the usage-ranking snapshot every 2 seconds.
It MUST display one full-width token bar divided into input, cache-read, and
output segments. The bar label MUST display the aggregate token count. Each
segment row MUST render its signed animation delta immediately after its main
Token value on the same line. A segment delta MUST NOT reserve a second line or
render below the segment row.

UR-7b. The response MUST contain `models`, with at most 20 global model rows
ordered by total tokens descending, call count descending, then model name in
UTF-8 byte order. Each row MUST contain `model`, `call_count`, `input_tokens`,
`cache_read_tokens`, `output_tokens`, and `cost_nano_usd`.

UR-7c. The usage page MUST display user and model rankings in equal-width columns
on desktop and stacked sections on narrow screens. The total, user-ranking, and
model-ranking token counters MUST animate from the currently displayed value to the
latest target over 7.2 seconds. Changing the selected time range MUST preserve the
currently displayed value as the animation start value.

UR-7d. The usage page MUST display each ranked user's call count, token count,
and charge. Selecting a user MUST open a modal that displays that user's model
rows. The modal MUST keep its dimensions stable while it is open.

UR-7d-1. Each user-ranking and model-ranking row on authenticated and public
usage-ranking pages MUST display only its total Token value in the Token column.
It MUST NOT render input, cache-read, or output breakdown values below that total.

UR-7e. When two user or model rows exchange rank, both rows MUST animate to their
new positions instead of replacing content at fixed positions. A non-administrator
user row MAY contain `rank_key`. `rank_key` MUST be generated from the internal
user ID and a process-local random salt. The salt MUST NOT leave process memory.
The key MUST change after process restart and MUST NOT be rendered as text.

UR-7f. Each user MUST store `usage_ranking_anonymous` as a non-null Boolean.
The default and migration value MUST be `true`. `GET /api/dashboard/auth/me`
MUST return the value. `PUT /api/dashboard/auth/me` MUST update the value only
when the request contains it.

UR-7g. For a non-administrator viewer and one ranked target, the response MUST
include the target username only when the target is the viewer or both users have
`usage_ranking_anonymous = false`. Otherwise, the response MUST omit both the
target username and target user ID. The viewer's preference MUST NOT modify any
other user's preference. The frontend heading above user rows MUST use the locale
equivalent of `Current ranking`.

UR-7g-1. Every authenticated usage-ranking response MUST contain
`current_user_rank`. If the viewer occurs in the returned top-20 user list, this
value MUST equal the viewer's one-based position in that list. Otherwise, this
value MUST be null. Every authenticated summary MUST label this value as the
locale equivalent of `Current rank` and render a null value as the locale
equivalent of `Unranked`. Administrator-only cost metrics MAY remain visible in
the administrator summary, but the summary MUST NOT replace the current rank
with a ranked-user count.

UR-7h. The public site MUST provide a navigation link and route for a public usage
ranking. The public endpoint MUST accept exactly `24h`, `7d`, or `30d`, default to
`24h`, and return total, anonymous user, and global model Token aggregates for the
selected window. It MUST NOT return a user ID, username, user charge, or model charge.
Public user rows MUST be ordered by total Tokens descending, call count descending,
then the internal user ID using UTF-8 byte order. The internal user ID MUST NOT be returned.

## 4. Runtime status page

UR-8. The runtime page MUST use the existing admin overview snapshot and display
node status, process counters, Provider/Channel health, and Replica status.

UR-9. Runtime data MUST refresh every 2 seconds, show a loading skeleton, and
show a retry state after a failed request. The page MUST animate section entry and
active navigation transitions without changing layout dimensions.

UR-9a. Each administrator page MUST reserve a 36 by 36 CSS-pixel refresh-status
control in its page header. The control MUST contain a 24 by 24 CSS-pixel
Monoize brand mark. While a refresh request is active, the mark MUST rotate
clockwise. When no refresh request is active, the mark MUST remain stationary.
The animation MUST stop when the user requests reduced motion.

UR-9b. The authenticated dashboard route `/dashboard/status` MUST be accessible
to every enabled authenticated role. It MUST render the public Group, Provider,
model, success-rate, and 24-hour timeline status view without exposing internal
IDs, credentials, or administrator-only runtime counters. It MUST refresh every
2 seconds and show a loading skeleton and retry state.

## 5. Privacy

UR-10. Neither endpoint or page may expose API keys, session tokens, database
passwords, raw request prompts, or request bodies.
UR-11. Non-administrator responses MUST NOT expose user IDs or charges. They MAY
expose a username only under UR-7g. Model rows MUST omit charges.

UR-12. One counter animation cycle starts when an idle counter receives a changed
target. At each rendered animation frame, its signed delta MUST equal the currently
displayed value minus the cycle start value. If another target arrives before
completion, the counter MUST retarget from its currently displayed value and the
signed delta MUST continue from that displayed net cycle change.
The delta MUST NOT show each intermediate update as a separate positive or negative
change. The delta MUST fade in, remain visible while the counter is moving, and
start fading out when the displayed value reaches the latest target. If the new
value equals the latest target, no new animation or delta MUST start. Reduced-motion
mode MUST update the value without number interpolation. Every interpolation MUST
last 7.2 seconds and MUST use an ease-in-out timing function. The delta MUST share
the main value's inline formatting context. After its fade-out completes, the
delta MUST be removed from layout.
