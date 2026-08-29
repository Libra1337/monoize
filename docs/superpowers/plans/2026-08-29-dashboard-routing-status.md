# Dashboard, API Key Routing, And Status Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the approved Dashboard usage panel, explicit API-key Channel selection for ambiguous models, and a public 24-hour Group status view.

**Architecture:** Keep analytics in the existing request-log aggregation path and refresh it with SWR. Persist API-key Channel bindings as JSON and apply them before routing affinity. Build public status data with set-based request-log aggregation and render model details in a Dialog.

**Tech Stack:** Rust, Axum, SeaQuery migrations, SQLite-compatible SQL, React, TypeScript, SWR, Framer Motion, Recharts, Bun.

---

### Task 1: Usage panel and analytics labels

**Files:**
- Modify: `frontend/src/components/usage/token-summary.tsx`
- Modify: `frontend/src/components/usage/usage-trend-chart.tsx`
- Modify: `frontend/src/components/usage/model-distribution.tsx`
- Modify: `frontend/src/pages/dashboard.tsx`
- Modify: `frontend/src/pages/usage-analysis.tsx`
- Modify: `src/users/request_logs.rs`
- Modify: `src/dashboard_handlers/analytics_request_logs.rs`
- Test: `frontend/tests/dashboard-experience.test.ts`
- Test: `frontend/tests/usage-analytics.test.ts`

- [ ] Run the focused frontend tests and confirm the new assertions fail.
- [ ] Render one Token summary Card with four divided metrics and a matching Skeleton.
- [ ] Animate refreshed values from their prior value for about 900 ms.
- [ ] Refresh both authenticated analytics views every 2 seconds while retaining prior data.
- [ ] Normalize analytics model names as `model`, then `upstream_model`, then `unknown`.
- [ ] Run the focused frontend and Rust analytics tests until they pass.

### Task 2: API-key Group and Channel selection

**Files:**
- Create: `migration/src/m20260829_000060_api_key_channel_bindings.rs`
- Modify: `migration/src/lib.rs`
- Modify: `src/users/mod.rs`
- Modify: `src/users/store.rs`
- Modify: `src/auth.rs`
- Modify: `src/dashboard_handlers/api_keys.rs`
- Modify: `src/handlers/mod.rs`
- Modify: `src/handlers/routing.rs`
- Modify: `frontend/src/lib/api.ts`
- Modify: `frontend/src/lib/swr.ts`
- Modify: `frontend/src/pages/api-keys.tsx`
- Modify: `frontend/src/locales/en.json`
- Modify: `frontend/src/locales/zh.json`
- Modify: `frontend/src/locales/zh-TW.json`
- Modify: `frontend/src/locales/ja.json`

- [ ] Add failing storage, API, authentication, routing, and frontend assertions.
- [ ] Persist and strictly decode `channel_bindings`; make an empty `group_ids` array mean all Groups.
- [ ] Return current same-Group, same-model Channel conflicts from the dashboard API.
- [ ] Require complete, current bindings on API-key create and update.
- [ ] Filter routing attempts by the matching binding before affinity selection.
- [ ] Replace the default-Group UI with All Groups and conflict Channel selectors using SWR and Skeletons.
- [ ] Run focused frontend and SQLite-backed Rust tests until they pass.

### Task 3: Public Group status timeline

**Files:**
- Modify: `src/public_handlers.rs`
- Modify: `frontend/src/pages/public-status.tsx`
- Modify: `frontend/src/lib/api.ts`
- Modify: `frontend/src/locales/en.json`
- Modify: `frontend/src/locales/zh.json`
- Modify: `frontend/src/locales/zh-TW.json`
- Modify: `frontend/src/locales/ja.json`
- Test: `frontend/tests/dashboard-experience.test.ts`

- [ ] Add failing API aggregation and UI source assertions.
- [ ] Aggregate each Group into 48 half-hour buckets for the previous 24 hours.
- [ ] Return Group success rate, latest observation, model summaries, and status freshness.
- [ ] Render LynShen status cards, a 48-cell timeline, state legend, and a model-detail Dialog.
- [ ] Retain the existing 30-second SWR refresh and Skeleton behavior.
- [ ] Run focused frontend and SQLite-backed Rust tests until they pass.

### Task 4: Release verification and deployment

- [ ] Run all frontend tests and `bun run build`.
- [ ] Run SQLite Rust tests and `cargo check --no-default-features` without starting PostgreSQL.
- [ ] Run `git diff --check` and inspect the complete diff against all modified specs.
- [ ] Commit and push `main`.
- [ ] Create and verify a new production backup.
- [ ] Deploy with the repository-owned deployment procedure.
- [ ] Verify `https://lynshen.org`, Dashboard analytics, API-key validation, and `/status`.
