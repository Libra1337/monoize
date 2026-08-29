# Dashboard Home Overview Specification

## 0. Scope

DH-0.1. This specification defines the authenticated browser page at `/dashboard`.

DH-0.2. `dashboard-usage-analysis.spec.md` defines the detailed Usage Analysis page.
`dashboard-ui-layout.spec.md` defines the shared Console shell and navigation.

## 1. Page Structure

DH-1. The page MUST render these sections in order:

1. one greeting header with no action control;
2. four overview cards;
3. one Token Usage section.

DH-2. The page MUST NOT render the former Model Data tab panel or the former API
Information panel.

DH-3. The four overview cards MUST use these responsive columns:

- below `md`: one column;
- from `md` through below `xl`: two columns;
- at or above `xl`: four columns.

DH-4. Each overview card MUST contain exactly two metric rows. A metric row contains one
label and one value. It MUST NOT contain a chart or decorative metric icon.

DH-5. The account overview card MUST read the authenticated session user. It MUST show the
current balance and subscription. It MUST NOT call an Admin-only billing-plan endpoint.

DH-6. An unlimited balance MUST render the localized unlimited label. Another balance MUST
render `balance_usd` with two USD fractional digits.

DH-7. A missing `billing_plan` MUST render the localized no-plan label. A present plan MUST
render its name, `grant_amount_usd`, and schedule.

## 2. Token Usage

DH-8. The Token Usage range control MUST contain exactly `24h`, `week`, and `month`.
The initial range is `24h`.

DH-9. The range values map to analytics queries as follows:

| Range | `range_hours` | `buckets` |
| --- | ---: | ---: |
| `24h` | 24 | 24 |
| `week` | 168 | 28 |
| `month` | 720 | 30 |

DH-10. Every Dashboard Token Usage query MUST request `scope=self`. This rule applies to
`user`, `admin`, and `super_admin` sessions. The page MUST NOT display another user's data.

DH-11. The Token Usage summary MUST render exact input, cache-read, output, and total Token
values returned by `GET /api/dashboard/analytics`. It MUST parse and add decimal integer
strings with `BigInt`. It MUST NOT use JavaScript `Number` for exact totals.

DH-12. The Token Usage trend MUST use the selected range buckets. The chart MAY receive
bounded display numbers. Adjacent visible text MUST retain the exact total values.

DH-13. A range change MUST keep the last resolved analytics response visible until the next
response resolves. An unresolved first request MUST render a shape-matched Skeleton.

DH-14. A failed analytics request MUST render an inline localized failure state and a retry
action. Retrying MUST revalidate the SWR key without a page reload.

DH-15. A resolved response with zero total Tokens MUST render an explicit empty state.

## 3. Analytics API

DH-16. `GET /api/dashboard/analytics` accepts optional `scope`. The only valid explicit
value is `self`. Another explicit value MUST return HTTP `400` with `invalid_request`.

DH-17. `scope=self` MUST aggregate only rows whose user ID equals the authenticated user ID,
including when the authenticated role is `admin` or `super_admin`.

DH-18. An omitted `scope` keeps the existing role behavior: Admin roles aggregate all users,
and role `user` aggregates only the authenticated user.

DH-19. Each analytics bucket MUST include these maps in addition to existing cost and call
maps:

```text
input_tokens_by_model: Record<string, string>
cache_read_tokens_by_model: Record<string, string>
output_tokens_by_model: Record<string, string>
```

DH-20. The analytics response MUST include these decimal integer strings:

```text
total_input_tokens
total_cache_read_tokens
total_output_tokens
total_tokens
```

DH-21. For each bucket and model, `total_tokens` equals input Tokens plus cache-read Tokens
plus output Tokens. Response-wide totals equal the checked sum of all returned buckets.

DH-22. Token aggregation MUST execute in the database in the same set-based model-bucket
query as cost and call aggregation. PostgreSQL MUST use exact integer or numeric aggregates.
SQLite MUST decode persisted integer values with checked `i128` arithmetic.

DH-23. A negative persisted Token value or an aggregate outside signed `i128` MUST return an
internal storage error. The server MUST NOT serialize a partial analytics response.

DH-24. Token totals MUST serialize as base-10 strings. They MUST NOT pass through a JSON
number, SQL floating-point value, Rust floating-point value, or Rust `i64` narrowing step.

## 4. Motion, Layout, And Localization

DH-25. The header, overview cards, Token summary, and Token trend MAY animate with opacity,
vertical translation, count-up, or progressive chart drawing for 180 through 260
milliseconds.

DH-26. Reduced-motion mode MUST render final values immediately. It MUST remove nonessential
translation and progressive chart drawing.

DH-27. The Token Usage section MUST stack into one column below 768 pixels. It MUST NOT set a
fixed viewport height or cause horizontal page overflow.

DH-28. Every user-visible string MUST use an i18n key present in `en`, `zh`, `zh-TW`, and
`ja`. English fallback text MUST be used in source code.
