# User Sub-Accounts Specification

## 0. Status

- Purpose: define user-level sub-accounts — creation eligibility, restrictions, balance
  distribution, and the admin/list surfaces.
- Scope: `users.parent_user_id`, `/api/dashboard/subaccounts*` endpoints, store-order and
  profile guards, the dashboard sub-account page, and the admin Users grouping.
- Related specs: `dashboard-session-authentication.spec.md` (sessions),
  `api-token-management.spec.md` (per-user keys), `store-billing.spec.md` (recharge),
  `sales-commission.spec.md` (sales agents), `groups-registry.spec.md` (GR-E1b agent class).

## 1. Data model

SAU-1. A sub-account is a `users` row whose `parent_user_id` is NOT NULL and references the
`id` of another user row (the **main account**). A NULL `parent_user_id` marks an ordinary
main account. `users.parent_user_id` has an index. No other table references the
parent/child relation; referential integrity is enforced by the write paths.

SAU-2. `POST /api/dashboard/subaccounts` with `{username, password}` creates one
sub-account under the authenticated caller. The request MUST be rejected with HTTP `403`
code `sub_account_forbidden` when any of the following holds:

1. the caller is itself a sub-account (`parent_user_id` NOT NULL);
2. the caller's `account_class` is `agent` (GR-E1b);
3. a `sales_agents` row exists for the caller.

Username and password rules equal registration: username 3..22 `[A-Za-z0-9_]`, not the
reserved `_monoize_` prefix, globally unique; password at least 8 characters. Violations
return the registration error codes. On success the sub-account row has `role = user`,
`account_class` and `group_id` copied from the caller, `balance_nano_usd = '0'`,
`enabled = 1`, and `parent_user_id` set to the caller's id.

SAU-3. `GET /api/dashboard/subaccounts` returns the caller's sub-accounts ordered by
`created_at ASC, id ASC`, each with `id`, `username`, `enabled`, `balance_nano_usd`,
`created_at`, `last_login_at`, `api_key_count` (grouped count over `api_keys`), and the
UTC-day `today_calls` and `today_cost_nano_usd` aggregates.

## 2. Restrictions

SAU-4. A sub-account MUST be rejected with HTTP `403` code `sub_account_restricted` from:

- `PUT /api/dashboard/auth/me` (profile edits),
- `PUT /api/dashboard/auth/password`,
- `POST /api/dashboard/store/orders` (recharge and plan purchases).

Every other dashboard surface — API keys, playground, logs, marketplace, analytics,
settings preferences — behaves exactly as for a main account. A sub-account can create and
manage its own API keys under the unmodified `api-token-management.spec.md` rules.

SAU-5. Model discovery restrictions apply to every account regardless of sub status:
`AKG-M1` for `/v1/models` and `MM-G1`/`MM-G2` for the marketplace surfaces.

## 3. Balance distribution

SAU-6. `POST /api/dashboard/subaccounts/{sub_user_id}/transfer` with
`{amount_nano_usd: string}` moves a positive integer nano-USD amount from the caller's
balance to the sub-account's balance in one transaction. The transaction locks the main row
before the sub row, verifies `sub.parent_user_id = caller id`, and rejects
`insufficient_balance` (HTTP `400`) when the main balance would go negative. It writes two
`billing_ledger` rows in the same transaction: kind `sub_account_grant` (main, negative
delta, `meta_json.to_user_id`) and kind `sub_account_receive` (sub, positive delta,
`meta_json.from_user_id`), each with `balance_after_nano_usd` and a fresh idempotency key.

SAU-7. There is no reverse transfer: a sub-account's balance can only be spent or adjusted
by an Admin through the ordinary admin user endpoints.

## 4. Dashboard surfaces

SAU-8. `/dashboard/subaccounts` is the main account's management page. It MUST show the
main balance, the sub-account list with per-sub balance (in the selected display currency),
API key count, today calls and spend, a create dialog (username + password), and a
distribution dialog whose amount input is in the selected display currency and converts to
nano-USD exactly. The sidebar entry is visible exactly to accounts eligible under SAU-2
and hidden for sub-accounts, agent-class accounts, and sales agents.

SAU-9. The Admin Users page MUST offer a `subaccounts` grouping after `agent` and before
`sales`. A user whose `parent_user_id` is set belongs to that grouping and no other (after
the sales precedence). The grouping adds a **Main account** attribution column showing
`parent_username`. All other columns equal the other groupings.

SAU-10. A sub-account's user-settings page MUST hide the profile and password cards and
show a notice that the main account manages them; preferences (language, theme) stay
available. The Store sidebar entry is hidden for sub-accounts.

## 5. Billing

SAU-11. A sub-account's API keys bill the sub-account's own balance and wallet under the
unchanged `metered-billing.spec.md` rules. Distribution under SAU-6 is the only
self-service way for a sub-account to receive balance.
