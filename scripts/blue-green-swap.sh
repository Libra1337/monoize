#!/usr/bin/env bash
# Blue-green container swap for the monoize platform on this host.
# Spec: spec/deployment-docker.spec.md (BG1..BG12, S1..S10).
#
# usage: blue-green-swap.sh <rev>
#   monoize:<rev>  runtime image to activate (must already be built)
#
# Ordering guarantee (BG3): the serving container is never stopped until the
# candidate answers /readyz 200 on the candidate port AND Caddy has reloaded
# onto the candidate AND the post-reload probe through Caddy returned 200.
# Continuity guarantee (BG11/BG12): after the reload the old container hands
# its store_primary lease to the candidate (SIGHUP) and keeps serving its
# in-flight requests; it is stopped only once its upstream connections drain
# to zero. The alert threshold never stops an active connection.

set -euo pipefail

REV=${1:?usage: blue-green-swap.sh <rev>}
[[ "$REV" =~ ^[a-f0-9]{7,40}$ ]] || exit 1
exec 9>/opt/monoize/blue-green-swap.lock
flock -n 9 || { echo 'another swap is active' >&2; exit 1; }
CADDYFILE=/etc/caddy/Caddyfile
DATA_DIR=/opt/monoize/data
BACKUP_DIR=/opt/monoize/backups
TS=$(date -u +%Y%m%dT%H%M%SZ)
LOG=/opt/monoize/swap-$REV-$TS.log

exec > >(tee -a "$LOG") 2>&1

log() { echo "[$(date -u +%H:%M:%S)] $*"; }
die() { log "FAIL: $*"; exit 1; }

cleanup_candidate() { docker rm -f monoize-next >/dev/null 2>&1 || true; }

# ---------------------------------------------------------------- S1 pre-state
[ -f "$CADDYFILE" ] || die "missing $CADDYFILE"
docker image inspect "monoize:$REV" >/dev/null 2>&1 || die "image monoize:$REV not found"

ACTIVE=$(grep -oE '127\.0\.0\.1:808[01]' "$CADDYFILE" | head -1 | grep -oE '808[01]$') \
  || die "no 127.0.0.1:808x upstream in $CADDYFILE"
NREFS=$(grep -oE '127\.0\.0\.1:808[01]' "$CADDYFILE" | sort -u | wc -l)
[ "$NREFS" -eq 1 ] || die "Caddyfile mixes multiple upstream ports; refusing"
CAND=8080; [ "$ACTIVE" = 8080 ] && CAND=8081
log "active port=$ACTIVE candidate port=$CAND"

# A leftover may still serve a long request; recovery must not guess.
docker inspect monoize >/dev/null 2>&1 || die "serving container missing; inspect interrupted swap"
for name in monoize-next monoize-prev; do
  if docker inspect "$name" >/dev/null 2>&1; then
    die "$name exists; preserve it and inspect its traffic before recovery"
  fi
done
[ "$(docker inspect monoize --format '{{.State.Status}}')" = running ] || die "serving container not running"
SERVING_PORT=$(docker inspect monoize --format '{{range .Config.Env}}{{println .}}{{end}}' | sed -n 's/^MONOIZE_LISTEN=127\.0\.0\.1://p')
[ "$SERVING_PORT" = "$ACTIVE" ] || die "serving port disagrees with Caddy"
[ "$(ss -ltnH "sport = :$CAND" | wc -l)" -eq 0 ] || die "candidate port occupied"
curl -fsS --max-time 5 "http://127.0.0.1:$ACTIVE/readyz" >/dev/null || die "serving container not ready"
docker exec monoize sh -c "grep -aq 'handing over the store_primary lease' /usr/local/bin/monoize" || die "serving runtime lacks handover support"
SERVING_REV=$(docker inspect monoize --format '{{.Config.Image}}')
SERVING_REV=${SERVING_REV#monoize:}
[[ "$SERVING_REV" =~ ^[a-f0-9]{7,40}$ ]] || die "unknown serving revision"
for source_rev in "$SERVING_REV" "$REV"; do
  [ -d "/opt/monoize/build-$source_rev/src/migration" ] || die "migration sources missing for $source_rev"
done
diff -qr "/opt/monoize/build-$SERVING_REV/src/migration" "/opt/monoize/build-$REV/src/migration" >/dev/null || die "migration differences require compatibility assessment"
source "$(dirname "${BASH_SOURCE[0]}")/blue-green-drain.sh"
wait_for_proxy_upgrades() {
  local profile
  while :; do
    profile=$(curl -fsS --max-time 5 'http://127.0.0.1:2019/debug/pprof/goroutine?debug=1') || return 1
    [[ "$profile" == 'goroutine profile: total '* ]] || return 1
    if ! grep -q 'reverseproxy.(\*Handler).handleUpgradeResponse' <<< "$profile"; then return; fi
    log "waiting for existing Caddy upgraded connections before reload"
    sleep 15
  done
}

RESTART_POLICY=$(docker inspect monoize --format '{{.HostConfig.RestartPolicy.Name}}')
# Config.StopTimeout renders as the literal string "<nil>" when unset.
STOP_TIMEOUT=$(docker inspect monoize --format '{{.Config.StopTimeout}}')
if [ -z "$STOP_TIMEOUT" ] || [ "$STOP_TIMEOUT" = "0" ] || [ "$STOP_TIMEOUT" = "<nil>" ]; then STOP_TIMEOUT=15; fi
[ "$STOP_TIMEOUT" -ge 15 ] || STOP_TIMEOUT=15
log "serving container restart-policy=$RESTART_POLICY stop-timeout=${STOP_TIMEOUT}s"

# ----------------------------------------------------------------- S2 backup
BTS=$(date -u +%Y%m%d-%H%M%S)
BACKUP="$BACKUP_DIR/monoize-pre-$REV-$BTS.db"
python3 - "$DATA_DIR/monoize.db" "$BACKUP" <<'PY' || die "sqlite backup failed"
import sqlite3, sys
src, dst = sys.argv[1], sys.argv[2]
s = sqlite3.connect(f"file:{src}?mode=ro", uri=True)
d = sqlite3.connect(dst)
s.backup(d)
result = d.execute("PRAGMA quick_check").fetchone()[0]
if result != "ok":
    raise RuntimeError(f"backup quick_check failed: {result}")
d.close(); s.close()
PY
log "sqlite backup -> $BACKUP ($(du -h "$BACKUP" | cut -f1))"

# -------------------------------------------------------------- S3 env capture
ENV_FILE=/opt/monoize/monoize-env-$REV.txt
(umask 077; docker inspect monoize --format '{{range .Config.Env}}{{println .}}{{end}}' > "$ENV_FILE")
chmod 600 "$ENV_FILE"
log "captured $(wc -l < "$ENV_FILE") env entries"

# -------------------------------------------------------------- S4 candidate
RESTART_FLAG=""
if [ "$RESTART_POLICY" != "no" ] && [ -n "$RESTART_POLICY" ]; then RESTART_FLAG="--restart $RESTART_POLICY"; fi
docker run -d --name monoize-next \
  --network host --user 1000:1000 \
  -v /opt/monoize/data:/app/data \
  --env-file "$ENV_FILE" \
  -e MONOIZE_LISTEN="127.0.0.1:$CAND" \
  -e MONOIZE_BOOT_STANDBY_LEASE=1 \
  --health-cmd "curl -fs http://127.0.0.1:$CAND/healthz >/dev/null || exit 1" \
  --health-interval 30s --health-timeout 5s --health-start-period 10s --health-retries 3 \
  $RESTART_FLAG \
  "monoize:$REV" >/dev/null || { cleanup_candidate; die "docker run candidate failed"; }
log "candidate monoize-next started on 127.0.0.1:$CAND (standby lease boot)"

# ---------------------------------------------------------- S5 readiness gate
READY=0
for i in $(seq 1 120); do
  ST=$(docker inspect monoize-next --format '{{.State.Status}}')
  [ "$ST" = running ] || { docker logs monoize-next --tail 30; cleanup_candidate; die "candidate not running (state=$ST)"; }
  CODE=$(curl -s -o /dev/null -w '%{http_code}' -m 3 "http://127.0.0.1:$CAND/readyz" || true)
  if [ "$CODE" = 200 ]; then READY=1; break; fi
  sleep 1
done
[ "$READY" = 1 ] || { docker logs monoize-next --tail 30; cleanup_candidate; die "candidate /readyz never reached 200 in 120 polls"; }
log "candidate ready (loop ${i}s). 'Store Primary lease' warnings during overlap are expected (BG9)."

# The serving runtime may predate the renewal/handover race fix. Defer cutover
# until it is idle, then check again before signaling after the proxy switch.
log "candidate staged; waiting without a deadline for the serving connections to finish before cutover"
while :; do
  wait_for_upstream_drain "$ACTIVE" "${MONOIZE_SWAP_DRAIN_MAX_SECONDS:-14400}" || die "connection inspection failed; both containers retained"
  wait_for_proxy_upgrades || die "Caddy connection inspection failed; serving configuration unchanged"
  CONNECTIONS=$(ss -tnH state established "( sport = :$ACTIVE )") || die "connection inspection failed"
  [ -z "$CONNECTIONS" ] && break
  sleep 15
done
curl -fsS --max-time 5 "http://127.0.0.1:$CAND/readyz" >/dev/null || die "candidate lost readiness while waiting; serving container unchanged"
log "serving connections drained; proceeding with cutover"

# ---------------------------------------------------- S6 rename + S7 caddy flip
PREV_OWNER=$(python3 - "$DATA_DIR/monoize.db" <<'PY'
import sqlite3, sys
c = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True, timeout=5)
row = c.execute("SELECT owner_id FROM store_primary_leases WHERE name='store_primary'").fetchone()
print(row[0] if row else "-")
PY
)
[ -n "$PREV_OWNER" ] && [ "$PREV_OWNER" != "-" ] || die "cannot identify serving lease owner; cutover aborted"
log "pre-swap lease owner verified"
PROBE_STOP="$LOG.probe.stop"
PROBE_READY="$LOG.probe.ready"
python3 "$(dirname "${BASH_SOURCE[0]}")/blue-green-probe.py" "$LOG.probe.jsonl" "$PROBE_READY" "$PROBE_STOP" &
PROBE_PID=$!
finish_probe() {
  touch "$PROBE_STOP"
  wait "$PROBE_PID"
}
trap 'touch "$PROBE_STOP"; wait "$PROBE_PID" || true' EXIT
for i in $(seq 1 50); do
  [ -f "$PROBE_READY" ] && break
  kill -0 "$PROBE_PID" 2>/dev/null || die "availability probe failed before cutover"
  sleep 0.1
done
[ -f "$PROBE_READY" ] || die "availability probe did not start"

docker rename monoize monoize-prev
cp "$CADDYFILE" "$CADDYFILE.pre-swap-$TS"
sed -i "s/127\.0\.0\.1:$ACTIVE/127.0.0.1:$CAND/g" "$CADDYFILE"
if ! caddy validate --config "$CADDYFILE" --adapter caddyfile >/dev/null 2>&1; then
  cp "$CADDYFILE.pre-swap-$TS" "$CADDYFILE"
  docker rename monoize-prev monoize
  cleanup_candidate
  die "caddy validate rejected the edited Caddyfile (restored)"
fi
if ! wait_for_proxy_upgrades; then
  cp "$CADDYFILE.pre-swap-$TS" "$CADDYFILE"
  docker rename monoize-prev monoize
  cleanup_candidate
  die "Caddy connection inspection failed before reload; serving configuration restored"
fi
systemctl reload caddy || die "caddy reload failed; retain both containers and inspect the active configuration"

log "caddy reloaded: upstream 127.0.0.1:$ACTIVE -> 127.0.0.1:$CAND"

# ------------------------------------------------------------ S8 post-reload verify
VCODE=$(curl -sk -o /dev/null -w '%{http_code}' -m 8 \
  --resolve api.lynshen.org:443:127.0.0.1 https://api.lynshen.org/readyz || true)
if [ "$VCODE" != 200 ]; then
  die "post-reload /readyz=$VCODE; both containers retained; no second reload while connections may exist"
fi

log "post-reload verification 200 via caddy"

# ------------------------------------------ S9a store lease handover via SIGHUP (BG11)
# The old container voluntarily deletes its store_primary row and keeps
# serving established requests; the standby loop on the candidate takes the
# lease within seconds, so store surfaces stay available during the drain.
# Runtime capability check: a pre-BG11 binary would DIE on SIGHUP (default
# disposition), which would cut every in-flight request — never signal it.
wait_for_upstream_drain "$ACTIVE" "${MONOIZE_SWAP_DRAIN_MAX_SECONDS:-14400}" || die "connection inspection failed; both containers retained"
LEASE_OK=0
if docker exec monoize-prev sh -c \
     "grep -aq 'handing over the store_primary lease' /usr/local/bin/monoize" 2>/dev/null; then
  docker kill --signal=SIGHUP monoize-prev >/dev/null
  for i in $(seq 1 60); do
    OWNER=$(python3 - "$DATA_DIR/monoize.db" <<'PY' || true
import sqlite3, sys
c = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True, timeout=5)
row = c.execute("SELECT owner_id FROM store_primary_leases WHERE name='store_primary'").fetchone()
print(row[0] if row else "-")
PY
)
    if [ -n "$OWNER" ] && [ "$OWNER" != "-" ] && [ "$OWNER" != "$PREV_OWNER" ]; then LEASE_OK=1; break; fi
    sleep 1
  done
  if [ "$LEASE_OK" = 1 ]; then
    log "store_primary lease handed over to new owner after ${i}s (BG11); old container keeps draining in-flight requests"
  else
    die "lease handover timed out; both containers retained"
  fi
else
  die "handover capability missing; both containers retained"
fi

# --------------------------------------- S9b drain old-upstream connections (BG12)
wait_for_upstream_drain "$ACTIVE" "${MONOIZE_SWAP_DRAIN_MAX_SECONDS:-14400}" || die "connection inspection failed; both containers retained"

# ---------------------------------------------------------------- S9 finalize
docker stop -t "$STOP_TIMEOUT" monoize-prev >/dev/null
docker rm monoize-prev >/dev/null
docker rename monoize-next monoize

# ---------------------------------------------------------------- S10 final
FCODE=$(curl -sk -o /dev/null -w '%{http_code}' -m 8 \
  --resolve api.lynshen.org:443:127.0.0.1 https://api.lynshen.org/readyz || true)
[ "$FCODE" = 200 ] || die "final /readyz=$FCODE on new active port $CAND — check monoize logs"
IMAGE=$(docker inspect monoize --format '{{.Config.Image}}')
HEALTH=$(docker inspect monoize --format '{{.State.Health.Status}}' 2>/dev/null || echo n/a)
finish_probe || die "cutover probe recorded failed samples; inspect $LOG.probe.jsonl"
trap - EXIT
log "SUCCESS: monoize now runs $IMAGE on 127.0.0.1:$CAND (health=$HEALTH)."
log "Next deploy will alternate back to port $(( CAND == 8080 ? 8081 : 8080 ))."
