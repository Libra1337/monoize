# Continuous deployment implementation plan

> **For agentic workers:** Use subagent-driven-development. Complete each task and review its result before deployment.

**Goal:** Deploy audited upstream fixes on Monoize-Claude while every existing production connection can complete naturally.

**Architecture:** New Caddy TCP connections select the ready candidate through a UID- and SYN-restricted loopback NAT target. Existing connections retain their destination. The candidate delegates Store operations until the old instance drains, then pauses forwarding and receives the lease.

**Tech Stack:** Rust/Axum, SQLite/SeaORM, Docker, Linux conntrack, Bash, Python, Bun.

**Spec:** spec/deployment-docker.spec.md and spec/standby-deployment.spec.md.

## Global constraints

- Do not restart or reload Caddy during the swap.
- Do not signal or stop an instance with accepted connections.
- Do not initialize another instance's live request-log spool.
- Do not introduce a migration incompatible with the old instance.
- Preserve the original checkout's staged and unstaged changes.
- Push the final commit to Monoize-Claude before deleting other fork branches.

## Review focus

- Removing the last NAT expression can unregister hooks and break mapped flows.
- Store forwarding must retain original body, query, auth, and trusted client identity.
- Failed socket queries and missing lease ownership must retain both instances.
- HTTP keepalive and WebSocket connections must stay on the original instance.
- SQLite write transactions must acquire the write lock before reading quota state.

## Task 1: Upstream adaptation

- [x] Apply image MIME detection and Safari composer sizing fixes against local specs.
- [x] Add behavior regressions and update described behavior in four documentation locales.
- [x] Document existing Codex endpoint aliases in four locales.
- [x] Run targeted browser, frontend, image, and docs checks.

## Task 2: Application overlap support

- [x] Implement spec/standby-deployment.spec.md in a focused Rust module.
- [x] Forward existing Store mutation, callback, and internal Replica routes exactly once.
- [x] Add authenticated pause/resume/status control and body-lifetime drain accounting.
- [x] Defer singleton workers and keep per-process request-log flushing active.
- [x] Verify gate ordering and byte-preserving forwarding with real loopback servers.
- [x] Verify independent SQLite pool admission and fix the write-lock acquisition boundary.

## Task 3: Deployment script

- [x] Implement scripts/blue-green-route.sh and scripts/monoize-routing.service.
- [x] Validate the route in a separate Linux namespace with preexisting and translated flows.
- [x] Rewrite scripts/blue-green-swap.sh to isolate spool and switch immediately after readiness.
- [x] Pause forwarding only after old drain, recheck sockets, then hand over the lease.
- [x] Run shell tests, route integration tests, and an independent deployment review.

## Task 4: Delivery

- [x] Run the changed Rust, frontend, browser, and docs checks.
- [ ] Commit, preserve original dirty patches/index/stash, and fast-forward Monoize-Claude.
- [ ] Push and verify the remote revision; restore the original staged/unstaged state.
- [ ] Detach the audit worktree and remove other local/origin branches after reachability checks.
- [ ] Build the final image with limited resources and existing build caches.
- [ ] Install project deployment scripts and run the supervised swap.
- [ ] Verify route/health/probes; retain old instances until natural drain completes.
