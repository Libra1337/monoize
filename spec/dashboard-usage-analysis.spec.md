# Dashboard Usage Analysis Specification

## 0. Scope

UA-0.1. This specification defines the authenticated page at `/dashboard/usage`.

UA-0.2. Usage Analysis means Token consumption analysis. It does not mean request-log
inspection. `request-logs.spec.md` remains the source of truth for `/dashboard/logs`.

## 1. Route And Data Scope

UA-1. `/dashboard/usage` MUST render inside `DashboardLayout` and require an authenticated
session.

UA-2. The page MUST use `GET /api/dashboard/analytics` with `scope=self`. It MUST NOT import,
mount, or call the request-log page, request-log hook, or request-log endpoint.

UA-3. `/dashboard/logs` MUST keep its existing route, components, filters, and behavior.

UA-4. The range control MUST contain exactly `24h`, `7d`, and `30d`. The initial range is
`7d`.

UA-5. The range values map to analytics queries as follows:

| Range | `range_hours` | `buckets` |
| --- | ---: | ---: |
| `24h` | 24 | 24 |
| `7d` | 168 | 28 |
| `30d` | 720 | 30 |

## 2. Summary And Trend

UA-6. The page MUST render one full-width summary panel containing four exact values in
this order: input Tokens, cache-read Tokens, output Tokens, and total Tokens. Adjacent
values MUST be separated by a visible vertical divider at widths of 640 pixels or more.
Below 640 pixels, the values MAY wrap and MUST use horizontal dividers.

UA-7. The metric control MUST contain exactly `total`, `input`, `cache_read`, and `output`.
The initial metric is `total`.

UA-8. The trend chart MUST plot the selected metric from every selected-range bucket. A
visible adjacent text summary MUST expose the exact selected metric total.

UA-9. Exact totals, percentages, comparisons, and ranking order MUST use `BigInt` values.
A chart-only display value MAY use a bounded `number` derived from the exact value.

UA-10. `input_tokens` is the inclusive input total and `cache_read_tokens` is a detail of
that total. Cache hit rate equals `cache_read / input` when input is positive. A zero
input total MUST render an em dash. Percentage rounding occurs only in the final display
formatter.

## 3. Model Analysis

UA-11. The model distribution MUST aggregate the selected metric by logical model across
all selected-range buckets.

UA-12. Ranked model rows MUST sort by exact selected metric descending. Equal values MUST
sort by model name in ascending byte order.

UA-13. Each ranked row MUST show the model name, exact Token value, and percentage of the
selected metric total. It MUST remain readable without the chart.

UA-13a. Each ranked row MUST reserve one wrapping line for the complete model name. The
Token value and percentage MUST render on a separate wrapping line. A numeric value MUST
NOT reduce the model-name line to zero width. Neither line may overflow the Card.

UA-13b. The distribution chart and ranked rows MUST stack vertically inside the Card. The
ranked rows MUST use the full Card content width. A viewport breakpoint MUST NOT place the
ranked rows in a narrow column beside the chart.

UA-14. The chart and ranked rows MUST use only logical model names. They MUST NOT expose an
internal Provider ID, Channel ID, database ID, Base URL, credential, or header.

UA-14a. Analytics aggregation MUST use the trimmed logical-model request-log value. If a
historical row has an empty logical-model value, it MUST use the trimmed upstream-model
value. If both values are empty, it MUST use the literal label `unknown`. The model chart
and every ranked row MUST render the resulting non-empty model label.

## 4. Fetching And Interaction

UA-15. The page MUST use SWR with `refreshInterval = 2000` milliseconds. Initial loading
MUST render shape-matched Skeletons for the summary, trend, distribution, and ranking
regions. A refresh MUST keep the last resolved response visible.

UA-16. A range change MUST keep the last resolved response visible until the next response
resolves. It MUST NOT require a close, reopen, or page refresh.

UA-17. A failed request MUST render an inline localized error and retry action. A retry MUST
revalidate the active SWR key.

UA-18. A resolved response with zero total Tokens MUST render a localized empty state.

## 5. Layout, Motion, And Accessibility

UA-19. The page MUST use full-width Console sections. It MUST NOT nest a decorative Card
inside another Card.

UA-20. Below 768 pixels, summary, chart, distribution, and ranking regions MUST stack in one
column. The page MUST NOT use a fixed content height or create horizontal overflow.

UA-21. Range and metric controls MUST expose visible focus. Charts MUST expose the same
values in adjacent text or ranked rows.

UA-22. Page entry and segmented-control movement MAY run for 180 through 260 milliseconds.
Token count-up and chart drawing MUST run for 800 through 1200 milliseconds so the change
remains readable. A refresh MUST animate from the last displayed value rather than zero.
Reduced-motion mode MUST render final values immediately and remove nonessential movement.

UA-22a. When the selected range or metric changes, the trend chart MUST keep its
Card, axes, grid, tooltip, and dimensions mounted. Only the Line path MUST
interpolate from the last resolved series to the selected series. The Line
animation MUST last 1,200 milliseconds and use ease-in-out timing. The trend
chart MUST NOT fade or translate as one block.

UA-22b. When the selected range or metric changes, the model-distribution donut
sectors and ranked-row progress bars MUST transition to the selected values over
1,000 milliseconds. Model labels and exact Token values MUST remain visible
during the transition.

UA-22c. A selection animation MUST start after data for the selected query
resolves. While that query is pending, the last resolved chart data MUST remain
visible. A polling response for an unchanged selection MUST update the chart
without starting a selection animation. Reduced-motion mode MUST render the
selected chart data without Line, sector, or progress-bar interpolation.

UA-23. Every visible string MUST use an i18n key present in `en`, `zh`, `zh-TW`, and `ja`.

## 6. Cache Hit Rate Sub-Page

UA-24. Cache hit rate MUST be reported on its own authenticated sub-page at
`/dashboard/usage/cache`. It MUST render inside `DashboardLayout`. The sidebar MUST expose it
as an entry adjacent to the Usage Analysis entry, in both the standard and the enterprise
navigation sets. `/dashboard/usage` MUST NOT render a cache hit rate panel.

UA-25. The sub-page MUST select its analytics scope from the authenticated role. A
`super_admin` session requests `GET /api/dashboard/analytics` with no `scope` parameter, so
DH-18 aggregates every user. An `admin` session requests it with `scope=group`, so the
aggregate covers exactly the request-log rows whose `user_id` belongs to a user whose
`group_id` equals the admin's own `group_id` (DH-18c). A `user` session requests
`scope=self`, which aggregates only itself. A per-model cache hit rate is actionable only
when it covers the traffic the reader is responsible for.

UA-26. The range control MUST contain exactly `24h`, `7d`, and `30d` with the mapping of
UA-5. The initial range is `7d`. The sub-page MUST NOT expose the metric control of UA-7,
because cache hit rate is a ratio of input Tokens and does not vary by metric.

### 6.1 Row set

UA-27. For a logical model `m`, let `input(m)` be the sum of `input_tokens_by_model[m]` and
`cacheRead(m)` the sum of `cache_read_tokens_by_model[m]` across all selected-range buckets.
Model-name normalization follows UA-14a.

UA-28. A **measured row** is a row for a model with `input(m) > 0`.

UA-29. An **untracked row** is a row for a model that appears in the reader's routable model
catalog and has no measured row. For an `admin` or `super_admin` session the catalog is the
union of the model maps of every Channel returned by `GET /api/dashboard/providers`. For a
`user` session the catalog is empty, because that endpoint requires the Admin role.

UA-30. The table MUST contain every measured row and every untracked row, and no other row.
An untracked row is required because an absent row and a zero-hit row are
indistinguishable to a reader; the sub-page MUST state which models had no traffic rather
than omit them.

UA-31. Measured rows MUST precede untracked rows. Measured rows MUST sort by `input(m)`
descending, because input volume determines the cost impact of a low hit rate; equal values
sort by model name in ascending byte order. Untracked rows MUST sort by model name in
ascending byte order.

### 6.2 Values and grades

UA-32. `hitBasisPoints(m)` equals `(cacheRead(m) * 10000 + input(m) / 2) / input(m)` under
`BigInt` integer division, and is `0` when `input(m) = 0`. All quantities MUST be computed as
`BigInt`. Rounding to a displayed percentage occurs only in the final display formatter.

UA-33. Each row MUST show the model name, `input(m)`, `cacheRead(m)`, the hit rate as a
percentage with at most one decimal digit, and a localized grade label. An untracked row MUST
render an em dash for `input(m)`, `cacheRead(m)`, and the hit rate, and MUST NOT render a
ratio bar.

UA-34. Each row MUST carry exactly one grade, assigned by this total function of
`(input(m), hitBasisPoints(m))`:

| Precondition | Grade |
| --- | --- |
| `input(m) = 0` | `no_traffic` |
| `0 < input(m) < 50000` | `insufficient` |
| `input(m) >= 50000` and `hitBasisPoints(m) < 3000` | `low` |
| `input(m) >= 50000` and `3000 <= hitBasisPoints(m) < 6000` | `partial` |
| `input(m) >= 50000` and `hitBasisPoints(m) >= 6000` | `high` |

The 50,000-Token floor exists because a hit rate measured over a smaller input total is
dominated by the unavoidable cache-miss cost of the first request in a conversation.

UA-35. The `low` grade MUST render with the destructive color token, `partial` with the
warning token, `high` with the success token, and `no_traffic` and `insufficient` with the
muted-foreground token. A grade MUST also be conveyed by a localized text label, so color is
not the only carrier of the distinction.

### 6.3 Summary, filtering, and states

UA-36. The sub-page MUST render one summary strip with four values in this order: total input
Tokens, total cache-read Tokens, the overall cache hit rate computed as in UA-10, and the
count of measured rows over the count of all rows.

UA-37. The sub-page MUST provide a case-insensitive substring filter over the model name and
a boolean control that restricts the table to measured rows. The boolean control MUST default
to off, so every row of UA-30 is visible without interaction.

UA-38. Initial loading MUST render shape-matched Skeletons for the summary strip and the
table. A refresh MUST keep the last resolved response visible. The sub-page MUST use SWR with
`refreshInterval = 2000` milliseconds.

UA-39. A resolved response for which the filters of UA-37 select no row MUST render a
localized empty state. A failed analytics request MUST render a localized inline error and a
retry action that revalidates the active SWR key.

UA-40. Every visible string on the sub-page MUST use an i18n key present in `en`, `zh`,
`zh-TW`, and `ja`.

### 6.4 Per-user cache table (super_admin)

UA-41. For a `super_admin` session on `/dashboard/usage/cache`, the sub-page MUST additionally
render one per-user cache table below the per-model table, sourced from
`GET /api/dashboard/usage/cache/users?range_hours=N` where `N` equals the `range_hours` of the
range selected per UA-26. An `admin` or `user` session MUST NOT render the per-user table.

UA-42. The endpoint MUST require role `super_admin`; any other session MUST receive HTTP 403
with code `forbidden`. It MUST clamp `range_hours` to `1..=720`, MUST apply the DH-18a probe
exclusion, and MUST aggregate only request-log rows with a resolvable `users` row. The
response MUST be a JSON object with `range_hours` and a `users` array whose entries contain
exactly `user_id`, `username`, `input_tokens`, and `cache_read_tokens` as integer strings.
Entries MUST order by `input_tokens` descending with ties broken by `username` in ascending
byte order, and the array MUST be capped at 100 entries.

UA-43. Each per-user row MUST show the username, `input_tokens`, `cache_read_tokens`, and the
hit rate computed and rounded as in UA-32 and UA-33. Rows MUST carry the UA-34 grade assigned
from `(input_tokens, hitBasisPoints)` and the UA-35 color rules. The UA-37 filter controls
MUST apply to this table: the substring filter matches the username, and the boolean control
restricts the table to rows with `input_tokens > 0`.

UA-44. The per-user table MUST follow the UA-38 loading and UA-39 empty/error states with the
same SWR refresh interval.
