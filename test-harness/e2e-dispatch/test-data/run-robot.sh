#!/usr/bin/env bash
set -euo pipefail

# Degraded-link matrix hook (#32): apply link impairment BEFORE the agent
# starts, so the whole session runs under the shaped link. The exact command
# is echoed so it lands in the container log (and the run artifact).
if [ -n "${GANG_SHAPE_CMD:-}" ]; then
    echo "robot: shaping link: $GANG_SHAPE_CMD"
    if ! eval "$GANG_SHAPE_CMD"; then
        echo "robot: FAIL: link shaping command failed" >&2
        exit 1
    fi
    echo "robot: link shaped"
fi

ADDR_FILE=/shared/relay.addr
for _ in $(seq 1 60); do
    [ -s "$ADDR_FILE" ] && break
    sleep 1
done
if [ ! -s "$ADDR_FILE" ]; then
    echo "robot: relay never published its multiaddr to $ADDR_FILE" >&2
    exit 1
fi
RELAY_ADDR="$(cat "$ADDR_FILE")"
echo "robot: dialing relay $RELAY_ADDR"

LOG=/tmp/agent.log
# Create the log before tail starts: tail -f racing the agent's first write
# dies with "cannot open" and the agent output never reaches the container log.
: > "$LOG"
gang agent --data-dir /data -r "$RELAY_ADDR" >> "$LOG" 2>&1 &
AGENT_PID=$!

# Stream agent output to the container log as well
tail -f "$LOG" &

# Wait for the agent to report its relay connection. The agent only prints
# this once its circuit reservation is held (i.e. it is actually dialable
# through the relay), so publishing the id below really means "ready".
for _ in $(seq 1 60); do
    grep -q "Connected to relay" "$LOG" && break
    sleep 1
done

# The DIALABLE (base58 libp2p) id is what the operator must register — a
# /p2p/ multiaddr component only accepts this form, not the gang id.
LIBP2P_ID=$(awk '/Peer ID \(libp2p\/dial\):/ {print $NF; exit}' "$LOG")
if [ -z "${LIBP2P_ID:-}" ] || ! grep -q "Connected to relay" "$LOG"; then
    echo "robot: failed to establish relay connection" >&2
    cat "$LOG" >&2
    exit 1
fi

# Publish log + libp2p id (id last, atomically — the operator keys off it)
cp "$LOG" /test-data/robot-agent.log
echo "$LIBP2P_ID" > /test-data/robot-libp2p-id.tmp
mv /test-data/robot-libp2p-id.tmp /test-data/robot-libp2p-id
echo "robot: published libp2p id $LIBP2P_ID"

wait "$AGENT_PID"
