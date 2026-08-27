# LynShen Store, Payment, And Redemption Design

## 1. Scope

This design replaces the current Store layout and the manual order-completion flow.

The Store sells one-time balance recharge products and one-time plan products. It accepts redemption codes. It supports these payment adapters:

- Alipay official website and mobile website payment.
- WeChat Pay v3 Native and H5 payment.
- Stripe Checkout.
- A configurable HTTP payment adapter.

The first release does not support recurring subscriptions. An Admin cannot mark an unpaid order as paid.

Implementation is split into two separately approved milestones. Milestone 1 contains the Store layout, payment core, Alipay, WeChat Pay, Stripe, reconciliation, refunds, and redemption protection. Milestone 2 contains verified configurable HTTP templates. Milestone 2 cannot weaken or bypass Milestone 1 security rules.

The logged-in Model Marketplace redesign is a separate subsystem. It is not part of this design.

## 2. Approved Layout

The user Store follows the approved option A layout.

The page starts with one horizontal summary. It shows available balance, current-month usage, and the active plan.

One sliding segmented control selects Balance, Plan, or Redemption. The indicator moves between three fixed tracks. Reduced-motion mode removes transforms and entry animation.

Balance and Plan use a two-column purchase area:

- The left column contains product selection and custom amount input.
- The right column contains an order summary with stable dimensions.
- Payment methods occupy one independent full-width row at the bottom of the left column.

Redemption does not render payment methods or an order summary.

The Store Management page uses four animated child pages:

- Products.
- Payment Channels.
- Orders.
- Redemption Codes.

Products use separate compact lists for balance products and plan products. Orders and redemption codes do not render inside the Products page.

## 3. Currency Rules

The account ledger remains `users.balance_nano_usd`. The Store does not create a second CNY ledger.

Payment amounts use integer minor units. CNY uses fen. USD uses cents. Ledger values use integer nano USD. No payment calculation uses binary floating-point arithmetic.

Plan quota values use CNY as their stored base. The currency control changes presentation. Plan quota presentation uses whole units and round-half-away-from-zero.

The primary node requests `https://open.er-api.com/v6/latest/USD` at startup and no more than once per 15 minutes. It stores the exact decimal `rates.CNY`, the source timestamp, and the local refresh timestamp. Replicas only read the stored snapshot.

The response must be valid JSON with `result = success`, `base_code = USD`, a positive integer source timestamp, and one finite decimal `rates.CNY`. The rate must be between 1 and 20 CNY per USD and contain at most 18 fractional digits.

Let `R` be CNY per USD. USD cents convert to CNY fen as `round(cents * R)`. CNY fen convert to USD cents as `round(fen / R)`. The service never persists a rounded reciprocal.

USD cents convert to nano USD exactly as `cents * 10,000,000`. CNY fen convert to nano USD as `round(fen * 10,000,000 / R)`. Each formula rounds half away from zero only at its final integer result.

The decimal exchange rate contains at most 18 fractional digits. A checkout that needs conversion fails when the last successful refresh is older than 60 minutes or the source timestamp is older than 48 hours. A failed refresh retains the prior snapshot but does not extend its validity.

A candidate that differs from the active rate by more than five percent is quarantined and does not replace it. Admin can approve or reject the candidate. Admin can pause conversion-based checkout. Same-currency checkout can continue only when its reward and entitlement calculations do not require the rate.

The service retains accepted snapshots for 30 days. Admin can restore the immediately prior accepted snapshot only while its original age limits still pass. Restore does not change its source or refresh timestamp. If no accepted snapshot is valid, conversion-based checkout remains paused.

At startup, a network failure uses the stored active snapshot only while both age limits pass. Without a valid snapshot, dependent checkout returns HTTP 503. The process remains available for non-Store traffic.

Conversion happens once during order quotation. The service converts from the product currency to the settlement currency, then rounds half away from zero to the settlement minor unit. The immutable quote stores the unrounded decimal inputs, rounded settlement amount, rate, and timestamps.

Alipay and WeChat Pay settle in CNY. When the Store currency control shows USD, the order summary also shows the exact CNY amount that the gateway will charge.

Stripe Checkout can charge CNY or USD when the Stripe account supports the selected currency.

The Stripe adapter rejects a CNY charge below 300 fen and a USD charge below 50 cents. Alipay and WeChat reject a CNY charge below 1 fen. These adapter limits apply after conversion.

The default custom-recharge range is 1,000 through 100,000,000 minor units for each currency. Admin can configure a separate positive minimum and maximum for CNY and USD. A product price must be positive, and checkout also applies the selected adapter minimum.

Each configurable HTTP Channel declares an allow-list containing `CNY`, `USD`, or both.

A configurable HTTP Channel declares positive minimum and maximum minor amounts for each allowed currency. Order creation applies the stricter of the Store product limit and the adapter limit.

The customer charge equals the quoted product price. The first release does not add tax, gateway fees, or payment surcharges. The merchant absorbs provider fees. Callback comparison uses the gross customer charge, not the provider net settlement.

Order creation stores an immutable exchange-rate snapshot. A later rate refresh does not change an existing order amount.

### 3.1 Product And Plan Lifecycle

Product edits affect only later orders. Every order stores an immutable product, price, reward, duration, Group, and quota snapshot.

Disabling or editing a product does not change an existing unpaid order. That order remains payable until its 30-minute expiration. An emergency Admin action can close every unpaid order for one product before changing it.

A referenced product cannot be physically deleted. Admin can disable it. Unreferenced products can be deleted with an audit record.

A plan duration is 3,600 through 31,536,000 seconds. It starts when fulfillment succeeds and ends exactly one duration later. An expired plan grants no access or quota.

Buying or redeeming another plan replaces the active plan immediately. Unused time and quota do not carry forward. The order summary shows this replacement before checkout.

All quota rules in a plan apply concurrently. Five-hour, 12-hour, and custom-hour windows are rolling. Day, week, and month windows use Asia/Shanghai calendar boundaries, with Monday as the first day of a week.

Quota checks use settled request charges. A request is denied before routing when any quota has no remaining amount. One already-routed request can cross a limit; later requests are denied.

## 4. Payment Core

The payment core owns order state, payment attempts, callback events, fulfillment, refunds, and audit records.

Each adapter implements these operations:

1. `create_checkout`
2. `query_payment`
3. `verify_callback`
4. `refund_payment`

An adapter cannot write a user balance or activate a plan. Only the payment core can fulfill an order.

Every provider mutation uses a stable idempotency key. After a request may have reached a provider, the payment core treats every timeout, disconnect, HTTP 5xx, and unrecognized response as ambiguous. It calls `query_payment` or the matching refund query before any retry. Only a verified not-found result permits another mutation.

Creating a checkout returns one of these actions:

- `redirect`: an HTTPS URL.
- `qr`: QR payload data plus an expiration time.
- `form`: a signed form action and an allow-listed set of hidden fields.

The browser does not receive merchant credentials or callback verification secrets.

## 5. Official Adapters

### 5.1 Alipay

The Alipay adapter supports computer website payment and mobile website payment. It signs requests with RSA2.

Admin configuration contains:

- App ID.
- Production or sandbox environment.
- Merchant private key.
- Alipay public key or certificate configuration.

The adapter selects the website or mobile product from the request context. The callback verifier checks the Alipay signature, App ID, seller identity, order number, amount, currency, and success status.

### 5.2 WeChat Pay

The WeChat adapter supports Native QR payment and H5 payment. It uses WeChat Pay API v3.

Admin configuration contains:

- Merchant ID.
- App ID.
- API v3 key.
- Merchant certificate serial.
- Merchant private key.

The adapter verifies platform certificate signatures and decrypts callback resources. It checks the merchant ID, App ID, order number, amount, currency, and success status.

### 5.3 Stripe

Stripe uses hosted Stripe Checkout. LynShen does not collect card numbers.

Admin configuration contains:

- Secret key.
- Publishable key.
- Webhook signing secret.
- Production or test environment.

The adapter creates a Checkout Session with the Store order number as the idempotency key and metadata reference. The webhook verifier checks the Stripe signature, Checkout Session, PaymentIntent, amount, currency, and payment status.

The Store can show card, Apple Pay, Google Pay, and other methods that the Stripe account enables. LynShen does not claim that a method is available until Stripe returns it.

## 6. Configurable HTTP Adapter

The configurable HTTP adapter is a separate milestone after the three official adapters. Its unfinished or disabled state does not block Alipay, WeChat Pay, or Stripe release.

Production Channels use versioned server allow-listed templates. Admin supplies endpoints, credentials, and documented template values. Adding a new production template requires adapter fixtures, callback tests, query tests, refund tests, and one controlled provider transaction.

Admin can create an unverified draft mapping for local validation. A draft cannot be enabled for production and cannot receive a public callback URL.

Verified templates can support JSON and form-encoded checkout requests.

Admin configuration defines:

- HTTPS checkout endpoint.
- HTTP method.
- Request content type.
- Request field mapping.
- Signature algorithm.
- Signature destination.
- Response field mapping.
- Callback field mapping.
- Success status values.
- Allowed settlement currencies.

Request templates can reference only documented order variables. The adapter does not execute JavaScript, shell commands, SQL, or arbitrary server-side code.

The initial signature allow-list is RSA2 and HMAC-SHA256. MD5 is disabled unless deployment configuration explicitly enables legacy MD5 and the Admin confirms a persistent warning for that Channel.

The response mapping can produce a redirect URL, QR payload, or signed form. A mapped redirect or form action must use HTTPS.

The callback configuration must identify the provider transaction ID, merchant order number, amount, currency, status, timestamp, nonce, and signature.

Outbound endpoints must use HTTPS on port 443. They cannot contain user information, a fragment, or an IP-literal host. Redirects are disabled.

Before every connection and retry, the service resolves the hostname. It rejects the request if any resolved IPv4 or IPv6 address is loopback, private, link-local, multicast, unspecified, reserved, documentation-only, or otherwise non-public. The connection pins a validated address while preserving the original TLS server name.

Checkout requests use a two-second connection timeout and a five-second total timeout. Responses are limited to 64 KiB. A template contains at most 64 fields, a field name contains at most 64 bytes, one expanded value contains at most 2 KiB, and the complete expanded body contains at most 32 KiB.

Amount fields use either integer minor units or a decimal with exactly two fractional digits. The Channel configuration selects one representation. The adapter does not infer a representation from a response.

Every checkout request includes a stable idempotency key. DNS rejection, validation failure, or connection failure before request bytes are sent fails without an automatic retry.

After any request byte is sent, a timeout, disconnect, malformed response, HTTP 5xx, or unmapped response is ambiguous. The adapter calls `query_payment` with the merchant order number before retrying. It retries only after a verified not-found result. A template without a query operation cannot be enabled for production.

Generic callbacks accept a timestamp skew of at most five minutes. A Channel that cannot supply a signed timestamp and nonce cannot be enabled for production.

## 7. Secret Encryption

Payment credentials and recoverable redemption codes use authenticated encryption.

The service loads a key ring from deployment configuration. The key ring identifies one active 256-bit key and zero or more decryption-only prior keys. The database stores the key ID, random nonce, ciphertext, and format version.

Encryption uses XChaCha20-Poly1305. Associated data includes the table name, row ID, and field name.

Admin reads never return a saved payment credential. Admin can only replace it.

Replacing a Channel credential creates an immutable encrypted credential version. It does not overwrite the prior version. Each payment attempt stores its credential-version ID and adapter account identity.

A retired credential version remains decryptable until every referenced order is outside the provider refund, dispute, callback, and reconciliation window. Deletion requires a preflight that finds zero active references and an audit record.

If encrypted rows exist and no matching key is available, the service rejects payment creation and redemption-code reveal. It does not delete or overwrite encrypted values.

The deployment backup must include the database and the matching key ring. Restoring only one of them is not a valid recovery.

Key rotation adds a new active key, keeps prior keys decrypt-only, and re-encrypts rows in bounded idempotent batches. A prior key cannot be removed until the database reports zero ciphertext rows for its key ID and a restore drill passes with the new key ring.

## 8. Order And Callback State

An order tracks payment and fulfillment separately.

Payment state is one of:

- `unpaid`
- `paid`
- `refund_pending`
- `refunded`
- `closed`

Fulfillment state is one of:

- `pending`
- `fulfilled`
- `failed`

A payment attempt state is one of `created`, `presented`, `expired`, `failed`, or `paid`. One order has at most one attempt whose state is `created` or `presented`. A new user retry creates a new order.

The allowed payment-state transitions are:

- `unpaid -> paid` after a verified callback or positive provider query.
- `unpaid -> closed` after expiration and a provider query that confirms no payment.
- `closed -> paid` only after later verified provider evidence. This transition records a late-payment alert.
- `paid -> refund_pending` after a valid refund reservation.
- `refund_pending -> refunded` after verified provider success.
- `refund_pending -> paid` after a definite provider rejection and a completed local compensation.

No other payment-state transition is valid.

The allowed fulfillment-state transitions are:

- `pending -> fulfilled` after one successful reward transaction.
- `pending -> failed` after verified payment and a failed reward transaction.
- `failed -> fulfilled` after idempotent reprocessing.

A fulfilled order cannot return to pending or failed.

Order creation inserts an unpaid order and one payment attempt. It stores the product snapshot, reward snapshot, settlement amount, settlement currency, exchange rate, Channel ID, and adapter kind.

Order creation requires an `Idempotency-Key` containing one UUID v4. The database makes `(user_id, idempotency_key)` unique and stores a digest of the canonical request. Repeating the same key and request returns the original order. Reusing the key with different input returns HTTP 409.

Order creation allows at most ten requests per minute per user and source IP. One user can have at most 20 unpaid orders. Exceeding either limit returns HTTP 429 with `Retry-After` and does not insert an order.

Order status reads require ownership or Admin. Polling allows at most 60 reads per minute per user and order, and 300 reads per minute per source IP. The client keeps one polling loop per visible order and honors `Retry-After`.

An order and its attempt expire 30 minutes after creation. Expiration does not discard a callback. A verified late payment follows the same idempotent fulfillment path and creates an alert.

Each Channel exposes a unique callback URL. The callback endpoint persists an event record before fulfillment.

Callback processing performs these checks:

1. Parse the bounded request body.
2. Verify the adapter signature.
3. Match the merchant identity.
4. Match the order number.
5. Match the exact amount and currency.
6. Enforce provider transaction uniqueness.
7. Change payment state to paid.
8. Apply the balance credit or plan entitlement.
9. Change fulfillment state to fulfilled.

Steps 6 and 7 run in one payment transaction with the verified event. Steps 8 and 9 run in a second fulfillment transaction. The fulfillment transaction uses an order-derived ledger or entitlement idempotency key.

If the process stops between the two transactions, the order remains paid with pending fulfillment and the reconciler resumes it. If reward application fails, its transaction rolls back and a separate conditional update marks fulfillment failed. A duplicate callback does not create another ledger credit or entitlement.

Order mutation locks the order row on PostgreSQL and uses a database write transaction on SQLite. Every transition includes the expected prior state in its update predicate.

A provider transaction ID is unique across all orders for one adapter account. If a different transaction pays an order that is already paid, the service records `duplicate_payment`, does not fulfill again, and requires reconciliation.

If payment verification succeeds but fulfillment fails, the event remains retryable and the order shows paid with failed fulfillment. An Admin can reprocess the verified event. Automatic fulfillment runs only while payment state is `paid`; it stops while a refund is pending.

A database write failure returns a non-success callback response. The provider can retry according to its protocol.

## 9. Refund Accounting

Admin order actions are:

- Query provider status.
- Reprocess a verified callback.
- Close an unpaid order.
- Request a refund.

The normal UI does not contain a manual Complete action.

Refund requests use the original provider transaction and amount. Provider acceptance changes payment state to `refund_pending`. A verified refund result changes it to `refunded`.

For eligible orders, the first release supports only full refunds. It does not implement partial refunds.

A new `store_balance_reservations` table records refund reserves. Each row contains order ID, user ID, nano USD amount, state, reserve ledger key, release ledger key, and timestamps. Order ID is unique.

Fulfillment and refund start both lock the order first. Balance operations then lock the user row. This lock order is fixed. Refund start requires payment state `paid`; fulfillment requires payment state `paid` and no reservation. The first committed transition makes the competing transition fail its predicate and retry from fresh state.

A paid but unfulfilled order needs no reward reversal.

A fulfilled balance order can start a refund only when the user balance is at least the original credited nano USD amount. One transaction locks the user balance, inserts the reservation, writes one negative ledger entry, updates the balance, and changes payment state to `refund_pending`. The unique order ID and ledger idempotency key prevent a second reserve.

A definite provider rejection runs one compensation transaction. It locks the reservation and user balance, writes one positive release ledger entry, updates the balance, marks the reservation released, and returns payment state to `paid`.

A verified provider refund marks the reservation consumed and payment state `refunded`. It does not write another balance delta.

The first release does not refund a fulfilled plan order. It permits a plan-order refund only when payment is verified and fulfillment is still pending or failed. This limit avoids a race with in-flight and delayed API usage settlement.

A provider timeout or unknown result leaves the order in `refund_pending`. Admin must query provider status before retrying. The service does not create a second refund request for the same order.

The refund request uses a stable provider idempotency key. A reconciler, not an Admin repeat click, resolves an ambiguous provider result.

The reconciler never retries fulfillment for `refund_pending` or `refunded`. It never starts a refund while a fulfillment transaction holds the order lock.

Every Admin order action records the Admin user ID, order ID, action, result, and timestamp.

## 10. Redemption Codes

A generated code contains 16 random base32 characters grouped as `XXXX-XXXX-XXXX-XXXX`.

The database stores:

- A SHA-256 digest for redemption lookup.
- The final four-character hint.
- An encrypted full code for Admin reveal.
- Reward, expiration, state, creator, and redemption data.

The generation dialog keeps the returned codes visible until the Admin closes it. It supports individual copy, copy all, and CSV export.

The Redemption Codes page masks codes by default. An Admin can reveal, copy, batch copy, or export full unused codes. Each reveal or export writes an audit record.

Reveal and export require a reauthentication grant created from the Admin current password. The grant is bound to the Admin user and dashboard session, is scoped to redemption-code access, and expires after five minutes. The server stores only a hash of the grant token.

The current account model requires a local password hash. If a future SSO or passwordless Admin has no locally verifiable password, sensitive Store actions fail closed until an explicit step-up provider supplies a signed assertion with an authentication time no older than five minutes. A normal SSO login assertion is not sufficient.

Logout, password change, session rotation, session expiration, Admin-role removal, or account disablement invalidates every related reauthentication grant.

Reveal returns at most 20 selected codes. Export returns at most 100 selected codes. Responses set `Cache-Control: no-store`, `Pragma: no-cache`, `Referrer-Policy: no-referrer`, and `X-Content-Type-Options: nosniff`. CSV uses `Content-Disposition: attachment`.

Audit records contain Admin ID, action, selected code IDs, count, IP, user agent, and time. They do not contain full codes. Application logs do not contain full codes or reauthentication tokens.

Used, expired, or revoked codes remain masked and cannot be returned by a reveal endpoint.

Existing codes created before this design have no encrypted full value. They remain redeemable by users, but Admin can see only the final four characters. Admin can revoke and replace them.

User redemption converts ASCII letters to uppercase and removes ASCII hyphens. The normalized value must contain exactly 16 allowed base32 characters. Redemption locks or serializes the code row and changes an unused code exactly once in the same reward transaction.

The redeem endpoint allows at most ten attempts per minute per user and per source IP. Five failed attempts in 15 minutes create a 30-minute account-and-IP cooldown. A rate-limited request returns HTTP 429 and does not disclose whether a code exists.

## 11. Frontend Data Flow

Store and Store Management reads use SWR and render skeletons during initial loading.

User mutations use optimistic state where rollback is possible. Payment creation does not optimistically credit balance or activate a plan.

While an order is unpaid, the payment screen polls order status every two seconds. It stops after success, closure, component unmount, or the configured checkout expiration.

After fulfillment, the frontend revalidates orders, balance, current user, and entitlement. The user does not need to close and reopen a page.

Alipay uses an official redirect. WeChat desktop uses a QR modal. WeChat mobile uses an H5 redirect. Stripe uses Stripe Checkout. A configurable Channel uses the action returned by its adapter.

## 12. Admin Channel Status

The Payment Channels page shows:

- Adapter kind.
- Enabled state.
- Configuration state.
- Callback URL with a copy action.
- Last callback time.
- Last callback result.
- Last configuration error.

A Channel cannot be enabled until required configuration passes local validation.

Saving credentials does not prove that the provider account is active. Alipay sandbox, Stripe test mode, and a controlled WeChat live test are separate release checks.

## 13. Reconciliation And Alerts

Only the primary node runs payment reconciliation. A database lease with a fencing token prevents two reconcilers from processing the same due item.

The reconciler runs once per minute. It selects bounded batches with deterministic order and processes:

- Presented attempts whose provider expiration has passed.
- Paid orders whose fulfillment state is `failed`.
- Refunds in `refund_pending`.
- Callback events marked retryable.

Before closing an expired unpaid order, the reconciler queries the provider. A positive payment result changes the order to paid and starts fulfillment. A confirmed unpaid result closes the order.

Retryable fulfillment uses exponential delays of 30 seconds, two minutes, ten minutes, and one hour. Later failures remain visible for Admin processing.

A refund-pending order is queried after one minute, five minutes, 15 minutes, and then hourly. It raises an alert after 15 minutes and remains pending until the provider returns a definite result.

The system records metrics for callback rejection, late payment, duplicate payment, failed fulfillment, stale unpaid attempts, refund timeout, reconciliation failures, and lease loss. Admin shows the affected order count and the latest error.

Every automated state change writes the same audit format as an Admin action and identifies the reconciler actor.

## 14. Migration

A read-only preflight runs before the schema migration. It reports row counts by order status, timestamp consistency, orphan references, duplicate order numbers, duplicate provider transaction fields if present, and every table or column that collides with a new payment object.

The current schema permits only `pending`, `completed`, and `cancelled`. The preflight aborts when it finds any other value, a status/timestamp mismatch, an unknown legacy callback table, a partially populated payment field, or an orphan reference. It never maps an unknown value to a known state.

The schema migration runs in one database transaction after preflight passes.

Existing completed orders migrate to payment state `paid` and fulfillment state `fulfilled`. Existing pending and cancelled orders migrate to payment state `closed` and fulfillment state `pending`. A new callback cannot fulfill a legacy order.

Existing Alipay and WeChat Channel rows become disabled, unconfigured official Channels. Existing custom Channel rows become disabled, unconfigured HTTP Channels. The migration creates one disabled, unconfigured Stripe Channel when none exists.

The current application has no provider checkout, transaction, callback, query, or refund fields. Its Channel `config_secret` is not an official credential contract. Migration encrypts each non-empty legacy value as a retired legacy credential version and does not use it for a new official adapter.

If preflight finds provider transaction data, callback data, or a credential format outside the current schema contract, migration aborts. A dedicated importer must preserve that adapter account and bind every historical order to an immutable credential version before migration can continue.

Future Channel edits never remove credentials needed by historical attempts. Late callbacks, queries, disputes, and refunds resolve credentials through the attempt credential-version ID, not the Channel current configuration.

The migration adds encrypted-code fields without changing existing digests. Existing redemption codes cannot be recovered because the prior digest is irreversible.

The migration writes a versioned manifest containing the preflight counts and migrated counts. A count mismatch aborts and rolls back the migration.

Migration tests cover SQLite and PostgreSQL SQL behavior. Local development runs SQLite only. PostgreSQL tests run in an isolated environment and never start PostgreSQL on the user's computer.

## 15. Security Limits

Callback endpoints are public and require adapter verification.

A callback request body is limited to 128 KiB and a five-second read timeout. Unsupported content types fail before parsing. Signature comparison uses a constant-time operation where the protocol permits it.

A callback event stores the body SHA-256 digest, allow-listed parsed fields, verification result, and encrypted raw body. Raw callback bodies expire after 30 days. Audit fields and order state remain after body deletion.

Logs and API responses redact signatures, private keys, API secrets, full redemption codes, reauthentication tokens, and sensitive callback fields.

Provider transaction IDs have a database unique constraint. Callback event IDs use adapter-specific idempotency keys.

Generic callback replay protection validates timestamp and nonce when the configured protocol supplies them. A nonce cannot be accepted twice for the same Channel.

Callback admission allows at most 600 requests per minute per Channel and source IP. Rate limiting does not replace signature verification.

Payment and reveal endpoints require the existing dashboard authorization model. Reveal, export, refund, reprocess, exchange-rate override, key rotation, and credential updates require Admin plus a matching five-minute reauthentication scope.

Every cookie-authenticated Store mutation requires `Content-Type: application/json` or the documented multipart icon type and an `Origin` equal to the configured public origin. A missing or mismatched Origin fails. State-changing actions never use GET. Provider callbacks are exempt from session CSRF checks and require provider verification instead.

The dashboard session cookie remains HttpOnly, Secure, SameSite Strict, and Path `/`. Session invalidation is checked before accepting a reauthentication grant or a sensitive mutation.

## 16. Verification

Backend tests cover:

- Official signature and callback test vectors.
- Both exchange directions, nano USD conversion, rate age, anomaly quarantine, startup failure, one rounding point, and provider minimum amounts.
- Amount, currency, merchant, and order mismatch.
- Twenty concurrent copies of one callback with one fulfillment.
- Callback versus expiration, close, refund, and a second provider transaction.
- Retry after verified payment and failed fulfillment.
- Balance refund reservation, concurrent spending, one compensation, and ambiguous provider results.
- Every ordering of fulfillment, refund start, callback retry, and reconciliation, with one final reward state.
- Plan refund rejection after fulfillment.
- Order idempotency-key replay, mismatched input, open-order cap, creation rate limit, and polling rate limit.
- Product edit, disable, emergency close, immutable quote, plan replacement, plan expiration, and every quota window.
- HTTP milestone JSON, form, redirect, QR, and form actions.
- HTTP milestone DNS rebinding, private addresses, redirect rejection, timeout, response limits, template limits, and query-before-retry behavior.
- Encryption, wrong-key failure, and prior-key decryption.
- Key rotation, bounded re-encryption, referenced credential retention, and old-key removal refusal.
- New-code reveal, reauthentication expiry, response headers, rate limits, and existing-code non-recoverability.
- CSRF Origin rejection, session invalidation, role removal, and passwordless Admin fail-closed behavior.
- Reconciler lease fencing, due-order selection, retry schedule, and alert thresholds.
- SQLite migration and isolated PostgreSQL migration.
- Migration preflight rejection for every unknown or inconsistent legacy state.

Frontend tests cover:

- Sliding Store and Admin tabs.
- Stable order-summary dimensions.
- Independent payment-method row.
- CNY and USD presentation.
- WeChat QR, official redirects, and Stripe Checkout actions.
- SWR polling and automatic post-payment refresh.
- Callback status and failure presentation.
- Redemption generation, reveal, copy, batch copy, and export.

Browser verification covers desktop and mobile Store layouts. It also covers reduced motion, empty states, long product lists, many plans, long Channel names, callback failures, and unavailable payment configuration.

Release verification uses:

- Alipay sandbox.
- Stripe test mode.
- One CNY 0.01 WeChat Pay transaction and refund after credentials are configured.
- One controlled transaction for each verified HTTP template before that separate milestone can enable a production Channel.

## 17. Production Gate

Design approval, implementation approval, migration-drill approval, and production-deployment approval are separate decisions. Text inside this document does not grant any approval.

The verified pre-change production backup recorded on 2026-08-27 is `/opt/monoize/backups/deploy-20260827T053602Z-a3dfa95`. It is a baseline for the old application. It is not sufficient for a later payment release.

Immediately before deployment, the operator stops Store writes and creates a new database backup. The operator restores that backup into an isolated path, runs database integrity checks, verifies row counts, and records the measured restore time.

The operator creates and backs up the payment key ring separately. A restore drill must decrypt one test credential and one test redemption code with the restored key ring.

The release uses a cold switch. Old and new Store versions do not run concurrently. The new version starts with every payment Channel disabled and with callback URLs unregistered or disabled at the providers.

Code deployment can occur with every payment Channel disabled only after a separate deployment approval.

A Channel can accept production purchases only after:

1. The deployment key ring is installed and backed up.
2. Required merchant credentials are configured.
3. The exact public callback URL is registered with the provider.
4. Signature and callback verification passes.
5. One controlled payment and refund passes.

Production deployment does not invent or embed merchant credentials. The operator must supply them through Admin or deployment configuration.

Before the first real payment, rollback can restore the old image and the matching pre-release database backup.

After any provider accepts a real payment or refund, restoring the old database is forbidden. The operator must disable new checkout, keep callback ingestion available when possible, reconcile provider transactions against local events, and forward-fix or restore into the new schema. Every transaction created after the release backup must remain represented.

A rollback drill tests both boundaries: a pre-payment image-and-database restore, and a post-payment maintenance procedure that preserves new provider transactions.
