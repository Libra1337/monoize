# LynShen Store And Billing Design

## Scope

This design adds a Store to LynShen Console. The Store sells balance recharge products and time-bound plans. It also accepts redemption codes. Users view orders on a separate page. Admins manage products, payment channels, orders, and redemption codes on one management page.

The first production release does not implement a direct WeChat Pay or Alipay merchant integration. A purchase creates a pending order. An admin confirms payment, or an external payment adapter calls the same idempotent completion service later. This boundary prevents the UI from claiming automatic payment while no merchant credentials exist.

## Currency Model

The existing account ledger remains `balance_nano_usd`. This is the only spendable balance.

Store product prices can use CNY or USD as their base currency. Plan quota values use CNY as their base. The Store currency control changes presentation and the payment quote. It does not create a second account balance.

The backend refreshes CNY per USD from `https://open.er-api.com/v6/latest/USD` every 15 minutes. It stores the last successful rate. A failed refresh keeps the last successful rate. Order creation copies the rate, source timestamp, product definition, and selected currency into an immutable quote snapshot.

Balance recharge products define a recharge amount and a bonus amount. The UI displays three stable rows: recharge amount, bonus amount, and actual received amount. The Simplified Chinese label is `实得金额`. Actual received equals recharge plus bonus. Completing an order converts the actual received amount to nano USD once with the order rate snapshot, then writes one idempotent ledger credit.

## Store Navigation

The user navigation adds `Store` and `Orders`. The admin navigation adds `Store Management`.

The Store uses one three-position segmented control:

1. Balance recharge.
2. Plan purchase.
3. Redemption code.

The indicator slides between positions. Content enters with a short horizontal fade. Motion is disabled under `prefers-reduced-motion`.

The Store card uses a 16 px radius. Controls use a 12 px radius. Sections use natural height. Adding products or plans expands the page vertically.

Payment methods form a full-width section at the bottom of the purchase card. The section never shares a row with product fields. The redemption-code view hides payment methods and the order summary.

The CNY/USD switch is in the upper-right of both purchase views. Both switches share one selected currency. It updates balance, monthly usage, product amounts, plan prices, plan quotas, and order summary. Plan quota amounts use integer standard rounding and show no decimal places.

## Products And Plans

An admin creates a recharge product with a base currency, recharge amount, bonus amount, sort order, and enabled state. The backend derives actual received; clients cannot submit a separate value.

An admin creates a plan with a name, description, price, base currency, purchase duration, Group access, sort order, and one or more quota rules. Each quota rule contains a window kind and a CNY quota. Supported windows are 5 hours, 12 hours, day, week, month, and a custom whole-hour duration.

All quota rules apply at the same time. Activating a plan creates an immutable entitlement snapshot. A later plan edit affects only later purchases and redemptions.

## Payment Channels And Orders

The database seeds disabled Alipay and WeChat templates. Admins can enable, disable, rename, and configure them. Admins can add custom channels. A channel icon uses either an uploaded image or an HTTPS image URL. Only enabled channels appear in the Store.

Order creation validates the current product and payment channel. It returns a pending order and the configured payment instructions. The order list is a separate user page. Admin order management can confirm or cancel pending orders. Confirmation is atomic and idempotent.

An order completion credits balance or activates a plan exactly once. Repeated confirmation returns the completed order without a second credit or entitlement.

## Redemption Codes

Admins generate one to 20 single-use codes at a time. A batch grants either a balance amount or one plan. Each code has an expiration time.

Redeeming a code does not create an order and does not require a payment channel. Redemption locks the code, verifies ownership state and expiration, applies the reward, and records the user and time in one transaction. A concurrent second redemption cannot apply the reward twice.

## Data Fetching And States

React pages use SWR. Initial reads show skeletons. User mutations update visible state optimistically and then revalidate. A failed mutation restores the previous state and shows a toast. Creating or completing an order updates Store, Orders, and session balance without closing and reopening a page.

The admin surface uses four animated tabs: Products, Payment Channels, Orders, and Redemption Codes. Modal forms validate amounts, names, URLs, image size, quota windows, code count, and validity before submission.

## Security

User endpoints derive the user from the dashboard session. They never accept a user id from the request body. Admin endpoints use the existing admin-session guard.

Uploaded icons accept PNG, JPEG, WebP, or SVG up to 2 MiB. SVG content is served as an attachment-safe image asset and is never injected as HTML. External icons require HTTPS. Payment channel secret configuration is encrypted or withheld from every response.

The backend computes every amount, conversion, bonus, and reward. Client totals are display-only. Decimal values never pass through binary floating-point arithmetic.

## Verification

SQLite migration and service tests cover schema constraints, order idempotency, recharge arithmetic, plan snapshots, exchange-rate snapshots, and redemption concurrency. PostgreSQL SQL shape remains implemented, but local PostgreSQL is not started on the user's computer.

Frontend tests cover sliding tabs, stable summary size, currency conversion, integer plan quota display, payment-channel visibility, separate orders navigation, admin forms, and the absence of payment controls in redemption mode.

Browser checks cover desktop and mobile layouts, reduced motion, empty states, custom products, custom payment channels, order confirmation, and code redemption.
