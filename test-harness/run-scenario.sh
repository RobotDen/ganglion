#!/usr/bin/env bash
# Run a Ganglion test harness scenario and capture timing metrics.
#
# Usage: ./run-scenario.sh <archetype>
# Archetypes: open-warehouse, nat-office, enterprise-dmz, mobile-cgnat

set -euo pipefail

ARCHETYPE="${1:?Usage: $0 <archetype>}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SCENARIO_DIR="$SCRIPT_DIR/$ARCHETYPE"

if [ ! -d "$SCENARIO_DIR" ]; then
    echo "Error: No scenario directory at $SCENARIO_DIR"
    echo "Valid archetypes: open-warehouse, nat-office, enterprise-dmz, mobile-cgnat"
    exit 1
fi

if [ ! -f "$SCENARIO_DIR/docker-compose.yml" ]; then
    echo "Error: No docker-compose.yml in $SCENARIO_DIR"
    exit 1
fi

PROJECT_NAME="ganglion-${ARCHETYPE}"

cleanup() {
    echo ""
    echo "=== Tearing down $ARCHETYPE scenario ==="
    docker compose -p "$PROJECT_NAME" -f "$SCENARIO_DIR/docker-compose.yml" down -v --remove-orphans 2>/dev/null || true
}
trap cleanup EXIT

echo "============================================"
echo "  Ganglion Test Harness: $ARCHETYPE"
echo "============================================"
echo ""

# Build images
echo "=== Building images ==="
BUILD_START=$(date +%s%N)
docker compose -p "$PROJECT_NAME" -f "$SCENARIO_DIR/docker-compose.yml" build --quiet
BUILD_END=$(date +%s%N)
BUILD_MS=$(( (BUILD_END - BUILD_START) / 1000000 ))
echo "Build completed in ${BUILD_MS}ms"
echo ""

# Start infrastructure (NAT gateways, firewalls) first if present
echo "=== Starting scenario: $ARCHETYPE ==="
UP_START=$(date +%s%N)
docker compose -p "$PROJECT_NAME" -f "$SCENARIO_DIR/docker-compose.yml" up -d
UP_END=$(date +%s%N)
UP_MS=$(( (UP_END - UP_START) / 1000000 ))
echo "Services started in ${UP_MS}ms"
echo ""

# Wait for services to stabilize
echo "=== Waiting for services to stabilize ==="
sleep 3

# Check service health
echo "=== Service status ==="
docker compose -p "$PROJECT_NAME" -f "$SCENARIO_DIR/docker-compose.yml" ps
echo ""

# Capture relay logs
echo "=== Relay logs ==="
docker compose -p "$PROJECT_NAME" -f "$SCENARIO_DIR/docker-compose.yml" logs relay 2>&1 | tail -20
echo ""

# Capture robot logs
echo "=== Robot logs ==="
docker compose -p "$PROJECT_NAME" -f "$SCENARIO_DIR/docker-compose.yml" logs robot 2>&1 | tail -20
echo ""

# Capture operator logs
echo "=== Operator logs ==="
docker compose -p "$PROJECT_NAME" -f "$SCENARIO_DIR/docker-compose.yml" logs operator 2>&1 | tail -20
echo ""

# Network connectivity test: ping from operator to relay
echo "=== Connectivity checks ==="
case "$ARCHETYPE" in
    open-warehouse)
        echo "Checking direct connectivity (operator → robot):"
        docker compose -p "$PROJECT_NAME" -f "$SCENARIO_DIR/docker-compose.yml" \
            exec -T operator ping -c 3 -W 2 172.20.0.20 2>&1 || echo "  (ping failed — expected if gang is handling connectivity)"
        ;;
    nat-office)
        echo "Checking operator → relay connectivity:"
        docker compose -p "$PROJECT_NAME" -f "$SCENARIO_DIR/docker-compose.yml" \
            exec -T operator ping -c 3 -W 2 10.0.0.10 2>&1 || echo "  (operator cannot reach relay directly — NAT in path)"
        echo ""
        echo "Checking robot → relay connectivity (should work via NAT):"
        docker compose -p "$PROJECT_NAME" -f "$SCENARIO_DIR/docker-compose.yml" \
            exec -T robot ping -c 3 -W 2 192.168.1.1 2>&1 || echo "  (robot cannot reach gateway)"
        ;;
    enterprise-dmz)
        echo "Checking robot → firewall gateway:"
        docker compose -p "$PROJECT_NAME" -f "$SCENARIO_DIR/docker-compose.yml" \
            exec -T robot ping -c 3 -W 2 172.16.10.1 2>&1 || echo "  (robot cannot reach firewall)"
        echo ""
        echo "Checking operator → relay (direct internet):"
        docker compose -p "$PROJECT_NAME" -f "$SCENARIO_DIR/docker-compose.yml" \
            exec -T operator ping -c 3 -W 2 10.1.0.10 2>&1 || echo "  (operator cannot reach relay)"
        ;;
    mobile-cgnat)
        echo "Checking robot → inner NAT gateway:"
        docker compose -p "$PROJECT_NAME" -f "$SCENARIO_DIR/docker-compose.yml" \
            exec -T robot ping -c 3 -W 2 10.64.0.1 2>&1 || echo "  (robot cannot reach inner gateway)"
        echo ""
        echo "Checking operator → relay:"
        docker compose -p "$PROJECT_NAME" -f "$SCENARIO_DIR/docker-compose.yml" \
            exec -T operator ping -c 3 -W 2 10.2.0.10 2>&1 || echo "  (operator cannot reach relay)"
        ;;
esac
echo ""

echo "============================================"
echo "  Scenario $ARCHETYPE complete"
echo "============================================"
echo ""
echo "Summary:"
echo "  Build time:   ${BUILD_MS}ms"
echo "  Startup time: ${UP_MS}ms"
echo ""
echo "To inspect manually:"
echo "  docker compose -p $PROJECT_NAME -f $SCENARIO_DIR/docker-compose.yml exec robot bash"
echo "  docker compose -p $PROJECT_NAME -f $SCENARIO_DIR/docker-compose.yml logs -f"
