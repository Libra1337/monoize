# Usage Chart Selection Motion Design

## Scope

This design changes motion on the Dashboard Token Usage view and the Usage
Analysis page. It does not change analytics data, polling intervals, chart
dimensions, colors, or accessibility labels.

## Trend Line Motion

The trend chart keeps its Card, title, axes, grid, tooltip, and dimensions in
place when the user changes the time range or Token metric.

The chart does not fade or translate as one block. The Line path changes from
the previous series to the selected series. Recharts performs the Line path
animation for 1,200 milliseconds with an ease-in-out timing function.

The first resolved chart may draw once. A user selection starts one Line path
animation. A 2-second SWR polling response for an unchanged selection updates
the values without starting another selection animation.

If the selected range is still loading, the chart keeps the last resolved
series. The new Line path animation starts after the selected response resolves.

## Model Distribution Motion

The model distribution keeps its Card and dimensions in place. When the user
changes the time range or Token metric, the donut sectors animate to the new
model shares for 1,000 milliseconds. Each model progress bar changes to its new
width over the same interval.

The model names and exact Token values remain visible during the transition.
A polling response for an unchanged selection updates the distribution without
restarting the selection animation.

## Reduced Motion

When the operating system requests reduced motion, the trend Line, donut
sectors, and progress bars render their final values immediately.

## State Model

The Usage Analysis page supplies one explicit `selectionKey` derived from the
selected range and metric. The Dashboard supplies one key derived from its
selected range and the fixed `total` metric.

Each chart stores the last completed selection key. A key change enters the
`selection_pending` state. The state remains pending while SWR exposes the
previous response. It changes to `selection_animating` when data for the new
selection resolves. It returns to `idle` after the motion interval.

Polling data received in the `idle` state does not enter
`selection_animating`.

## Verification

Automated tests must verify these conditions:

1. The trend Line enables animation only for an explicit selection change.
2. The trend Line animation duration equals 1,200 milliseconds.
3. The trend chart does not use whole-chart `AnimatePresence` transitions.
4. The model Pie and progress bars animate for 1,000 milliseconds after an
   explicit selection change.
5. A polling update with an unchanged selection does not restart either motion.
6. Reduced-motion mode disables all three animations.
7. TypeScript compilation and the frontend production build pass.
