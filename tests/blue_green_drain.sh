#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/../scripts/blue-green-drain.sh"

log() { printf '%s\n' "$*"; }

(
  ss() {
    [ "$#" -eq 2 ] && [ "$1" = '-tanH' ] && [ "$2" = '( sport = :8080 )' ] || return 9
    printf '%s\n' \
      'LISTEN 0 128 127.0.0.1:8080 0.0.0.0:*' \
      'TIME-WAIT 0 0 127.0.0.1:8080 127.0.0.1:40000' \
      'CLOSED 0 0 127.0.0.1:8080 127.0.0.1:40001' \
      'ESTAB 0 0 127.0.0.1:8080 127.0.0.1:40002' \
      'CLOSE-WAIT 0 0 127.0.0.1:8080 127.0.0.1:40003' \
      'SYN-RECV 0 0 127.0.0.1:8080 127.0.0.1:40004' \
      'FIN-WAIT-1 0 0 127.0.0.1:8080 127.0.0.1:40005' \
      'FIN-WAIT-2 0 0 127.0.0.1:8080 127.0.0.1:40006'
  }
  expected=$(printf '%s\n' \
    'ESTAB 0 0 127.0.0.1:8080 127.0.0.1:40002' \
    'CLOSE-WAIT 0 0 127.0.0.1:8080 127.0.0.1:40003' \
    'SYN-RECV 0 0 127.0.0.1:8080 127.0.0.1:40004' \
    'FIN-WAIT-1 0 0 127.0.0.1:8080 127.0.0.1:40005' \
    'FIN-WAIT-2 0 0 127.0.0.1:8080 127.0.0.1:40006')
  [ "$(accepted_upstream_connections 8080)" = "$expected" ] || {
    echo 'accepted socket filtering discarded a live state or retained an inactive state'
    exit 1
  }
)

(
  ticks=0
  date() { echo "$((ticks * 15000))"; }
  ss() {
    if [ "$ticks" -lt 3 ]; then echo 'CLOSE-WAIT 0 0 127.0.0.1:8080 127.0.0.1:40000'; fi
  }
  sleep() { ticks=$((ticks + 1)); }
  wait_for_upstream_drain 8080 14400
  [ "$ticks" -eq 3 ] || { echo 'stopped before the long request ended'; exit 1; }
)

(
  ss() { echo 'LISTEN 0 128 127.0.0.1:8081 0.0.0.0:*'; return 7; }
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

(
  pauses=0 resumes=0 sleeps=0 paused=0
  date() { echo 0; }
  ss() {
    if [ "$pauses" -eq 1 ] && [ "$paused" -eq 1 ]; then
      echo 'ESTAB 0 0 127.0.0.1:8080 127.0.0.1:40000'
    fi
  }
  pause_forwarding() { pauses=$((pauses + 1)); paused=1; }
  resume_forwarding() { resumes=$((resumes + 1)); paused=0; }
  sleep() { sleeps=$((sleeps + 1)); }
  wait_for_forwarding_drain 8080 14400
  [ "$pauses" -eq 2 ] && [ "$resumes" -eq 1 ] && [ "$sleeps" -eq 1 ] && [ "$paused" -eq 1 ] || {
    echo 'a connection arriving between drain and pause must force resume and another drain'
    exit 1
  }
)

(
  paused=0 resumed=0
  date() { echo 0; }
  ss() { [ "$paused" -eq 0 ] || return 7; }
  pause_forwarding() { paused=1; }
  resume_forwarding() { resumed=1; }
  sleep() { echo 'failed paused inspection must not retry as an empty drain'; exit 1; }
  if wait_for_forwarding_drain 8081 14400; then
    echo 'failed socket recheck was mistaken for a completed forwarding drain'
    exit 1
  fi
  [ "$paused" -eq 1 ] && [ "$resumed" -eq 0 ]
)

(
  date() { echo 0; }
  ss() { :; }
  pause_forwarding() { return 7; }
  resume_forwarding() { echo 'failed pause must not continue handover'; exit 1; }
  if wait_for_forwarding_drain 8081 14400; then exit 1; fi
)

if accepted_upstream_connections 9090; then exit 1; fi
if wait_for_upstream_drain 8080 invalid; then exit 1; fi
if wait_for_forwarding_drain 8080 invalid; then exit 1; fi
(
  checked=0
  drain_health_check() { checked=1; return 3; }
  ss() { echo 'socket query must not override a failed candidate check'; exit 1; }
  if wait_for_upstream_drain 8080 14400; then exit 1; fi
  [ "$checked" = 1 ]
)
echo 'PASS: accepted socket states, unbounded drain, query failures, forwarding race, pause failure, and input validation'
