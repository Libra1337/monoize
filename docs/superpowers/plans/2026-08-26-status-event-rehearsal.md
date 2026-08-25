# Status Event Rehearsal Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove the physical-dispatch boundary, checked spool capacity, durable one-file event publication, replay, incomplete-data behavior, Replica shipment, and event conservation in isolation.

**Architecture:** Add a status harness to the standalone `rehearsal/` crate. Separate capacity math, filesystem accounting, state-file publication, event spool, database sink, and source aggregation. Fault injection is explicit and deterministic; no module is imported by the Monoize application during Phase 1.

**Tech Stack:** Rust 2024, Tokio, SQLx, serde_json, tempfile, OS filesystem metadata APIs, Docker PostgreSQL.

---

### Task 1: Implement checked capacity math

**Files:**
- Create: `rehearsal/src/status/capacity.rs`
- Create: `rehearsal/src/status/mod.rs`
- Test: `rehearsal/tests/status_capacity.rs`

- [ ] **Step 1: Write exact-boundary tests**

```rust
let input = CapacityInput { peak_events_per_second: 100, outage_seconds: 900, safety_milli: 1200, max_in_flight: 1024, entry_max_bytes: 4096, allocation_unit: 4096 };
let required = calculate(input).unwrap();
assert_eq!(required.outage_slots, 108_000);
assert_eq!(required.minimum_slots, 109_024);
assert_eq!(required.minimum_bytes, 446_562_304);
assert!(validate_quota(required.minimum_bytes, required).is_ok());
assert!(validate_quota(required.minimum_bytes - 1, required).is_err());
```

- [ ] **Step 2: Verify failure**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test status_capacity`

Expected: FAIL because capacity math is absent.

- [ ] **Step 3: Implement checked formulas**

Use `u64::checked_*` for parsing, rounding, products, sums, and subtraction. Reject peak zero, outage below 900, safety below 1200, in-flight zero, entry maximum below 1024, allocation unit zero, overflow, configured values below approved node values, and quota below minimum.

- [ ] **Step 4: Run capacity tests**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test status_capacity`

Expected: PASS for exact boundary, one byte below, allocation round-up, invalid configuration, and overflow.

- [ ] **Step 5: Commit**

```text
git add rehearsal/src/status rehearsal/tests/status_capacity.rs
git commit -m "test: prove checked status spool capacity"
```

### Task 2: Implement filesystem accounting and admission

**Files:**
- Create: `rehearsal/src/status/filesystem.rs`
- Test: `rehearsal/tests/status_filesystem.rs`

- [ ] **Step 1: Write failing accounting tests**

```rust
let scan = scan_spool(temp.path(), reservation).unwrap();
assert_eq!(scan.accounted_bytes, expected_allocated_blocks);
assert!(scan.temporary_events.is_empty());
assert_eq!(admission(scan, capacity, fs_capacity).unwrap(), Admission::Ready);
```

Add cases for a symlink, non-regular entry, temporary event, allocated size above logical length, insufficient free bytes, insufficient file slots, and unknown filesystem capacity.

- [ ] **Step 2: Verify failure**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test status_filesystem`

Expected: FAIL because allocation and file-slot probes are absent.

- [ ] **Step 3: Implement platform probes**

On Windows, read allocated bytes with `GetCompressedFileSizeW` and volume capacity with `GetDiskFreeSpaceExW`; record the filesystem allocation unit with `GetDiskFreeSpaceW`. On Unix, use `stat`, `statvfs`, and allocated block counts. Return an explicit capacity-query error when file-slot capacity is unavailable or ambiguous. Provide an injectable `FilesystemProbe` for deterministic fault tests.

- [ ] **Step 4: Run filesystem tests**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test status_filesystem`

Expected: PASS; actual allocation probe creates, syncs, measures, renames, directory-syncs, and deletes one non-sparse file.

- [ ] **Step 5: Commit**

```text
git add rehearsal/src/status/filesystem.rs rehearsal/tests/status_filesystem.rs
git commit -m "test: measure status spool filesystem capacity"
```

### Task 3: Implement durable event publication and state latch

**Files:**
- Create: `rehearsal/src/status/event.rs`
- Create: `rehearsal/src/status/spool.rs`
- Create: `rehearsal/src/status/state.rs`
- Test: `rehearsal/tests/status_spool.rs`

- [ ] **Step 1: Write crash-boundary tests**

```rust
for point in [CrashPoint::BeforeRename, CrashPoint::AfterRename, CrashPoint::AfterDirectorySync] {
    let result = publish_with_crash(&spool, event.clone(), point);
    let recovered = Spool::recover(spool.path(), config.clone()).unwrap();
    assert_conserved(event.id(), result, recovered);
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test status_spool`

Expected: FAIL because durable spool publication is absent.

- [ ] **Step 3: Implement one-file publication**

Build event IDs as `{source_node_id}.{uuid}.{dispatch_index}`. Encode at most the configured logical bytes. Reserve one allocation unit-rounded entry and file slot before dispatch. Publish with exclusive temporary create, write, file sync, same-directory rename, and directory sync. Publish node state with the same protocol. Recover final files; treat temporary event and state files according to PST-Q12 and PST-Q13.

- [ ] **Step 4: Run spool tests**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test status_spool`

Expected: PASS for crash points, oversized events, over-allocation, quota exhaustion, unwritable directory, directory-sync failure, and clean/unclean shutdown state.

- [ ] **Step 5: Commit**

```text
git add rehearsal/src/status/event.rs rehearsal/src/status/spool.rs rehearsal/src/status/state.rs rehearsal/tests/status_spool.rs
git commit -m "test: rehearse durable status event publication"
```

### Task 4: Prove global physical-dispatch concurrency

**Files:**
- Create: `rehearsal/src/status/dispatch.rs`
- Test: `rehearsal/tests/status_dispatch.rs`

- [ ] **Step 1: Write mixed-path concurrency tests**

```rust
let gate = DispatchGate::new(8, spool.clone());
run_mixed_dispatches(&gate, 64, [Path::HttpInitial, Path::HttpRetry, Path::FailForward, Path::WebSocket]).await;
assert_eq!(gate.maximum_observed(), 8);
assert_eq!(spool.outstanding_reservations(), 0);
```

- [ ] **Step 2: Verify failure**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test status_dispatch`

Expected: FAIL because the gate is absent.

- [ ] **Step 3: Implement one fair semaphore and reservation guard**

Acquire one Tokio semaphore permit before spool reservation. Hold permit and reservation until a terminal outcome publishes or releases. Cancellation while waiting creates neither. Reservation failure records a future counted outcome in the loss ledger without changing the synthetic request result.

- [ ] **Step 4: Run dispatch tests**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test status_dispatch`

Expected: PASS for all path types, cancellation, excluded outcomes, counted outcomes, and persistence failures.

- [ ] **Step 5: Commit**

```text
git add rehearsal/src/status/dispatch.rs rehearsal/tests/status_dispatch.rs
git commit -m "test: bound all physical status dispatches"
```

### Task 5: Implement idempotent database drain and Replica shipment

**Files:**
- Create: `rehearsal/src/status/sink.rs`
- Create: `rehearsal/src/status/shipment.rs`
- Test: `rehearsal/tests/status_replay.rs`

- [ ] **Step 1: Write replay and ambiguous-commit tests**

```rust
sink.commit_then_return_ambiguous_once();
batcher.flush().await.unwrap_err();
batcher.flush().await.unwrap();
assert_eq!(sink.unique_event_count().await.unwrap(), generated_ids.len());
assert_eq!(spool.final_file_count().unwrap(), 0);
```

- [ ] **Step 2: Verify failure**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test status_replay`

Expected: FAIL because sink and shipment are absent.

- [ ] **Step 3: Implement bounded batches and heartbeat state**

Insert at most 100 events per transaction with conflict-ignore by event ID. Delete files only after definite commit. Retain on failure or ambiguity. Replica shipment retains files until synthetic Primary HTTP 200, applies duplicate batches idempotently, and persists heartbeat fields and retirement transitions.

- [ ] **Step 4: Run replay tests**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test status_replay`

Expected: PASS for database outage, ambiguous commit, Primary restart, Replica restart, four concurrent duplicate senders, retirement, and clock skew.

- [ ] **Step 5: Commit**

```text
git add rehearsal/src/status/sink.rs rehearsal/src/status/shipment.rs rehearsal/tests/status_replay.rs
git commit -m "test: prove status replay and Replica shipment"
```

### Task 6: Implement status aggregation and stable buckets

**Files:**
- Create: `rehearsal/src/status/aggregate.rs`
- Test: `rehearsal/tests/status_aggregate.rs`

- [ ] **Step 1: Write threshold tests**

```rust
assert_eq!(state(Recent { successes: 1, failures: 0, latest_age: 60 }), State::Operational);
assert_eq!(state(Recent { successes: 99, failures: 1, latest_age: 60 }), State::Degraded);
assert_eq!(state(Recent { successes: 0, failures: 1, latest_age: 60 }), State::Major);
assert_eq!(state(Recent { successes: 0, failures: 0, latest_age: 1_801 }), State::Unavailable);
```

- [ ] **Step 2: Verify failure**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test status_aggregate`

Expected: FAIL because aggregation is absent.

- [ ] **Step 3: Implement exact public aggregation**

Aggregate only enabled current Providers and embedded Channels by current public names. Compute `data_through`, 30-minute state, 24-hour success rate, Group worst state, and `data_complete` from all active sources. Freeze one exact response body per 15-second UTC bucket.

- [ ] **Step 4: Run aggregation tests**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test status_aggregate`

Expected: PASS for each outcome classifier, boundary timestamp, zero samples, deleted Provider events, incomplete source, pending source, retired source, clock skew, stable body, and next bucket.

- [ ] **Step 5: Commit**

```text
git add rehearsal/src/status/aggregate.rs rehearsal/tests/status_aggregate.rs
git commit -m "test: qualify public Provider status aggregation"
```

### Task 7: Build deterministic fault and load harness

**Files:**
- Create: `rehearsal/src/status/harness.rs`
- Create: `rehearsal/fixtures/status/profiles.json`
- Create: `rehearsal/scripts/run-status-gate.ps1`
- Test: `rehearsal/tests/status_conservation.rs`

- [ ] **Step 1: Write conservation validator**

```rust
assert_eq!(report.generated_count, report.committed_unique + report.durable_final + report.lost_ledger);
assert_eq!(report.unaccounted, 0);
assert_eq!(report.duplicate_commits, 0);
assert!(report.max_dispatches <= report.configured_dispatches);
```

- [ ] **Step 2: Verify failure before a report exists**

Run: `cargo test --manifest-path rehearsal/Cargo.toml --test status_conservation`

Expected: FAIL because the profile report is absent.

- [ ] **Step 3: Implement scaled smoke and full profiles**

The JSON defines Primary-only database outage, five-source database outage, and five-source shipment outage. Smoke mode uses 60 seconds with outage seconds 10 through 40. Gate mode uses 30 minutes with outage minutes 5 through 20 and continuing input through minute 30. Inject all five crash points and every separate failure from PST-G8.

- [ ] **Step 4: Run Gate D qualification**

Run: `powershell -ExecutionPolicy Bypass -File rehearsal/scripts/run-status-gate.ps1 -Mode Gate`

Expected: zero unaccounted events, zero loss in main profiles, recovery throughput at least 125 percent of input for each complete five-minute window, final drain within 15 minutes, per-process RSS increase at most 256 MiB, aggregate at most 768 MiB, and every injected loss accompanied by `data_complete=false`.

- [ ] **Step 5: Commit evidence**

```text
git add rehearsal/src/status/harness.rs rehearsal/fixtures/status rehearsal/scripts/run-status-gate.ps1 rehearsal/tests/status_conservation.rs rehearsal/evidence/status-*.json
git commit -m "test: record status Gate D conservation evidence"
```
