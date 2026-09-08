# Enterprise Accounts, API Key Analytics, And EPay Design

## Scope

This change adds an Enterprise account class, per-API-Key analytics, and one EPay adapter. Enterprise and standard users share login, wallet, Store products, plans, orders, redemption, logs, usage accounting, and API Key management.

The EPay adapter replaces the existing Alipay and WeChat adapter kinds. Stripe and the custom HTTP adapter remain.

## Enterprise Account Class

Each user has one account class: standard or enterprise. The account class is independent from the user, Admin, and super-Admin authorization roles. Existing users migrate to standard.

Each Group has one account class. Existing Groups migrate to standard. Provider, Channel, model offer, and price records inherit the account class of their Group. A write that links resources from different account classes fails.

Group public visibility and private user grants continue to apply inside one account class. A user can access a Group only when the Group account class matches the user account class and the Group is public or explicitly granted to the user.

Marketplace reads, model resolution, Channel selection, request routing, price estimation, and final billing use resources from the authenticated user account class only. They never fall back to the other account class. Enterprise prices are independent stored prices. They are not discounts or display conversions of standard prices.

An Admin can change a user account class after a secondary confirmation that names the destructive effect. One database transaction locks the user, changes the account class, deletes all current API Keys, stores non-secret audit metadata for those Keys, and writes an account-class audit event. Any failure rolls back the transaction.

The transition preserves wallet balance, plan entitlement, orders, redemption history, request logs, and historical usage. It invalidates the user authorization, catalog, routing, and pricing caches. Every deleted Key fails authentication immediately and cannot be restored.

The Admin Group, Provider, Channel, model, and pricing pages contain a Standard or Enterprise scope control. Lists and selectors do not mix scopes.

The Enterprise Dashboard uses the existing design system. It contains wallet summary, API Keys, usage, logs, Enterprise Marketplace, and API documentation. It omits nonessential standard-user overview modules.

## API Key Analytics

The API Key list remains a compact overview. Selecting a Key opens a modal without navigation.

The modal supports the last 24 hours, last 7 days, last 30 days, and all retained history. It returns total Tokens, input Tokens, cache-read Tokens, output Tokens, request count, consumed Coin, the current independent Key balance, a trend, and a model breakdown.

The model breakdown returns model name, Tokens, request count, and consumed Coin. Rows sort by total Tokens descending and model name ascending.

Consumed Coin is the sum of immutable recorded request charges. Historical charges are not recomputed from current prices. A Key without an independent limit displays Follows wallet balance. Disabled and expired Keys retain readable history.

The 24-hour range uses hourly buckets. The 7-day and 30-day ranges use daily buckets. All history uses monthly buckets when the retained span exceeds 90 days and daily buckets otherwise.

User endpoints require Key ownership. Admin endpoints can inspect a Key for a selected user. No response contains recoverable Key material. Deleted Keys retain only a display-name snapshot and a non-reversible identifier in historical records.

The frontend uses SWR, an initial skeleton, stable modal dimensions, and optimistic range selection. Existing values remain visible until the next result arrives. Numeric animation starts at the previous displayed value.

## EPay Channel

Payment adapter kinds become epay, stripe, and http. One EPay Channel stores one gateway base URL, merchant ID, encrypted merchant secret, callback configuration, and independent Alipay and WeChat method switches.

The Admin can configure each enabled method public label, icon, sort order, and enabled state. EPay accepts CNY orders only. USD checkout uses Stripe. EPay does not convert a USD Store order into CNY.

Payment creation uses POST /mapi.php with application/x-www-form-urlencoded. The request uses the documented fields: pid, type, out_trade_no, notify_url, return_url, name, money, clientip, device, param, sign, and sign_type. The type is alipay or wxpay. Money contains exactly two decimal places derived from integer fen.

When EPay returns a QR payload, the Store displays it in the payment modal. When EPay returns a payment URL, the Store displays an action that navigates after user activation. A response without either value fails payment creation and grants no balance or entitlement.

For signing and verification, the service removes sign, sign_type, and empty values. It sorts the remaining names by ascending ASCII order. It joins unencoded key=value pairs with ampersands, appends the merchant secret without another separator, computes MD5, and compares lowercase hexadecimal output in constant time.

MD5 is limited to this protocol. Merchant secrets are encrypted at rest and never returned by APIs or written to logs.

The callback accepts the documented GET request. It applies success only when trade_status equals TRADE_SUCCESS. Before transition, it verifies signature, merchant ID, local order number, provider transaction identity, exact CNY fen amount, and expected method.

Callback, active query, and reconciliation use one payment state-transition service and one idempotent fulfillment ledger. Repeated, concurrent, and reordered evidence fulfills an order at most once. The callback returns plain text success only after verified evidence and the required transition commit durably.

The adapter supports merchant validation, order query, full refund, refund query when available, settlement retrieval when available, and available-method discovery. After a timeout, disconnect, or HTTP 5xx, the service queries the original provider order before retrying creation or refund.

The reconciler scans ambiguous attempts, paid orders with pending fulfillment, and pending refunds.

The gateway base URL is configurable. The service accepts HTTP or HTTPS only for explicit gateway deployments. It rejects credential-bearing URLs, loopback, link-local, and private addresses. It rejects cross-host redirects and validates DNS results before each connection.

Migration removes active Alipay and WeChat adapter kinds. Legacy configuration produces one disabled EPay draft and does not create an enabled credential. The user catalog shows no EPay method until credentials, callback settings, method switches, capability checks, and Admin enablement are valid.

## Errors And Security

An account-class mismatch returns the same unavailable-resource result as an absent resource. It does not reveal the other catalog.

Invalid EPay signatures, merchant mismatches, amount mismatches, method mismatches, and malformed callbacks do not change financial state. Local orders with conflicting verified identifiers enter manual review.

Payment amounts use integer fen. Floating-point money calculations are forbidden. Logs store allow-listed identifiers and body digests instead of secrets or complete signed payloads.

## Verification

SQLite and PostgreSQL migration tests prove that existing users and Groups become standard and that migration creates no Enterprise routing resources.

Isolation tests cover Marketplace, routing, Channel selection, price estimation, final charges, public Groups, private grants, Admin scope filters, and cache invalidation.

Account-class transition tests prove atomic Key deletion, historical-data preservation, audit creation, rollback, and immediate rejection of former Keys.

API Key analytics tests compare every total and bucket with request records for all four ranges. They cover independent balance, wallet-following balance, disabled Keys, deleted-Key history, empty results, ownership rejection, and deterministic ordering.

EPay tests use fixed signature and form vectors. They cover both methods, CNY-only checkout, QR and URL responses, malformed responses, duplicate callbacks, callback-query races, invalid signatures, merchant, amount, and method mismatches, ambiguous transport failures, queries, refunds, and reconciliation.

Outbound tests cover loopback, private networks, link-local networks, DNS rebinding, cross-host redirects, timeouts, and response limits.

Frontend tests cover Enterprise navigation, Admin scope controls, destructive transition confirmation, API Key analytics, skeletons, range changes, EPay method switches, QR display, and Stripe-only USD checkout.

Release checks include Rust tests, frontend tests, type checks, builds, affected four-locale documentation, English and Chinese screenshots for documented UI flows, and git diff --check.

## Deployment

The production migration first runs against a restored SQLite backup. It must preserve financial totals and expected row counts. The release deploys with EPay disabled. Real credentials are entered later through the protected Admin flow.

Shared Caddy remains active. A proxy change requires caddy validate followed by systemctl reload caddy. Deployment never stops, restarts, or kills Caddy.

Before and after deployment, checks cover https://lynshen.org, https://sub.joinreso.com/health, the Monoize health endpoint, and Caddy active state.
