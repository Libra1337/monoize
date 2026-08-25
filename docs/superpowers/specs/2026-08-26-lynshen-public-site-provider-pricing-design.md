# LynShen Public Site, Provider, Pricing, and Status Design

Date: 2026-08-26

Status: Revised after read-only preflight and risk review. This document authorizes no product implementation and no production mutation.

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

Do not change behavioral files under `spec/` until this revised document receives user approval.

Do not implement migrations, backend behavior, frontend behavior, or documentation pages under this authorization.

Do not deploy or mutate production under this authorization.

Approval of this revised document authorizes behavior-specification updates and migration-rehearsal planning only.

After those specifications and plans receive user review, require explicit authorization to create the executable Phase 1 rehearsal artifacts. Phase 1 authorization still does not authorize product integration, production access, or deployment.

Require a separate user approval before product implementation. Require another separate approval before production deployment.

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

Produce behavioral specification updates, preflight tooling design, migration-rehearsal plans, and release gates after Gate A.

Do not implement executable migration artifacts or product behavior in this phase.

### Phase 1: Migration Rehearsal Artifacts

Create executable migration prototypes, fixtures, pricing snapshot tooling, an isolated Marketplace query/cursor/encoding benchmark, and status-event fault-injection harnesses for isolated test databases.

Do not connect these artifacts to production startup or production routes. The Marketplace benchmark must run as a test executable that registers no HTTP listener. Do not expose new public pages in this phase.

Do not merge or deploy the destructive migration prototype. Require separate product-implementation approval after Gates B-E produce evidence.

Treat Phase 1 as an independent release milestone. Its estimate must separately include fixture generation, both database runs, Marketplace read and source-write benchmarks, pricing snapshots, five-node status load, fault injection, recovery measurement, result review, and reruns after a failed gate. Do not include Phase 1 effort inside an ordinary UI feature estimate. Approving this design or Phase 0 does not allocate hardware or authorize Phase 1 execution.

### Phase 2: Product Implementation

After separate product-implementation approval, implement schema migration, runtime contracts, public surfaces, documentation, and management UI as one release candidate.

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
- `public_name_key BLOB/BYTEA NOT NULL`
- `configuration_generation BIGINT NOT NULL`

Keep every existing Provider field that section 7.4 does not remove. Keep its current validation and runtime semantics unless this document states a replacement.

Validate `multiplier` with the existing exact decimal type. Require a value greater than zero and at most nine fractional digits.

Treat a null `pricing_profile` as no Provider default. Require a model override before that model can become priced.

For every non-null Provider default or model override Profile input, trim surrounding Unicode White_Space code points first. Reject an empty trimmed value.

Compare the trimmed value case-sensitively with a Profile present in at least one billing-rate record. Persist and return only the trimmed value. Do not case-fold it.

Allow that Profile to lack rates for a specific model. Report that mapping as unpriced.

Use `public_name` on public responses. Do not expose the internal Provider `name`.

For each Group, Provider, and Channel public name, trim surrounding Unicode whitespace and normalize the result to Unicode NFC. Require 1-64 Unicode scalar values. Reject C0 controls, DEL, CR, LF, and tab. Persist and return the normalized value.

Derive a public-name key as the exact UTF-8 bytes of the normalized public name. Store the key as `BLOB` on SQLite and `BYTEA` on PostgreSQL. Require SQLite database encoding `UTF-8`. Add a database CHECK that `public_name_key = CAST(public_name AS BLOB)` on SQLite and `public_name_key = convert_to(public_name, 'UTF8')` on PostgreSQL. Apply the same rule to `monoize_groups.public_name_key` and the embedded `channel_public_name_key`.

Compare, constrain, and order public names through their binary keys. Do not rely on a text collation. Return the normalized text, not the binary key, through an API.

Derive every binary key on the server. Do not accept `public_name_key`, `channel_public_name_key`, `model_name_key`, or `model_search_key` in a management request. Map a database public-name unique-index violation to HTTP 409 with `public_name_conflict`. Return only `entity_type` (`group`, `provider`, or `channel`) and the conflicting normalized public name. Do not return an internal name, ID, SQL constraint name, or database error.

After migration, require a management create request that assigns a Group, Provider, or Channel `public_name` to include `confirm_public_exposure: true`.

Require the same field when an update changes a normalized public name. For a Provider request, one top-level confirmation covers the Provider and embedded Channel names.

Treat the field as request-only. Do not persist or return it. Reject a missing or non-true confirmation with HTTP 400 and `public_exposure_confirmation_required`.

The reviewed migration manifest supplies the equivalent confirmation for migrated names.

Initialize `configuration_generation` to one during migration. Increment it exactly once in the same transaction as each logical mutation to the Provider row, embedded Channel, or Provider model mappings. A failed transaction does not increment it.

Treat migrated Provider and Channel IDs as opaque legacy IDs. Generate lowercase UUID v4 IDs for every Provider and embedded Channel created after migration. Do not accept either ID in a create body. Do not allow either ID to change in an update body.

### 7.2 Channel

Keep Channel as a first-class API, runtime, status, and UI value object. Do not keep it as a separate database entity.

Move every surviving Channel column into `monoize_providers` with a `channel_` prefix. This includes Channel ID, internal name, public name, API type, Base URL, API key, enabled state, retry and breaker overrides, probe overrides, affinity overrides, proxy URL, extra headers, session affinity, and missing-usage policy.

Add `channel_public_name_key BLOB/BYTEA NOT NULL` beside `channel_public_name`. Apply the public-name derivation and database CHECK from section 7.1.

Make every required embedded Channel column `NOT NULL`. Make `channel_id` unique.

A Provider row cannot exist without its embedded Channel fields. One Provider row contains exactly one Channel. This enforces the invariant identically on SQLite and PostgreSQL.

Create and update the Provider and embedded Channel through one row mutation. Return one singular Channel value in the API.

Replace `monoize_channel_models` with `monoize_provider_models`. Use `(provider_id, model_name)` as its unique logical key. Cascade model deletion from `monoize_providers.id`.

Use this `monoize_provider_models` shape:

- `provider_id TEXT NOT NULL`
- `model_name TEXT NOT NULL`
- `model_name_key BLOB/BYTEA NOT NULL`
- `model_search_key BLOB/BYTEA NOT NULL`
- `redirect TEXT NULL`
- `pricing_profile_mode TEXT NOT NULL`
- `pricing_profile_override TEXT NULL`
- `multiplier_override TEXT NULL`
- `created_at TEXT NOT NULL`

Do not retain a separate model-mapping row ID. Use the logical model name in validation warnings and management responses.

For every new logical model name, trim surrounding Unicode White_Space code points. Require 1-256 UTF-8 bytes after trimming. Reject C0 controls, DEL, CR, LF, and tab. Persist and compare the trimmed value case-sensitively. During migration preflight, block a stored logical model name that differs from its trimmed value or violates these byte and control constraints. Do not silently rename it.

Set `model_name_key` to the exact UTF-8 bytes of `model_name`. Set `model_search_key` to those bytes after adding 32 to each byte from ASCII `A` through `Z`; leave every other byte unchanged. Management writes and migrations must derive both keys. Add the same SQLite and PostgreSQL UTF-8 equality CHECK for `model_name_key` that public names use. Add a CHECK for `model_search_key` that applies all 26 exact uppercase-to-lowercase ASCII replacements to `model_name` and then encodes the result as BLOB/BYTEA. The replacement expression must not invoke a locale-sensitive lowercase function.

Use `(provider_id, model_name_key)` as the database primary key. Keep `(provider_id, model_name)` as the public logical description of that key. Index `monoize_providers` by `(group_id, priority, created_at, id)`. Index `monoize_provider_models` by `(model_name_key, provider_id)`. Use these indexes for Group routing, Marketplace ordering, and offer lookup. The arbitrary substring predicate may scan `model_search_key`; do not claim that a B-tree index accelerates it.

Remove Channel `weight`. Convert `weight <= 0` to `enabled = false` during migration.

### 7.3 Group

Add `public_name TEXT NOT NULL` and `public_name_key BLOB/BYTEA NOT NULL` to `monoize_groups`.

Use the public Group name on public responses. Do not expose an unapproved internal label.

Require public Group names to be unique after trimming and Unicode NFC normalization. Compare the normalized values case-sensitively through `public_name_key`.

Create these database unique indexes:

- `monoize_groups(public_name_key)` for global Group public-name uniqueness;
- `monoize_providers(group_id, public_name_key)` for Provider public-name uniqueness within one Group;
- `monoize_providers(group_id, channel_public_name_key)` for Channel public-name uniqueness within one Group.

Use BLOB/BYTEA byte order for these indexes on both databases. The unique indexes are the final concurrency guard. Application prechecks may produce a clearer error but do not replace an index. Validate the final names for all rows affected by one management transaction before writing. When one statement reorders or renames values that temporarily collide, use a two-step update with transaction-local non-conflicting keys, then write the final keys before commit. Do not use deferrable uniqueness because SQLite does not provide equivalent behavior.

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

Require public names to pass the normalization, length, control-character, and binary-key rules in section 7. Emit the normalized text and hexadecimal binary key in the manifest.

Reject a global Group public-name-key collision. Reject a Provider or Channel public-name-key collision within one Group. Report every conflicting source row. Do not resolve a collision by appending an automatic suffix.

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

The public Marketplace endpoints return only public Group, Provider, Channel, model, capability, and effective price fields defined in this section. Encode every displayed rate as a canonical non-negative base-10 decimal JSON string in nano-USD per source unit, with at most nine fractional digits and no exponent notation. The frontend parses it as a decimal string and never as a JavaScript number.

Accept these query parameters on `GET /api/public/marketplace`:

- `q`: optional substring. Canonicalize and validate it by the rules below.
- `group`: optional public Group name. Canonicalize it by the rule below.
- `cursor`: optional opaque cursor from the preceding response.
- `limit`: optional integer from 1 through 50. Default to 24.

Normalize a supplied `group` with the public-name rules in section 7. Derive its binary key and perform exact Group lookup through `public_name_key`. Do not compare the text through a database collation.

Return HTTP 400 with `invalid_request` for an invalid Group, cursor, limit, or oversized search. Return this allow-listed logical response shape:

```text
generated_at: RFC3339 UTC
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

Order rows by the zero-based current Group ordinal and then `model_name_key`. Use the same exact UTF-8 byte ordering in SQLite, PostgreSQL, cursor encoding, and response assembly. Do not delegate model ordering to a locale-dependent database collation.

Render one visible Group section heading before the first row of each Group. Repeat the heading when a fetched page starts inside that Group. Never place rows from two Groups under one heading.

Define canonical `q` as the input after Unicode White_Space trimming. Do not apply Unicode normalization. Reject C0 controls, DEL, CR, LF, tab, or a canonical value longer than 128 UTF-8 bytes. Treat an empty canonical value as absent. Otherwise, encode it as UTF-8 and add 32 to each byte from ASCII `A` through `Z`; leave every other byte unchanged. Bind that value as `BLOB` on SQLite and `BYTEA` on PostgreSQL.

Filter mappings in the database with binary substring containment against `model_search_key`. Use `instr(model_search_key, ?1) > 0` on SQLite and the equivalent `position($1::bytea in model_search_key) > 0` predicate on PostgreSQL. Do not load the complete model catalog into application memory. Do not delegate matching to a text collation. Arbitrary substring search may scan the bounded mapping set; the qualification limits and latency gate below apply to that scan.

Generate a 32-byte random Marketplace cursor HMAC key during migration. Store its base64url encoding in a dedicated `state_records` row inside the migration transaction. Do not return it through an API or place it in a log or reviewed manifest. Load it once at startup. Refuse readiness with `marketplace_cursor_key_unavailable` when the row is absent, malformed, or does not decode to exactly 32 bytes.

Encode a list `cursor` as `<payload>.<signature>`. Encode both parts with base64url without padding. Compute the signature as HMAC-SHA-256 over the exact binary payload. Compare signatures in constant time before parsing cursor fields.

Use this common binary payload prefix:

```text
version: u8, value 1
endpoint_kind: u8, list = 1, offers = 2
revision: u64 big-endian
limit: u16 big-endian
filter_digest: 32 SHA-256 bytes
```

Build `filter_digest` from the endpoint kind and each canonical filter. Encode each field as one tag byte, a u32 big-endian byte length, and exact UTF-8 bytes. Include canonical `q` and normalized Group for a list cursor. Include normalized Group and exact model for an offer cursor.

Append this list keyset after the common prefix:

```text
group_ordinal: u64 big-endian
model_length: u16 big-endian
model: exact UTF-8 bytes
```

The first request has no keyset. A response cursor contains the sort key of its last returned row. The next query selects rows strictly after that key.

Limit the complete list cursor to 512 ASCII bytes. Reject malformed input, an invalid signature, a cursor whose endpoint kind, filter digest, or limit differs from the request, or a cursor whose revision differs from the current Marketplace revision. Return HTTP 409 with `marketplace_cursor_stale` only for a validly signed revision mismatch. Return HTTP 400 with `invalid_request` for every other cursor error. Do not place an internal database ID or internal name in a cursor.

Use the zero-based position in the complete canonical Group order as `group_ordinal`. Reject cursor construction if that position cannot fit `u64`.

Add `GET /api/public/marketplace/offers`. Require `group` to resolve to one normalized public Group name and `model` to equal the exact logical model ID from a list row. Accept `cursor` and `limit` with the same validation rules. Use a default limit of 20 and a maximum of 50.

Normalize and resolve the offer `group` through `public_name_key` by the list rule. For `model`, compute the Unicode White_Space-trimmed value only for validation. Return HTTP 400 with `invalid_request` when the trimmed value differs from the supplied value, when the value is empty, exceeds 256 UTF-8 bytes, or contains C0 controls, DEL, CR, LF, or tab. Do not Unicode-normalize or otherwise rewrite an accepted value. Derive `model_name_key` from the exact accepted model bytes for lookup. Return HTTP 404 with `marketplace_model_not_found` only after both inputs pass validation and no visible exact model row exists.

Return this allow-listed offer response:

```text
generated_at: RFC3339 UTC
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

Append this offer keyset after the common prefix:

```text
provider_priority: i32 big-endian two's-complement
provider_name_length: u16 big-endian
public_provider_name: exact UTF-8 bytes
channel_name_length: u16 big-endian
public_channel_name: exact UTF-8 bytes
```

Compare Provider priority numerically before comparing `public_name_key` and `channel_public_name_key`. Compare both keys by exact UTF-8 byte order. Use the same ordering in SQLite, PostgreSQL, cursor encoding, and response assembly. Do not delegate name ordering to a locale-dependent database collation.

Limit the complete offer cursor to 1024 ASCII bytes. Apply the same signature, revision, filter, limit, and error rules. The next query selects offers strictly after that key. Return HTTP 404 with `marketplace_model_not_found` when no visible row matches the Group and model. Order offers by Provider priority, then public Provider and Channel name.

Create a dedicated `marketplace_generation` table. Do not store this record as JSON in `state_records`. The table contains exactly one row with these fields:

- `singleton_id SMALLINT PRIMARY KEY`, fixed to `1` by a CHECK;
- `revision BIGINT NOT NULL`, from `1` through `9223372036854775807` by a CHECK;
- `generated_at_unix_us BIGINT NOT NULL`, from `0` through `253402300799999999` by a CHECK.

Create the row with revision one and the current database UTC time during migration. Create all source rows first. Create the generation row and its triggers after source backfill, so migration backfill does not consume revisions. Prevent deletion of the singleton row with a database trigger. On PostgreSQL, add a statement-level trigger that rejects `TRUNCATE marketplace_generation`. Product code and maintenance tooling must not truncate this table on SQLite.

Add a database trigger on `marketplace_generation` updates. Permit an update only when `singleton_id` remains one, `revision = OLD.revision + 1`, and `generated_at_unix_us > OLD.generated_at_unix_us`. Reject every rollback, repeated value, skipped revision, primary-key change, and timestamp that does not increase. An exact direct increment that satisfies these rules may cause a harmless extra cache invalidation but cannot reuse an earlier cursor generation.

Install database triggers for `INSERT`, relevant-column `UPDATE`, and `DELETE` on each of these tables:

- `monoize_groups`;
- `monoize_providers`;
- `monoize_provider_models`;
- `billing_rate_records`;
- `model_metadata_records`;
- `system_settings`.

Treat the first five tables as full Marketplace source tables. For each full source table, define the exact column allow-list that can affect public Marketplace serialization, ordering, visibility, capability, or effective pricing. Generate its `UPDATE OF` trigger column list from that manifest. The update trigger must fire when any allow-listed column appears in the update target list, even when the new value equals the old value. Exclude audit-only columns that cannot change a public response.

Treat `system_settings` as a filtered Marketplace source table. Its only included logical row has `key = 'reasoning_suffix_map'`. Include `key` and `value`; exclude `updated_at`. A change that inserts or deletes this row, changes its key into or out of this value, or changes its value can change pricing-model normalization and therefore public visibility or price. Marketplace snapshot construction must read this persisted row inside the same generation-checked database read as the other Marketplace sources. Decode a missing row with `default_reasoning_suffix_map()`, exactly as the billing settings loader does. Return HTTP 503 with `marketplace_source_invalid` when the row exists but its JSON does not decode to the required map. Do not fall back from invalid persisted JSON, and do not derive Marketplace output from a separately refreshed runtime settings snapshot.

Record every included and excluded source column with a reason in the generation-source manifest. Fail artifact generation when a source column lacks one classification.

On PostgreSQL, use one statement-level trigger per operation and source table. For a full source table, a plain `INSERT`, relevant-column `UPDATE`, or `DELETE` statement advances the generation once, including a statement that affects zero rows. An `INSERT ... ON CONFLICT DO UPDATE` statement advances once for its statement-level `INSERT` event and once for its statement-level relevant-column `UPDATE` event. An `INSERT ... ON CONFLICT DO NOTHING` statement advances once for its `INSERT` event. No PostgreSQL full-source statement advances once per affected row.

For PostgreSQL `system_settings`, use transition tables. Compare the filtered old and new projections whose key equals `reasoning_suffix_map`. Advance once when an `INSERT`, `UPDATE`, or `DELETE` statement makes those projections byte-different in `key` or `value`. Do not advance when a statement changes only `updated_at`, writes byte-identical `key` and `value`, or touches only another setting. An upsert advances according to each statement-level operation that PostgreSQL emits only when that operation changes the filtered projection. Do not use an `UPDATE OF` column list on this transition-table trigger.

Create one PostgreSQL statement-level `TRUNCATE` trigger for each source table. A truncate of `system_settings` advances because it can remove `reasoning_suffix_map`, including when the table is empty. PostgreSQL therefore has 18 source-operation triggers plus six source-table `TRUNCATE` triggers.

On SQLite, use one row-level trigger per operation and source table because SQLite has no statement-level trigger. One affected full-source row advances the generation exactly once. A full-source statement that affects zero rows does not advance it. Apply `WHEN` predicates to the three `system_settings` triggers: insert only when the new key matches, delete only when the old key matches, and update only when an old or new key matches and `key` or `value` changes byte-for-byte. SQLite therefore has 18 source-operation triggers. Product code and maintenance tooling must not disable these triggers or emulate `TRUNCATE` by bypassing them.

Each trigger invocation must atomically increment `revision` and set `generated_at_unix_us` inside the source transaction. Let `clock_us` be the database UTC clock in integer Unix microseconds. Set the new timestamp to `max(clock_us, previous_generated_at_unix_us + 1)`. SQLite derives `clock_us` from integer Unix seconds and the three millisecond digits returned by `strftime('%f', 'now')`. PostgreSQL derives it from `clock_timestamp()`. The stored value is therefore strictly increasing even when several invocations share one clock tick. One transaction may advance the revision several times. Only strict generation change is an invariant; consecutive committed generations need not differ by one.

The table primary key and singleton CHECK make a duplicate singleton impossible. The trigger must abort the complete source transaction when the singleton row is missing, outside its allowed revision or timestamp range, or cannot advance either value without overflow. Schema checks define malformed stored values as out of range. The update guard must also abort the complete source transaction when its exact increment or timestamp condition fails. A failed or rolled-back source transaction changes neither the source state nor the committed generation.

Keep a machine-readable generation-source manifest in the migration rehearsal artifacts. List the six source tables, the filtered `system_settings` row rule, every included and excluded source column with a reason, every trigger name, every management writer, every background or catalog-sync writer, every seed or maintenance writer, and every direct-SQL test writer. Compare the manifest with database metadata during rehearsal and deployment preflight. Fail the check for a missing or disabled trigger, an unlisted source table, an unclassified source column, an unlisted filtered-row rule, or an unlisted writer.

The initial audited writer inventory is:

| Source table | Current repository writer locations |
| --- | --- |
| `monoize_groups` | `src/users/groups.rs`; Group migration and direct-SQL fixtures |
| `monoize_providers` | `src/monoize_routing.rs`; Provider migrations and direct-SQL fixtures |
| `monoize_provider_models` | target Provider/model store derived from `src/monoize_routing.rs`; migration prototype and direct-SQL fixtures |
| `billing_rate_records` | `src/billing_rate_store.rs`; `src/model_registry_store.rs`; billing migrations and direct-SQL fixtures |
| `model_metadata_records` | `src/model_registry_store.rs`; metadata migrations and direct-SQL fixtures |
| `system_settings` filtered to `reasoning_suffix_map` | `src/settings.rs`; `src/dashboard_handlers/settings.rs`; settings migrations and direct-SQL fixtures |

Regenerate this inventory with repository search in Gate B. Do not treat the listed paths as permanently exhaustive. Trigger coverage is table-wide and therefore also covers a newly added writer, but Gate B fails until the manifest names that writer.

Before Marketplace cache lookup, read only the current generation record. For each endpoint and each canonical query parameter set, build an immutable encoded response snapshot on cache miss.

Read the generation record again after reading all source rows and before encoding. Retry the complete read when either generation value changed. Make at most three build attempts. Return HTTP 503 with `marketplace_snapshot_busy` when all three attempts observe a concurrent generation change.

Copy `revision` from the stable generation record. Convert its `generated_at_unix_us` to RFC3339 UTC with exactly six fractional-second digits and a `Z` suffix. Use that exact string as response `generated_at`. Reuse the exact uncompressed JSON bytes and ETag while the record remains unchanged. Do not recompute `generated_at` on cache expiry, eviction, or a conditional request. The revision, not timestamp uniqueness, identifies a Marketplace generation.

Use a process-local LRU cache with at most 256 Marketplace response snapshots. Expire an entry 60 seconds after its last access.

Limit each uncompressed encoded JSON response body to 1048576 bytes. Return no more than the requested limit. Evaluate each candidate body with its final envelope and `next_cursor` value. Stop before the first item that would exceed the body limit. Set `next_cursor` from the last included sort key so that the excluded item is first on the next page. If no item fits, return HTTP 500 with `public_response_too_large`, do not cache the body, and emit an error metric without public internal details.

On a list cache miss, select at most `limit + 1` distinct Group and logical-model rows strictly after the cursor keyset. Apply `q` before the keyset limit. Load visible offers for all selected rows in one set-based Provider and model-mapping query. Load applicable billing-rate and model-metadata rows in set-based batches. Do not issue one database query per Group, model, Provider, offer, or rate.

On an offer cache miss, select at most `limit + 1` visible offers strictly after the cursor keyset. Load their applicable billing-rate and model-metadata rows in set-based batches. Query count may increase only with the existing bounded SQLite bind-parameter chunking. It must not increase once per returned row.

Benchmark Marketplace source-write invalidation separately from public reads. Use the same maximum-envelope host. Restore a fresh operation-specific fixture before each scenario. Keep every unrelated source table at its envelope maximum. For insert scenarios, seed the tested table at its envelope maximum minus 100,000 rows and reserve 100,000 absent IDs. For update, delete, and conflict scenarios, seed the tested table at its envelope maximum and reserve 100,000 present disposable IDs. No scenario may exceed the envelope.

For `billing_rate_records`, `model_metadata_records`, and `monoize_provider_models`, run one transaction that mutates 100,000 rows with statement sizes of 1, 100, 1,000, and 10,000 rows. Test insert, relevant-column update, delete, `INSERT ... ON CONFLICT DO UPDATE`, and `INSERT ... ON CONFLICT DO NOTHING` separately. For the concurrency scenario, each of eight writers repeatedly commits one relevant-column update statement over 1,000 disjoint rows per transaction for ten minutes. Record transaction p50, p95, and p99 latency, committed rows per second, generation increments, lock-wait time, SQLite busy retries, PostgreSQL deadlocks, WAL bytes, peak WAL growth, and checkpoint time.

Also replay each actual full-catalog synchronization transaction found in the generation-source writer manifest. Preserve its delete, batch, insert, and upsert statement order and its production batch-size rules. Run it once at the maximum catalog envelope and once while eight concurrent disjoint management updates execute. Apply the same measurements and bounds.

Require zero deadlocks and zero exhausted SQLite busy retries. Require every single-writer 100,000-row transaction and every maximum-envelope full-catalog synchronization transaction to commit within 60 seconds. Require the eight-writer workload to sustain at least 5,000 committed source rows per second with transaction p99 at most 5 seconds. Require peak WAL growth to stay below 2 GiB and the post-run checkpoint to finish within 30 seconds. PostgreSQL generation deltas must equal the operation-event count defined above for every statement. SQLite generation deltas must equal affected source-row trigger invocations. Both revisions must remain below the ceiling and both databases must meet every latency, throughput, lock, and WAL bound.

Gate B fails when either database misses one source-write bound. Do not waive a failure. A failed result requires a revised generation design that coalesces invalidation without permitting a stale Marketplace generation. Update this document and receive explicit design approval before creating a replacement Phase 1 artifact.

Qualify one release against this maximum catalog envelope:

- 128 Groups;
- 5,000 Providers and embedded Channels;
- 250,000 Provider model mappings;
- 100,000 distinct Group and logical-model Marketplace rows;
- 100,000 model-metadata rows;
- 500,000 billing-rate rows.

These values define the supported qualification envelope. They are not hidden row-deletion rules. Emit an operator warning at 80 percent of any value. Fail migration and deployment preflight when an input exceeds a value. Require a revised benchmark and explicit approval before raising an envelope value.

Run the Marketplace benchmark as an optimized release build on an otherwise idle Linux x86-64 host constrained by the benchmark runner to exactly four logical CPUs and 8 GiB RAM. Use a local SSD whose measured sequential read throughput is at least 400 MiB/s and whose measured random 4-KiB read throughput at queue depth one is at least 10,000 IOPS. Record the storage measurements. Run SQLite in WAL mode and PostgreSQL 16 in separate runs. Use the maximum envelope, include one model mapped to all 5,000 Providers, and use search terms that match zero rows, one row, 50 rows, and at least half of the mapping rows.

In Phase 1, invoke the proposed database query, cursor, aggregation, size-limit, encoding, and cache code through the isolated benchmark executable. Do not register an HTTP listener or link the prototype into application startup. After product-implementation approval, run the same fixed data and query sets through the built public endpoints. Both runs must meet the same bounds before deployment approval.

After five warm-up minutes, run 32 concurrent workers for at least ten minutes. Continue until the run contains at least 10,000 verified cache-miss samples for each operation kind. Send 80 percent list operations, distributed equally across the four search selectivities and first, middle, and final cursor positions. Send 20 percent offer operations, distributed equally across first, middle, and final cursor positions for the 5,000-offer model. Use more than 256 canonical query sets and cache instrumentation. Include only verified cache misses in operation latency samples; report cache hits separately.

Use a fixed synthetic-data seed, fixed canonical query-set file, fixed benchmark tool version, and fixed application commit. Record the host CPU model, storage model, operating-system image, SQLite version, PostgreSQL version, database configuration, and application environment with each result.

Before timing, run `ANALYZE` on both databases and checkpoint them. Do not run `VACUUM` during a measurement. Use a connection-pool maximum of 32. Give PostgreSQL an 8-GiB memory limit separate from the application process and include its database-process memory in a separate recorded metric. Give SQLite and the application one combined 8-GiB limit. Do not reuse a database instance between the SQLite and PostgreSQL runs.

Require the list operation p95 to be at most 500 milliseconds and p99 at most 1,000 milliseconds. Require the offers operation p95 to be at most 400 milliseconds and p99 at most 800 milliseconds. For Phase 2 and the final release candidate, an operation is one complete HTTP request. Require the process resident-memory increase from the post-start idle baseline to stay at or below 512 MiB. Record database statement counts, rows scanned, response sizes, CPU time, cache disposition, and peak resident memory for both databases. Gate B fails when either database misses one bound.

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

Fetch the first offer page when the modal opens. Use a separate SWR key for each Group, model, revision, and offer cursor. Render matching offer Skeletons while the first page loads. Fetch another page only after the user activates a localized Load more action. Keep the modal open while appending a successful page. Show an inline retry action after a failed page.

When either endpoint returns `marketplace_cursor_stale`, clear all Marketplace page keys, close an open modal, fetch the first list page, and show a localized catalog-updated notice. Do not combine rows from different revisions.

Use backend-produced exact decimal display values. Format only those decimal strings in the frontend. The browser must not multiply a base rate by a multiplier.

Use SWR. Render matching Skeleton layouts while loading. Keep at most three list pages in browser state. Discard pages before that window when the user continues forward.

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

Use `MONOIZE_STATUS_EVENT_SPOOL_DIR`, default `./data/status-event-spool`. Use `MONOIZE_STATUS_EVENT_SPOOL_MAX_BYTES`, default `536870912`, as the process-local allocated-byte quota. Use `MONOIZE_STATUS_EVENT_SPOOL_ENTRY_MAX_BYTES`, default `4096`, as the maximum logical JSON file length. Use `MONOIZE_STATUS_EVENT_MAX_OUTAGE_SECONDS`, default `900`. Reject a value below `900`. Use `MONOIZE_STATUS_EVENT_SPOOL_SAFETY_FACTOR_MILLI`, default `1200`, to represent a factor of `1.200`. Reject a value below `1200`.

Use `MONOIZE_STATUS_EVENT_MAX_IN_FLIGHT_DISPATCHES`, default `1024`, as a positive process-wide upper bound. Create one shared fair semaphore with exactly this many permits. Every forwarding path must acquire one permit before it reserves spool capacity and begins a physical upstream HTTP or WebSocket dispatch. Hold the permit until the physical dispatch reaches a terminal outcome and its reservation is published or released. Apply the same semaphore to initial attempts, same-Channel retries, and Provider fail-forward attempts. Waiting for a permit does not create a status-specific timeout or error; existing lifecycle cancellation and deadlines still apply. No code path may start a physical upstream dispatch without this permit. Deployment preflight must prove that the configured value is at least the node's `approved_node_max_in_flight_dispatches` from section 13.6. The default is not a capacity approval.

Require `MONOIZE_STATUS_EVENT_PEAK_EVENTS_PER_SECOND` as a positive base-10 integer with no default on every Primary and Replica. Test and development nodes must set an explicit value; Phase 1 nodes use their assigned qualification rate.

Resolve the filesystem that contains the final spool directory. Read its allocation unit. Define `entry_reservation_bytes` as `MONOIZE_STATUS_EVENT_SPOOL_ENTRY_MAX_BYTES` rounded up to that allocation unit. Verify the value by creating, syncing, and measuring a non-sparse probe file whose logical length equals the entry maximum. Require the probe's OS-reported allocated size to be at most `entry_reservation_bytes`. A logical file length is never a substitute for OS-reported allocated size.

For each Primary or Replica node, set `node_peak_events_per_second = MONOIZE_STATUS_EVENT_PEAK_EVENTS_PER_SECOND`. Deployment preflight must prove that this configured value is at least that node's `approved_node_peak_events_per_second` from section 13.6. Do not reduce the approved peak through another concurrency, gateway, or rate-limit setting. Define:

```text
outage_event_slots = ceil(
    node_peak_events_per_second
    * MONOIZE_STATUS_EVENT_MAX_OUTAGE_SECONDS
    * MONOIZE_STATUS_EVENT_SPOOL_SAFETY_FACTOR_MILLI
    / 1000
)

in_flight_event_slots = MONOIZE_STATUS_EVENT_MAX_IN_FLIGHT_DISPATCHES
minimum_spool_event_slots = outage_event_slots + in_flight_event_slots
outage_bytes = outage_event_slots * entry_reservation_bytes
in_flight_bytes = in_flight_event_slots * entry_reservation_bytes
minimum_spool_bytes = outage_bytes + in_flight_bytes
```

Compute every product, sum, round-up, and subtraction with checked unsigned integer arithmetic. Implement `ceil(x / 1000)` as checked `(x + 999) / 1000`. Treat a parse error, arithmetic overflow, unavailable allocation unit, failed allocation probe, or configured quota below `minimum_spool_bytes` as a fatal configuration error. Do not start admission or background shipment with an invalid capacity configuration.

Charge every final and temporary event file by its OS-reported allocated size. Keep the bounded final and temporary node-state files outside the event quota but include their allocated blocks in the filesystem free-byte check. Reject symlinks and non-regular files in the spool directory. Before one physical dispatch, reserve `entry_reservation_bytes` and one future file slot. While that dispatch owns a temporary event file, charge the greater of the reservation and that file's allocated size, not their sum. After the same-directory rename succeeds, atomically replace the reservation charge with the final file's allocated size. Release the reservation without a file charge for an excluded outcome. If a temporary or final event file allocates more than `entry_reservation_bytes`, fail publication, set the incomplete-data latch, retain the measured allocation in quota accounting until cleanup succeeds, and do not start another physical dispatch from that permit until accounting is reconciled.

Reject an encoded event whose logical JSON length exceeds `MONOIZE_STATUS_EVENT_SPOOL_ENTRY_MAX_BYTES`. Set the incomplete-data latch for that counted event. Do not truncate or split the event.

Define `accounted_spool_bytes` as the sum of event-file allocation charges and outstanding per-dispatch reservation charges, with each active event counted once by the preceding transition rule. Define `remaining_spool_bytes = MONOIZE_STATUS_EVENT_SPOOL_MAX_BYTES - accounted_spool_bytes` with checked subtraction. Before readiness becomes healthy, require `remaining_spool_bytes >= minimum_spool_bytes`. This condition reserves one complete future outage plus every permitted in-flight dispatch even when a recovered backlog already exists.

Read filesystem bytes available to the deployed service account. Before readiness becomes healthy, require available bytes to be at least `remaining_spool_bytes + 67108864`. The final term reserves 64 MiB for the node-state replacement, directory blocks, filesystem metadata, and measurement drift. This byte check does not replace file-slot checks.

Define `remaining_quota_event_slots = floor(remaining_spool_bytes / entry_reservation_bytes)`. Query the filesystem's available inode or file-record count and any independent per-directory entry limit. Require at least `remaining_quota_event_slots + 1024` available file slots in every applicable finite limit. The final 1024 slots reserve node-state replacement, probe files, and service maintenance. A filesystem API result that positively states that one limit is dynamic or absent makes only that limit inapplicable. An unknown, unsupported, permission-denied, or ambiguous result is a capacity-query failure, not an unlimited result.

Fail process startup when the spool path cannot report allocated sizes, available bytes, or applicable file-slot capacity, or when `accounted_spool_bytes` exceeds the configured maximum. When configuration is valid but a recovered backlog, filesystem free-byte check, or file-slot check fails a startup admission condition, start the status batcher and Replica shipment in recovery-only mode, keep readiness unhealthy, and reject forwarding admission. Re-evaluate all startup admission conditions after every committed drain batch and at least once every 2 seconds. Begin serving only after every condition passes. This recovery path must not delete or ignore a durable event.

After the process begins forwarding, the outage reserve becomes usable working capacity. Do not return to recovery-only mode merely because current outage events reduce `remaining_spool_bytes` below `minimum_spool_bytes`. Enforce the byte and file-slot limits on every new reservation. If a reservation fails, keep the user dispatch behavior defined below and set the incomplete-data latch for a later counted outcome. A later readiness failure may still occur for an independently defined fatal service condition, but reserve consumption alone is not such a condition.

At startup, treat every final event file as replayable. Treat every temporary event file as an event whose durable publication did not complete. Set the incomplete-data latch, charge its allocated blocks during cleanup, delete it, and sync the directory. Enter recovery-only mode when deletion or directory sync fails. For a temporary node-state file, keep the final node-state file authoritative when it exists. Delete and sync the temporary file. When no final node-state file exists, treat the temporary state as evidence of an incomplete previous lifetime and set the latch before cleanup. Do not parse a temporary file as a committed event or state record.

Apply the same formula, allocation probe, allocated-size scan, free-byte check, and file-slot checks in deployment preflight for every Primary and Replica. Record each input, filesystem result, and computed result. Re-run the checks after resolving the deployed data directory and before starting the service. A default 512-MiB spool is valid only when it satisfies that node's formula and filesystem checks; the default is not a capacity guarantee.

Also reject startup when the entry limit is below `1024`, the total limit is below `entry_reservation_bytes`, or the spool directory cannot pass create, write, file-sync, allocation-size read, rename, directory-sync, and delete probes.

After acquiring the global dispatch permit, reserve one full allocated entry and one file slot before the physical dispatch. Reservation failure does not block that dispatch. On a counted terminal outcome, publish the event into that reservation; if no reservation exists, set the incomplete-data latch because one required event was lost. On an excluded outcome, release the reservation without setting the latch. Publication failure for a counted event sets the latch. Release its reservation only after cleanup succeeds or after the remaining file allocation is transferred into settled quota accounting.

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

### 13.6 Qualification Load and Fault Profile

For every deployed or planned Primary and Replica node, measure two values over its retained 30-day metrics interval. Measure the highest sustained rate of counted physical upstream dispatches over any consecutive five-minute window. Round it up to a whole event per second and call it `measured_node_peak_events_per_second`. Also measure the largest simultaneous count of physical upstream HTTP requests and WebSocket sessions from permit acquisition through terminal outcome. Call it `measured_node_max_in_flight_dispatches`.

When one node lacks 30 complete days, derive a conservative node event-rate peak and simultaneous-dispatch maximum from its longest available request-log, reverse-proxy dispatch, and connection-duration evidence. Multiply each observed value by two before rounding up. Require the owner to approve positive `approved_node_peak_events_per_second` and `approved_node_max_in_flight_dispatches` values that are at least their measured or derived node values. When only aggregate evidence exists, assign the complete aggregate value to every node. When no usable evidence exists, Gate D is blocked until the owner supplies and approves both positive design values for every node. Do not substitute zero for missing evidence.

Let `approved_aggregate_peak_events_per_second` be the sum of all approved node event-rate peaks for the intended topology. Compute the sum with checked unsigned arithmetic. Set `qualification_events_per_second = max(100, 5 * approved_aggregate_peak_events_per_second)`. Use the same constrained qualification host and storage floor defined for Marketplace. In each multi-source profile, allocate generated events among the Primary and four synthetic Replica sources in proportion to their approved node event-rate peaks. When the intended topology has fewer than five nodes, assign the remaining synthetic nodes the highest approved Replica peak, or the Primary peak when no Replica peak exists. Call each resulting rate `assigned_events_per_second`. Round allocations up to whole events per second, then reduce the largest allocation as needed so the sum equals `qualification_events_per_second`. Record every allocation and require their sum to equal the qualification rate.

For every qualification source, set its global semaphore to at least its approved simultaneous-dispatch maximum. Add held-open HTTP and WebSocket dispatches until the instrumented simultaneous count equals that configured bound. While those permits are held, verify that one additional dispatch waits and creates no reservation. Release permits and verify bounded forward progress without exceeding the configured count. This concurrency profile is separate from the event-rate profiles and does not replace them.

Run these three load-and-outage profiles separately:

1. Primary-only database outage: reject every Primary event-table transaction during minutes 5 through 20.
2. Multi-source database outage: run one Primary and four synthetic Replicas; reject every Primary event-table transaction during minutes 5 through 20 while Replica shipment remains reachable.
3. Multi-source shipment outage: run one Primary and four synthetic Replicas; reject every Replica status shipment during minutes 5 through 20 while Primary-local event-table writes remain available.

For each profile, generate events at `qualification_events_per_second` for 30 minutes. Keep upstream dispatch admission and spool publication running through the outage. At minute 20, restore the failed path while continuing the input rate for ten minutes. Then stop new input and drain the backlog. A pass in one profile does not substitute for another.

Require every generated counted event to reach one of three reconciled states: one committed unique row, one remaining durable final spool file, or one event-ID entry in the test harness's explicit lost-event ledger accompanied by the incomplete latch. The lost-event ledger is a qualification-harness oracle; production does not retain event IDs that it failed to spool. Require zero unaccounted events. The main load-and-outage profiles must contain zero lost-event ledger entries. Permit such entries only in an injection that intentionally exhausts or disables durable persistence. Require `data_complete = false` while an event older than `data_through` remains pending. Require the global dispatch semaphore, per-dispatch reservations, allocated spool blocks, and file-slot usage to remain within their configured bounds. For each synthetic source, set its configured peak to `assigned_events_per_second` and compute its qualification spool with the section 13.4 formula, outage value `900`, logical entry limit `4096`, measured allocation unit, safety factor `1200`, and configured global in-flight dispatch bound.

During recovery, require committed throughput to exceed the continuing input rate by at least 25 percent in every complete five-minute measurement window. After new input stops, require the remaining backlog to drain within 15 minutes. Require each process's resident-memory increase from its post-start idle baseline to stay at or below 256 MiB. Require the aggregate increase across the Primary and four Replica processes to stay at or below 768 MiB.

Repeat the profile with process termination at five points: before temporary-file rename, after rename, during a database transaction, after an ambiguous commit, and during Replica shipment. Restart the affected process. Require idempotent replay, correct pending counts, and the specified incomplete-data latch behavior.

Run separate injections for full spool quota, unwritable spool directory, lost directory-sync capability, database unavailability, Primary restart, Replica restart, clock skew beyond 30 seconds, and four Replicas replaying duplicate batches concurrently. Record accepted dispatches, counted outcomes, reservations, durable files, committed unique events, duplicates ignored, test-harness lost-event ledger entries, maximum backlog, recovery throughput, and peak resident memory. Gate D fails when conservation does not hold:

```text
generated counted event IDs
= committed unique event IDs
+ durable pending event IDs not already committed
+ test-harness lost-event IDs not already committed or pending
```

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

Build one immutable Status snapshot at most once per 15-second UTC bucket. Derive `data_through` and every aggregation from the bucket build time. Set `generated_at` once when the snapshot build completes. Reuse the exact uncompressed JSON bytes, `generated_at`, and ETag for the rest of that bucket. Refresh the frontend every 30 seconds through SWR.

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

Use approved migration-manifest names and post-migration names accepted with `confirm_public_exposure: true`. Never fall back to internal names.

Apply an application-level per-IP limit of 60 public API requests per minute with a burst of 20.

Use the existing trusted-proxy client-IP extraction contract. Do not trust an arbitrary forwarded IP header.

Return HTTP 429 with `rate_limited` when the limit is exceeded.

Add `Cache-Control`, ETag, `X-Content-Type-Options`, and existing CSP protections.

Use one process-local token bucket keyed by the canonical trusted-proxy client IP across `/api/public/site`, `/api/public/marketplace`, `/api/public/marketplace/offers`, and `/api/public/status`. Refill continuously at one token per second, cap tokens at 20, and consume one token per request. These rules implement the 60-per-minute rate with burst 20 per serving process.

The production topology approved by this design has exactly one public application process behind Caddy. Replica nodes do not expose public UI or public endpoints. This makes the application bucket the only public serving bucket for the approved topology.

Block production deployment when two or more application processes can serve a public endpoint. Before such a topology is permitted, enforce one equivalent 60-per-minute, burst-20 bucket per canonical client IP at the single public gateway or in a shared atomic rate-limit store. Do not sum independent process-local limits and describe the result as global.

Cap the bucket map at 10,000 entries. Define an entry as idle when it has received no request for at least 120 seconds. When a request from a new IP arrives at the cap, evict the least-recently-seen idle entry. When no entry is idle, return HTTP 429 with `rate_limited` and do not insert a bucket for the new IP. Continue to process an existing IP through its existing bucket while the map is at the cap.

Set Site and Marketplace `Cache-Control: public, max-age=15, stale-while-revalidate=30`. Set Status `Cache-Control: public, max-age=15`. Compute a weak ETag as `W/"<sha256>"`, where `<sha256>` is the lowercase SHA-256 hex digest of the exact uncompressed JSON snapshot bytes.

Include the canonical path and canonical query parameters in every public cache key. Send `Vary: Accept-Encoding`. Do not send `Vary: Cookie` because these endpoints ignore authentication state. The weak validator remains valid when Caddy changes only the content encoding.

For Site, build an immutable snapshot from the three selected settings. Set its ETag from its exact uncompressed JSON body. Keep it until one of those settings changes.

For Marketplace, use the persistent generation time as `generated_at`. For Status, use the bucket snapshot time. Do not use the current request time in a cacheable response body.

Parse `If-None-Match` as an HTTP entity-tag list. Return HTTP 304 with no body when the list contains the current weak ETag under weak comparison or contains `*`. Ignore a malformed header and return the normal HTTP 200 representation. Apply rate limiting before conditional-response evaluation.

Add automated negative assertions for every secret or internal field name.

Require review of the migration public-name manifest before deployment. Post-migration management writes use the explicit confirmation contract in section 7.1.

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

Before Gate B, emit every Provider classified as semantic-change. Include its old Group and Channel memberships, old weighted attempt order, target rows, target order, and target identifiers. Require one written approval per listed Provider. An empty report must state that it contains zero items and include the source fingerprint.

Test public-name and logical-model BLOB/BYTEA keys and database CHECK constraints on SQLite and PostgreSQL. Test public-name normalization collisions, concurrent inserts, direct-SQL invalid keys, rejected request key fields, and byte-identical ordering. Test every ASCII uppercase mapping and representative non-ASCII model bytes in `model_search_key`. Verify `(provider_id, model_name_key)` rejects duplicate logical models and that both model-key CHECK constraints reject a mismatched direct-SQL key. Verify the three public-name unique indexes reject the specified scopes without relying on application prechecks. Verify public-name collisions return HTTP 409 with `public_name_conflict` and no internal name or ID.

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

Test the checked minimum-spool formula at exact boundary and one byte below. Cover missing or invalid configured peak, configured event-rate peak below the approved node value, configured in-flight bound below the approved node simultaneous-dispatch value during deployment preflight, zero or invalid in-flight bound, arithmetic overflow, outage below 900, safety factor below 1200, and allocation-unit round-up. Verify one global semaphore bounds all initial attempts, same-Channel retries, Provider fail-forward attempts, and HTTP and WebSocket dispatches. Verify cancellation while waiting does not consume a permit or reservation. Verify no instrumented physical dispatch count exceeds the configured simultaneous-permit count.

Test recovered backlog that leaves less than one minimum-spool reserve, OS-allocated bytes above quota despite smaller logical lengths, a temporary file present at startup, a temporary file growing during publication, an entry whose allocated size exceeds its reservation, insufficient filesystem bytes, inode exhaustion, per-directory file-entry exhaustion, and each filesystem capacity or allocation-query failure. Verify fatal configuration and filesystem-query cases stop the process. Verify a valid configuration with insufficient admission capacity enters recovery-only mode, drains without accepting forwarding traffic, and becomes ready only after every byte and file-slot condition passes. Verify each Primary and Replica records its own approved event-rate peak, approved simultaneous-dispatch maximum, outage, logical entry limit, allocation unit, entry reservation, safety factor, configured in-flight bound, configured quota, allocated-byte total, available bytes, available file slots, and applicable directory limit.

Run section 13.6 profile 1 on the Primary-only topology. Run profiles 2 and 3 on one Primary plus four synthetic Replicas. Run the stated recovery, crash-point, and conservation checks for each applicable profile. Retain machine-readable measurements and event-ID reconciliation output.

### 21.4 Public Security

Assert that public responses omit all internal and secret fields.

Test the exact `/api/public/site` allow-list. Test rate limiting, bucket-cap exhaustion, idle eviction, cache headers, ETag lists, wildcard validators, malformed validators, malformed query input, and large datasets.

Test Marketplace list and offer pagination at limits 1, 24, 50, and invalid values. Test cursor tampering, invalid signatures, filter mismatch, revision changes, duplicate sort keys, maximum-length cursor keys, ASCII-case search, non-ASCII literal search, empty pages, and the 1048576-byte boundary. Test that an offer `model` with any leading or trailing Unicode White_Space returns HTTP 400 `invalid_request`; test that a valid exact but absent model returns HTTP 404 `marketplace_model_not_found`.

Test that SQLite and PostgreSQL return byte-identical pagination sequences for names and model IDs containing non-ASCII text.

Run the maximum-envelope Marketplace benchmark from section 12. Test zero-, one-, 50-, and broad-match searches. Verify that search remains database-filtered and application memory does not scale with the complete mapping catalog.

Run every source-write invalidation scenario and bound from section 12 on SQLite and PostgreSQL. Verify the backend-specific generation deltas for plain writes, zero-row writes, upsert-update, and conflict-do-nothing statements.

Count database statements for 1, 24, and 50 returned rows. Verify that statement count is independent of returned-row count except for bounded bind-parameter chunks.

Test missing and malformed cursor HMAC key rows. Test stable cursors across process restart and equivalent decoding on SQLite and PostgreSQL.

Inject Marketplace generation changes before and after every source query. Verify that a response contains one revision only and that three consecutive changes return `marketplace_snapshot_busy`.

Test that repeated Site requests over time return byte-identical uncompressed JSON bodies and HTTP 304 while the three selected settings are unchanged. Test that repeated Marketplace requests do the same while its source generation is unchanged. Test weak ETag syntax with identity and compressed transfer encodings. For each of the six Marketplace source tables, exercise each applicable management, background synchronization, bulk synchronization, migration or seed, and direct-SQL writer listed in the manifest. Mark an inapplicable writer category explicitly instead of inventing a path. Verify that every committed relevant insert, update, and delete strictly advances both generation values and changes the Marketplace encoded body and ETag. Verify that rollback restores the preceding generation. Verify that deletion of the singleton fails. Verify that PostgreSQL rejects `TRUNCATE marketplace_generation`. Verify that a direct update cannot decrease or reuse a revision, skip a revision, change `singleton_id`, or keep or decrease the generation timestamp. Verify that one exact direct increment with a later timestamp succeeds and only invalidates the cache. In isolated fixtures that intentionally bypass or alter the target constraints, verify that a missing, out-of-range, or exhausted singleton aborts a source write. Verify PostgreSQL `TRUNCATE` on each Marketplace source table advances the generation and that SQLite tooling never uses `TRUNCATE`.

For `system_settings`, test insert, delete, key changes into and out of `reasoning_suffix_map`, byte-different value changes, byte-identical value writes, `updated_at`-only writes, and unrelated setting writes. Require generation advancement only for changes that can affect the filtered row. Verify that a missing row uses the billing default, invalid persisted JSON returns HTTP 503 `marketplace_source_invalid`, and a valid map change alters normalized pricing lookup and produces the expected Marketplace body and ETag change. Test PostgreSQL transition-table behavior for multi-row statements and every upsert conflict outcome used by current writers.

Compare the machine-readable generation-source manifest with source-code search results, source table schemas, and database trigger metadata. Require all six tables, the filtered-row rule, every source column classified as included or excluded, all 18 source-operation triggers per database, the six PostgreSQL source-table `TRUNCATE` triggers, and every discovered writer to be present. Mutate each included column and verify a generation change. Mutate each excluded column and verify the serialized Marketplace body remains byte-identical.

Test that two Status requests in one snapshot bucket return byte-identical bodies and HTTP 304. Test that the next bucket produces one later `generated_at` and recomputes `data_through`.

Test startup and deployment preflight with one public process. Test that a declared multi-process public topology fails the deployment gate until gateway or shared-store rate limiting is configured and verified.

Test that every new or changed public name requires `confirm_public_exposure: true`. Test that an unchanged normalized name does not require it.

Test modal focus trapping, Escape close, focus restoration, keyboard navigation, mobile layout, and reduced motion.

During Phase 1, run Marketplace query, cursor, aggregation, size-limit, encoding, cache, and cross-database tests through the isolated executable. Also run response-schema allow-list serialization, secret-field negative, standalone token-bucket and bucket-cap, and ETag-parser tests without registering an HTTP route. Defer HTTP integration, SWR, modal, and browser tests until product implementation receives separate approval. Re-run the complete section against the final implementation.

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

## 22. Release Gates

Do not begin behavior-specification updates or migration-rehearsal planning until all Gate A evidence exists.

Passing Gate A does not authorize product implementation.

### Gate A: Written Design

- The user approves this document.
- The user confirms intentional public display of reviewed Group, Provider, and Channel public names.
- The user confirms that the old Provider management contract has no required external caller.
- Design approval authorizes behavior-specification updates and migration-rehearsal planning only. It does not authorize executable rehearsal artifacts, product implementation, production migration, or deployment.

### Gate B: Migration Rehearsal

- A redacted production copy passes migration on SQLite.
- Synthetic equivalent fixtures pass migration on PostgreSQL.
- The projected row expansion is reviewed.
- Every semantic-change Provider has explicit written approval.
- Transaction rollback tests pass.
- Public-name and logical-model binary-key CHECK, primary-key, uniqueness, and byte-order tests pass on both databases.
- The generation-source manifest covers every source table, source column, trigger, and discovered writer.
- The isolated maximum-envelope Marketplace query, cursor, aggregation, encoding, and cache benchmark meets every latency, memory, query, and response-size bound on both databases without registering an HTTP listener.
- The maximum-envelope source-write invalidation benchmark meets every transaction-latency, throughput, lock, generation-delta, WAL, and checkpoint bound on both databases.

### Gate C: Pricing Equality

- The frozen pre-migration and post-migration pricing snapshots are byte-equivalent after canonical serialization.
- Every scenario charge is exactly equal.
- No unresolved difference is waived.

### Gate D: Status Reliability

- Retry, replay, concurrency, and fault-injection tests pass.
- Lost events mark status data incomplete.
- Status persistence failures do not change API billing or response results.
- Production event-rate and simultaneous-dispatch evidence, or owner-approved conservative design values, exist for every intended Primary and Replica.
- Every Primary and Replica passes the minimum spool-quota, allocated-block, free-byte, and file-slot preflight with recorded inputs.
- The measured physical dispatch concurrency never exceeds the configured process-wide semaphore bound.
- All three section 13.6 load-and-outage profiles, recovery bounds, crash points, four-Replica injections, and conservation checks pass.

### Gate E: Public Security

- In the rehearsal form, schema-level allow-list serialization tests pass for every proposed public response without registering an HTTP route.
- In the rehearsal form, secret-field negative tests pass against serialized fixture responses.
- In the rehearsal form, the isolated token-bucket, bucket-cap, ETag parser, cursor, pagination, and response-size test suites pass.
- The migration public-name manifest receives explicit approval.
- The production topology preflight plan identifies exactly one intended public application process, or requires an equivalent gateway or shared-store global limit before implementation may request deployment.

Passing the rehearsal form of Gates B-E creates the evidence required for a separate product-implementation approval. Re-run Gates B-E against the final implementation before requesting deployment approval. Rehearsal results do not authorize implementation or deployment.

For the final implementation run of Gate B, execute the fixed Marketplace benchmark corpus through the real public endpoints and meet the same bounds.

For the final implementation run of Gate E, run the actual HTTP allow-list, secret-field, rate-limit, cache, ETag, pagination, and response-size tests. Run deployment preflight against the real serving topology. The rehearsal substitutions above do not satisfy the final Gate E.

## 23. Deployment Gate and Rollback

Use a cold maintenance-window cutover because the migration removes old columns.

Treat the candidate as Primary-only until migration finishes and route verification passes. Stop every Replica before write drain. A Replica must not point at the migrated database until it runs the same candidate image and its read-only schema verification accepts the new schema.

Before cutover:

1. Build and verify the candidate image.
2. Resolve the deployed spool directory for every Primary and Replica. Run and record the minimum spool-quota, allocation probe, allocated-block scan, free-byte check, file-slot checks, and global dispatch-bound preflight for every node. Stop when any node would fail startup or enter recovery-only mode.
3. Stop every Replica, stop new writes on the Primary, and record that no old binary remains connected.
4. Drain in-flight requests and background batches.
5. Run the final database preflight. Require its database fingerprint to equal the approved manifest fingerprint.
6. Create a current SQLite Online Backup snapshot, including all committed WAL content.
7. Hash the snapshot and run `PRAGMA quick_check`.
8. Restore the snapshot into an isolated directory.
9. Start the old image against the restored copy without external network access.
10. Verify health and expected row counts.
11. Run the new image and migration against a second restored copy.
12. Verify migration, pricing snapshot equality, and public contract tests.

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

Migration equality, pricing equality, source-write invalidation performance, status-event failure behavior, per-node spool capacity, stable ETag behavior, Marketplace size bounds, public-name approval, global rate-limit topology, and restore rehearsal remain mandatory release gates.

After this revised design receives user approval, proceed only with behavior-specification updates and migration-rehearsal planning. Require explicit approval before creating executable Phase 1 rehearsal artifacts. Require separate written approval before product implementation. Require another separate written approval before production deployment.
