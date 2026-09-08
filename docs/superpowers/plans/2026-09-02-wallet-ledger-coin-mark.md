# Wallet Ledger and Coin Mark Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the wallet's nested redemption layout with a balance ledger and graphical Coin mark.

**Architecture:** Add a user-scoped ledger read method and dashboard route. Render the ledger with SWR on Wallet, keep orders behind a Wallet tab, and centralize Coin icon rendering in reusable frontend components.

**Tech Stack:** Rust, Axum, SeaORM database wrapper, React, TypeScript, SWR, lucide-react, Tailwind.

---

### Task 1: Ledger contract

**Files:**
- Modify: `spec/coin-wallet-navigation.spec.md`
- Modify: `frontend/tests/coin-wallet.test.ts`

- [ ] Add assertions for the ledger endpoint, graphical Coin mark, and Wallet tab structure.
- [ ] Run the focused test and record the expected failure because the route and components do not exist yet.

### Task 2: User-scoped ledger API

**Files:**
- Modify: `src/users/mod.rs`
- Modify: `src/users/store.rs`
- Modify: `src/dashboard_handlers/store_billing.rs`
- Modify: `src/dashboard_handlers/mod.rs`
- Modify: `src/app.rs`

- [ ] Add a serializable `BillingLedgerEntry` with the six fields in CN-14.
- [ ] Add `UserStore::list_billing_ledger` with a bounded limit of 50 and deterministic descending ordering.
- [ ] Add `GET /api/dashboard/wallet/ledger` using the current session user and map storage failures to `internal_error`.
- [ ] Verify the query filters by the authenticated user ID before returning rows.

### Task 3: Coin mark and ledger UI

**Files:**
- Create: `frontend/src/components/coin-mark.tsx`
- Create: `frontend/src/components/coin-amount.tsx`
- Modify: `frontend/src/lib/api.ts`
- Modify: `frontend/src/pages/wallet.tsx`
- Modify: `frontend/src/pages/store/redemption-panel.tsx`
- Modify: `frontend/src/locales/en.json`
- Modify: `frontend/src/locales/zh.json`
- Modify: `frontend/src/locales/zh-TW.json`
- Modify: `frontend/src/locales/ja.json`

- [ ] Add a lucide-based Coin mark with accessible label and size variants.
- [ ] Add `CoinAmount` to render the mark beside numeric Coin values without a textual `C` prefix.
- [ ] Add the SWR ledger fetch and loading skeleton to Wallet.
- [ ] Render ledger rows with positive/negative styling and localized kind labels.
- [ ] Convert the redemption area to one compact form row and add Wallet tabs for ledger and orders.

### Task 4: Replace plain Coin prefixes

**Files:**
- Modify: `frontend/src/pages/dashboard.tsx`
- Modify: `frontend/src/pages/admin-usage.tsx`
- Modify: `frontend/src/pages/orders.tsx`
- Modify: `frontend/src/components/user-center-menu.tsx`
- Modify: `frontend/src/pages/model-marketplace.tsx`
- Modify: `frontend/src/pages/public-marketplace.tsx`
- Modify: `frontend/src/lib/store-money.ts`

- [ ] Expose numeric Coin formatter variants for components that need an icon.
- [ ] Replace plain `C` prefixes with `CoinAmount` or `CoinMark` plus numeric text.
- [ ] Preserve payment settlement currency labels where they select CNY or USD.

### Task 5: Verification and release

- [ ] Run `npx tsc -p frontend/tsconfig.app.json --noEmit`.
- [ ] Run `npm run build` from `frontend`.
- [ ] Run `cargo check --no-default-features`.
- [ ] Run `git diff --check`.
- [ ] Deploy by building a new release image, validating Caddy, and restarting only `monoize.service`.
- [ ] Verify `lynshen.org`, `sub.joinreso.com/health`, both services, and a 60-second clean log window.
