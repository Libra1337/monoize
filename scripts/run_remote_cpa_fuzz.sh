#!/bin/sh
set -eu

channel_id=mono_ch_cc5ceb64fd4041e98cbea51c565172af
record="$({
  docker exec monoize-postgres sh -lc \
    'psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" -F "|" -Atc "select base_url, api_key from monoize_channels where id = '\''"$1"'\''"' \
    sh "$channel_id"
})"
base_url=${record%%|*}
api_key=${record#*|}

if [ -z "$base_url" ] || [ -z "$api_key" ] || [ "$record" = "$base_url" ]; then
  echo "failed to load CPA channel credentials" >&2
  exit 2
fi

case "$base_url" in
  */v1) responses_url="${base_url%/}/responses" ;;
  *) responses_url="${base_url%/}/v1/responses" ;;
esac

export MONOIZE_PROBE_API_KEY="$api_key"
export CPA_RESPONSES_URL="$responses_url"
exec python3 /opt/monoize/maintenance/fuzz_cpa_reasoning_tool_pair.py
