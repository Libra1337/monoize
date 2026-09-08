# Provider Migration and Pricing Rehearsal Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build an isolated executable that classifies legacy Providers, transforms each Group × Channel pair, validates the target schema on SQLite and PostgreSQL, and proves exact pricing equality.

**Architecture:** Create a standalone Rust crate under `rehearsal/`. Keep its migration model, transformation logic, and report encoder independent from Monoize startup and HTTP routing. Use SQLx only inside backend adapters; pure transformation and pricing code receives typed snapshots and returns deterministic target rows and canonical evidence.

**Tech Stack:** Rust 2024, SQLx SQLite/PostgreSQL, rust_decimal, serde, SHA-256, UUID, Tokio, Docker PostgreSQL for tests.

---

### Task 1: Create the isolated rehearsal crate

**Files:**
- Create: `rehearsal/Cargo.toml`
- Create: `rehearsal/src/lib.rs`
- Create: `rehearsal/src/bin/lynshen-rehearsal.rs`
- Create: `rehearsal/tests/isolation.rs`

- [ ] **Step 1: Write the failing isolation test**

```rust
#[test]
fn root_application_does_not_register_rehearsal() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let lib = std::fs::read_to_string(root.join("src/lib.rs")).unwrap();
    let migrations = std::fs::read_to_string(root.join("src/migration/mod.rs")).unwrap();
    assert!(!lib.contains("lynshen_rehearsal"));
    assert!(!migrations.contains("lynshen_rehearsal"));
}
```

- [ ] **Step 2: Run the test and verify the crate is absent**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test isolation`

Expected: FAIL because `rehearsal/Cargo.toml` does not exist.

- [ ] **Step 3: Create the crate with CLI-managed dependencies**

Run:

```text
cargo init --lib rehearsal
cargo add --manifest-path rehearsal/Cargo.toml anyhow base64 chrono hex hmac rand rust_decimal serde serde_json sha2 tempfile unicode-normalization uuid
cargo add --manifest-path rehearsal/Cargo.toml sqlx --features runtime-tokio-rustls,sqlite,postgres,chrono,uuid,rust_decimal
cargo add --manifest-path rehearsal/Cargo.toml tokio --features full
```

Add the binary entry point:

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    monoize_lynshen_rehearsal::cli::run(std::env::args_os()).await
}
```

- [ ] **Step 4: Run the isolation test**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test isolation`

Expected: PASS and no change under root `src/`.

- [ ] **Step 5: Commit**

```text
git add rehearsal/Cargo.toml rehearsal/Cargo.lock rehearsal/src rehearsal/tests/isolation.rs
git commit -m "test: isolate LynShen rehearsal crate"
```

### Task 2: Define legacy and target snapshots

**Files:**
- Create: `rehearsal/src/provider/model.rs`
- Create: `rehearsal/src/provider/mod.rs`
- Test: `rehearsal/tests/provider_model.rs`

- [ ] **Step 1: Write serialization tests**

```rust
#[test]
fn canonical_multiplier_rejects_binary_float_shapes() {
    assert_eq!(CanonicalDecimal::parse("1.2000").unwrap().as_str(), "1.2");
    for invalid in ["0", "-1", "+1", "1e0", "NaN", "1.0000000001"] {
        assert!(CanonicalDecimal::parse(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn model_keys_are_exact_and_ascii_folded() {
    let keys = ModelKeys::new("GPT-4o-模型").unwrap();
    assert_eq!(keys.name, b"GPT-4o-\xE6\xA8\xA1\xE5\x9E\x8B");
    assert_eq!(keys.search, b"gpt-4o-\xE6\xA8\xA1\xE5\x9E\x8B");
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test provider_model`

Expected: FAIL because `CanonicalDecimal` and `ModelKeys` are absent.

- [ ] **Step 3: Implement typed snapshots**

Implement these public types without database dependencies:

```rust
pub struct LegacyProvider { pub id: String, pub name: String, pub priority: i32, pub group_ids: Vec<String>, pub channels: Vec<LegacyChannel> }
pub struct LegacyChannel { pub id: String, pub name: String, pub enabled: bool, pub weight: i32, pub models: Vec<LegacyModel> }
pub struct LegacyModel { pub name: String, pub redirect: Option<String>, pub resolved_profile: Option<String>, pub multiplier: CanonicalDecimal }
pub struct TargetProvider { pub id: String, pub source_provider_id: String, pub group_id: String, pub channel: TargetChannel, pub priority: i32, pub pricing_profile: Option<String>, pub multiplier: CanonicalDecimal, pub models: Vec<TargetModel> }
pub struct TargetModel { pub name: String, pub redirect: Option<String>, pub pricing: PricingMode, pub multiplier_override: Option<CanonicalDecimal> }
pub enum PricingMode { Inherit, Override(String), Unpriced }
```

`CanonicalDecimal::parse` MUST use `rust_decimal::Decimal`, require a positive value and at most nine input fractional digits, and serialize normalized non-exponent text. `ModelKeys::new` MUST trim Unicode White_Space, validate 1 through 256 UTF-8 bytes, and ASCII-fold only bytes `A` through `Z`.

- [ ] **Step 4: Run focused tests**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test provider_model`

Expected: PASS.

- [ ] **Step 5: Commit**

```text
git add rehearsal/src/provider rehearsal/tests/provider_model.rs
git commit -m "test: model Provider migration snapshots"
```

### Task 3: Implement deterministic classification and expansion

**Files:**
- Create: `rehearsal/src/provider/transform.rs`
- Test: `rehearsal/tests/provider_transform.rs`

- [ ] **Step 1: Write failing route-safe and semantic-change tests**

```rust
#[test]
fn expands_in_stored_group_and_channel_order() {
    let source = fixture_provider(&["g-b", "g-a"], &["c-2", "c-1"]);
    let result = transform_provider(&source).unwrap();
    assert_eq!(result.classification, Classification::SemanticChange);
    assert_eq!(result.targets.iter().map(|p| (&p.group_id, &p.channel.id)).collect::<Vec<_>>(), vec![(&"g-b".into(), &"c-2".into()), (&"g-b".into(), &"c-1".into()), (&"g-a".into(), &"c-2".into()), (&"g-a".into(), &"c-1".into())]);
    assert_eq!(result.targets[0].id, source.id);
}

#[test]
fn blocks_zero_group_and_zero_channel_sources() {
    assert_eq!(transform_provider(&fixture_provider(&[], &["c-1"])).unwrap_err().code(), "provider_has_no_group");
    assert_eq!(transform_provider(&fixture_provider(&["g-a"], &[])).unwrap_err().code(), "provider_has_no_channel");
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test provider_transform`

Expected: FAIL because `transform_provider` is absent.

- [ ] **Step 3: Implement the transformation**

Implement:

```rust
pub enum Classification { RouteSafe, SemanticChange }
pub struct TransformResult { pub classification: Classification, pub targets: Vec<TargetProvider> }
pub fn transform_provider(source: &LegacyProvider) -> Result<TransformResult, TransformError>;
pub fn deterministic_id(kind: &str, provider_id: &str, group_id: &str, channel_id: &str) -> String;
pub fn infer_default_profile(models: &[LegacyModel]) -> Option<String>;
pub fn infer_default_multiplier(models: &[LegacyModel]) -> CanonicalDecimal;
```

Use SHA-256 over length-prefixed UTF-8 fields and format the first 16 digest bytes as a lowercase UUID with RFC 4122 variant and version 5 bits. Resolve frequency ties by exact UTF-8 byte order. Preserve the old Provider ID for the first pair. Use `inherit` only on exact Profile equality; otherwise use `override` or `unpriced`. Store a multiplier override only when it differs from the inferred default.

- [ ] **Step 4: Run transformation tests**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test provider_transform`

Expected: PASS, including deterministic repeated execution and tie cases.

- [ ] **Step 5: Commit**

```text
git add rehearsal/src/provider/transform.rs rehearsal/tests/provider_transform.rs
git commit -m "test: rehearse deterministic Provider expansion"
```

### Task 4: Add migration preflight reports and fingerprints

**Files:**
- Create: `rehearsal/src/provider/preflight.rs`
- Create: `rehearsal/fixtures/provider/public-name-manifest.json`
- Test: `rehearsal/tests/provider_preflight.rs`

- [ ] **Step 1: Write failing evidence tests**

```rust
#[test]
fn report_contains_no_secret_material() {
    let report = build_report(&legacy_fixture_with_secret("sk-secret")).unwrap();
    let json = canonical_json(&report).unwrap();
    assert!(!json.contains("sk-secret"));
    assert!(json.contains("source_fingerprint"));
    assert!(json.contains("semantic_change"));
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test provider_preflight`

Expected: FAIL because the report builder is absent.

- [ ] **Step 3: Implement canonical evidence**

Implement `PreflightReport` with source fingerprint, envelope counts, blockers, semantic-change rows, old attempt order, target identifiers, and public-name manifest entries. Hash secrets as part of the fingerprint but never serialize secret values. Canonical JSON MUST sort object keys and terminate with one LF.

- [ ] **Step 4: Run report tests**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test provider_preflight`

Expected: PASS and fixture report contains zero secrets or Base URLs.

- [ ] **Step 5: Commit**

```text
git add rehearsal/src/provider/preflight.rs rehearsal/fixtures/provider rehearsal/tests/provider_preflight.rs
git commit -m "test: emit Provider migration preflight evidence"
```

### Task 5: Rehearse target DDL on SQLite and PostgreSQL

**Files:**
- Create: `rehearsal/src/provider/schema.rs`
- Create: `rehearsal/src/provider/sqlite.rs`
- Create: `rehearsal/src/provider/postgres.rs`
- Create: `rehearsal/tests/provider_schema_sqlite.rs`
- Create: `rehearsal/tests/provider_schema_postgres.rs`

- [ ] **Step 1: Write backend contract tests**

```rust
async fn assert_target_contract(db: &dyn RehearsalDb) {
    db.insert_valid_target().await.unwrap();
    assert!(db.insert_duplicate_provider_model().await.is_err());
    assert!(db.insert_mismatched_model_key().await.is_err());
    assert!(db.insert_duplicate_public_provider_name().await.is_err());
    assert!(!db.table_exists("monoize_channels").await.unwrap());
    assert!(!db.table_exists("monoize_channel_models").await.unwrap());
}
```

- [ ] **Step 2: Verify SQLite failure**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test provider_schema_sqlite`

Expected: FAIL because target DDL is absent.

- [ ] **Step 3: Implement backend-specific transactional DDL**

Create `monoize_providers`, `monoize_provider_models`, and required public-name columns and indexes from `provider-pricing.spec.md`. SQLite keys use BLOB; PostgreSQL keys use BYTEA. Each backend migration must create new tables, copy validated target rows, validate counts and foreign keys, drop legacy tables, and rename target tables in one transaction.

- [ ] **Step 4: Run both database suites**

Run:

```text
cargo test --manifest-path rehearsal/Cargo.toml --test provider_schema_sqlite
$env:LYNSHEN_REHEARSAL_POSTGRES_URL='postgres://postgres:postgres@127.0.0.1:55432/lynshen_rehearsal'; cargo test --manifest-path rehearsal/Cargo.toml --test provider_schema_postgres
```

Expected: PASS on SQLite and PostgreSQL. The PostgreSQL test may skip only when the URL is absent during local unit runs; Gate B requires it to be present.

- [ ] **Step 5: Commit**

```text
git add rehearsal/src/provider/schema.rs rehearsal/src/provider/sqlite.rs rehearsal/src/provider/postgres.rs rehearsal/tests/provider_schema_*.rs
git commit -m "test: validate Provider target schema on both databases"
```

### Task 6: Prove rollback and repeatability

**Files:**
- Test: `rehearsal/tests/provider_migration_failures.rs`

- [ ] **Step 1: Add failure-point tests**

```rust
for point in [FailurePoint::AfterSchema, FailurePoint::AfterProviders, FailurePoint::AfterModels, FailurePoint::BeforeLegacyDrop] {
    let before = database_fingerprint(&db).await.unwrap();
    assert!(migrate_with_failure(&db, point).await.is_err());
    assert_eq!(database_fingerprint(&db).await.unwrap(), before);
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test provider_migration_failures`

Expected: FAIL before failure injection exists.

- [ ] **Step 3: Add injection and target-schema no-op detection**

Implement named failure points inside the transaction. Detect the complete target schema before any write. A second run against the target schema returns `AlreadyMigrated` and leaves database bytes and logical fingerprint unchanged.

- [ ] **Step 4: Run both backend failure suites**

Run: `cargo test --manifest-path rehearsal/Cargo.toml provider_migration_failures`

Expected: PASS for every failure point and second-run no-op case.

- [ ] **Step 5: Commit**

```text
git add rehearsal/tests/provider_migration_failures.rs rehearsal/src/provider
git commit -m "test: prove Provider migration rollback and idempotency"
```

### Task 7: Implement pricing golden snapshots

**Files:**
- Create: `rehearsal/src/pricing.rs`
- Create: `rehearsal/fixtures/pricing/scenarios.json`
- Test: `rehearsal/tests/pricing_golden.rs`

- [ ] **Step 1: Write exact equality tests**

```rust
#[test]
fn aggregate_is_scaled_and_truncated_once() {
    let rates = [LineItem::new(1, 1), LineItem::new(1, 1)];
    assert_eq!(charge(&rates, CanonicalDecimal::parse("1.5").unwrap()).unwrap(), 3);
}

#[test]
fn pre_and_post_snapshots_are_byte_equal() {
    let fixture = load_scenarios();
    assert_eq!(snapshot_legacy(&fixture).unwrap(), snapshot_target(&fixture).unwrap());
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test pricing_golden`

Expected: FAIL because canonical charge and snapshot functions are absent.

- [ ] **Step 3: Implement checked integer pricing**

Represent quantities and unit prices as `u64`, subtotals and sums as checked `u128`, and multiplier coefficients as exact decimal mantissa/scale. Compute `floor(base * mantissa / 10^scale)` once. Reject overflow. Snapshot rows include opaque mapping digest, logical and upstream model, usage class, quantity, unit rate, multiplier, base charge, and final charge; sort rows by digest and scenario ID.

- [ ] **Step 4: Run golden suite**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test pricing_golden`

Expected: PASS with byte-identical canonical JSON for Profile ties, multiplier ties, redirects, cache and meter classes, missing rates, and maximum values.

- [ ] **Step 5: Commit**

```text
git add rehearsal/src/pricing.rs rehearsal/fixtures/pricing rehearsal/tests/pricing_golden.rs
git commit -m "test: prove exact pricing migration equality"
```

### Task 8: Add CLI evidence commands

**Files:**
- Create: `rehearsal/src/cli.rs`
- Create: `rehearsal/evidence/.gitkeep`
- Modify: `rehearsal/src/lib.rs`
- Test: `rehearsal/tests/cli_provider.rs`

- [ ] **Step 1: Write CLI contract tests**

```rust
assert_success(&["provider", "preflight", "--database", sqlite_url, "--output", report]);
assert_success(&["provider", "migrate-copy", "--database", sqlite_copy_url, "--output", migration_report]);
assert_success(&["pricing", "compare", "--before", before, "--after", after, "--output", pricing_report]);
```

- [ ] **Step 2: Verify failure**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test cli_provider`

Expected: FAIL because commands are absent.

- [ ] **Step 3: Implement explicit commands**

Commands MUST require an output path under `rehearsal/evidence/`, refuse production host strings and `/opt/monoize`, never mutate the input to `preflight`, and require `migrate-copy` input to be a copy. Evidence includes tool version, git commit, database backend/version, source fingerprint, timestamps, and result.

- [ ] **Step 4: Run the complete Provider and pricing rehearsal**

Run:

```text
cargo test --manifest-path rehearsal/Cargo.toml --all-targets
cargo run --manifest-path rehearsal/Cargo.toml --bin lynshen-rehearsal -- provider preflight --database sqlite:rehearsal/fixtures/provider/production-redacted.db --output rehearsal/evidence/provider-preflight.json
cargo run --manifest-path rehearsal/Cargo.toml --bin lynshen-rehearsal -- pricing compare --before rehearsal/evidence/pricing-before.json --after rehearsal/evidence/pricing-after.json --output rehearsal/evidence/pricing-comparison.json
```

Expected: tests PASS; reports end with `"gate_b_candidate":true` and `"gate_c_candidate":true` only when every invariant passes.

- [ ] **Step 5: Commit**

```text
git add rehearsal/src/cli.rs rehearsal/src/lib.rs rehearsal/evidence rehearsal/tests/cli_provider.rs
git commit -m "test: emit Provider and pricing rehearsal evidence"
```
