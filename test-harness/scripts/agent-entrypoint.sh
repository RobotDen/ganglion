#!/usr/bin/env bash
# Agent entrypoint wrapper for the archetype scenarios.
#
# Waits for the relay to publish its dialable multiaddr to the shared volume
# (written by relay-entrypoint.sh), then starts the agent pointed at the
# relay so the scenario actually exercises gang connectivity.
#
# Optional env:
#   SHARED_DIR     shared volume mount point (default /shared)
#   WAIT_ATTEMPTS  1s attempts to wait for the relay address (default 60)
#   AGENT_DATA_DIR agent data directory (default /data)
set -euo pipefail

SHARED_DIR="${SHARED_DIR:-/shared}"
ADDR_FILE="${SHARED_DIR}/relay.addr"
WAIT_ATTEMPTS="${WAIT_ATTEMPTS:-60}"
AGENT_DATA_DIR="${AGENT_DATA_DIR:-/data}"

for _ in $(seq 1 "$WAIT_ATTEMPTS"); do
    [ -s "$ADDR_FILE" ] && break
    sleep 1
done

if [ ! -s "$ADDR_FILE" ]; then
    echo "agent-entrypoint: relay address never appeared at $ADDR_FILE" >&2
    exit 1
fi

RELAY_ADDR="$(cat "$ADDR_FILE")"
echo "agent-entrypoint: dialing relay $RELAY_ADDR"
exec gang agent --data-dir "$AGENT_DATA_DIR" -r "$RELAY_ADDR"
