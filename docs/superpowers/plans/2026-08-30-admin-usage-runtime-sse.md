# Admin Usage, Runtime, and Responses SSE Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix malformed multi-object Responses SSE events and add administrator-only usage-ranking and runtime-status pages.

**Architecture:** Decode each upstream Responses SSE field through one bounded strict parser before mutating stream state. Aggregate the preceding 24 hours in SQL, return one typed administrator snapshot, and render two guarded dashboard routes with SWR polling.

**Tech Stack:** Rust, Axum, SeaORM, SQLite/PostgreSQL-compatible SQL, React, TypeScript, SWR, Framer Motion, Tailwind CSS.

---

### Task 1: Freeze observable behavior

**Files:**
- Modify: `spec/unified_responses_proxy.spec.md`
- Modify: `spec/admin-usage-runtime.spec.md`

- [x] Define strict bounded parsing for multiple complete typed JSON values in one Responses SSE field.
- [x] Define administrator authorization, SQL aggregation, two-second refresh, stable loading states, and the rotating brand mark.

### Task 2: Test and fix Responses SSE decoding

**Files:**
- Modify: `src/urp/stream_decode/openai_responses.rs`
- Modify: `src/urp/stream_decode/openai_responses/tests.inc.rs`
- Modify: `src/urp/stream_decode/openai_responses/stream_loop_part1.inc.rs`

- [x] Add unit tests for two complete JSON objects, an optional terminal sentinel, trailing garbage, missing `type`, value-count bounds, and input-size bounds.
- [x] Run the focused Rust tests and confirm the new compatibility test fails before implementation.
- [x] Add one parser that validates the complete field before returning any values.
- [x] Route every returned typed object through the existing event mapper in source order.
- [x] Run the focused Rust tests and confirm all cases pass.

### Task 3: Implement administrator usage aggregation

**Files:**
- Modify: `src/users/mod.rs`
- Modify: `src/users/request_logs.rs`
- Modify: `src/dashboard_handlers/admin.rs`
- Modify: `src/dashboard_handlers/mod.rs`
- Modify: `src/app.rs`

- [x] Add a SQLite test that inserts multiple users, models, token classes, charges, and an out-of-window row.
- [x] Confirm the test fails because the ranking query is absent.
- [x] Add a cross-database SQL aggregate grouped by user and model.
- [x] Group model rows per user, validate non-negative aggregates, order users by charge/calls/id, and limit users to 20.
- [x] Register `GET /dashboard/admin/usage-ranking` behind `require_admin`.
- [x] Run the focused store and handler tests.

### Task 4: Add administrator pages and navigation

**Files:**
- Create: `frontend/src/pages/admin-usage.tsx`
- Create: `frontend/src/pages/admin-runtime.tsx`
- Create: `frontend/src/components/admin/refresh-status-logo.tsx`
- Modify: `frontend/src/pages/admin-dashboard.tsx`
- Modify: `frontend/src/pages/layout.tsx`
- Modify: `frontend/src/App.tsx`
- Modify: `frontend/src/lib/api.ts`
- Modify: `frontend/src/lib/swr.ts`
- Modify: `frontend/src/locales/en.json`
- Modify: `frontend/src/locales/zh.json`
- Modify: `frontend/src/locales/zh-TW.json`
- Modify: `frontend/src/locales/ja.json`

- [x] Define typed API snapshots and SWR keys.
- [x] Render a full-width segmented Tokens bar, stable ranking rows, and a model-detail modal.
- [x] Render node, process, Provider/Channel, and Replica status from the existing overview endpoint.
- [x] Poll both pages every 2 seconds and animate the fixed-size brand mark while SWR validates.
- [x] Guard all administrator routes before their page fetchers execute and hide their navigation from ordinary users.
- [x] Add native technical translations in all four supported locales.

### Task 5: Verify and deploy

**Files:**
- Verify all modified files.

- [x] Run `cargo test --no-default-features --lib` and `cargo check --no-default-features --lib`.
- [ ] Run `bun run test` and `bun run build` under `frontend`.
- [x] Run `git diff --check` and inspect the final diff for secret exposure.
- [ ] Commit and push `main`.
- [ ] Build an isolated production candidate on `103.240.199.109`, probe it on port 18080, back up SQLite, and switch only the Monoize service.
- [ ] Verify `https://lynshen.org`, `https://lynshen.org/api/public/status`, and `https://sub.joinreso.com/health` without stopping or restarting Caddy.
