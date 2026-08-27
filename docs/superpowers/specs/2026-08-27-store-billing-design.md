# LynShen Store, Payment, And Redemption Design

## 1. Scope

This design replaces the current Store layout and the manual order-completion flow.

The Store sells one-time balance recharge products and one-time plan products. It accepts redemption codes. It supports these payment adapters:

- Alipay official website and mobile website payment.
- WeChat Pay v3 Native and H5 payment.
- Stripe Checkout.
- A configurable HTTP payment adapter.

The first release does not support recurring subscriptions. An Admin cannot mark an unpaid order as paid.

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

Plan quota values use CNY as their stored base. The currency control changes presentation. Plan quota presentation uses whole units and round-half-away-from-zero.

Alipay and WeChat Pay settle in CNY. When the Store currency control shows USD, the order summary also shows the exact CNY amount that the gateway will charge.

Stripe Checkout can charge CNY or USD when the Stripe account supports the selected currency.

Each configurable HTTP Channel declares an allow-list containing `CNY`, `USD`, or both.

Order creation stores an immutable exchange-rate snapshot. A later rate refresh does not change an existing order amount.

## 4. Payment Core

The payment core owns order state, payment attempts, callback events, fulfillment, refunds, and audit records.

Each adapter implements these operations:

1. `create_checkout`
2. `query_payment`
3. `verify_callback`
4. `refund_payment`

An adapter cannot write a user balance or activate a plan. Only the payment core can fulfill an order.

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

The configurable HTTP adapter supports JSON and form-encoded checkout requests.

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

The initial signature allow-list is RSA2, HMAC-SHA256, and MD5. MD5 requires an explicit legacy warning in Admin.

The response mapping can produce a redirect URL, QR payload, or signed form. A mapped redirect or form action must use HTTPS.

The callback configuration must identify the provider transaction ID, merchant order number, amount, currency, status, timestamp, nonce, and signature.

## 7. Secret Encryption

Payment credentials and recoverable redemption codes use authenticated encryption.

The service loads a key ring from deployment configuration. The key ring identifies one active 256-bit key and zero or more decryption-only prior keys. The database stores the key ID, random nonce, ciphertext, and format version.

Encryption uses XChaCha20-Poly1305. Associated data includes the table name, row ID, and field name.

Admin reads never return a saved payment credential. Admin can only replace it.

If encrypted rows exist and no matching key is available, the service rejects payment creation and redemption-code reveal. It does not delete or overwrite encrypted values.

The deployment backup must include the database and the matching key ring. Restoring only one of them is not a valid recovery.

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

Order creation inserts an unpaid order and one payment attempt. It stores the product snapshot, reward snapshot, settlement amount, settlement currency, exchange rate, Channel ID, and adapter kind.

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

Steps 6 through 9 run in one database transaction. A duplicate callback does not create another ledger credit or entitlement.

If payment verification succeeds but fulfillment fails, the event remains retryable and the order shows paid with failed fulfillment. An Admin can reprocess the verified event.

A database write failure returns a non-success callback response. The provider can retry according to its protocol.

## 9. Refunds And Reconciliation

Admin order actions are:

- Query provider status.
- Reprocess a verified callback.
- Close an unpaid order.
- Request a refund.

The normal UI does not contain a manual Complete action.

Refund requests use the original provider transaction and amount. Provider acceptance changes payment state to `refund_pending`. A verified refund result changes it to `refunded`.

The first release supports full refunds. It does not implement partial refunds.

A paid but unfulfilled order needs no reward reversal.

A fulfilled balance order can start a refund only when the user balance is at least the original credited nano USD amount. Starting the refund deducts that amount as an idempotent refund reserve. A definite provider rejection releases the reserve exactly once. A verified refund keeps the deduction.

A fulfilled plan order can start a refund only when its entitlement is still active, the entitlement source is that order, and no settled API usage exists after its start time. Starting the refund suspends that entitlement. A definite provider rejection restores it. A verified refund revokes it.

A provider timeout or unknown result leaves the order in `refund_pending`. Admin must query provider status before retrying. The service does not create a second refund request for the same order.

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

Used, expired, or revoked codes remain masked and cannot be returned by a reveal endpoint.

Existing codes created before this design have no encrypted full value. They remain redeemable by users, but Admin can see only the final four characters. Admin can revoke and replace them.

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

## 13. Migration

The migration runs in one database transaction.

Existing completed orders migrate to payment state `paid` and fulfillment state `fulfilled`. Existing pending and cancelled orders migrate to payment state `closed` and fulfillment state `pending`. A new callback cannot fulfill a legacy order.

Existing Alipay and WeChat Channel rows become disabled, unconfigured official Channels. Existing custom Channel rows become disabled, unconfigured HTTP Channels. The migration creates one disabled, unconfigured Stripe Channel when none exists.

The migration adds encrypted-code fields without changing existing digests. Existing redemption codes cannot be recovered because the prior digest is irreversible.

Migration tests cover SQLite and PostgreSQL SQL behavior. Local development runs SQLite only. PostgreSQL tests run in an isolated environment and never start PostgreSQL on the user's computer.

## 14. Security Limits

Callback endpoints are public and require adapter verification.

The callback request body has a fixed size limit. Logs and API responses redact signatures, private keys, API secrets, full redemption codes, and sensitive callback fields.

Provider transaction IDs have a database unique constraint. Callback event IDs use adapter-specific idempotency keys.

Generic callback replay protection validates timestamp and nonce when the configured protocol supplies them. A nonce cannot be accepted twice for the same Channel.

Payment and reveal endpoints require the existing dashboard authorization model. Reveal, export, refund, reprocess, and credential updates require Admin.

## 15. Verification

Backend tests cover:

- Official signature and callback test vectors.
- Exact decimal and currency conversion.
- Amount, currency, merchant, and order mismatch.
- Twenty concurrent copies of one callback with one fulfillment.
- Retry after verified payment and failed fulfillment.
- Query and full refund behavior.
- HTTP JSON, form, redirect, QR, and form actions.
- Encryption, wrong-key failure, and prior-key decryption.
- New-code reveal and existing-code non-recoverability.
- SQLite migration and isolated PostgreSQL migration.

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
- One controlled transaction for each enabled configurable HTTP Channel.

## 16. Production Gate

Code deployment can occur with every payment Channel disabled.

A Channel can accept production purchases only after:

1. The deployment key ring is installed and backed up.
2. Required merchant credentials are configured.
3. The exact public callback URL is registered with the provider.
4. Signature and callback verification passes.
5. One controlled payment and refund passes.

Production deployment does not invent or embed merchant credentials. The operator must supply them through Admin or deployment configuration.
