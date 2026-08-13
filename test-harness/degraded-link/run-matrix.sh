#!/usr/bin/env bash
# Degraded-link matrix runner (#32).
#
#   ./run-matrix.sh                        # required gate: all 5 deterministic profiles
#   ./run-matrix.sh --profile lossy        # one profile
#   ./run-matrix.sh --chaos [--seed N]     # randomized netem chaos (nightly)
#
# Each profile runs the REAL e2e-dispatch round-trip (deploy -> invoke ->
# verify over the relay) with link impairment applied INSIDE the robot (and
# where needed, operator) container before the agent starts. The impairment
# commands, mode, seed, duration, and result are recorded as a JSON artifact
# per profile under artifacts/, so any run can be replayed.
#
# Determinism contract:
#   gate profiles use only fixed delay, tbf rate caps, iptables statistic-nth
#   loss, and route blocking — reproducible run-to-run. Chaos mode generates
#   netem random loss/jitter/reorder from a recorded seed: the PARAMETERS
#   replay exactly; netem's per-packet draw is kernel RNG, so a replay
#   reproduces the impairment distribution, not the packet-level trace.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
E2E_DIR="$SCRIPT_DIR/../e2e-dispatch"
ART_DIR="$SCRIPT_DIR/artifacts"
mkdir -p "$ART_DIR"

say() { printf '\033[1;36m==>\033[0m %s\n' "$1"; }

MODE="gate"
ONLY_PROFILE=""
SEED=""
while [ $# -gt 0 ]; do
  case "$1" in
    --profile) ONLY_PROFILE="$2"; shift 2 ;;
    --chaos)   MODE="chaos"; shift ;;
    --seed)    SEED="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

# Chaos seed: explicit, else derived from the epoch (recorded either way).
if [ "$MODE" = "chaos" ] && [ -z "$SEED" ]; then
  SEED="$(date +%s)"
fi

# Generate a randomized netem profile from the seed. awk srand(seed) makes
# the PARAMETER GENERATION deterministic per seed.
chaos_params() {
  awk -v seed="$1" 'BEGIN {
    srand(seed)
    delay  = 20 + int(rand() * 280)   # 20..300 ms
    jitter = int(rand() * 60)         # 0..60 ms
    loss   = rand() * 5               # 0..5 %
    reord  = rand() * 3               # 0..3 %
    printf "delay %dms %dms loss %.2f%% reorder %.2f%%", delay, jitter, loss, reord
  }'
}

json_escape() { printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'; }

run_one() {
  local name="$1" desc="$2" class="$3" robot_shape="$4" operator_shape="$5"
  local started dur result rc
  started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  say "profile '$name' ($class): $desc"
  [ -n "$robot_shape" ]    && say "  robot:    $robot_shape"
  [ -n "$operator_shape" ] && say "  operator: $operator_shape"

  # Gate profiles retry ONCE on failure — the same policy as the archetype
  # scenarios (run_scenario_with_retry): container startup on shared runners
  # is occasionally flaky, and a persistent regression still fails twice.
  # Chaos runs never retry (a chaos failure is the signal).
  local t0 t1 attempts note=""
  attempts=1
  [ "$class" = "gate" ] && attempts=2
  t0=$(date +%s)
  rc=1
  local try
  for try in $(seq 1 "$attempts"); do
    rc=0
    GANG_SHAPE_CMD="$robot_shape" GANG_SHAPE_OPERATOR_CMD="$operator_shape" \
      "$E2E_DIR/run-test.sh" || rc=$?
    if [ "$rc" -eq 0 ]; then
      [ "$try" -gt 1 ] && note=" (passed on retry)"
      break
    fi
    # Preserve the failing attempt's compose logs next to the artifacts.
    if [ -f "$E2E_DIR/test-data/compose.log" ]; then
      cp "$E2E_DIR/test-data/compose.log" \
        "$ART_DIR/$(date -u +%Y%m%dT%H%M%SZ)-$name-attempt$try-compose.log"
    fi
    [ "$try" -lt "$attempts" ] && { say "  attempt $try failed — retrying in 10s"; sleep 10; }
  done
  t1=$(date +%s)
  dur=$((t1 - t0))
  if [ "$rc" -eq 0 ]; then result="pass$note"; else result="fail"; fi

  local artifact ts
  ts="$(date -u +%Y%m%dT%H%M%SZ)"
  artifact="$ART_DIR/$ts-$name.json"
  cat > "$artifact" <<JSON
{
  "profile": "$name",
  "class": "$class",
  "mode": "$MODE",
  "seed": "${SEED:-null}",
  "description": "$(json_escape "$desc")",
  "robot_shape": "$(json_escape "$robot_shape")",
  "operator_shape": "$(json_escape "$operator_shape")",
  "started": "$started",
  "duration_secs": $dur,
  "result": "$result"
}
JSON
  say "  -> $result in ${dur}s (artifact: ${artifact#"$SCRIPT_DIR"/})"
  return "$rc"
}

FAILED=0
RAN=0

if [ "$MODE" = "chaos" ]; then
  PARAMS="$(chaos_params "$SEED")"
  say "chaos mode, seed=$SEED -> netem: $PARAMS"
  say "replay: ./run-matrix.sh --chaos --seed $SEED"
  run_one "chaos" "seeded random netem impairment" "chaos" \
    "tc qdisc add dev eth0 root netem $PARAMS" "" || FAILED=$((FAILED + 1))
  RAN=1
else
  for f in "$SCRIPT_DIR"/profiles/*.profile; do
    # Fixture variables, sourced per profile.
    PROFILE_NAME="" PROFILE_DESC="" PROFILE_CLASS="" ROBOT_SHAPE="" OPERATOR_SHAPE=""
    # shellcheck source=/dev/null
    . "$f"
    if [ -n "$ONLY_PROFILE" ] && [ "$PROFILE_NAME" != "$ONLY_PROFILE" ]; then
      continue
    fi
    run_one "$PROFILE_NAME" "$PROFILE_DESC" "$PROFILE_CLASS" \
      "$ROBOT_SHAPE" "$OPERATOR_SHAPE" || FAILED=$((FAILED + 1))
    RAN=$((RAN + 1))
    sleep 5  # let Docker networking settle between profiles
  done
  if [ "$RAN" -eq 0 ]; then
    echo "no profile matched '$ONLY_PROFILE'" >&2
    exit 2
  fi
fi

echo
if [ "$FAILED" -eq 0 ]; then
  say "degraded-link matrix: $RAN profile(s) PASSED"
else
  say "degraded-link matrix: $FAILED of $RAN profile(s) FAILED"
fi
exit "$([ "$FAILED" -eq 0 ] && echo 0 || echo 1)"
