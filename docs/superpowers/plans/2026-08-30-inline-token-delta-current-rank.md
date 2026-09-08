# Inline Token Delta And Current Rank Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render each Token segment delta beside its rolling value and show an ordinary viewer's actual top-20 position or `Unranked`.

**Architecture:** Extend the existing authenticated usage-ranking response with one nullable rank computed after sorting and before response projection. Reuse `AnimatedTokenValue` for the three segment rows, but make its delta an inline, transient motion element that unmounts after the animation cycle.

**Tech Stack:** Rust, Axum, Serde JSON, React, TypeScript, Framer Motion, Bun tests.

---

### Task 1: Lock The API And UI Contract With Failing Tests

**Files:**
- Modify: `src/dashboard_handlers/admin.rs`
- Modify: `frontend/tests/dashboard-experience.test.ts`

- [ ] **Step 1: Add a Rust unit test for top-20 rank projection**

Add a pure helper test that expects a one-based rank for a matching ID in the first 20 rows and `None` for a matching ID after position 20.

- [ ] **Step 2: Add frontend source-contract assertions**

Assert that the response type contains `current_user_rank?: number | null`, the non-admin summary renders `#${data.current_user_rank}`, all three segment rows use `AnimatedTokenValue` with `showDelta`, and `AnimatedTokenValue` no longer uses `flex-col` or `min-h-4`.

- [ ] **Step 3: Run focused tests and verify failure**

Run: `cargo test --locked --no-default-features dashboard_handlers::admin::tests::current_user_rank_is_limited_to_returned_top_twenty`

Expected: FAIL because the rank helper does not exist.

Run: `cd frontend && bun test tests/dashboard-experience.test.ts`

Expected: FAIL because the response field and inline delta layout do not exist.

### Task 2: Implement Authenticated Current Rank

**Files:**
- Modify: `src/dashboard_handlers/admin.rs`
- Modify: `frontend/src/lib/api.ts`
- Modify: `frontend/src/pages/admin-usage.tsx`
- Modify: `frontend/src/locales/en.json`
- Modify: `frontend/src/locales/zh.json`
- Modify: `frontend/src/locales/zh-TW.json`
- Modify: `frontend/src/locales/ja.json`

- [ ] **Step 1: Add the pure rank helper**

The helper returns `position + 1` only when the caller ID occurs before index 20. It returns `None` otherwise.

- [ ] **Step 2: Add `current_user_rank` to non-admin authenticated responses**

Compute the rank after the existing sort and before truncation. Return the field only for an authenticated non-administrator. Keep administrator and public response behavior unchanged.

- [ ] **Step 3: Render the current-rank summary**

Add `current_user_rank?: number | null` to `AdminUsageRanking`. For ordinary users, replace the ranked-user count with `#N` or the localized `Unranked` text. Keep the ranked-user count for administrators.

- [ ] **Step 4: Add all four locale strings**

Add native strings for `Current rank` and `Unranked` in `en`, `zh`, `zh-TW`, and `ja`.

### Task 3: Move Segment Deltas Inline

**Files:**
- Modify: `frontend/src/components/usage/token-summary.tsx`
- Modify: `frontend/src/pages/admin-usage.tsx`
- Modify: `frontend/src/pages/public-usage-ranking.tsx`

- [ ] **Step 1: Make `AnimatedTokenValue` an inline row**

Render the main value and delta in one `inline-flex items-baseline` container. Render the delta only while `deltaVisible` is true. Use `AnimatePresence` exit opacity so the delta fades out, then leaves layout.

- [ ] **Step 2: Enable deltas on the three segment values**

Replace static authenticated segment values with `<AnimatedTokenValue value={segment.value} showDelta />`. Add `showDelta` to the three public segment values.

- [ ] **Step 3: Run focused tests and verify green**

Run: `cargo test --locked --no-default-features dashboard_handlers::admin::tests::current_user_rank_is_limited_to_returned_top_twenty`

Expected: PASS.

Run: `cd frontend && bun test tests/dashboard-experience.test.ts`

Expected: PASS.

### Task 4: Verify And Release

**Files:**
- Verify: all modified files

- [ ] **Step 1: Run frontend checks**

Run: `cd frontend && bun run lint` and `cd frontend && bun run build`.

- [ ] **Step 2: Run Rust checks**

Run: `cargo check --locked --no-default-features` and the affected Rust tests.

- [ ] **Step 3: Check patch integrity**

Run: `git diff --check`.

- [ ] **Step 4: Commit, push, and deploy safely**

Push `main`. Build and validate a candidate on `127.0.0.1:18080`. Do not stop, restart, or kill Caddy. Switch only `monoize.service`, then verify both `https://lynshen.org/` and `https://sub.joinreso.com/health`.
