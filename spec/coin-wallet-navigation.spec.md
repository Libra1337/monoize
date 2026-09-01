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

## Sidebar

CN-11. The desktop sidebar provides one persistent toggle that animates between expanded and collapsed widths. Collapsed navigation preserves icon actions and exposes labels through tooltips.

CN-12. The expanded/collapsed preference is persisted locally and restored before the first desktop paint. Mobile continues to use the existing sheet navigation.
