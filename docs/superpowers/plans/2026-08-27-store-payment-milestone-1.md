# Store Payment Milestone 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the manual Store order flow with exact-money Alipay, WeChat Pay v3, and Stripe payments, protected redemption codes, reconciliation, recovery accounting, and gated plan quotas.

**Architecture:** Keep `StoreBillingStore` as the transactional facade, but split provider protocols, cryptography, reconciliation, quota admission, and retention into focused modules. Persist every payment attempt and provider event before fulfillment, enforce transitions and economic recovery in the database, and expose one Store-serving Primary through the existing dashboard router. Keep every Channel disabled until credentials, capability evidence, compliance, privacy, and release gates pass.

**Tech Stack:** Rust 2024, Axum 0.8, SeaORM/SQLx, SQLite for local tests, PostgreSQL SQL compiled and tested only in isolated CI, Reqwest/Rustls, RSA-SHA256, AES-256-GCM, HMAC-SHA256, Ed25519, React 19, TypeScript, SWR, Tailwind, Framer Motion, Bun.

---

### Task 1: Freeze The Formal Store Contract

**Files:**
- Modify: `spec/store-billing.spec.md`
- Modify: `docs/superpowers/specs/2026-08-27-store-billing-design.md`

- [ ] **Step 1: Replace the legacy three-state order contract**

Define `payment_state`, `fulfillment_state`, `refund_state`, `dispute_state`, versioned attempts, immutable quotes, economic recovery, official Channel capability, credential versions, redemption recovery, quota gate, availability profile, and retention invariants. Delete SB-O-7 through SB-O-11 semantics that allow an Admin to complete an unpaid order.

```text
payment_state = unpaid | processing | paid | failed | closed
fulfillment_state = pending | fulfilled | failed | reversed
POST /api/dashboard/store/admin/orders/{id}/complete = absent
```

- [ ] **Step 2: Add exact API and error contracts**

List quote, attempt, callback, return, poll, refund, reveal, export, reconciliation, capability, compliance, privacy, and quota-gate endpoints. Specify allowed transitions, idempotency keys, role checks, reauthentication scopes, CSRF origin checks, response cache headers, rate limits, and provider-event mismatch errors.

```text
POST /api/dashboard/store/orders/{id}/attempts
POST /api/store/callbacks/{channel_id}
POST /api/dashboard/store/admin/orders/{id}/refunds
POST /api/dashboard/store/admin/redemption-codes/{id}/reveal
```

- [ ] **Step 3: Verify specification completeness**

Run:

```powershell
rg -n "complete|manual paid|TBD|TODO|placeholder" spec/store-billing.spec.md
rg -n "Alipay|WeChat|Stripe|payment_state|fulfillment_state|economic recovery|privacy|RTO|RPO" spec/store-billing.spec.md
```

Expected: no active manual-completion requirement and no placeholder; every required invariant has at least one match.

- [ ] **Step 4: Commit the contract**

```powershell
git add spec/store-billing.spec.md docs/superpowers/specs/2026-08-27-store-billing-design.md docs/superpowers/plans/2026-08-27-store-payment-milestone-1.md
git commit -m "docs: freeze store payment milestone one"
```

### Task 2: Add Payment Cryptography And Secret Boundaries

**Files:**
- Modify through CLI: `Cargo.toml`
- Modify through CLI: `Cargo.lock`
- Create: `src/store_billing/crypto.rs`
- Create: `src/store_billing/credentials.rs`
- Modify: `src/store_billing/mod.rs`
- Test: unit tests in both new modules

- [ ] **Step 1: Add dependencies with Cargo**

```powershell
cargo add aes-gcm ed25519-dalek hmac rsa subtle url zeroize
```

Expected: Cargo resolves versions compatible with Rust 2024 and updates both Cargo files.

- [ ] **Step 2: Write failing key-ring and signature tests**

Cover AES-256-GCM associated data, wrong-key rejection, active/prior key lookup, RSA-SHA256 sign/verify, Stripe HMAC constant-time verification, Ed25519 key IDs, and secret redaction.

```rust
#[test]
fn ciphertext_is_bound_to_record_identity() {
    let encrypted = ring.encrypt("credential:channel-a:v2", b"secret").unwrap();
    assert!(ring.decrypt("credential:channel-b:v2", &encrypted).is_err());
}
```

- [ ] **Step 3: Run tests and confirm failure**

```powershell
cargo test store_billing::crypto store_billing::credentials --lib
```

Expected: failure because the modules and key-ring API do not exist.

- [ ] **Step 4: Implement the minimal cryptographic APIs**

Expose `PaymentKeyRing`, `EncryptedSecret`, `CredentialVersion`, `ReauthScope`, RSA helpers, HMAC verification, and Ed25519 signer/verifier. Never implement protocol parsing in this module.

```rust
pub trait SecretCipher: Send + Sync {
    fn encrypt(&self, aad: &str, plaintext: &[u8]) -> Result<EncryptedSecret, CryptoError>;
    fn decrypt(&self, aad: &str, value: &EncryptedSecret) -> Result<zeroize::Zeroizing<Vec<u8>>, CryptoError>;
}
```

- [ ] **Step 5: Verify and commit**

```powershell
cargo test store_billing::crypto store_billing::credentials --lib
git add Cargo.toml Cargo.lock src/store_billing
git commit -m "feat: add store payment cryptography"
```

### Task 3: Migrate The Store Payment Schema

**Files:**
- Create: `src/migration/m20260827_000051_store_payment_core.rs`
- Modify: `src/migration/mod.rs`
- Modify: `src/migration/m20260827_000049_store_billing.rs`
- Test: migration tests in `src/migration/m20260827_000051_store_payment_core.rs`
- Test: `tests/store_payment_migration.rs`

- [ ] **Step 1: Write failing SQLite migration tests**

Assert tables for credential versions, compliance confirmations, merchant capabilities, attempts, events, refunds, recovery claims, balance holds, reconciliation leases, privacy records, access audits, deletion runs, availability leases, quota gates, buckets, reservations, and admission keys. Assert that obsolete manual completion state and legacy writable secret columns are absent after migration.

```rust
assert_table(&db, "store_payment_attempts").await;
assert_column_absent(&db, "store_payment_channels", "config_secret").await;
assert_trigger_rejects(&db, "paid_to_unpaid").await;
```

- [ ] **Step 2: Run the focused test and confirm failure**

```powershell
cargo test m20260827_000051_store_payment_core --lib
cargo test --test store_payment_migration
```

Expected: failure because migration `000051` is absent.

- [ ] **Step 3: Generate the migration shell when SeaORM CLI is available**

```powershell
sea-orm-cli migrate generate store_payment_core
```

If the repository layout is unsupported by the CLI, keep the CLI error in the task log and create only the minimal migration module manually.

- [ ] **Step 4: Implement backend-specific DDL and preflight**

Use SQLite triggers and PostgreSQL constraints/functions for immutable quotes, transition legality, one economic recovery per provider claim, `reserved + recovered <= original`, unique provider transactions, and monotonic lease epochs. Migrate legacy completed orders to `paid/fulfilled`; migrate pending or cancelled orders to version-1 `closed/pending`; disable all Channels.

```text
legacy completed -> contract_version=1, payment_state=paid, fulfillment_state=fulfilled
legacy pending|cancelled -> contract_version=1, payment_state=closed, fulfillment_state=pending
```

- [ ] **Step 5: Run SQLite tests and compile PostgreSQL branches without a server**

```powershell
cargo test m20260827_000051_store_payment_core --lib
cargo test --test store_payment_migration
cargo check --all-targets
```

Expected: all commands pass; no PostgreSQL process starts.

- [ ] **Step 6: Commit**

```powershell
git add src/migration tests/store_payment_migration.rs
git commit -m "feat: add store payment schema"
```

### Task 4: Implement Quotes, Orders, Attempts, And State Transitions

**Files:**
- Replace: `src/store_billing/models.rs`
- Create: `src/store_billing/order.rs`
- Create: `src/store_billing/state_machine.rs`
- Modify: `src/store_billing/store.rs`
- Test: `tests/store_payment_state.rs`

- [ ] **Step 1: Write failing transition and idempotency tests**

Cover immutable product snapshots, 30-minute expiry, order creation idempotency, payload mismatch, open-order cap, attempt persistence before checkout, late payment, duplicate provider transaction, callback/close races, and `paid/pending` crash recovery.

```rust
assert_eq!(transition(PaymentState::Paid, PaymentEvent::ProviderFailed), Err(TransitionError::Stale));
assert_eq!(create_twice_same_key().await.order_id, first.order_id);
```

- [ ] **Step 2: Confirm tests fail**

```powershell
cargo test --test store_payment_state
```

- [ ] **Step 3: Implement pure transitions and transactional persistence**

Define `OrderQuoteV2`, `PaymentAttempt`, `PaymentEvent`, `FulfillmentState`, and compare-and-swap revisions. Persist an attempt before calling an adapter. A timeout or 5xx marks the attempt ambiguous and requires query before retry.

```rust
pub fn apply_payment_event(current: PaymentState, event: PaymentEventKind) -> Result<PaymentState, TransitionDecision>;
pub async fn create_attempt(&self, user_id: &str, order_id: &str, key: &str) -> Result<PaymentAttempt, StoreBillingError>;
```

- [ ] **Step 4: Remove manual completion behavior**

Delete `complete_order`, its handler, route, tests, frontend action, and locale text. Keep fulfillment callable only from verified payment/reconciliation or redemption transactions.

```powershell
rg -n "complete_order|admin/orders/.*/complete|manual.*paid" src tests frontend spec
```

Expected: zero implementation matches.

- [ ] **Step 5: Verify and commit**

```powershell
cargo test --test store_payment_state
cargo test --test store_billing
git add src/store_billing tests
git commit -m "feat: add store payment state machine"
```

### Task 5: Implement Official Payment Adapters

**Files:**
- Create: `src/store_billing/adapters/mod.rs`
- Create: `src/store_billing/adapters/alipay.rs`
- Create: `src/store_billing/adapters/wechat.rs`
- Create: `src/store_billing/adapters/stripe.rs`
- Create: `src/store_billing/payment.rs`
- Test: `tests/store_payment_adapters.rs`
- Test fixtures: `tests/fixtures/store_payments/`

- [ ] **Step 1: Write provider-contract tests from official fixtures**

Cover Alipay canonical parameter ordering and RSA2 verification, WeChat Pay v3 Authorization and AES-GCM resource decryption, Stripe Checkout request encoding and Webhook HMAC verification, trusted return URLs, amount/currency/merchant mismatch, and async event classification.

```rust
#[async_trait]
pub trait PaymentAdapter {
    async fn checkout(&self, request: CheckoutRequest) -> Result<CheckoutAction, AdapterError>;
    async fn query(&self, attempt: &PaymentAttempt) -> Result<QueryResult, AdapterError>;
    async fn refund(&self, request: RefundRequest) -> Result<RefundResult, AdapterError>;
    fn verify_callback(&self, request: CallbackRequest<'_>) -> Result<VerifiedProviderEvent, AdapterError>;
}
```

- [ ] **Step 2: Confirm fixture tests fail**

```powershell
cargo test --test store_payment_adapters
```

- [ ] **Step 3: Implement adapter boundaries**

Build provider requests from immutable attempt data. Parse only allow-listed response fields. Return redirect, QR, or Stripe Checkout actions. Do not log callback bodies, signature headers, credentials, or full provider responses.

- [ ] **Step 4: Verify no AGPL source was copied**

```powershell
rg -n "QuantumNous|new-api|Epay|PayMoney|GetPayConfig" src tests frontend
```

Expected: no copied identifiers or source attribution inside implementation files; the design review record remains the only architecture reference.

- [ ] **Step 5: Verify and commit**

```powershell
cargo test --test store_payment_adapters
git add src/store_billing tests/fixtures tests/store_payment_adapters.rs
git commit -m "feat: add official store payment adapters"
```

### Task 6: Implement Callback Ingestion And Reconciliation

**Files:**
- Create: `src/store_billing/callbacks.rs`
- Create: `src/store_billing/reconciliation.rs`
- Modify: `src/store_billing/store.rs`
- Modify: `src/app.rs`
- Test: `tests/store_payment_callbacks.rs`
- Test: `tests/store_reconciliation.rs`

- [ ] **Step 1: Write failing callback race tests**

Send 20 concurrent copies of one event and assert one event row, one payment transition, and one fulfillment. Cover payment-before-dispute, dispute-before-payment, refund-before-payment, dispute during `refund_pending`, chargeback during `refund_pending`, refund success after chargeback, and stale terminal events.

```rust
assert_eq!(count_fulfillment_ledger(&db, order_id).await, 1);
assert_eq!(count_provider_event(&db, event_id).await, 1);
```

- [ ] **Step 2: Write failing reconciler lease tests**

Cover exclusive lease epoch, stale worker rejection, due-order backoff, `paid/pending` fulfillment, ambiguous attempt query, provider mismatch, alert thresholds, and shutdown after lease loss.

- [ ] **Step 3: Implement ingestion and reconciliation**

Read at most 128 KiB in five seconds, store digest plus encrypted body, verify before applying, and reply using the provider's required acknowledgement. Reconciliation queries before any ambiguous retry and applies every result through the same event transaction.

- [ ] **Step 4: Verify and commit**

```powershell
cargo test --test store_payment_callbacks --test store_reconciliation
git add src/store_billing src/app.rs tests
git commit -m "feat: reconcile verified store payments"
```

### Task 7: Implement Refunds, Disputes, Holds, And Settlement Evidence

**Files:**
- Create: `src/store_billing/recovery.rs`
- Create: `src/store_billing/settlement.rs`
- Modify: `src/store_billing/store.rs`
- Test: `tests/store_economic_recovery.rs`

- [ ] **Step 1: Write failing concurrent recovery tests**

Cover balance reserve, concurrent spend, refund rejection for fulfilled plans, dispute/chargeback overlap, duplicate and reversed events, provider refusal, frozen balance release, negative debt, payment hold, settlement reimport, and unmatched lines.

```rust
assert!(reserved + recovered <= original_credit);
assert_eq!(ledger_rows("recovery:provider-claim-1").await, 1);
```

- [ ] **Step 2: Confirm failure**

```powershell
cargo test --test store_economic_recovery
```

- [ ] **Step 3: Implement database-first recovery**

Reserve recoverable balance before provider refund. Share one economic limit across refund, dispute, and chargeback claims. Use unique claim and ledger keys. Keep payment hold separate from ordinary balance spending but block Store checkout, redemption consumption, and plan admission.

- [ ] **Step 4: Verify and commit**

```powershell
cargo test --test store_economic_recovery
git add src/store_billing tests/store_economic_recovery.rs
git commit -m "feat: add store economic recovery"
```

### Task 8: Protect Recoverable Redemption Codes

**Files:**
- Create: `src/store_billing/redemption.rs`
- Modify: `src/store_billing/store.rs`
- Modify: `src/dashboard_handlers/store_billing.rs`
- Test: `tests/store_redemption_security.rs`

- [ ] **Step 1: Write failing Crockford and reveal tests**

Cover v2 alphabet, case and hyphen normalization, legacy v1 digest lookup, encryption with record-bound AAD, one-time redemption, full generation response, reveal with five-minute scope, export headers, cooldown, expiry cleanup, and deletion of ciphertext after use.

```rust
assert_eq!(normalize_code("abcd-efgh"), "ABCDEFGH");
assert_header("Cache-Control", "no-store");
assert_header("X-Content-Type-Options", "nosniff");
```

- [ ] **Step 2: Confirm failure**

```powershell
cargo test --test store_redemption_security
```

- [ ] **Step 3: Implement generation, reveal, export, redemption, and cleanup**

Store digest, hint, version, and encrypted full code. Delete ciphertext on use, revocation, or within 24 hours after expiry. Audit reveal, copy, and export without writing plaintext to logs.

- [ ] **Step 4: Verify and commit**

```powershell
cargo test --test store_redemption_security --test store_billing
git add src/store_billing src/dashboard_handlers tests
git commit -m "feat: protect store redemption codes"
```

### Task 9: Gate Plan Products And Admission

**Files:**
- Create: `src/store_billing/quota.rs`
- Create: `src/store_billing/admission_token.rs`
- Create: `src/store_billing/quota_gate.rs`
- Modify: `src/store_billing/store.rs`
- Modify: `src/handlers/billing.rs`
- Test: `tests/store_plan_quota.rs`
- Test: `tests/store_admission_token.rs`

- [ ] **Step 1: Write failing exact quota tests**

Cover immutable `N/D`, nano USD to CNY fen conversion, deterministic bucket locks, concurrent reserve, exact settle/release, generation replacement, all window boundaries, above-reserve anomaly, and payment hold.

- [ ] **Step 2: Write failing Ed25519 token tests**

Cover key ID, audience, request/reservation binding, 30-second TTL, two-minute skew, durable same-node replay marker, cross-node replay, rotation, prior-key verification, and retirement refusal.

- [ ] **Step 3: Implement gate and admission modules**

SQLite uses `BEGIN IMMEDIATE`, WAL, five-second busy timeout, and a persistent compatibility fingerprint. A `pending` or `failed` gate blocks plan product enablement, plan checkout, plan redemption generation, fulfillment, and admission. PostgreSQL uses row locks in deterministic bucket order.

```rust
pub async fn reserve(&self, input: AdmissionRequest) -> Result<SignedAdmission, QuotaError>;
pub async fn settle(&self, reservation_id: &str, actual_nano_usd: i128) -> Result<(), QuotaError>;
pub async fn release(&self, reservation_id: &str) -> Result<(), QuotaError>;
```

- [ ] **Step 4: Run local SQLite tests only**

```powershell
cargo test --test store_plan_quota --test store_admission_token
```

Expected: pass without starting PostgreSQL. Isolated PostgreSQL load and five-Replica fault drills remain a production gate, not a local command.

- [ ] **Step 5: Commit**

```powershell
git add src/store_billing src/handlers/billing.rs tests
git commit -m "feat: gate store plan quota admission"
```

### Task 10: Expose Payment And Operations APIs

**Files:**
- Replace: `src/dashboard_handlers/store_billing.rs`
- Modify: `src/dashboard_handlers/mod.rs`
- Modify: `src/app.rs`
- Modify: `tests/api/store_billing.rs`
- Create: `tests/api/store_payments.rs`

- [ ] **Step 1: Write failing route and authorization tests**

Cover user order creation idempotency, attempt creation, polling, return state, public callbacks, Admin Channel configuration, capability and compliance records, refunds, reconciliation actions, reveal/export, privacy records, deletion status, reauthentication, CSRF origin, Primary-only writes, and absence of manual complete.

```rust
assert_eq!(request(POST, "/api/dashboard/store/admin/orders/o1/complete").status(), StatusCode::NOT_FOUND);
assert_eq!(cross_origin_attempt.status(), StatusCode::FORBIDDEN);
```

- [ ] **Step 2: Confirm route failures**

```powershell
cargo test --test api store_payments
```

- [ ] **Step 3: Implement handlers and process limits**

Use existing dashboard auth and error envelopes. Add five-minute scoped reauthentication, trusted public-origin URL construction, process-wide order/poll/callback buckets, 128 KiB callback limit, no-store sensitive responses, and Primary lease checks.

- [ ] **Step 4: Verify and commit**

```powershell
cargo test --test api store_billing
cargo test --test api store_payments
git add src/dashboard_handlers src/app.rs tests/api
git commit -m "feat: expose verified store payments"
```

### Task 11: Replace Store And Admin Payment UI

**Files:**
- Modify: `frontend/src/lib/store-api.ts`
- Modify: `frontend/src/pages/store/index.tsx`
- Modify: `frontend/src/pages/store/order-summary.tsx`
- Modify: `frontend/src/pages/store/payment-methods.tsx`
- Modify: `frontend/src/pages/store-admin/admin-panels.tsx`
- Modify: `frontend/src/pages/store-admin/channel-dialog.tsx`
- Modify: `frontend/src/pages/store-admin/redemption-dialog.tsx`
- Modify: `frontend/src/pages/orders.tsx`
- Modify: `frontend/src/locales/en.json`
- Modify: `frontend/src/locales/zh.json`
- Modify: `frontend/src/locales/zh-TW.json`
- Modify: `frontend/src/locales/ja.json`
- Test: `frontend/tests/store-payment-flow.test.ts`
- Test: `frontend/tests/store-admin.test.ts`

- [ ] **Step 1: Write failing frontend tests**

Cover effective Channel availability, WeChat QR action, Alipay/Stripe redirect action, stable order summary, automatic SWR polling, paid/pending fulfillment, payment failure, expired attempts, plan gate disablement, refund state, full generated code display, scoped reveal/export, skeletons, optimistic mutation rollback, and no manual paid control.

```ts
expect(source).not.toContain('/complete')
expect(source).toContain('mutate(')
expect(source).toContain('useSWR')
```

- [ ] **Step 2: Confirm failure**

```powershell
Set-Location frontend
bun test tests/store-payment-flow.test.ts tests/store-admin.test.ts
```

- [ ] **Step 3: Implement the real payment flow**

Create the local order first, then request a provider attempt. Render `redirect`, `qr`, and `stripe_checkout` actions. Poll with SWR until payment and fulfillment are terminal. Keep payment methods as one independent row. Show actionable unavailable reasons without exposing internal configuration.

- [ ] **Step 4: Implement operations UI**

Replace manual completion with refund, retry fulfillment, capability evidence, compliance, credential version, reconciliation, dispute, privacy, deletion, and quota-gate status controls. Sensitive operations require reauthentication and use `Cache-Control: no-store` responses.

- [ ] **Step 5: Verify and commit**

```powershell
bun test tests/store-payment-flow.test.ts tests/store-admin.test.ts
bun run build
Set-Location ..
git add frontend
git commit -m "feat: build real store payment flows"
```

### Task 12: Documentation, Retention, Availability, And Release Verification

**Files:**
- Modify: `docs/content/docs/dashboard/store.mdx`
- Modify: `docs/content/docs/dashboard/store.zh.mdx`
- Modify: `docs/content/docs/dashboard/store.zh-TW.mdx`
- Modify: `docs/content/docs/dashboard/store.ja.mdx`
- Modify: `docs/public/images/en/store.webp`
- Modify: `docs/public/images/zh/store.webp`
- Create: `src/store_billing/availability.rs`
- Create: `src/store_billing/retention.rs`
- Test: `tests/store_availability.rs`
- Test: `tests/store_retention.rs`

- [ ] **Step 1: Write failing availability and retention tests**

Cover profile/backend mismatch, stale Store lease, monotonic epoch fencing, SQLite off-host backup requirements, callback-window/RTO comparison, current privacy record, regional target allow-list, role-scoped evidence reads, daily bounded deletion, legal hold expiry, and retention-failure checkout pause.

- [ ] **Step 2: Implement runtime production gates**

Expose a read-only readiness report. Keep payment Channels disabled when availability, capability, compliance, privacy, key-ring, exchange-rate, quota, or retention requirements fail. Do not implement automatic production Channel enablement.

- [ ] **Step 3: Update all four documentation locales and screenshots**

Use Simplified Technical English. Document official Channel setup, callback registration, test transaction, refund, reconciliation, redemption reveal, plan gate, retention, and failure recovery. Recapture English and Simplified Chinese Store screenshots.

- [ ] **Step 4: Run local verification without PostgreSQL**

```powershell
cargo fmt --check
cargo test --test store_billing --test store_payment_migration --test store_payment_state --test store_payment_adapters --test store_payment_callbacks --test store_reconciliation --test store_economic_recovery --test store_redemption_security --test store_plan_quota --test store_admission_token --test store_availability --test store_retention
cargo test --test api store
cargo check --all-targets
Set-Location frontend
bun test
bun run build
Set-Location ../docs
bun install
bun run build
Set-Location ..
git diff --check
```

Expected: every command passes; no local PostgreSQL process starts.

- [ ] **Step 5: Self-review against the specification**

```powershell
rg -n "admin/orders/.*/complete|complete_order|manual paid" src tests frontend
rg -n "config_secret|signature.*log|raw.*callback.*log" src/store_billing src/dashboard_handlers
git status --short
```

Expected: no obsolete completion path, no raw secret logging, and only intended tracked changes.

- [ ] **Step 6: Commit the verified implementation**

```powershell
git add src tests frontend docs spec Cargo.toml Cargo.lock
git commit -m "docs: verify store payment milestone one"
```

Production deployment, merchant credential entry, provider sandbox calls, provider callback registration, PostgreSQL drills, and real-money transactions require separate explicit authorization. They are not execution steps in this plan.
