# Model Marketplace Specification

## 0. Scope

MM-0.1. This specification defines the public Model Marketplace at `/marketplace`, the
authenticated Model Marketplace at `/dashboard/marketplace`, the two public APIs,
persistent generation, pagination, pricing display, and modal interaction.

MM-0.2. `provider-pricing.spec.md` defines public-name, model-key, effective pricing, and
unpriced-mapping behavior. `public-site.spec.md` defines route authentication, shared
public API controls, and layout requirements.

MM-0.3. The authenticated `GET /api/dashboard/marketplace/models` endpoint remains a
separate Console contract for Playground and other authenticated consumers. A public
Marketplace response MUST NOT be used as an authorization decision.

## 1. Public list API

MM-L1. `GET /api/public/marketplace` MUST accept optional `q`, optional `group`, optional
`cursor`, and optional `limit`. Limit defaults to 24 and MUST be from 1 through 50.

MM-L2. A supplied Group MUST use the public-name normalization and binary-key lookup from
`provider-pricing.spec.md`. An invalid Group, cursor, limit, or search MUST return HTTP
`400` with code `invalid_request`.

MM-L3. Canonical `q` is the input trimmed by Unicode White_Space without Unicode
normalization. An empty canonical value is absent. A present value MUST contain at most 128
UTF-8 bytes and MUST NOT contain a C0 control, DEL, CR, LF, or tab.

MM-L4. Search bytes MUST apply ASCII `A` through `Z` to lowercase by adding 32 and leave
every other byte unchanged. SQLite MUST filter with `instr(model_search_key, ?1) > 0`.
PostgreSQL MUST use equivalent BYTEA substring containment. Filtering MUST occur in the
database before keyset limiting.

MM-L5. The response MUST contain exactly:

```text
generated_at: RFC3339 UTC with six fractional digits and Z
revision: unsigned decimal string
next_cursor: opaque string | null
items: Array<{
  public_group_name: string,
  model: string,
  capabilities: string[],
  input_rate_range: { min: decimal string, max: decimal string, unit: string } | null,
  output_rate_range: { min: decimal string, max: decimal string, unit: string } | null,
  offer_count: positive integer
}>
```

MM-L6. Rows MUST order by zero-based current Group ordinal and `model_name_key`, using exact
UTF-8 byte order on both databases and in cursor construction.

MM-L7. The page MUST render one visible Group heading before each Group's first row. A page
that begins inside a Group MUST repeat its heading. Rows from different Groups MUST NOT
share a heading.

## 2. Public offers API

MM-O1. `GET /api/public/marketplace/offers` MUST require `group` and `model`, and accept
optional `cursor` and `limit`. Limit defaults to 20 and MUST be from 1 through 50.

MM-O2. Group uses MM-L2. Model MUST equal its Unicode White_Space-trimmed value, contain 1
through 256 UTF-8 bytes, and contain no C0 control, DEL, CR, LF, or tab. An invalid value
returns HTTP `400` `invalid_request`. A valid exact model without a visible row returns HTTP
`404` `marketplace_model_not_found`.

MM-O3. The response MUST contain exactly:

```text
generated_at: RFC3339 UTC with six fractional digits and Z
revision: unsigned decimal string
public_group_name: string
model: string
next_cursor: opaque string | null
offers: Array<{
  public_provider_name: string,
  public_channel_name: string,
  api_type: responses | chat_completion | messages | gemini | openai_image | replicate,
  rates: Array<{
    usage_class: string,
    unit: string,
    display_rate_nano_usd: decimal string,
    context_tier: string | null,
    service_tier: string | null,
    modality: string | null,
    cache_ttl: string | null
  }>
}>
```

MM-O4. Offers MUST order by numeric Provider priority, Provider public-name key, and Channel
public-name key. Name comparisons use exact UTF-8 byte order.

MM-O5. The response MUST NOT contain Billing Profile names, multipliers, rate row IDs,
internal IDs or names, Base URLs, API keys, proxy URLs, custom headers, or internal errors.

MM-O6. A Marketplace source read or response serialization failure MUST return HTTP `503`
with code `marketplace_source_invalid` and a fixed public message. The server MUST log the
underlying error. The response MUST NOT contain SQL, a table or constraint name, an
internal ID, or the underlying error text.

## 3. Cursor contract

MM-C1. Migration MUST create one random 32-byte cursor HMAC key, store its unpadded
base64url encoding in a dedicated `state_records` row inside the migration transaction, and
never return or log it. Startup MUST refuse readiness with
`marketplace_cursor_key_unavailable` when absent or invalid.

MM-C2. A cursor is `<payload>.<signature>`, both base64url without padding. Signature is
HMAC-SHA-256 over exact payload bytes and MUST be compared in constant time before parsing.

MM-C3. The common payload prefix is:

```text
version: u8 = 1
endpoint_kind: u8, list = 1, offers = 2
revision: u64 big-endian
limit: u16 big-endian
filter_digest: 32 SHA-256 bytes
```

MM-C4. Filter digest input MUST contain endpoint kind and each canonical filter as tag byte,
u32 big-endian byte length, and exact UTF-8 bytes. List includes `q` and Group. Offers
includes Group and exact model.

MM-C5. A list cursor appends Group ordinal as u64 big-endian, model byte length as u16
big-endian, and exact model UTF-8 bytes. Its total ASCII length is at most 512.

MM-C6. An offer cursor appends Provider priority as i32 big-endian two's complement,
Provider public-name length and bytes, then Channel public-name length and bytes. Its total
ASCII length is at most 1,024.

MM-C7. A response cursor stores its final returned key. The next query selects strictly
after that key.

MM-C8. Malformed syntax, signature failure, endpoint, filter, or limit mismatch returns
HTTP `400` `invalid_request`. A validly signed revision mismatch returns HTTP `409`
`marketplace_cursor_stale`.

MM-C9. A cursor MUST NOT contain an internal database ID or internal name.

## 4. Persistent generation

MM-G1. `marketplace_generation` MUST contain exactly one row:

```text
singleton_id SMALLINT PRIMARY KEY CHECK = 1
revision BIGINT NOT NULL CHECK 1..9223372036854775807
generated_at_unix_us BIGINT NOT NULL CHECK 0..253402300799999999
```

MM-G2. Migration MUST create source rows before the generation row and source triggers.
The initial revision is one and initial time is current database UTC.

MM-G3. Deletion of the singleton MUST fail. PostgreSQL `TRUNCATE
marketplace_generation` MUST fail. SQLite product and maintenance code MUST not truncate it.

MM-G4. A direct generation update is valid only when ID remains one, revision equals old
plus one, and time is strictly later. Rollback, repeat, skip, ID change, non-increasing time,
or overflow MUST abort the transaction.

MM-G5. Source tables are `monoize_groups`, `monoize_providers`,
`monoize_provider_models`, `billing_rate_records`, `model_metadata_records`, and filtered
`system_settings`.

MM-G6. The first five tables are full sources. A machine-readable manifest MUST classify
every column as included or excluded. An UPDATE trigger covers every included column and
fires when it appears in the update target list.

MM-G7. `system_settings` includes only the logical row `reasoning_suffix_map`. Its included
columns are `key` and `value`; `updated_at` is excluded. Insert, delete, key movement into
or out of this row, and byte-different value change advance generation. Another setting,
an updated-time-only write, or a byte-identical write does not.

MM-G8. Marketplace snapshot construction MUST read persisted `reasoning_suffix_map` inside
the generation-checked read. A missing row uses `default_reasoning_suffix_map()`. Invalid
JSON returns HTTP `503` `marketplace_source_invalid` without runtime-snapshot fallback.

MM-G9. PostgreSQL MUST use one statement-level trigger for each source operation and table.
Full-source plain operations advance once, including zero-row statements. Upsert update may
advance once for INSERT and once for relevant UPDATE. Conflict-do-nothing advances once for
INSERT. Filtered settings triggers use transition tables and advance only for MM-G7 change.

MM-G10. PostgreSQL MUST have 18 source-operation and six source-table TRUNCATE triggers.
Each source TRUNCATE advances generation, including an empty table.

MM-G11. SQLite MUST use row-level triggers. Each affected full-source row advances once.
Its three settings triggers use MM-G7 predicates. SQLite MUST have 18 source-operation
triggers and MUST not bypass them to emulate TRUNCATE.

MM-G12. Every trigger invocation atomically increments revision and sets time to
`max(database_clock_us, old_time + 1)`. A malformed, missing, exhausted, or blocked
singleton aborts the source transaction. Rollback restores source and generation state.

MM-G13. The generation manifest MUST list six tables, filtered-row rule, column
classifications, trigger names, and every management, synchronization, seed, maintenance,
migration, and direct-SQL writer. Rehearsal and deployment preflight MUST compare it with
database metadata and repository search.

## 5. Consistent snapshots, queries, and size

MM-S1. Before cache lookup, read current generation. On a cache miss, read source rows,
read generation again, and accept only equal values. Retry at most three builds. Three
changes return HTTP `503` `marketplace_snapshot_busy`.

MM-S2. Response revision and generated time MUST come from the stable generation row.
Cache eviction or expiry MUST NOT recompute time.

MM-S3. A process-local LRU MUST store at most 256 Marketplace response snapshots and
expire one 60 seconds after last access.

MM-S4. One exact uncompressed JSON body MUST not exceed 1,048,576 bytes. Query at most
`limit + 1`; stop before the first item that would exceed the final envelope and set the
cursor from the last included key. If no item fits, return HTTP `500`
`public_response_too_large`, do not cache, and emit an internal metric.

MM-S5. List query MUST select at most `limit + 1` distinct Group/model rows after search
and keyset filtering, load their visible offers in one set-based query, and load rates and
metadata in set-based batches.

MM-S5.1. A stable Marketplace generation MAY build one immutable Group/model candidate
projection. The projection MUST contain one row per visible Group/model pair and MUST contain
no Provider secret, upstream URL, internal credential, or price. A List cache-miss MUST read at
most `limit + 1` rows from this projection before it counts visible offers. It MUST count offers
only for the selected candidate keys. A zero-candidate page executes one candidate statement.
A non-empty page executes one candidate statement and one set-based offer-count statement.

MM-S6. Offer query MUST select at most `limit + 1` visible offers after the keyset and load
rates and metadata in set-based batches. Query count may grow only through bounded SQLite
bind chunks, not by returned row count.

## 6. Effective public prices

MM-P1. Only an enabled Provider with enabled embedded Channel and a priced mapping may
contribute an offer. Process-local breaker state MUST NOT exclude a configured offer.

MM-P1a. A disabled Provider contributes zero items to `GET /api/public/marketplace` and
zero offers to `GET /api/public/marketplace/offers`, even when its embedded Channel and
priced model mapping remain enabled.

MM-P2. For one integer nano-USD base rate `r` and exact decimal effective multiplier `m`,
the displayed rate is exact decimal `r * m` without per-unit truncation.

MM-P3. Input and output ranges use minimum and maximum MM-P2 values within one Group and
logical model. Different Groups MUST render separately.

MM-P4. Display rates are informational. The API and modal MUST state that billing sums
integer line items, applies one multiplier, and truncates once at final charge.

MM-P5. Public rates MUST serialize as canonical non-negative decimal strings in nano-USD
per source unit, with at most nine fractional digits and no exponent. Frontend MUST NOT
parse them through JavaScript `Number` or calculate multiplier products.

## 7. Browser behavior

MM-U1. The page MUST use public APIs and SWR. Its keys MUST be separate from authenticated
Playground Marketplace keys.

MM-U2. Initial list loading and hydration MUST render matching Skeletons. Browser state
MUST retain at most three list pages and discard earlier pages when advancing.

MM-U2.1. If SWR returns a current first page, that page MUST replace the first retained
page. Later retained pages MUST remain in order. If current SWR data is absent, the page
MUST use retained pages. The empty state MUST render only when all resolved pages contain
zero items.

MM-U3. Model details MUST open in a modal, not an expanded table row. Click, tap, Enter,
and Space MUST open it. Focus moves to its heading, remains trapped, and returns to the
invoker after Escape or close. Background scroll is disabled and `aria-labelledby` is set.

MM-U4. The modal MUST show Group, Provider, and Channel public names, API type, input and
output prices, and every applicable cache, image, tool, duration, and meter rate.

MM-U5. Opening fetches the first offers page with an SWR key over Group, model, revision,
and cursor. A matching Skeleton displays until it resolves. A localized Load more action
fetches another page. Success appends without closing. Failure shows inline retry.

MM-U6. `marketplace_cursor_stale` MUST clear public Marketplace keys, close the modal,
fetch the first list page, and show a localized catalog-updated notice. Rows from different
revisions MUST NOT combine.

MM-U7. Every user-visible string MUST exist in `en`, `zh`, `zh-TW`, and `ja` catalogs.

### 7.1 Authenticated Marketplace

MM-UA1. `/dashboard/marketplace` MUST require an authenticated session and render inside
`DashboardLayout`. Navigating to it from another Console page MUST preserve the Console
shell.

MM-UA2. The page MUST use the public Marketplace allow-list response or an authenticated
response with the same field allow-list. It MUST NOT expose internal Provider names,
Channel names, IDs, Base URLs, API keys, proxy URLs, custom headers, multipliers, Profile
names, or internal errors.

MM-UA3. The page MUST render Group sections explicitly. A model in two Groups MUST render
once in each Group. Two Groups MUST NOT share one combined price range.

MM-UA4. The toolbar MUST provide Group and capability filters plus a CNY/USD segmented
control. The currency value MUST use the same in-memory Store currency provider as
`/dashboard/store`. It MUST NOT use `localStorage`.

MM-UA5. Currency conversion MUST use the current Store exchange-rate snapshot. CNY and USD
prices MUST render as `¥<amount> / 1M tokens` and `$<amount> / 1M tokens`, respectively.
The page MUST NOT show nano-USD values to a user.

MM-UA6. A per-token nano-USD rate MUST first multiply by exactly 1,000,000 source units.
Currency conversion MUST use exact integer or rational arithmetic. Rounding MUST occur once
at the final currency minor unit.

MM-UA7. The list MUST use compact model rows that expand naturally with the number of Groups
and models. It MUST NOT use a fixed viewport table height.

MM-UA8. Offer details MUST open in a modal. The modal MUST show only allow-listed Group,
Provider, Channel, API type, capability, and human-price values.

MM-UA9. The page MUST use SWR, show shape-matched initial Skeletons, retain the prior result
during filter changes, and expose inline retry for a failed request.

## 8. Qualification

MM-Q1. The supported catalog envelope is 128 Groups, 5,000 Providers and embedded
Channels, 250,000 model mappings, 1,000,000 rate rows, 250,000 metadata rows, 2,000,000
derived offer-rate entries, and one Group/model row with 5,000 Provider offers. One
Provider offer is one visible Provider-model mapping. One derived offer-rate entry is one
rate item nested in one Provider offer. Preflight fails above an envelope value.

MM-Q2. Read qualification MUST run on SQLite and PostgreSQL with 32 concurrent workers,
five warm-up minutes, ten measured minutes, and at least 10,000 verified cache-miss samples
per operation kind. List is 80 percent across zero, one, 50, and broad search and first,
middle, final cursor positions. Offers is 20 percent across three cursor positions.

MM-Q2.1. `mode=qualification` MUST reject `query_limit`. It MUST create 32 worker
connections after fixture load and `ANALYZE`. Each worker MUST cycle the same 400-case query
set from an offset equal to `worker_index mod 400`. Each sample MUST execute the exact
cursor and public DTO validation used by smoke mode.

MM-Q2.2. The warm-up clock starts after all workers are ready. Warm-up samples MUST execute
queries and validation but MUST NOT contribute to reported latency, sample, statement, or
response-byte counters. Measurement starts after at least 300 elapsed seconds.

MM-Q2.3. Measurement MUST continue until at least 600 elapsed seconds and at least 10,000
verified List samples and 10,000 verified Offers samples exist. A failed validation counts
as a sample and increments `failed_samples`. The report MUST record actual worker count,
whole warm-up seconds, and whole measured seconds.

MM-Q2.4. A read qualification passes only when worker, duration, per-kind sample, failed
sample, latency, memory, response-size, and exact source-count requirements all pass. A
read pass MUST NOT set Gate B passed while source-write qualification or another MM-Q6
requirement is absent.

MM-Q2.4.1. Qualification MUST sample process RSS at least once per second from before the
first worker connection until every worker exits. `rss_after_bytes` MUST store the maximum
sampled RSS, and `rss_delta_bytes` MUST equal `max(0, peak_rss - rss_before)`. A before/after
comparison without peak sampling does not satisfy MM-Q3.

MM-Q2.5. Fixture load MUST insert at most 500 rows per SQL statement. All fixture inserts
for one backend MUST use one transaction. A failed batch MUST leave zero committed fixture
rows. Qualification timing MUST start after fixture load, commit, read-back hashing, and
`ANALYZE` complete.

MM-Q2.6. The isolated read benchmark MUST build the MM-S5.1 projection in the same fixture
transaction. The source hash and source row counts MUST exclude the projection. SQLite and
PostgreSQL MUST analyze the projection before qualification timing starts.

MM-Q2.6.1. In the isolated read fixture, one mapping is priced if and only if at least one
`billing_rate_records` row for its logical model has `public_repeat_count > 0`. All mappings
for one logical model have the same fixture pricing state. Projection construction, List
offer counts, and Offers queries MUST apply this same predicate. Projection construction MUST
occur after rate insertion. Rebuilding the projection after a rate transaction MUST add a
newly priced Group/model pair and remove a newly unpriced Group/model pair.

MM-Q2.7. Each SQLite qualification worker connection MUST set `PRAGMA cache_size = -8192`.
The 32 worker page-cache budget MUST therefore be at most 256 MiB. The fixture-load and smoke
connections MAY use the existing 64 MiB cache. The fixture-load connection MUST close before
qualification RSS sampling starts.

MM-Q3. List latency MUST be p95 at most 500 ms and p99 at most 1,000 ms. Offers MUST be p95
at most 400 ms and p99 at most 800 ms. Application resident-memory increase MUST be at most
512 MiB. An uncompressed response MUST satisfy MM-S4.

MM-Q4. Source-write qualification MUST test 100,000-row insert, relevant update, delete,
upsert-update, and conflict-do-nothing transactions at statement sizes 1, 100, 1,000, and
10,000 on the three large source tables, plus eight disjoint writers and every real
full-catalog sync sequence.

MM-Q5. Source-write qualification requires zero PostgreSQL deadlocks, zero exhausted SQLite
busy retries, at most 60 seconds per single-writer or full-sync transaction, at least 5,000
committed source rows per second under eight writers, transaction p99 at most five seconds,
WAL growth below 2 GiB, and checkpoint below 30 seconds.

MM-Q6. Gate B fails if either database misses one read, write, query-count, memory, response,
lock, generation-delta, WAL, ordering, or cursor requirement.

MM-Q7. The isolated PostgreSQL benchmark MUST parse its connection URL before connection.
The parsed host MUST equal `127.0.0.1`; another host MUST return
`postgres_rehearsal_host_required`. The parsed database name MUST equal
`lynshen_rehearsal`. Another database name, a missing database name, or an invalid URL MUST
return `postgres_rehearsal_database_required`. Both errors MUST occur before connection or
DDL. The benchmark MUST drop and recreate only the
`lynshen_marketplace_benchmark` schema. It MUST NOT drop, recreate, truncate, or write the
`public` schema. Its connection MUST set `search_path` to `lynshen_marketplace_benchmark`
before creating a benchmark table. Reset, table creation, and index creation MUST run in one
transaction. A failure MUST restore the pre-run benchmark schema. PostgreSQL `ANALYZE` MUST
name exactly these six schema-qualified tables: `monoize_groups`, `monoize_providers`,
`monoize_provider_models`, `marketplace_group_models`, `billing_rate_records`, and
`model_metadata_records`. It MUST NOT analyze another schema.

MM-Q8. The rehearsal CLI MUST accept `sqlite`, `postgres`, and `paired` as benchmark
backends. The `postgres` and `paired` backends MUST read
`LYNSHEN_REHEARSAL_POSTGRES_URL`. A missing value MUST return
`missing LYNSHEN_REHEARSAL_POSTGRES_URL` before benchmark execution. `paired` MUST run
SQLite and PostgreSQL with one identical BenchmarkConfig and write one comparison report.

MM-Q9. SQLite and PostgreSQL benchmark reports for one seed and envelope MUST have equal
`fixture_recipe_sha256`, `loaded_source_sha256`, `query_set_sha256`, loaded row counts, and
materialized offer-rate entry counts. A mismatch fails rehearsal and MUST NOT qualify Gate B.
The paired report MUST contain both complete backend reports, `comparison_passed: true`, and
`gate_b_qualified: false` until every MM-Q2 through MM-Q6 requirement is recorded as passed.
