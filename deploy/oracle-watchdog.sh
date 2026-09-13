#!/usr/bin/env bash
# Fallback shell watchdog for the oracle pod (use rti-oracle-watchdog when
# the binary is available). Stops this pod via the Runpod API when the
# heartbeat file has not been touched for IDLE_MINUTES.
# RTI's lifecycle touches the file through the HTTP watchdog; with this
# script, expose /heartbeat with e.g. `socat` or just `touch /tmp/rti-heartbeat`
# from the bridge plugin whenever a request arrives.
set -u
IDLE_MINUTES="${WATCHDOG_IDLE_MINUTES:-10}"
HB="${HEARTBEAT_FILE:-/tmp/rti-heartbeat}"
API="${RUNPOD_API_URL:-https://api.runpod.io/graphql}"
touch "$HB"
while true; do
  sleep 60
  if [ -n "$(find "$HB" -mmin +"$IDLE_MINUTES" 2>/dev/null)" ]; then
    echo "watchdog: idle for ${IDLE_MINUTES} min; stopping pod ${RUNPOD_POD_ID}"
    curl -s -X POST "$API" -H "Authorization: Bearer ${RUNPOD_API_KEY}" -H "Content-Type: application/json" \
      -d "{\"query\":\"mutation { podStop(input: {podId: \\\"${RUNPOD_POD_ID}\\\"}) { id desiredStatus } }\"}"
    sleep 300
  fi
done
