# Usage Chart Selection Motion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Animate only the trend Line and model distribution when an authenticated user changes a range or Token metric.

**Architecture:** Add one focused Hook that holds the last rendered dataset while a new selection loads. The Hook marks the next resolved selected dataset as animated, while later polling datasets render without animation. Both chart components consume this state and retain their fixed containers.

**Tech Stack:** React 19, TypeScript, SWR, Recharts, Framer Motion reduced-motion preference, Bun tests, Vite.

---

### Task 1: Define Observable Motion Behavior

**Files:**
- Modify: `spec/dashboard-usage-analysis.spec.md`
- Modify: `frontend/tests/dashboard-experience.test.ts`

- [x] **Step 1: Write failing source-contract assertions**

Require a shared `useSelectionDataset` Hook, `selectionKey` props, a 1,200 ms Line animation, a 1,000 ms Pie animation, polling without animation, and no whole-chart `AnimatePresence`.

- [x] **Step 2: Run the focused test and verify failure**

Run `bun test frontend/tests/dashboard-experience.test.ts` in the cached Linux build environment.
Expected: the motion assertions fail because the current trend chart uses whole-chart `AnimatePresence`.

- [x] **Step 3: Align the product specification**

Replace UA-22a with testable requirements for Line path interpolation, model distribution interpolation, selection-only animation, fixed chart dimensions, and reduced-motion behavior.

### Task 2: Implement Selection-Aware Dataset State

**Files:**
- Create: `frontend/src/hooks/use-selection-dataset.ts`
- Modify: `frontend/src/components/usage/usage-trend-chart.tsx`
- Modify: `frontend/src/components/usage/model-distribution.tsx`
- Modify: `frontend/src/pages/dashboard.tsx`
- Modify: `frontend/src/pages/usage-analysis.tsx`

- [x] **Step 1: Implement the shared Hook**

The Hook accepts `selectionKey`, `loading`, and the next dataset. It keeps the
displayed dataset during a pending selection. When the selected request resolves,
it publishes the new dataset with `animate = true`. For the same selection key,
it publishes polling datasets with `animate = false`.

- [x] **Step 2: Animate only the trend Line**

Remove `AnimatePresence` and whole-chart opacity/translation. Keep the chart
container mounted. Set `Line.isAnimationActive` from the Hook and set
`animationDuration={1200}` with ease-in-out easing.

- [x] **Step 3: Restore model distribution motion**

Add `selectionKey` to `ModelDistribution`. Feed the Hook's displayed ranking to
the Pie and rows. Set Pie animation to 1,000 ms and progress-bar width transition
to 1,000 ms only for an explicit selection change.

- [x] **Step 4: Pass stable selection keys**

Dashboard passes its selected range. Usage Analysis passes `${range}:${metric}`
to both chart components.

### Task 3: Verify And Publish

**Files:**
- Test: `frontend/tests/dashboard-experience.test.ts`

- [x] **Step 1: Run focused frontend tests**

Run `bun test frontend/tests/dashboard-experience.test.ts`.
Expected: all tests pass.

- [x] **Step 2: Run frontend gates**

Run `bun test frontend/tests`, `bunx tsc -b`, and `bun run build` from `frontend`.
Expected: no test failures, no TypeScript errors, and a successful Vite build.

- [x] **Step 3: Run repository checks**

Run `git diff --check`.
Expected: exit code 0.

- [ ] **Step 4: Commit and push**

Commit the spec, Hook, components, pages, and tests. Push `main` to `origin`.

- [ ] **Step 5: Build and deploy the candidate**

Build the Linux candidate with the existing server cache. Run the Rust SQLite
library tests during the build. Test the candidate at `127.0.0.1:18080`, switch
only `monoize.service`, and keep Caddy active.

- [ ] **Step 6: Verify production**

Verify the unit image equals the running image, Monoize is healthy with zero
container restarts, Caddy is active, `https://lynshen.org` returns 200, and
`https://sub.joinreso.com/health` returns 200.
