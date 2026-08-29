# Dashboard Usage, Marketplace, API Docs, and Redemption Design

## Status

Approved design for implementation on 2026-08-29.

## Scope

This change modifies the authenticated LynShen Console only. It changes the Dashboard home page, adds an authenticated Usage Analysis page, adds an authenticated Model Marketplace page, adds an authenticated API Docs page, changes the brand link, and corrects Store redemption-code access.

The existing request-log page and its route remain unchanged. The public Marketplace and public API Docs remain separate public experiences.

## Visual Brief

- Mood: restrained operational console with clear numeric hierarchy.
- Palette: use the existing semantic tokens. Their reference colors are neutral background `#FAFAFA`, foreground `#171717`, card `#FFFFFF`, border `#E5E5E5`, and primary blue `#2563EB`.
- Typography: use the existing `font-display`, `font-sans-cjk`, and `font-code` tokens. Do not add a font package.
- Layout: use full-width console sections, compact metric groups, one chart surface per analytical question, and no nested decorative cards.
- Motion: use opacity and vertical translation for page entry, progressive chart drawing, numeric count-up, and shared-layout movement for segmented controls. Each transition lasts 180 through 260 milliseconds. Reduced-motion mode removes translation and progressive drawing.

## Route Design

The authenticated router contains these routes under `DashboardLayout`:

- `/dashboard`: Dashboard overview.
- `/dashboard/usage`: Usage Analysis.
- `/dashboard/marketplace`: authenticated Model Marketplace.
- `/dashboard/api-docs`: authenticated API Docs.
- `/dashboard/logs`: existing request logs, unchanged.

The public router keeps `/apidocs` and moves the public Marketplace to `/marketplace`. `/dashboard/marketplace` becomes authenticated and belongs only to `DashboardLayout`. An authenticated navigation action does not leave `DashboardLayout`.

The brand mark and site name in every Dashboard sidebar variant link to `/`. The mobile sheet closes after this navigation.

## Dashboard Home

The Dashboard home keeps the greeting and the four existing overview cards. It removes the Model Data analysis panel and the API Information panel.

The next section is Token Usage. It contains:

- one segmented range control with `24h`, `This week`, and `This month`;
- input tokens;
- cache-read input tokens;
- output tokens;
- total tokens;
- cache-hit rate when the denominator is positive;
- one horizontal token-composition track;
- one compact request or spend trend preview below the token summary.

The selected range changes the analytics request. Token values come from set-based request-log aggregation scoped to the authenticated user, including when that user is an Admin. The page never computes range totals from a truncated request-log page. System-wide Admin analytics remain on `/dashboard/admin`.

When a new response arrives, visible totals count from the previous visible value to the new value. The composition track animates with transforms. The range control uses a shared-layout indicator. The first load renders a shape-matched skeleton and reserves final component dimensions.

## Usage Analysis

The navigation label and page title are `Usage Analysis` in English and `用量分析` in Simplified Chinese.

The page contains model usage analysis only. It does not render a request-log table, log filters, log detail, or a link that changes the page into a log view.

The page contains:

1. A range control for 24 hours, 7 days, and 30 days. The default is 7 days.
2. Four summary values: spend, requests, total tokens, and cache-hit rate.
3. A time-series panel. A metric control switches among spend, requests, and tokens.
4. A model-distribution panel. A measure control switches among spend, requests, and tokens.
5. A ranked model list. Each row shows model name, exact formatted value, percentage, and an accessible progress indicator.

All totals are scoped to the authenticated user. Money remains a signed nano-USD decimal string until display conversion. Token and request totals remain integers. A chart may receive bounded `number` values only for drawing.

Empty data renders zero totals and one shared empty state. A failed request preserves the selected range and renders an inline retry action.

## Authenticated Model Marketplace

The authenticated Marketplace is not a copy of the public Marketplace or the reference site. It uses the Dashboard design system and stays inside `DashboardLayout`.

The page provides:

- model search;
- Group filter;
- capability filter;
- CNY/USD segmented currency control;
- one explicit Group heading for each Group;
- one compact model row per Group and model;
- a modal for Provider offers.

Each model row displays the model ID, capabilities, context limit, input price range, output price range, and offer count. Prices display as `¥x.xx / 1M tokens` in CNY or `$x.xx / 1M tokens` in USD. The UI never exposes nano-USD as a user-facing unit.

The Marketplace and Store share one currency preference and one current exchange-rate snapshot. The preference is in-memory application state and a user action updates both pages without a reload. The rate source, freshness, fallback, exact rational conversion, and rounding follow `store-billing.spec.md`. An unavailable or expired CNY rate disables CNY display and explains the reason; it does not invent a rate.

Selecting a model opens a modal. The modal lists Provider public name, Channel public name, API family, and formatted rate rows. It never exposes internal IDs, Base URLs, credentials, multipliers, or private names.

Search and filters keep the previous result while the next SWR request resolves. Loading additional data reserves space and does not resize the toolbar.

## Authenticated API Docs

The authenticated API Docs page stays inside `DashboardLayout`. It uses authenticated public settings to show the effective API Base URL.

The page has a compact endpoint-family navigation and one content panel. It supports Responses, Chat Completions, Messages, Gemini-compatible Responses, and Image Generations. Each family provides:

- method and path;
- authentication header;
- minimal request fields;
- one request sample for cURL, Python, JavaScript, and Go;
- streaming behavior when supported;
- success response shape;
- common error shape.

Copy actions provide visible success feedback. Examples use environment variables and never render an authenticated user's full API key. A missing Base URL renders an explicit configuration state rather than a broken sample.

## Redemption Codes

Store encryption remains fail closed. Deployment configures a Store `PaymentKeyRing` before code generation or reveal is enabled.

Generation returns every new plaintext code once. The generation dialog remains open after success, replaces the form with the complete codes, and provides Copy All and per-code copy actions. Closing the result removes plaintext from frontend state.

An unused, unexpired v2 code can be revealed or copied only after an Admin reauthentication grant. The response is non-cacheable and the operation is audited.

A v1 code stores only a digest and hint. It cannot be recovered. The UI labels it as legacy and unrecoverable. It offers revoke as the safe action. The system does not fabricate or derive a replacement from the digest.

The current production error `redemption-code encryption is unavailable` is treated as a deployment-key readiness failure. The UI reports that code access is unavailable, and the deployment preflight blocks Store code generation until a valid active key is loaded.

## Data and API Changes

The analytics response is extended with exact token aggregates per time bucket and per model. It includes input, cache-read input, output, and total token counts. Existing analytics fields remain unchanged.

The server obtains token values from canonical request-log columns or the canonical usage breakdown defined by the request-log specification. It uses database aggregation for the requested time range. It does not load all matching request logs into Rust or the browser.

The authenticated Marketplace uses the public allow-listed Marketplace data contract for display data, but it fetches it through an authenticated Dashboard hook and renders it with authenticated controls. Public data never grants access or affects routing authorization.

## Accessibility and Responsive Behavior

- Every segmented control uses buttons or radios with an accessible selected state.
- Every icon-only action has an accessible name and a 44 by 44 CSS-pixel target.
- Charts expose the same values in adjacent text summaries or ranked rows.
- Keyboard focus remains visible in both themes.
- Desktop and mobile layouts do not use a fixed content height.
- At widths below 768 pixels, analytical panels stack in one column.
- Model price rows wrap units without horizontal page overflow.
- Reduced-motion mode preserves final values and removes nonessential movement.

## Testing and Verification

Implementation requires tests that first fail for:

- authenticated route ownership for Usage Analysis, Marketplace, and API Docs;
- unchanged request-log routing;
- Dashboard removal of Model Data and API Information;
- exact token aggregation on SQLite;
- Usage Analysis range and metric mapping;
- Marketplace CNY/USD formatting and shared preference behavior;
- API Docs sample generation and missing Base URL handling;
- brand navigation to `/`;
- redemption generation result visibility;
- v2 reveal with a configured key;
- missing-key readiness failure;
- v1 unrecoverable behavior.

Verification includes frontend tests, targeted Rust tests, `cargo check`, frontend production build, docs build, `git diff --check`, responsive screenshots in English and Simplified Chinese, and a production preflight before deployment.

## Non-Goals

- Do not redesign the request-log page.
- Do not copy the reference site's sidebar or Marketplace.
- Do not add subscription billing behavior.
- Do not expose internal Provider or Channel configuration.
- Do not attempt to decrypt legacy digest-only redemption codes.
