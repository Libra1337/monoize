#!/usr/bin/env bash
set -euo pipefail

CHECK=0
if [ "${1:-}" = --check ]; then CHECK=1; shift; fi
if [ "$#" -eq 0 ]; then
  read -r STABLE ACTIVE PROXY_UID < /opt/monoize/blue-green-route.state
elif [ "$#" -eq 3 ]; then
  STABLE=$1 ACTIVE=$2 PROXY_UID=$3
else
  echo 'usage: blue-green-route.sh [--check] [stable-port active-port proxy-uid]' >&2
  exit 1
fi
[[ "$STABLE" =~ ^808[01]$ && "$ACTIVE" =~ ^808[01]$ && "$PROXY_UID" =~ ^[0-9]+$ ]] || exit 1
[ "$PROXY_UID" != 1000 ] || exit 1
CHAIN=MONOIZE_BG
JUMP=(-s 127.0.0.1/32 -d 127.0.0.1/32 -p tcp --syn --dport "$STABLE"
      -m owner --uid-owner "$PROXY_UID" -m comment --comment monoize-blue-green -j "$CHAIN")

if ! iptables -w -t nat -S "$CHAIN" >/dev/null 2>&1; then
  [ "$CHECK" -eq 0 ] || exit 1
  iptables -w -t nat -N "$CHAIN"
  iptables -w -t nat -A "$CHAIN" -p tcp -j REDIRECT --to-ports "$ACTIVE"
fi
RULES=$(iptables -w -t nat -S "$CHAIN")
[ "$(grep -c '^-A ' <<< "$RULES")" -eq 1 ] || {
  echo 'unexpected Monoize routing rule count; preserve current traffic' >&2
  exit 1
}
if ! iptables -w -t nat -C "$CHAIN" -p tcp -j REDIRECT --to-ports 8080 2>/dev/null &&
   ! iptables -w -t nat -C "$CHAIN" -p tcp -j REDIRECT --to-ports 8081 2>/dev/null; then
  echo 'unexpected Monoize routing target; preserve current traffic' >&2
  exit 1
fi
OUTPUT=$(iptables -w -t nat -S OUTPUT)
JUMPS=$(grep -- "-j $CHAIN\b" <<< "$OUTPUT" || true)
if [ -n "$JUMPS" ]; then
  [ "$(wc -l <<< "$JUMPS")" -eq 1 ] || exit 1
  iptables -w -t nat -C OUTPUT "${JUMP[@]}"
else
  [ "$CHECK" -eq 0 ] || exit 1
fi
if [ "$CHECK" -eq 0 ]; then
  # Keep a NAT expression even for the identity route; removing the last one
  # can unregister hooks and invalidate existing translated connections.
  iptables -w -t nat -R "$CHAIN" 1 -p tcp -j REDIRECT --to-ports "$ACTIVE"
  if [ -z "$JUMPS" ]; then
    iptables -w -t nat -I OUTPUT 1 "${JUMP[@]}"
  fi
fi
iptables -w -t nat -C "$CHAIN" -p tcp -j REDIRECT --to-ports "$ACTIVE"
iptables -w -t nat -C OUTPUT "${JUMP[@]}"
