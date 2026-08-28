# Hard review: Store retention (PR #3)

**Branch:** `origin/cursor/store-retention-7cea`  
**Tip:** `85a46cc`  
**Scope:** SB-PR-11..14 (`spec/store-billing.spec.md`), `src/store_billing/retention.rs`, `tests/store_retention.rs`, related checkout/migration wiring.

## Verdict: **REQUEST CHANGES**

Core retention mechanics (policy gates, bounded batches, hold exclusion, 3-failure pause/containment, legal-hold extension semantics, scheduler Primary-only wiring) are largely correct and well tested. Two spec-alignment defects in deletion ordering/eligibility should be fixed before merge. Transaction rollback is improved by `6594136` for admin mutations but remains inconsistent on the main deletion path.

---

## Summary matrix

| Area | Spec refs | Result |
|------|-----------|--------|
| Policy gates | SB-PR-11, SB-PR-11A | **Pass** (minor test gap on invalid JSON parse path) |
| Bounded batch limits | SB-PR-11B | **Pass** for per-class 500 cap; **Fail** for financial cross-type ordering |
| Hold extension non-restore | SB-PR-14 | **Pass** (implementation + `legal_hold_expiry_allows_deletion_and_does_not_restore`) |
| 3-failure pause + containment | SB-PR-12, SB-PR-12A/B | **Pass** (one edge-case test gap) |
| Tx rollback (`6594136`) | SB-PR-11B, SB-PR-13/14D | **Partial** — fixed for legal hold/containment; not applied to deletion/failure txs |
| Channels disabled invariants | SB-PR-12A, SB-C-25/26 | **Pass** in `order.rs`; migration does not enable channels |

---

## Policy gates (SB-PR-11)

### Pass

- **Current privacy record at run start:** `run_at` resolves policy before claiming (`retention.rs:247–250`). Missing current record → `policy_version = "unavailable"`, `error_category = privacy_policy_unavailable` (`262`, `575`). Invalid JSON/timestamps/retention shape → `privacy_policy_invalid` (`254–261`, `1651–1657`).
- **Run state machine:** Running/succeeded/failed shapes match SB-PR-11A (`613–668`, `671–720`, migration run rows).
- **Scheduler Primary-only:** `main.rs:26–31` spawns `spawn_daily_retention_job` only when `!is_replica`. Replica mounts mutation router only (`app.rs:2049–2052`); GET retention stays on full dashboard router on Primary.
- **Tests:** `missing_privacy_policy_fails_and_records_audit` (`tests/store_retention.rs:246–271`), `invalid_privacy_retention_document_fails_run` (`774–808`).

### Notes

- `valid_retention` hard-requires policy JSON fields `raw_callback_days == 30`, `network_metadata_days == 90`, `redemption_audit_days == 730` (`retention.rs:1651–1657`), while deletion cutoffs for those three classes are also hardcoded (`1040`, `1077`, `1148`). That matches SB-PR-11C/D fixed periods even if the policy object carries redundant fields.
- No test for malformed `retention_json` that fails deserialization (only wrong numeric values).

---

## Bounded batch limits (SB-PR-11B)

### Pass

- `RETENTION_BATCH_SIZE = 500` (`retention.rs:14`) applied per count class for raw, network, grants, redemption audits.
- Financial class uses one shared 500-root budget across order/event/ledger/refund/claim/settlement/audit tables (`1185–1254`), matching “each count class” (not each table).
- Child rows deleted with a financial root do not increment the root counter (`1247–1252`, `1322–1354`).
- Hold exclusion uses `starts_at <= run_time AND expires_at > run_time` (`1050`, `1087`, etc.), equivalent to SB-PR-11B’s half-open interval.
- Selection and mutation share one transaction in `execute_success` (`620–667`).
- Test `bounded_deletion_caps_each_class_at_batch_size` (`store_retention.rs:811–859`) proves 501 grants → 500 then 1 on the next run.

### Fail — financial root selection order

**Spec:** SB-PR-11B requires candidates in **ascending retention timestamp and ID order** within each class.

**Code:** `delete_financial_records` walks a **fixed table list** (orders → provider events → ledger → …) and exhausts the 500 budget table-by-table (`1188–1254`).

**Impact:** If 500+ eligible `store_orders` exist, older `store_provider_events` / ledger rows with earlier timestamps are skipped until orders drain, violating global timestamp ordering within `financial_records`.

**Example:** Order `2020-01-01`, event `2019-01-01`, both past cutoff — implementation deletes orders first, not the older event.

**Required fix:** Union financial root candidates across tables, order by `(retention_timestamp, id)`, take 500, then delete.

---

## Fail — provider event financial deletion vs network retention

**Spec tension resolved by minimum age:**

- SB-PR-11C: event **row must remain**; network fields cleared only at ≥ 90 days.
- SB-PR-11E: `financial_records` may delete provider events at `financial_records_days`.

**Code:** `delete_financial_records` can delete `store_provider_events` rows whenever `received_at <= now - financial_records_days` (`1196–1200`, `1313–1318`), **without** requiring age ≥ 90 days or cleared network fields.

**Impact:** With `financial_records_days < 90` (allowed by `valid_retention` at `1654`), a 45-day-old event can be row-deleted while `source_ip` / `user_agent` are still present — violating SB-PR-11C’s 90-day network rule and “row MUST remain”.

**Required fix:** Exclude provider events from financial deletion until `received_at <= now - 90 days` (or until network fields are already null and other SB-PR-11C invariants hold).

---

## Hold extension non-restore (SB-PR-14)

### Pass

- Holds are immutable inserts only; extension validates prior hold id/class/identifiers/expiry inside the write tx (`344–353`, `376–404`).
- Expiry is evaluation-time only (`1050`); no hold row mutation on expiry.
- `legal_hold_expiry_allows_deletion_and_does_not_restore` (`store_retention.rs:862–930`):
  - During hold: held event kept, unheld deleted.
  - After hold expiry: held event cleared like any other.
  - New hold on already-deleted identifier: creates hold row but **does not** restore ciphertext (`922–928`).
- Extension rules tested in `self_approval_and_extension_rules_are_enforced` (`607–687`).

### Note

- No test that an **extension** (`extends_hold_id`) after deletion also does not restore data; behavior follows from insert-only holds + no restore path in `apply_retention`.

---

## 3-failure pause + containment (SB-PR-12 / 12A / 12B)

### Pass

- Failure increments counter, does not clear pause (`948–1005`, especially `992`).
- Alert + pause when `failures >= 3 && !paused` in same failure transaction (`966–994`).
- Success resets `consecutive_failures = 0` only; does **not** clear `checkout_paused` (`642–646`) — tested (`store_retention.rs:536–545`).
- Containment: requires paused + active alert; marks alert contained; clears pause; leaves failure count unchanged (`460–518`, `511–517`).
- Checkout gate: `retention_checkout_paused` with row lock (`790–812`); new order rejected before insert (`order.rs:218–223`); mapped to HTTP 503 `store_retention_paused` (`dashboard_handlers/store_billing.rs:559`).
- Idempotent order replay checked **before** pause gate (`order.rs:181–188`, `209–217`).
- Terminal attempt replay returns before pause check (`order.rs:668–681`).
- Test `three_failures_pause_checkout_and_containment_clears_pause_only` (`511–566`), `paused_checkout_rejects_new_orders_before_insert` (`569–604`).

### Notes

- No test for SB-PR-12B: after containment with `consecutive_failures` still ≥ 3, a **fourth** failure creates a **new** alert and re-pauses checkout.
- Interrupted runs increment failures and write failed audits in `claim_run_locked` (`893–917`); covered by `competing_owner_interrupts_active_claim` (`690–738`).

---

## Tx rollback (`6594136`)

### Pass (admin mutations)

Commit `6594136` wraps `create_legal_hold` and `contain` bodies in `async { … }.await` + `finish_transaction` (`342–420`, `460–541`, `1598–1611`). Validation failures (`InvalidInput`, `ContainmentUnavailable`) now roll back instead of leaving partial hold/audit rows.

### Partial — deletion / failure paths

- `execute_success` (`613–668`) and `finalize_failure` (`671–721`) still call `tx.commit()` directly and rely on `WriteTransaction` drop behavior on error paths — **no** `finish_transaction` wrapper.
- `WriteTransaction` has explicit `rollback()` but **no `Drop` rollback** (`src/db/mod.rs:25–44`). SB-PR-11B requires explicit rollback on run failure.
- **Risk:** If `apply_retention` partially executes then a later statement errors, rollback depends on SeaORM/pg/sqlite implicit behavior rather than the explicit pattern established in `6594136`.

**Recommendation:** Wrap `execute_success` and `finalize_failure` bodies in the same `finish_transaction` pattern for spec parity and consistency with `6594136`.

### Test gap

- No regression test asserting invalid legal-hold extension (`earlier` expiry at `store_retention.rs:649–666`) leaves **zero** rows in `store_legal_holds` / `store_legal_hold_items` after the failed request — the fix in `6594136` is untested.

---

## Channels disabled invariants (SB-PR-12A + channel governance)

### Pass

- **New checkout while paused:** Retention pause is checked before channel lock / `enabled = 1` lookup (`order.rs:218–228`, `683–700`), so pause cannot be bypassed via disabled channels.
- **New checkout while unpaused:** Disabled or unavailable channels still return `ChannelUnavailable` / `payment_channel_unavailable` after pause check — governance preserved.
- **Allowed while paused:** Retention admin mutations route through `store_mutation_guard` (Primary + Origin) (`app.rs:2248–2307`); retention runtime does not depend on channel enablement.
- **Migration SB-C-2 preserved:** `m20260828_000058` adds retention tables and reauth scopes only; no `UPDATE store_payment_channels SET enabled = 1` (`migration_054` seeds only `store_retention_state`).
- **Reauth scope migration:** `migration_054_upgrades_reauth_scope_without_losing_existing_grants_or_indexes` verifies legacy grants survive and unknown scopes are rejected (`tests/store_payment_migration.rs:80–145`); retention scopes registered in `reauth.rs:144–145`.

### Note

- No integration test that retention **runs** succeed when all payment channels remain disabled (low risk; retention does not touch channel tables).

---

## Additional line-level findings

### Medium

| Location | Finding |
|----------|---------|
| `retention.rs:1388–1421` | `oldest_remaining_at` scans tables independently with `LIMIT 1` per table, then takes min. Correct minimum, but redundant queries; not a spec violation. |
| `retention.rs:256–258` | Storage errors from `execute_success` collapse to `"storage"` error category — acceptable but loses granularity in run record. |
| `dashboard_handlers/store_billing.rs:1253–1265` | `create_store_legal_hold_admin` relies on middleware for Primary/Origin (OK via `store_mutation_guard`); unlike `run_store_retention_admin`, no redundant in-handler Primary check — acceptable because middleware enforces SB-PR-13A Primary requirement. |

### Low / hygiene

| Location | Finding |
|----------|---------|
| `retention.rs:814–846` | Daily scheduler logs and swallows `RunActive`; spec does not require retry — OK. |
| `tests/store_retention.rs` | 12 tokio tests — good coverage for happy paths and main failure modes; missing Postgres duplicate of SQLite retention tests. |
| Spec on branch | SB-PR-11..14 expanded text matches implementation intent; workspace base branch still has shorter SB-PR-11..14 stubs — ensure PR merges spec expansion. |

---

## Tests reviewed (representative)

| Test | Covers |
|------|--------|
| `missing_privacy_policy_fails_and_records_audit` | SB-PR-11 unavailable policy |
| `invalid_privacy_retention_document_fails_run` | SB-PR-11 invalid policy |
| `run_clears_expired_callback_and_network_fields_idempotently` | SB-PR-11C, idempotency |
| `legal_hold_skips_held_records_and_writes_create_audit` | SB-PR-11B hold exclusion, SB-PR-13A audit |
| `bounded_deletion_caps_each_class_at_batch_size` | SB-PR-11B 500 cap (grants class) |
| `three_failures_pause_checkout_and_containment_clears_pause_only` | SB-PR-12/12B |
| `paused_checkout_rejects_new_orders_before_insert` | SB-PR-12A |
| `legal_hold_expiry_allows_deletion_and_does_not_restore` | SB-PR-14 non-restore |
| `self_approval_and_extension_rules_are_enforced` | SB-PR-13/14 extension |
| `competing_owner_interrupts_active_claim` | SB-PR-11G interrupted |

---

## Required changes before approval

1. **Financial candidate ordering** (`retention.rs:1178–1254`): Select financial roots globally by retention timestamp + id across all SB-PR-11E tables, then delete at most 500.
2. **Provider event deletion floor** (`retention.rs:1196–1200`, `1313–1318`): Do not financially delete event rows younger than the SB-PR-11C 90-day network retention floor (or equivalent invariant).
3. **Explicit run tx rollback** (`retention.rs:613–721`): Apply `finish_transaction` (or equivalent explicit rollback) to success/failure finalization paths for SB-PR-11B parity with `6594136`.

## Recommended (non-blocking)

- Test that invalid hold extension leaves no hold rows (validates `6594136`).
- Test re-pause + new alert after containment when failure count ≥ 3.
- Test idempotent order/terminal-attempt replay while checkout paused.
- Bounded-batch tests for raw/network/financial classes (not only grants).

---

## Conclusion

The PR delivers a substantial, mostly spec-aligned retention subsystem with strong tests for policy failure, holds, pause/containment, and batch limits. **REQUEST CHANGES** because financial-record selection order and provider-event deletion eligibility can violate SB-PR-11B/11C, and run-level transaction rollback should match the explicit pattern introduced in `6594136`.
