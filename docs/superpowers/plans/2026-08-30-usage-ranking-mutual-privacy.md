# Usage Ranking Mutual Privacy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add mutually consented ranking identity disclosure and align the Dashboard status refresh mark.

**Architecture:** Persist one default-private Boolean on each user. Apply the privacy matrix in the authenticated ranking handler so the frontend never receives forbidden identity fields. Reuse the existing optimistic current-user mutation and modal.

**Tech Stack:** Rust, SeaORM migration, SQLite/PostgreSQL, React 19, SWR, TypeScript, Bun.

---

### Task 1: Freeze observable behavior

**Files:** `spec/admin-usage-runtime.spec.md`, `spec/dashboard-ui-layout.spec.md`, `frontend/tests/dashboard-experience.test.ts`, `src/dashboard_handlers/admin.rs`

- [ ] Write source-contract and Rust unit tests for the mutual privacy matrix, selectable anonymous rows, current-ranking copy, personal Switch, and status-header layout.
- [ ] Run the focused tests and confirm they fail because the preference and matrix are absent.
- [ ] Update the product specifications with exact state and layout requirements.

### Task 2: Persist and expose the preference

**Files:** `src/migration/m20260830_000061_usage_ranking_privacy.rs`, `src/migration/mod.rs`, `src/users/mod.rs`, `src/users/store.rs`, `src/entity/users.rs`, `src/dashboard_handlers/auth.rs`

- [ ] Add the non-null default-private column and a SQLite migration test.
- [ ] Load the Boolean into every `User` and return it from `/dashboard/auth/me`.
- [ ] Accept an optional Boolean in `PUT /dashboard/auth/me` and persist it without changing unspecified fields.

### Task 3: Apply mutual privacy and update the UI

**Files:** `src/dashboard_handlers/admin.rs`, `frontend/src/lib/api.ts`, `frontend/src/lib/swr.ts`, `frontend/src/pages/user-settings.tsx`, `frontend/src/pages/admin-usage.tsx`, `frontend/src/pages/public-status.tsx`, and four locale JSON files

- [ ] Include identity for self, for mutually public peers, and for administrators only.
- [ ] Return non-cost model rows for all ordinary-user ranking entries so every row remains selectable.
- [ ] Add the optimistic personal privacy Switch and current-ranking labels in all four locales.
- [ ] Add compact blue input, orange cache-read, and green output counts to both ranking tables.
- [ ] Put the Dashboard-only refresh mark in the status header's upper-right column.

### Task 3a: Add the public ranking page

- [ ] Add one allow-listed public ranking endpoint with `24h`, `7d`, and `30d` windows and no identity or charge fields.
- [ ] Add the public route and navigation link in all four locales.
- [ ] Reuse the ranking presentation with anonymous rows, animated totals, range selection, and three-color Token breakdowns.

### Task 4: Verify and deploy

- [ ] Run focused Rust and Bun tests, then all frontend tests, TypeScript, Vite build, Rust SQLite library tests, and `git diff --check`.
- [ ] Commit and push `main`.
- [ ] Build and probe a candidate at `127.0.0.1:18080` with a writable database copy.
- [ ] Switch only `monoize.service`. Keep Caddy active and verify both domains before and after.
