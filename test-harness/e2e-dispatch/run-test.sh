#!/usr/bin/env bash
# End-to-end dispatch test runner (ADR-020 Phase 32).
#
# What this test validates:
#   1. Builds + signs the test WASM component (exercises the component
#      toolchain)
#   2. Starts relay + robot + operator containers; the relay publishes its
#      dialable multiaddr, the robot connects and holds a circuit reservation,
#      then publishes its dialable libp2p id to /test-data
#   3. Operator registers the robot (`gang peer add <libp2p-id> --relay …`)
#   4. Operator performs a REAL deploy over the relay circuit:
#      `gang deploy e2e-robot /test-data/diagnostics.wasm`
#   5. Operator invokes it (`gang run e2e-robot diagnostics`) and asserts the
#      component's actual JSON output came back
#   6. Operator lists capabilities (`gang caps e2e-robot`) and asserts the
#      deployed capability is present
#   7. Tears down (always, via the EXIT trap)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

EXIT_CODE=0

# Always tear down, no matter how the script exits (mirrors run-scenario.sh).
cleanup() {
    echo
    echo "--- Tearing down ---"
    docker compose down -v --remove-orphans 2>/dev/null || true
    rm -f test-data/robot-libp2p-id test-data/robot-libp2p-id.tmp test-data/robot-agent.log
}
trap cleanup EXIT

echo "=== Ganglion E2E Dispatch Test ==="
echo

# Step 1: Build test WASM component
echo "--- Building test component ---"
./build-test-component.sh
echo

# Step 2: Create robot + operator scripts (mounted as a shared volume)
mkdir -p test-data
rm -f test-data/robot-libp2p-id test-data/robot-libp2p-id.tmp test-data/robot-agent.log

# The robot waits for the relay's published multiaddr, starts the agent
# pointed at it, waits for the relay connection + circuit reservation, then
# publishes its dialable libp2p id and agent log to /test-data.
cat > test-data/run-robot.sh << 'ROBOT_SCRIPT'
#!/usr/bin/env bash
set -euo pipefail

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
ROBOT_SCRIPT
chmod +x test-data/run-robot.sh

cat > test-data/run-operator-test.sh << 'OPERATOR_SCRIPT'
#!/usr/bin/env bash
set -euo pipefail

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
OPERATOR_SCRIPT
chmod +x test-data/run-operator-test.sh

# Step 3: Start containers. Under `set -e` a plain invocation would abort
# the script before we can capture the exit code, so use `|| EXIT_CODE=$?`;
# teardown happens in the EXIT trap regardless.
echo "--- Starting e2e scenario ---"
docker compose up --build --abort-on-container-exit --exit-code-from operator 2>&1 || EXIT_CODE=$?

if [ $EXIT_CODE -eq 0 ]; then
    echo
    echo "=== E2E DISPATCH TEST PASSED ==="
else
    echo
    echo "=== E2E DISPATCH TEST FAILED (exit code: $EXIT_CODE) ==="
fi

exit $EXIT_CODE
