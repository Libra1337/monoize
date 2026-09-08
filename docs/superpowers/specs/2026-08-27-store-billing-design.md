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

Plan quota admission is an independent acceptance workstream inside Milestone 1. Payment implementation can compile before its load gate passes, but plan products cannot be enabled in production until quota load and fault drills pass.

The logged-in Model Marketplace redesign is a separate subsystem. It is not part of this design.

The payment design uses `QuantumNous/new-api` commit `8f6961c675932f406260ff0c218bc2aa0603e9b2` as an external architecture reference. That repository uses AGPL-3.0 while Monoize uses MIT. Monoize does not copy its source, comments, tests, UI code, or internal identifiers. Direct code reuse requires separate license review or a commercial license.

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

The client uses the exact HTTPS host, the system trust store, no redirects, a five-second timeout, and a 64 KiB response limit. Each attempt stores source URL, parser version, HTTP status, response-body SHA-256 when a body exists, and an error category. It never stores the body in application logs.

Production stores an exchange-source governance record with provider name, endpoint, terms and attribution URLs, allowed commercial use, caching permission, request limit, required attribution, reviewer identity, review evidence digest, reviewed time, and next review time no more than 180 days later.

Conversion checkout cannot enable without a current accepted governance record. A detected terms change, revoked permission, exceeded rate limit, or expired review pauses refresh-dependent checkout and creates a critical alert. Changing the source endpoint requires a design revision, parser fixtures, anomaly baseline, operations approval, and legal or authorized terms review.

The response must be valid JSON with `result = success`, `base_code = USD`, a positive integer source timestamp, and one finite decimal `rates.CNY`. The rate must be between 1 and 20 CNY per USD and contain at most 18 fractional digits.

The source timestamp cannot be more than five minutes in the future, more than 48 hours before refresh time, or older than the active source timestamp. Every comparison uses UTC.

The service parses the decimal rate into positive integers `N` and `D`, where `D = 10^scale`, `scale` is 0 through 18, and `R = N / D`. It reduces the pair by their greatest common divisor. It does not use decimal division or persist a rounded reciprocal.

USD cents convert to CNY fen as `round(cents * N / D)`. CNY fen convert to USD cents as `round(fen * D / N)`. USD cents convert to nano USD exactly as `cents * 10,000,000`. CNY fen convert to nano USD as `round(fen * D * 10,000,000 / N)`.

All products use checked integer multiplication, division, quotient, and remainder. Round-half-away-from-zero is applied once to the final nonnegative rational result. An intermediate overflow returns `amount_overflow` and writes no order.

The decimal exchange rate contains at most 18 fractional digits. A checkout that needs conversion fails when the last successful refresh is older than 60 minutes or the source timestamp is older than 48 hours. A failed refresh retains the prior snapshot but does not extend its validity.

A candidate that differs from the active rate by more than the configured threshold is quarantined and does not replace it. The threshold defaults to five percent and is configurable from 0.1 through 25 percent. Admin can approve or reject the candidate. Admin can pause conversion-based checkout. Same-currency checkout can continue only when its reward and entitlement calculations do not require the rate.

The service retains accepted snapshots for 30 days. Admin can restore the immediately prior accepted snapshot only while its original age limits still pass. Restore does not change its source or refresh timestamp. If no accepted snapshot is valid, conversion-based checkout remains paused.

At startup, a network failure uses the stored active snapshot only while both age limits pass. Without a valid snapshot, dependent checkout returns HTTP 503. The process remains available for non-Store traffic.

The primary runs one non-overlapping refresh task immediately after startup and then 15 minutes after each completed attempt. It persists consecutive failure count, last attempt time, last error category, and paused state. Replicas never refresh.

Three consecutive failures create a warning. Six failures or an active snapshot within 15 minutes of expiry create a critical alert. Admin shows the alert, active snapshot, quarantined candidate, failure count, and next attempt time.

The Store operator owns rate alerts. Production readiness requires routing the warning and critical metrics to the deployment alert system. Recovery requires inspecting the recorded digest and error category, checking source reachability, and explicitly resuming or approving a quarantined candidate with reauthentication. Process restart alone does not clear pause or failure state.

Production configuration names one primary Rate Operations Admin and one distinct backup Admin. A warning requires acknowledgement within 30 minutes. A critical alert requires acknowledgement within 15 minutes and a recorded containment decision within 30 minutes.

Containment keeps conversion checkout paused. The Rate Operations Admin can leave only same-currency products enabled when their reward calculation does not require FX. The Admin cannot extend snapshot age or enter an arbitrary replacement rate.

After four hours without a valid refresh, the Payment Operations Owner records one decision: continue the partial Store, disable every checkout, or schedule maintenance. After 24 hours, the incident escalates to the business owner and remains open until source recovery. These are operational response objectives, not permission to accept stale rates.

Recovery from suspected source compromise requires a clean network and trust-store check plus two valid source observations at least 15 minutes apart. Both observations must pass timestamp, range, and configured-change checks. One Rate Operations Admin proposes resume; the distinct backup Admin approves it with reauthentication. Recovery from an ordinary transient outage requires one valid observation and one reauthenticated Admin.

The incident record contains alert times, response times, source digests, decisions, approvers, affected products, and recovery evidence. Missing the response objective creates a production-gate incident review.

When no first valid snapshot exists, conversion-dependent products remain visible but checkout is disabled with an exchange-rate-unavailable state. The operator can retry refresh, correct network access, or pause those products. The operator cannot type an unverified rate into the database through Admin.

Conversion happens once during order quotation. The service converts from the product currency to the settlement currency, then rounds half away from zero to the settlement minor unit. The immutable quote stores the unrounded decimal inputs, rounded settlement amount, rate, and timestamps.

Alipay and WeChat Pay settle in CNY. When the Store currency control shows USD, the order summary also shows the exact CNY amount that the gateway will charge.

Stripe Checkout can charge CNY or USD when the Stripe account supports the selected currency.

Alipay and WeChat reject a CNY charge below 1 fen. Stripe validates credentials by retrieving the account country and supported presentment currencies. It resolves minimum amounts from a versioned capability table bound to the configured Stripe API version and account country. A missing capability disables that currency instead of guessing a minimum. Adapter limits apply after conversion.

The default custom-recharge range is 1,000 through 100,000,000 minor units for each currency. Admin can configure a separate positive minimum and maximum for CNY and USD. A product price must be positive, and checkout also applies the selected adapter minimum.

Each configurable HTTP Channel declares an allow-list containing `CNY`, `USD`, or both.

A configurable HTTP Channel declares positive minimum and maximum minor amounts for each allowed currency. Order creation applies the stricter of the Store product limit and the adapter limit.

The customer charge equals the quoted product price. The first release does not add tax, gateway fees, or payment surcharges. The merchant absorbs provider fees. Callback comparison uses the gross customer charge, not the provider net settlement.

Order creation stores an immutable exchange-rate snapshot. A later rate refresh does not change an existing order amount.

### 3.1 Product And Plan Lifecycle

Product edits affect only later orders. Every order stores an immutable product, price, reward, duration, Group, and quota snapshot.

Each product has a monotonic revision. Admin update, disable, and delete require the expected revision. A stale revision returns HTTP 409 and changes no row.

Database triggers reject an update to order quote, settlement amount, settlement currency, exchange-rate rational, reward snapshot, or payment-contract version after insertion. Entitlement generation snapshots and settled quota reservations are also immutable.

Disabling or editing a product does not change an existing unpaid order. That order remains payable until its 30-minute expiration.

Emergency disable first locks the product, sets `enabled = false`, increments revision, and creates one close-request job per unpaid order. It does not mark a presented attempt closed. Reconciliation queries each provider, closes only a confirmed unpaid order, and fulfills any verified payment from the immutable quote.

A referenced product cannot be physically deleted. Admin can disable it. Unreferenced products can be deleted with an audit record.

A plan duration is 3,600 through 31,536,000 seconds. It starts when fulfillment succeeds and ends exactly one duration later. An expired plan grants no access or quota.

Buying or redeeming another plan replaces the active plan immediately. Unused time and quota do not carry forward. The order summary shows this replacement before checkout.

All quota rules in a plan apply concurrently. Five-hour, 12-hour, and custom-hour windows are rolling. Day, week, and month windows use Asia/Shanghai calendar boundaries, with Monday as the first day of a week.

Each entitlement has a monotonic generation and state. Each quota window bucket stores settled and reserved CNY fen. Each request reservation has a unique request ID, entitlement ID, generation, maximum charge, state, and timestamps.

At plan fulfillment, an order-funded entitlement copies the order exchange-rate rational `N/D`. A redemption-funded entitlement copies the valid active rational inside the redemption transaction. That generation uses the same immutable `N/D` for every reservation and settlement until it expires or is replaced. A later market-rate refresh does not change its quota accounting.

The billing engine prices a request in integer nano USD. It converts actual nano USD to CNY fen as `round(nano_usd * N / (D * 10,000,000))`. It converts a maximum nano USD bound to reserved CNY fen with ceiling division of the same rational, so reservation cannot round below the eventual charge.

Before routing, the billing engine computes a finite maximum nano USD charge from known input usage, the selected pricing snapshot, and the request output or meter caps. The reservation stores maximum nano USD, reserved CNY fen, rate numerator, rate denominator, and pricing revision. When no finite cap exists, plan admission returns `plan_request_unbounded` and does not route.

Funding source is fixed before routing. An applicable active plan requires a successful reservation and charges only that plan generation; it does not also deduct ordinary balance. A request with no applicable plan uses the existing ordinary-balance path. The first release does not silently fall back from an exhausted or unbounded plan to ordinary balance.

The Primary admission transaction locks the entitlement and every applicable window bucket in deterministic order. It verifies active state, generation, expiration, and `settled + reserved + maximum <= quota` for every rule. It then increments every reserved counter and inserts one reservation.

SQLite supports only the single-Primary topology and does not use Replica admission tokens. SQLite admission and settlement use short `BEGIN IMMEDIATE` transactions, WAL mode, foreign keys enabled, and a 5-second busy timeout. Network calls and price calculation run outside the write transaction.

Plan products on SQLite require a local-file concurrency drill at `max(100 requests/second, two times measured seven-day peak)` for ten minutes, with five quota windows and 100 concurrent requests against one entitlement. Acceptance requires zero unhandled `SQLITE_BUSY`, exact reservation conservation, p95 lock wait below 50 ms, and p99 below 200 ms.

If the SQLite drill fails, plan-product creation and enablement return `plan_requires_postgres`. Balance products, redemption of balance, and payment Channels remain available. PostgreSQL is not started on the user's computer.

SQLite persists one `store_feature_gates` row for `plan_quota`. Its state is `pending`, `passed`, or `failed`. Migration initializes it to `pending`. A passed row stores schema version, application version, SQLite version, WAL and busy-timeout settings, filesystem identifier, drill result digest, measured time, and Admin importer.

The backend derives one effective `plan_quota_enabled` value from that matching row at startup and after configuration change. A mismatched version or setting returns the gate to `pending`.

No Dashboard endpoint can set the gate to `passed`. The project-owned offline CLI runs the drill against a temporary SQLite database on the same filesystem with the target settings. After success, it acquires the target database write lock and writes the result manifest and digest. Failure writes `failed`. Interrupted or malformed output leaves `pending`.

The gate fingerprint uses a quota-engine compatibility ID, schema version, SQLite library version, WAL mode, busy timeout, page size, synchronous mode, and filesystem identifier. An application release that does not change this fingerprint keeps the passed gate; the ordinary application version alone does not invalidate it.

Before a deployment that changes the fingerprint, the release workflow automatically runs the new binary's quota drill on the target host and stores a `next` passed manifest. Cutover promotes it only when its fingerprint matches the starting process.

If startup finds no matching manifest and active plan entitlements exist, startup stops before the listener with `sqlite_plan_gate_pending`. The operator must run the drill or roll back the binary. It does not start and then make purchased plans unexpectedly unavailable.

If no active plan entitlement exists, startup can continue with plan features disabled and starts one low-priority drill against a temporary same-filesystem database. Success sets `passed` and emits a recovery event. Failure sets `failed`, creates a critical alert, and retries only after an Admin acknowledges the failure or the fingerprint changes.

A pending gate creates a warning immediately. A failed gate creates a critical alert. Admin shows old and new fingerprints, invalidation reason, drill progress, lock latency, failure output digest, and required operator action.

The product write API rejects creation or enablement of a plan while the effective gate is false. Catalog queries omit disabled-gate plan products. Plan redemption-code generation, plan order creation, plan fulfillment, and plan-funded API admission all reject with `plan_requires_postgres` while the gate is false.

SQLite triggers reject an enabled plan-product row, a new plan entitlement generation, and a plan redemption reward while the persisted gate is not `passed`. The frontend reads the gate, disables plan controls, and shows a database requirement. Frontend state cannot override the backend or trigger.

Migration preflight aborts on SQLite when an active plan entitlement exists and no matching passed gate is available. It does not silently suspend a purchased entitlement.

The Primary returns a signed short-lived admission token bound to the reservation and request ID. A Replica must obtain this token from the Primary before routing a plan-funded request. Primary unavailability fails plan admission closed. A Replica cannot create a local optimistic reservation.

The token is compact JWS with `alg = EdDSA`, an Ed25519 key, `typ = lynshen-plan-admission`, and a required `kid`. Claims contain issuer, Replica ID audience, token ID, reservation ID, request ID, entitlement ID, generation, maximum nano USD, reserved CNY fen, pricing revision, issued time, not-before time, and expiration.

Token TTL is 30 seconds. Verification allows at most five seconds of clock skew. The Replica rejects an unknown key ID, wrong audience, duplicate token ID, mismatched request, future token, expired token, or signature failure before upstream routing.

Before routing, the bound Replica atomically creates and fsyncs a durable token-claim marker in its metering spool. An existing marker rejects replay. Cross-Replica replay fails the audience check. The marker remains until the Primary acknowledges one settlement or release, and for at least five minutes after token expiry.

Admission signing uses a key ring separate from credential encryption. Rotation publishes the next public key to Replicas through configuration epoch, waits for every non-stale Replica heartbeat to acknowledge the key ID, then activates the private key. The prior public key remains valid for at least five minutes and until no unexpired token references it.

Internal admission requests use the existing Replica authentication channel and include the configured Replica ID. The Primary refuses to issue a token when the authenticated node and requested audience differ.

Settlement locks the same generation and bucket rows. It verifies the stored rate and pricing revision, changes the reservation to settled, subtracts reserved CNY fen, and adds the converted actual CNY fen. A request that fails before billable upstream work releases the reservation once.

A Replica writes settlement or release with the reservation ID to its durable metering spool before reporting terminal billing success. The Primary applies that event idempotently. Shipment delay leaves the amount reserved and raises the existing spool-health alert; it never recreates available quota locally.

Plan replacement creates a new generation. Existing in-flight reservations remain bound to the prior generation and settle against its snapshot. New requests use only the new generation. Expiration or replacement does not erase an unsettled reservation.

Plan fulfillment locks the user entitlement pointer. `(source_kind, source_id)` is unique, so one order or redemption cannot create two generations. Replacement inserts one immutable generation and changes the pointer with an expected-generation predicate. A concurrent duplicate, refund transition, or newer replacement makes the stale update fail without overwriting current entitlement.

If provider behavior produces a charge above the reserved maximum, settlement records a quota-bound violation, applies the full charge to the old generation, blocks later plan admission, and raises a critical alert. It does not charge the new plan generation.

## 4. Payment Core

The payment core owns order state, payment attempts, callback events, fulfillment, refunds, and audit records.

Each adapter implements these operations:

1. `create_checkout`
2. `query_payment`
3. `verify_callback`
4. `refund_payment`

Each adapter also declares capabilities for payment query, refund query, dispute Webhook, dispute query, and settlement report. A Channel cannot call or display an unsupported operation. Production configuration verifies required capabilities against the merchant account.

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

The baseline Alipay contract supports trade notification, trade query, refund, refund query, and bill download. It does not claim an automated dispute or chargeback event unless the configured merchant product exposes a documented signed API and its capability test passes. Otherwise a bill mismatch opens a manual reconciliation case.

### 5.2 WeChat Pay

The WeChat adapter supports Native QR payment and H5 payment. It uses WeChat Pay API v3.

Admin configuration contains:

- Merchant ID.
- App ID.
- API v3 key.
- Merchant certificate serial.
- Merchant private key.

The adapter verifies platform certificate signatures and decrypts callback resources. It checks the merchant ID, App ID, order number, amount, currency, and success status.

The baseline WeChat Pay contract supports payment notification, order query, refund notification, refund query, and bill download. Complaint or dispute automation is conditional on merchant API permission and a passing capability test. Without it, the adapter records statement differences as manual reconciliation cases.

### 5.3 Stripe

Stripe uses hosted Stripe Checkout. LynShen does not collect card numbers.

Admin configuration contains:

- Secret key.
- Publishable key.
- Webhook signing secret.
- Production or test environment.

The adapter creates a Checkout Session with the Store order number as the idempotency key and metadata reference. The webhook verifier checks the Stripe signature, Checkout Session, PaymentIntent, amount, currency, and payment status.

The Stripe contract supports signed dispute Webhooks, Disputes API query, refunds, refund query, balance transactions, and settlement reconciliation. Provider event ID is the dispute-event idempotency key.

The Store can show card, Apple Pay, Google Pay, and other methods that the Stripe account enables. LynShen does not claim that a method is available until Stripe returns it.

### 5.4 Clean-Room Reference Rules

The implementation independently reproduces only these observable architecture patterns from the external reference:

- Compute effective Channel availability from explicit enablement, current compliance acknowledgement, complete credentials, callback configuration, product configuration, and adapter capability validation.
- Return only effectively available payment methods to the Store. Each method includes adapter kind, public name, icon, supported currencies, minimum amount, and checkout action kinds.
- Persist adapter kind, credential version, merchant account identity, and expected payment method on every attempt. A callback from another adapter or merchant account cannot mutate the order.
- Treat Stripe Checkout completion, asynchronous payment success, asynchronous payment failure, and Session expiration as distinct provider events.
- Validate custom success and cancel URLs against an exact configured HTTPS origin allow-list. A rejected URL does not create an order or provider Session.
- Test missing credentials, incomplete Webhook configuration, adapter mismatch, duplicate callback, asynchronous event routing, and amount overflow.

The local order and payment attempt commit before an outbound checkout mutation. If provider checkout creation fails or is ambiguous, the attempt remains queryable and follows the query-before-retry rules. Monoize does not create a provider checkout first and then attempt to persist its order.

Stripe Webhook parsing enforces the configured Stripe API version. It does not ignore an API-version mismatch. Logs contain event ID, event type, order number, verification result, and body digest; they never contain the signature header or raw body.

All amounts remain integer minor units or integer nano USD. The design does not adopt floating-point payment fields, floating-point ratios, or conversion through `float32` or `float64`.

The design does not adopt a manual paid or manual completion endpoint. Admin recovery starts from verified provider evidence, a persisted callback, or a provider query.

An implementation review records the external reference commit, lists the independently adopted behavior, and confirms that no AGPL source or test text appears in the change.

The release stores a license review record containing the Monoize commit, external repository and commit, both license identifiers, files reviewed, SBOM and dependency-license manifest digest, similarity-scan result, reviewer identity, decision, restrictions, and review time. The reviewer must be authorized to approve license use for the deployment.

The release scan normalizes whitespace and comments, tokenizes changed non-generated source and test files, and reports every contiguous exact match of 80 or more tokens against the referenced repository tree. Public protocol constants and generated files are listed as exclusions with reasons.

A source or dependency similarity hit blocks release until the reviewer classifies and records it. A clean-room engineering checklist does not replace legal advice. Direct AGPL code reuse remains prohibited unless a separately recorded compatible-license or commercial-license decision permits it.

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

The order stores payment-state and fulfillment-state timestamps, including `paid_at`, `fulfillment_started_at`, `fulfilled_at`, and `fulfillment_failed_at`. A transition writes its timestamp in the same transaction as the state.

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
- `closed -> paid` only for payment-contract version 2 after later verified provider evidence. This transition records a late-payment alert.
- `paid -> refund_pending` after a valid refund reservation.
- `refund_pending -> refunded` after verified provider success.
- `refund_pending -> paid` after a definite provider rejection and a completed local compensation.

No other payment-state transition is valid.

The allowed fulfillment-state transitions are:

- `pending -> fulfilled` after one successful reward transaction.
- `pending -> failed` after verified payment and a failed reward transaction.
- `failed -> fulfilled` after idempotent reprocessing.

A fulfilled order cannot return to pending or failed.

### 8.1 Provider Event Ordering

Event application locks order, recovery, and user rows in that order. It persists every verified event before projection. Duplicate event identity is a no-op.

`store_provider_events` has a unique `(credential_version_id, provider_event_id)` key, immutable body digest, immutable verified fields, projection state, and state revision. `store_order_event_applications` uses `provider_event_row_id` as its primary key. One verified event can project to one order at most once.

`store_orders` has a monotonic state revision. SQLite and PostgreSQL migration SQL create equivalent transition triggers. A trigger rejects every payment, fulfillment, dispute, or hold transition that is not listed in this design. Application updates also include expected state and revision; zero updated rows cause a fresh read instead of a blind retry.

Each verified event has projection state `pending`, `applied`, `superseded`, or `manual_review`. An event that lacks prerequisite local payment evidence remains `pending`; it is never discarded as invalid solely because it arrived first.

An adapter uses a provider object version when the provider guarantees monotonic versions. A conflicting or backward event without such a version triggers provider query. It does not directly change state.

| Verified event | Current state | Required result |
| --- | --- | --- |
| Payment success | `unpaid` or version 2 `closed` | Set `paid`; start fulfillment only when no hold, refund success, or lost dispute exists. |
| Payment failure or close | `unpaid` | Query provider, then close only when query confirms unpaid. Never downgrade `paid` or `refunded`. |
| Refund success | `paid` or `refund_pending` | Set `refunded`, consume shared recovery once, and block fulfillment. |
| Refund success | Dispute `open` or `lost` | Set payment state `refunded` and consume shared recovery once. Keep dispute state and payment hold unchanged. |
| Refund success before payment event | `unpaid` | Query the original payment, record verified payment evidence, then set `refunded` without fulfillment. |
| Refund failure | `refund_pending` | Return to `paid` only after refund query confirms failure and no refund-success event exists. |
| Dispute opened | `paid` or `refunded` | Set dispute `open`, add the shared recovery claim, set payment hold, and block pending fulfillment. |
| Dispute opened | `refund_pending` | Keep `refund_pending`, set dispute `open`, reuse the shared recovery reserve, set payment hold, block fulfillment, and query both payment and refund. |
| Dispute opened before payment evidence | `unpaid` or version 2 `closed` | Keep the event `pending`, set no user reward or recovery, and query payment by provider object and merchant order. If paid is confirmed, record payment evidence and then apply dispute-open without fulfillment. If unpaid is confirmed, keep a reconciliation case until the provider explains the dispute object. |
| Dispute won | `open` | Set `won`, resolve only that claim, and resume eligible paid/pending fulfillment when no other hold reason exists. |
| Dispute won | Payment `refund_pending` | Keep payment state `refund_pending`, set dispute `won`, resolve only the dispute claim, retain the refund claim and shared reserve, and continue refund query. |
| Dispute lost or chargeback | `open`, `won`, `paid`, or `refunded` | Set `lost`, recover at most the original reward once, keep payment hold, and block fulfillment. |
| Dispute lost or chargeback | `refund_pending` | Keep refund state pending, set dispute `lost`, consume at most the shared original reward once, keep payment hold, block fulfillment, and query the refund. Later refund success changes payment state to `refunded` without another recovery; refund failure returns payment state to `paid` while dispute remains `lost`. |
| Dispute lost or chargeback before payment evidence | `unpaid` or version 2 `closed` | Keep the event `pending`, query payment, and open a critical reconciliation case. Confirmed payment records payment evidence, applies dispute loss, and never fulfills. Unconfirmed identity performs no user balance mutation. |
| Dispute reopened | `won` | Return to `open` only with a new dispute ID or higher provider version. |
| Settlement difference | Any | Open a reconciliation case and query provider; do not mutate user reward directly. |

Refund success is terminal for refund state. Dispute lost is terminal for one dispute version. A later contradictory event requires query and a newer provider version before transition. Event arrival time alone never overrides a terminal state.

Refund state and dispute state are orthogonal. Refund success never clears a dispute. Dispute success or loss never fabricates a refund result. The shared recovery invariant prevents either event order from deducting the original reward twice.

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
8. Apply the balance credit or plan entitlement only when the event-order matrix permits fulfillment.
9. Change fulfillment state to fulfilled after reward commit.

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

A new `store_order_reward_recoveries` table contains one row per fulfilled balance order. It stores original credited nano USD, reserved nano USD, recovered nano USD, one debit ledger key, one release ledger key, state, and timestamps. Database checks require nonnegative values and `reserved + recovered <= original`.

A `store_order_recovery_claims` table records refund, dispute, and chargeback reasons separately. Multiple claims can reference one recovery row, but they share its single economic reserve. Claim insertion never creates a second debit for that order.

`store_order_recovery_claims` has unique `(credential_version_id, provider_claim_id, kind)` and unique `(provider_event_row_id, kind)` keys. A claim references one immutable order recovery row. It cannot change order ID after insertion.

Every recovery debit, release, and final loss uses a unique ledger idempotency key derived from recovery row and mutation kind. The ledger table rejects duplicate keys.

PostgreSQL constraint triggers and separate SQLite `BEFORE INSERT` and `BEFORE UPDATE` triggers query the recovery and ledger rows inside the same write transaction. They reject a mutation when cumulative reserved plus recovered value exceeds original credited value, when a second debit key exists, or when an event application already exists. Application code cannot disable these triggers.

The migration verification suite inspects installed trigger SQL and unique indexes on both backends. Missing or changed recovery constraints stop startup before Store routes mount.

Fulfillment and refund start both lock the order first. Balance operations then lock recovery and user rows. This lock order is fixed. Refund start requires payment state `paid`; fulfillment requires payment state `paid`, no active recovery claim, and zero reserved or recovered amount. The first committed transition makes the competing transition fail its predicate and retry from fresh state.

A paid but unfulfilled order needs no reward reversal.

A fulfilled balance order can start a refund only when the user balance is at least the original credited nano USD amount or the same recovery row is already fully reserved. One transaction locks order, recovery, and user; inserts the refund claim; creates the single negative ledger reserve when needed; and changes payment state to `refund_pending`.

A definite provider rejection resolves the refund claim. It releases the economic reserve exactly once only when no dispute or chargeback claim remains and no recovery was consumed. It then returns payment state to `paid`.

A verified provider refund marks the shared recovery amount consumed and payment state `refunded`. It does not write another balance delta. A later dispute or chargeback cannot recover the same original credit again.

The first release does not refund a fulfilled plan order. It permits a plan-order refund only when payment is verified and fulfillment is still pending or failed. This limit avoids a race with in-flight and delayed API usage settlement.

A provider timeout or unknown result leaves the order in `refund_pending`. Admin must query provider status before retrying. The service does not create a second refund request for the same order.

The refund request uses a stable provider idempotency key. A reconciler, not an Admin repeat click, resolves an ambiguous provider result.

The reconciler never retries fulfillment for `refund_pending` or `refunded`. It never starts a refund while a fulfillment transaction holds the order lock.

Every Admin order action records the Admin user ID, order ID, action, result, and timestamp.

## 10. Redemption Codes

New codes use strict Crockford Base32 alphabet `0123456789ABCDEFGHJKMNPQRSTVWXYZ`. They contain 16 OS-CSPRNG characters grouped as `XXXX-XXXX-XXXX-XXXX`, which provides 80 random bits. Generation never emits `I`, `L`, `O`, or `U`.

The database stores:

- A SHA-256 digest for redemption lookup.
- The final four-character hint.
- An encrypted full code for Admin reveal.
- Reward, expiration, state, creator, and redemption data.
- Code format `crockford-v2` or `legacy-v1`.

The generation dialog keeps the returned codes visible until the Admin closes it. It supports individual copy, copy all, and CSV export.

The Redemption Codes page masks codes by default. An Admin can reveal, copy, batch copy, or export full unused codes. Each reveal or export writes an audit record.

Reveal and export require a reauthentication grant created from the Admin current password. The grant is bound to the Admin user and dashboard session, is scoped to redemption-code access, and expires after five minutes. The server stores only a hash of the grant token.

The current account model requires a local password hash. If a future SSO or passwordless Admin has no locally verifiable password, sensitive Store actions fail closed until an explicit step-up provider supplies a signed assertion with an authentication time no older than five minutes. A normal SSO login assertion is not sufficient.

Logout, password change, session rotation, session expiration, Admin-role removal, or account disablement invalidates every related reauthentication grant.

Reveal returns at most 20 selected codes. Export returns at most 100 selected codes. Responses set `Cache-Control: no-store`, `Pragma: no-cache`, `Referrer-Policy: no-referrer`, and `X-Content-Type-Options: nosniff`. CSV uses `Content-Disposition: attachment`.

Audit records contain Admin ID, action, selected code IDs, count, IP, user agent, and time. They do not contain full codes. Application logs do not contain full codes or reauthentication tokens.

Used, expired, or revoked codes remain masked and cannot be returned by a reveal endpoint.

Successful redemption and revocation delete the encrypted full code in the same transaction. A Primary cleanup task deletes encrypted full values within 24 hours after expiration. Digest, hint, reward, status, and audit fields remain.

Existing codes created before this design use legacy alphabet `ABCDEFGHJKLMNPQRSTUVWXYZ23456789`, format `legacy-v1`, and no encrypted full value. They remain redeemable by users, but Admin can see only the final four characters. Admin can revoke and replace them.

User redemption converts ASCII letters to uppercase and removes ASCII hyphens. It does not map look-alike characters. A v2 candidate must contain exactly 16 Crockford characters. A legacy candidate must contain exactly 16 legacy characters.

The service computes the applicable v2 and legacy SHA-256 digest candidates and performs one bounded database lookup with code format. Invalid syntax and no match both return HTTP 404 `invalid_redemption_code` with the same public message.

Redemption locks or serializes the code row and changes an unused code exactly once in the same reward transaction.

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

Payment compliance acknowledgement is versioned. The acknowledgement states that the operator controls the merchant account, registered callback, offered products, refund process, dispute process, and applicable customer notices. It is an operational confirmation, not a substitute for legal review.

Confirmation requires an Admin dashboard session, reauthentication, explicit `confirmed = true`, and the current terms version. The database stores terms version, Admin ID, time, and source IP. API access tokens cannot confirm it.

A terms-version change invalidates the prior acknowledgement and makes every production Channel effectively unavailable until a current confirmation exists. It does not disable callback verification, provider query, refunds, or reconciliation for historical attempts.

Effective Channel availability is true only when the stored Channel is enabled, current compliance acknowledgement exists, required credential fields exist, callback verification is configured, at least one product and settlement currency are compatible, and adapter capability validation passes. Public Store responses return only effectively available Channels.

A production Channel must provide either an automated settlement-report operation or a documented Admin upload format with signature or digest verification. A Channel without either path cannot pass the production gate.

Each Channel stores a capability register. Every payment query, refund, refund query, dispute event, dispute query, bill download, and settlement-report capability is `supported`, `unsupported`, or `manual`.

A capability entry stores merchant-account digest, environment, provider product name, permission or API response digest, test transaction ID when applicable, verifier Admin, verified time, and expiry time. Production verification expires after 90 days or immediately after credential, merchant account, provider product, or API-version change.

Core checkout, payment query, callback verification, refund, refund query, and settlement report must be supported. Optional dispute automation can be unsupported only when the manual case path, provider deadline source, owner assignment, and tabletop drill pass.

Admin cannot mark a capability supported without a passing adapter probe or controlled transaction. Screenshots alone do not prove an automated capability. A manual capability requires a documented provider console path and evidence-upload procedure.

Saving credentials does not prove that the provider account is active. Alipay sandbox, Stripe test mode, and a controlled WeChat live test are separate release checks.

## 13. Reconciliation And Alerts

Only the primary node runs payment reconciliation. A database lease with a fencing token prevents two reconcilers from processing the same due item.

The reconciler runs once per minute. It selects bounded batches with deterministic order and processes:

- Presented attempts whose provider expiration has passed.
- Paid orders whose fulfillment state is `pending` for at least 30 seconds.
- Paid orders whose fulfillment state is `failed`.
- Refunds in `refund_pending`.
- Callback events marked retryable.

Before closing an expired unpaid order, the reconciler queries the provider. A positive payment result changes the order to paid and starts fulfillment. A confirmed unpaid result closes the order.

The reconciler immediately attempts a due paid/pending order through the same idempotent fulfillment transaction. Retryable pending or failed fulfillment uses delays of 30 seconds, two minutes, ten minutes, and one hour. Later failures remain visible for Admin processing.

A refund-pending order is queried after one minute, five minutes, 15 minutes, and then hourly. It raises an alert after 15 minutes and remains pending until the provider returns a definite result.

The system records metrics for callback rejection, late payment, duplicate payment, failed fulfillment, stale unpaid attempts, refund timeout, reconciliation failures, and lease loss. Admin shows the affected order count and the latest error.

Provider events distinguish payment, refund, dispute opened, dispute won, dispute lost, chargeback, fee, and settlement adjustment. Each order has dispute state `none`, `open`, `won`, or `lost`.

Every automated dispute event passes the adapter signature verifier and account-identity check. `(credential_version_id, provider_event_id)` is unique. An adapter without a provider event ID uses a documented canonical signed-field digest. Repeated events return success without another reserve, ledger entry, or state change.

A verified dispute-open event places the user in payment hold and adds a claim to the shared order recovery row. It reserves only the unrecovered original credit that is still available in the user balance; an existing refund reserve is reused. It also suspends an active plan sourced from that order.

A dispute-won event resolves its claim. It releases the shared reserve only when no refund, dispute, or chargeback claim remains, restores the plan only until its original end time, and clears the hold when no other dispute remains.

A verified dispute-lost or chargeback event consumes the shared reserve and writes one idempotent negative ledger adjustment only for `original - recovered - reserved`. The recovery row then records `recovered = original`. The balance can become negative. It revokes an active plan from that order and keeps the account payment hold until Admin review.

Payment hold does not disable login, API keys, Store reads, order reads, callbacks, refunds, or reconciliation.

Payment hold rejects new Store order creation with HTTP 423 `payment_hold`. It rejects plan-funded API admission. A request with no applicable plan can continue through ordinary balance only when the existing effective-balance and account-enabled rules permit it; hold does not create a plan-to-balance fallback.

Payment hold rejects redemption before code lookup or mutation with HTTP 423 `payment_hold`. It does not consume, expire, or reveal the submitted code.

An already presented payment attempt remains queryable. A verified payment is recorded, but pending fulfillment remains blocked while hold is active. Admin can refund it or clear the hold after every recovery claim is resolved.

An active plan unrelated to the disputed order keeps its original dates but cannot fund requests during hold. The system does not extend its end time for the blocked interval.

The system clears hold automatically only when every dispute is won or closed without loss, every recovery claim is resolved, and user balance is nonnegative. Otherwise Admin reauthentication, a reason, and resolved reconciliation case are required. Admin cannot clear hold while balance is negative or a provider claim remains open.

The Primary imports or fetches each provider daily settlement report when the provider supports it. A report line has a provider-unique ID and classifies gross charge, refund, dispute, fee, tax, currency conversion, and net settlement. Reimport is idempotent.

Provider fees, taxes, and settlement FX differences are merchant accounting entries and do not change user rewards. An unmatched payment, refund, dispute, or unknown settlement difference creates a reconciliation case and critical alert.

An unresolved paid fulfillment older than one hour, refund older than 24 hours, reconciliation case older than 24 hours, or dispute without an update for seven days remains open and appears in Admin. The system never silently closes a provider-money mismatch by age.

Production configuration names a primary Payment Operations Owner, a distinct backup owner, and a Finance Approver. All three identities are enabled Admin users. A production Channel cannot enable without these assignments and an alert destination.

The system assigns each manual case to the primary owner and starts an SLA clock. Unmatched payment, refund, chargeback, or lost-dispute cases are critical: acknowledge within 15 minutes, stop affected new checkout within 30 minutes when exposure can grow, and record provider evidence within four hours.

Paid-but-unfulfilled and refund-pending cases require acknowledgement within 30 minutes. Statement fee or settlement-FX differences require acknowledgement by the next business day. A provider dispute response must be submitted at least 24 hours before the provider deadline.

An unacknowledged case escalates to the backup owner. A missed critical four-hour target or provider deadline escalates to the Finance Approver and business owner. Alerts remain active until the case has evidence, a resolution code, and an audit record.

Case closure, permitted payment-hold clearance, ledger recovery adjustment, or acceptance of an unexplained settlement difference requires reauthentication by the assigned owner and approval by the distinct Finance Approver. Neither user can approve their own action.

Admin shows case owner, severity, provider deadline, internal deadline, evidence links, action history, approver, and escalation state. It does not permit deletion of a case or its audit trail.

Every automated state change writes the same audit format as an Admin action and identifies the reconciler actor.

## 14. Migration

A read-only preflight runs before the schema migration. It reports row counts by order status, timestamp consistency, orphan references, duplicate order numbers, duplicate provider transaction fields if present, and every table or column that collides with a new payment object.

The current schema permits only `pending`, `completed`, and `cancelled`. The preflight aborts when it finds any other value, a status/timestamp mismatch, an unknown legacy callback table, a partially populated payment field, or an orphan reference. It never maps an unknown value to a known state.

The schema migration runs in one database transaction after preflight passes.

Existing completed orders migrate to payment state `paid` and fulfillment state `fulfilled`. Existing pending and cancelled orders migrate to payment state `closed` and fulfillment state `pending`. A new callback cannot fulfill a legacy order.

Migrated orders use payment-contract version 1 and have no provider attempt or callback identity. Version 1 `closed` is terminal. Callback, query, refund, and `closed -> paid` paths accept only version 2 orders. If preflight finds evidence that a legacy order received provider payment, migration aborts for a dedicated importer; it does not close that order.

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

One deployment has exactly one Store-serving Primary process. The reverse proxy routes Store Dashboard endpoints and public payment callbacks only to that Primary. Replica nodes do not mount those endpoints; they call the Primary internal admission service for plan reservations.

Order, polling, and callback token buckets are process-wide because one Primary serves them. Redemption failure counters and cooldowns are stored in the database so a restart cannot clear them.

A future deployment with more than one Store-serving process must configure a gateway or shared rate-limit backend before startup. Without an explicit shared mode, a second Store-serving process is an invalid topology and must not accept Store traffic.

### 15.1 Primary Availability And Recovery

Every production deployment declares one Store availability profile: `postgresql_primary` or `sqlite_primary`. Startup rejects a missing profile or a profile that does not match the configured database backend.

The PostgreSQL profile has a Store RTO of five minutes and a committed-state RPO of zero. Health routing detects an unavailable Primary within 30 seconds. Promotion first fences the old Store process by revoking its lease epoch and removing its callback and Dashboard routes. The replacement acquires a database-backed exclusive Store lease with a new monotonic epoch before it mounts any Store endpoint or issues an admission token. A process that loses the lease fails Store writes and admissions closed within one lease interval. The lease interval is at most 15 seconds.

PostgreSQL database failover is owned by the configured database service. The replacement Store process starts only after the promoted database accepts writes and the prior writer is fenced. It loads pending payment attempts, callback events, quota reservations, and spool acknowledgements from committed storage. It queries ambiguous provider attempts before retrying. Provider callback and query reconciliation starts before checkout is re-enabled.

The SQLite profile has a process-restart RTO of two minutes on a healthy host. Host loss has an RTO of 30 minutes and an RPO of at most 60 seconds. Production SQLite requires continuous encrypted off-host replication or backup with a measured copy lag of at most 60 seconds. A backup stored only on the application host is invalid. The restore procedure verifies database integrity, Store migration version, key-ring availability, SQLite quota-gate fingerprint, and the last replicated timestamp before routing Store traffic.

During every Primary outage, checkout, payment polling mutations, refund mutations, redemption, and plan admission fail closed. Public callbacks may receive a temporary failure so the provider retries. Ordinary API requests funded by the existing balance continue only when they do not require the Store Primary or a payment hold. After SQLite recovery, reconciliation queries every attempt created since the last confirmed backup boundary before checkout resumes. A missing or ambiguous provider query keeps the affected order blocked for manual review.

Each enabled Channel capability record includes the provider's measured callback retry window and query-availability window. Both windows must exceed the declared Store RTO by at least 15 minutes. A Channel with a shorter or unknown window cannot enable in production.

Production readiness includes one PostgreSQL or SQLite drill matching the selected profile. The drill removes Primary routing, proves fencing, promotes or restores the replacement, replays duplicate callbacks, reconciles ambiguous attempts, and measures detection time, RTO, RPO, and provider retry recovery. A release fails when any measured value exceeds the declared objective or when two Store Primaries accept writes in one lease epoch.

Payment and reveal endpoints require the existing dashboard authorization model. Reveal, export, refund, reprocess, exchange-rate override, key rotation, and credential updates require Admin plus a matching five-minute reauthentication scope.

Every cookie-authenticated Store mutation requires `Content-Type: application/json` or the documented multipart icon type and an `Origin` equal to the configured public origin. A missing or mismatched Origin fails. State-changing actions never use GET. Provider callbacks are exempt from session CSRF checks and require provider verification instead.

The dashboard session cookie remains HttpOnly, Secure, SameSite Strict, and Path `/`. Session invalidation is checked before accepting a reauthentication grant or a sensitive mutation.

### 15.2 Privacy, Retention, And Data Residency

Production stores one accepted, versioned Store privacy record. The record names the policy version, jurisdiction, allowed storage regions, retention values, legal basis, privacy reviewer, approval evidence digest, approved time, and next review time. The next review time is no more than 365 days later. Store callbacks and production checkout cannot enable without a current accepted record.

Encrypted raw callback bodies are retained for 30 days. Source IP addresses and User-Agent values are retained for 90 days and are then deleted. A deployment may retain a keyed, non-reversible pseudonym after deletion only when the accepted privacy record states its purpose and rotation period. The pseudonym key is not a payment encryption key.

Allow-listed parsed provider events, payment orders, ledger entries, refunds, disputes, chargebacks, settlement reports, economic-recovery claims, and their immutable financial audits are retained for seven years unless the accepted jurisdiction policy requires a different period. A different period requires an explicit value and reviewer evidence in the privacy record; code does not silently replace the configured value.

Redemption reveal, copy, and export audit records are retained for two years. Reauthentication grant hashes are deleted no later than 24 hours after grant expiry. Expired nonce and callback replay records are deleted after the longer of their protocol replay window and 24 hours. Deleting these short-lived records does not delete the associated financial audit.

Raw callback bodies are accessible only to Payment Operations during an assigned incident and to a Privacy or Security reviewer. Financial records are accessible to Payment Operations and Finance roles. Redemption reveal and export audits are accessible to Security and authorized Admin reviewers. Every read, export, retention override, and legal-hold operation writes an immutable access audit containing actor, role, scope, reason, time, and result.

Payment data, callback evidence, backups, and encryption keys remain inside the allowed storage regions in the accepted privacy record. A cross-region replica, log sink, support export, or backup target is rejected unless its region is listed. Application logs never become an alternative retention store for deleted payment data.

A Primary daily deletion job removes data whose retention period has expired. It records counts by data class, the oldest remaining timestamp, failures, and the policy version. Three consecutive failures create a critical alert and pause new checkout until an authorized operator records containment. Deletion is idempotent and runs in bounded batches.

A legal hold suspends deletion only for its declared data class and identifiers. It stores a reason, scope, requesting authority, approving Privacy or Legal reviewer, start time, and mandatory expiry time. An expired hold stops applying automatically. Extending a hold requires a new approval record. A hold never restores data already deleted.

## 16. Verification

Backend tests cover:

- Official signature and callback test vectors.
- Effective Channel availability truth table across enablement, compliance version, credentials, Webhook, product, currency, and capability state.
- Compliance confirmation session requirement, reauthentication, audit fields, version invalidation, checkout disablement, and continued historical callback processing.
- Trusted success and cancel URL origins, rejected foreign origins, and no local or provider order after rejection.
- Stripe completed, asynchronous success, asynchronous failure, and expired event classification without API-version bypass.
- Callback logs exclude raw body and signature header while retaining event ID and body digest.
- Reduced rational rate parsing, both exchange directions, checked overflow, nano USD conversion, future and stale timestamps, configurable anomaly quarantine, startup failure, and one final rounding point.
- Exchange response digest, parser version, consecutive-failure alerts, persisted pause, and restart without a first snapshot.
- Exchange-source governance acceptance, review expiry, terms change, rate-limit breach, attribution requirement, and source-change rejection.
- Rate warning and critical response clocks, four-hour decision, 24-hour escalation, dual-Admin compromise recovery, and missed-SLA incident review.
- Stripe account-country and API-version capability validation with unsupported-currency failure.
- Amount, currency, merchant, and order mismatch.
- Twenty concurrent copies of one callback with one fulfillment.
- Callback versus expiration, close, refund, and a second provider transaction.
- Version 1 closed-order callback rejection and version 2 late-payment acceptance.
- Retry after verified payment and failed fulfillment.
- Crash after payment commit with paid/pending reconciliation and one later fulfillment.
- Balance refund reservation, concurrent spending, one compensation, and ambiguous provider results.
- Refund, dispute, and chargeback claims in every order, with `reserved + recovered <= original` and one recovery ledger debit.
- SQLite and PostgreSQL transition-trigger rejection, duplicate event application, duplicate claim identity, duplicate ledger key, missing trigger startup failure, and concurrent recovery attempts across multiple connections.
- Every ordering of fulfillment, refund start, callback retry, and reconciliation, with one final reward state.
- Plan refund rejection after fulfillment.
- Order idempotency-key replay, mismatched input, open-order cap, creation rate limit, and polling rate limit.
- Product revision conflicts, emergency disable versus callback, immutable quote triggers, plan replacement races, plan expiration, and every quota window.
- Primary quota reservation, Replica admission token, concurrent reservations, replacement generations, exact settlement, release, and above-reserve anomaly.
- An isolated PostgreSQL quota load drill with five Replicas, 10,000 entitlements, five windows per entitlement, and at least `max(500 requests/second, two times measured seven-day peak)` for ten minutes. It includes 100 concurrent requests against one entitlement and never starts PostgreSQL on the user's computer.
- A SQLite WAL quota drill at its defined target, including concurrent admission, settlement, replacement, lock timeout, process restart, and zero unhandled `SQLITE_BUSY`. Failure proves that plan products require PostgreSQL.
- SQLite pending and failed gates across Admin product writes, catalog reads, plan redemption generation, order creation, fulfillment, database triggers, startup version mismatch, and frontend controls.
- SQLite unchanged compatibility ID, pre-cutover `next` manifest promotion, active-entitlement startup refusal, no-entitlement automatic rerun, failed-drill alert, and acknowledged retry.
- Quota fault drills that stop the Primary after reserve, expire tokens, delay and duplicate spool shipment, replace a plan during traffic, inject database lock waits, and shift a Replica clock by plus or minus two minutes.
- Ed25519 admission-token signature, unknown key, wrong node audience, durable same-node replay, cross-node replay, TTL, clock skew, key publication, activation, and prior-key retirement.
- Quota acceptance requires zero duplicate or missing finalizations, exact reservation conservation, no charge to a replacement generation, p95 admission below 100 ms, and p99 below 250 ms at the target load. Injected outages are excluded from latency percentiles and must fail closed.
- HTTP milestone JSON, form, redirect, QR, and form actions.
- HTTP milestone DNS rebinding, private addresses, redirect rejection, timeout, response limits, template limits, and query-before-retry behavior.
- Encryption, wrong-key failure, and prior-key decryption.
- Key rotation, bounded re-encryption, referenced credential retention, and old-key removal refusal.
- Crockford v2 generation, legacy v1 lookup, invalid alphabet, new-code reveal, reauthentication expiry, response headers, rate limits, and existing-code non-recoverability.
- CSRF Origin rejection, session invalidation, role removal, and passwordless Admin fail-closed behavior.
- Reconciler lease fencing, due-order selection, retry schedule, and alert thresholds.
- Dispute open, win, loss, chargeback debt, plan suspension, daily settlement reimport, fee classification, and unmatched-line cases.
- Every event-order matrix row and pairwise reversed delivery, including refund-before-payment, dispute-before-payment, dispute during refund pending, chargeback during refund pending, refund success after chargeback, dispute-before-fulfillment, stale terminal events, and query-required conflicts.
- Per-adapter capability tests for supported and unsupported dispute, query, refund, and settlement operations.
- Merchant capability register probe evidence, 90-day expiry, credential-change invalidation, screenshot rejection, and manual-path tabletop requirement.
- Payment-hold Store read, checkout rejection, redemption non-consumption, plan block, ordinary-balance boundary, pending-payment handling, and hold-clear refusal.
- Manual case assignment, acknowledgement and escalation clocks, provider deadline, dual approval, self-approval rejection, and immutable case audit.
- Single Store Primary routing, Replica endpoint absence, persisted redemption cooldown, and invalid multi-process topology.
- PostgreSQL Store lease fencing, stale-epoch rejection, 30-second detection, five-minute promotion, and zero committed-state loss.
- SQLite process restart, off-host restore, 60-second backup lag, 30-minute host-loss recovery, and post-restore provider reconciliation.
- Provider retry-window capability rejection when the measured window does not exceed the selected RTO by 15 minutes.
- Privacy-record expiry, regional target rejection, role-scoped evidence access, immutable access audit, and checkout refusal without an accepted policy.
- Daily bounded deletion for raw callbacks, IP, User-Agent, reauthentication grants, replay records, and expired legal holds.
- Legal-hold scope, approval, expiry, extension, and non-restoration of already deleted data.
- SQLite migration and isolated PostgreSQL migration.
- Migration preflight rejection for every unknown or inconsistent legacy state.
- A clean-room review records external commit `8f6961c675932f406260ff0c218bc2aa0603e9b2`, adopted behaviors, license difference, and a no-verbatim-copy check.
- License review record completeness, dependency digest, similarity-hit release block, reviewer authorization, and missing-review production refusal.

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
6. Account and currency capability validation passes.
7. Supported dispute events or the documented manual case path, plus the settlement-report path, are configured.
8. Exchange-rate refresh, quarantine, pause, and alert checks pass.
9. The reverse proxy routes Store traffic to exactly one Primary.
10. A SQLite deployment passes the SQLite quota drill, or plan products remain disabled until PostgreSQL is used.
11. A deployment with Replicas passes admission-token rotation and replay drills.
12. Event-order and payment-hold acceptance tests pass for every enabled official adapter.
13. Primary and backup Payment Operations owners, a distinct Finance Approver, primary and backup Rate Operations Admin assignments, and alert destinations are configured. Rate roles can reuse Payment Operations identities, but one user cannot propose and approve the same action.
14. A tabletop drill passes for rate-source outage, suspected source compromise, unmatched payment, manual dispute, missed provider deadline, and owner escalation.
15. Current payment compliance terms are acknowledged and the clean-room license review is recorded.
16. Database transition and recovery constraints are installed, verified, and pass multi-connection concurrency tests on the production backend type.
17. The SQLite quota fingerprint is prevalidated for the release, or the deployment uses PostgreSQL for plan products.
18. Every enabled Channel has a current merchant capability register and a passing automated or manual evidence path.
19. The exchange-source governance and formal license review records are current and accepted by their assigned reviewers.
20. The selected Store availability profile has a current failover drill with measured detection time, RTO, RPO, fencing, provider retry recovery, and no dual-Primary write acceptance.
21. SQLite deployments have an encrypted off-host backup target with measured lag at most 60 seconds. PostgreSQL deployments have a verified database-writer fencing path.
22. Every enabled Channel has measured callback retry and query windows that exceed the selected Store RTO by at least 15 minutes.
23. A current Store privacy record defines jurisdiction, allowed regions, retention, access roles, deletion, and legal-hold policy. Its reviewer evidence is accepted.
24. The deletion job has completed successfully, regional targets match the accepted privacy record, and no unresolved critical retention alert exists.

Production deployment does not invent or embed merchant credentials. The operator must supply them through Admin or deployment configuration.

Before the first real payment, rollback can restore the old image and the matching pre-release database backup.

After any provider accepts a real payment or refund, restoring the old database is forbidden. The operator must disable new checkout, keep callback ingestion available when possible, reconcile provider transactions against local events, and forward-fix or restore into the new schema. Every transaction created after the release backup must remain represented.

A rollback drill tests both boundaries: a pre-payment image-and-database restore, and a post-payment maintenance procedure that preserves new provider transactions.
