#!/usr/bin/env bash

wait_for_upstream_drain() {
  local port=$1 alert_seconds=$2 started elapsed connections now alerted=0
  [[ "$port" =~ ^808[01]$ && "$alert_seconds" =~ ^[0-9]+$ ]] || return 1
  started=$(date +%s) || return 1
  while :; do
    # A failed socket query must not look like an empty result.
    connections=$(ss -tnH state established "( sport = :$port )") || return 1
    if [ -z "$connections" ]; then
      log "drain finished with zero connections on old port $port"
      return 0
    fi
    connections=$(printf '%s\n' "$connections" | wc -l)
    now=$(date +%s) || return 1
    elapsed=$((now - started))
    if [ "$elapsed" -ge "$alert_seconds" ] && [ "$alerted" -eq 0 ]; then
      log "WARN: drain alert threshold ${alert_seconds}s reached with ${connections} connections; waiting without stopping"
      alerted=1
    fi
    log "draining old upstream :$port — ${connections} established connections, ${elapsed}s elapsed"
    sleep 15 || return 1
  done
}
