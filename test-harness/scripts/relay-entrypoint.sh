#!/usr/bin/env bash
# Relay entrypoint wrapper for the archetype scenarios.
#
# Starts the relay server, extracts the DIALABLE (base58 libp2p) peer id from
# the relay's own startup output, and publishes the client-config multiaddr to
# a shared volume for the agents.
#
# NOTE: the multiaddr must carry the libp2p peer id (`12D3KooW…`), not the
# gang identity id (`12D3-<hex>`) — a `/p2p/` component built from the gang id
# does not parse and every agent dial fails with "Invalid base string".
# The relay prints both, labeled; we take the dialable one.
#
# Required env:
#   RELAY_IP       IP the agents should dial (the relay's internet-side addr)
#   GANG_KEY_PATH  path to the mounted test identity key
# Optional env:
#   RELAY_PORT     TCP/UDP listen port (default 4001)
#   SHARED_DIR     shared volume mount point (default /shared)
set -euo pipefail

RELAY_IP="${RELAY_IP:?RELAY_IP must be set to the dialable relay IP}"
RELAY_PORT="${RELAY_PORT:-4001}"
SHARED_DIR="${SHARED_DIR:-/shared}"

# Start the relay with its output teed to a log we can parse. GANG_KEY_PATH
# points at the mounted deterministic test key, so the identity is stable.
LOG=/tmp/relay-stdout.log
gang relay --port "$RELAY_PORT" 2>&1 | tee "$LOG" &
RELAY_PIPELINE=$!

# Wait for the relay to print its dialable libp2p peer id.
LIBP2P_ID=""
for _ in $(seq 1 60); do
    LIBP2P_ID="$(awk '/Peer ID \(libp2p\/dial\):/ {print $NF}' "$LOG" 2>/dev/null | head -1)"
    [ -n "$LIBP2P_ID" ] && break
    sleep 1
done
if [ -z "$LIBP2P_ID" ]; then
    echo "relay-entrypoint: relay never printed its libp2p peer id" >&2
    tail -20 "$LOG" >&2 || true
    exit 1
fi

MULTIADDR="/ip4/${RELAY_IP}/tcp/${RELAY_PORT}/p2p/${LIBP2P_ID}"
mkdir -p "$SHARED_DIR"
# Write atomically so agents never observe a partially written file.
echo "$MULTIADDR" > "${SHARED_DIR}/relay.addr.tmp"
mv "${SHARED_DIR}/relay.addr.tmp" "${SHARED_DIR}/relay.addr"
echo "relay-entrypoint: published relay multiaddr $MULTIADDR"

# Keep the container's lifetime tied to the relay process.
wait "$RELAY_PIPELINE"
