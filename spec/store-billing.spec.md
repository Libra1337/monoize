# Store Billing Specification

## 0. Scope And Terms

SB-0.1. The Store MUST sell one-time balance products and one-time plan products.

SB-0.2. The Store MUST support Alipay, WeChat Pay v3, and Stripe Checkout in Milestone 1.

SB-0.3. The configurable HTTP payment adapter belongs to Milestone 2. A draft HTTP Channel MUST NOT accept production checkout or callbacks in Milestone 1.

SB-0.4. The Store MUST support purchase orders, payment attempts, verified provider events, fulfillment, full refunds, disputes, chargebacks, settlement reconciliation, and redemption codes.

SB-0.5. An Admin MUST NOT set an unpaid order to paid or fulfilled. The API MUST NOT expose a manual paid or manual complete endpoint.

SB-0.6. `CNY` and `USD` are the only Store currencies.

SB-0.7. A payment amount MUST be a canonical base-10 integer string in the currency minor unit. A canonical nonnegative integer string matches `0|[1-9][0-9]*`.

SB-0.8. CNY minor units are fen. USD payment minor units are cents. The account ledger remains integer nano USD in `users.balance_nano_usd`.

SB-0.9. Payment code MUST NOT use `f32` or `f64` for money, exchange rates, quotas, reserves, or settlements.

SB-0.10. A Store deployment MUST mount Store mutation and callback endpoints on exactly one Store Primary unless a later specification defines shared routing and shared rate limits.

## 1. Exchange Rates And Money

SB-FX-1. The Store Primary MUST request `https://open.er-api.com/v6/latest/USD` at startup and 15 minutes after each completed attempt.

SB-FX-2. A Replica MUST read the persisted rate. A Replica MUST NOT request or update the remote rate.

SB-FX-3. The HTTP client MUST use the exact HTTPS host, system trust store, no redirects, a five-second timeout, and a 65,536-byte response limit.

SB-FX-4. A valid response MUST have `result = success`, `base_code = USD`, one positive integer source timestamp, and one positive finite decimal `rates.CNY` with at most 18 fractional digits.

SB-FX-5. The source timestamp MUST NOT be more than five minutes in the future, more than 48 hours before refresh time, or older than the active source timestamp.

SB-FX-6. The decimal rate MUST be between 1 and 20 CNY per USD inclusive.

SB-FX-7. The service MUST parse the decimal as reduced positive integers `N/D`. It MUST NOT persist or calculate a rounded reciprocal.

SB-FX-8. USD cents to CNY fen MUST equal `round(cents * N / D)`. CNY fen to USD cents MUST equal `round(fen * D / N)`.

SB-FX-9. USD cents to nano USD MUST equal `cents * 10,000,000`. CNY fen to nano USD MUST equal `round(fen * D * 10,000,000 / N)`.

SB-FX-10. Nano USD to plan CNY fen MUST equal `round(nano_usd * N / (D * 10,000,000))` using the entitlement rate generation.

SB-FX-11. `round` in SB-FX-8 through SB-FX-10 means round half away from zero. The service MUST round once at the final nonnegative rational result.

SB-FX-12. Checked integer overflow MUST return HTTP `409` with code `amount_overflow` and MUST write no order or ledger mutation.

SB-FX-13. A conversion-dependent checkout MUST fail with HTTP `503` and code `exchange_rate_unavailable` when the active local refresh is older than 60 minutes or the source timestamp is older than 48 hours.

SB-FX-14. A failed refresh MUST retain the prior snapshot without changing its validity timestamps.

SB-FX-15. A candidate rate change above the configured threshold MUST enter quarantine. The threshold MUST be configurable from 0.1 percent through 25 percent and MUST default to 5 percent.

SB-FX-16. Admin approval of a quarantined rate and Admin resume after suspected compromise MUST require reauthentication. Suspected compromise recovery MUST require two valid observations at least 15 minutes apart and two distinct Admin identities.

SB-FX-17. Each order MUST store the exact rate rational, source timestamp, refresh timestamp, settlement amount, and settlement currency used at quote time. Later rate updates MUST NOT change an order.

SB-FX-18. Alipay and WeChat Pay MUST settle in CNY. Stripe MAY settle in CNY or USD only when the configured Stripe account capability allows the currency.

SB-FX-19. The Store MUST persist refresh attempt URL, parser version, HTTP status, response-body SHA-256 digest when present, result category, consecutive failure count, and attempt time. It MUST NOT log the response body.

SB-FX-20. Three consecutive refresh failures MUST create a warning. Six failures or expiry within 15 minutes MUST create a critical alert.

SB-FX-21. Conversion checkout MUST require a current accepted exchange-source governance record. The review interval MUST NOT exceed 180 days.

## 2. Products, Quotes, And Entitlements

SB-P-1. A product MUST have an immutable ID, kind, name, description, price currency, price minor amount, sort order, enabled state, and monotonic revision.

SB-P-2. A product kind MUST be `balance` or `plan`. A price MUST be greater than zero.

SB-P-3. A balance product MUST store recharge and bonus amounts in its price currency. Actual received MUST equal recharge plus bonus with checked integer addition.

SB-P-4. A custom recharge MUST use the Admin minimum and maximum for its selected payment currency. Its bonus MUST be zero.

SB-P-5. A plan price and quota base MUST use CNY. A currency control MAY present the price in USD by using the active exchange rate.

SB-P-6. A plan MUST have a duration from 3,600 through 31,536,000 seconds and at least one quota rule.

SB-P-7. A quota rule kind MUST be `5h`, `12h`, `day`, `week`, `month`, or `custom`. A custom rule MUST use a whole-hour duration from 1 through 8,760 hours.

SB-P-8. Quota amounts MUST be positive CNY fen. Two rules in one plan MUST NOT use the same effective window duration.

SB-P-9. All plan quota rules MUST apply concurrently. `5h`, `12h`, and `custom` windows MUST roll. Day, week, and month windows MUST use Asia/Shanghai calendar boundaries. A week MUST start on Monday.

SB-P-10. A plan quota shown to a user MUST use a whole currency unit and round half away from zero. It MUST NOT show a decimal separator.

SB-P-11. A plan MAY contain zero or more Group IDs. Group IDs MUST follow `groups-registry.spec.md`. A nonempty list MUST restrict routing by intersection with the user's available Groups.

SB-P-12. Product update, disable, emergency disable, and delete MUST require the expected revision. A stale revision MUST return HTTP `409` and change no row.

SB-P-13. An order MUST contain an immutable versioned snapshot of product, price, reward, duration, Groups, quota rules, Channel public identity, settlement currency, rate rational, and payment contract version.

SB-P-14. Product edits MUST affect only orders created after the edit. A product referenced by an order MUST NOT be physically deleted.

SB-P-15. Disabling a product MUST NOT modify an existing unpaid order. That order MAY be paid until its 30-minute expiration.

SB-P-16. Emergency disable MUST create a close request for each unpaid order. It MUST query the provider before closing an attempt that may have been presented.

SB-P-17. One user MUST have at most one current Store entitlement pointer. Each entitlement generation MUST be immutable.

SB-P-18. A new plan fulfillment or redemption MUST replace the active plan immediately. Unused time and quota MUST NOT carry forward.

SB-P-19. Each entitlement generation MUST copy one immutable exchange rational `N/D`. Every reservation and settlement for that generation MUST use that rational.

SB-P-20. `(source_kind, source_id)` MUST be unique for entitlements. `source_kind` MUST be `order` or `redemption`.

## 3. Payment Channels, Credentials, And Capability

SB-C-1. A Channel adapter kind MUST be `alipay`, `wechat`, `stripe`, or `http`.

SB-C-2. The migration MUST create disabled, unconfigured Alipay, WeChat Pay, and Stripe Channels when an equivalent Channel is absent.

SB-C-3. A saved Channel credential MUST create a new immutable encrypted credential version. It MUST NOT overwrite a prior version.

SB-C-3A. `POST /api/dashboard/store/admin/reauth` MUST accept an Admin password and scope. Scope `credential_update` MUST return one random token with a five-minute expiry. The database MUST store only the token SHA-256 digest and current session-token SHA-256 digest. A grant MUST be invalid after its session becomes invalid, its user loses Admin role, or its expiry passes.

SB-C-3B. `PUT /api/dashboard/store/admin/payment-channels/{id}/credential` MUST require header `X-Store-Reauth-Token` with a valid `credential_update` grant. It MUST validate the adapter-specific credential, encrypt the exact JSON with record-bound AAD, insert one active version, retire every prior active version, invalidate merchant capabilities, and disable the Channel in one transaction.

SB-C-3C. A successful credential replacement response MUST contain only credential version ID, Channel ID, adapter kind, account-identity digest, status, and creation time. It MUST set `Cache-Control: no-store`. It MUST NOT return any credential field or ciphertext field.

SB-C-3D. A persisted Channel adapter kind MUST be immutable. `PUT /api/dashboard/store/admin/payment-channels/{id}` MUST require `expected_revision`. It MUST return HTTP `409` when the stored revision differs. Credential replacement MUST lock the Channel row, increment its revision, and disable it in the credential transaction.

SB-C-4. Credentials and recoverable redemption codes MUST use XChaCha20-Poly1305 with a 256-bit key, a random nonce, and associated data containing table name, row ID, and field name.

SB-C-5. The key ring MUST name one active encryption key and zero or more decrypt-only prior keys. An encrypted row without a matching key MUST block checkout and code reveal.

SB-C-5A. `MONOIZE_STORE_PAYMENT_KEYS_JSON` MUST contain one JSON object with `active` and `prior`. Each key object MUST contain `id` and `key_base64`. `key_base64` MUST decode to exactly 32 bytes. An absent variable MUST permit startup with payment creation unavailable. An empty or otherwise invalid present variable MUST stop startup.

SB-C-6. Admin reads MUST NOT return saved credentials, private keys, Webhook secrets, API v3 keys, or full redemption codes except through a scoped reveal response.

SB-C-7. A payment attempt MUST store its adapter kind, credential version ID, merchant-account identity, expected payment method, and payment contract version.

SB-C-8. A credential version MUST remain decryptable while any referenced attempt is inside its provider callback, refund, dispute, or reconciliation window.

SB-C-8A. A Stripe credential plaintext MUST be JSON with exactly `secret_key`, `publishable_key`, `webhook_signing_secret`, `api_version`, `account_id`, and `live_mode`. Every string MUST be nonempty. `api_version` MUST use `YYYY-MM-DD` or `YYYY-MM-DD.name` format. Unknown fields MUST fail validation.

SB-C-8B. A Stripe credential account identity MUST equal the lowercase SHA-256 hexadecimal digest of the exact UTF-8 `account_id`. Checkout MUST reject a credential when this digest differs from the immutable attempt identity.

SB-C-9. A Channel MUST have a current compliance confirmation. Confirmation MUST require an Admin session, reauthentication, `confirmed = true`, and the current terms version.

SB-C-10. A terms-version change MUST make production checkout unavailable. It MUST NOT disable historical callback verification, query, refund, dispute, or reconciliation.

SB-C-11. Each Channel capability MUST be `supported`, `unsupported`, or `manual` for payment query, refund, refund query, dispute event, dispute query, bill download, and settlement report.

SB-C-12. A capability record MUST contain merchant-account digest, environment, provider product, evidence digest, controlled transaction ID when applicable, verifier Admin, verification time, and expiry time.

SB-C-13. Production capability verification MUST expire after 90 days and immediately after a credential, merchant account, provider product, or API-version change.

SB-C-14. Checkout, payment query, callback verification, refund, refund query, and settlement report MUST be supported before a Channel becomes effectively available.

SB-C-15. A dispute capability MAY be `manual` only when the manual case path, provider deadline source, owner assignment, evidence path, and tabletop drill are current.

SB-C-16. Effective Channel availability MUST require stored enablement, current compliance, complete credentials, configured callback verification, compatible product and currency, valid amount capability, current merchant capability, current privacy record, current license record, and passing runtime gates.

SB-C-17. The user catalog MUST return only effectively available Channels. It MUST return public name, icon, adapter kind, supported currencies, amount limits, and checkout action kinds.

SB-C-18. A Channel icon MUST be built-in, HTTPS URL, or validated same-origin upload. An uploaded icon MUST pass the byte-signature and SVG restrictions in the prior Store icon contract.

SB-C-19. Every enabled Channel MUST record measured callback retry and query availability windows. Both MUST exceed the selected Store RTO by at least 15 minutes.

## 4. Orders, Attempts, And Provider Mutations

SB-O-1. An order payment state MUST be `unpaid`, `paid`, `refund_pending`, `refunded`, or `closed`.

SB-O-2. An order fulfillment state MUST be `pending`, `fulfilled`, or `failed`.

SB-O-3. A payment attempt state MUST be `created`, `presented`, `expired`, `failed`, or `paid`.

SB-O-4. One order MUST have at most one attempt in `created` or `presented`.

SB-O-5. The service MUST commit the local order and attempt before it sends provider checkout bytes.

SB-O-6. Order creation MUST require a user-scoped `Idempotency-Key`. Reuse with the same canonical request MUST return the same order. Reuse with different input MUST return HTTP `409`.

SB-O-7. A user MUST create at most five orders per minute and have at most ten unpaid unexpired orders. Excess creation MUST return HTTP `429`.

SB-O-8. A user MUST poll one order at most 30 times per minute across that user's order-detail requests on the single Store Primary. The order MUST belong to that user.

SB-O-9. Every provider mutation MUST use a stable idempotency key derived from the local object.

SB-O-10. A timeout, disconnect, HTTP 5xx, or unrecognized response after any provider request byte is sent MUST be ambiguous. The service MUST query provider state before retry.

SB-O-10A. A repeated attempt request that finds the same attempt in `created` state MUST return HTTP `502` with code `payment_provider_ambiguous` unless SB-O-10B applies. It MUST NOT create a second local attempt or use a new Provider idempotency key.

SB-O-10B. Stripe does not expose a Checkout Session query key before Checkout Session creation returns. A repeated request for the same `created` Stripe attempt MUST resend the byte-equivalent creation contract with the same `Idempotency-Key`. Stripe idempotency replay is recovery of the first mutation. It MUST NOT count as a second checkout mutation. A successful replay MUST persist the returned Checkout Session on the original attempt.

SB-O-10C. A Stripe idempotency replay that cannot load its historical credential or runtime configuration MUST return `payment_configuration_unavailable`. It MUST leave the original attempt in `created` state with no failure kind. A later replay with restored configuration MUST reuse the same local attempt and Provider idempotency contract.

SB-O-11. Only a verified Provider `NotFound` result or a Provider rejection that proves no payment object was created MAY permit a second checkout or refund mutation. SB-A-10B is such proof for Stripe Checkout Session creation. A transport error, HTTP 5xx, or unrecognized response is not proof.

SB-O-12. Checkout MUST return one action: `redirect`, `qr`, or `form`. Redirect and form action URLs MUST use HTTPS.

SB-O-12A. `POST /api/dashboard/store/orders/{id}/attempts` MUST commit the attempt before sending provider bytes. A successful response MUST contain `{ "attempt": PaymentAttempt, "action": CheckoutAction }`. The service MUST persist the exact browser-safe action before returning it.

SB-O-12B. Missing runtime keys, missing public origin, credential decryption failure, credential mismatch, and unsupported adapter checkout MUST set a new attempt to `failed` with failure kind `configuration_unavailable`. The endpoint MUST return HTTP `503` with code `payment_configuration_unavailable`. A definite provider rejection MUST set the attempt to `failed` with failure kind `provider_rejected` and return HTTP `422` with code `payment_provider_rejected`. An ambiguous provider result MUST leave the attempt in `created` state and return HTTP `502` with code `payment_provider_ambiguous`. Replaying a failed attempt MUST return the error mapped by its persisted failure kind without another provider request.

SB-O-12D. An order with a failed `provider_rejected` Alipay or WeChat attempt MUST reject a new attempt key with HTTP `409` and code `provider_query_required`. Only a verified Provider `NotFound` or `Closed` query result MAY clear this block. A Stripe rejection classified under SB-A-10B MUST permit a new attempt key because no Checkout Session exists. Rejection MUST occur before another checkout mutation.

SB-O-12E. A new payment-attempt transaction MUST lock its `store_orders` row before it counts or inserts attempts. A payment callback projection transaction MUST lock the same row before it validates or changes an attempt. PostgreSQL MUST use `SELECT ... FOR UPDATE`. SQLite MUST hold the serialized write transaction.

SB-O-12C. A first successful attempt response MUST use HTTP `201`. A repeated successful attempt response MUST use HTTP `200` and MUST return the persisted action without a provider request when the attempt is `presented` or `paid`.

SB-O-13. A custom return or cancel URL MUST match the configured HTTPS origin allow-list exactly. A rejected URL MUST create no local order and no provider object.

SB-O-13A. `MONOIZE_PUBLIC_ORIGIN` MUST be one HTTPS origin without credentials, path, query, or fragment. An absent value MUST permit startup with payment creation unavailable. An empty or otherwise invalid present value MUST stop startup.

SB-O-13B. Stripe success and cancel URLs MUST use `/dashboard/store`. The query MUST contain the exact local `order_id` and `checkout=success` or `checkout=cancel`.

SB-O-14. The allowed payment transitions are `unpaid -> paid`, `unpaid -> closed`, version-2 `closed -> paid`, `paid -> refund_pending`, `refund_pending -> refunded`, and `refund_pending -> paid` after definite rejection and local compensation.

SB-O-15. No other payment transition is valid. SQLite and PostgreSQL MUST enforce equivalent transition constraints in the database.

SB-O-16. The allowed fulfillment transitions are `pending -> fulfilled`, `pending -> failed`, and `failed -> fulfilled`.

SB-O-17. A state update MUST include the expected current state and revision. Zero updated rows MUST cause a fresh read.

SB-O-18. A provider transaction ID MUST be unique per adapter account. A second transaction for one paid order MUST create a reconciliation case and MUST NOT fulfill twice.

SB-O-19. A version-1 closed legacy order MUST reject new callback fulfillment. Version-2 `closed -> paid` MUST record a late-payment alert.

## 5. Official Adapter Contracts

SB-A-1. Every adapter MUST implement checkout creation, payment query, callback verification, and full refund.

SB-A-2. An adapter MUST NOT modify a user balance, entitlement, order state, or recovery row directly.

SB-A-3. Alipay MUST support computer website and mobile website payment and MUST sign with RSA2.

SB-A-3A. An Alipay credential plaintext MUST be JSON with exactly `app_id`, `seller_id`, `merchant_private_key_pem`, `alipay_public_key_pem`, and `environment`. `environment` MUST be `production` or `sandbox`. Checkout MUST use `alipay.trade.page.pay` for `computer_web` and `alipay.trade.wap.pay` for `mobile_web`. The signed form MUST use the fixed official gateway for the selected environment.

SB-A-3B. Alipay checkout MUST settle in CNY. `biz_content` MUST contain the immutable order number, decimal amount with two fractional digits, subject, and a product code. Computer website payment MUST use `FAST_INSTANT_TRADE_PAY`. Mobile website payment MUST use `QUICK_WAP_WAY`. Common parameters MUST use `RSA2`, `utf-8`, JSON, version `1.0`, configured App ID, return URL, notify URL, and a UTC+08:00 timestamp.

SB-A-4. Alipay callback verification MUST check signature, App ID, seller identity, order number, amount, currency, and success state.

SB-A-5. Alipay MUST support trade query, refund, refund query, and bill download. Automated dispute support MUST remain unavailable unless the merchant capability test proves it.

SB-A-5A. Alipay payment query MUST call `alipay.trade.query` at the fixed gateway for the credential environment. It MUST sign App ID, method, format, charset, sign type, timestamp, version, and `biz_content` with RSA2. `biz_content` MUST contain the exact immutable order number.

SB-A-5B. An Alipay query response MUST use HTTP `200` and contain `alipay_trade_query_response` plus `sign`. The service MUST verify `sign` over the exact raw response-node JSON bytes with the configured Alipay public key. Code `40004` with sub-code `ACQ.TRADE_NOT_EXIST` MUST map to `NotFound`. Code `10000` MUST match order number, seller ID, CNY amount, and one documented trade state before projection.

SB-A-6. WeChat Pay MUST support Native QR and H5 payment using API v3.

SB-A-6A. A WeChat Pay credential plaintext MUST be JSON with exactly `merchant_id`, `app_id`, `api_v3_key`, `merchant_certificate_serial`, `merchant_private_key_pem`, `platform_certificate_serial`, and `platform_public_key_pem`. Every field MUST be nonempty. `api_v3_key` MUST contain exactly 32 UTF-8 bytes. The merchant certificate fields MUST authorize outbound requests. The platform certificate fields MUST verify inbound callbacks.

SB-A-6B. WeChat Native checkout MUST POST to `/v3/pay/transactions/native`. WeChat H5 checkout MUST POST to `/v3/pay/transactions/h5`. The JSON body MUST contain the configured App ID and merchant ID, immutable order number, description, callback URL, positive integer CNY fen, and `CNY`. H5 checkout MUST also contain the canonical client IP and `scene_info.h5_info.type = Wap`.

SB-A-6C. WeChat authorization MUST use scheme `WECHATPAY2-SHA256-RSA2048`. It MUST sign the exact method, canonical path, timestamp, nonce, and JSON body with the merchant private key. It MUST include merchant ID and merchant certificate serial.

SB-A-7. WeChat callback verification MUST require `Wechatpay-Timestamp`, `Wechatpay-Nonce`, `Wechatpay-Serial`, and `Wechatpay-Signature`. `Wechatpay-Serial` MUST equal the credential platform certificate serial. The signed bytes MUST be the exact timestamp, LF, nonce, LF, raw body, and LF. The timestamp MUST differ from Store Primary time by at most 300 seconds.

SB-A-7A. A WeChat payment callback body MUST be JSON with one nonempty event `id`, `event_type = TRANSACTION.SUCCESS`, and one `resource`. The resource MUST use `original_type = transaction` and `algorithm = AEAD_AES_256_GCM`. The service MUST decrypt it with the 32-byte API v3 key, the exact 12-byte nonce, and the exact associated data.

SB-A-7B. A decrypted WeChat payment resource MUST contain the configured merchant ID, configured App ID, nonempty order number, nonempty transaction ID, `trade_state = SUCCESS`, positive integer amount in fen, and `currency = CNY`. The verified payment event ID MUST equal the outer event `id`.

SB-A-7C. A WeChat platform-verification credential version MAY differ from the payment attempt credential version after platform certificate rotation. The callback MUST bind the attempt by Channel, order number, adapter kind, and the exact merchant-account identity digest. The provider event `credential_version_id` MUST remain the attempt credential version. The allow-listed parsed event MUST record the platform-verification credential version ID.

SB-A-7C1. A WeChat merchant-account identity digest MUST equal the lowercase SHA-256 hexadecimal digest of canonical merchant-side credential bytes. The canonical bytes MUST contain `merchant_id`, `app_id`, `api_v3_key`, `merchant_certificate_serial`, and `merchant_private_key_pem` in that order. Each field MUST be encoded as its unsigned 64-bit big-endian byte length followed by its exact UTF-8 bytes. The canonical bytes MUST exclude `platform_certificate_serial` and `platform_public_key_pem`. A callback MAY use a different verification credential only when its merchant-account identity digest equals the attempt digest.

SB-A-7C2. WeChat callback selection MUST evaluate every stored credential that can verify and decrypt the callback. It MUST bind each verified credential by Channel, order number, adapter kind, and merchant-account identity. Multiple verification credentials that bind the same attempt MUST form one binding. The selected verification credential MUST be the first credential in active-first, creation-time-descending, ID-descending order that belongs to that binding. Zero bound attempts or more than one distinct bound attempt MUST use the unbound manual-review path.

SB-A-7D. An Alipay or WeChat callback MAY bind one attempt whose Provider object ID equals the immutable order number or is null. The Channel, order number, adapter identity, credential identity, and merchant identity rules for that adapter MUST still match. The callback MUST reject zero matches or more than one match. A verified payment projection MUST set a null Provider object ID to the immutable order number.

SB-A-8. WeChat Pay MUST support order query, refund notification, refund query, and bill download. Complaint automation MUST remain unavailable unless the merchant capability test proves it.

SB-A-8A. WeChat payment query MUST send a signed GET to `/v3/pay/transactions/out-trade-no/{order_number}?mchid={merchant_id}`. The path segment and query value MUST use percent encoding. The signed canonical URL MUST equal the transmitted path and query.

SB-A-8B. Every WeChat query response, including `ORDER_NOT_EXIST`, MUST pass platform certificate serial, timestamp, nonce, and RSA signature verification over the exact raw body. A successful response MUST match merchant ID, App ID, order number, CNY amount, and one documented trade state before projection.

SB-A-8C. A WeChat query request MUST use the attempt's historical merchant signing credential. Response verification MUST select a stored platform certificate by `Wechatpay-Serial` from credential versions with the same Channel, adapter kind, and merchant-account identity. Platform certificate rotation MUST NOT replace the historical merchant signing credential.

SB-A-8D. WeChat query state `REFUND` MUST map to `Ambiguous` until a verified refund event or refund query establishes the refund transition. It MUST NOT project payment success or start fulfillment.

SB-A-9. Stripe MUST use hosted Checkout. Monoize MUST NOT collect or store card numbers.

SB-A-10. Stripe MUST create a Checkout Session with the Store order number as idempotency key and metadata reference.

SB-A-10A. Stripe Checkout creation MUST send `mode=payment`, one line item, `client_reference_id=order_number`, `metadata[store_attempt_id]=attempt_id`, `success_url`, and `cancel_url`. It MUST send the order number as `Idempotency-Key`. It MUST authenticate with the credential `secret_key`. It MUST reject a non-HTTPS action URL.

SB-A-10B. A Stripe client-error response MUST be a definite rejection only when its JSON contains one nonempty `error.type` and one nonempty `error.message`. A redirect, server error, malformed body, or unrecognized body MUST be ambiguous.

SB-A-10C. Stripe payment query MUST send an authenticated GET to `/v1/checkout/sessions/{provider_object_id}` with the credential API version. A response with `error.type = invalid_request_error` and `error.code = resource_missing` at HTTP `404` MUST map to `NotFound`. A successful Checkout Session MUST match its ID, client reference, amount, currency, and payment state before projection.

SB-A-10D. A payment query contract MUST contain the exact Provider object ID, immutable merchant order number, integer minor amount, and currency. A missing or mismatched contract field MUST fail verification. It MUST NOT map to a Provider payment state.

SB-A-11. Stripe Webhook verification MUST check signature, configured API version, Checkout Session, PaymentIntent, amount, currency, merchant account, and payment state.

SB-A-11A. A Stripe payment success event MUST use type `checkout.session.completed` or `checkout.session.async_payment_succeeded`. Its `data.object` MUST be a Checkout Session with nonempty `id`, `payment_intent`, `client_reference_id`, and `metadata.store_attempt_id`. `amount_total` MUST be a positive integer. `currency` MUST be `cny` or `usd`. `payment_status` MUST be `paid`. The event `account` MUST equal the credential `account_id`.

SB-A-12. Stripe MUST classify Checkout completion, asynchronous success, asynchronous failure, expiration, refund, dispute, and chargeback as distinct events.

SB-A-13. Stripe amount minimums MUST come from a versioned capability table bound to account country, currency, and configured API version. An unknown combination MUST disable that currency.

SB-A-14. Logs MUST NOT contain provider signature headers, raw callback bodies, merchant credentials, or full provider responses.

SB-A-15. The implementation MUST NOT copy AGPL source, tests, comments, UI, or internal identifiers from `QuantumNous/new-api`. The release MUST record the external commit and a similarity scan.

## 6. Callback Events And Event Ordering

SB-EV-1. A callback endpoint MUST be public and MUST require successful adapter verification before state projection.

SB-EV-2. A callback body MUST be at most 131,072 bytes and MUST complete reading within five seconds.

SB-EV-2A. A verified Stripe callback MUST return HTTP `200` with `{ "received": true }` after an applied, duplicate, or manual-review projection. Invalid signatures and invalid verified fields MUST return HTTP `400`. Missing callback keys MUST return HTTP `503`. A storage or fulfillment error MUST return HTTP `500`.

SB-EV-2B. A verified Alipay callback MUST return HTTP `200` with UTF-8 text `success` after an applied or duplicate projection. A manual-review projection caused by an order, attempt, credential, merchant, amount, or currency mismatch MUST return a non-200 response. The request MUST use `application/x-www-form-urlencoded`. Each decoded field name MUST occur once. An invalid signature, `sign_type`, App ID, seller ID, order number, trade number, amount, or trade state MUST return a non-200 response. A non-200 response MUST NOT contain a credential or raw callback body.

SB-EV-2C. A verified WeChat callback MUST return HTTP `200` with JSON `{ "code": "SUCCESS", "message": "成功" }` after an applied or duplicate projection. A manual-review projection MUST return a non-200 response. The request MUST use `application/json`. An invalid header, signature, timestamp, certificate serial, resource, merchant ID, App ID, order number, transaction ID, amount, currency, or trade state MUST return a non-200 response. A non-200 response MUST NOT contain a credential, decrypted resource, or raw callback body.

SB-EV-3. A callback event MUST store credential version, provider event identity, body digest, allow-listed verified fields, verification result, encrypted raw body, projection state, and state revision.

SB-EV-4. `(credential_version_id, provider_event_id)` MUST be unique. One verified event MUST project to at most one order.

SB-EV-4A. A verified Alipay or WeChat payment callback MAY bind an attempt whose Provider object ID is absent only when exactly one attempt matches the callback Channel, adapter kind, immutable order number, and required credential or merchant-account identity. A candidate with an absent Provider object ID MUST be `created` or `failed` with `failure_kind = provider_rejected`. The payment projection transaction MUST repeat the candidate query after it locks the order row for every Alipay or WeChat payment callback, including a selected attempt with a nonempty Provider object ID. The selected attempt MUST be the only matching candidate in that transaction. The transaction MUST set the absent Provider object ID to the immutable order number. It MUST NOT change a nonempty Provider object ID or bind an attempt from another order.

SB-EV-4B. A verified Alipay or WeChat callback with zero or multiple matching attempts MUST insert one `store_provider_events` row with `projection_state = manual_review` before it returns the adapter-defined non-200 response. The row MUST use the verification credential version, encrypted raw body, body digest, and allow-listed parsed fields. A retry with the same `(credential_version_id, provider_event_id)` MUST NOT insert another event. This path MUST NOT insert `store_order_event_applications`.

SB-EV-4C. Callback projection input MUST carry the attempt credential version and the verification credential version as separate fields. An applied event MUST use the attempt credential version. A lock-time zero-candidate or multiple-candidate event MUST use the verification credential version. A synthetic Provider query MUST set both fields to the same attempt credential version and MUST NOT enter the unbound callback event path.

SB-EV-5. Projection state MUST be `pending`, `applied`, `superseded`, or `manual_review`.

SB-EV-6. A duplicate verified event with the same body digest and immutable parsed identity MUST return the provider success acknowledgement and MUST create no second ledger, recovery, fulfillment, or state transition. A duplicate `(credential_version_id, provider_event_id)` with a different body digest or immutable parsed identity MUST preserve the original event, return manual review, and idempotently create one open `provider_event_identity_conflict` reconciliation case. The case ID MUST be deterministic for the credential version and Provider event identity.

SB-EV-6A. The payment migration MUST create `idx_store_attempt_order_candidates` on `store_payment_attempts`. Its leading columns MUST be `(order_id, channel_id, adapter_kind, created_at DESC, id DESC)`.

SB-EV-7. An event received before prerequisite payment evidence MUST remain pending and trigger provider query. It MUST NOT be discarded because of arrival order.

SB-EV-8. Payment success on `unpaid` or version-2 `closed` MUST set paid. It MUST start fulfillment only when no refund success, lost dispute, or payment hold blocks it.

SB-EV-9. Payment failure or close on `unpaid` MUST query provider and close only after verified unpaid state. It MUST NOT downgrade paid or refunded state.

SB-EV-10. Refund success before payment evidence MUST query the original payment, persist payment evidence, then set refunded without fulfillment.

SB-EV-11. Dispute open before payment evidence MUST remain pending and query by provider object and merchant order. Confirmed payment MUST apply dispute open without fulfillment.

SB-EV-12. Dispute or chargeback during `refund_pending` MUST retain refund state, apply dispute state, share the existing recovery reserve, retain payment hold, and query both operations.

SB-EV-13. Refund success after chargeback MUST set refunded and MUST NOT create another economic recovery.

SB-EV-14. A contradictory terminal event MUST require a provider query and a newer provider object version when the provider exposes versions. Arrival time MUST NOT override terminal state.

SB-EV-15. A database write failure MUST return a non-success callback acknowledgement so the provider can retry.

## 7. Fulfillment, Refunds, Disputes, And Settlement

SB-RC-1. Payment projection and fulfillment MUST use separate transactions. A crash between them MUST leave `paid/pending` for reconciliation.

SB-RC-2. Balance fulfillment MUST append one idempotent `store_recharge` ledger credit and update the balance in one transaction.

SB-RC-3. Plan fulfillment MUST insert one entitlement generation and update the current pointer with an expected-generation predicate.

SB-RC-4. A duplicate callback or reconciliation run MUST NOT credit or activate twice.

SB-RC-5. Milestone 1 MUST support full refunds only.

SB-RC-6. A fulfilled plan order MUST NOT be refundable. A paid plan order MAY be refunded only while fulfillment is pending or failed.

SB-RC-7. A fulfilled balance refund MUST reserve the original credited nano USD before calling the provider. The reserve transaction MUST lock order, recovery, and user in that order.

SB-RC-8. Refund, dispute, and chargeback claims for one order MUST share one economic recovery row.

SB-RC-8A. An economic recovery state MUST be `open`, `reserved`, `recovered`, or `released`. A recovery claim state MUST be `open`, `resolved`, or `consumed`. A refund claim MUST be `open` while its refund is `created` or `pending`, `resolved` after definite rejection, and `consumed` after verified success.

SB-RC-8B. Starting the same refund idempotency key again MUST return the existing refund without another balance mutation. Starting a different refund while the order is not `paid` MUST fail before a Provider request.

SB-RC-9. Database constraints MUST enforce nonnegative values and `reserved + recovered <= original` across concurrent claims.

SB-RC-10. Recovery claim identity and recovery ledger keys MUST be unique. One original reward MUST be debited at most once.

SB-RC-10A. Reuse of `(credential_version_id, provider_claim_id, kind)` MUST return the existing claim only when it belongs to the same order. Reuse for another order MUST fail without state mutation.

SB-RC-11. A definite refund rejection MUST release a reserve once only when no unresolved dispute or chargeback claim remains.

SB-RC-12. An ambiguous refund MUST remain `refund_pending`. The reconciler MUST query it before any retry.

SB-RC-13. A verified dispute open MUST set payment hold, create or reuse the shared recovery claim, block pending fulfillment, and suspend an active plan sourced from that order.

SB-RC-13A. A Store plan entitlement MUST store nullable `suspended_at` and `suspension_reason`. Entitlement reads and plan admission MUST exclude a suspended entitlement. A verified dispute win MAY clear a `payment_dispute` suspension only while the original entitlement end time is in the future and no other claim remains open.

SB-RC-14. A verified dispute win MUST resolve that claim. It MAY clear hold only when all claims are resolved and user balance is nonnegative.

SB-RC-15. A verified dispute loss or chargeback MUST consume at most the remaining original reward, MAY make balance negative, revoke the affected plan, and retain payment hold.

SB-RC-16. Payment hold MUST block Store order creation, redemption before lookup, and plan admission. It MUST NOT block login, Store reads, callbacks, refunds, reconciliation, or eligible ordinary-balance API use.

SB-RC-17. An already presented payment MUST remain queryable during hold. Verified payment MUST be recorded, but fulfillment MUST remain pending.

SB-RC-18. Each daily settlement line MUST have a provider-unique identity and class `gross`, `refund`, `dispute`, `fee`, `tax`, `currency_conversion`, or `net`.

SB-RC-18A. `store_settlement_reports` MUST store Channel, credential version, Provider report identity, report date, body digest, and import time. `(credential_version_id, provider_report_id)` MUST be unique.

SB-RC-18B. `store_settlement_lines` MUST store report, credential version, Provider line identity, class, signed integer minor amount, currency, optional Provider transaction identity, optional matched order, and creation time. `(credential_version_id, provider_line_id)` MUST be unique.

SB-RC-19. Settlement import MUST be idempotent. Fees, taxes, and settlement FX MUST NOT change user rewards.

SB-RC-20. An unmatched payment, refund, dispute, or unknown settlement difference MUST create a critical manual case.

## 8. Redemption Codes And Reauthentication

SB-R-1. New codes MUST use Crockford Base32 alphabet `0123456789ABCDEFGHJKMNPQRSTVWXYZ` and 16 random characters grouped as `XXXX-XXXX-XXXX-XXXX`.

SB-R-2. The database MUST store code format, SHA-256 lookup digest, final four-character hint, encrypted full code, reward snapshot, expiry, state, creator, and redemption data.

SB-R-2A. A v2 unused unexpired code MUST store `code_format_version = 2` and one complete encrypted value bound to AAD `store_redemption_codes:{code_id}:code`. A v1 row MUST have `code_format_version = 1` and no encrypted value. An encrypted value MUST store format version, key ID, nonce, and ciphertext as separate columns. Destruction MUST set every encrypted field to null and set `ciphertext_destroyed_at` in the same transaction.

SB-R-3. A generation response MUST return every full generated code once. The dialog MUST keep them visible until the Admin closes it.

SB-R-4. An unused v2 code MAY be revealed or exported only with a reauthentication grant scoped to redemption access and expiring after five minutes.

SB-R-5. The server MUST store only a hash of a reauthentication token. Logout, password change, session rotation, session expiry, role removal, or account disablement MUST invalidate the grant.

SB-R-6. A reveal response MUST contain at most 20 codes. An export MUST contain at most 100 codes.

SB-R-6A. Reauthentication scope `redemption_access` MUST authorize reveal, copy, and export. `POST /api/dashboard/store/admin/redemption-codes/reveal` MUST accept one to 20 code IDs and action `reveal` or `copy`. `POST /api/dashboard/store/admin/redemption-codes/export` MUST accept one to 100 code IDs and return CSV. `POST /api/dashboard/store/admin/redemption-codes/{id}/revoke` MUST revoke one unused code.

SB-R-7. Reveal and export responses MUST set `Cache-Control: no-store`, `Pragma: no-cache`, `Referrer-Policy: no-referrer`, and `X-Content-Type-Options: nosniff`.

SB-R-8. Reveal, copy, and export MUST write an audit containing Admin, action, code IDs, count, IP, User-Agent, and time. It MUST NOT contain full codes.

SB-R-9. Successful redemption and revocation MUST delete encrypted full code in the same transaction. Cleanup MUST delete encrypted full code within 24 hours after expiry.

SB-R-10. Legacy v1 codes MUST remain redeemable through their existing digest and alphabet. They MUST NOT become recoverable.

SB-R-11. Redemption normalization MUST uppercase ASCII letters and remove ASCII hyphens. It MUST NOT map look-alike characters.

SB-R-12. Invalid syntax and no match MUST both return HTTP `404` with code `invalid_redemption_code`.

SB-R-13. Redemption MUST lock or serialize the code row and apply the reward and used state in one transaction. It MUST NOT create a payment order or require a payment Channel.

SB-R-14. Redemption MUST allow at most ten attempts per minute per user and source IP. Five failed attempts in 15 minutes MUST create a 30-minute account-and-IP cooldown.

SB-R-14A. Redemption limits MUST use a persistent lock row keyed by user and SHA-256 source-IP digest plus persistent attempt rows. The service MUST lock the limit row before it counts attempts. A rate-limited or cooldown request MUST perform no code lookup or mutation.

## 9. Plan Quota Admission

SB-Q-1. A quota bucket MUST store settled and reserved CNY fen. A request reservation MUST store unique request ID, entitlement generation, maximum charge, state, and timestamps.

SB-Q-2. Admission MUST lock the entitlement and applicable buckets in deterministic order. It MUST require `settled + reserved + maximum <= quota` for every applicable rule.

SB-Q-3. Admission MUST fail with HTTP `402` and code `plan_quota_exhausted` when any rule fails.

SB-Q-4. Settlement MUST subtract the reserved maximum and add exact actual CNY fen in one transaction. Release MUST subtract the reserved maximum and add no settled amount.

SB-Q-5. A provider charge above the reservation MUST apply to the old generation, block later plan admission, and create a critical quota violation. It MUST NOT charge a replacement generation.

SB-Q-6. SQLite admission MUST use one Store Primary, WAL, foreign keys, a five-second busy timeout, and short `BEGIN IMMEDIATE` transactions. Network calls and price calculation MUST occur outside the transaction.

SB-Q-7. SQLite plan features MUST require a persisted compatibility fingerprint and a passing quota gate for the exact application version, SQLite version, journal mode, busy timeout, filesystem identity, and quota manifest.

SB-Q-8. A `pending` or `failed` SQLite gate MUST block plan product enablement, catalog availability, order creation, plan code generation, fulfillment, and admission. Balance products MUST remain available when other Store gates pass.

SB-Q-9. A Replica MUST obtain a Primary-signed Ed25519 admission token before routing a plan-funded request. Primary unavailability MUST fail plan admission closed.

SB-Q-10. A token MUST bind version, key ID, issuer, node audience, entitlement, generation, reservation, request ID, maximum charge, issued time, and expiry.

SB-Q-11. Token TTL MUST be 30 seconds. Clock skew MUST be at most two minutes. Key rotation MUST publish a new key before activation and retain a prior key until all issued tokens expire plus skew.

SB-Q-12. A Replica MUST fsync a durable claim marker before routing. Same-node replay MUST fail by marker. Cross-node replay MUST fail by audience.

SB-Q-13. A Replica MUST spool settlement or release before reporting terminal billing success. Primary application MUST be idempotent.

## 10. Reconciliation, Manual Cases, And Operations

SB-OP-0. The application MUST keep the periodic reconciliation scheduler disabled until it implements every scan class in SB-OP-3 and the corresponding verified Provider query. An isolated fulfillment-recovery run MAY execute in tests before this gate opens.

SB-OP-1. Only the Store Primary MUST run reconciliation. The reconciler MUST acquire the `store_reconciler` row in `store_reconciliation_leases`. A lease MUST contain an opaque owner ID, a strictly increasing fencing epoch, and an expiry 90 seconds after acquisition. A second owner MUST NOT process work before expiry. Every reconciled fulfillment transaction MUST lock and validate the exact owner and fencing epoch before it changes financial state.

SB-OP-1A. Each fenced transaction MUST compare lease expiry with the run start time plus the elapsed monotonic run duration. It MUST NOT reuse the unadjusted run start time after a Provider call. A run that reaches lease expiry without renewal MUST stop before another state change.

SB-OP-2. After SB-OP-0 opens the scheduler gate, reconciliation MUST run once per minute. One fulfillment-recovery run MUST select at most 100 candidates ordered by `paid_at ASC, id ASC`.

SB-OP-3. Reconciliation MUST scan expired presented attempts, paid/pending orders older than 30 seconds, paid/failed orders, refund-pending orders, and retryable provider events.

SB-OP-3A. An expired presented-attempt scan MUST select at most 100 attempts with `provider_expires_at <= now`, ordered by `provider_expires_at ASC, id ASC`. It MUST query with the attempt's exact historical credential version and immutable order contract.

SB-OP-3AA. The reconciler MUST also query `created` and `failed/provider_rejected` Alipay or WeChat attempts at least 30 seconds after their last update. These adapters MUST use the immutable merchant order number when no Provider object ID was persisted. Stripe attempts without a Checkout Session ID MUST use SB-O-10B and MUST NOT fabricate a query result.

SB-OP-3B. A verified `Paid` query result MUST enter the same idempotent payment projection as a verified callback. The projection and any resulting fulfillment MUST validate the active reconciliation owner and fencing epoch before financial state changes.

SB-OP-3C. A verified `NotFound`, `Unpaid`, or `Closed` result for an expired presented attempt MUST change that attempt from `presented` to `expired` and its order from `unpaid` to `closed` in one fenced transaction. A verified `Ambiguous` result, a verification failure, and a transport failure MUST leave both states unchanged. A contract-version 2 order that later receives verified payment MAY transition from `closed` to `paid` under SB-O-16.

SB-OP-3D. A payment projected from a verified query MUST create one deterministic `payment_query_succeeded` provider-event identity per credential version, attempt, and Provider transaction. A repeated query MUST reuse that projection identity and MUST NOT fulfill twice.

SB-OP-3E. A verified `NotFound` or `Closed` result for a `created` or `failed/provider_rejected` attempt MUST change that attempt to `expired`, clear `failure_kind`, and leave the order `unpaid`. This transition permits one new attempt. A verified `Unpaid` result MUST keep the attempt blocked because a Provider object may still accept payment.

SB-OP-3F. A query verification error, credential error, unsupported recovery path, `Ambiguous` result, or blocked `Unpaid` result MUST upsert one open reconciliation case with deterministic ID `payment-query:{attempt_id}`. The case MUST store only an error category and non-secret attempt evidence. A definite terminal result MUST close the case.

SB-OP-3G. An attempt with an open payment-query reconciliation case MUST NOT be queried again until at least 60 seconds after the case `updated_at` time.

SB-OP-4. A paid order without a retry row becomes eligible 30 seconds after `paid_at`. After the first failed reconciliation, the next delay MUST be two minutes. After the second failure, the next delay MUST be ten minutes. After every later failure, the next delay MUST be one hour.

SB-OP-4A. `store_fulfillment_retries` MUST contain one row per order. It MUST store `attempt_count`, `next_attempt_at`, `last_error_category`, and `updated_at`. A successful fulfillment MUST delete this row in the same fenced transaction. A failed fulfillment MUST upsert the next delay without changing the order payment state or user reward.

SB-OP-4B. PostgreSQL MUST store Store order revisions, provider-event revisions, lease epochs, and fulfillment retry counts as signed 64-bit integers. SQLite MUST accept the same signed 64-bit range.

SB-OP-5. Refund query delays MUST be one minute, five minutes, 15 minutes, then hourly. A refund pending longer than 15 minutes MUST alert.

SB-OP-6. Production MUST assign one primary Payment Operations Owner, one distinct backup owner, and one Finance Approver.

SB-OP-7. Critical cases MUST be acknowledged within 15 minutes. Exposure growth MUST be contained within 30 minutes. Provider evidence MUST be recorded within four hours.

SB-OP-8. Paid-but-unfulfilled and refund-pending cases MUST be acknowledged within 30 minutes.

SB-OP-9. Case closure, hold clearance, recovery adjustment, and unexplained settlement acceptance MUST require owner reauthentication and approval by a distinct Finance Approver.

SB-OP-10. A case and its audit trail MUST NOT be deleted through Admin.

## 11. Primary Availability

SB-HA-1. Production MUST declare `postgresql_primary` or `sqlite_primary`. Startup MUST reject a profile that does not match the database backend.

SB-HA-2. `postgresql_primary` MUST detect Primary loss within 30 seconds, have RTO at most five minutes, and have committed-state RPO zero.

SB-HA-3. PostgreSQL promotion MUST fence the old process and database writer before the replacement mounts Store endpoints.

SB-HA-4. A Store process MUST hold an exclusive database lease with a monotonic epoch. Lease interval MUST be at most 15 seconds. Lease loss MUST fail Store writes and admission closed within one interval.

SB-HA-5. `sqlite_primary` MUST have process-restart RTO at most two minutes on a healthy host, host-loss RTO at most 30 minutes, and RPO at most 60 seconds.

SB-HA-6. Production SQLite MUST use encrypted off-host replication or backup with measured lag at most 60 seconds. An application-host-only backup MUST fail readiness.

SB-HA-7. During Primary outage, checkout, polling mutations, refund mutations, redemption, and plan admission MUST fail closed.

SB-HA-8. Recovery MUST reconcile every ambiguous attempt from the recovery boundary before checkout resumes.

SB-HA-9. Production enablement MUST require a current failover drill that proves fencing, provider retry recovery, declared RTO/RPO, and zero dual-Primary writes.

## 12. Privacy, Retention, And Access

SB-PR-1. Production checkout and callbacks MUST require a current accepted, versioned Store privacy record. Review interval MUST NOT exceed 365 days.

SB-PR-2. The privacy record MUST define jurisdiction, allowed storage regions, legal basis, retention values, reviewer, approval evidence digest, approval time, and next review time.

SB-PR-3. Encrypted raw callback bodies MUST be retained for 30 days.

SB-PR-4. Source IP and User-Agent MUST be retained for 90 days and then deleted. A retained pseudonym MUST be keyed, non-reversible, purpose-bound, and use a separate rotating key.

SB-PR-5. Parsed provider events, orders, ledger entries, refunds, disputes, chargebacks, settlement reports, recovery claims, and financial audits MUST be retained for seven years unless the accepted jurisdiction record defines another period.

SB-PR-6. Redemption reveal, copy, and export audits MUST be retained for two years.

SB-PR-7. Reauthentication grant hashes MUST be deleted no later than 24 hours after expiry.

SB-PR-8. Raw callback bodies MUST be readable only by assigned Payment Operations during an incident and authorized Privacy or Security reviewers.

SB-PR-9. Every evidence read, export, retention override, and legal-hold operation MUST write an immutable access audit with actor, role, scope, reason, time, and result.

SB-PR-10. Payment data, callback evidence, backups, logs, and encryption keys MUST remain inside privacy-record allowed regions.

SB-PR-11. A daily Primary job MUST delete expired data in bounded idempotent batches and record counts, oldest remaining time, failures, and policy version.

SB-PR-12. Three consecutive deletion failures MUST create a critical alert and pause new checkout.

SB-PR-13. A legal hold MUST define reason, scoped data identifiers, requesting authority, distinct approver, start time, and mandatory expiry. Expiry MUST stop the hold automatically.

SB-PR-14. A legal hold MUST NOT restore already deleted data. An extension MUST create a new approval record.

## 13. Security And API Surface

SB-S-1. Callback rate limiting MUST allow at most 600 requests per minute per Channel and source IP. Signature verification remains mandatory.

SB-S-2. Every cookie-authenticated Store mutation MUST require JSON or the documented multipart icon type and an `Origin` equal to the configured public origin.

SB-S-3. Reveal, export, refund, reprocess, exchange-rate recovery, key rotation, credential update, hold clearance, and financial case closure MUST require a five-minute scoped reauthentication grant.

SB-S-4. Sensitive responses MUST set `Cache-Control: no-store`. Logs and API responses MUST redact secrets, signatures, full codes, and reauthentication tokens.

SB-S-5. Session user endpoints MUST include:

- `GET /api/dashboard/store/catalog`
- `GET /api/dashboard/store/exchange-rate`
- `GET /api/dashboard/store/entitlement`
- `GET /api/dashboard/store/orders`
- `POST /api/dashboard/store/orders`
- `GET /api/dashboard/store/orders/{id}`
- `POST /api/dashboard/store/orders/{id}/attempts`
- `POST /api/dashboard/store/redeem`
- `GET /api/dashboard/store/icons/{id}`

SB-S-6. Public provider endpoints MUST include `POST /api/store/callbacks/{channel_id}`. They MUST NOT require a dashboard session.

SB-S-7. Admin endpoints MUST include product, settings, Channel, credential, compliance, capability, order query, event reprocess, unpaid close, refund, redemption generation, reveal, export, revocation, reconciliation case, rate governance, privacy, retention, and readiness operations under `/api/dashboard/store/admin/`.

SB-S-8. `POST /api/dashboard/store/admin/orders/{id}/complete` MUST NOT exist.

SB-S-9. User and Admin mutations MUST enforce the repository Primary write policy. Replica Store routes MUST return the repository write rejection.

## 14. Frontend

SB-UI-1. `/dashboard/store` MUST contain Balance, Plan, and Redemption modes in one sliding segmented control.

SB-UI-2. Balance and Plan MUST use a two-column purchase area. Payment methods MUST occupy one independent full-width row at the bottom of the left column.

SB-UI-3. Redemption MUST render no payment method and no order summary.

SB-UI-4. The order summary MUST keep stable dimensions between Balance and Plan.

SB-UI-5. CNY and USD selection MUST be one shared state for balance, usage, product, bonus, actual received, plan price, plan quota, and order summary.

SB-UI-6. Simplified Chinese MUST use `实得` for recharge plus bonus.

SB-UI-7. Store and Store Management reads MUST use SWR and initial skeletons. User mutations MUST use optimistic cache updates with rollback when the mutation can be reversed locally.

SB-UI-8. Payment creation MUST NOT optimistically credit balance or activate a plan.

SB-UI-9. An unpaid order MUST poll every two seconds until payment state is `paid`, `refunded`, or `closed`, fulfillment state is `fulfilled` or `failed`, expiration, or unmount. Completion MUST revalidate order, balance, user, and entitlement data.

SB-UI-9A. The browser MUST persist one pending checkout fingerprint, order idempotency key, attempt idempotency key, and optional order ID in tab-scoped session storage. A retry with the same user and canonical purchase input MUST reuse these values. Only `payment_configuration_unavailable` and `payment_provider_rejected` MAY rotate the attempt key after the order ID is known. Network failures, `internal_error`, `payment_provider_ambiguous`, and unrecognized errors MUST retain both keys.

SB-UI-10. Alipay and Stripe MUST use provider redirects. WeChat desktop MUST use a QR modal. WeChat mobile MUST use H5 redirect.

SB-UI-10A. The WeChat Native QR modal MUST render the exact verified Provider payload as a scannable SVG QR code. It MUST NOT display the payload as plain text instead of a QR code.

SB-UI-11. Store Management MUST have Products, Payment Channels, Orders, and Redemption Codes child pages with an animated active indicator.

SB-UI-12. Orders MUST show payment and fulfillment state separately. It MUST expose query, verified event reprocess, close, refund, dispute, hold, and case actions according to role and state. It MUST NOT show manual Complete.

SB-UI-13. Generated redemption codes MUST remain fully visible in the generation result. List rows MUST remain masked until scoped reveal.

SB-UI-14. Main Store cards MUST use a 16-pixel radius. Interactive controls SHOULD use a 12-pixel radius. Product lists MUST expand naturally.

SB-UI-15. Store Management MUST edit Channel metadata and Channel credentials with separate save actions. The credential form MUST render only for an existing official Channel. It MUST start empty whenever the dialog opens. It MUST NOT render a saved credential value. Credential save MUST request the current Admin password, obtain a `credential_update` reauthentication grant, replace the credential, optimistically mark the Channel disabled, and revalidate the Channel list.

## 15. Migration And Verification

SB-M-1. A read-only preflight MUST report legacy order states, timestamp consistency, orphan references, duplicate order numbers, unknown callback objects, partial payment fields, and schema collisions.

SB-M-2. Preflight MUST abort on unknown order state, inconsistent timestamp, orphan, unknown callback table, partial provider data, or credential format outside the documented legacy contract.

SB-M-3. Legacy `completed` orders MUST migrate to payment `paid`, fulfillment `fulfilled`, contract version 1.

SB-M-4. Legacy `pending` and `cancelled` orders MUST migrate to payment `closed`, fulfillment `pending`, contract version 1.

SB-M-5. New callbacks, query, refund, and late payment MUST reject version-1 closed orders.

SB-M-6. Legacy Channel secrets MUST become retired encrypted credential versions and MUST NOT be used by an official adapter.

SB-M-7. Migration MUST disable every Channel and create a count manifest. Any preflight or count mismatch MUST roll back the transaction.

SB-M-8. SQLite and PostgreSQL MUST implement equivalent state, recovery, immutability, and uniqueness constraints.

SB-M-9. Local automated verification MUST use SQLite and MUST NOT start PostgreSQL on the user's computer.

SB-M-10. Production Channel enablement MUST require passing migration, callback, adapter, event-order, recovery, reconciliation, quota, key rotation, capability, failover, privacy, retention, backup, and restore gates from the design.

SB-M-11. Code deployment and production Channel enablement are separate approvals. A deployed Channel MUST remain disabled until its provider-specific controlled payment and refund pass.

SB-M-12. Before the first real provider payment, rollback MAY restore the old image and matching database backup. After a real payment or refund, database rollback is forbidden; operations MUST stop checkout and reconcile or forward-fix every provider transaction.
