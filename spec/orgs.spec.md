# Organization Spaces Specification

## 0. Status

- Purpose: organization spaces ("组织空间") — a shared workspace with its own wallet, an
  invite link, and member-visible API keys.
- Scope: `users.is_org`, `orgs`, `org_members`, `org_key_shares`, `api_keys.org_id` /
  `api_keys.org_share_mode`, `/api/dashboard/orgs*`, `/dashboard/org`, `/join/{token}`.
  `groups-registry.spec.md` (account classes), `api-token-management.spec.md` (keys),
  `store-billing.spec.md` (wallet, exchange rates).

## 1. Data model

ORG-1. An organization is represented by one `users` row with `is_org = 1` (the **org
wallet row**) plus one `orgs` metadata row whose `id` equals that user row's id. The org
wallet row never holds a session, never logs in, and its `balance_nano_usd` is the org
wallet. `is_org = 0` is an ordinary user.

ORG-2. `orgs`: `id` (PK, equals the org wallet user id), `owner_user_id`, `display_name`
(1..64 chars), `avatar_emoji` (1..8 bytes), `avatar_color` (7-char `#rrggbb`), `avatar_image`
(NULL or a `data:image/` URL of at most 300000 characters — the custom uploaded avatar shown
in preference to the emoji), `invite_token` (unique, 32-char random), `invite_expires_at`
(NULL = never), `invite_created_at`, `created_at`, `updated_at`.

ORG-3. `org_members`: `(org_id, user_id)` PK, `role` ∈ `owner` | `member`, `joined_at`.
Exactly one member row with `role = owner` per org.

ORG-4. `api_keys.org_id` (NULL = ordinary key) names the space a key is shared into.
`api_keys.org_share_mode` is `private`, `public`, `allow`, or `deny`. `org_key_shares
(api_key_id, member_user_id)` holds the member list whose meaning follows the mode: for
`allow` the listed members may use the key and nobody else; for `deny` every member except
the listed ones may use it. Keys always belong to and bill their `user_id` owner, carry the
ordinary API-key `model_limits` restriction, and sharing changes only visibility of the key
material inside the space.

## 2. Creation

ORG-5. `POST /api/dashboard/orgs` with `{display_name, avatar_emoji?, avatar_color?,
invite_expiry: "24h"|"3d"|"7d"|"30d"|"never"}` creates one org. The caller MUST have
`account_class = enterprise`, `parent_user_id IS NULL`, no `sales_agents` row, and
`is_org = 0`; otherwise HTTP `403 org_forbidden`. A caller MAY own at most 2 orgs
(`MAX_ORGS_PER_USER`); further creations return `409 org_limit_reached`.

ORG-6. Creation carries no fee: the enterprise account class is the sole qualification
(admin-granted), and the org wallet starts at zero. The wallet is later funded only by
owner deposits under ORG-12.

ORG-7. The creator becomes the `owner` member row. The org wallet user row is created with
`is_org = 1`, `role = user`, `account_class = enterprise`, and the default enterprise
group (first public enterprise group, else the system default group).

## 3. Invite link

ORG-8. Each org has exactly one invite link, generated at creation with the chosen expiry
(`24h`, `3d`, `7d`, `30d`, or never). The link path is `/join/{invite_token}`. The owner
can copy it from the space at any time; only the owner receives the token through the API.

ORG-9. `GET /api/dashboard/orgs/invite/{token}` (session required) returns
`{org_id, display_name, avatar_emoji, avatar_color, owner_username, member_count}` without
marking anything; a missing, expired, or full org link returns `404 invite_invalid` with
the same body (no distinction).

ORG-10. `POST /api/dashboard/orgs/join {token}` adds the caller as a `member`. It MUST
reject with `404 invite_invalid` when the token is unknown or expired, `409
org_member_limit_reached` when the org already has 15 members (`MAX_ORG_MEMBERS`), and
`409 org_already_member` when the caller is already a member. Org wallet rows cannot join.

ORG-11. `POST /api/dashboard/orgs/{org_id}/invite` with `{invite_expiry}` regenerates the
single link (old token becomes invalid immediately) and is owner-only.

## 4. Wallet

ORG-12. `POST /api/dashboard/orgs/{org_id}/deposit {amount_nano_usd}` (any member) moves a
positive amount from that member's personal wallet into the org wallet.

ORG-13. `POST /api/dashboard/orgs/{org_id}/distribute {member_user_id, amount_nano_usd}`
(owner only) moves a positive amount from the org wallet into one member's personal
wallet. Both directions write two `billing_ledger` rows in one transaction
(`org_deposit`/`org_deposit_receive` and `org_grant`/`org_receive`) with the org id and
counterparty in `meta_json` and row locks taken org-wallet-first.

## 5. Keys and sharing

ORG-14. `POST /api/dashboard/orgs/{org_id}/keys {name, share_mode?, model_limits?}` creates
an ordinary API key owned by the caller, billed to the caller's personal wallet, tagged
with the org id. The owner's default `share_mode` is `public`; a member's default is
`private`. `model_limits` maps to the ordinary API-key model restriction.

ORG-15. `GET /api/dashboard/orgs/{org_id}/keys` returns (a) the caller's keys in the space
with their sharing state and FULL key material and (b) keys usable by the caller (mode
`public`; or `allow` with a share row for the caller; or `deny` without one) including
full key material for copying. Key material in the space is always visible, never
once-only. Each of the caller's own key entries with mode `allow` or `deny` also carries
`shared_with`: the `user_id` list of its current `org_key_shares` rows, so the sharing
editor can present the saved selection.

ORG-16. `PUT /api/dashboard/orgs/{org_id}/keys/{key_id}/sharing {mode:
"private"|"public"|"allow"|"deny", member_ids?}` is restricted to the key's owner.
`allow`/`deny` MUST list only current members; otherwise `400 invalid_request`.

ORG-17. Removing a member (`DELETE /api/dashboard/orgs/{org_id}/members/{user_id}`,
owner only, cannot remove the owner) deletes their membership and share rows and makes
their keys in that space private again.

## 6. Surfaces

ORG-18. `/dashboard/org` is the org space page: org switcher (member's orgs), tabs for
overview (wallet balance, deposit/distribute, invite link card), members (list, remove),
keys (mine + shared to me, create, sharing editor), and the org ledger. Creation and
joining are reachable from the same page.

ORG-19. `/join/{token}` is a centered landing: org avatar, display name, owner username,
member count, and Accept/Decline buttons. Accept enters the space; Decline returns to the
dashboard.

ORG-20. `GET /api/dashboard/orgs` lists the caller's orgs with role, member count, wallet
balance, and (owner only) the invite token and expiry. Admin surfaces exclude `is_org`
rows from the ordinary user groupings.

## 7. Limits

ORG-21. `MAX_ORGS_PER_USER = 2` and `MAX_ORG_MEMBERS = 15` are compile-time constants in
this release; admin adjustment is a later change.

## 8. Ledger

ORG-22. `GET /api/dashboard/orgs/{org_id}/ledger` (any member) returns the org wallet's
`billing_ledger` rows, newest first, at most 200.

## 9. Space analytics and logs

ORG-23. `GET /api/dashboard/orgs/{org_id}/analytics?buckets={1..48}&range_hours={1..720}`
(any member) returns exactly the response shape of `GET /api/dashboard/analytics`, where
every aggregate (bucketed model cost/calls/tokens, provider calls, today and range totals)
is computed over request-log rows whose `api_key_id` belongs to a key with
`api_keys.org_id = {org_id}`. Rows of members' private keys (org_id NULL) are excluded.

ORG-24. `GET /api/dashboard/orgs/{org_id}/request-logs` (any member) accepts the same
query parameters as `GET /api/dashboard/request-logs` (`limit` 1..200, `offset`, `model`,
`status`, `api_key_id`, `search`, `time_from`, `time_to`; `username` is ignored) and
returns the same response shape (`data`, `total`, `total_charge_nano_usd`, `limit`,
`offset`) over the same org-key row set. Non-admin callers get masked error detail under
the same `mask_sensitive_info` setting as the personal log list. A non-member caller
receives `404 not_found` for both endpoints.

ORG-25. `/org/{org_id}` navigation offers Overview, Usage Analysis, Cache Hit Rate, Logs,
Members, Keys, Wallet; the three analytics/logs pages are the workspace pages bound to the
ORG-23/24 endpoints instead of the personal ones.

ORG-26. `/join/{token}` without a session redirects to `/login` carrying the invite path
as return state; a successful login returns to the invite. The return path is accepted
only when it starts with a single `/`.
