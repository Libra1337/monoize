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

UA-6. The page MUST render four exact summary values: input Tokens, cache-read Tokens,
output Tokens, and total Tokens.

UA-7. The metric control MUST contain exactly `total`, `input`, `cache_read`, and `output`.
The initial metric is `total`.

UA-8. The trend chart MUST plot the selected metric from every selected-range bucket. A
visible adjacent text summary MUST expose the exact selected metric total.

UA-9. Exact totals, percentages, comparisons, and ranking order MUST use `BigInt` values.
A chart-only display value MAY use a bounded `number` derived from the exact value.

UA-10. Cache hit rate equals `cache_read / (input + cache_read)` when the denominator is
positive. A zero denominator MUST render an em dash. Percentage rounding occurs only in
the final display formatter.

## 3. Model Analysis

UA-11. The model distribution MUST aggregate the selected metric by logical model across
all selected-range buckets.

UA-12. Ranked model rows MUST sort by exact selected metric descending. Equal values MUST
sort by model name in ascending byte order.

UA-13. Each ranked row MUST show the model name, exact Token value, and percentage of the
selected metric total. It MUST remain readable without the chart.

UA-14. The chart and ranked rows MUST use only logical model names. They MUST NOT expose an
internal Provider ID, Channel ID, database ID, Base URL, credential, or header.

## 4. Fetching And Interaction

UA-15. The page MUST use SWR. Initial loading MUST render shape-matched Skeletons for the
summary, trend, distribution, and ranking regions.

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

UA-22. Page entry, count-up, chart drawing, and segmented-control movement MAY run for 180
through 260 milliseconds. Reduced-motion mode MUST render final values immediately and
remove nonessential movement.

UA-23. Every visible string MUST use an i18n key present in `en`, `zh`, `zh-TW`, and `ja`.
