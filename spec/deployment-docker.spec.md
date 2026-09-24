# Docker Deployment Specification (production host 64.90.22.212)

## 0. Scope

- This specification defines the runtime deployment procedure of the `monoize`
  platform container on the production host `64.90.22.212` (Debian 12).
- It supersedes `spec/deployment-watchdog.spec.md` on the production host: that
  file governs the repository-local PM2 `deploy.sh`, which is not used on this
  host. The production host runs the platform as Docker container `monoize`
  behind a host-level Caddy reverse proxy.
- Persistent state: SQLite database at `/opt/monoize/data/monoize.db`, mounted
  at `/app/data`. Source trees land in `/opt/monoize/build-<rev>/`; pre-swap
  backups land in `/opt/monoize/backups/`.

## 1. Definitions

- **Serving container**: the container currently named `monoize`. It holds the
  port that the Caddy active upstream names (Section 2).
- **Candidate container**: the container named `monoize-next` started by a swap
  run. It must bind only the candidate port.
- **Active port** `P_active`: the TCP port that the `www.lynshen.org` and
  `api.lynshen.org` upstream directives in `/etc/caddy/Caddyfile` reference.
- **Candidate port** `P_candidate`: the single element of `{8080, 8081}` that is
  not `P_active`.
- **Swap script**: `/opt/monoize/blue-green-swap.sh` on the production host,
  maintained in `scripts/blue-green-swap.sh` and invoked as `blue-green-swap.sh <rev>` where `monoize:<rev>` is the runtime
  image to activate.

## 2. Invariants

BG1. Both upstream ports MUST be loopback-only. Containers MUST bind
`MONOIZE_LISTEN=127.0.0.1:<port>`; `0.0.0.0` binds are forbidden for new
containers.

BG2. Port alternation. A swap run MUST compute `P_active` by reading
`/etc/caddy/Caddyfile` and MUST run the candidate on `P_candidate`. The steady
state after a successful swap has exactly one container (`monoize`) listening
on the new `P_active`.

BG3. Zero-downtime ordering. The swap script MUST NOT send a stop signal to
the serving container until all of the following hold:

1. the candidate container is running;
2. `GET http://127.0.0.1:<P_candidate>/readyz` has returned HTTP 200;
3. `/etc/caddy/Caddyfile` upstreams reference `127.0.0.1:<P_candidate>`;
4. `caddy validate` has accepted the edited Caddyfile;
5. Caddy has reloaded (`systemctl reload caddy` or `caddy reload`) successfully;
6. `GET https://127.0.0.1/readyz` with `Host: api.lynshen.org` (TLS SNI
   bypassed) has returned HTTP 200 after the reload.

BG4. Backup precondition. The swap script MUST complete a SQLite online backup
(`sqlite3` `conn.backup()` via python3) into `/opt/monoize/backups/` with a
filename carrying the source revision and UTC timestamp, BEFORE starting the
candidate container.

BG5. Environment fidelity. The candidate container MUST run with the
environment captured from the serving container via
`docker inspect --format '{{range .Config.Env}}'`, plus the
`MONOIZE_LISTEN=127.0.0.1:<P_candidate>` override, plus the same
`--network host --user 1000:1000 -v /opt/monoize/data:/app/data` run flags and
restart policy as the serving container.

BG5a. Previous deployments may have captured duplicate environment entries.
Read the final `MONOIZE_LISTEN` entry for the serving-port check. Remove all
`MONOIZE_LISTEN` and `MONOIZE_BOOT_STANDBY_LEASE` entries from the captured
env file before supplying the candidate's explicit overrides. The candidate
MUST have exactly one entry for each overridden variable.

BG6. Failure containment. If any step before the Caddy reload fails (image
build, backup, candidate start, readiness timeout), the script MUST:
1. stop and remove the candidate container (if any);
2. leave the serving container and the Caddyfile unmodified;
3. exit nonzero.
After attempting Caddy reload, a reload or post-reload verification failure MUST
retain both containers and exit nonzero. Do not automatically reload a second
time: the first attempt may have admitted upgraded connections. Inspect the
active proxy configuration before any manual rollback.

BG7. Final state. After a successful swap the host MUST satisfy all of:

1. exactly one container named `monoize` is running, image `monoize:<rev>`;
2. no container named `monoize-next` or `monoize-prev` exists;
3. `/etc/caddy/Caddyfile` references `127.0.0.1:<P_candidate-of-this-run>` in
   every `reverse_proxy` upstream (the `www` and `api` blocks and the
   `http://127.0.0.1:8095` local-entry block);
4. `GET /readyz` through Caddy returns HTTP 200.

BG8. Graceful stop. The serving container MUST be stopped with
`docker stop` (SIGTERM). The stop grace period MUST be at least 15 seconds. The connection count
MUST already be zero before SIGTERM.

BG9. Lease overlap via standby boot. The swap script MUST run the candidate
with `MONOIZE_BOOT_STANDBY_LEASE=1` so a startup `store_primary` acquisition
failure enters standby per SB-HA-4D-1 of `store-billing.spec.md` instead of
crash-looping. The candidate receives no production traffic before the Caddy
reload, so the serving container remains the only Store writer during the
overlap. After the serving container stops, its lease expires within the
15-second TTL and the candidate acquires it on its retry loop.

BG10. Stable local entry for same-host consumers. `/etc/caddy/Caddyfile` MUST
contain a loopback-only site block `http://127.0.0.1:8095` whose
`reverse_proxy` names the same active upstream as the `www`/`api` blocks. Any
component that must call the platform from the same host (for example the
Apeiron bridge, `APEIRON_BRIDGE_URL`) MUST target `http://127.0.0.1:8095`,
never a direct `127.0.0.1:808x` upstream port. Direct upstream references in
the Caddyfile (`127.0.0.1:8080` / `127.0.0.1:8081`) may appear only inside
`reverse_proxy` directives; the S7 port rewrite applies to every such
occurrence.

BG11. Store-lease handover on SIGHUP. The platform MUST install a SIGHUP
handler whose first delivery sets a handover flag. The `store_primary` lease
renewal loop MUST check the flag each tick: when set, the holder MUST delete
its own lease row (`DELETE ... WHERE name AND owner_id AND epoch` match its own
values), mark renewal as failed, stop renewing, and the process MUST keep
serving established requests (SIGHUP MUST NOT trigger process shutdown or the
HTTP drain). The swap script MUST send SIGHUP to the previous container only
after BG3.6 holds and only after proving the runtime supports BG11 (the
running binary contains the handover log marker; a pre-BG11 runtime MUST NOT
be signaled, because default SIGHUP disposition would kill it), then MUST poll
the lease owner until it differs from the pre-swap owner, for at most 60 polls with a one-second interval.

BG11a. If the handover flag becomes true during a lease renewal retry round, the
renewal loop MUST process handover before reacting to that round's failure. That
failure MUST NOT set the application shutdown flag. The previous process MUST keep
serving established requests while its lease is released or expires.

BG12. Unbounded connection drain. The swap script MUST count established TCP
sockets whose local port equals the previous instance's listening port every
15 seconds. It MUST NOT send SIGHUP or stop the previous container while that
count is nonzero. `MONOIZE_SWAP_DRAIN_MAX_SECONDS` (default 14400) is an alert
threshold only. Crossing it MUST log the remaining connection count and continue
waiting without a stop signal. A failed socket query MUST retain both containers.

BG13. The swap MUST hold an exclusive `/opt/monoize/blue-green-swap.lock`.
Reject overlapping swaps and preexisting `monoize-next` or `monoize-prev`
containers without stopping or removing them. Run the script in a host supervisor
that survives SSH disconnection. After any attempted Caddy reload, failure MUST
retain both containers until their respective connections have drained.

BG14. Before candidate startup, require identical migration source trees for the
serving and candidate revisions. A difference requires a separate compatibility
assessment. Verify the online backup with `PRAGMA quick_check` before continuing.

BG15. After candidate readiness, wait without a deadline for zero accepted
connections on the previous instance before changing the Caddyfile. During this
wait all production traffic MUST continue to use the previous instance. The
candidate MUST remain in standby mode. Check candidate readiness again after
waiting. After reload, wait again for zero previous connections before SIGHUP.
This protects an old runtime that does not contain the BG11a renewal-race fix.

BG16. Before Caddy reload, query its goroutine profile and wait while
`reverseproxy.(*Handler).handleUpgradeResponse` is present. An unavailable or
unrecognized profile MUST abort before reload. This is a preflight observation,
not an atomic exclusion against an upgrade arriving between the check and reload.

## 3. Swap procedure (normative sequence)

S1. Preconditions: `monoize:<rev>` image exists (built from
`/opt/monoize/build-<rev>/image-out`); serving container `monoize` is running
and healthy; neither `monoize-next` nor `monoize-prev` exists; and the exclusive swap lock
is held. Validate migration source identity per BG14.

S2. SQLite online backup per BG4.

S3. Capture the serving container environment per BG5 into
`/opt/monoize/monoize-env-<rev>.txt` (mode 0600).

S4. Start candidate: `docker run -d --name monoize-next --network host
--user 1000:1000 -v /opt/monoize/data:/app/data --env-file
/opt/monoize/monoize-env-<rev>.txt -e MONOIZE_LISTEN=127.0.0.1:<P_candidate>
-e MONOIZE_BOOT_STANDBY_LEASE=1
--restart unless-stopped monoize:<rev>`, with a `--health-cmd` override that
probes `/healthz` on the candidate port (the image default hardcodes 8080).

S5. Readiness gate: poll `http://127.0.0.1:<P_candidate>/readyz` at most 120 times,
with a three-second request timeout and a one-second interval. Require HTTP 200.

S6. Wait for BG15 and BG16. Rename `docker rename monoize monoize-prev`
(does not disturb its socket).

S7. Edit `/etc/caddy/Caddyfile`: replace every upstream occurrence of
`127.0.0.1:<P_active>` with `127.0.0.1:<P_candidate>`. Keep a timestamped
copy of the pre-edit file. Run `caddy validate --config /etc/caddy/Caddyfile`,
then reload Caddy.

S8. Post-reload verification per BG3.6.

S9. Wait for zero old accepted connections per BG12, then perform store-lease
handover per BG11: capability probe, SIGHUP to `monoize-prev`,
poll the lease owner change (at most 60 seconds). Skipped with a log line when
the previous runtime predates BG11.

S10. Connection drain per BG12: poll established connections on the old active
port until zero. The alert threshold MUST NOT stop the previous container.

S11. `docker stop monoize-prev && docker rm monoize-prev`, then
`docker rename monoize-next monoize`. S9 MUST already have confirmed that the
candidate owns the lease before the previous container is stopped.

S12. Final verification per BG7 and print the new active port.

## 4. Verification of zero downtime

V1. After the idle gate opens and through final verification, an origin HTTPS
probe issuing
`GET https://www.lynshen.org/` (or the origin-direct equivalent with
`--resolve www.lynshen.org:443:64.90.22.212`) at an interval of at most 500 ms
MUST observe zero non-2xx responses and zero connection failures. Start it before
Caddy reload and require its first sample to succeed. Record every HTTP status
and connection exit code. The pre-cutover idle wait MUST NOT continuously probe
the previous HTTP port because that can keep idle upstream connections alive.
