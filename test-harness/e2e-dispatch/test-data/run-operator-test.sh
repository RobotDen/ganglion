#!/usr/bin/env bash
set -euo pipefail

# Degraded-link matrix hook (#32): operator-side shaping (e.g. the downlink
# half of a symmetric-latency profile).
if [ -n "${GANG_SHAPE_OPERATOR_CMD:-}" ]; then
    echo "operator: shaping link: $GANG_SHAPE_OPERATOR_CMD"
    if ! eval "$GANG_SHAPE_OPERATOR_CMD"; then
        echo "operator: FAIL: link shaping command failed" >&2
        exit 1
    fi
    echo "operator: link shaped"
fi

echo "=== Operator e2e dispatch test ==="

echo "Step 1: Wait for the relay multiaddr and the robot's libp2p id"
for _ in $(seq 1 90); do
    [ -s /shared/relay.addr ] && [ -s /test-data/robot-libp2p-id ] && break
    sleep 1
done
if [ ! -s /shared/relay.addr ] || [ ! -s /test-data/robot-libp2p-id ]; then
    echo "FAIL: relay addr or robot libp2p id never appeared" >&2
    exit 1
fi
RELAY_ADDR="$(cat /shared/relay.addr)"
ROBOT_LIBP2P_ID="$(cat /test-data/robot-libp2p-id)"
echo "PASS: relay $RELAY_ADDR, robot $ROBOT_LIBP2P_ID"

echo "Step 2: Register robot peer (dialable libp2p id + relay)"
# TOFU auto-accept: this container is non-interactive, so the strict policy's
# first-connect prompt cannot be answered. Key CHANGES still hard-fail.
gang config set host_key_policy tofu
gang peer add e2e-robot "$ROBOT_LIBP2P_ID" --relay "$RELAY_ADDR" --role robot-agent
if ! gang peer list | grep -q "e2e-robot"; then
    echo "FAIL: e2e-robot missing from gang peer list" >&2
    exit 1
fi
echo "PASS: robot registered with operator"

echo "Step 3: Deploy the signed component over the relay circuit"
DEPLOYED=0
for attempt in $(seq 1 5); do
    if OUT=$(gang -q deploy e2e-robot /test-data/diagnostics.wasm 2>&1); then
        echo "$OUT"
        DEPLOYED=1
        break
    fi
    echo "deploy attempt $attempt failed, retrying in 3s:" >&2
    echo "$OUT" >&2
    sleep 3
done
if [ "$DEPLOYED" != 1 ]; then
    echo "FAIL: gang deploy never succeeded" >&2
    cat /test-data/robot-agent.log >&2 || true
    exit 1
fi
if ! echo "$OUT" | grep -q "Deployed 'diagnostics'"; then
    echo "FAIL: deploy output did not confirm the capability name: $OUT" >&2
    exit 1
fi
echo "PASS: deployed 'diagnostics' via relay"

echo "Step 4: Invoke the capability and assert its real output"
RUN_OUT=$(gang -q run e2e-robot diagnostics --format json)
echo "$RUN_OUT" | head -20
if ! echo "$RUN_OUT" | grep -q '"component": *"test-diagnostics"'; then
    echo "FAIL: run output missing component marker" >&2
    echo "$RUN_OUT" >&2
    exit 1
fi
if ! echo "$RUN_OUT" | grep -q '"system_info"'; then
    echo "FAIL: run output missing system_info" >&2
    echo "$RUN_OUT" >&2
    exit 1
fi
echo "PASS: WASM component executed on the robot and returned real output"

echo "Step 5: List capabilities"
CAPS_OUT=$(gang -q caps e2e-robot)
echo "$CAPS_OUT"
if ! echo "$CAPS_OUT" | grep -q "diagnostics"; then
    echo "FAIL: caps output missing 'diagnostics'" >&2
    exit 1
fi
echo "PASS: capability listed"

echo
echo "=== e2e dispatch test passed ==="
