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

SC-0.3. All commission amounts are integer CNY fen. Commission MUST NOT be stored in
nano USD and MUST NOT pass through `f32` or `f64`.

Commission is denominated in CNY because every recharge price is quoted in CNY and an agent
is paid in CNY. Converting to nano USD would make an agent's earned amount move with the
exchange rate after the sale, so the same order would owe a different amount tomorrow.

SC-0.4. A **discount** is expressed in basis points of the order face value. One basis point
is 1/10000. A discount reduces what the buyer pays and reduces the agent's commission by the
same basis points; it MUST NOT change what the buyer receives and MUST NOT change platform
revenue.

## 1. Rates and bounds

SC-1.1. The commission rate MUST be exactly 500 basis points (5%) of the order face value,
for every agent and every order.

SC-1.2. `discount_bp` MUST be an integer in `[0, 500]`. A value above 500 MUST be rejected,
because the discount is funded from the commission and 500 is the whole commission.

SC-1.3. For an order with face value `base_fen` and an applied code with `discount_bp = d`:

```
discount_fen   = floor(base_fen * d / 10000)
payment_fen    = base_fen - discount_fen
commission_fen = floor(base_fen * (500 - d) / 10000)
received_fen   = base_fen
```

All four MUST use integer arithmetic. Worked example, `base_fen = 10000` and `d = 100`:
`discount_fen = 100`, `payment_fen = 9900`, `commission_fen = 400`, `received_fen = 10000`.

SC-1.4. Platform revenue for that order is `payment_fen - commission_fen`, which equals
`base_fen - floor(base_fen * d / 10000) - floor(base_fen * (500 - d) / 10000)` and is
independent of `d` up to one fen of floor rounding. The discount is therefore funded by the
agent, not by the platform.

## 2. Data model

SC-D1. Table `sales_agents`:

| Column | Type | Constraint |
|---|---|---|
| `user_id` | TEXT | PK, references `users(id)` |
| `code` | TEXT | NOT NULL, UNIQUE |
| `discount_bp` | INTEGER | NOT NULL, `CHECK (discount_bp BETWEEN 0 AND 500)` |
| `commission_balance_fen` | TEXT | NOT NULL, canonical nonnegative integer |
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
| `state` | TEXT | NOT NULL, `CHECK (state IN ('requested', 'paid', 'rejected'))` |
| `payout_note` | TEXT | NOT NULL, agent-supplied payout destination |
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
`sales_discount_bp` INTEGER. Both are part of the immutable order snapshot of
`store-billing.spec.md` SB-P-13 and MUST be covered by the quote-immutability trigger. A
null `sales_code` means no code was applied at creation; a later claim MUST NOT write them,
because they record what the buyer submitted, not who was credited.

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

SC-3.3. `base_fen` for accrual MUST be the order's face value in CNY fen, read from the
frozen quote. An order whose `payment_currency` is not CNY MUST NOT accrue commission,
because the rate is defined on a CNY face value.

SC-3.4. A refund of an order with a commission entry MUST reverse it in the refund
transaction: set `reversed_at`, and decrease `commission_balance_fen` by `commission_fen`.
The balance MUST NOT go negative; when the agent has already withdrawn the amount, the
balance MUST clamp at zero and the shortfall MUST remain recorded by the reversed entry, so
the operator can settle it out of band. Without reversal the agent keeps commission on money
the buyer got back.

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
exactly `{ "amount_fen": string, "payout_note": string }`. `amount_fen` MUST be a canonical
positive integer at most the agent's `commission_balance_fen`, and at least 10000 fen
(100 CNY). `payout_note` MUST be nonempty after trimming and at most 512 characters.

SC-5.2. Creating a withdrawal MUST decrease `commission_balance_fen` by `amount_fen` in the
same transaction that inserts the `requested` row. Holding the amount at request time
prevents an agent from requesting the same balance twice before an Admin acts.

SC-5.3. An agent MUST NOT have more than one `requested` withdrawal at a time. A second
request MUST return HTTP `409` with code `sales_withdrawal_pending`.

SC-5.4. `POST /api/dashboard/store/admin/sales/withdrawals/{id}/decide` MUST require an
Admin session, the SB-S-2 Origin check, and a five-minute reauthentication grant with scope
`sales_withdrawal`. Its exact body MUST be
`{ "decision": "paid" | "rejected", "decision_note": string }`.

SC-5.5. A `paid` decision MUST set `state = 'paid'`, `decided_at`, and `decided_by`, and
MUST NOT change `commission_balance_fen`, because SC-5.2 already removed the amount.

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

SC-6.4. `GET /api/dashboard/sales/withdrawals` MUST require an agent session and return that
agent's own withdrawals in descending `requested_at` order with at most 100 records.

## 8. Frontend

SC-UI-1. The sidebar MUST show a Sales entry at `/dashboard/sales` when and only when the
authenticated user is an agent, for both `standard` and `enterprise` account classes. This
extends the navigation sets of `dashboard-ui-layout.spec.md` DL5 and DL5c.

SC-UI-2. The Sales page MUST show the agent's code, the three SC-6.2 windows, the SC-6.3
entry list, the claim form, and the withdrawal panel. It MUST use SWR with a skeleton that
matches the ready layout, and MUST apply an optimistic value for a withdrawal request and a
claim submission, rolling back on error.

SC-UI-3. The Store purchase panel MUST render a sales-code input beside the custom-amount
input, each occupying half of the row at `sm` and above and stacking below it. The field MUST
be optional and MUST be labelled as optional.

SC-UI-4. A rejected code MUST render the message that the sales code is invalid and MUST ask
the user to re-enter it. The page MUST NOT submit an order while a code is present and known
to be invalid.

SC-UI-5. When a code with a nonzero discount is applied, the order summary MUST show the
face value, the discount, and the amount payable as separate lines, so the buyer can see
that the amount received is the face value rather than the discounted amount.

SC-UI-6. Store Management MUST expose a Sales child surface listing agents and pending
withdrawals, with a decide action per withdrawal that obtains the SC-5.4 reauthentication
grant. Creating an agent MUST create the user account and the `sales_agents` row in one
request and MUST return the generated code once in the creation response.

SC-UI-7. Every string introduced by this document MUST exist in `en`, `zh`, `zh-TW`, and
`ja`.

## 9. Admin agent creation

SC-7.1. `POST /api/dashboard/store/admin/sales/agents` MUST require an Admin session, the
SB-S-2 Origin check, and a `sales_withdrawal`-scoped grant is NOT required. Its exact body
MUST be `{ "username": string, "password": string, "discount_bp": integer }`.

SC-7.2. It MUST create one `users` row with role `user` and one `sales_agents` row with a
generated SC-D1b code and `commission_balance_fen = "0"`, in one transaction. A duplicate
username MUST create neither row.

SC-7.3. `PUT /api/dashboard/store/admin/sales/agents/{user_id}` MUST accept
`{ "discount_bp": integer, "enabled": boolean }`. A changed `discount_bp` MUST NOT alter any
existing `sales_commission_entries` row, because each entry froze the basis points that
applied to its order.

SC-7.4. Disabling an agent MUST make their code unresolvable for new orders under SC-2.2 and
MUST leave existing entries, balance, and withdrawals intact.
