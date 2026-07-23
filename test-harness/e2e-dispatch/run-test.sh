#!/usr/bin/env bash
# End-to-end connectivity smoke test runner.
#
# What this test ACTUALLY validates today:
#   1. Builds the test WASM component (exercises the component toolchain)
#   2. Starts relay + robot + operator containers
#   3. Robot agent connects to the relay and publishes its peer ID to the
#      shared /test-data mount
#   4. Operator registers the robot (`gang peer add`) and asserts the robot's
#      relay connection was established
#   5. Tears down (always, via the EXIT trap)
#
# TODO(dispatch-workstream): upgrade to a real deploy/invoke round-trip once
# ADR-020 Phase 32 lands. `gang deploy <remote>` currently bails because
# operator remote dispatch is not implemented in the CLI, so this test cannot
# yet validate the full deploy -> invoke -> result flow. When dispatch lands:
#   - operator: gang deploy e2e-robot /test-data/diagnostics.wasm
#   - operator: gang run e2e-robot diagnostics, assert structured output
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

EXIT_CODE=0

# Always tear down, no matter how the script exits (mirrors run-scenario.sh).
cleanup() {
    echo
    echo "--- Tearing down ---"
    docker compose down -v --remove-orphans 2>/dev/null || true
    rm -f test-data/robot-peer-id test-data/robot-peer-id.tmp test-data/robot-agent.log
}
trap cleanup EXIT

echo "=== Ganglion E2E Connectivity Smoke Test ==="
echo

# Step 1: Build test WASM component
echo "--- Building test component ---"
./build-test-component.sh
echo

# Step 2: Create robot + operator scripts (mounted as a shared volume)
mkdir -p test-data
rm -f test-data/robot-peer-id test-data/robot-peer-id.tmp test-data/robot-agent.log

# The robot starts the agent pointed at the relay, waits for the relay
# connection to be established, then publishes its peer ID and agent log to
# /test-data for the operator to consume.
cat > test-data/run-robot.sh << 'ROBOT_SCRIPT'
#!/usr/bin/env bash
set -euo pipefail

RELAY_ADDR="/ip4/172.28.0.10/tcp/4001"
LOG=/tmp/agent.log

gang agent --data-dir /data -r "$RELAY_ADDR" > "$LOG" 2>&1 &
AGENT_PID=$!

# Stream agent output to the container log as well
tail -f "$LOG" &

# Wait for the agent to report its relay connection
for _ in $(seq 1 30); do
    grep -q "Connected to relay" "$LOG" && break
    sleep 1
done

PEER_ID=$(awk '/Peer ID:/ {print $NF; exit}' "$LOG")
if [ -z "${PEER_ID:-}" ] || ! grep -q "Connected to relay" "$LOG"; then
    echo "robot: failed to establish relay connection" >&2
    cat "$LOG" >&2
    exit 1
fi

# Publish log + peer ID (peer ID last, atomically — the operator keys off it)
cp "$LOG" /test-data/robot-agent.log
echo "$PEER_ID" > /test-data/robot-peer-id.tmp
mv /test-data/robot-peer-id.tmp /test-data/robot-peer-id
echo "robot: published peer ID $PEER_ID"

wait "$AGENT_PID"
ROBOT_SCRIPT
chmod +x test-data/run-robot.sh

cat > test-data/run-operator-test.sh << 'OPERATOR_SCRIPT'
#!/usr/bin/env bash
set -euo pipefail

echo "=== Operator e2e connectivity smoke test ==="

RELAY_ADDR="/ip4/172.28.0.10/tcp/4001"

echo "Step 1: Wait for robot to publish its peer ID"
for _ in $(seq 1 60); do
    [ -s /test-data/robot-peer-id ] && break
    sleep 1
done
if [ ! -s /test-data/robot-peer-id ]; then
    echo "FAIL: robot never published its peer ID to /test-data" >&2
    exit 1
fi
ROBOT_PEER_ID=$(cat /test-data/robot-peer-id)
echo "PASS: robot peer ID is $ROBOT_PEER_ID"

echo "Step 2: Register robot peer"
gang peer add e2e-robot "$ROBOT_PEER_ID" --relay "$RELAY_ADDR" --role robot-agent
if ! gang peer list | grep -q "e2e-robot"; then
    echo "FAIL: e2e-robot missing from gang peer list" >&2
    exit 1
fi
echo "PASS: robot registered with operator"

echo "Step 3: Verify robot's relay connection was established"
if ! grep -q "Connected to relay" /test-data/robot-agent.log; then
    echo "FAIL: robot agent log does not show an established relay connection" >&2
    cat /test-data/robot-agent.log >&2
    exit 1
fi
echo "PASS: robot established relay connection"

# TODO(dispatch-workstream): upgrade to a real deploy/invoke round-trip once
# ADR-020 Phase 32 lands (gang deploy <remote> currently bails).

echo
echo "=== e2e connectivity smoke test passed ==="
OPERATOR_SCRIPT
chmod +x test-data/run-operator-test.sh

# Step 3: Start containers. Under `set -e` a plain invocation would abort
# the script before we can capture the exit code, so use `|| EXIT_CODE=$?`;
# teardown happens in the EXIT trap regardless.
echo "--- Starting e2e scenario ---"
docker compose up --build --abort-on-container-exit --exit-code-from operator 2>&1 || EXIT_CODE=$?

if [ $EXIT_CODE -eq 0 ]; then
    echo
    echo "=== E2E CONNECTIVITY SMOKE TEST PASSED ==="
else
    echo
    echo "=== E2E CONNECTIVITY SMOKE TEST FAILED (exit code: $EXIT_CODE) ==="
fi

exit $EXIT_CODE
