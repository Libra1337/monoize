# Dashboard Currency And Usage Ranking Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose usage ranking to every authenticated user, add one persisted CNY/USD display preference, and stop Usage Trend line jitter during rapid metric changes.

**Architecture:** Keep `StoreCurrencyProvider` as the only currency state owner. Add a shared SWR exchange-rate hook and a reusable account-menu segmented control. Convert nano-USD through existing exact `BigInt` helpers and keep billing storage unchanged.

**Tech Stack:** React 19, TypeScript, SWR, Framer Motion, Tailwind CSS, Bun tests, Rust/Axum.

---

### Task 1: Lock The Observable Contract

**Files:**
- Modify: `spec/dashboard-ui-layout.spec.md`
- Modify: `spec/admin-usage-runtime.spec.md`
- Modify: `docs/superpowers/specs/2026-08-29-dashboard-usage-marketplace-docs-design.md`

- [ ] Define the ordinary-user route, account-menu order, valid stored values, storage failure behavior, exchange-rate cache reuse, loading fallback, and exact conversion rules.
- [ ] Run `rg -n "usage-ranking|monoize-display-currency-v1|display-currency" spec docs/superpowers/specs` and verify every requirement is concrete.

### Task 2: Write Failing Frontend Tests

**Files:**
- Modify: `frontend/tests/dashboard-experience.test.ts`
- Modify: `frontend/tests/store-page.test.ts`

- [ ] Assert that `/dashboard/usage-ranking` is outside `AdminRoute` and its navigation item is in the ordinary list.
- [ ] Assert that the account menu renders a CNY/USD segmented control backed by `useStoreCurrency`.
- [ ] Assert that `StoreCurrencyProvider` validates and persists `monoize-display-currency-v1`.
- [ ] Assert that Usage Trend animates explicit range or metric selections without interpolating polled paths.
- [ ] Run `bun test tests/dashboard-experience.test.ts tests/store-page.test.ts` and verify failure occurs because the currency control and persistence are absent.

### Task 3: Implement Shared Currency State And Presentation

**Files:**
- Modify: `frontend/src/hooks/use-store-currency.tsx`
- Create: `frontend/src/hooks/use-store-exchange-rate.ts`
- Modify: `frontend/src/components/user-center-menu.tsx`
- Modify: `frontend/src/pages/dashboard.tsx`
- Modify: `frontend/src/pages/admin-usage.tsx`
- Modify: `frontend/src/components/usage/usage-trend-chart.tsx`
- Modify: `frontend/src/locales/en.json`
- Modify: `frontend/src/locales/zh.json`
- Modify: `frontend/src/locales/zh-TW.json`
- Modify: `frontend/src/locales/ja.json`

- [ ] Add guarded read/write helpers for exactly `CNY` and `USD`, defaulting to CNY.
- [ ] Add a shared SWR hook for `/api/dashboard/store/exchange-rate` with cached-data preservation.
- [ ] Add the accessible segmented control between Settings and Theme with one shared motion indicator.
- [ ] Format account, Dashboard, and usage-ranking money from the shared preference and rate.
- [ ] Add native labels in all four locales.
- [ ] Disable Recharts path interpolation and animate only explicit range or metric selections.
- [ ] Run the two focused Bun tests and verify they pass.

### Task 4: Verify And Release

**Files:**
- Verify all modified files.

- [ ] Run `bun test` in `frontend`.
- [ ] Run `node node_modules/typescript/bin/tsc --noEmit` in `frontend`.
- [ ] Run `node node_modules/vite/bin/vite.js build` in `frontend`.
- [ ] Run `cargo test --no-default-features --lib` without starting PostgreSQL.
- [ ] Run `git diff --check`.
- [ ] Commit and push `main`.
- [ ] Build an isolated server candidate on port `18080`, back up SQLite and the Monoize systemd unit, switch only `monoize.service`, and keep Caddy active.
- [ ] Verify `https://lynshen.org`, `/dashboard/usage-ranking`, and `https://sub.joinreso.com/health`.
