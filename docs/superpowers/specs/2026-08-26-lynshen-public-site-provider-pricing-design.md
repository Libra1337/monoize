# LynShen Public Site, Provider, Pricing, and Status Design

Date: 2026-08-26

Status: Proposed after read-only preflight. This document authorizes no product implementation and no production mutation.

## 1. Objective

Add a public site, public API documentation, a Group-aware model marketplace, and a public Provider status page.

Replace the existing many-to-many routing shape with these invariants:

```text
Group 1 --- N Provider
Provider 1 --- 1 Channel
```

Add Provider-level Billing Profile and multiplier defaults. Allow one model to override either value.

Keep existing forwarding endpoint paths unchanged. Change only dashboard management contracts and public read contracts.

## 2. Authorization Boundary

The current authorization covers this design document and read-only preflight checks.

Do not change behavioral files under `spec/` until this document receives user approval.

Do not implement migrations, backend behavior, frontend behavior, or documentation pages under this authorization.

Do not deploy or mutate production under this authorization.

Require a separate user approval before implementation. Require another separate approval before production deployment.

## 3. Verified Baseline

The preflight inspected the repository and production through read-only commands.

### 3.1 Repository

- The repository root is `C:/Users/Administrator/Documents/GitHub/monoize`.
- The checked-out branch is `master` at commit `cf36bd8`.
- The inspected source, specification, frontend, and documentation trees contain 654 files.
- The backend uses Rust, Axum, SeaORM, SQLite, and PostgreSQL.
- The frontend uses React, React Router, SWR, and Bun.
- The existing Marketplace path is `/dashboard/marketplace`.
- The existing Provider contract contains `group_ids` and `channels`.
- The existing router orders Providers by priority and performs weighted random ordering across Channels inside one Provider.
- The existing billing path uses exact decimal multipliers and truncates scaled integer nano-USD values toward zero.
- The existing error classifier distinguishes RateLimited, Transient, Persistent, and excluded client errors.

### 3.2 Production

- `lynshen.org` and `www.lynshen.org` resolve publicly to `103.240.199.109`.
- Direct HTTPS requests to that address pass certificate validation and return HTTP 200.
- Caddy terminates TLS and proxies both names to `127.0.0.1:8080`.
- HSTS is enabled for one year and includes subdomains.
- The `monoize` system service is enabled and active.
- The production container is healthy.
- The production image is `monoize:cf36bd8` with digest `sha256:174425c3ec517ad117276a4e8dc6ee809f68e274e4b89a19839e185cfbdf2cd8`.
- Production uses SQLite in WAL mode under `/opt/monoize/data`.
- SQLite `PRAGMA quick_check` returned `ok`.
- Production currently contains one Provider, one Group assignment, one Channel, and nine model mappings.
- Every current model multiplier is exactly `1`.
- The current production data does not require a Cartesian Provider expansion.
- Retained Caddy logs contain 81 Provider-management requests. Every observed request used a browser User-Agent.

The access-log result does not prove that no external management client exists.

### 3.3 Existing Backup

- The existing backup checksum file validates the database and service-unit artifacts.
- The backup database passes SQLite `PRAGMA quick_check`.
- The backup contains zero Providers while production contains one Provider.

The existing backup is not an acceptable rollback point for this change.

## 4. Release Decomposition

Use four controlled phases.

### Phase 0: Design and Preflight

Produce specifications, preflight tooling design, migration fixtures, and release gates.

Do not implement product behavior in this phase.

### Phase 1: Migration and Core Contracts

Implement schema migration, Provider storage, pricing resolution, and status-event persistence.

Do not expose new public pages in this phase.

Do not merge or deploy Phase 1 independently. The destructive schema and every updated repository caller must ship as one release after Phase 2 passes all gates.

### Phase 2: Public Product Surfaces

Implement the public layout, welcome page, Marketplace, API documentation, status page, and management UI.

Build all four locales and complete security review.

### Phase 3: Production Cutover

Create and restore-test a current backup. Use a maintenance window for the destructive migration.

Do not run the old and new binaries concurrently against the migrated SQLite database.

## 5. Public Routes and Layouts

Use these exact browser routes:

| Path | Surface | Authentication |
| --- | --- | --- |
| `/` | Public welcome page | None |
| `/login` | Login and registration page | None |
| `/apidocs` | Public API documentation | None |
| `/status` | Public Group and Provider status | None |
| `/dashboard/marketplace` | Public model marketplace | None |
| `/dashboard` and all other `/dashboard/*` paths | Console | Required |
| `/settings` | User settings | Required |

Register `/dashboard/marketplace` as an explicit top-level public route. Do not render it inside `DashboardLayout`.

Keep the route path unchanged. Remove the Marketplace child from the protected `/dashboard` route.

Keep `/` public for authenticated and unauthenticated visitors. Do not redirect authenticated visitors away from `/`.

Use one `PublicLayout` for `/`, `/apidocs`, `/status`, and `/dashboard/marketplace`.

Render these public navigation entries: Home, Model Marketplace, API Docs, Status, Console, and Login.

## 6. Public Visual Design

Use the approved Paper Console direction.

Use the repository design tokens from `DESIGN_SYSTEM.md` and `spec/frontend-design-system.spec.md`.

Use the neutral background, blue primary color, serif display font, sans-serif body font, and code font.

Do not introduce a second brand palette.

The welcome page contains these sections in order:

1. Product statement and two actions.
2. Supported API families.
3. Group and pricing explanation.
4. Three-step connection flow.
5. API code example.
6. Status-page action.

Do not display model, Provider, Channel, or Group counts on the welcome page.

Animate only `transform` and `opacity`. Use 150-300 millisecond interaction durations.

Disable positional and repeating animation under `prefers-reduced-motion: reduce`.

Support 375, 768, 1024, and 1440 pixel viewports without horizontal scrolling.

Meet WCAG AA text contrast. Use at least 16 CSS pixels for body text at the 375-pixel viewport. Keep prose lines at 65-75 characters when space permits.

Provide a keyboard-visible skip link to main content. Use semantic landmarks and headings in descending order. Provide visible 3-pixel focus rings, accessible names for icon-only actions, and 44-by-44 pixel touch targets.

Use Lucide or the existing product icon set. Do not use emoji as interface icons. Hover and active states must not shift layout.

## 7. Provider, Channel, and Group Storage

### 7.1 Provider

Replace `monoize_providers.group_ids` with `group_id TEXT NOT NULL`.

Add a foreign key from `monoize_providers.group_id` to `monoize_groups.id`.

Add these Provider fields:

- `pricing_profile TEXT NULL`
- `multiplier TEXT NOT NULL`
- `public_name TEXT NOT NULL`
- `configuration_generation BIGINT NOT NULL`

Keep every existing Provider field that section 7.4 does not remove. Keep its current validation and runtime semantics unless this document states a replacement.

Validate `multiplier` with the existing exact decimal type. Require a value greater than zero and at most nine fractional digits.

Treat a null `pricing_profile` as no Provider default. Require a model override before that model can become priced.

For every non-null Provider default or model override Profile input, trim surrounding Unicode White_Space code points first. Reject an empty trimmed value.

Compare the trimmed value case-sensitively with a Profile present in at least one billing-rate record. Persist and return only the trimmed value. Do not case-fold it.

Allow that Profile to lack rates for a specific model. Report that mapping as unpriced.

Use `public_name` on public responses. Do not expose the internal Provider `name`.

For each Group, Provider, and Channel public name, trim surrounding Unicode whitespace and normalize the result to Unicode NFC. Require 1-64 Unicode scalar values. Reject C0 controls, DEL, CR, LF, and tab. Persist and return the normalized value.

After migration, require a management create request that assigns a Group, Provider, or Channel `public_name` to include `confirm_public_exposure: true`.

Require the same field when an update changes a normalized public name. For a Provider request, one top-level confirmation covers the Provider and embedded Channel names.

Treat the field as request-only. Do not persist or return it. Reject a missing or non-true confirmation with HTTP 400 and `public_exposure_confirmation_required`.

The reviewed migration manifest supplies the equivalent confirmation for migrated names.

Initialize `configuration_generation` to one during migration. Increment it exactly once in the same transaction as each logical mutation to the Provider row, embedded Channel, or Provider model mappings. A failed transaction does not increment it.

Treat migrated Provider and Channel IDs as opaque legacy IDs. Generate lowercase UUID v4 IDs for every Provider and embedded Channel created after migration. Do not accept either ID in a create body. Do not allow either ID to change in an update body.

### 7.2 Channel

Keep Channel as a first-class API, runtime, status, and UI value object. Do not keep it as a separate database entity.

Move every surviving Channel column into `monoize_providers` with a `channel_` prefix. This includes Channel ID, internal name, public name, API type, Base URL, API key, enabled state, retry and breaker overrides, probe overrides, affinity overrides, proxy URL, extra headers, session affinity, and missing-usage policy.

Make every required embedded Channel column `NOT NULL`. Make `channel_id` unique.

A Provider row cannot exist without its embedded Channel fields. One Provider row contains exactly one Channel. This enforces the invariant identically on SQLite and PostgreSQL.

Create and update the Provider and embedded Channel through one row mutation. Return one singular Channel value in the API.

Replace `monoize_channel_models` with `monoize_provider_models`. Use `(provider_id, model_name)` as its unique logical key. Cascade model deletion from `monoize_providers.id`.

Use this `monoize_provider_models` shape:

- `provider_id TEXT NOT NULL`
- `model_name TEXT NOT NULL`
- `redirect TEXT NULL`
- `pricing_profile_mode TEXT NOT NULL`
- `pricing_profile_override TEXT NULL`
- `multiplier_override TEXT NULL`
- `created_at TEXT NOT NULL`

Use `(provider_id, model_name)` as the primary key. Do not retain a separate model-mapping row ID. Use the logical model name in validation warnings and management responses.

Remove Channel `weight`. Convert `weight <= 0` to `enabled = false` during migration.

### 7.3 Group

Add `public_name TEXT NOT NULL` to `monoize_groups`.

Use the public Group name on public responses. Do not expose an unapproved internal label.

Require public Group names to be unique after trimming and Unicode NFC normalization. Compare the normalized values case-sensitively.

Reject Group deletion with HTTP 409 and `group_in_use` while a Provider references that Group. Require the administrator to move or delete those Providers first.

Keep the existing effective-Group routing order. For a request with non-null ordered effective Groups, rank a Provider by the index of its single `group_id`. Within one rank, order Providers by `priority ASC`, then the existing deterministic tie breakers. An empty effective-Group list selects no Provider. A null list for internal traffic makes every Group eligible.

### 7.4 Obsolete Contract

Remove these obsolete fields and behaviors:

- Provider `group_ids`
- Provider `channels`
- Channel `weight`
- Provider `max_retries`
- Multi-Channel weighted selection
- Multi-Channel Provider attempt limits
- Old dashboard Channel test route
- `monoize_channels` table and entity
- `monoize_channel_models` table and entity
- Channel database store

Keep `channel_max_retries` as the same-Channel physical retry count.

Allow a Provider to contain zero model mappings. Such a Provider is ineligible for every model route and absent from Marketplace model offers until it receives a mapping.

Do not keep compatibility columns, tables, aliases, request fields, or response fields.

## 8. Provider API Contract

Use these management paths:

```text
GET    /api/dashboard/providers
POST   /api/dashboard/providers
GET    /api/dashboard/providers/{provider_id}
PUT    /api/dashboard/providers/{provider_id}
DELETE /api/dashboard/providers/{provider_id}
POST   /api/dashboard/providers/{provider_id}/channel/test
POST   /api/dashboard/providers/reorder
```

Require the reorder body to contain one `group_id` and every current Provider ID in that Group exactly once. Reject an ID from another Group. Assign dense priorities starting at zero inside that Group. Do not change another Group's priorities.

Use singular fields in Provider requests and responses:

```json
{
  "group_id": "group-id",
  "pricing_profile": "openai",
  "multiplier": "1.2",
  "public_name": "Provider A",
  "confirm_public_exposure": true,
  "channel": {
    "public_name": "Primary Channel"
  }
}
```

Use the same request-only `confirm_public_exposure` field on Group create and update requests under the rules in section 7.1.

Reject `group_ids`, `channels`, and unknown legacy fields with HTTP 400 and `invalid_request`.

Keep `/v1/**` forwarding request contracts unchanged.

## 9. Migration Rules

### 9.1 Preflight Classification

Classify each old Provider before migration.

An old Provider is route-safe when it has exactly one Group ID and at most one enabled Channel with positive weight.

Block migration when a Provider has zero Group IDs, zero Channel rows, an unknown Group ID, or malformed persisted configuration.

An old Provider requires explicit semantic-change approval when it has two or more enabled Channels with positive weight or more than one Group ID.

Multiple enabled Channels cannot preserve weighted random ordering and `max_retries` after conversion to single-Channel Providers. Multiple Group IDs cannot preserve the old single-candidate semantics for a request whose effective Groups overlap more than one expanded row.

Do not claim semantic equivalence for that case.

The current production database is route-safe.

### 9.2 Expansion

Expand one old Provider into one new Provider for every `(Group, Channel)` pair.

Include disabled and zero-weight Channels in the expansion so the migration preserves their configuration. Convert each zero-weight Channel to disabled.

Copy the selected old Channel fields into the expanded Provider row. Copy its model mappings into `monoize_provider_models` under the expanded Provider ID.

Use stored Group order. Sort Channels by `created_at ASC, id ASC`.

Keep the original Provider ID on the first pair in the expansion order. Keep each old Channel ID on that Channel's first Group pair. This preserves every old Provider and Channel identifier at least once for historical request-log lookup.

Generate a Provider ID for every pair after the first. Generate a Channel ID for every copy after that old Channel's first Group pair. Derive each ID from a deterministic SHA-256 digest of a versioned entity type, old Provider ID, Group ID, and old Channel ID. Encode every digest input as a length-prefixed UTF-8 byte string so concatenation cannot create an ambiguous input.

Use `p_` for a generated Provider ID and `c_` for a generated Channel ID, followed by the first 32 lowercase hexadecimal digest characters.

Abort before writing when a generated ID collides with an existing or generated ID.

Keep the old Provider internal name on the row that keeps the old Provider ID. Generate the internal name as `old Provider name / Group name / Channel name` for every other expanded row.

Append an eight-character deterministic digest only when generated internal names collide.

Do not rewrite historical request-log IDs or `tried_providers_json`. Update request-log lookup joins to resolve a Channel from the embedded `monoize_providers.channel_id`. A historical Provider ID resolves to the row that kept the old Provider ID. A historical Channel ID resolves to that Channel's first Group copy. Persisted request-time name snapshots remain authoritative when present.

### 9.3 Ordering

For each Group, sort expanded rows by this key:

```text
old Provider priority,
old Provider created_at,
old Provider id,
old Channel created_at,
old Channel id
```

Assign dense Provider priorities starting at zero in that order.

For a route-safe database, this preserves the previous Provider precedence.

For a semantic-change database, record the exact before-and-after attempt order in the preflight report.

### 9.4 Public Names

The preflight emits proposed public names for every Group, Provider, and Channel.

Require an explicit reviewed public-name manifest before migration execution.

Build the manifest deterministically from the database. Include every migration-relevant field in its SHA-256 fingerprint. Feed `HMAC-SHA-256(comparison_key, field_tag || secret_value)`, rather than a raw secret or an unkeyed secret digest, into the fingerprint for each API key, proxy credential, or other secret field. Generate the comparison key before the first preflight, keep it in a separate owner-readable file, reuse it for the final preflight, and destroy it after cutover or rollback. Do not place the key in the reviewed manifest. Include the proposed target IDs and public names.

Re-run preflight after writes stop. Require the reviewed fingerprint to equal the final preflight fingerprint before migration starts.

Require public names to be non-empty after trimming.

Require Provider public names to be unique within one Group.

Never infer approval from the presence of an existing internal name.

### 9.5 Transaction and Repeatability

Execute all schema changes and data changes in one database transaction per backend.

Use the SeaORM migration ledger to prevent a successful migration from running again.

Make the migration function a no-op when the complete target schema and migration ledger entry already exist.

Roll back every change when any validation, schema operation, row write, or constraint creation fails.

Test a failure after every migration stage. Verify that the old schema and rows remain unchanged.

## 10. Pricing Resolution

### 10.1 Effective Configuration

Add nullable model fields:

- `pricing_profile_mode TEXT NOT NULL`
- `pricing_profile_override TEXT NULL`
- `multiplier_override TEXT NULL`

Remove the old required model `multiplier` column after migration.

Allow `pricing_profile_mode` values `inherit`, `override`, and `unpriced` only. Require a non-null valid `pricing_profile_override` exactly when the mode is `override`. Require a null override in the other two modes. Validate an override Profile against the same billing-rate Profile set as the Provider default.

Apply the Profile trim, case-sensitive validation, persistence, and response rules from section 7.1 to `pricing_profile_override` before runtime lookup.

Validate a non-null `multiplier_override` with the same exact-decimal, positivity, and fractional-digit rules as the Provider multiplier.

Resolve values as follows:

```text
if model.pricing_profile_mode == "inherit":
    effective_profile = provider.pricing_profile
if model.pricing_profile_mode == "override":
    effective_profile = model.pricing_profile_override
if model.pricing_profile_mode == "unpriced":
    effective_profile = null

effective_multiplier = model.multiplier_override ?? provider.multiplier
```

Use `effective_multiplier` for both charge arithmetic and the existing request `max_multiplier` routing filter.

Within the selected Profile, test the normalized upstream model first. Test the normalized logical model second.

Use the first complete applicable rate matrix. Keep the existing provider-type and usage-class matching rules.

Remove global pricing-profile pattern selection from runtime Provider billing after migration.

Keep model metadata Profile values as administrative suggestions only. Do not use them as an implicit billing fallback.

### 10.2 Charge Arithmetic

Keep the current charge arithmetic.

Compute all base token and meter line items in integer nano-USD.

Sum the base line items. Apply the effective multiplier once to the aggregate base charge.

Use exact decimal scaling. Truncate toward zero at the same final scaling step.

Do not calculate billable prices with JavaScript floating-point arithmetic.

### 10.3 Migration Inference

Before migration, resolve the effective Profile, pricing model, rate records, and multiplier for every Provider model mapping.

Choose the most frequent resolved Profile as the Provider default.

Count only non-null resolved Profiles. Use a null Provider default when every old mapping is unpriced.

Break a Profile frequency tie by ascending UTF-8 byte order.

Store Profile mode `inherit` when the old resolved Profile equals the Provider default.

Store Profile mode `override` and the old resolved Profile when that Profile is non-null and differs from the Provider default.

Store Profile mode `unpriced` when the old resolved Profile is null. This explicit mode preserves an unpriced model even when another model supplies a non-null Provider default.

Choose the most frequent canonical multiplier as the Provider default. Use canonical multiplier `1` when the expanded Provider has no model mappings.

Break a multiplier frequency tie by ascending exact decimal numeric value.

Store a model multiplier override whenever its old multiplier differs from the Provider default.

These defaults are storage compression only. They must not change any effective model value.

### 10.4 Pricing Golden Snapshot

Create a canonical pre-migration pricing snapshot from the same database copy used for migration rehearsal.

Project each old mapping once for every target Group produced by section 9.2. Use the deterministic target Provider and Channel IDs in that projected pre-migration snapshot. This makes the pre-migration and post-migration snapshot cardinalities equal after Group expansion.

Include these fields for every mapping:

- An opaque mapping digest
- Logical model
- Upstream model
- Provider type
- Resolved Profile
- Resolved pricing model
- Ordered rate-record IDs and integer rates
- Canonical multiplier
- Exact decimal display rates for each applicable unit
- Charges for the defined usage scenarios

Use these usage scenarios when their usage classes apply:

- One unit
- 999 units
- One million units
- Input only
- Output only
- Cache read only
- Each cache write duration
- Each meter class
- Mixed token and meter usage

Freeze and hash the billing-rate rows used by the snapshot.

Generate the same snapshot after migration.

Block release when any resolved field, exact decimal display rate, scenario charge, or rate-table digest differs.

Do not promise price preservation without this equality result.

## 11. Missing Pricing

Allow an administrator to save an unpriced Provider or model mapping.

Mark the mapping unpriced when its Profile mode is `unpriced`, no inherited Profile exists, or the selected Profile lacks a complete required rate matrix.

Exclude an unpriced mapping from routing and public Marketplace aggregation.

Return HTTP 403 and `model_pricing_required` when at least one structurally eligible mapping exists but every such mapping is unpriced. Preserve the existing no-model or no-Provider error when no structurally eligible mapping exists.

Restore routing and Marketplace visibility automatically after required rates become available.

Return structured warnings from Provider create and update responses. Include model IDs and missing usage classes.

## 12. Public Marketplace

Keep the exact path `/dashboard/marketplace`.

Add `GET /api/public/marketplace` for the public page. Keep `GET /api/dashboard/marketplace/models` authenticated for Playground and other Console consumers.

The public endpoint returns only public Group, Provider, Channel, model, capability, and effective price fields defined in this section. Encode every displayed rate as a canonical non-negative base-10 decimal JSON string in nano-USD per source unit, with at most nine fractional digits and no exponent notation. The frontend parses it as a decimal string and never as a JavaScript number.

Return this allow-listed logical response shape:

```text
generated_at: RFC3339 UTC
groups: Array<{
  public_name: string,
  models: Array<{
    model: string,
    capabilities: string[],
    input_rate_range: { min: decimal string, max: decimal string, unit: string } | null,
    output_rate_range: { min: decimal string, max: decimal string, unit: string } | null,
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
  }>
}>
```

Derive `capabilities` and every rate discriminator from the selected complete billing-rate matrix and reviewed model metadata. Do not pass through arbitrary metadata JSON.

Group records by public Group name. Never merge records across Groups.

Within one Group, merge equal logical model IDs into one row.

For one offer rate whose base integer nano-USD unit price is `r` and effective multiplier is the exact decimal `m`, compute the display rate as the exact product `r * m`. Do not truncate the per-unit display rate. Compute input and output ranges from those display rates for mappings whose Provider and single Channel are enabled and priced.

The display rate is informational. The billing engine continues to sum integer base line items, apply the effective multiplier once, and truncate once at final charge. State this rule in the Marketplace modal and API Docs.

Do not exclude a priced Provider solely because its process-local circuit breaker is currently open.

The Marketplace describes configured sale availability. The status page describes recent runtime availability.

Open model details in a modal dialog. Do not expand a table row inline.

Open the dialog by click, tap, Enter, or Space. Move focus to its heading. Trap focus while open. Close on Escape or the close action. Return focus to the invoking model row. Set `aria-labelledby` and prevent background scrolling.

Show these fields in the modal:

- Public Group name
- Public Provider name
- Public Channel name
- Channel API type
- Input and output display prices
- Every applicable cache, image, tool, duration, and meter rate

Do not return Billing Profile names, multipliers, billing-rate row IDs, internal IDs, internal names, Base URLs, API keys, proxy URLs, custom headers, or internal errors.

Return the modal offers in the same Marketplace response as the model range. Do not perform a second unauthenticated detail request when the dialog opens. Order Groups by current Group order, models by logical model ID, and offers by Provider priority then public Provider and Channel name.

Use backend-produced exact decimal display values. Format only those decimal strings in the frontend. The browser must not multiply a base rate by a multiplier.

Use SWR. Render matching Skeleton layouts while loading.

Keep public Marketplace data in a separate SWR key from authenticated Playground data. The Playground must apply the authenticated user's effective Group restrictions and must not treat the public all-Group response as an authorization decision.

## 13. Upstream Status Events

### 13.1 Event Table

Create `upstream_call_events` with these fields:

- `id TEXT PRIMARY KEY`
- `group_id TEXT NOT NULL`
- `provider_id TEXT NOT NULL`
- `channel_id TEXT NOT NULL`
- `outcome TEXT NOT NULL`
- `failure_class TEXT NULL`
- `upstream_status INTEGER NULL`
- `occurred_at_unix_ms BIGINT NOT NULL`
- `source_node_id TEXT NOT NULL`
- `provider_generation BIGINT NOT NULL`

Allow `outcome` values `success` and `failure` only.

Allow failure classes `rate_limited`, `transient`, and `persistent` only.

Do not store user IDs, API key IDs, request bodies, response bodies, prompts, model content, URLs, or error text.

Do not add foreign keys to mutable Provider, Channel, or Group rows. Join current configuration when producing public output.

Index `(provider_id, occurred_at_unix_ms)` and `(occurred_at_unix_ms)`.

Delete events older than 48 hours in batches of at most 1000 rows. Run cleanup once at startup after migrations and once every 15 minutes. Log a cleanup error and continue serving; retry it on the next interval.

Create `status_source_state` with one row per `source_node_id`. Store `last_seen_at_unix_ms`, `ship_interval_ms`, `pending_event_count`, `oldest_pending_event_unix_ms`, `retired_at_unix_ms`, `clock_synchronized`, `clock_good_heartbeat_streak`, `incomplete_since_unix_ms`, and `incomplete_until_unix_ms`. Update the Primary row locally at least once per configured ship interval. Update a Replica row in the same transaction that applies its heartbeat and status-event batch. Do not expose source-node identifiers through the public API.

### 13.2 Event Identity

Assign a monotonically increasing physical dispatch index inside each canonical request lifecycle. Start at zero and increment once immediately before every physical upstream dispatch, including same-Channel retries and Provider fail-forward attempts.

Create one internal lifecycle UUID v4 at admission. Do not derive it from a client-supplied request ID.

For forwarding paths without dashboard request-log admission, create the lifecycle UUID before the first upstream dispatch.

Build the event ID as the exact UTF-8 string `{source_node_id}.{lowercase-hyphenated-lifecycle-uuid}.{base-10-dispatch-index}`. The allowed source node IDs make this string a valid cross-platform spool filename stem.

Use `primary` as the source node ID on a Primary. Use the stable Replica UUID from the existing metering identity contract on a Replica.

Reuse the same ID during retry of event persistence or replica shipment.

Insert with conflict-ignore semantics. A replay must not increase event counts.

### 13.3 Counting Boundary

Evaluate every completed physical upstream HTTP or WebSocket dispatch for event creation. Set `occurred_at_unix_ms` to the source node's Unix time when that physical dispatch reaches its terminal outcome.

Create an event only for a usable terminal success or for a failure assigned one of the three allowed classes by the shared health classifier. Do not create an event for an excluded outcome.

Count every same-Channel physical retry separately.

Record success only when the upstream attempt produces a usable terminal success.

Record a streaming attempt as failure when an in-stream terminal upstream error occurs.

Record a disconnected client as success when upstream processing still completes as a billable success.

Reuse the failure class already produced for passive Channel health handling.

Count RateLimited, Transient, and Persistent failures.

Exclude an outcome when the existing health classifier returns no failure class and the outcome is not a usable terminal success.

Therefore, exclude ordinary upstream 400, 409, and 422 responses without a listed structured failure signal.

Count upstream credential, quota, model availability, network, timeout, rate-limit, and server failures when the shared classifier selects them.

Exclude local authentication, authorization, balance, validation, transform, encoding, pricing, and internal failures that occur before upstream dispatch.

Exclude active probes and dashboard connectivity tests.

### 13.4 Persistence Failure Policy

Do not change the user API result when status-event persistence fails.

Write events through a dedicated durable spool and asynchronous batcher. Use one JSON file per event, temporary-file write plus file sync, same-directory atomic rename, and directory sync. Use stable event IDs for filenames. Recover final files at startup and insert with conflict-ignore semantics.

Use `MONOIZE_STATUS_EVENT_SPOOL_DIR`, default `./data/status-event-spool`. Use `MONOIZE_STATUS_EVENT_SPOOL_MAX_BYTES`, default `536870912`, as the process-local sum of durable bytes plus outstanding reservations. Use `MONOIZE_STATUS_EVENT_SPOOL_ENTRY_MAX_BYTES`, default `4096`, as the per-event reservation. Accept only positive base-10 integers. Reject startup when the entry limit is below `1024`, the total limit is below the entry limit, or the spool directory cannot pass create, write, file-sync, rename, directory-sync, and delete probes.

Reserve one full entry before a physical dispatch. Reservation failure does not block the dispatch. On a counted terminal outcome, publish the event into that reservation; if no reservation exists, set the incomplete-data latch because one required event was lost. On an excluded outcome, release the reservation without setting the latch. Publication failure for a counted event sets the latch and releases the reservation after cleanup of any temporary file.

Flush at most 100 events per database transaction and at least once every 2 seconds. Wake the batcher when a final spool file is published. Delete a final file only after the insert transaction commits. Retain it unchanged after a failed or ambiguous transaction outcome so replay is idempotent.

Set a node-local incomplete-data latch only when a counted event cannot be retained in either the durable spool or the committed event table. Record the first loss time and the most recent loss time. A transient database or shipment failure with an intact spool file is pending data, not lost data.

Persist the latch and a `clean_shutdown` boolean as a bounded JSON node-state file in the status spool directory. Limit it to 1024 bytes. Publish it with temporary-file write, file sync, same-directory atomic rename, and directory sync.

Before accepting traffic, atomically set `clean_shutdown = false`. When an existing node-state file already contains false at startup, treat the previous process lifetime as incomplete and extend `incomplete_until_unix_ms` to 24 hours after the new startup time. After graceful shutdown stops admission, drains physical dispatches, and drains the status spool, atomically set `clean_shutdown = true`. A missing state file on a newly created empty spool directory is a first start and does not imply loss.

Expose an error metric and an error log for every transition into incomplete state.

Extend every replica heartbeat with the source clock time, configured ship interval, status-event pending count, oldest pending event time, incomplete interval, and a retirement flag. Persist these values on the Primary in `status_source_state`, including heartbeat-only batches. Publishing a status event wakes the existing ship loop.

On graceful shutdown, send the retirement flag only after the status-event spool drains and the pending count is zero. A retired source does not affect later completeness. A later heartbeat from the same source ID clears retirement.

Set `clock_synchronized = false` when the heartbeat source clock differs from Primary receipt time by more than 30 seconds. Set it true after three consecutive heartbeats within that bound. A clock transition to false extends `incomplete_until_unix_ms` to at least 24 hours after Primary receipt time because source event timestamps may be misplaced.

Set `incomplete_until_unix_ms` to at least 24 hours after the most recent loss. Clear the latch only after all status-event spools drain and that time passes without another loss. A later loss extends the time.

Load the persisted latch before accepting traffic. Mark readiness unhealthy while a latch cannot be persisted because the data directory is unwritable.

If both event persistence and latch persistence fail, keep the in-memory latch set, mark readiness unhealthy, and emit a continuously visible error metric. The system cannot prove completeness across a crash under simultaneous loss of both persistence paths. Do not claim otherwise.

For each status response, define `active_source` as the local source or a non-retired persisted source seen within the previous 24 hours. Set `data_through_unix_ms = now - max(30000, 3 * maximum active_source.ship_interval_ms)`. Use 30 seconds when no persisted source is active.

Return `data_complete: false` when any of these conditions holds for the interval ending at `data_through_unix_ms`:

- an active source has a positive pending-event count and `oldest_pending_event_unix_ms <= data_through_unix_ms`;
- a non-retired source seen within the previous 24 hours has missed more than `max(30000, 3 * ship_interval_ms)` milliseconds of heartbeats;
- a source has `clock_synchronized = false` and was seen within the previous 24 hours;
- a source has `incomplete_until_unix_ms` later than `data_through_unix_ms`.

Ignore a non-retired stale source after 24 hours without a heartbeat because none of its possible events can remain in the 24-hour display window.

Display a public data-quality warning. Do not present affected percentages as complete.

### 13.5 Replica Shipment

Add status events as a fourth data class in the existing replica metering pipeline.

Keep request logs, last-used updates, and balance deltas unchanged.

Apply status events idempotently on the Primary by event ID.

Retain spool files until the Primary returns HTTP 200.

Test ambiguous HTTP outcomes, shipment replay, concurrent replicas, Primary restart, and Replica restart.

## 14. Public Status Calculation

Add `GET /api/public/status`.

Return this allow-listed logical response shape:

```text
generated_at: RFC3339 UTC
data_through: RFC3339 UTC
data_complete: boolean
groups: Array<{
  public_name: string,
  state: operational | minor_degradation | major_degradation | unavailable | insufficient_data,
  insufficient_provider_count: nonnegative integer,
  providers: Array<{
    public_name: string,
    state: operational | minor_degradation | major_degradation | unavailable | insufficient_data,
    success_rate_24h_basis_points: integer from 0 through 10000, or null
  }>
}>
```

Do not return attempt counts, internal identifiers, internal names, Channel fields, error classes, upstream statuses, or source-node fields. Order Groups by current Group order. Order Providers by Group-local priority, creation time, and ID.

Aggregate counted events by current Group and Provider configuration. Include only Providers whose Provider and embedded Channel are enabled. Include only Groups that contain at least one such Provider.

Count an event only when its Provider still exists, the Provider and embedded Channel remain enabled, the event `channel_id` equals the Provider's embedded `channel_id`, and the event `group_id` equals the Provider's current `group_id`. Every mutation that increments `configuration_generation` starts a fresh public observation window for that Provider.

Require `provider_generation` to equal the current Provider `configuration_generation`. The generation equality, not timestamp comparison, enforces the fresh observation window.

Use the latest 30 minutes for current state and the latest 24 hours for displayed success rate.

Let `attempts_30m` be the counted attempts whose event timestamp is at least `data_through - 30 minutes` and at most `data_through`. Let `successes_30m` be their successes. Classify current state exactly as follows:

| 30-minute counted attempts | Success rate | State |
| --- | --- | --- |
| Fewer than 10 | Any | `insufficient_data` |
| At least 10 | At least 95% | `operational` |
| At least 10 | At least 80% and below 95% | `minor_degradation` |
| At least 10 | At least 50% and below 80% | `major_degradation` |
| At least 10 | Below 50% | `unavailable` |

Compare the 30-minute thresholds with integer cross-products. Operational means `successes_30m * 100 >= attempts_30m * 95`. Minor means the 95 comparison fails and `successes_30m * 100 >= attempts_30m * 80`. Major means the 80 comparison fails and `successes_30m * 100 >= attempts_30m * 50`. Otherwise the state is unavailable. Do not use floating-point arithmetic.

Let `attempts_24h` and `successes_24h` use the equivalent inclusive window `data_through - 24 hours <= occurred_at_unix_ms <= data_through`. Show no 24-hour percentage when `attempts_24h` is zero. Otherwise calculate `floor(successes_24h * 10000 / attempts_24h)` with integer arithmetic and return that basis-point value. The frontend displays it with exactly two decimal places.

For one Group, select the worst known Provider state in this exact order: `unavailable`, `major_degradation`, `minor_degradation`, then `operational`.

Ignore `insufficient_data` when at least one Provider has a known state. Show the count of Providers with insufficient data separately.

Show Group `insufficient_data` only when every included Provider has insufficient data.

Cache the aggregate response for 15 seconds. Refresh the frontend every 30 seconds through SWR.

## 15. Public API Security

Return only an allow-listed response structure from each public endpoint.

Add `GET /api/public/site`. Return exactly this logical response shape:

```text
site_name: string
site_description: string
api_base_url: string
```

Read only those three setting keys. Return their configured defaults when a row is absent.

Do not return registration, CAPTCHA, authentication, transform, redirect, pricing, suffix-map, or other settings.

Use this endpoint for the public layout, welcome page, Marketplace, API Docs, and status page. Keep the existing login endpoint for registration and CAPTCHA settings.

Fetch public site settings through SWR. Render matching title, description, and Base URL Skeletons during initial loading. None of the public surfaces performs a server mutation.

Never serialize a complete Provider, Channel, Group, rate-record, or settings entity into a public response.

Use reviewed public names. Never fall back to internal names.

Apply an application-level per-IP limit of 60 public API requests per minute with a burst of 20.

Use the existing trusted-proxy client-IP extraction contract. Do not trust an arbitrary forwarded IP header.

Return HTTP 429 with `rate_limited` when the limit is exceeded.

Add `Cache-Control`, ETag, `X-Content-Type-Options`, and existing CSP protections.

Use one process-local token bucket keyed by the canonical trusted-proxy client IP across `/api/public/site`, `/api/public/marketplace`, and `/api/public/status`. Refill continuously at one token per second, cap tokens at 20, and consume one token per request. These rules implement the 60-per-minute rate with burst 20.

Cap the bucket map at 10,000 entries. Define an entry as idle when it has received no request for at least 120 seconds. When a request from a new IP arrives at the cap, evict the least-recently-seen idle entry. When no entry is idle, return HTTP 429 with `rate_limited` and do not insert a bucket for the new IP. Continue to process an existing IP through its existing bucket while the map is at the cap.

Set Site and Marketplace `Cache-Control: public, max-age=15, stale-while-revalidate=30`. Set Status `Cache-Control: public, max-age=15`. Compute a strong ETag from the exact encoded response body of each endpoint. Return HTTP 304 with no body when `If-None-Match` equals the current ETag. Apply rate limiting before conditional-response evaluation.

Add automated negative assertions for every secret or internal field name.

Require review of the public-name manifest before deployment.

## 16. API Documentation

Add the public `/apidocs` page.

Document these API families:

- OpenAI Responses
- OpenAI Chat Completions
- Anthropic Messages
- Gemini Generate Content
- Image generation
- Streaming
- Authentication
- Errors

Provide cURL, Python, JavaScript, and Go examples for every request family.

Use raw HTTP examples. Do not require one SDK.

Read the displayed Base URL from `GET /api/public/site`. When `api_base_url` is empty, derive it from the browser origin plus `/v1` only when the current origin uses `https`; otherwise show a configuration error and disable copy actions.

Document `model_pricing_required`, authentication errors, rate limits, and streaming termination.

Support exactly `en`, `zh`, `zh-TW`, and `ja`.

Use Simplified Technical English in all source prose. Preserve canonical English product nouns in every locale.

Update every affected user-facing documentation page in all four locales. Recapture affected dashboard screenshots in both the English and Simplified Chinese image sets.

## 17. Branding

Change the default `site_name` from `Monoize Dashboard` to `LynShen Console`.

Change the static HTML fallback title from `Monoize Console` to `LynShen Console`.

Use runtime `site_name` on the login page, Console layout, welcome page, Marketplace, API Docs, and status page.

Render every user-visible string on the four new or changed public surfaces through the existing i18next catalog. Add complete keys to `en`, `zh`, `zh-TW`, and `ja`. Use the browser-language and saved-language behavior already defined by the frontend. Do not add locale path prefixes.

Migrate a stored name only when it exactly equals an old built-in default.

Preserve every administrator-defined custom name.

## 18. Management UI

Use one Provider editor with these sections:

1. Identity and Group.
2. Public names.
3. Default Billing Profile and multiplier.
4. Single Channel configuration.
5. Model mappings and overrides.
6. Validation summary.

Use a single-select Group control.

Do not render an Add Channel action.

When create or update requires public-exposure confirmation, show an unchecked confirmation control. Explain that the entered Group, Provider, and Channel public names become public.

Send `confirm_public_exposure: true` only after the administrator checks that control.

Show inherited values and effective values for each model.

Show backend-produced price previews. Do not calculate prices in the browser.

Allow save with pricing warnings. Block save only for structural or validation errors.

Use SWR optimistic updates and rollback failed mutations.

Render a matching Skeleton during initial loading and hydration.

## 19. Breaking Change Management

Treat the Provider management contract as a breaking API change.

Update all repository callers in the same release:

- React frontend
- SDK smoke tests
- Backend integration tests
- Documentation
- Specifications

Publish a release note that lists removed fields and the singular replacements.

Do not provide a compatibility period or compatibility alias.

The retained production log sample contains browser callers only. This evidence does not cover deleted logs or unknown external clients.

Require the owner to confirm that no external Provider-management automation depends on the old request shape.

Use a maintenance window even when that confirmation is provided.

## 20. Specification Changes After Approval

Update these existing specification files before implementation:

- `spec/channel-management.spec.md`
- `spec/database-provider-routing.spec.md`
- `spec/monoize-upstream-routing.spec.md`
- `spec/groups-registry.spec.md`
- `spec/model-marketplace.spec.md`
- `spec/model-metadata-dashboard.spec.md`
- `spec/dashboard-ui-layout.spec.md`
- `spec/dashboard-session-authentication.spec.md`
- `spec/security-access-control.spec.md`
- `spec/admin-dashboard.spec.md`
- `spec/database-configuration.spec.md`
- `spec/system-settings-ui.spec.md`
- `spec/metered-billing.spec.md`
- `spec/user-billing-and-model-metadata.spec.md`
- `spec/model-registry-storage.spec.md`
- `spec/playground.spec.md`
- `spec/request-logs.spec.md`
- `spec/primary-replica-deployment.spec.md`
- `spec/initial-seaorm-migration.spec.md`
- `spec/deployment-watchdog.spec.md`
- `spec/docs-site.spec.md`
- `spec/frontend-design-system.spec.md`

Create these specification files directly under `spec/`:

- `spec/public-site.spec.md`
- `spec/provider-pricing.spec.md`
- `spec/public-provider-status.spec.md`

Use low-entropy, testable English. Do not create a subdirectory under `spec/`.

## 21. Test Strategy

### 21.1 Migration

Test SQLite and PostgreSQL.

Cover zero, one, and multiple Groups. Cover zero, one, disabled, zero-weight, and multiple enabled Channels.

Cover deterministic IDs, ID collisions, name collisions, malformed JSON, missing foreign rows, and every injected transaction failure.

Run the migration twice. Verify the second run is a no-op.

Verify obsolete columns and indexes are absent.

### 21.2 Pricing

Run the pricing golden snapshot before and after migration.

Cover Profile ties, multiplier ties, redirects, provider types, cache rates, meter rates, missing rates, and maximum integer values.

Require exact equality for integer charges and canonical decimal display rates. Do not use tolerance-based assertions.

### 21.3 Status Events

Cover every error classifier branch and every physical retry path.

Inject spool quota exhaustion, filesystem failure, database failure, ambiguous commit, replay, concurrent replica shipment, and process restart.

Verify request results remain unchanged by status persistence failure.

Verify `data_complete` becomes false when an event is lost.

### 21.4 Public Security

Assert that public responses omit all internal and secret fields.

Test the exact `/api/public/site` allow-list. Test rate limiting, bucket-cap exhaustion, idle eviction, cache headers, ETag behavior, malformed query input, and large datasets.

Test that every new or changed public name requires `confirm_public_exposure: true`. Test that an unchanged normalized name does not require it.

Test modal focus trapping, Escape close, focus restoration, keyboard navigation, mobile layout, and reduced motion.

### 21.5 Commands

Require these commands before an implementation can be declared complete:

```text
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets

cd frontend
bun install
bun run lint
bun run build
bun run test

cd docs
bun install
bun run build

docker build .
git diff --check
```

## 22. Implementation Release Gates

Do not begin product implementation until all Gate A evidence exists.

### Gate A: Written Design

- The user approves this document.
- The user confirms intentional public display of reviewed Group, Provider, and Channel public names.
- The user confirms that the old Provider management contract has no required external caller.

### Gate B: Migration Rehearsal

- A redacted production copy passes migration on SQLite.
- Synthetic equivalent fixtures pass migration on PostgreSQL.
- The projected row expansion is reviewed.
- Every semantic-change Provider has explicit written approval.
- Transaction rollback tests pass.

### Gate C: Pricing Equality

- The frozen pre-migration and post-migration pricing snapshots are byte-equivalent after canonical serialization.
- Every scenario charge is exactly equal.
- No unresolved difference is waived.

### Gate D: Status Reliability

- Retry, replay, concurrency, and fault-injection tests pass.
- Lost events mark status data incomplete.
- Status persistence failures do not change API billing or response results.

### Gate E: Public Security

- Public response allow-list tests pass.
- Secret-field negative tests pass.
- Rate-limit tests pass.
- The public-name manifest receives explicit approval.

Passing Gates A-E permits an implementation-complete decision. It does not permit deployment.

## 23. Deployment Gate and Rollback

Use a cold maintenance-window cutover because the migration removes old columns.

Treat the candidate as Primary-only until migration finishes and route verification passes. Stop every Replica before write drain. A Replica must not point at the migrated database until it runs the same candidate image and its read-only schema verification accepts the new schema.

Before cutover:

1. Build and verify the candidate image.
2. Stop every Replica, stop new writes on the Primary, and record that no old binary remains connected.
3. Drain in-flight requests and background batches.
4. Run the final preflight. Require its database fingerprint to equal the approved manifest fingerprint.
5. Create a current SQLite Online Backup snapshot, including all committed WAL content.
6. Hash the snapshot and run `PRAGMA quick_check`.
7. Restore the snapshot into an isolated directory.
8. Start the old image against the restored copy without external network access.
9. Verify health and expected row counts.
10. Run the new image and migration against a second restored copy.
11. Verify migration, pricing snapshot equality, and public contract tests.

Do not rely on the existing zero-Provider backup.

Require explicit deployment approval after the restore rehearsal succeeds.

During cutover, stop the old container before the new container touches the production database.

Keep customer traffic and every real write frozen from the final preflight through release acceptance or rollback completion. During this interval, admit only the public read checks and controlled synthetic forwarding requests defined by the release checklist. Use credentials and request data dedicated to verification. Treat every billing, request-log, and status-event write from those synthetic requests as disposable if rollback occurs.

Do not use the existing `deploy-watchdog` mode for this release. That watchdog restores only the old binary and does not restore a database. Starting the old image against this migrated schema is forbidden.

Keep an operator present for at least 300 seconds after startup. Do not restore customer traffic during this observation period. The operator must run the route and forwarding verification below, then either accept the release and restore traffic or execute the database-aware rollback steps. Do not add an automatic binary-only restart fallback.

Verify `/`, `/login`, `/apidocs`, `/status`, `/dashboard/marketplace`, `/dashboard`, and one controlled forwarding request.

Rollback requires these steps:

1. Stop the new container.
2. Preserve the failed migrated database for diagnosis.
3. Restore the verified current backup.
4. Start image `monoize:cf36bd8`.
5. Verify health, login, Provider count, model count, and one controlled forwarding request.

Do not start the old image against the migrated schema.

Permit this backup restore only before release acceptance, while real writes remain frozen. After customer traffic resumes, do not restore the pre-cutover snapshot in place. A later rollback requires a separately reviewed forward-compatible migration or an explicit data-reconciliation and data-loss plan.

## 24. Current Decision

The architecture is feasible.

The current production data shape is route-safe and does not require Cartesian expansion.

The existing backup is not a valid current rollback point.

Migration equality, pricing equality, status-event failure behavior, public-name approval, and restore rehearsal remain mandatory release gates.

Proceed only with written specification work after this design receives user approval.
