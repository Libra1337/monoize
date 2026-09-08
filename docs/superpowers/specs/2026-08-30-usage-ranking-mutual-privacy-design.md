# Usage Ranking Mutual Privacy Design

## Scope

This design adds one user-controlled identity preference to the authenticated usage ranking. It also aligns the Dashboard status refresh mark with the status title.

## Privacy state

Each user stores `usage_ranking_anonymous` as a Boolean. New and migrated users use `true`.

An administrator always receives the target user's identifier, username, cost, and model rows.

An ordinary viewer receives the target username only when one condition is true:

- the target is the viewer; or
- both the viewer and the target have `usage_ranking_anonymous = false`.

If neither condition is true, the response omits the target username and identifier. The row remains selectable and includes its model rows without cost.

## Settings

The personal settings page shows one optimistic Switch. The Switch is on when the user is anonymous. Saving failure restores the previous value and shows an error.

## Ranking presentation

The user-ranking heading uses the locale equivalent of `Current ranking`. The current user's row shows the current username. Other rows show either the allowed username or the locale's anonymous label. Every row with model data opens the existing model-detail dialog. Each user and model row shows compact blue input, orange cache-read, and green output counts below the total Token value.

The public site navigation contains a public usage-ranking page. It defaults to 24 hours and offers 7-day and 30-day windows. Public visitors receive aggregate totals, anonymous user rows, and global model rows. A public response never contains a user ID or username because a visitor cannot satisfy mutual disclosure.

## Status header

The Dashboard status page places the refresh mark in the same full-width header row as the title block. The mark aligns to the upper-right edge. The public status route does not render the mark.
