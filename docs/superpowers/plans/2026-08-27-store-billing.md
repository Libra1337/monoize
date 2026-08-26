# Store Billing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a tested Store for balance recharge, plan purchase, orders, payment channels, and redemption codes to LynShen Console.

**Architecture:** Add one SeaORM migration and a focused `store_billing` service over the existing database pool and user ledger. Expose session-scoped and admin-scoped handlers through the dashboard router. Add three React pages backed by one typed Store API and SWR hooks; keep currency arithmetic in tested integer-string helpers.

**Tech Stack:** Rust 2024, Axum 0.8, SeaORM/SQLx SQLite and PostgreSQL SQL branches, rust_decimal, React 19, TypeScript, SWR, Tailwind, Framer Motion, Bun tests.

---

### Task 1: Store Schema

**Files:**
- Create: `src/migration/m20260827_000049_store_billing.rs`
- Modify: `src/migration/mod.rs`
- Test: `src/migration/m20260827_000049_store_billing.rs`

- [ ] Write a SQLite migration test that runs the migrator, asserts all Store tables and indexes exist, and verifies invalid `kind`, `currency`, duplicate order number, and duplicate redemption digest inserts fail.
- [ ] Run `cargo test migration::m20260827_000049_store_billing --lib` and confirm failure because the migration module is absent.
- [ ] Add backend-specific DDL for `store_exchange_rates`, `store_products`, `store_balance_products`, `store_plan_quotas`, `store_payment_channels`, `store_orders`, `store_plan_entitlements`, and `store_redemption_codes`. Seed disabled `alipay` and `wechat` channels.
- [ ] Add migration `000049` after `000048` in `Migrator::migrations()` and declare its module.
- [ ] Run the focused migration test and `cargo test migration --lib` until both pass.
- [ ] Commit `feat: add store billing schema`.

### Task 2: Exact Money And Exchange Rate

**Files:**
- Create: `src/store_billing/money.rs`
- Create: `src/store_billing/exchange_rate.rs`
- Create: `src/store_billing/mod.rs`
- Modify: `src/lib.rs`
- Test: unit tests in both new modules

- [ ] Write failing tests for canonical minor-unit parsing, CNY/USD conversion, half-away-from-zero rounding, `1 USD = 6.7370 CNY`, invalid remote payloads, and last-good-rate fallback.
- [ ] Run `cargo test store_billing::money store_billing::exchange_rate --lib` and confirm missing-module failures.
- [ ] Implement `Currency`, `Money`, `ExchangeRateSnapshot`, `convert_minor`, and `parse_er_api_response` with `i128` and `rust_decimal` only.
- [ ] Implement a 15-minute refresh cache that persists the last valid snapshot through the Store service and never clears it after a failed refresh.
- [ ] Run focused tests and commit `feat: add exact store currency conversion`.

### Task 3: Store Domain Service

**Files:**
- Create: `src/store_billing/models.rs`
- Create: `src/store_billing/store.rs`
- Modify: `src/store_billing/mod.rs`
- Modify: `src/users/store.rs`
- Test: `tests/store_billing.rs`

- [ ] Write failing SQLite service tests for catalog ordering, disabled records, recharge amount plus bonus, immutable order quote, user order scoping, order completion idempotency, cancelled-order rejection, plan snapshot replacement, code expiry, and concurrent single-use redemption.
- [ ] Run `cargo test --test store_billing` and confirm the missing service API causes failure.
- [ ] Implement typed inputs and outputs plus a cloneable `StoreBillingStore` over `DbPool`.
- [ ] Add transaction helpers that lock with `FOR UPDATE` on PostgreSQL and rely on SQLite write serialization, update `users.balance_nano_usd`, and insert ledger rows with order/code idempotency keys.
- [ ] Implement product, channel, order, entitlement, and redemption CRUD with validation from `spec/store-billing.spec.md`.
- [ ] Run the service tests and commit `feat: implement store billing service`.

### Task 4: Dashboard APIs

**Files:**
- Create: `src/dashboard_handlers/store_billing.rs`
- Modify: `src/dashboard_handlers/mod.rs`
- Modify: `src/app.rs`
- Modify: `src/app.rs` AppState initialization
- Modify: `tests/api.rs`
- Create: `tests/api/store_billing.rs`

- [ ] Write failing API tests for unauthenticated access, user ownership, admin guards, catalog, exchange rate, order create/list, complete/cancel idempotency, product/channel mutation, redemption generation, and redemption without a payment channel.
- [ ] Run `cargo test --test api store_billing` and confirm route-not-found failures.
- [ ] Add `store_billing: StoreBillingStore` to `AppState` and initialize it from the existing database pool and HTTP client.
- [ ] Implement session and admin handlers using existing `require_session_user` and `require_admin` guards and the shared JSON error envelope.
- [ ] Register every route from SB-A-1 and SB-A-2 under `/api/dashboard/store`.
- [ ] Run API tests and commit `feat: expose store billing dashboard APIs`.

### Task 5: Frontend API, Money Helpers, And Navigation

**Files:**
- Create: `frontend/src/lib/store-money.ts`
- Create: `frontend/src/lib/store-api.ts`
- Create: `frontend/tests/store-money.test.ts`
- Create: `frontend/tests/store-navigation.test.ts`
- Modify: `frontend/src/App.tsx`
- Modify: `frontend/src/pages/layout.tsx`
- Modify: `frontend/src/locales/en.json`
- Modify: `frontend/src/locales/zh.json`
- Modify: `frontend/src/locales/zh-TW.json`
- Modify: `frontend/src/locales/ja.json`

- [ ] Write failing Bun tests for exact formatting, CNY-to-USD conversion, integer quota rounding, the `实得` wording, Store and Orders routes, and admin-only Store Management navigation.
- [ ] Run `cd frontend; bun test tests/store-money.test.ts tests/store-navigation.test.ts` and confirm failures.
- [ ] Implement integer-string formatting helpers and typed API methods without floating-point money arithmetic.
- [ ] Register `/dashboard/store`, `/dashboard/orders`, and `/dashboard/store-admin`, then add role-aware navigation with the existing animated active indicator.
- [ ] Add complete locale keys and run the focused tests.
- [ ] Commit `feat: add store navigation and currency helpers`.

### Task 6: User Store And Orders UI

**Files:**
- Create: `frontend/src/pages/store/index.tsx`
- Create: `frontend/src/pages/store/store-skeleton.tsx`
- Create: `frontend/src/pages/orders.tsx`
- Create: `frontend/tests/store-page.test.ts`
- Modify: `frontend/src/lib/api.ts`

- [ ] Write failing source/component tests for the three-position indicator, shared currency state, stable summary height, full-width payment row, natural-height products, no payment/summary in redemption mode, and separate Orders page.
- [ ] Run the focused Bun tests and confirm failures.
- [ ] Build Store sections with 16 px cards, 12 px controls, Framer Motion transitions, reduced-motion handling, live exchange-rate metadata, product/channel empty states, and responsive grids.
- [ ] Fetch catalog, rate, session data, entitlement, and orders with SWR skeletons. Use optimistic `mutate` with rollback for order creation and redemption.
- [ ] Render balance cards as recharge, bonus, and `实得金额`; render plan quota values as whole amounts in the selected currency.
- [ ] Build Orders as its own page with stable columns, status filters, and order details dialog.
- [ ] Run tests and commit `feat: build user store and order pages`.

### Task 7: Store Administration UI

**Files:**
- Create: `frontend/src/pages/store-admin/index.tsx`
- Create: `frontend/src/pages/store-admin/product-dialog.tsx`
- Create: `frontend/src/pages/store-admin/channel-dialog.tsx`
- Create: `frontend/src/pages/store-admin/redemption-dialog.tsx`
- Create: `frontend/tests/store-admin.test.ts`

- [ ] Write failing tests for four animated admin tabs, balance and plan product fields, multiple quota windows, custom-hour validation, built-in and custom channels, URL/upload icon modes, order completion/cancellation, code generation, and code list secrecy.
- [ ] Run the focused tests and confirm failures.
- [ ] Implement SWR lists and optimistic mutations with rollback and toast errors.
- [ ] Implement modal forms with Group data, exact minor conversion, derived actual received, image validation, and one-to-20 code generation.
- [ ] Keep payment channels on their own full-width admin panel and let all lists expand naturally.
- [ ] Run tests and commit `feat: build store administration`.

### Task 8: Documentation, Screenshots, And Verification

**Files:**
- Create: `docs/content/docs/dashboard/store.mdx`
- Create: `docs/content/docs/dashboard/store.zh.mdx`
- Create: `docs/content/docs/dashboard/store.zh-TW.mdx`
- Create: `docs/content/docs/dashboard/store.ja.mdx`
- Create: `docs/public/images/en/store.webp`
- Create: `docs/public/images/zh/store.webp`
- Modify: `docs/content/docs/dashboard/meta.json`
- Modify: `docs/content/docs/dashboard/meta.zh.json`
- Modify: `docs/content/docs/dashboard/meta.zh-TW.json`
- Modify: `docs/content/docs/dashboard/meta.ja.json`

- [ ] Add the Store operation page in all four locales using Simplified Technical English and canonical Product, Provider, Channel, Group, and endpoint nouns.
- [ ] Build the frontend with `cd frontend; bun run build` and run all frontend tests.
- [ ] Run `cargo fmt --check`, focused Store tests, then the full Rust test set that does not start PostgreSQL.
- [ ] Start the local application with SQLite, capture English and Simplified Chinese Store screenshots, and verify desktop/mobile layouts and browser errors.
- [ ] Run `cd docs; bun install; bun run build`.
- [ ] Run `rg -n "实到|实际到账" frontend docs spec` and require zero active Store UI matches.
- [ ] Commit `docs: document store billing`.

### Task 9: Review And Deployment

**Files:**
- Verify: every file listed in Tasks 1 through 8

- [ ] Review the final diff against every SB requirement and run `git diff --check`.
- [ ] Run the repository's release build without PostgreSQL.
- [ ] Back up the current production database and verify that the backup opens and passes its database integrity check.
- [ ] Build and deploy the release with the project deployment script, restart the configured service, and verify `https://lynshen.org` Store routes and health.
- [ ] Do not delete the pre-deployment backup. Record the deployed commit and rollback command.
