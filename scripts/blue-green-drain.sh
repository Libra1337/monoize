#!/usr/bin/env bash

accepted_upstream_connections() {
  local port=$1 connections
  [[ "$port" =~ ^808[01]$ ]] || return 1
  # Inspect all states in one snapshot. Half-closed or handshaking sockets
  # must not be mistaken for a drained instance.
  connections=$(ss -tanH "( sport = :$port )") || return 1
  printf '%s\n' "$connections" | awk '
    NF && $1 != "LISTEN" && $1 != "TIME-WAIT" && $1 != "CLOSED"
  '
}

wait_for_upstream_drain() {
  local port=$1 alert_seconds=$2 started elapsed connections now alerted=0
  [[ "$port" =~ ^808[01]$ && "$alert_seconds" =~ ^[0-9]+$ ]] || return 1
  started=$(date +%s) || return 1
  while :; do
    if declare -F drain_health_check >/dev/null; then
      drain_health_check || return 1
    fi
    # A failed socket query must not look like an empty result.
    connections=$(accepted_upstream_connections "$port") || return 1
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
    log "draining old upstream :$port — ${connections} accepted connections, ${elapsed}s elapsed"
    sleep 15 || return 1
  done
}

wait_for_forwarding_drain() {
  local port=$1 alert_seconds=$2 connections
  [[ "$port" =~ ^808[01]$ && "$alert_seconds" =~ ^[0-9]+$ ]] || return 1
  while :; do
    wait_for_upstream_drain "$port" "$alert_seconds" || return 1
    pause_forwarding || return 1
    connections=$(accepted_upstream_connections "$port") || return 1
    [ -z "$connections" ] && return 0
    resume_forwarding || return 1
    sleep 15 || return 1
  done
}
