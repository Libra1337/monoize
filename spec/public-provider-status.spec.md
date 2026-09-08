# Public Provider Status Specification

## 0. Scope

PST-0.1. This specification defines the public runtime-status aggregate and
`GET /api/public/status`.

PST-0.2. The aggregate uses terminal rows from the existing `request_logs` table. It MUST
NOT create a second status-event table, status spool, or status shipment protocol.
`request-logs.spec.md` defines request-log durability and Replica shipment.

PST-0.3. `provider-pricing.spec.md` defines the singular embedded Channel on each Provider.
`public-site.spec.md` defines shared public API limits, cache validators, and security
headers.

## 1. Source rows

PST-S1. A source row is eligible only when all of these conditions are true:

- `created_at_unix_ms` is not null and is in the inclusive 24-hour snapshot window;
- `provider_id` identifies a current enabled Provider;
- `channel_id` equals that Provider's current embedded Channel ID;
- that embedded Channel is enabled.

PST-S2. One eligible request-log row contributes exactly one outcome. `status = "success"`
or `status = "client_gone"` contributes success. Every other terminal status contributes
failure. A row with no current matching Provider and Channel contributes no outcome.

PST-S3. Status aggregation MUST NOT read or return `user_id`, `api_key_id`, request or
response bodies, prompts, URLs, error text, IP addresses, or another customer value.

PST-S4. A request that failed over between Providers contributes only its terminal
request-log outcome. `tried_providers_json` does not contribute additional outcomes.

PST-S5. The model key is the first non-empty trimmed value among `model`,
`upstream_model`, and `unknown`.

## 2. Public response

PST-P1. `GET /api/public/status` MUST require no dashboard session and return exactly:

```text
generated_at: RFC3339 UTC
data_through: RFC3339 UTC
data_complete: boolean
groups: Array<{
  public_name: string,
  state: operational | minor_degradation | major_degradation | unavailable | insufficient_data,
  insufficient_provider_count: nonnegative integer,
  success_rate_24h_basis_points: integer 0..10000 | null,
  last_observed_at: RFC3339 UTC | null,
  timeline: Array<{
    started_at: RFC3339 UTC,
    state: operational | minor_degradation | major_degradation | unavailable | insufficient_data
  }>,
  models: Array<{
    name: string,
    state: operational | minor_degradation | major_degradation | unavailable | insufficient_data,
    success_rate_24h_basis_points: integer 0..10000 | null
  }>,
  providers: Array<{
    public_name: string,
    state: operational | minor_degradation | major_degradation | unavailable | insufficient_data,
    success_rate_24h_basis_points: integer 0..10000 | null
  }>
}>
```

PST-P2. The response MUST NOT contain attempt counts, internal names, internal IDs, Channel
fields, failure classes, upstream statuses, source-node fields, or secrets.

PST-P3. Include only enabled Providers whose embedded Channel is enabled. Include a Group
only when it contains at least one included Provider. Order Groups by current Group order.
Order Providers by Group-local priority, creation time, and ID.

PST-P4. `data_through` equals the snapshot creation time. The 24-hour window starts at
`data_through - 24 hours`. A successful response MUST set `data_complete = true`. A source
read or serialization failure MUST return PST-P12 instead of a partial response.

PST-P5. Current state uses the inclusive 30-minute window ending at `data_through`. Fewer
than 10 outcomes is `insufficient_data`. Otherwise classify with integer cross-products:

- at least 98 percent: `operational`;
- at least 90 and below 98 percent: `minor_degradation`;
- at least 80 and below 90 percent: `major_degradation`;
- below 80 percent: `unavailable`.

PST-P6. The 24-hour success rate uses the inclusive window ending at `data_through`. Zero
outcomes returns null. Otherwise return `floor(successes * 10000 / outcomes)`.

PST-P7. Each Group timeline MUST contain exactly 48 ordered 30-minute UTC buckets. The
last bucket starts at `floor(data_through_unix_ms / 1800000) * 1800000`. Each bucket
aggregates eligible rows in the Group. Fewer than 10 outcomes is `insufficient_data`;
otherwise classification uses PST-P5. `last_observed_at` is the latest eligible request
time or null.

PST-P8. Each Group model row MUST aggregate eligible rows by PST-S5 model key over the same
24-hour window. Models MUST sort by UTF-8 byte order. Model state uses the 30-minute window
from PST-P5. Model success rate uses PST-P6. A model row MUST NOT expose Provider or Channel
identity.

PST-P9. Group state is the worst known Provider state in this order: unavailable, major
degradation, minor degradation, operational. Ignore insufficient Providers when one known
state exists. `insufficient_provider_count` equals the ignored count. A Group is
`insufficient_data` only when every Provider is insufficient.

PST-P10. One immutable serialized snapshot MUST be reused for 15 seconds. The snapshot key
MUST include the application instance and routing-configuration revision. `generated_at`,
`data_through`, response bytes, and ETag MUST remain identical during that interval.
Browser SWR refresh is 30 seconds.

PST-P11. The endpoint MUST apply the public token-bucket limit and ETag behavior from
`public-site.spec.md`. A matching `If-None-Match` MUST return HTTP `304` with an empty body.

PST-P12. A source read or response serialization failure MUST return HTTP `503` with code
`status_source_invalid` and a fixed public message. The server MUST log the underlying
error. The response MUST NOT contain SQL, a table or constraint name, an internal ID, or
the underlying error text.

## 3. Public UI

PST-U1. `/status` MUST render an overall summary and the snapshot update time. The summary
MUST report insufficient data when every Group is `insufficient_data`. It MUST report all
observed Groups operational only when at least one Group is operational and none has a
degradation or outage.

PST-U2. The page MUST render the five state thresholds as a legend.

PST-U3. The page MUST render one section per Group. Each section MUST show Group state,
24-hour success rate, 48 timeline buckets, latest observation time, and every Provider
state in that Group.

PST-U4. Activating the Group model action MUST open a modal that lists the Group model
rows. It MUST NOT expand the Group section in place.

PST-U5. The page MUST provide a Skeleton before initial data is available. It MUST retain
the previous response during SWR revalidation. It MUST show a fixed public error state when
the endpoint fails.

## 4. Verification

PST-T1. SQLite tests MUST prove the five PST-P5 boundary states, the PST-S2
`client_gone` rule, the 48-bucket order, public-name allow-listing, and UTF-8 model order.

PST-T2. An endpoint test MUST prove that a request-log insert during the 15-second cache
interval does not change response bytes or ETag.

PST-T3. Frontend tests MUST prove that the page uses a modal for model details and does not
use an inline disclosure element.
