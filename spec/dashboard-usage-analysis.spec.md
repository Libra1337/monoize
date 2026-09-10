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

## 6. Cache Hit Rate By Model

UA-24. The page MUST render one full-width panel that reports cache hit rate per logical
model. The panel MUST derive its rows from the same `GET /api/dashboard/analytics`
response as the trend and distribution panels. It MUST NOT issue an additional request.

UA-25. For every logical model `m` in the selected range, let `input(m)` be the sum of
`input_tokens_by_model[m]` and `cacheRead(m)` be the sum of `cache_read_tokens_by_model[m]`
across all selected-range buckets. The panel MUST contain exactly one row for every `m`
with `input(m) > 0` and no row for any other `m`. Model-name normalization follows UA-14a.

UA-26. `hitBasisPoints(m)` equals `(cacheRead(m) * 10000 + input(m) / 2) / input(m)` under
`BigInt` integer division. All three quantities MUST be computed as `BigInt`. Rounding to a
displayed percentage occurs only in the final display formatter.

UA-27. Rows MUST sort by `input(m)` descending, because input volume determines the cost
impact of a low hit rate. Equal `input(m)` values MUST sort by model name in ascending byte
order.

UA-28. Each row MUST show the model name, the exact `cacheRead(m)` value, the exact
`input(m)` value, and `hitBasisPoints(m)` rendered as a percentage with at most one decimal
digit.

UA-29. Each row MUST carry exactly one grade, assigned by the following total function of
`(input(m), hitBasisPoints(m))`:

| Precondition | Grade |
| --- | --- |
| `input(m) < 50000` | `insufficient` |
| `input(m) >= 50000` and `hitBasisPoints(m) < 3000` | `low` |
| `input(m) >= 50000` and `3000 <= hitBasisPoints(m) < 6000` | `partial` |
| `input(m) >= 50000` and `hitBasisPoints(m) >= 6000` | `high` |

The 50,000-Token floor exists because a hit rate measured over a smaller input total is
dominated by the unavoidable cache-miss cost of the first request in a conversation.

UA-30. The `low` grade MUST render with the destructive color token, `partial` with the
warning token, `high` with the success token, and `insufficient` with the muted-foreground
token. A grade MUST also be conveyed by a localized text label, so color is not the only
carrier of the distinction.

UA-31. A row grade MUST NOT change when the selected metric changes. Cache hit rate is a
ratio of input Tokens and is independent of the metric control.

UA-32. Initial loading MUST render shape-matched Skeletons for the panel. A resolved
response with no row satisfying UA-25 MUST render the localized empty state of UA-18.

UA-33. When the selected range changes, each row's ratio bar MUST transition to the
selected value over 1,000 milliseconds. Model names, exact Token values, and percentages
MUST remain visible during the transition. Reduced-motion mode MUST render the selected
values without interpolation.
