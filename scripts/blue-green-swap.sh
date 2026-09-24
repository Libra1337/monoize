#!/usr/bin/env bash
# Spec: spec/deployment-docker.spec.md and spec/standby-deployment.spec.md.
set -euo pipefail

REV=${1:?usage: blue-green-swap.sh <rev>}
[[ "$REV" =~ ^[a-f0-9]{7,40}$ ]] || exit 1
exec 9>/opt/monoize/blue-green-swap.lock
flock -n 9 || { echo 'another swap is active' >&2; exit 1; }
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
CADDYFILE=/etc/caddy/Caddyfile
DATA_DIR=/opt/monoize/data
BACKUP_DIR=/opt/monoize/backups
ROUTE_STATE=/opt/monoize/blue-green-route.state
TS=$(date -u +%Y%m%dT%H%M%SZ)
LOG=/opt/monoize/swap-$REV-$TS.log
exec > >(tee -a "$LOG") 2>&1
log() { echo "[$(date -u +%H:%M:%S)] $*"; }
die() { log "FAIL: $*"; exit 1; }
source "$SCRIPT_DIR/blue-green-drain.sh"

cleanup_candidate() {
  docker stop -t 30 monoize-next >/dev/null 2>&1 || true
  docker rm monoize-next >/dev/null 2>&1 || true
}

[ -f "$CADDYFILE" ] || die "missing $CADDYFILE"
docker image inspect "monoize:$REV" >/dev/null 2>&1 || die "image monoize:$REV not found"
STABLE=$(grep -oE '127\.0\.0\.1:808[01]' "$CADDYFILE" | head -1 | grep -oE '808[01]$') || die "no stable Caddy upstream"
NREFS=$(grep -oE '127\.0\.0\.1:808[01]' "$CADDYFILE" | sort -u | wc -l)
[ "$NREFS" -eq 1 ] || die "Caddyfile mixes Monoize upstream ports"
CADDY_USER=$(systemctl show caddy --property=User --value)
[ -n "$CADDY_USER" ] || die "Caddy has no explicit service user"
PROXY_UID=$(id -u "$CADDY_USER")
[ "$PROXY_UID" != 1000 ] || die "Caddy and application UIDs must differ"
CADDY_PID=$(systemctl show caddy --property=MainPID --value)
[ "$CADDY_PID" -gt 0 ] || die "Caddy is not running"
curl -fsS --max-time 5 http://127.0.0.1:2019/config/apps/http/servers | python3 -c '
import json,sys
ports=set()
def visit(value):
    if isinstance(value,dict):
        dial=value.get("dial")
        if dial in ("127.0.0.1:8080","127.0.0.1:8081"):
            ports.add(dial.rsplit(":",1)[1])
        for item in value.values(): visit(item)
    elif isinstance(value,list):
        for item in value: visit(item)
visit(json.load(sys.stdin))
sys.exit(ports != {sys.argv[1]})
' "$STABLE" || die "running Caddy configuration disagrees with its stable upstream"
[ "$(stat -c %u "/proc/$CADDY_PID")" = "$PROXY_UID" ] || die "Caddy process UID mismatch"
systemctl is-enabled --quiet monoize-routing.service || die "routing restore unit must be installed and enabled"

docker inspect monoize >/dev/null 2>&1 || die "serving container missing; inspect interrupted swap"
for name in monoize-next monoize-prev; do
  if docker inspect "$name" >/dev/null 2>&1; then
    die "$name exists; preserve it and inspect its traffic before recovery"
  fi
done
[ "$(docker inspect monoize --format '{{.State.Status}}')" = running ] || die "serving container not running"
ACTIVE=$(docker inspect monoize --format '{{range .Config.Env}}{{println .}}{{end}}' | sed -n 's/^MONOIZE_LISTEN=127\.0\.0\.1://p' | tail -n 1)
[[ "$ACTIVE" =~ ^808[01]$ ]] || die "invalid serving listen address"
if [ -f "$ROUTE_STATE" ]; then
  read -r SAVED_STABLE SAVED_ACTIVE SAVED_UID < "$ROUTE_STATE"
  [ "$SAVED_STABLE $SAVED_ACTIVE $SAVED_UID" = "$STABLE $ACTIVE $PROXY_UID" ] || die "routing state disagrees with serving container"
  "$SCRIPT_DIR/blue-green-route.sh" --check "$STABLE" "$ACTIVE" "$PROXY_UID" || die "live routing disagrees with saved state"
else
  [ "$STABLE" = "$ACTIVE" ] || die "missing route state with a non-identity target"
  if iptables -w -t nat -S MONOIZE_BG >/dev/null 2>&1; then die "routing chain exists without state"; fi
fi
CAND=8080; [ "$ACTIVE" = 8080 ] && CAND=8081
LISTENERS=$(ss -ltnH "sport = :$CAND") || die "listener inspection failed"
[ -z "$LISTENERS" ] || die "candidate port occupied"
curl -fsS --max-time 5 "http://127.0.0.1:$ACTIVE/readyz" >/dev/null || die "serving container not ready"
docker exec monoize sh -c "grep -aq 'handing over the store_primary lease' /usr/local/bin/monoize" || die "serving runtime lacks handover support"
SERVING_REV=$(docker inspect monoize --format '{{.Config.Image}}')
SERVING_REV=${SERVING_REV#monoize:}
[[ "$SERVING_REV" =~ ^[a-f0-9]{7,40}$ ]] || die "unknown serving revision"
for source_rev in "$SERVING_REV" "$REV"; do
  [ -d "/opt/monoize/build-$source_rev/src/migration" ] || die "migration sources missing for $source_rev"
done
diff -qr "/opt/monoize/build-$SERVING_REV/src/migration" "/opt/monoize/build-$REV/src/migration" >/dev/null || die "migration differences require compatibility assessment"
RESTART_POLICY=$(docker inspect monoize --format '{{.HostConfig.RestartPolicy.Name}}')
STOP_TIMEOUT=$(docker inspect monoize --format '{{.Config.StopTimeout}}')
if ! [[ "$STOP_TIMEOUT" =~ ^[0-9]+$ ]] || [ "$STOP_TIMEOUT" -lt 15 ]; then STOP_TIMEOUT=15; fi
log "stable=$STABLE active=$ACTIVE candidate=$CAND; Caddy will not reload"

BACKUP="$BACKUP_DIR/monoize-pre-$REV-$TS.db"
python3 - "$DATA_DIR/monoize.db" "$BACKUP" <<'PY' || die "sqlite backup failed"
import sqlite3, sys
src = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True)
dst = sqlite3.connect(sys.argv[2])
src.backup(dst)
result = dst.execute("PRAGMA quick_check").fetchone()[0]
if result != "ok":
    raise RuntimeError(f"backup quick_check failed: {result}")
dst.close()
src.close()
PY
log "verified sqlite backup: $BACKUP"
ENV_FILE=/opt/monoize/monoize-env-$REV.txt
(umask 077; docker inspect monoize --format '{{range .Config.Env}}{{println .}}{{end}}' | sed '/^MONOIZE_LISTEN=/d; /^MONOIZE_BOOT_STANDBY_LEASE=/d; /^MONOIZE_REQUEST_LOG_SPOOL_DIR=/d; /^MONOIZE_DEPLOYMENT_PREVIOUS_URL=/d; /^MONOIZE_DEPLOYMENT_CONTROL_TOKEN=/d' > "$ENV_FILE")
chmod 600 "$ENV_FILE"
SPOOL_NAME=request-log-spool-$REV-$TS
mkdir -m 700 "$DATA_DIR/$SPOOL_NAME"
chown 1000:1000 "$DATA_DIR/$SPOOL_NAME"
TOKEN=$(openssl rand -hex 32)
{
  printf 'MONOIZE_LISTEN=127.0.0.1:%s\n' "$CAND"
  printf 'MONOIZE_BOOT_STANDBY_LEASE=1\n'
  printf 'MONOIZE_REQUEST_LOG_SPOOL_DIR=/app/data/%s\n' "$SPOOL_NAME"
  printf 'MONOIZE_DEPLOYMENT_PREVIOUS_URL=http://127.0.0.1:%s\n' "$ACTIVE"
  printf 'MONOIZE_DEPLOYMENT_CONTROL_TOKEN=%s\n' "$TOKEN"
} >> "$ENV_FILE"
CONTROL_FILE=/opt/monoize/monoize-control-$REV.txt
(umask 077; printf 'header = "Authorization: Bearer %s"\n' "$TOKEN" > "$CONTROL_FILE")
unset TOKEN
control() {
  local method=$1 action=$2 timeout=5
  [ "$action" != pause ] || timeout=0
  curl --silent --show-error --fail --max-time "$timeout" --config "$CONTROL_FILE" \
    -X "$method" "http://127.0.0.1:$CAND/internal/deployment/$action"
}
status_is() {
  python3 -c 'import json,sys; s=json.load(sys.stdin); sys.exit(not(s["mode"]==sys.argv[1] and s["lease_owned"]==(sys.argv[2]=="true")))' "$1" "$2"
}
RESTART_ARGS=()
if [ "$RESTART_POLICY" != no ] && [ -n "$RESTART_POLICY" ]; then RESTART_ARGS=(--restart "$RESTART_POLICY"); fi
docker run -d --name monoize-next --network host --user 1000:1000 \
  -v /opt/monoize/data:/app/data --env-file "$ENV_FILE" \
  --health-cmd "curl -fs http://127.0.0.1:$CAND/healthz >/dev/null || exit 1" \
  --health-interval 30s --health-timeout 5s --health-start-period 10s --health-retries 3 \
  "${RESTART_ARGS[@]}" "monoize:$REV" >/dev/null || { cleanup_candidate; die "candidate start failed"; }
READY=0
for i in $(seq 1 120); do
  ST=$(docker inspect monoize-next --format '{{.State.Status}}')
  [ "$ST" = running ] || { cleanup_candidate; die "candidate not running"; }
  CODE=$(curl -s -o /dev/null -w '%{http_code}' -m 3 "http://127.0.0.1:$CAND/readyz" || true)
  if [ "$CODE" = 200 ]; then READY=1; break; fi
  sleep 1
done
[ "$READY" = 1 ] || { cleanup_candidate; die "candidate readiness failed"; }
if ! control GET status | status_is forwarding false; then
  cleanup_candidate
  die "candidate is not forwarding to the current lease owner"
fi
log "candidate ready with isolated spool and Store forwarding"

PREV_OWNER=$(python3 - "$DATA_DIR/monoize.db" <<'PY'
import sqlite3, sys
c = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True, timeout=5)
row = c.execute("SELECT owner_id FROM store_primary_leases WHERE name='store_primary'").fetchone()
print(row[0] if row else "-")
PY
)
[ -n "$PREV_OWNER" ] && [ "$PREV_OWNER" != - ] || die "cannot identify previous lease owner"
PROBE_STOP="$LOG.probe.stop"
PROBE_READY="$LOG.probe.ready"
python3 "$SCRIPT_DIR/blue-green-probe.py" "$LOG.probe.jsonl" "$PROBE_READY" "$PROBE_STOP" &
PROBE_PID=$!
finish_probe() { touch "$PROBE_STOP"; wait "$PROBE_PID"; }
PAUSE_ATTEMPTED=0
HANDOVER_SENT=0
on_exit() {
  local status=$?
  if [ "$PAUSE_ATTEMPTED" = 1 ] && [ "$HANDOVER_SENT" = 0 ]; then
    control POST resume >/dev/null || log "WARN: forwarding resume needs inspection"
  fi
  touch "$PROBE_STOP"
  wait "$PROBE_PID" || true
  return "$status"
}
trap on_exit EXIT
trap 'exit 143' TERM
trap 'exit 130' INT
for i in $(seq 1 50); do
  [ -f "$PROBE_READY" ] && break
  kill -0 "$PROBE_PID" 2>/dev/null || die "availability probe failed before cutover"
  sleep 0.1
done
[ -f "$PROBE_READY" ] || die "availability probe did not start"

docker rename monoize monoize-prev
printf '%s %s %s\n' "$STABLE" "$CAND" "$PROXY_UID" > "$ROUTE_STATE.$TS"
chmod 644 "$ROUTE_STATE.$TS"
mv "$ROUTE_STATE.$TS" "$ROUTE_STATE"
"$SCRIPT_DIR/blue-green-route.sh" "$STABLE" "$CAND" "$PROXY_UID" || die "route switch failed; both containers retained"
log "new Caddy connections now target :$CAND; existing connections remain on their original instance"
VCODE=$(curl -sk -o /dev/null -w '%{http_code}' -m 8 --resolve api.lynshen.org:443:127.0.0.1 https://api.lynshen.org/readyz || true)
[ "$VCODE" = 200 ] || die "post-switch readiness=$VCODE; both containers retained"
# Continuous public probes could keep reusing an old pooled upstream forever.
# Finish the cutover window before waiting for natural idle-connection expiry.
sleep 2
finish_probe || die "cutover probe recorded failed samples; both containers retained"

drain_health_check() {
  curl -fsS --max-time 5 "http://127.0.0.1:$CAND/readyz" >/dev/null
}
pause_forwarding() {
  PAUSE_ATTEMPTED=1
  control POST pause | status_is paused false
}
resume_forwarding() {
  control POST resume | status_is forwarding false || return 1
  PAUSE_ATTEMPTED=0
}
wait_for_forwarding_drain "$ACTIVE" "${MONOIZE_SWAP_DRAIN_MAX_SECONDS:-14400}" || die "forwarding drain failed; both containers retained"
log "previous connections and forwarded operations drained; handing over Store lease"
HANDOVER_SENT=1
docker kill --signal=SIGHUP monoize-prev >/dev/null || die "handover signal failed; both containers retained"
LEASE_OK=0
for i in $(seq 1 60); do
  if control GET status | status_is local true; then LEASE_OK=1; break; fi
  sleep 1
done
[ "$LEASE_OK" = 1 ] || die "candidate lease acquisition timed out; both containers retained"
CONNECTIONS=$(accepted_upstream_connections "$ACTIVE") || die "final drain check failed"
[ -z "$CONNECTIONS" ] || die "old connections reappeared; retain previous instance"
"$SCRIPT_DIR/blue-green-route.sh" --check "$STABLE" "$CAND" "$PROXY_UID" || die "route changed during drain"
docker stop -t "$STOP_TIMEOUT" monoize-prev >/dev/null
docker rm monoize-prev >/dev/null
docker rename monoize-next monoize
FCODE=$(curl -sk -o /dev/null -w '%{http_code}' -m 8 --resolve api.lynshen.org:443:127.0.0.1 https://api.lynshen.org/readyz || true)
[ "$FCODE" = 200 ] || die "final readiness=$FCODE"
control GET status | status_is local true || die "candidate lease not healthy after finalization"
trap - EXIT
log "SUCCESS: monoize runs monoize:$REV on :$CAND; old connections completed naturally"
