# Dashboard Usage, Marketplace, API Docs, and Redemption Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build authenticated Dashboard usage analytics, Marketplace, and API Docs pages, simplify the Dashboard home, link the brand home, and make Store redemption-code access operational and explicit.

**Architecture:** Extend the existing exact Dashboard analytics pipeline with token aggregates and keep personal analytics scoped to the authenticated user. Reuse the existing SWR, Recharts, motion, Store exchange-rate, public Marketplace allow-list, and Store `PaymentKeyRing` contracts through focused presentation components. Keep public pages and request logs independent from authenticated pages.

**Tech Stack:** Rust 2024, Axum, SeaORM, SQLite/PostgreSQL-compatible SQL, React 19, TypeScript, Vite, SWR, Recharts, Framer Motion, shadcn/ui, Tailwind CSS, Bun tests, Playwright screenshots.

---

### Task 1: Freeze Observable Contracts

**Files:**
- Modify: `spec/dashboard-home-overview.spec.md`
- Create: `spec/dashboard-usage-analysis.spec.md`
- Modify: `spec/dashboard-ui-layout.spec.md`
- Modify: `spec/model-marketplace.spec.md`
- Modify: `spec/public-site.spec.md`
- Modify: `spec/store-billing.spec.md`
- Modify: `frontend/tests/public-routes.test.ts`
- Create: `frontend/tests/dashboard-experience.test.ts`

- [ ] **Step 1: Write failing route and source-contract tests**

Add Bun tests that require `/marketplace` in `PUBLIC_ROUTES`, require `/dashboard/marketplace` to be protected, and inspect `App.tsx`, `layout.tsx`, `dashboard.tsx`, `usage-analysis.tsx`, and `dashboard-api-docs.tsx` for the approved route and page boundaries. The tests require `to="/"` on the brand link and prohibit request-log imports from `usage-analysis.tsx`.

- [ ] **Step 2: Run the tests and verify RED**

Run: `cd frontend && bun test tests/public-routes.test.ts tests/dashboard-experience.test.ts`

Expected: FAIL because `/marketplace` and the authenticated routes do not exist and Dashboard still renders Model Data and API Information.

- [ ] **Step 3: Update the six specifications**

Write concrete requirements for the exact routes, self-scoped token analytics, 24h/week/month Dashboard ranges, 24h/7d/30d Usage Analysis ranges, CNY/USD pricing, unchanged logs, one-time redemption plaintext, v1 unrecoverability, and Store key readiness. Remove the obsolete Dashboard row-C and public `/dashboard/marketplace` requirements.

- [ ] **Step 4: Run spec and whitespace checks**

Run: `git diff --check`

Expected: PASS with no whitespace errors.

- [ ] **Step 5: Commit the contract change**

```powershell
git add spec frontend/tests/public-routes.test.ts frontend/tests/dashboard-experience.test.ts
git commit -m "spec: define dashboard usage experience"
```

### Task 2: Extend Exact Analytics With Token Aggregates

**Files:**
- Modify: `src/users/mod.rs`
- Modify: `src/users/request_logs.rs`
- Modify: `src/dashboard_handlers/analytics_request_logs.rs`
- Modify: `frontend/src/lib/api.ts`
- Modify: `frontend/src/lib/swr.ts`
- Test: `src/users/request_logs.rs`
- Test: `src/dashboard_handlers/tests.rs`

- [ ] **Step 1: Write failing SQLite aggregation tests**

Insert request logs for two users and two models with input, cache-read, and output token columns. Assert that the requested user receives exact bucket totals and `tokens_by_model`, while another user's tokens never contribute. Add a handler serialization test that requires decimal-string token fields where values can exceed JavaScript's safe integer range.

- [ ] **Step 2: Run the targeted tests and verify RED**

Run: `cargo test request_logs::tests::dashboard_analytics --lib`

Expected: FAIL because `DashboardAnalyticsRaw` has no token fields.

- [ ] **Step 3: Add token aggregate types and SQL**

Extend each model bucket with `input_tokens`, `cache_read_tokens`, and `output_tokens`. Aggregate the canonical integer columns in the same set-based query used for model cost and calls. Use checked `i128` decoding for token totals and reject negative or overflowing persisted aggregate values.

- [ ] **Step 4: Extend the handler response and frontend types**

Add these response properties:

```text
input_tokens_by_model: Record<string, string>
cache_read_tokens_by_model: Record<string, string>
output_tokens_by_model: Record<string, string>
total_input_tokens: string
total_cache_read_tokens: string
total_output_tokens: string
total_tokens: string
```

Keep every existing response field. Add query value `scope=self`; when present, the handler passes the authenticated user ID to aggregation even for an Admin. Keep the current omitted-scope behavior for the system Dashboard. Make `useDashboardAnalytics` accept the scope and map the Dashboard/Usage hooks to `scope=self`.

- [ ] **Step 5: Run targeted Rust tests and verify GREEN**

Run: `cargo test dashboard_analytics --lib`

Expected: PASS.

- [ ] **Step 6: Commit exact analytics**

```powershell
git add src/users/mod.rs src/users/request_logs.rs src/dashboard_handlers/analytics_request_logs.rs frontend/src/lib/api.ts frontend/src/lib/swr.ts
git commit -m "feat: aggregate dashboard token usage"
```

### Task 3: Add Shared Currency and Analytics Presentation Utilities

**Files:**
- Create: `frontend/src/hooks/use-store-currency.tsx`
- Create: `frontend/src/lib/usage-analytics.ts`
- Modify: `frontend/src/lib/store-money.ts`
- Modify: `frontend/src/App.tsx`
- Modify: `frontend/src/pages/store/index.tsx`
- Create: `frontend/tests/usage-analytics.test.ts`
- Modify: `frontend/tests/store-money.test.ts`
- Modify: `frontend/tests/store-page.test.ts`

- [ ] **Step 1: Write failing utility and shared-state tests**

Test exact token totals, cache-hit rate, top-model ordering, nano-USD per-token conversion to `¥x.xx / 1M tokens` and `$x.xx / 1M tokens`, and one context-backed currency value shared by Store and Marketplace.

- [ ] **Step 2: Run the tests and verify RED**

Run: `cd frontend && bun test tests/usage-analytics.test.ts tests/store-money.test.ts tests/store-page.test.ts`

Expected: FAIL because the utilities and provider do not exist and Store owns local currency state.

- [ ] **Step 3: Implement exact pure utilities**

Implement BigInt-only aggregation and formatting helpers. Multiply per-token nano-USD rates by exactly `1_000_000` before currency conversion. Round only the final currency minor unit with the existing half-away-from-zero rule.

- [ ] **Step 4: Implement the currency provider**

Add `StoreCurrencyProvider` above routes. Expose `{ currency, setCurrency }`, default to CNY, and do not persist in `localStorage`. Replace Store's local `useState<StoreCurrency>` with the provider.

- [ ] **Step 5: Run the tests and verify GREEN**

Run: `cd frontend && bun test tests/usage-analytics.test.ts tests/store-money.test.ts tests/store-page.test.ts`

Expected: PASS.

- [ ] **Step 6: Commit shared presentation state**

```powershell
git add frontend/src/hooks/use-store-currency.tsx frontend/src/lib/usage-analytics.ts frontend/src/lib/store-money.ts frontend/src/App.tsx frontend/src/pages/store/index.tsx frontend/tests
git commit -m "feat: share store currency and usage formatting"
```

### Task 4: Simplify Dashboard Home and Add Usage Analysis

**Files:**
- Rewrite: `frontend/src/pages/dashboard.tsx`
- Create: `frontend/src/pages/usage-analysis.tsx`
- Create: `frontend/src/components/usage/token-summary.tsx`
- Create: `frontend/src/components/usage/usage-trend-chart.tsx`
- Create: `frontend/src/components/usage/model-distribution.tsx`
- Modify: `frontend/src/App.tsx`
- Modify: `frontend/src/pages/layout.tsx`
- Modify: `frontend/src/locales/en.json`
- Modify: `frontend/src/locales/zh.json`
- Modify: `frontend/src/locales/zh-TW.json`
- Modify: `frontend/src/locales/ja.json`
- Test: `frontend/tests/dashboard-experience.test.ts`

- [ ] **Step 1: Expand failing Dashboard source tests**

Require the three new usage components, the range controls, a skeleton, `motion`, reduced-motion-safe shared helpers, and a `/dashboard/usage` navigation item. Require Dashboard to omit `Model Data`, analysis tabs, and API Information. Require Usage Analysis to omit `useRequestLogs` and `RequestLogsPage`.

- [ ] **Step 2: Run the tests and verify RED**

Run: `cd frontend && bun test tests/dashboard-experience.test.ts`

Expected: FAIL because the components and route do not exist.

- [ ] **Step 3: Implement Dashboard Token Usage**

Keep the greeting and four compact overview cards. Render Token Summary and a compact trend preview. Use 24h/week/month analytics queries, SWR skeletons, shared-layout range movement, tabular numerals, and count-up animation that resolves immediately under reduced motion.

- [ ] **Step 4: Implement Usage Analysis**

Render 24h/7d/30d range buttons, four exact summary values, metric-segmented time series, model distribution, and ranked rows. Use `ChartContainer`, Recharts, existing chart tokens, accessible text values, stable responsive dimensions, and inline retry/empty states.

- [ ] **Step 5: Add all four locale trees**

Add the same keys in `en`, `zh`, `zh-TW`, and `ja`. Keep `Provider`, `Channel`, API paths, and environment variables in canonical English.

- [ ] **Step 6: Run frontend tests and build**

Run: `cd frontend && bun test tests/dashboard-experience.test.ts tests/live-usage-format.test.ts && bun run build`

Expected: tests PASS and Vite build exits 0.

- [ ] **Step 7: Commit Dashboard and Usage Analysis**

```powershell
git add frontend/src/pages/dashboard.tsx frontend/src/pages/usage-analysis.tsx frontend/src/components/usage frontend/src/App.tsx frontend/src/pages/layout.tsx frontend/src/locales frontend/tests/dashboard-experience.test.ts
git commit -m "feat: add dashboard usage analysis"
```

### Task 5: Build Authenticated Marketplace With Human Prices

**Files:**
- Modify: `frontend/src/public-routes.ts`
- Modify: `frontend/src/App.tsx`
- Rewrite: `frontend/src/pages/model-marketplace.tsx`
- Modify: `frontend/src/pages/public-layout.tsx`
- Modify: `frontend/src/pages/welcome.tsx`
- Modify: `frontend/src/lib/swr.ts`
- Modify: `frontend/src/locales/en.json`
- Modify: `frontend/src/locales/zh.json`
- Modify: `frontend/src/locales/zh-TW.json`
- Modify: `frontend/src/locales/ja.json`
- Create: `frontend/tests/dashboard-marketplace.test.ts`
- Modify: `frontend/tests/public-routes.test.ts`

- [ ] **Step 1: Write failing Marketplace tests**

Require public `/marketplace`, protected `/dashboard/marketplace`, Dashboard route ownership, Group and capability filters, shared CNY/USD control, readable `/ 1M tokens` prices, and a modal that uses only allow-listed Provider/Channel fields.

- [ ] **Step 2: Run tests and verify RED**

Run: `cd frontend && bun test tests/public-routes.test.ts tests/dashboard-marketplace.test.ts`

Expected: FAIL because the existing page is a fixed-height nano-USD table and the public route conflicts with Dashboard.

- [ ] **Step 3: Move the public route**

Set `PUBLIC_PATHS.marketplace` to `/marketplace`, update public navigation links, and mount `ModelMarketplacePage` at `/dashboard/marketplace` under `DashboardLayout`.

- [ ] **Step 4: Implement authenticated Marketplace**

Render a stable toolbar, explicit Group sections, compact model rows, capability chips, readable input/output price ranges, and a responsive offer modal. Use SWR with previous data retained and fetch the current Store exchange-rate snapshot. Do not use fixed viewport table height.

- [ ] **Step 5: Run tests and build**

Run: `cd frontend && bun test tests/public-routes.test.ts tests/dashboard-marketplace.test.ts tests/public-marketplace.test.ts && bun run build`

Expected: PASS and build exits 0.

- [ ] **Step 6: Commit Marketplace**

```powershell
git add frontend/src/public-routes.ts frontend/src/App.tsx frontend/src/pages/model-marketplace.tsx frontend/src/pages/public-layout.tsx frontend/src/pages/welcome.tsx frontend/src/lib/swr.ts frontend/src/locales frontend/tests
git commit -m "feat: add authenticated model marketplace"
```

### Task 6: Add Authenticated API Docs and Home Brand Link

**Files:**
- Create: `frontend/src/pages/dashboard-api-docs.tsx`
- Create: `frontend/src/lib/api-samples.ts`
- Modify: `frontend/src/pages/api-docs.tsx`
- Modify: `frontend/src/pages/layout.tsx`
- Modify: `frontend/src/App.tsx`
- Modify: `frontend/src/locales/en.json`
- Modify: `frontend/src/locales/zh.json`
- Modify: `frontend/src/locales/zh-TW.json`
- Modify: `frontend/src/locales/ja.json`
- Create: `frontend/tests/dashboard-api-docs.test.ts`

- [ ] **Step 1: Write failing sample and route tests**

Test exact endpoint selection, request bodies, environment-variable authentication, missing Base URL state, copy controls, Dashboard route ownership, and brand `to="/"` in expanded, collapsed, and mobile sidebar variants.

- [ ] **Step 2: Run tests and verify RED**

Run: `cd frontend && bun test tests/dashboard-api-docs.test.ts tests/dashboard-experience.test.ts`

Expected: FAIL because the authenticated page and shared sample module do not exist and brand links to `/dashboard`.

- [ ] **Step 3: Extract sample generation**

Move pure endpoint, body, and language sample generation into `api-samples.ts`. Keep the public page behavior unchanged while consuming the shared module.

- [ ] **Step 4: Implement authenticated API Docs**

Render compact family navigation, method/path, Base URL, auth header, request samples, streaming notes, success shape, and common errors. Use SWR skeletons and copy feedback. Never inject an actual API key.

- [ ] **Step 5: Link the brand home**

Change the sidebar brand link to `/` and close the mobile sheet through the existing `onNavigate` callback.

- [ ] **Step 6: Run tests and build**

Run: `cd frontend && bun test tests/dashboard-api-docs.test.ts tests/dashboard-experience.test.ts tests/public-routes.test.ts && bun run build`

Expected: PASS and build exits 0.

- [ ] **Step 7: Commit API Docs and brand navigation**

```powershell
git add frontend/src/pages/dashboard-api-docs.tsx frontend/src/pages/api-docs.tsx frontend/src/lib/api-samples.ts frontend/src/pages/layout.tsx frontend/src/App.tsx frontend/src/locales frontend/tests
git commit -m "feat: add dashboard API docs"
```

### Task 7: Correct Redemption Code Access and Readiness

**Files:**
- Modify: `frontend/src/pages/store-admin/admin-panels.tsx`
- Modify: `frontend/src/pages/store-admin/index.tsx`
- Modify: `frontend/src/pages/store-admin/redemption-dialog.tsx`
- Create: `frontend/src/pages/store-admin/redemption-access-dialog.tsx`
- Modify: `frontend/src/lib/store-api.ts`
- Modify: `frontend/src/locales/en.json`
- Modify: `frontend/src/locales/zh.json`
- Modify: `frontend/src/locales/zh-TW.json`
- Modify: `frontend/src/locales/ja.json`
- Modify: `src/dashboard_handlers/store_billing.rs`
- Modify: `tests/api/store_payments.rs`
- Modify: `tests/store_redemption_security.rs`
- Modify: `frontend/tests/store-admin.test.ts`

- [ ] **Step 1: Reproduce missing-key and plaintext-visibility failures**

Add API tests that assert generation and reveal return one stable readiness error when `PaymentKeyRing` is absent. Add frontend tests that require one-time full code output, per-code copy, reauthenticated v2 reveal, and an explicit legacy v1 unrecoverable state.

- [ ] **Step 2: Run tests and verify RED**

Run: `cargo test store_redemption --test store_redemption_security`

Run: `cd frontend && bun test tests/store-admin.test.ts`

Expected: at least one test FAIL because list rows have no functional reveal/copy UI and missing-key handling exposes the raw backend message.

- [ ] **Step 3: Normalize backend readiness errors**

Map missing `state.payment_keys` for generate, reveal, and export to one documented Store error code. Extend list records with an allow-listed `can_reveal` boolean and fixed `reveal_unavailable_reason` enum so the UI can distinguish v1 rows without exposing ciphertext fields. Preserve fail-closed behavior, `Cache-Control: no-store`, reauthentication, audit writes, and v1 non-recoverability.

- [ ] **Step 4: Implement Admin reveal/copy UI**

Add actions for unused records. Obtain a scoped reauthentication grant from the current Admin password, call reveal/copy, display plaintext only in a modal, and clear it on close. Show a legacy badge and revoke-only explanation for v1 records.

- [ ] **Step 5: Improve generation result safety**

Keep the generation dialog open after success, display every complete returned code, add per-code copy and Copy All, and clear clipboard-neutral frontend state when the dialog closes.

- [ ] **Step 6: Run Store tests and verify GREEN**

Run: `cargo test store_redemption --test store_redemption_security`

Run: `cd frontend && bun test tests/store-admin.test.ts tests/store-api.test.ts`

Expected: PASS.

- [ ] **Step 7: Commit redemption access**

```powershell
git add src/dashboard_handlers/store_billing.rs tests frontend/src/pages/store-admin frontend/src/lib/store-api.ts frontend/src/locales frontend/tests/store-admin.test.ts
git commit -m "fix: make redemption code access operational"
```

### Task 8: Documentation, Screenshots, Full Verification, and Deployment

**Files:**
- Modify: `docs/content/docs/dashboard/index.mdx`
- Modify: `docs/content/docs/dashboard/index.zh.mdx`
- Modify: `docs/content/docs/dashboard/index.zh-TW.mdx`
- Modify: `docs/content/docs/dashboard/index.ja.mdx`
- Modify: `docs/content/docs/dashboard/meta.json`
- Modify: `docs/content/docs/dashboard/meta.zh.json`
- Modify: `docs/content/docs/dashboard/meta.zh-TW.json`
- Modify: `docs/content/docs/dashboard/meta.ja.json`
- Create: `docs/content/docs/dashboard/usage-analysis.mdx`
- Create: `docs/content/docs/dashboard/usage-analysis.zh.mdx`
- Create: `docs/content/docs/dashboard/usage-analysis.zh-TW.mdx`
- Create: `docs/content/docs/dashboard/usage-analysis.ja.mdx`
- Modify: `docs/public/images/en/dashboard.webp`
- Modify: `docs/public/images/zh/dashboard.webp`

- [ ] **Step 1: Update four-language docs in STE**

Document the Dashboard Token Usage ranges, Usage Analysis route, authenticated Marketplace, authenticated API Docs, shared currency behavior, and legacy redemption limitation. Keep every instruction short and active.

- [ ] **Step 2: Run focused frontend and Rust verification**

Run: `cd frontend && bun test`

Run: `cargo test dashboard_analytics --lib`

Run: `cargo test store_redemption --test store_redemption_security`

Expected: all tests PASS.

- [ ] **Step 3: Run full builds without local PostgreSQL**

Run: `cargo check`

Run: `cd frontend && bun run build`

Run: `cd docs && bun install && bun run build`

Expected: all commands exit 0. Do not start PostgreSQL.

- [ ] **Step 4: Recapture responsive screenshots**

Run the local application with SQLite. Capture Dashboard, Usage Analysis, Marketplace, API Docs, and Store Admin redemption states at 1440x900 and 390x844 in English and Simplified Chinese. Replace both documented Dashboard WebP files and verify no overlap or horizontal overflow.

- [ ] **Step 5: Run final repository checks**

Run: `git diff --check`

Run: `git status --short`

Expected: only intended files are changed before the final commit.

- [ ] **Step 6: Commit docs and verification artifacts**

```powershell
git add docs
git commit -m "docs: document dashboard usage pages"
```

- [ ] **Step 7: Push and perform production preflight**

Push `codex/lynshen-public-product`. On `103.240.199.109`, verify the current database backup and restore path, confirm `MONOIZE_STORE_PAYMENT_KEYS_JSON` contains one valid active 32-byte key without printing it, keep payment Channels disabled, build the release image, and record the rollback image digest.

- [ ] **Step 8: Deploy and verify production**

Deploy the verified image. Check `https://lynshen.org/`, authenticated Dashboard routes, analytics responses, public `/marketplace`, code generation readiness, container health, and Caddy health. Do not generate, reveal, redeem, or revoke a production code during verification.
