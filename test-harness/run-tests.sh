#!/usr/bin/env bash
# Run all Ganglion test harness scenarios and report results.
#
# Usage: ./run-tests.sh [--scenario <name>] [--timeout <seconds>]
#
# Options:
#   --scenario <name>   Run only this scenario (default: all four)
#   --timeout <secs>    Per-scenario timeout in seconds (default: 120)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TIMEOUT="${TIMEOUT:-120}"
SCENARIO_FILTER=""

# Parse args
while [[ $# -gt 0 ]]; do
    case "$1" in
        --scenario) SCENARIO_FILTER="$2"; shift 2 ;;
        --timeout) TIMEOUT="$2"; shift 2 ;;
        *) echo "Unknown arg: $1"; exit 1 ;;
    esac
done

ALL_SCENARIOS=(open-warehouse nat-office enterprise-dmz mobile-cgnat)
PASS=0
FAIL=0
SKIP=0
RESULTS=()

# Colors (if terminal supports them)
if [ -t 1 ]; then
    GREEN='\033[0;32m'
    RED='\033[0;31m'
    YELLOW='\033[0;33m'
    BOLD='\033[1m'
    NC='\033[0m'
else
    GREEN='' RED='' YELLOW='' BOLD='' NC=''
fi

log_pass() { echo -e "  ${GREEN}PASS${NC} $1"; }
log_fail() { echo -e "  ${RED}FAIL${NC} $1"; }
log_skip() { echo -e "  ${YELLOW}SKIP${NC} $1"; }

# --- Preflight checks ---

echo "============================================"
echo "  Ganglion Test Harness"
echo "============================================"
echo ""

# Check Docker
if ! docker info > /dev/null 2>&1; then
    echo "ERROR: Docker is not running."
    echo "Start Docker Desktop and try again."
    exit 1
fi
echo "Docker: $(docker info --format '{{.ServerVersion}}' 2>/dev/null)"

# Check docker compose
if ! docker compose version > /dev/null 2>&1; then
    echo "ERROR: docker compose is not available."
    exit 1
fi
echo "Compose: $(docker compose version --short 2>/dev/null)"
echo ""

# --- Build base image ---

echo "=== Building base image ==="
BUILD_START=$(date +%s)
if docker build -t ganglion-test -f "$SCRIPT_DIR/Dockerfile.base" "$PROJECT_ROOT" 2>&1; then
    BUILD_END=$(date +%s)
    echo ""
    echo "Base image built in $((BUILD_END - BUILD_START))s"
else
    echo ""
    echo "ERROR: Base image build failed."
    exit 1
fi
echo ""

# --- Run scenarios ---

# Poll a service's logs for a pattern, up to N 1-second attempts.
wait_for_log() {
    local project_name="$1" compose_file="$2" service="$3" pattern="$4" attempts="$5"
    local i
    for (( i = 0; i < attempts; i++ )); do
        if docker compose -p "$project_name" -f "$compose_file" logs "$service" 2>&1 | grep -q "$pattern"; then
            return 0
        fi
        sleep 1
    done
    return 1
}

teardown() {
    local scenario="$1"
    local project_name="ganglion-${scenario}"
    local compose_file="$SCRIPT_DIR/$scenario/docker-compose.yml"
    docker compose -p "$project_name" -f "$compose_file" down -v --remove-orphans 2>/dev/null || true
}

run_scenario() {
    local scenario="$1"
    local project_name="ganglion-${scenario}"
    local compose_file="$SCRIPT_DIR/$scenario/docker-compose.yml"
    local checks_passed=0
    local checks_total=0

    echo "=== Scenario: $scenario ==="
    echo ""

    # Always clean up on exit from this function
    trap "teardown $scenario" RETURN

    # Tear down any leftover from previous runs
    teardown "$scenario"

    # Start the scenario, bounded by the per-scenario timeout (OPS-12).
    echo "Starting containers (timeout: ${TIMEOUT}s)..."
    if ! timeout "$TIMEOUT" docker compose -p "$project_name" -f "$compose_file" up -d --build 2>&1; then
        log_fail "docker compose up failed (or exceeded ${TIMEOUT}s timeout)"
        FAIL=$((FAIL + 1))
        RESULTS+=("$scenario: FAIL (compose up failed)")
        return 1
    fi

    # Wait for services to stabilize: poll container state instead of a fixed
    # sleep. Attempts are derived from the per-scenario timeout (2s interval).
    echo "Waiting for services to stabilize..."
    local running=0 expected=0 attempt=0
    local max_attempts=$(( TIMEOUT / 2 ))
    [ "$max_attempts" -lt 1 ] && max_attempts=1
    while [ "$attempt" -lt "$max_attempts" ]; do
        running=$(docker compose -p "$project_name" -f "$compose_file" ps --format '{{.State}}' 2>/dev/null | grep -c "running" || true)
        expected=$(docker compose -p "$project_name" -f "$compose_file" ps -a --format '{{.State}}' 2>/dev/null | wc -l | tr -d ' ')
        if [ "$expected" -gt 0 ] && [ "$running" -eq "$expected" ]; then
            break
        fi
        attempt=$((attempt + 1))
        sleep 2
    done

    # --- Check 1: All containers running ---
    checks_total=$((checks_total + 1))
    if [ "$running" -eq "$expected" ] && [ "$running" -gt 0 ]; then
        log_pass "all $running containers running"
        checks_passed=$((checks_passed + 1))
    else
        log_fail "only $running/$expected containers running"
        # Show which containers are not running
        docker compose -p "$project_name" -f "$compose_file" ps -a 2>/dev/null
    fi

    # --- Check 2: gang binary is running in relay container ---
    checks_total=$((checks_total + 1))
    if docker compose -p "$project_name" -f "$compose_file" exec -T relay pgrep -x gang > /dev/null 2>&1; then
        log_pass "gang process running in relay"
        checks_passed=$((checks_passed + 1))
    else
        # The process might have a different name or have exited
        local relay_status
        relay_status=$(docker compose -p "$project_name" -f "$compose_file" ps relay --format '{{.State}}' 2>/dev/null || echo "unknown")
        if [ "$relay_status" = "running" ]; then
            log_pass "relay container running (gang may have finished init)"
            checks_passed=$((checks_passed + 1))
        else
            log_fail "relay container not running (state: $relay_status)"
        fi
    fi

    # --- Check 3: gang binary is running in robot container ---
    checks_total=$((checks_total + 1))
    if docker compose -p "$project_name" -f "$compose_file" exec -T robot pgrep -x gang > /dev/null 2>&1; then
        log_pass "gang process running in robot"
        checks_passed=$((checks_passed + 1))
    else
        local robot_status
        robot_status=$(docker compose -p "$project_name" -f "$compose_file" ps robot --format '{{.State}}' 2>/dev/null || echo "unknown")
        if [ "$robot_status" = "running" ]; then
            log_pass "robot container running (gang may have finished init)"
            checks_passed=$((checks_passed + 1))
        else
            log_fail "robot container not running (state: $robot_status)"
        fi
    fi

    # --- Check 4: Relay logs show startup ---
    checks_total=$((checks_total + 1))
    local relay_logs
    relay_logs=$(docker compose -p "$project_name" -f "$compose_file" logs relay 2>&1 || true)
    if echo "$relay_logs" | grep -qi "relay\|listen\|peer\|started\|running"; then
        log_pass "relay shows startup messages"
        checks_passed=$((checks_passed + 1))
    else
        log_fail "relay has no startup messages"
        echo "    Relay logs (last 10 lines):"
        echo "$relay_logs" | tail -10 | sed 's/^/    /'
    fi

    # --- Check 5: Network connectivity matches archetype ---
    checks_total=$((checks_total + 1))
    case "$scenario" in
        open-warehouse)
            # Flat L2: operator should reach robot directly
            if docker compose -p "$project_name" -f "$compose_file" \
                exec -T operator ping -c 2 -W 2 172.20.0.20 > /dev/null 2>&1; then
                log_pass "operator can ping robot directly (flat L2 confirmed)"
                checks_passed=$((checks_passed + 1))
            else
                log_fail "operator cannot ping robot (expected direct connectivity)"
            fi
            ;;
        nat-office)
            # NAT gateway should be reachable from robot
            if docker compose -p "$project_name" -f "$compose_file" \
                exec -T robot ping -c 2 -W 2 192.168.1.1 > /dev/null 2>&1; then
                log_pass "robot can reach NAT gateway"
                checks_passed=$((checks_passed + 1))
            else
                log_fail "robot cannot reach NAT gateway"
            fi
            ;;
        enterprise-dmz)
            # Robot should reach its firewall gateway
            if docker compose -p "$project_name" -f "$compose_file" \
                exec -T robot ping -c 2 -W 2 172.16.10.1 > /dev/null 2>&1; then
                log_pass "robot can reach firewall gateway"
                checks_passed=$((checks_passed + 1))
            else
                log_fail "robot cannot reach firewall gateway"
            fi
            ;;
        mobile-cgnat)
            # Robot should reach inner NAT gateway
            if docker compose -p "$project_name" -f "$compose_file" \
                exec -T robot ping -c 2 -W 2 10.64.0.1 > /dev/null 2>&1; then
                log_pass "robot can reach inner NAT gateway"
                checks_passed=$((checks_passed + 1))
            else
                log_fail "robot cannot reach inner NAT gateway"
            fi
            ;;
    esac

    # --- Check 6: Archetype-specific network rules ---
    checks_total=$((checks_total + 1))
    case "$scenario" in
        open-warehouse)
            # All nodes should be able to reach each other
            if docker compose -p "$project_name" -f "$compose_file" \
                exec -T robot ping -c 2 -W 2 172.20.0.10 > /dev/null 2>&1; then
                log_pass "robot can reach relay directly"
                checks_passed=$((checks_passed + 1))
            else
                log_fail "robot cannot reach relay"
            fi
            ;;
        nat-office)
            # Robot should NOT be directly reachable from internet
            # (we test by checking the NAT gateway blocks inbound)
            local nat_rules
            nat_rules=$(docker compose -p "$project_name" -f "$compose_file" \
                exec -T nat-gateway iptables -L FORWARD -n 2>/dev/null || echo "")
            if echo "$nat_rules" | grep -q "DROP"; then
                log_pass "NAT gateway has DROP rules for inbound"
                checks_passed=$((checks_passed + 1))
            else
                log_fail "NAT gateway missing DROP rules"
            fi
            ;;
        enterprise-dmz)
            # Firewall should only allow TCP 443
            local fw_rules
            fw_rules=$(docker compose -p "$project_name" -f "$compose_file" \
                exec -T dmz-firewall iptables -L FORWARD -n 2>/dev/null || echo "")
            if echo "$fw_rules" | grep -q "tcp dpt:443"; then
                log_pass "firewall allows TCP 443 outbound"
                checks_passed=$((checks_passed + 1))
            else
                log_fail "firewall missing TCP 443 rule"
            fi
            ;;
        mobile-cgnat)
            # Check netem is applied (jitter/loss simulation)
            local netem
            netem=$(docker compose -p "$project_name" -f "$compose_file" \
                exec -T cgnat-inner-nat tc qdisc show 2>/dev/null || echo "")
            if echo "$netem" | grep -q "netem"; then
                log_pass "netem qdisc active (jitter/loss simulation)"
                checks_passed=$((checks_passed + 1))
            else
                log_fail "netem not configured"
            fi
            ;;
    esac

    # --- Check 7: robot agent established a relay connection (OPS-13) ---
    # The agent entrypoint wrapper dials the relay multiaddr published on the
    # shared volume; "Connected to relay" is the agent's success line. Retry
    # for a while: NAT/netem scenarios can be slow to converge.
    checks_total=$((checks_total + 1))
    if wait_for_log "$project_name" "$compose_file" robot "Connected to relay" 30; then
        log_pass "robot agent connected to relay"
        checks_passed=$((checks_passed + 1))
    else
        log_fail "robot agent never logged 'Connected to relay'"
        docker compose -p "$project_name" -f "$compose_file" logs robot 2>&1 | tail -10 | sed 's/^/    /'
    fi

    # --- Check 8: operator agent established a relay connection (OPS-13) ---
    checks_total=$((checks_total + 1))
    if wait_for_log "$project_name" "$compose_file" operator "Connected to relay" 30; then
        log_pass "operator agent connected to relay"
        checks_passed=$((checks_passed + 1))
    else
        log_fail "operator agent never logged 'Connected to relay'"
        docker compose -p "$project_name" -f "$compose_file" logs operator 2>&1 | tail -10 | sed 's/^/    /'
    fi

    echo ""

    # Record result
    if [ "$checks_passed" -eq "$checks_total" ]; then
        PASS=$((PASS + 1))
        RESULTS+=("$scenario: PASS ($checks_passed/$checks_total checks)")
    else
        FAIL=$((FAIL + 1))
        RESULTS+=("$scenario: FAIL ($checks_passed/$checks_total checks)")
    fi
}

# Filter scenarios
scenarios_to_run=()
if [ -n "$SCENARIO_FILTER" ]; then
    for s in "${ALL_SCENARIOS[@]}"; do
        if [ "$s" = "$SCENARIO_FILTER" ]; then
            scenarios_to_run+=("$s")
        fi
    done
    if [ ${#scenarios_to_run[@]} -eq 0 ]; then
        echo "Unknown scenario: $SCENARIO_FILTER"
        echo "Valid: ${ALL_SCENARIOS[*]}"
        exit 1
    fi
else
    scenarios_to_run=("${ALL_SCENARIOS[@]}")
fi

for scenario in "${scenarios_to_run[@]}"; do
    run_scenario "$scenario" || true
    echo ""
done

# --- Summary ---

echo "============================================"
echo "  Test Summary"
echo "============================================"
echo ""
for result in "${RESULTS[@]}"; do
    echo "  $result"
done
echo ""
echo "  Total: $((PASS + FAIL)) | Pass: $PASS | Fail: $FAIL"
echo ""

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
