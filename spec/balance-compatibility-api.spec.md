# Balance Compatibility API Specification

## 0. Status

- **Purpose:** Expose the balance charged by Monoize through the Codex account-usage and DeepSeek user-balance response schemas.
- **Scope:** `GET /api/codex/usage`, `GET /user/balance`, API-key authentication, API-key sub-accounts, user balances, and replica pending deductions.

## 1. Authentication

BC-A1. Both endpoints MUST require forwarding API-key authentication as defined by `api-key-authentication.spec.md`.

BC-A2. Both endpoints MUST accept either `Authorization: Bearer <token>` or `x-api-key: <token>`.

BC-A3. Both endpoints MUST apply the authenticated API key's IP whitelist before reading or returning balance data.

BC-A4. A missing or invalid API key MUST return HTTP `401` with code `unauthorized`.

BC-A5. These endpoints MUST NOT apply the forwarding balance admission gate. An authenticated caller with a zero or negative effective balance MUST receive HTTP `200` and a response that reports the unavailable balance state.

## 2. Effective balance

BC-B1. Monoize MUST resolve exactly one balance subject for each request:

1. Monoize MUST re-read the authenticated API-key row from the database by `api_key_id`.
2. If the current API-key row has `sub_account_enabled = true`, the subject is that API key and the stored balance is `api_keys.sub_account_balance_nano`.
3. Otherwise, the subject is the owning user and the stored balance is `users.balance_nano_usd`.

BC-B2. Monoize MUST parse the stored balance as a signed `i128` nano-dollar integer. Invalid persisted balance data MUST return HTTP `500` with code `internal_error`. Monoize MUST NOT substitute zero.

BC-B3. A missing API-key row, missing owning-user row, or owner mismatch detected after authentication MUST return HTTP `401` with code `unauthorized`.

BC-B4. On a replica, a finite effective balance MUST equal:

```text
effective_balance_nano_usd = stored_balance_nano_usd - pending_deductions[subject_id]
```

The subtraction MUST use checked `i128` arithmetic. Overflow MUST return HTTP `500` with code `internal_error`.

BC-B5. On a primary, or when the replica has no pending deduction for the subject, `effective_balance_nano_usd` MUST equal the stored balance.

BC-B6. A user-balance subject with `balance_unlimited = true` MUST be marked unlimited and MUST NOT subtract replica pending deductions. An API-key sub-account is always finite.

BC-B7. `available` MUST equal:

```text
unlimited OR effective_balance_nano_usd > 0
```

BC-B8. Every finite nano-dollar value returned as USD text MUST use the exact canonical formatter from `user-billing-and-model-metadata.spec.md` U2. It MUST NOT use binary floating-point conversion.

## 3. Codex response

BC-C1. `GET /api/codex/usage` MUST return HTTP `200`, `Content-Type: application/json`, and `Cache-Control: no-store` after successful authentication and balance resolution.

BC-C2. The response body MUST be:

```json
{
  "plan_type": "unknown",
  "rate_limit": {
    "allowed": true,
    "limit_reached": false,
    "primary_window": null,
    "secondary_window": null
  },
  "credits": {
    "has_credits": true,
    "unlimited": false,
    "balance": "1.25"
  },
  "spend_control": null,
  "additional_rate_limits": null,
  "rate_limit_reached_type": null,
  "rate_limit_reset_credits": {
    "available_count": 0
  }
}
```

BC-C3. The values in BC-C2 MUST be computed as follows:

- `rate_limit.allowed = available`.
- `rate_limit.limit_reached = NOT available`.
- Both rate-limit windows are `null` because Monoize balance has no time window.
- `credits.has_credits = true` iff the balance is finite and `effective_balance_nano_usd > 0`.
- `credits.unlimited = unlimited`.
- `credits.balance` is the effective USD balance string for a finite balance and `null` for an unlimited balance. One displayed Codex credit equals one USD of Monoize balance.
- `rate_limit_reached_type = null` when `available = true`.
- `rate_limit_reached_type = {"type":"rate_limit_reached"}` when `available = false`.
- Monoize does not expose Codex rolling windows, spend controls, additional metered limits, or reset credits through this endpoint.

## 4. DeepSeek response

BC-D1. `GET /user/balance` MUST return HTTP `200`, `Content-Type: application/json`, and `Cache-Control: no-store` after successful authentication and balance resolution.

BC-D2. The response body MUST be:

```json
{
  "is_available": true,
  "balance_infos": [
    {
      "currency": "USD",
      "total_balance": "1.25",
      "granted_balance": "0",
      "topped_up_balance": "1.25"
    }
  ]
}
```

BC-D3. `is_available` MUST equal `available`.

BC-D4. `balance_infos` MUST contain exactly one USD entry. `total_balance` and `topped_up_balance` MUST both equal the effective USD balance string. `granted_balance` MUST equal `"0"` because Monoize does not distinguish granted credit from topped-up credit.

BC-D5. For an unlimited user balance, `is_available` MUST be `true`; the three amount fields MUST still report the stored finite user-balance value, with `granted_balance = "0"`.

## 5. Routing and side effects

BC-R1. The only Codex compatibility path defined by this specification is `/api/codex/usage`. Monoize MUST NOT add `/codex/usage` as an alias.

BC-R2. The only DeepSeek compatibility path defined by this specification is `/user/balance`. Monoize MUST NOT add `/api/user/balance` as an alias.

BC-R3. Both endpoints MUST be served by primary and replica nodes.

BC-R4. A successful request MUST NOT create a forwarding request log, create a billing-ledger row, or mutate a balance.
