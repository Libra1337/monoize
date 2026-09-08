# Marketplace Rehearsal Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove public Marketplace cursors, stable snapshots, source invalidation, response allow-lists, size bounds, and SQLite/PostgreSQL query behavior without registering an HTTP listener.

**Architecture:** Add Marketplace modules to the isolated `rehearsal/` crate. Treat request validation, cursor signing, aggregation, encoding, ETag parsing, token buckets, and source-manifest validation as pure components. Put database queries behind one adapter interface and invoke them directly from tests and a benchmark command.

**Tech Stack:** Rust 2024, SQLx SQLite/PostgreSQL, serde_json, HMAC-SHA256, SHA-256, Tokio, Docker PostgreSQL.

---

### Task 1: Implement canonical public input and signed cursors

**Files:**
- Create: `rehearsal/src/marketplace/input.rs`
- Create: `rehearsal/src/marketplace/cursor.rs`
- Create: `rehearsal/src/marketplace/mod.rs`
- Test: `rehearsal/tests/marketplace_cursor.rs`

- [ ] **Step 1: Write failing cursor tests**

```rust
#[test]
fn cursor_rejects_tampering_filter_and_revision_changes() {
    let key = [7_u8; 32];
    let cursor = ListCursor::new(42, digest("q=gpt"), 3, b"GPT-4o").encode(&key);
    assert!(ListCursor::decode(&cursor, &key, 42, digest("q=gpt")).is_ok());
    assert!(ListCursor::decode(&(cursor + "x"), &key, 42, digest("q=gpt")).is_err());
    assert!(ListCursor::decode(&cursor, &key, 43, digest("q=gpt")).is_err());
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test marketplace_cursor`

Expected: FAIL because cursor types are absent.

- [ ] **Step 3: Implement exact binary cursor layouts**

Implement list and offer cursor payloads from `model-marketplace.spec.md` MM-C1 through MM-C9. Use constant-time HMAC verification. Validate limits 1 through 50, `q` UTF-8 byte length, exact offer model whitespace, and public-name normalization. Return stable rehearsal error codes matching the spec.

- [ ] **Step 4: Run cursor tests**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test marketplace_cursor`

Expected: PASS for tampering, endpoint mismatch, filter mismatch, limit mismatch, maximum key length, ASCII-case search, and non-ASCII literal bytes.

- [ ] **Step 5: Commit**

```text
git add rehearsal/src/marketplace rehearsal/tests/marketplace_cursor.rs
git commit -m "test: rehearse signed Marketplace cursors"
```

### Task 2: Create generation schema and source manifest

**Files:**
- Create: `rehearsal/src/marketplace/generation.rs`
- Create: `rehearsal/fixtures/marketplace/generation-sources.json`
- Test: `rehearsal/tests/marketplace_generation.rs`

- [ ] **Step 1: Write failing trigger delta tests**

```rust
for source in FULL_SOURCES {
    assert_generation_delta(&db, source, Operation::Insert, ExpectedDelta::backend_rows()).await;
    assert_generation_delta(&db, source, Operation::UpdateRelevant, ExpectedDelta::backend_rows()).await;
    assert_generation_delta(&db, source, Operation::Delete, ExpectedDelta::backend_rows()).await;
}
assert_generation_delta(&db, "system_settings", Operation::UnrelatedUpdate, ExpectedDelta::Exact(0)).await;
```

- [ ] **Step 2: Verify failure**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test marketplace_generation`

Expected: FAIL because generation DDL and manifest validation are absent.

- [ ] **Step 3: Implement generation DDL and validation**

Create the singleton and triggers for all six sources. PostgreSQL uses statement-level transition tables plus source-table TRUNCATE triggers. SQLite uses row triggers. Filter `system_settings` to logical changes involving `reasoning_suffix_map`. Validate singleton monotonic updates and forbid deletion.

- [ ] **Step 4: Run both backend generation suites**

Run: `cargo test --manifest-path rehearsal/Cargo.toml marketplace_generation`

Expected: PASS for insert, relevant update, irrelevant update, delete, upsert update, conflict do nothing, rollback, singleton guards, and PostgreSQL TRUNCATE.

- [ ] **Step 5: Commit**

```text
git add rehearsal/src/marketplace/generation.rs rehearsal/fixtures/marketplace/generation-sources.json rehearsal/tests/marketplace_generation.rs
git commit -m "test: qualify Marketplace source generation"
```

### Task 3: Implement set-based Marketplace queries

**Files:**
- Create: `rehearsal/src/marketplace/query.rs`
- Create: `rehearsal/src/marketplace/sqlite.rs`
- Create: `rehearsal/src/marketplace/postgres.rs`
- Test: `rehearsal/tests/marketplace_query.rs`

- [ ] **Step 1: Write backend parity tests**

```rust
let sqlite_pages = collect_list_pages(&sqlite, Query::new("模型", 1)).await.unwrap();
let postgres_pages = collect_list_pages(&postgres, Query::new("模型", 1)).await.unwrap();
assert_eq!(sqlite_pages, postgres_pages);
assert_eq!(sqlite_pages.iter().flat_map(|p| &p.items).count(), expected_distinct_rows);
assert!(sqlite.statement_count() <= fixed_list_statement_bound());
```

- [ ] **Step 2: Verify failure**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test marketplace_query`

Expected: FAIL because query adapters are absent.

- [ ] **Step 3: Implement bounded set queries**

Select at most `limit + 1` Group/model keys, then fetch offers, applicable rates, and reviewed metadata in set-based batches. Use BLOB/BYTEA keyset predicates and database substring containment. Compute Provider prices with exact decimal arithmetic. Never hydrate secret columns. Record statement count and scanned rows.

- [ ] **Step 4: Run parity tests**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test marketplace_query`

Expected: PASS at limits 1, 24, and 50 for first, middle, and final cursors, duplicate names, non-ASCII names, zero matches, one match, 50 matches, and broad matches.

- [ ] **Step 5: Commit**

```text
git add rehearsal/src/marketplace/query.rs rehearsal/src/marketplace/sqlite.rs rehearsal/src/marketplace/postgres.rs rehearsal/tests/marketplace_query.rs
git commit -m "test: rehearse set-based Marketplace queries"
```

### Task 4: Add exact public encoders and 1 MiB bound

**Files:**
- Create: `rehearsal/src/public_contract.rs`
- Create: `rehearsal/src/marketplace/encode.rs`
- Test: `rehearsal/tests/public_contract.rs`

- [ ] **Step 1: Write allow-list and boundary tests**

```rust
let bytes = encode_marketplace(&snapshot).unwrap();
let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
assert_exact_keys(&value, &["generated_at", "revision", "next_cursor", "items"]);
for forbidden in ["api_key", "base_url", "proxy_url", "internal_id", "pricing_profile", "multiplier"] {
    assert!(!String::from_utf8_lossy(&bytes).contains(forbidden));
}
assert_eq!(encode_bounded(exact_1_mib_fixture()).unwrap().len(), 1_048_576);
assert_eq!(encode_bounded(one_item_too_large_fixture()).unwrap_err().code(), "public_response_too_large");
```

- [ ] **Step 2: Verify failure**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test public_contract`

Expected: FAIL because exact response DTOs are absent.

- [ ] **Step 3: Implement DTO-only serialization**

Define response structs containing exactly the keys in MM-L5, MM-O3, PS-S1, and PST-P1. Encode candidates with their final envelope and cursor. Stop before the first item that exceeds 1,048,576 bytes; error when zero items fit.

- [ ] **Step 4: Run public contract tests**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test public_contract`

Expected: PASS with exact keys and no secret-field substrings.

- [ ] **Step 5: Commit**

```text
git add rehearsal/src/public_contract.rs rehearsal/src/marketplace/encode.rs rehearsal/tests/public_contract.rs
git commit -m "test: enforce public Marketplace allow-lists"
```

### Task 5: Add stable snapshot, ETag, and token-bucket components

**Files:**
- Create: `rehearsal/src/public_cache.rs`
- Create: `rehearsal/src/token_bucket.rs`
- Test: `rehearsal/tests/public_cache.rs`
- Test: `rehearsal/tests/token_bucket.rs`

- [ ] **Step 1: Write failing cache and bucket tests**

```rust
assert_eq!(weak_etag(body), format!("W/\"{}\"", sha256_hex(body)));
assert!(if_none_match_matches("W/\"abc\", \"def\"", "W/\"abc\""));
assert!(if_none_match_matches("*", "W/\"abc\""));
assert!(!if_none_match_matches("malformed", "W/\"abc\""));

let mut buckets = Buckets::new(20, 1.0, 10_000, Duration::from_secs(120));
assert_eq!((0..20).filter(|_| buckets.take(ip, now)).count(), 20);
assert!(!buckets.take(ip, now));
```

- [ ] **Step 2: Verify failure**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test public_cache --test token_bucket`

Expected: FAIL because parsers and bucket map are absent.

- [ ] **Step 3: Implement deterministic components**

ETag hashes exact uncompressed bytes. Entity-tag parsing supports lists, weak comparison, wildcard, and ignores malformed input. The bucket refills continuously, shares consumption across endpoint kinds, caps at 10,000 entries, evicts the least-recently-seen idle entry, and rejects a new IP when no idle entry exists.

- [ ] **Step 4: Run component tests**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test public_cache --test token_bucket`

Expected: PASS for identity/compressed validators, stable bytes over time, refill, burst, capacity, idle eviction, and no-idle rejection.

- [ ] **Step 5: Commit**

```text
git add rehearsal/src/public_cache.rs rehearsal/src/token_bucket.rs rehearsal/tests/public_cache.rs rehearsal/tests/token_bucket.rs
git commit -m "test: qualify public cache and rate-limit primitives"
```

### Task 6: Build fixed maximum-envelope fixtures and benchmark command

**Files:**
- Create: `rehearsal/src/marketplace/fixture.rs`
- Create: `rehearsal/src/marketplace/benchmark.rs`
- Create: `rehearsal/fixtures/marketplace/query-set.json`
- Test: `rehearsal/tests/marketplace_fixture.rs`

- [ ] **Step 1: Write deterministic fixture tests**

```rust
let a = FixtureManifest::generate(0x4c594e5348454e, Envelope::SMOKE).unwrap();
let b = FixtureManifest::generate(0x4c594e5348454e, Envelope::SMOKE).unwrap();
assert_eq!(a.sha256, b.sha256);
assert_eq!(a.groups, 8);
assert_eq!(a.providers, 128);
assert_eq!(a.provider_models, 4_096);
```

- [ ] **Step 2: Verify failure**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test marketplace_fixture`

Expected: FAIL because deterministic fixture generation is absent.

- [ ] **Step 3: Implement smoke and qualification envelopes**

Use one fixed seed. `SMOKE` has 8 Groups, 128 Providers, 4,096 mappings, 2,048 distinct model rows, 2,048 metadata rows, 8,192 rates, and 32,768 derived offer-rate entries. `QUALIFICATION` has exactly 128 Groups, 5,000 Providers, 250,000 mappings, 100,000 distinct model rows, 250,000 metadata rows, 1,000,000 rates, and 2,000,000 derived offer-rate entries. Generate one Group/model row with 5,000 Provider offers and more than 256 canonical query cases.

- [ ] **Step 4: Run smoke benchmark**

Run: `cargo run --manifest-path rehearsal/Cargo.toml --bin lynshen-rehearsal -- marketplace benchmark --backend sqlite --envelope smoke --query-set rehearsal/fixtures/marketplace/query-set.json --output rehearsal/evidence/marketplace-sqlite-smoke.json`

Expected: report includes p50/p95/p99, cache hits/misses, statement counts, response bytes, CPU time, and RSS change; every smoke correctness check passes.

- [ ] **Step 5: Commit**

```text
git add rehearsal/src/marketplace/fixture.rs rehearsal/src/marketplace/benchmark.rs rehearsal/fixtures/marketplace/query-set.json rehearsal/tests/marketplace_fixture.rs rehearsal/evidence/marketplace-sqlite-smoke.json
git commit -m "test: add fixed Marketplace benchmark corpus"
```

### Task 7: Run Gate B Marketplace qualification

**Files:**
- Create: `rehearsal/scripts/start-postgres.ps1`
- Create: `rehearsal/scripts/run-marketplace-gate.ps1`
- Create: `rehearsal/evidence/marketplace-gate-summary.json`

- [ ] **Step 1: Add a failing evidence validator**

```rust
#[test]
fn gate_b_requires_both_backends_and_all_bounds() {
    let report = GateReport::load("evidence/marketplace-gate-summary.json").unwrap();
    assert_eq!(report.backends, ["sqlite", "postgres"]);
    assert!(report.read_bounds_passed && report.write_bounds_passed && report.generation_manifest_passed);
}
```

- [ ] **Step 2: Verify the validator fails before qualification**

Run: `cargo test --manifest-path rehearsal/Cargo.toml gate_b_requires_both_backends_and_all_bounds`

Expected: FAIL because no summary exists.

- [ ] **Step 3: Implement reproducible PowerShell runners**

The runner starts PostgreSQL 17 on local port 55432, records image digest and server version, runs ANALYZE and checkpoints, runs each backend with 32 workers, and executes source-write cases at statement sizes 1, 100, 1,000, and 10,000. It records host CPU, storage, OS, database configuration, git commit, fixture and query-set hashes, WAL, checkpoint, lock, memory, and latency metrics.

- [ ] **Step 4: Run the qualification**

Run: `powershell -ExecutionPolicy Bypass -File rehearsal/scripts/run-marketplace-gate.ps1`

Expected: both backends meet MM-Q1 through MM-Q6 and the source-write bounds; summary sets `gate_b_marketplace_passed` to true. Any miss leaves it false and blocks product integration.

- [ ] **Step 5: Commit evidence and runner**

```text
git add rehearsal/scripts rehearsal/evidence/marketplace-*.json
git commit -m "test: record Marketplace Gate B evidence"
```
