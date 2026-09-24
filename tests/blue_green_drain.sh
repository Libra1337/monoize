#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/../scripts/blue-green-drain.sh"

log() { printf '%s\n' "$*"; }

(
  ticks=0
  date() { echo "$((ticks * 15000))"; }
  ss() {
    if [ "$ticks" -lt 3 ]; then echo 'long-lived request'; fi
  }
  sleep() { ticks=$((ticks + 1)); }
  wait_for_upstream_drain 8080 14400
  [ "$ticks" -eq 3 ] || { echo 'stopped before the long request ended'; exit 1; }
)

(
  ss() { return 7; }
  if wait_for_upstream_drain 8081 14400; then
    echo 'a failed connection query was mistaken for a completed drain'
    exit 1
  fi
)

(
  ss() { :; }
  sleep() { echo 'empty upstream must not wait'; exit 1; }
  wait_for_upstream_drain 8081 14400
)

if wait_for_upstream_drain 8080 invalid; then exit 1; fi
echo 'PASS: alert threshold, query failure, empty drain, and input validation'
