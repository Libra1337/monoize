# Coin Display, Wallet, and Console Navigation Specification

This specification defines the Coin display unit, wallet route ownership, usage ranking ranges, and desktop sidebar behavior.

## Coin display

CN-1. Coin is the only user-visible monetary unit in authenticated Store, Wallet, Usage, Usage Ranking, Dashboard, and Marketplace views.

CN-2. The UI renders Coin with the symbol `C` for all user-visible monetary values in those views. Payment-currency controls MAY show `CNY` or `USD` because they select the provider settlement currency.

CN-3. Wallet and usage amounts originate from internal nano-USD balances. The UI converts them to CNY minor units using the current `cny_per_usd` snapshot and then displays Coin, where `1 C = 1 CNY`.

CN-4. A recharge product priced in CNY displays its CNY amount as Coin. A recharge product priced in USD displays its USD amount multiplied by the current `cny_per_usd` snapshot as Coin. Marketplace model rates retain their source billing basis: a USD-basis rate uses `1 C = 1 USD`, while a CNY-basis rate uses `1 C = 1 CNY`. Conversion MUST occur once at the final display unit.

CN-5. Payment orders retain `CNY` or `USD` as the settlement currency and persist the exchange-rate snapshot. Coin is a display unit and is not sent as a payment-provider currency.

## Usage ranking

CN-6. Authenticated usage ranking requests accept `range=24h`, `range=7d`, or `range=30d`. Missing range selects `24h`; any other value returns HTTP 400.

CN-7. User and model rankings, totals, calls, costs, and current rank MUST be computed from the same selected time window and refreshed together.

CN-8. The authenticated ranking page MUST provide animated range selection for 24h, 7d, and 30d without resetting values through zero during transitions.

## Wallet route

CN-9. `/dashboard/wallet` is an authenticated child page. It contains available balance, current-period usage, current plan, redemption-code entry, and order history.

CN-10. `/dashboard/store` contains only balance top-up and plan purchase flows. It MUST NOT render account summary, redemption entry, or order history.

CN-13. `/api/dashboard/wallet/ledger` MUST require an authenticated user and return at most 50 entries for that user, ordered by `created_at` descending and `id` descending.

CN-14. Each ledger entry MUST include `id`, `kind`, signed `delta_nano_usd`, optional `balance_after_nano_usd`, `meta`, and RFC 3339 `created_at`. The endpoint MUST never return another user's entry.

CN-15. The Wallet page MUST render ledger entries as a chronological balance statement. It MUST distinguish positive and negative deltas, show the entry kind and timestamp, and convert amounts to Coin using the current exchange-rate snapshot.

CN-16. Coin amounts MUST use a graphical Coin mark component. User-facing balance, ledger, Store, Marketplace, Dashboard, and Usage values MUST NOT use a plain `C` character as the currency logo.

CN-17. Redemption input MUST be a single compact action row without a nested card. Order history MUST be available through its own Wallet tab and MUST not be mixed into the ledger list.

## Sidebar

CN-11. The desktop sidebar provides one persistent toggle that animates between expanded and collapsed widths. Collapsed navigation preserves icon actions and exposes labels through tooltips.

CN-12. The expanded/collapsed preference is persisted locally and restored before the first desktop paint. Mobile continues to use the existing sheet navigation.
