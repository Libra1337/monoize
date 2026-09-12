# Provider Storage, Migration, and Pricing Specification

## 0. Scope and precedence

PP-0.1. This specification defines the post-migration Provider, embedded Channel, Group
public-name, Provider-model, management API, migration, and effective pricing contracts.

PP-0.2. This specification replaces every pre-migration rule that permits one Provider to
reference multiple Groups or multiple Channel rows. It also replaces Channel-level model
multiplier ownership.

PP-0.3. The migration MUST remove obsolete fields, columns, tables, entities, stores, and
request or response aliases. It MUST NOT retain a compatibility representation.

## 1. Post-migration invariants

PP-I1. Each Provider MUST reference exactly one Group through `group_id`.

PP-I2. Each Provider MUST contain exactly one Channel value. The Channel MUST be stored in
the Provider row through `channel_`-prefixed columns. A separate Channel table MUST NOT
exist.

PP-I3. A Provider MAY contain zero or more model mappings in
`monoize_provider_models`. A Provider with zero mappings is ineligible for every model
route and has no public Marketplace offer.

PP-I4. One Provider model mapping MUST be identified by `(provider_id, model_name_key)`.
It MUST NOT contain a separate row ID.

## 2. Provider storage

PP-P1. `monoize_providers` MUST contain these Provider-owned columns:

```text
id TEXT PRIMARY KEY
group_id TEXT NOT NULL
name TEXT NOT NULL
public_name TEXT NOT NULL
public_name_key BLOB/BYTEA NOT NULL
priority INTEGER NOT NULL
enabled INTEGER NOT NULL
pricing_profile TEXT NULL
multiplier TEXT NOT NULL
configuration_generation BIGINT NOT NULL
created_at TEXT NOT NULL
```

PP-P2. `group_id` MUST reference `monoize_groups.id`. Group deletion MUST use restricted
semantics. Provider deletion MUST cascade to its model mappings.

PP-P3. `multiplier` MUST be a canonical positive exact decimal string with at most nine
fractional digits. Parsing, comparison, storage, and serialization MUST NOT use binary
floating point.

PP-P4. A null `pricing_profile` means that the Provider supplies no default Profile. A
non-null value MUST be trimmed by Unicode White_Space, remain non-empty, and match one
Profile present in at least one billing-rate record. Matching is case-sensitive.

PP-P5. Profile validation does not require a complete rate matrix for every mapped model.
A structurally valid but incomplete mapping is unpriced under section 8.

PP-P6. `configuration_generation` MUST start at one. One logical mutation that changes
the Provider, its embedded Channel, or its model mappings MUST increment it exactly once
inside the mutation transaction. A failed or rolled-back mutation MUST not increment it.

PP-P7. New Provider IDs MUST be lowercase UUID v4 strings. A create request MUST NOT
accept an ID. An update request MUST NOT change an ID. Migrated legacy IDs remain opaque.

PP-P8. Providers MUST have an index over `(group_id, priority, created_at, id)`.

## 3. Embedded Channel storage

PP-C1. Every surviving pre-migration Channel column MUST move into
`monoize_providers` with a `channel_` prefix. The set includes Channel ID, internal name,
public name, API type, Base URL, API key, enabled state, retry and breaker overrides,
probe overrides, affinity overrides, proxy URL, extra headers, session-affinity settings,
and missing-usage policy.

PP-C2. Every required embedded Channel column MUST be `NOT NULL`. `channel_id` MUST be
unique across Provider rows.

PP-C3. New Channel IDs MUST be lowercase UUID v4 strings. A Provider create request MUST
NOT accept a Channel ID. A Provider update request MUST NOT change it. Migrated legacy
Channel IDs remain opaque.

PP-C4. Channel `weight` MUST NOT exist. A disabled Channel is represented only by its
enabled field.

PP-C5. Provider `max_retries` MUST NOT exist. `channel_max_retries` remains the count of
same-Channel physical retries. One Provider attempt therefore permits at most
`channel_max_retries + 1` physical attempts, except for an existing affinity-target extra
retry explicitly retained by the routing specification.

PP-C6. The old dashboard Channel test route MUST NOT exist. The only Channel test route is
`POST /api/dashboard/providers/{provider_id}/channel/test`.

## 4. Provider model storage

PP-M1. `monoize_provider_models` MUST contain exactly these logical fields:

```text
provider_id TEXT NOT NULL
model_name TEXT NOT NULL
model_name_key BLOB/BYTEA NOT NULL
model_search_key BLOB/BYTEA NOT NULL
redirect TEXT NULL
pricing_profile_mode TEXT NOT NULL
pricing_profile_override TEXT NULL
multiplier_override TEXT NULL
created_at TEXT NOT NULL
PRIMARY KEY (provider_id, model_name_key)
```

PP-M2. `provider_id` MUST reference `monoize_providers.id` with delete cascade.

PP-M3. A management write MUST trim `model_name` by Unicode White_Space. The stored value
MUST contain 1 through 256 UTF-8 bytes. It MUST NOT contain a C0 control, DEL, CR, LF, or
tab. Comparison is case-sensitive.

PP-M4. `model_name_key` MUST equal the exact UTF-8 bytes of `model_name`.
`model_search_key` MUST equal those bytes after adding 32 to every byte from ASCII `A`
through `Z`; every other byte remains unchanged.

PP-M5. SQLite MUST store keys as BLOB. PostgreSQL MUST store keys as BYTEA. Both databases
MUST enforce byte-equality CHECK constraints for both keys. The search-key CHECK MUST use
26 explicit ASCII replacements and MUST NOT use a locale-sensitive lowercase operation.

PP-M6. The application MUST derive both keys. A management request that contains either
key MUST return HTTP `400` with code `invalid_request`.

PP-M7. The table MUST have an index over `(model_name_key, provider_id)`.

PP-M8. `pricing_profile_mode` MUST be exactly `inherit`, `override`, or `unpriced`.
`pricing_profile_override` MUST be non-null exactly for `override`. It MUST be null for the
other two modes.

PP-M9. A non-null Profile override MUST satisfy PP-P4. A non-null multiplier override MUST
satisfy PP-P3.

## 5. Public names and binary keys

PP-N1. Group, Provider, and Channel public names MUST be trimmed by Unicode whitespace and
normalized to Unicode NFC. The result MUST contain 1 through 64 Unicode scalar values and
MUST NOT contain a C0 control, DEL, CR, LF, or tab.

PP-N2. Each public-name key MUST equal the exact UTF-8 bytes of the normalized name.
SQLite MUST use BLOB and enforce `key = CAST(name AS BLOB)`. PostgreSQL MUST use BYTEA and
enforce `key = convert_to(name, 'UTF8')`. SQLite database encoding MUST be UTF-8.

PP-N2.1. Group `public_name` and `public_name_key` MUST be non-null after migration.
PostgreSQL MUST enforce both properties with column and CHECK constraints. SQLite MUST
reject each insert or update that makes either value null or makes the key differ from the
UTF-8 bytes of the public name.

PP-N3. The application MUST derive public-name keys. A management request that contains a
key MUST return HTTP `400` with code `invalid_request`.

PP-N4. These unique indexes are required:

- `monoize_groups(public_name_key)`;
- `monoize_providers(group_id, public_name_key)`;
- `monoize_providers(group_id, channel_public_name_key)`.

PP-N5. The indexes in PP-N4 are the final concurrent-write guard. Application prechecks
MAY improve error detail but MUST NOT replace an index.

PP-N6. A collision MUST return HTTP `409` with code `public_name_conflict` and include only
the entity type (`group`, `provider`, or `channel`) and normalized public name. It MUST NOT
include an internal name, internal ID, constraint name, or database error.

PP-N7. A create request that assigns a public Group, Provider, or Channel name MUST contain
`confirm_public_exposure: true`. An update that changes a normalized public name MUST
contain the same value. One Provider-level confirmation covers the Provider and embedded
Channel. The field is request-only.

PP-N8. A missing or non-true required confirmation MUST return HTTP `400` with code
`public_exposure_confirmation_required`. An approved migration manifest supplies the
confirmation for migrated names.

## 6. Management API

PP-A1. Provider management MUST expose exactly these paths:

```text
GET    /api/dashboard/providers
POST   /api/dashboard/providers
POST   /api/dashboard/providers/wholesale
GET    /api/dashboard/providers/{provider_id}
PUT    /api/dashboard/providers/{provider_id}
DELETE /api/dashboard/providers/{provider_id}
POST   /api/dashboard/providers/{provider_id}/channel/test
POST   /api/dashboard/providers/reorder
```

PP-A2. Provider requests and responses MUST use singular `group_id` and `channel` fields.
They MUST NOT contain `group_ids` or `channels`.

PP-A3. A request containing `group_ids`, `channels`, `weight`, `max_retries`, or another
removed legacy Provider or Channel field MUST return HTTP `400` with code
`invalid_request`.

PP-A4. Reorder input MUST contain one `group_id` and every current Provider ID in that
Group exactly once. A missing, duplicate, unknown, or cross-Group ID MUST return HTTP
`400`. Success MUST assign dense priority values from zero inside that Group and MUST NOT
change another Group.

PP-A5. Provider create or update MUST mutate the Provider, embedded Channel, and complete
model mapping set in one transaction. A failure MUST preserve the preceding state.

PP-A6. Successful create and update responses MUST return structured pricing warnings by
logical model name and missing usage class. An unpriced mapping is a warning, not a
structural validation failure.

PP-A6.1. Each warning MUST contain exactly `logical_model: string` and
`missing_usage_classes: string[]`. The array MUST contain unique usage-class names in
ascending UTF-8 byte order. It MUST contain `input_uncached` and `output` when a mapping
has no effective Profile or uses `unpriced` mode. An incomplete Profile MUST report each
required token usage class that prevents selection of a complete rate matrix.

PP-A6.2. The create or update response MUST otherwise preserve the Provider response
shape and add `pricing_warnings`. The array MUST be empty when every mapping has a complete
effective rate matrix.

PP-A7. Group create and update requests MUST apply PP-N7 and PP-N8. Group deletion while a
Provider references the Group MUST return HTTP `409` with code `group_in_use`.

PP-A8. `/v1/**` request shapes remain unchanged.

## 7. Effective pricing

PP-E1. Effective values MUST resolve as follows:

```text
inherit  -> effective_profile = provider.pricing_profile
override -> effective_profile = model.pricing_profile_override
unpriced -> effective_profile = null

effective_multiplier = model.multiplier_override ?? provider.multiplier
```

PP-E2. Runtime pricing MUST test the reasoning-suffix-normalized upstream model first and
the normalized logical model second within the selected Profile.

PP-E3. Runtime pricing MUST select the first complete applicable rate matrix under the
existing Provider-type and usage-class rules.

PP-E4. Global `pricing_profile_model_patterns` MUST NOT select a runtime Provider billing
Profile after migration. Model metadata Profile values are administrative suggestions and
MUST NOT serve as an implicit billing fallback.

PP-E5. `effective_multiplier` MUST be used for both charge arithmetic and the existing
request `max_multiplier` filter.

PP-E6. Base token and meter line items MUST be checked integer nano-USD values. The engine
MUST sum them, apply the effective exact decimal multiplier once, and truncate toward zero
once at final scaling.

PP-E7. A browser MUST NOT calculate a billable price or multiply a rate with JavaScript
binary floating-point arithmetic.

## 8. Missing pricing

PP-U1. A mapping is unpriced when its mode is `unpriced`, no inherited Profile exists, or
its selected Profile lacks a complete required rate matrix.

PP-U2. An unpriced mapping MUST be excluded from routing and public Marketplace offers.

PP-U3. If at least one structurally eligible mapping exists and every such mapping is
unpriced, forwarding MUST return HTTP `403` with code `model_pricing_required`.

PP-U4. If no structurally eligible mapping exists, forwarding MUST preserve the existing
no-model or no-Provider error instead of returning `model_pricing_required`.

PP-U5. A mapping MUST become routable and publicly visible without another Provider write
after its required rate records become complete.

## 9. Migration preflight

PP-X1. Preflight MUST classify every old Provider before writing.

PP-X2. A Provider is route-safe exactly when it has one Group ID and at most one enabled
Channel with positive weight.

PP-X3. Preflight MUST block a Provider with zero Groups, zero Channel rows, an unknown
Group, malformed persisted configuration, an invalid model name under PP-M3, or a name
collision under PP-N4.

PP-X4. A Provider with multiple Groups or two or more enabled positive-weight Channels is
a semantic-change Provider. Preflight MUST emit its old Group and Channel memberships,
weighted attempt order, target rows, target order, and target IDs. Migration execution
MUST require one written approval per emitted Provider.

PP-X5. Preflight MUST NOT claim routing equivalence for a semantic-change Provider.

PP-X6. Preflight MUST emit a reviewed public-name manifest containing normalized text and
hexadecimal binary keys for every Group, Provider, and Channel target.

PP-X6.1. The approved manifest MUST use schema version `1`. It MUST identify each Group by
source Group ID. It MUST identify each Provider and Channel target by source Provider ID,
source Channel ID, target Group ID, target Provider ID, and target Channel ID. Each entry
MUST contain the approved normalized public name and its lowercase hexadecimal UTF-8 key.
The complete entry set MUST equal the complete deterministic migration target set.

PP-X6.2. Before product migration starts, the operator MUST store the approved manifest as
the `state_records` value whose key is `('system', 'provider_pricing_migration',
'approved_manifest_v1')`. The value MUST be the exact reviewed JSON document. Migration
MUST reject a missing, malformed, duplicate, incomplete, or additional entry.

PP-X6.3. A fresh database MAY omit the manifest only when it contains zero legacy
Providers and exactly one Group whose internal name is the built-in value `default`. In
that case migration MUST set the Group public name to `Default`. Another zero-Provider
database MUST satisfy PP-X6.2.

PP-X7. The manifest fingerprint MUST cover every migration-relevant field. For an API key,
proxy credential, or other secret, it MUST include
`HMAC-SHA-256(comparison_key, field_tag || secret_value)` and MUST NOT include the secret or
an unkeyed digest. The comparison key MUST be external to the manifest.

PP-X8. After writes stop, final preflight MUST reproduce the approved fingerprint before
migration begins.

PP-X8.1. Product migration MUST read the comparison key from the owner-readable file named
by `MONOIZE_PROVIDER_MIGRATION_COMPARISON_KEY_FILE`. A missing variable, unreadable file,
empty key, or file longer than 4096 bytes MUST abort migration before a schema write.

PP-X8.2. Product migration MUST recompute the source fingerprint inside its migration
transaction before a schema write. It MUST compare the lowercase hexadecimal result with
the manifest `source_fingerprint`. A mismatch MUST abort the transaction. The fingerprint
input MUST include every Group, Provider, Channel, model mapping, pricing setting,
model-metadata Profile, and enabled Billing Rate field read by the transformation. It MUST
replace API keys, proxy URLs, and extra headers with HMAC-SHA-256 values keyed by the
comparison key before hashing the canonical field stream.

PP-X8.3. Product migration MUST persist only manifest public names. It MUST NOT derive a
public name from an internal Group, Provider, or Channel name.

## 10. Migration transformation

PP-T1. One old Provider MUST expand into one target Provider for every `(Group, Channel)`
pair, including disabled and zero-weight Channels. A zero-weight Channel MUST become
disabled.

PP-T2. Expansion MUST use stored Group order and Channel order
`created_at ASC, id ASC`.

PP-T3. The first pair MUST keep the old Provider ID. Each old Channel's first Group copy
MUST keep its Channel ID. Later copies MUST use deterministic IDs.

PP-T4. A deterministic ID MUST use SHA-256 over versioned entity type, old Provider ID,
Group ID, and old Channel ID, with every component length-prefixed as UTF-8. Generated
Provider IDs use `p_` plus the first 32 lowercase hexadecimal digest characters. Generated
Channel IDs use `c_` plus the same encoding. Any collision MUST abort before writing.

PP-T5. The row that keeps the old Provider ID MUST keep its internal name. Other internal
names MUST use `old Provider name / Group name / Channel name`, with an eight-character
deterministic digest appended only to resolve a collision.

PP-T6. Each Group's target Providers MUST sort by old Provider priority, old Provider
creation time, old Provider ID, old Channel creation time, and old Channel ID. Migration
MUST assign dense Group-local priorities from zero.

PP-T7. Historical request-log IDs and `tried_providers_json` MUST NOT be rewritten.
Channel lookup MUST resolve through the embedded `monoize_providers.channel_id`.
Persisted request-time names remain authoritative when present.

PP-T8. Before migration, the rehearsal MUST resolve each old mapping's effective Profile,
pricing model, rate rows, and multiplier.

PP-T9. The target Provider default Profile MUST be the most frequent non-null resolved
Profile. A tie uses ascending UTF-8 byte order. If all mappings are unpriced, the default
is null.

PP-T10. A target mapping uses `inherit` when its old resolved Profile equals the Provider
default, `override` when a non-null value differs, and `unpriced` when the old resolved
Profile is null.

PP-T11. The target Provider multiplier MUST be the most frequent canonical old multiplier.
A tie uses ascending exact decimal numeric value. A Provider without mappings uses `1`.
A target mapping stores an override exactly when its old value differs.

PP-T12. Migration MUST create the target schema, copy and validate target rows, create all
constraints and indexes, remove `monoize_channels`, `monoize_channel_models`, Provider
`group_ids`, Provider `max_retries`, and obsolete model multiplier storage, then commit one
transaction.

PP-T13. Migration MUST be idempotent. A second invocation on the target schema MUST make
no change.

PP-T14. Any failure in validation, schema creation, row writing, constraint creation, or
obsolete-storage removal MUST roll back the complete migration and preserve the old schema
and rows.

## 11. Pricing equality gate

PP-G1. Rehearsal MUST create canonical pre- and post-migration pricing snapshots from the
same database copy. The pre-snapshot MUST project every old mapping once per PP-T1 target.

PP-G2. Each row MUST contain an opaque mapping digest, logical and upstream models,
Provider type, resolved Profile, resolved pricing model, ordered rate IDs and integer
rates, canonical multiplier, exact decimal display rates, and scenario charges.

PP-G3. Scenarios MUST cover one unit, 999 units, one million units, input only, output only,
cache read, each cache write duration, each meter class, and mixed token and meter usage
when applicable.

PP-G4. Rehearsal MUST freeze and hash the used billing-rate rows.

PP-G5. Gate C passes only when canonical snapshots are byte-equivalent and every integer
charge is exactly equal. A tolerance or waived difference is forbidden.

## 12. Management UI

PP-F1. One Provider editor MUST show Identity and Group, public names, default Billing
Profile and multiplier, single Channel configuration, model mappings and overrides, and a
validation summary in that order.

PP-F2. Group selection MUST be singular. The UI MUST NOT render Add Channel.

PP-F3. When PP-N7 requires confirmation, the editor MUST show an unchecked explanation
that the Group, Provider, and Channel names become public. It sends
`confirm_public_exposure: true` only after selection.

PP-F4. Each model row MUST show inherited and effective Profile and multiplier values.
Price previews MUST come from the backend. The browser MUST NOT derive them.

PP-F5. The editor MUST allow save with PP-A6 warnings and block only structural or input
validation errors.

PP-F6. Data fetching MUST use SWR. A mutation MUST update optimistically, roll back on
failure, and revalidate all affected Provider, routing, Marketplace, and Playground keys.
Initial loading and hydration MUST render a matching Skeleton.

## 13. Verification

PP-V1. SQLite and PostgreSQL tests MUST cover every invariant, CHECK, unique index,
transaction rollback point, deterministic ID, collision, and idempotent second run.

PP-V2. Migration tests MUST cover zero, one, and multiple Groups; zero, one, disabled,
zero-weight, and multiple enabled Channels; malformed JSON; missing rows; and every
semantic-change report.

PP-V3. API tests MUST reject every removed field and verify no compatibility field or
table remains.

PP-V4. Pricing tests MUST cover Profile and multiplier ties, redirects, Provider types,
cache and meter rates, missing rates, maximum integers, and exact PP-G5 equality.

PP-V5. UI tests MUST verify singular Group and Channel controls, public-exposure
confirmation, optimistic rollback, revalidation, warnings, and Skeletons.

## Account-Class Price Isolation

PP-ENT1. Every price resolution MUST include the account class inherited from the selected Provider Group.

PP-ENT2. A price candidate whose account class differs from the authenticated user account class MUST be ineligible.

PP-ENT3. A missing in-class price MUST fail price resolution. Resolution MUST NOT fall back to a candidate from the other account class.

PP-ENT4. Enterprise and standard rates MAY contain different values for the same normalized model, API type, usage class, context tier, service tier, modality, and cache TTL.

PP-ENT5. PP-ENT1 through PP-ENT4 are enforced through the effective Profile, not through an
account-class column on billing-rate records. Rate lookup is keyed by `effective_profile`,
and `effective_profile` derives from the selected Provider or Channel (PP-M8), whose account
class is already restricted to the authenticated user account class. An Enterprise request
therefore reaches only Enterprise Providers and resolves only their Profiles.

PP-ENT6. Required invariant that PP-ENT5 depends on: one Profile name MUST NOT be reachable
from Providers of both account classes. A shared Profile name would make one set of
billing-rate records resolve for both classes and defeat PP-ENT2 and PP-ENT3, even
though every routing query filters by account class. Enterprise and standard Providers
therefore MUST use disjoint Profile names. The exclusivity applies among the classes
`standard`, `enterprise`, and `private` only; the `agent` class is exempt by PP-W8.

PP-ENT7. Provider create and Provider update MUST enforce PP-ENT6 before the write commits.
The account class is the class of the Group the write targets, which is the requested Group
when the write moves the Provider and the current Group otherwise. A Profile name is
reachable from an account class when a Provider of that class names it as its Provider
Profile or a model entry of that Provider overrides its Profile to that name. A write whose
requested Profile name, including every model-level override, is reachable from the other
account class MUST fail with HTTP `409` and code `pricing_profile_account_class_conflict`,
and MUST NOT change any row. The Provider being updated is excluded from the reachability
check, so keeping its own Profile is never a conflict.

PP-ENT8. On update, `pricing_profile` and the Channel are optional and an absent field keeps
its stored value. The PP-ENT7 check MUST therefore run against the effective post-update
state: the requested Provider Profile when the request supplies one and the stored Profile
otherwise, and the requested Channel model entries when the request supplies them and the
stored model entries otherwise. A request that names only `group_id` MUST still be checked,
because moving a Provider carries its stored Profile into the target account class.

## 14. Wholesale Providers (agent account class)

PP-W1. A Provider whose Group account class is `agent` is a **wholesale Provider**. It MUST
be created only through `POST /api/dashboard/providers/wholesale` with Admin authorization.
The ordinary Provider create request (PP-A1 `POST /api/dashboard/providers`) MUST reject a
target Group of the `agent` class with HTTP `400` code `invalid_request`.

PP-W1a. A Provider update MUST NOT change whether the Provider belongs to the `agent`
class: moving a Provider from a non-agent Group into an agent Group and moving a wholesale
Provider out of the `agent` class MUST both fail with HTTP `400` code `invalid_request`.
Moving a wholesale Provider between two agent Groups is allowed.

PP-W2. The wholesale create request MUST use `deny_unknown_fields` and contain exactly these
fields:

```text
group_id              TEXT       target Group id (agent class)
source_provider_id    TEXT       source Provider id
multiplier            decimal?   Provider default multiplier
name                  string?    internal Provider name override
channel_name          string?    internal Channel name override
model_multipliers     {model_name: decimal}?  per-model wholesale multipliers
confirm_public_exposure boolean
enabled               boolean?   default true
priority              integer?
```

PP-W3. The target Group MUST exist and its account class MUST be `agent`; otherwise the
request MUST fail with HTTP `400` code `invalid_request` and change no row. The source
Provider MUST exist and its Group account class MUST be one of `standard`, `enterprise`,
`private`; otherwise the request MUST fail with HTTP `400` code `invalid_request` and change
no row. A wholesale Provider MUST NOT source from another wholesale Provider.

PP-W4. The created Provider copies from the source Provider, field for field: the embedded
Channel configuration (`provider_type`, `base_url`, `api_key`, `enabled`,
`allow_missing_usage`, every passive/active-probe/affinity override, `proxy_url`,
`extra_headers`, `session_affinity_auto`), `transforms`, `api_type_overrides`, provider-level
probe and timeout overrides, `extra_fields_whitelist`, `strip_cross_protocol_nested_extra`,
`circuit_breaker_enabled`, `per_model_circuit_break`, `channel_max_retries`,
`channel_retry_interval_ms`, `pricing_profile`, and every model mapping (`model_name`,
`redirect`, `pricing_profile_mode`, `pricing_profile_override`).

PP-W5. Wholesale multiplier defaults: the created Provider `multiplier` MUST equal the
request `multiplier` when present, else the source Provider `multiplier`. Every created model
mapping MUST store an explicit `multiplier_override` equal to
`model_multipliers[model_name]` when the request supplies that model, else the source
mapping's effective multiplier under PP-E1 (`source multiplier_override ?? source Provider
multiplier`). The copy is a one-time materialization: later edits of the source Provider do
not propagate.

PP-W6. A `model_multipliers` key that names no model mapping of the source Provider MUST be
rejected with HTTP `400` code `invalid_request`.

PP-W7. `name` defaults to the source Provider's name and `channel_name` to the source
Channel's name. Public names are derived from those names under PP-N1 through PP-N4, and
`confirm_public_exposure: true` is required under PP-N7 and PP-N8.

PP-W8. Pricing-Profile validation for a wholesale Provider: the PP-P4 known-Profile check
applies unchanged, but the PP-ENT6/PP-ENT7 cross-class exclusivity does not. A wholesale
Provider MAY reference a Profile reachable from any other account class, and a Profile
reachable from an `agent` Provider MUST NOT make a write in another class conflict. The
exclusivity between two distinct non-agent classes is unchanged.

PP-W9. The response MUST equal the ordinary create response (PP-A6 including
`pricing_warnings`) with HTTP `201`.

PP-W10. After creation, a wholesale Provider is an ordinary Provider. Updates, reordering,
deletion, Channel tests, routing, health, and Marketplace visibility use the existing
Provider rules unchanged, including per-model `multiplier_override` edits through
`PUT /api/dashboard/providers/{provider_id}`.

PP-W11. The wholesale create mutation MUST be one database transaction (PP-A5) and MUST
increment `configuration_generation` exactly once (PP-P6).

## 15. Wholesale management UI

PP-WF1. The Provider page MUST offer the `agent` account-class scope alongside the other
classes. On the `agent` scope, the create action MUST open a wholesale Provider dialog
instead of the ordinary Provider editor.

PP-WF2. The wholesale dialog MUST contain, in order: target agent Group select, source
Provider select limited to Providers whose Group class is `standard`, `enterprise`, or
`private` and labelled with the source Group, default wholesale multiplier input, per-model
multiplier table prefilled with the source mapping's effective multipliers and individually
editable, name overrides, and the public-exposure confirmation of PP-F3.

PP-WF2a. When the registry contains no `agent` Group, the wholesale dialog MUST NOT render
an empty Group select as the only path. It MUST offer inline creation through the Group
create API with `account_class = agent`, `is_public = true`, and the PP-N7 public-exposure
confirmation, and MUST select the created Group for the pending wholesale Provider after
the Group is created.

PP-WF3. Data fetching in the wholesale dialog MUST use SWR with a Skeleton fallback; the
submit MUST optimistically insert the Provider row, roll back on failure, and revalidate the
Provider and Marketplace keys (PP-F6).
