# Store Billing Specification

## 0. Scope And Terms

SB-0.1. The Store MUST support balance products, plan products, payment channels, purchase orders, and redemption codes.

SB-0.2. `CNY` and `USD` are the only Store currencies.

SB-0.3. Monetary database fields MUST be canonical base-10 integer strings in the smallest unit named by the field. CNY minor values are fen. USD account values are nano USD. A canonical integer string matches `0|[1-9][0-9]*`.

SB-0.4. Decimal conversion MUST use checked integer or decimal arithmetic. Store code MUST NOT convert money through `f32` or `f64`.

SB-0.5. The account ledger remains `users.balance_nano_usd`. The Store MUST NOT create a separately spendable CNY balance.

## 1. Exchange Rate

SB-FX-1. A primary node MUST request `https://open.er-api.com/v6/latest/USD` at process start and at most once per 15-minute interval after the previous attempt.

SB-FX-1A. A replica node MUST load only the persisted exchange-rate snapshot. A replica MUST NOT request the remote exchange-rate service or write `store_exchange_rates`.

SB-FX-2. A valid response MUST contain a finite positive `rates.CNY` decimal and a source update time. The backend MUST persist the decimal as `cny_per_usd` without binary floating conversion.

SB-FX-3. A failed or invalid refresh MUST retain the last successful rate. If no successful rate exists, endpoints that require a CNY/USD conversion MUST return HTTP `503` with code `exchange_rate_unavailable`.

SB-FX-4. `GET /api/dashboard/store/exchange-rate` MUST require a dashboard session and return `base: "USD"`, `quote: "CNY"`, `cny_per_usd`, `source_updated_at`, and `refreshed_at`.

SB-FX-5. Conversion from a source minor amount to a target minor amount MUST use the current rate and round half away from zero to the target minor unit. Plan quota presentation MUST round half away from zero to a whole display currency unit.

## 2. Store Products

### 2.1 Common Product Fields

SB-P-1. `store_products` MUST contain: `id`, `kind`, `name`, `description`, `price_currency`, `price_minor`, `duration_seconds`, `group_ids`, `sort_order`, `enabled`, `created_at`, and `updated_at`.

SB-P-2. `kind` MUST be `balance` or `plan`. `price_currency` MUST be `CNY` or `USD`. `price_minor` MUST be a canonical integer string greater than zero. `name` after trimming MUST contain 1 to 100 characters. `description` after trimming MUST contain at most 500 characters.

SB-P-3. `group_ids` MUST follow `groups-registry.spec.md` GR-C1 through GR-C3. Product writes MUST trim each id, remove empty ids, and remove later duplicates while preserving first-occurrence order. The canonical list MUST contain at most 32 ids. Every canonical id MUST exist in `monoize_groups` in the same write transaction that persists the product. A balance product MUST store `[]`. A plan product MAY store `[]`, which means no additional Group restriction.

SB-P-4. A balance product MUST have `duration_seconds = NULL`. A plan product MUST have `duration_seconds` between `3600` and `31_536_000` inclusive.

SB-P-5. Public Store reads MUST return only enabled products ordered by `sort_order ASC`, `created_at ASC`, then `id ASC`. Admin reads MUST return enabled and disabled products in the same order. The public catalog MUST include `settings` with the four custom recharge bound fields from SB-P-8 and MUST NOT include any other system setting.

### 2.2 Balance Products

SB-P-6. `store_balance_products` MUST contain one row for each balance product: `product_id`, `recharge_minor`, and `bonus_minor`. Both amount fields MUST use the product `price_currency` and MUST be canonical non-negative integer strings. `recharge_minor` MUST equal `store_products.price_minor`.

SB-P-7. Actual received MUST equal `recharge_minor + bonus_minor` with checked integer addition. Clients MUST NOT submit actual received as a writable field.

SB-P-8. A custom recharge request MUST use an admin-configured minimum and maximum in the selected payment currency. Its bonus is zero. `StoreSettings` MUST contain canonical integer strings named `custom_recharge_cny_min_minor`, `custom_recharge_cny_max_minor`, `custom_recharge_usd_min_minor`, and `custom_recharge_usd_max_minor`. The values MUST use `system_settings` keys with the same field name prefixed by `store.`. Each missing key MUST read as `1000` for a minimum or `100000000` for a maximum. A settings write MUST reject a zero minimum, a zero maximum, or a minimum greater than its same-currency maximum.

### 2.3 Plan Products And Quotas

SB-P-9. `store_plan_quotas` MUST contain: `id`, `product_id`, `window_kind`, `window_seconds`, `quota_fen_cny`, and `sort_order`.

SB-P-10. `window_kind` MUST be `5h`, `12h`, `day`, `week`, `month`, or `custom`. Fixed kinds MUST use window seconds `18000`, `43200`, `86400`, `604800`, or `2592000`. A custom rule MUST use a whole-hour duration from 1 through 8760 hours.

SB-P-11. `quota_fen_cny` MUST be a canonical integer string greater than zero. A plan MUST contain at least one quota rule. Two rules in one plan MUST NOT have the same `window_seconds`.

SB-P-12. Plan quotas use CNY as the stored base. User display in CNY MUST show `round_half_away_from_zero(quota_fen_cny / 100)`. USD display MUST first convert with the current exchange rate, then round half away from zero to a whole USD amount. Plan quota display MUST contain no decimal separator.

SB-P-13. Admin product reads MUST include enabled and disabled products. Deleting a missing product MUST return `not_found`. Deleting a product referenced by an order MUST return `conflict` and MUST NOT delete it.

## 3. Payment Channels

SB-C-1. `store_payment_channels` MUST contain: `id`, `kind`, `name`, `mode`, `endpoint`, `icon_kind`, `icon_value`, `config_secret`, `sort_order`, `enabled`, `created_at`, and `updated_at`.

SB-C-2. `kind` MUST be `alipay`, `wechat`, or `custom`. `mode` MUST be `redirect`, `qr`, or `manual`. `name` after trimming MUST contain 1 to 80 characters.

SB-C-3. The first migration MUST seed one disabled Alipay channel and one disabled WeChat channel. Both seeded rows MAY be edited or enabled. Neither row MAY be hard-coded as available.

SB-C-4. `icon_kind` MUST be `builtin`, `url`, or `upload`. A URL icon MUST use HTTPS. An uploaded icon MUST be PNG, JPEG, WebP, or SVG and at most 2 MiB. An `upload` channel write MUST use an `icon_value` that starts with `/api/dashboard/store/icons/`. The response MUST expose this same-origin path, not raw uploaded bytes.

SB-C-5. A Store read MUST return enabled channels only. An admin read MUST return all channels. Neither response MUST contain `config_secret`.

SB-C-6. Order creation MUST count enabled payment channels before validating the selected channel. If the count is zero, it MUST return HTTP `409` with code `no_payment_channel`. If the count is nonzero and the selected channel is missing or disabled, it MUST return HTTP `400` with code `invalid_payment_channel`.

SB-C-7. Admin payment channel reads MUST include enabled and disabled channels and MUST omit `config_secret`. Deleting a missing channel MUST return `not_found`. Deleting a channel referenced by an order MUST return `conflict` and MUST NOT delete it.

## 4. Orders And Fulfillment

SB-O-1. `store_orders` MUST contain: `id`, `order_number`, `user_id`, `product_id`, `product_kind`, `status`, `payment_channel_id`, `payment_currency`, `payment_minor`, `cny_per_usd`, `rate_source_updated_at`, `quote_json`, `created_at`, `updated_at`, `completed_at`, and `cancelled_at`.

SB-O-2. `status` MUST be `pending`, `completed`, or `cancelled`. `order_number` MUST be unique. `quote_json` MUST contain a versioned immutable snapshot of the product, amounts, plan quotas, duration, Groups, and payment channel public fields used at order creation.

SB-O-3. `POST /api/dashboard/store/orders` MUST derive `user_id` from the session. It MUST accept `product_id`, `payment_channel_id`, `payment_currency`, and an optional custom recharge minor amount.

SB-O-4. Order creation MUST reject a disabled or missing product with HTTP `404` code `product_not_available`. It MUST reject a disabled or missing payment channel with HTTP `400` code `invalid_payment_channel`.

SB-O-5. Order creation MUST quote the selected payment currency with the current exchange rate. The persisted `payment_minor`, `cny_per_usd`, and `quote_json` MUST NOT change after insertion.

SB-O-6. `GET /api/dashboard/store/orders` MUST return only orders owned by the session user. Admin order reads MUST return every order. Lists MUST order by `created_at DESC`, then `id DESC` and support a maximum page size of 100.

SB-O-7. `POST /api/dashboard/store/admin/orders/{id}/complete` MUST require an admin session. It MUST lock or serialize the order, validate `status`, apply the reward, and set `status = completed` in one database transaction.

SB-O-8. Completing an already completed order MUST return HTTP `200` with the current order and MUST NOT apply another reward. Completing a cancelled order MUST return HTTP `409` code `order_cancelled`.

SB-O-9. Completing a balance order MUST convert actual received from its quote currency to nano USD with the persisted order rate. It MUST add the amount to `users.balance_nano_usd` with checked arithmetic and append one `billing_ledger` row with kind `store_recharge`, a delta dedupe key derived from the order id, and order metadata.

SB-O-10. Completing a plan order MUST create or replace the user's active Store entitlement from `quote_json`. The user's ordinary balance MUST remain unchanged.

SB-O-11. `POST /api/dashboard/store/admin/orders/{id}/cancel` MUST change only a pending order to cancelled. Repeating cancellation MUST return the cancelled order. A completed order MUST return HTTP `409` code `order_completed`.

## 5. Plan Entitlements

SB-E-1. `store_plan_entitlements` MUST contain one current row per user: `id`, `user_id`, `product_id`, `product_name`, `starts_at`, `ends_at`, `cny_per_usd`, `group_ids`, `quota_json`, `source_kind`, and `source_id`.

SB-E-2. Activating an entitlement MUST copy all plan fields and quota rules. A later product edit or delete MUST NOT modify an existing entitlement.

SB-E-3. An entitlement starts at completion or redemption time. It ends exactly `duration_seconds` later. Replacing an active entitlement starts the new entitlement immediately. Unused time from the prior entitlement is not carried forward.

SB-E-4. `source_kind` MUST be `order` or `redemption`. The pair `(source_kind, source_id)` MUST be unique.

SB-E-5. Store responses MUST expose the current entitlement when `ends_at > now`. An expired entitlement MUST be treated as absent.

SB-E-6. Every plan quota rule applies concurrently. Quota enforcement MUST compare settled request charges converted from nano USD to CNY with the entitlement rate snapshot. A request is denied with HTTP `402` code `plan_quota_exhausted` when any applicable window has no remaining quota before routing. One already-routed request MAY consume the remaining amount and make the settled total exceed a limit; later requests MUST be denied.

SB-E-7. Window boundaries use `Asia/Shanghai`. `day`, `week`, and `month` use the current calendar day, Monday-based calendar week, and calendar month. Fixed-hour and custom-hour windows are rolling windows ending at request time.

SB-E-8. A non-empty entitlement `group_ids` list restricts routing using the same intersection and order rules as billing plan subscriptions BP-R1 through BP-R3.

## 6. Redemption Codes

SB-R-1. `store_redemption_codes` MUST contain: `id`, `code_digest`, `code_hint`, `reward_kind`, `reward_json`, `status`, `expires_at`, `redeemed_by_user_id`, `redeemed_at`, `created_by_user_id`, and `created_at`.

SB-R-2. Plain redemption codes MUST contain 16 random base32 characters grouped as `XXXX-XXXX-XXXX-XXXX`. The database MUST store a SHA-256 digest and the final four-character hint. Admin list responses MUST NOT return the original code after the generation response.

SB-R-3. An admin generation request MUST create 1 through 20 codes with an expiration from 1 through 365 days. A reward MUST be either a positive CNY or USD balance amount or an enabled plan snapshot source.

SB-R-4. `POST /api/dashboard/store/redeem` MUST require a dashboard session and accept only `code`. It MUST normalize ASCII letters to uppercase and remove ASCII hyphens before digest comparison.

SB-R-5. Redemption MUST serialize the code row. A missing code returns HTTP `404` code `invalid_redemption_code`. An expired code returns HTTP `409` code `redemption_code_expired`. A used code returns HTTP `409` code `redemption_code_used`.

SB-R-6. Successful redemption MUST apply the reward and change the code to used with `redeemed_by_user_id` and `redeemed_at` in one transaction. It MUST NOT create a payment order or require a payment channel.

SB-R-7. Balance redemption MUST append one `billing_ledger` row with kind `redemption_credit` and a delta dedupe key derived from the code id. Plan redemption MUST activate one entitlement with `source_kind = redemption`.

SB-R-8. A USD balance redemption MUST NOT require an exchange-rate snapshot. A CNY balance redemption and a plan redemption MUST return HTTP `503` code `exchange_rate_unavailable` when no snapshot exists.

## 7. API Surface

SB-A-1. Session user endpoints are:

- `GET /api/dashboard/store/catalog`
- `GET /api/dashboard/store/exchange-rate`
- `GET /api/dashboard/store/entitlement`
- `GET /api/dashboard/store/orders`
- `POST /api/dashboard/store/orders`
- `POST /api/dashboard/store/redeem`

SB-A-2. Admin endpoints are:

- `GET|POST /api/dashboard/store/admin/products`
- `PUT|DELETE /api/dashboard/store/admin/products/{id}`
- `GET|POST /api/dashboard/store/admin/payment-channels`
- `PUT|DELETE /api/dashboard/store/admin/payment-channels/{id}`
- `GET /api/dashboard/store/admin/orders`
- `POST /api/dashboard/store/admin/orders/{id}/complete`
- `POST /api/dashboard/store/admin/orders/{id}/cancel`
- `GET|POST /api/dashboard/store/admin/redemption-codes`
- `GET|PUT /api/dashboard/store/admin/settings`

Every endpoint in SB-A-2 MUST require an admin session.

SB-A-3. User mutations MUST reject replica nodes by the repository's shared write policy. Admin mutations MUST use the same policy.

SB-A-4. API errors MUST use the dashboard JSON error envelope. Invalid money uses code `invalid_amount`. Invalid currency uses code `invalid_currency`. Arithmetic overflow uses HTTP `409` code `amount_overflow`.

SB-A-5. `GET /api/dashboard/store/entitlement` MUST derive the user from the session. It MUST return the active entitlement from SB-E-5 or JSON `null`.

## 8. Frontend

SB-UI-1. `/dashboard/store` MUST render the Store. `/dashboard/orders` MUST render user orders. `/dashboard/store-admin` MUST render Store administration only for admin roles.

SB-UI-2. The Store MUST use a three-position sliding segmented control for balance, plan, and redemption. The selected section MUST use `aria-selected = true`. Reduced-motion mode MUST remove transforms and entry animations.

SB-UI-3. The payment section MUST occupy one full-width row at the bottom of the purchase tool. It MUST appear in balance and plan modes. It MUST be absent from the redemption DOM subtree.

SB-UI-4. The order summary MUST keep a stable minimum block size when switching between balance and plan modes. It MUST be absent in redemption mode.

SB-UI-5. Simplified Chinese recharge copy MUST use `实得` and `实得金额`. It MUST NOT use `实到` for the sum of recharge and bonus.

SB-UI-6. The CNY/USD controls in balance and plan modes MUST share one state. Changing either control MUST update balance, current-month usage, product price, bonus, actual received, plan price, plan quota, and summary values without a reload.

SB-UI-7. Plan quota values MUST display as whole numbers using SB-FX-5. Currency symbols MUST match the selected display currency. A CNY plan quota MUST NOT show a USD symbol.

SB-UI-8. Store, Orders, and Store Management reads MUST use SWR and render skeletons during initial loading. User-triggered mutations MUST use optimistic state and rollback on error.

SB-UI-9. The admin page MUST provide four sliding tabs: Products, Payment Channels, Orders, and Redemption Codes. Product and channel forms MUST use modal dialogs.

SB-UI-10. Main Store cards MUST use a 16 px radius. Interactive controls SHOULD use a 12 px radius. Product containers MUST use natural height and MUST expand vertically as records are added.

SB-UI-11. Payment channel cards MUST render the configured icon. An image load failure MUST render a text fallback. Alipay and WeChat built-in channels MUST use their brand icons.

## 9. Data Fetching, Cache, And Tests

SB-Q-1. A successful user mutation MUST revalidate catalog/session balance, orders, and current entitlement as applicable. A page close and reopen MUST NOT be required.

SB-Q-2. Backend tests MUST cover SQLite migration constraints, exchange-rate parsing, exact conversion, order quote immutability, idempotent completion, balance ledger credit, entitlement snapshot replacement, expired codes, and concurrent code redemption.

SB-Q-3. PostgreSQL-specific schema statements and transaction-lock branches MUST exist and compile. Automated local verification for this change MUST NOT start PostgreSQL on the user's computer.

SB-Q-4. Frontend tests MUST cover the tab indicator, stable summary dimensions, shared currency selection, integer plan quota display, payment channel visibility, custom products, separate order page, admin controls, and redemption mode without payment or summary controls.
