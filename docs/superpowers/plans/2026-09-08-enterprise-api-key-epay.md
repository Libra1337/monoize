# Enterprise Accounts, API Key Analytics, And EPay Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add isolated Enterprise routing and pricing, API Key usage analytics, and one CNY-only EPay adapter that replaces the old Alipay and WeChat adapter kinds.

**Architecture:** Add `account_class` to users and Groups, derive routing-resource class through the Group, and enforce class filtering at shared access boundaries. Aggregate API Key analytics from immutable request logs. Implement EPay behind the existing Store adapter, order, state-machine, fulfillment, refund, and reconciliation interfaces.

**Tech Stack:** Rust, Axum, SeaORM/SeaORM Migration, SQLite/PostgreSQL SQL, React, TypeScript, SWR, Bun, Vitest.

---

### Task 1: Freeze Observable Specifications

**Files:**
- Modify: `spec/groups-registry.spec.md`
- Modify: `spec/database-provider-routing.spec.md`
- Modify: `spec/model-marketplace.spec.md`
- Modify: `spec/provider-pricing.spec.md`
- Modify: `spec/api-token-management.spec.md`
- Modify: `spec/api-key-sub-account-billing.spec.md`
- Modify: `spec/store-billing.spec.md`

- [ ] Add testable `standard|enterprise` user and Group invariants, cross-class rejection, destructive Key deletion, and no-fallback rules.
- [ ] Add exact API Key analytics ranges, totals, bucketing, ordering, authorization, and balance presentation.
- [ ] Replace the Store adapter contract `alipay|wechat|stripe|http` with `epay|stripe|http`; specify EPay CNY-only creation, signing, callback, query, refund, and reconciliation.
- [ ] Run `rg -n "account_class|api key analytics|epay" spec` and verify every design section maps to one specification.

### Task 2: Add Account-Class Schema And Models

**Files:**
- Create: `src/migration/m20260908_000063_enterprise_account_class.rs`
- Modify: `src/migration/mod.rs`
- Modify: `src/entity/users.rs`
- Modify: `src/entity/monoize_groups.rs`
- Modify: `src/users/mod.rs`
- Modify: `src/users/groups.rs`
- Test: `tests/enterprise_account_class.rs`

- [ ] Write a failing SQLite migration test that asserts `users.account_class`, `monoize_groups.account_class`, allowed values, default `standard`, the request-log analytics index, and an account-class audit table.
- [ ] Run `cargo test --test enterprise_account_class migration_ -- --nocapture`; expect failure because migration 063 is absent.
- [ ] Add migration 063 and register it after migration 062. Add typed `AccountClass` parsing/serialization and model fields.
- [ ] Run the targeted migration tests; expect pass.

### Task 3: Implement Atomic User-Class Switching

**Files:**
- Modify: `src/users/store.rs`
- Modify: `src/dashboard_handlers/users.rs`
- Modify: `src/dashboard_handlers/auth.rs`
- Modify: `src/app.rs`
- Test: `tests/enterprise_account_class.rs`

- [ ] Write failing tests for Admin authorization, required destructive confirmation, API Key balance settlement, deletion of every Key, preserved wallet/plan/log rows, audit creation, and transaction rollback.
- [ ] Run the exact switching tests; expect endpoint or method absence.
- [ ] Add `PUT /api/dashboard/users/{id}/account-class` with exact JSON `{ "account_class": "enterprise", "confirm_delete_api_keys": true }`. Reuse the existing batch Key-deletion settlement path inside one transaction and invalidate affected caches after commit.
- [ ] Return `account_class` from session and Admin user responses. Run the targeted tests; expect pass.

### Task 4: Enforce Enterprise Catalog And Routing Isolation

**Files:**
- Modify: `src/users/groups.rs`
- Modify: `src/dashboard_handlers/groups.rs`
- Modify: `src/dashboard_handlers/providers.rs`
- Modify: `src/public_handlers.rs`
- Modify: `src/handlers/routing.rs`
- Modify: `src/monoize_routing.rs`
- Modify: `src/model_registry_store.rs`
- Modify: `src/billing_rate_store.rs`
- Test: `tests/enterprise_account_class.rs`

- [ ] Write failing tests that create overlapping standard and Enterprise model names and prove Marketplace, Group access, Provider selection, Channel binding, quote, and charge never cross classes.
- [ ] Run targeted isolation tests; expect standard data leakage before implementation.
- [ ] Thread authenticated `AccountClass` through the shared Group-access predicate, catalog query, routing candidate query, and pricing lookup. Reject cross-class Admin writes.
- [ ] Run targeted tests; expect pass.

### Task 5: Add Enterprise Admin And Dashboard UI

**Files:**
- Modify: `frontend/src/pages/users.tsx`
- Modify: `frontend/src/pages/groups.tsx`
- Modify: `frontend/src/pages/providers.tsx`
- Modify: `frontend/src/pages/model-marketplace.tsx`
- Modify: `frontend/src/pages/dashboard.tsx`
- Modify: `frontend/src/components/user-center-menu.tsx`
- Modify: `frontend/src/locales/en.json`
- Modify: `frontend/src/locales/zh.json`
- Modify: `frontend/src/locales/zh-TW.json`
- Modify: `frontend/src/locales/ja.json`
- Test: `frontend/tests/enterprise-account-class.test.tsx`

- [ ] Write failing component tests for scope controls, destructive confirmation text, Enterprise navigation, and omission of standard-only Dashboard modules.
- [ ] Run `cd frontend; bun test tests/enterprise-account-class.test.tsx`; expect failure.
- [ ] Add SWR-backed scope controls and class-switch mutation with optimistic state and rollback. Render the simpler Enterprise navigation from the session account class.
- [ ] Run the targeted frontend test; expect pass.

### Task 6: Add API Key Analytics Backend

**Files:**
- Modify: `src/users/request_logs.rs`
- Modify: `src/users/mod.rs`
- Modify: `src/dashboard_handlers/api_keys.rs`
- Modify: `src/app.rs`
- Test: `tests/api_key_analytics.rs`

- [ ] Write failing tests for ownership, Admin inspection, four ranges, hourly/daily/monthly buckets, token fields, recorded-charge sums, model ordering, independent balance, and wallet-following balance.
- [ ] Run `cargo test --test api_key_analytics -- --nocapture`; expect 404 or missing API.
- [ ] Add `GET /api/dashboard/tokens/{id}/analytics?range=24h|7d|30d|all` and a set-based aggregation implementation over request logs.
- [ ] Run the targeted tests; expect pass.

### Task 7: Add API Key Analytics Modal

**Files:**
- Modify: `frontend/src/pages/api-keys.tsx`
- Modify: `frontend/src/lib/api.ts`
- Modify: `frontend/src/locales/en.json`
- Modify: `frontend/src/locales/zh.json`
- Modify: `frontend/src/locales/zh-TW.json`
- Modify: `frontend/src/locales/ja.json`
- Test: `frontend/tests/api-key-analytics.test.tsx`

- [ ] Write failing tests for modal opening, skeletons, range selection, stable stale values, Key balance label, token totals, trend, and model rows.
- [ ] Run the targeted Bun test; expect failure.
- [ ] Add typed SWR data loading and a responsive dialog. Reuse the existing slow numeric transition and chart motion patterns without resetting values to zero.
- [ ] Run the targeted frontend test; expect pass.

### Task 8: Implement EPay Protocol Adapter

**Files:**
- Create: `src/store_billing/adapters/epay.rs`
- Modify: `src/store_billing/adapters/mod.rs`
- Modify: `src/store_billing/models.rs`
- Modify: `src/store_billing/credentials.rs`
- Modify: `Cargo.toml`
- Test: `src/store_billing/adapters/epay.rs`

- [ ] Write fixed tests for credential validation, ASCII canonicalization, empty-field exclusion, lowercase MD5, two-decimal fen formatting, mapi form creation, callback verification, and response parsing.
- [ ] Run `cargo test store_billing::adapters::epay::tests -- --nocapture`; expect module absence.
- [ ] Add exact EPay types and pure protocol functions. Use the pinned RustCrypto `md-5` crate version recorded in Cargo.lock after `cargo add md-5@0.10.6`.
- [ ] Run protocol tests; expect pass.

### Task 9: Integrate EPay Checkout, Callback, Query, And Refund

**Files:**
- Modify: `src/store_billing/checkout.rs`
- Modify: `src/store_billing/callbacks.rs`
- Modify: `src/store_billing/webhooks.rs`
- Modify: `src/store_billing/reconciliation.rs`
- Modify: `src/store_billing/refund_operations.rs`
- Modify: `src/store_billing/availability.rs`
- Modify: `src/app.rs`
- Test: `tests/store_epay.rs`

- [ ] Write failing integration tests for CNY-only creation, method snapshot, QR, pay URL, GET callback, duplicate fulfillment, mismatch review, query-before-retry, and full refund.
- [ ] Run the EPay integration test; expect unsupported adapter.
- [ ] Bind EPay to the existing Store HTTP client and state machine. Add the callback route and reuse immutable attempts, idempotent ledger fulfillment, and reconciler leases.
- [ ] Run EPay integration and existing Store tests; expect pass.

### Task 10: Migrate Payment Kinds And Update Store UI

**Files:**
- Create: `src/migration/m20260908_000064_epay_adapter.rs`
- Modify: `src/migration/mod.rs`
- Modify: `frontend/src/lib/store-api.ts`
- Modify: `frontend/src/pages/store/payment-methods.tsx`
- Modify: `frontend/src/pages/store/store-selection.ts`
- Modify: `frontend/src/pages/store/index.tsx`
- Modify: `frontend/src/pages/store-admin/channel-dialog.tsx`
- Modify: `frontend/src/pages/store-admin/admin-panels.tsx`
- Modify: `frontend/src/pages/store-admin/governance-dialogs.tsx`
- Modify: `frontend/src/pages/store-admin/governance-state.ts`
- Modify: `frontend/src/locales/en.json`
- Modify: `frontend/src/locales/zh.json`
- Modify: `frontend/src/locales/zh-TW.json`
- Modify: `frontend/src/locales/ja.json`
- Test: `frontend/tests/store-epay.test.tsx`

- [ ] Write failing migration and frontend tests that permit only `epay|stripe|http`, render separate Alipay/WeChat EPay methods, enforce CNY, and build the EPay credential payload.
- [ ] Run targeted Rust and Bun tests; expect failures on old adapter kinds.
- [ ] Add migration 064 to convert legacy official Channels into one disabled EPay draft. Replace old Store UI branches with EPay method configuration and QR/URL behavior.
- [ ] Run targeted tests; expect pass.

### Task 11: Documentation And Release Verification

**Files:**
- Modify affected pages under: `docs/en/`, `docs/zh/`, `docs/zh-TW/`, `docs/ja/`
- Update affected screenshots under: `docs/public/images/en/`, `docs/public/images/zh/`

- [ ] Update all four locales for Enterprise isolation, Key analytics, and EPay Admin configuration using Simplified Technical English.
- [ ] Capture English and Simplified Chinese screenshots for the changed API Key and Store Admin flows.
- [ ] Run `cargo fmt --check`, targeted Rust tests, `cargo test`, `cargo check`, `cd frontend; bun test`, `cd frontend; bun run build`, `cd docs; bun run build`, and `git diff --check`.
- [ ] Review `git status --short`; exclude the pre-existing Dashboard test modification and archive files unless this implementation intentionally requires them.
- [ ] Do not enable EPay in production without real gateway credentials and controlled payment/refund verification.
