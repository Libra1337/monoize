# Sales Commission Specification

## 0. Scope and terms

SC-0.1. This document defines sales codes, commission accrual, code discounts, retroactive
order claims, agent withdrawals, and the agent and Admin surfaces for them. Order creation,
payment, fulfillment, and refunds remain governed by `store-billing.spec.md`; this document
only adds fields and hooks and names them explicitly.

SC-0.2. A **sales agent** is a `users` row with one `sales_agents` row keyed by `user_id`. An
agent MUST authenticate through the existing dashboard session of
`dashboard-session-authentication.spec.md`. Agent capability MUST NOT be a `UserRole`
variant and MUST NOT be a column on `users`, because `UserRole` is a global permission axis
and `users.account_class` already partitions standard from enterprise billing.

SC-0.3. All commission amounts are integer Coin minor units. Coin is pegged to CNY at
`1 C = 1 CNY` by `coin-wallet-navigation.spec.md` CN-3, so one Coin minor unit is one CNY
fen and the two are the same integer. Commission MUST NOT be stored in nano USD and MUST NOT
pass through `f32` or `f64`.

Commission is denominated in Coin because every recharge price is quoted in CNY and an agent
is paid in CNY. Converting to nano USD would make an agent's earned amount move with the
exchange rate after the sale, so the same order would owe a different amount tomorrow.

SC-0.5. An agent's commission balance is a platform-side accrual, not cash and not spendable
balance. It MUST NOT appear in `users.balance_nano_usd`, MUST NOT be usable to pay for API
usage, and MUST NOT enter `billing_ledger`. It becomes money only when an Admin settles a
withdrawal out of band under section 6.

SC-0.4. A **discount** is expressed in basis points of the order face value. One basis point
is 1/10000. A discount reduces what the buyer pays and reduces the agent's commission by the
same basis points; it MUST NOT change what the buyer receives and MUST NOT change platform
revenue.

## 1. Rates and bounds

SC-1.1. The commission rate is one Admin-configurable global setting `commission_rate_bp`,
an integer in `[0, 2000]`, defaulting to 500 basis points (5%). It MUST apply to every agent.
A change MUST NOT alter any existing `sales_commission_entries` row, because each entry froze
the rate that applied to its order.

SC-1.2. `discount_bp` MUST be an integer in `[0, commission_rate_bp]`. A value above the
current rate MUST be rejected with `sales_discount_above_rate`. The discount is funded from
the commission, so a discount larger than the rate would make the agent's commission
negative. Lowering `commission_rate_bp` below an existing agent's `discount_bp` MUST be
rejected with the same code rather than silently clamping, so no agent is left owing money
on a sale.

SC-1.3. For an order with face value `base_fen`, applied rate `r = commission_rate_bp`, and
an applied code with `discount_bp = d`:

```
discount_fen   = floor(base_fen * d / 10000)
payment_fen    = base_fen - discount_fen
commission_fen = floor(base_fen * (r - d) / 10000)
received_fen   = base_fen
```

All four MUST use integer arithmetic. Worked example with `r = 500`, `base_fen = 10000`, and
`d = 100`: `discount_fen = 100`, `payment_fen = 9900`, `commission_fen = 400`,
`received_fen = 10000`.

SC-1.4. Platform revenue for that order is `payment_fen - commission_fen`, which equals
`base_fen - floor(base_fen * d / 10000) - floor(base_fen * (r - d) / 10000)` and is
independent of `d` up to one fen of floor rounding. At `r = 500` a 100 CNY face value yields
95 CNY of platform revenue whether the discount is 0% or 5%. The discount is therefore funded
by the agent's own commission; the platform MUST NOT subsidize it.

SC-1.5. An entry MUST record the `commission_rate_bp` that applied, so the arithmetic of a
past order remains reproducible after the rate changes.

SC-1.6. Both shares use floor division, so neither the discount nor the commission is ever
rounded up at the platform's expense. At `r = 500` the commission is `floor(base_fen / 20)`.

SC-1.7. The minimum custom recharge for CNY MUST be 100 fen (1 CNY), which is the default of
`store-billing.spec.md` SB-P-4b. At `r = 500` this yields a minimum commission of 5 fen
(0.05 CNY), so every order a buyer can place through the custom-amount field earns a
commission expressible in two decimal places.

## 2. Data model

SC-D1. Table `sales_agents`:

| Column | Type | Constraint |
|---|---|---|
| `user_id` | TEXT | PK, references `users(id)` |
| `code` | TEXT | NOT NULL, UNIQUE |
| `discount_bp` | INTEGER | NOT NULL, `CHECK (discount_bp BETWEEN 0 AND 500)` |
| `commission_balance_fen` | TEXT | NOT NULL, canonical integer, MAY be negative per SC-3.4c |
| `enabled` | INTEGER | NOT NULL, `CHECK (enabled IN (0, 1))` |
| `created_at` | TEXT | NOT NULL, RFC3339 |
| `updated_at` | TEXT | NOT NULL, RFC3339 |

SC-D1a. `code` MUST be stored as plaintext with a UNIQUE index, unlike a redemption code,
which `store-billing.spec.md` SB-R-1 encrypts. A redemption code is a bearer instrument
equal to money, so disclosure transfers value. A sales code carries no value to its holder:
the worst outcome of disclosure is that a third party credits commission to that agent. The
agent's own page MUST display the code on every load, which an encrypt-and-reveal scheme
cannot serve without a reauthentication grant per view.

SC-D1b. `code` MUST be 8 characters drawn from Crockford Base32 excluding `I`, `L`, `O`, and
`U`, generated from a cryptographically secure source, and compared case-insensitively after
uppercasing and removing ASCII hyphens and spaces.

SC-D2. Table `sales_commission_entries`:

| Column | Type | Constraint |
|---|---|---|
| `id` | TEXT | PK |
| `agent_user_id` | TEXT | NOT NULL, references `sales_agents(user_id)` |
| `order_id` | TEXT | NOT NULL, **UNIQUE** |
| `order_number` | TEXT | NOT NULL |
| `buyer_user_id` | TEXT | NOT NULL |
| `base_fen` | TEXT | NOT NULL, canonical positive integer |
| `commission_fen` | TEXT | NOT NULL, canonical positive integer |
| `discount_bp` | INTEGER | NOT NULL |
| `commission_rate_bp` | INTEGER | NOT NULL, the SC-1.5 frozen rate |
| `origin` | TEXT | NOT NULL, `CHECK (origin IN ('code', 'claim'))` |
| `reversed_at` | TEXT | NULL, RFC3339 |
| `created_at` | TEXT | NOT NULL, RFC3339 |

SC-D2a. The UNIQUE constraint on `order_id` is the sole mechanism that makes commission
at-most-once per order. It MUST hold regardless of whether the entry arrived from an applied
code or from a retroactive claim, so a claim cannot duplicate an accrual that a code already
produced.

SC-D3. Table `sales_withdrawals`:

| Column | Type | Constraint |
|---|---|---|
| `id` | TEXT | PK |
| `agent_user_id` | TEXT | NOT NULL, references `sales_agents(user_id)` |
| `amount_fen` | TEXT | NOT NULL, canonical positive integer |
| `state` | TEXT | NOT NULL, `CHECK (state IN ('requested', 'paid', 'rejected', 'cancelled'))` |
| `requested_at` | TEXT | NOT NULL, RFC3339 |
| `decided_at` | TEXT | NULL, RFC3339 |
| `decided_by` | TEXT | NULL, Admin `users.id` |
| `decision_note` | TEXT | NULL |

SC-D4. Table `sales_claim_attempts`, used only for rate limiting:

| Column | Type | Constraint |
|---|---|---|
| `id` | TEXT | PK |
| `agent_user_id` | TEXT | NOT NULL |
| `succeeded` | INTEGER | NOT NULL, `CHECK (succeeded IN (0, 1))` |
| `attempted_at` | TEXT | NOT NULL, RFC3339 |

SC-D5. `store_orders` MUST gain two nullable columns: `sales_code` TEXT and
`sales_discount_bp` INTEGER. A null `sales_code` means no code was applied at creation; a
later claim MUST NOT write them, because they record what the buyer submitted, not who was
credited.

SC-D5a. Both columns are part of the immutable order snapshot of `store-billing.spec.md`
SB-P-13, so migration 068 MUST rebuild the `trg_store_orders_quote_immutable` guard of
migration 051 to include them, on both SQLite and PostgreSQL. Its `down` MUST rebuild the
guard without them before dropping the columns.

The code decides which agent an order pays. Left outside the guard, an update could reassign
the commission of an order that is already paid and fulfilled, which the frozen amounts
beside it are already protected against.

SC-D6. Migration `m20260909_000068_sales_commission` MUST create SC-D1 through SC-D4, add
the SC-D5 columns, and create an index on `sales_commission_entries(agent_user_id,
created_at)` for the agent dashboard aggregates and an index on
`sales_commission_entries(order_number)` for claim lookup. Its `down` MUST drop what it
created.

## 3. Applying a code at order creation

SC-2.1. `POST /api/dashboard/store/orders` MUST accept one optional field `sales_code`. An
absent, null, or empty-after-trimming value MUST create the order with no code applied.

SC-2.2. A submitted code MUST resolve to an `enabled` `sales_agents` row after the SC-D1b
normalization. An unresolvable code MUST return HTTP `400` with code `sales_code_invalid`
and MUST create no order.

SC-2.3. An agent MUST NOT apply their own code to their own order. `agent_user_id` equal to
the ordering user MUST return HTTP `400` with code `sales_code_self_referral` and MUST
create no order.

SC-2.4. When a code resolves, order creation MUST set `payment_minor = payment_fen` and
`quote_json.product.balance.actual_received_minor = base_fen` per SC-1.3, where `base_fen`
is the face value that SB-P-3 or SB-P-4 would have produced without a code. It MUST persist
`sales_code` and `sales_discount_bp` on the order.

SC-2.5. The SB-P-3 equality between a fixed balance product's price and the order's
`payment_minor` MUST be evaluated against `base_fen`, not against the discounted
`payment_minor`. Without this, every discounted order fails the existing
`balance recharge and product price differ` check.

SC-2.6. The channel amount bounds of SB-C-30 and SB-C-31 MUST be evaluated against the
discounted `payment_minor`, because that is the amount the provider will charge. A discount
that pushes the amount below a channel minimum MUST make the channel unavailable exactly as
any other out-of-range amount does.

SC-2.7. A code MUST apply only to a `balance` product order. A `plan` product order with a
`sales_code` MUST return HTTP `400` with code `sales_code_not_applicable`.

SC-2.7a. An order whose face value is below 100 minor units (1 CNY) MUST reject an applied
code with HTTP `400` and code `sales_code_amount_too_small`, and MUST create no order. It
MUST NOT create the order with a zero commission instead.

At `r = 500` a face value below 20 fen floors its commission to zero under SC-1.6, so the
agent would make a real sale and be credited nothing. Refusing the order tells the buyer to
remove the code or raise the amount, whereas accruing zero would look to the agent like the
sale was never attributed. The 100-minor-unit bound is stricter than the 20 needed for a
nonzero result, because SC-1.7 already sets 1 CNY as the smallest purchasable amount and a
commission should be expressible in two decimal places.

SC-2.7a. An order carrying a code MUST have a face value of at least 100 Coin minor units,
that is 1 CNY. A smaller face value MUST return HTTP `400` with code
`sales_code_amount_too_small` and MUST create no order.

The bound exists so commission is always representable in two decimal places and never
floors to zero. At the 500 bp default, 1 CNY yields exactly 5 minor units (0.05 CNY), the
smallest nonzero commission the currency can express. A 1 minor unit order would yield
`floor(1 * 500 / 10000) = 0`, crediting the agent nothing for a sale that consumed one of
their referrals. The bound is on the face value, not the discounted payable amount, because
the face value is the commission base under SC-1.3.

SC-2.8. `creation_request_digest` MUST include the normalized `sales_code`, so that a retry
of the same idempotency key with a different code is a conflict rather than a silent reuse
of the first quote.

## 4. Accrual

SC-3.1. Commission MUST accrue inside the same transaction as the `store_recharge`
`billing_ledger` insert of SB-RC fulfillment, and MUST NOT accrue at payment projection.
Accruing at projection would credit an agent for an order whose fulfillment later fails,
because SB-RC-1 makes projection and fulfillment separate transactions.

SC-3.2. Accrual MUST insert one `sales_commission_entries` row with `origin = 'code'` and
increase `sales_agents.commission_balance_fen` by `commission_fen`. Both writes MUST be in
that transaction. A unique-violation on `order_id` MUST be treated as already accrued and
MUST NOT fail fulfillment, which makes a repeated callback idempotent.

SC-3.3. `base_fen` for accrual MUST be the balance quote's `recharge_minor`: the amount the
buyer owed before any discount, read from the frozen quote.

It MUST NOT be `payment_minor`, which a discount lowers, because SC-1.4 defines the
commission against the undiscounted amount. It MUST NOT be `actual_received_minor`, which a
bonus raises above what the platform was paid: a product selling 100 CNY of balance with a
20 CNY bonus collects 100 CNY, and accruing on 120 would pay the agent 6 CNY out of that 100
and cut platform revenue to 94.

SC-3.3a. An order whose `payment_currency` is not CNY MUST NOT accrue commission, and SC-2.2
MUST reject a code submitted on such an order with `sales_code_currency_unsupported`. The
rate and the discount are both defined on a CNY face value; accepting a code on a USD order
would apply the discount while crediting nothing, making the platform fund it alone.

SC-3.4. A refund of an order with a commission entry MUST reverse it in the refund
transaction: set `reversed_at`, and decrease `commission_balance_fen` by `commission_fen`.
Without reversal the agent keeps commission on money the buyer got back.

SC-3.4a. The balance MUST be allowed to go negative, and MUST NOT be clamped at zero. A
negative balance is a debt the agent owes, carried until later commission repays it. When
the agent had already withdrawn the reversed amount, clamping at zero would silently forgive
that debt and the next sale would pay the agent again from a zero base.

SC-3.4b. A subsequent accrual MUST add to the balance whatever its sign, so a debt is repaid
before the agent can withdraw again. The balance therefore rises from negative toward zero
and only becomes withdrawable once it is at least the SC-5.1 minimum.

SC-3.4c. `commission_balance_fen` MUST be a canonical integer that MAY carry a leading `-`.
The reversed entry MUST remain in place with its original `commission_fen`, so the debt's
origin stays auditable after the balance recovers.

## 5. Retroactive claim

SC-4.1. `POST /api/dashboard/sales/claims` MUST require an agent session and accept exactly
`{ "order_number": string, "user_id": string }`.

SC-4.2. A claim MUST succeed only when all of the following hold: the order exists with that
`order_number`; its `user_id` equals the submitted `user_id`; its `payment_state` is `paid`;
its `fulfillment_state` is `fulfilled`; its `payment_currency` is CNY; no
`sales_commission_entries` row exists for that `order_id`; and the submitted `user_id` is not
the claiming agent.

SC-4.3. A successful claim MUST insert one entry with `origin = 'claim'`,
`discount_bp = 0`, `commission_fen = floor(base_fen * 500 / 10000)`, and MUST increase the
agent's balance by that amount in the same transaction. It MUST record one
`sales_claim_attempts` row with `succeeded = 1`.

SC-4.4. A claim whose order number and user ID do not correspond, or whose order does not
exist, MUST return HTTP `404` with code `sales_claim_mismatch` and the message that the
order and user do not correspond. It MUST record one `sales_claim_attempts` row with
`succeeded = 0` and MUST NOT create an entry.

SC-4.5. A claim for an order that already has an entry MUST return HTTP `409` with code
`sales_claim_already_credited`. This MUST hold whether the existing entry came from a code or
an earlier claim, which is what prevents a second credit for one order.

SC-4.6. A failed claim MUST NOT consume the order's single credit opportunity: the order
remains claimable until a claim succeeds. The one-success limit is enforced by the SC-D2a
unique constraint, not by counting attempts.

SC-4.7. Because SC-4.6 permits unlimited retries, claims MUST be rate limited per agent: at
most 10 attempts per minute, and 5 failures within 15 minutes MUST impose a 30-minute
cooldown returning HTTP `429` with code `sales_claim_rate_limited`. Without a limit an agent
can enumerate `user_id` values against a known order number and learn which buyer an order
belongs to.

## 6. Withdrawal

SC-5.1. `POST /api/dashboard/sales/withdrawals` MUST require an agent session and accept
exactly `{ "amount_fen": string }`. `amount_fen` MUST be a canonical positive integer at
most the agent's `commission_balance_fen`, and at least 10000 fen (100 CNY).

SC-5.1a. An agent whose `commission_balance_fen` is zero or negative MUST NOT be able to
withdraw, because SC-5.1 requires a positive amount capped at the balance. The Sales surface
MUST disable the withdrawal action in that state.

SC-5.1b. `DELETE /api/dashboard/sales/withdrawals/{id}` MUST require an agent session, accept
no body, and cancel that agent's own withdrawal while its state is `requested`. It MUST set
`state = 'cancelled'` and return `amount_fen` to `commission_balance_fen` in one transaction.
A withdrawal belonging to another agent MUST return HTTP `404`, and one already decided MUST
return HTTP `409` with `sales_withdrawal_not_pending`.

Cancellation exists because SC-5.2 holds the amount at request time and SC-5.3 admits only
one pending withdrawal. Without it, an agent who typed the wrong amount would be blocked
until an Admin acted on a request neither party wants.

The request MUST NOT carry payout details. Settlement happens out of band: the Admin
contacts the agent through an existing channel and transfers the money, then records the
outcome under SC-5.4. Storing a payout destination would put bank or wallet identifiers into
the database with no code path that reads them.

SC-5.2. Creating a withdrawal MUST decrease `commission_balance_fen` by `amount_fen` in the
same transaction that inserts the `requested` row. Holding the amount at request time
prevents an agent from requesting the same balance twice before an Admin acts.

SC-5.3. An agent MUST NOT have more than one `requested` withdrawal at a time. A second
request MUST return HTTP `409` with code `sales_withdrawal_pending`.

SC-5.4. `POST /api/dashboard/store/admin/sales/withdrawals/{id}/decide` MUST require an
Admin session, the SB-S-2 Origin check, and a five-minute reauthentication grant with scope
`sales_withdrawal`. Its exact body MUST be
`{ "decision": "paid" | "rejected", "decision_note": string }`, where `decision_note` MAY be
empty.

SC-5.5. A `paid` decision records that the Admin has already transferred the money out of
band. It MUST set `state = 'paid'`, `decided_at`, and `decided_by`, and MUST NOT change
`commission_balance_fen`, because SC-5.2 already removed the amount. It MUST NOT move money
itself: no `billing_ledger` row and no change to `users.balance_nano_usd`.

SC-5.6. A `rejected` decision MUST set `state = 'rejected'` and MUST return `amount_fen` to
`commission_balance_fen` in the same transaction.

SC-5.7. A decision on a withdrawal not in state `requested` MUST return HTTP `409` with code
`sales_withdrawal_not_pending`.

SC-5.8. `GET /api/dashboard/store/admin/sales/withdrawals` MUST require an Admin session and
return withdrawals in descending `requested_at` order with at most 100 records, each carrying
the agent username and current balance.

## 7. Agent surfaces

SC-6.1. `GET /api/dashboard/sales/overview` MUST require an agent session and return the
agent's `code`, `discount_bp`, `commission_balance_fen`, aggregates, and the pending
withdrawal when one exists. A non-agent session MUST return HTTP `403` with code
`sales_agent_required`.

SC-6.2. Aggregates MUST be computed over non-reversed entries and MUST cover exactly three
windows: today in UTC, the trailing 7 days, and the trailing 30 days. Each window MUST report
`sales_fen` as the sum of `base_fen`, `commission_fen` as the sum of `commission_fen`, and
`order_count` as the number of entries.

SC-6.3. `GET /api/dashboard/sales/entries` MUST require an agent session and return that
agent's entries in descending `created_at` order with at most 100 records, each carrying
`order_number`, `base_fen`, `commission_fen`, `origin`, `reversed_at`, and `created_at`. It
MUST NOT return the buyer's username, email, or any other buyer identity beyond the
`buyer_user_id` the agent already submits when claiming.

SC-UI-2c. The withdrawal panel MUST render the complete SC-6.4 log with each entry's state
and its decision time when decided, and MUST expose a cancel action while a request is
pending. It MUST state the withdrawable balance rather than a minimum, because SC-5.1 has no
minimum beyond one minor unit.

SC-6.4. `GET /api/dashboard/sales/withdrawals` MUST require an agent session and return that
agent's own withdrawals in descending `requested_at` order with at most 100 records. Every
state MUST appear, including `cancelled` and `rejected`, so the list is a complete withdrawal
log rather than only the outstanding request.

## 8. Frontend

SC-UI-1. An agent MUST land on a dedicated Sales surface at `/sales`, outside the dashboard
shell. It MUST NOT reuse the standard-user navigation of `dashboard-ui-layout.spec.md` DL5
or the Enterprise navigation of DL5c, and it MUST NOT render the dashboard sidebar. An agent
session that requests `/dashboard` or any `/dashboard/*` route MUST be redirected to
`/sales`; a non-agent session that requests `/sales` MUST be redirected to `/dashboard`.

An agent exists only to sell. Presenting the API-key, usage, log, and Marketplace surfaces
would imply capabilities the account is not created for, and presenting the Store would let
an agent buy under their own code, which SC-2.3 forbids.

SC-UI-2. The Sales surface MUST present its content as two sub-pages selected by a tab
control: an overview carrying the agent's code, balance, and the three SC-6.2 windows; and a
records page carrying the claim form, the withdrawal panel, and the SC-6.3 entry list. One
continuous page ran past the viewport at 100% zoom, which forced an agent to scroll or zoom
out to read today's numbers. The surface MUST expose sign-out. It MUST use SWR with a skeleton that matches the ready layout, and MUST
apply an optimistic value for a withdrawal request and a claim submission, rolling back on
error.

SC-UI-2a. Every monetary value on the Sales surface MUST render as Coin through the shared
Coin mark of `coin-wallet-navigation.spec.md` CN-16. The surface MUST NOT show a nano value
and MUST NOT apply an exchange rate, because SC-0.3 stores commission in Coin minor units
already.

SC-UI-2b. The balance MUST render under one label with its sign, so a debt reads as
`-<amount>` rather than being relabelled. It MUST carry `destructive` styling when negative.
The sign MUST be rendered separately from the amount, because the Coin formatter accepts only
a nonnegative value, and a screen reader MUST receive the sign as a word rather than only as
a hyphen glyph. A reversed entry MUST be visibly marked as reversed in the SC-6.3 list and
MUST NOT be silently omitted, because it is the origin of the debt.

SC-UI-3. The Store purchase panel MUST render a sales-code input beside the custom-amount
input, each occupying half of the row at `sm` and above and stacking below it. The field MUST
be optional and MUST be labelled as optional.

SC-UI-4. A rejected code MUST render the message that the sales code is invalid and MUST ask
the user to re-enter it. The page MUST NOT submit an order while a code is present and known
to be invalid.

SC-UI-5. When a code with a nonzero discount is applied, the order summary MUST show the
face value, the discount, and the amount payable as separate lines, so the buyer can see
that the amount received is the face value rather than the discounted amount.

SC-UI-6. Sales administration MUST be its own dashboard page at `/dashboard/sales-admin`
with its own admin navigation entry. It MUST NOT be a child tab of Store Management: agents,
commission records, and withdrawals are a distinct workflow from products and payment
channels, and nesting it there pushed the page past the viewport.

The page MUST expose the commission rate with an edit control, the agent list with creation
and enable/disable, the withdrawal queue with a decide action per pending request, the
SC-7.7 Admin claim form, and the SC-7.6 commission records across every agent. Creating an
agent MUST create the user account and the `sales_agents` row in one request and MUST return
the generated code and password once in the creation response.

SC-UI-7. Every string introduced by this document MUST exist in `en`, `zh`, `zh-TW`, and
`ja`.

## 9. Admin agent creation

SC-7.1. `POST /api/dashboard/store/admin/sales/agents` MUST require an Admin session and the
SB-S-2 Origin check. Its exact body MUST be `{ "discount_bp": integer }`. The Admin supplies
neither a username nor a password.

SC-7.2. It MUST generate one SC-D1b code, then create one `users` row and one `sales_agents`
row in a single transaction, with `username` equal to the generated code, a password of at
least 16 characters from a cryptographically secure source, role `user`, and
`commission_balance_fen = "0"`. A code collision on either the `users.username` unique index
or the `sales_agents.code` unique index MUST retry with a new code and MUST create neither
row on failure.

SC-7.2a. The generated password MUST be returned exactly once, in the creation response, and
MUST NOT be retrievable afterwards, because only its hash is stored. The agent MAY change
both username and password afterwards through the existing account endpoints; the sales code
MUST NOT change with the username, because orders already reference the code.

SC-7.3. `PUT /api/dashboard/store/admin/sales/agents/{user_id}` MUST accept
`{ "discount_bp": integer, "enabled": boolean }`. A changed `discount_bp` MUST NOT alter any
existing `sales_commission_entries` row, because each entry froze the basis points that
applied to its order.

SC-7.5. `PUT /api/dashboard/store/admin/sales/settings` MUST require an Admin session and the
SB-S-2 Origin check, and accept exactly `{ "commission_rate_bp": integer }` within the SC-1.1
bounds. It MUST reject a value below any enabled agent's `discount_bp` under SC-1.2.

SC-7.6. `GET /api/dashboard/store/admin/sales/entries` MUST require an Admin session and
return commission entries across every agent in descending `created_at` order with at most
100 records. It MUST accept an optional `agent_user_id` filter. Unlike the agent view of
SC-6.3, it MUST include `agent_user_id`, the agent username, and `buyer_user_id`, because the
Admin is reconciling who owes whom and already has access to both identities.

SC-7.7. `POST /api/dashboard/store/admin/sales/claims` MUST require an Admin session and the
SB-S-2 Origin check, and accept exactly
`{ "agent_user_id": string, "order_number": string, "user_id": string }`. It credits the named
agent for a past order on that agent's behalf.

SC-7.7a. It MUST apply the same eligibility rules as SC-4.2, evaluated against the named
agent rather than the caller: the order exists, its `user_id` matches, it is paid and
fulfilled, its currency is CNY, it has no existing entry, and the buyer is not the named
agent. It MUST produce the same errors as SC-4.4 and SC-4.5.

SC-7.7b. It MUST NOT be subject to the SC-4.7 rate limit and MUST NOT record a
`sales_claim_attempts` row. That limit exists to stop an agent from enumerating buyer
identities through the error channel; an Admin can already read any order directly, so the
limit would restrict a caller who has nothing to learn while making bulk correction
impractical.

SC-7.7c. A successful Admin claim MUST be indistinguishable from an agent claim in
`sales_commission_entries`: `origin` is `claim` and the entry credits the named agent. The
Admin's identity MUST be recorded in `store_access_audits` rather than on the entry, so the
agent's own totals are not polluted by who filed the claim.

SC-7.4. Disabling an agent MUST make their code unresolvable for new orders under SC-2.2 and
MUST leave existing entries, balance, and withdrawals intact.
