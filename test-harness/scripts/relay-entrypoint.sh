#!/usr/bin/env bash
# Relay entrypoint wrapper for the archetype scenarios.
#
# Resolves the relay's peer ID from its fixed test identity (GANG_KEY_PATH
# points at a mounted key from test-harness/keys/ — TEST KEYS, DO NOT USE),
# publishes the dialable relay multiaddr to a shared volume for the agents,
# then execs the relay server.
#
# Required env:
#   RELAY_IP       IP the agents should dial (the relay's internet-side addr)
#   GANG_KEY_PATH  path to the mounted test identity key
# Optional env:
#   RELAY_PORT     TCP/UDP listen port (default 4001)
#   SHARED_DIR     shared volume mount point (default /shared)
set -euo pipefail

RELAY_IP="${RELAY_IP:?RELAY_IP must be set to the relay's dialable IP}"
RELAY_PORT="${RELAY_PORT:-4001}"
SHARED_DIR="${SHARED_DIR:-/shared}"

# GANG_KEY_PATH points at the mounted deterministic test key, so `gang
# identity show` reports the same peer ID the relay server will use.
PEER_ID="$(gang identity show | awk '/Peer ID/ {print $NF}')"
if [ -z "$PEER_ID" ]; then
    echo "relay-entrypoint: could not determine relay peer ID" >&2
    exit 1
fi

MULTIADDR="/ip4/${RELAY_IP}/tcp/${RELAY_PORT}/p2p/${PEER_ID}"
mkdir -p "$SHARED_DIR"
# Write atomically so agents never observe a partially written file.
echo "$MULTIADDR" > "${SHARED_DIR}/relay.addr.tmp"
mv "${SHARED_DIR}/relay.addr.tmp" "${SHARED_DIR}/relay.addr"
echo "relay-entrypoint: published relay multiaddr $MULTIADDR"

exec gang relay --port "$RELAY_PORT"
