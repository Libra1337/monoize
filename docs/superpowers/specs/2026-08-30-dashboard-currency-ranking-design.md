# Dashboard Currency And Usage Ranking Design

## Scope

The authenticated sidebar exposes Usage Ranking to every enabled role. The compatibility
route `/dashboard/admin/usage` remains Admin-only. The account menu adds one CNY/USD display
preference row between Settings and Theme.

## State And Data Flow

`StoreCurrencyProvider` remains the single owner of display currency. It restores one valid
value from `localStorage`, defaults to CNY, and persists later selections. Storage failures do
not block the in-memory update. Store, Marketplace, Dashboard overview, account menu, and usage
ranking consume the same Context value.

All CNY conversions reuse `/api/dashboard/store/exchange-rate` through one SWR cache key. USD
does not require an exchange-rate response. Exact integer helpers perform conversion. The UI
does not mutate billing records or checkout settlement currency until a Store checkout action
explicitly creates an order.

## Interaction

The currency row uses two native buttons in one rounded segmented control. A shared motion
indicator moves between CNY and USD. Each button exposes `aria-pressed`. The global reduced
motion preference disables movement. Loading money surfaces reserve their dimensions with
Skeletons. A missing CNY rate renders an unavailable marker.

The shared Usage Trend line does not use Recharts path interpolation. An explicit metric or
range selection remounts one keyed motion wrapper and fades the new chart upward over 450 ms.
Polling retains that key and updates the path without replaying the transition. Rapid selection
cannot retarget an unfinished line path.

## Verification

Source-level frontend tests verify the ordinary-user route and navigation, the persisted
Context, the account-menu control, and all four locales. Money helper tests verify exact CNY
and USD conversion. The chart source test verifies explicit selection transitions and disabled
Recharts path interpolation.
TypeScript, Bun tests, the Vite production build, Rust SQLite tests, and `git diff --check` must
pass before deployment.
