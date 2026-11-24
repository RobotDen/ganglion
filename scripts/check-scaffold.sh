#!/usr/bin/env bash
# Build the Rust capability scaffold and run it end to end, so the template
# can't silently stop producing a working capability.
#
#   gang capability scaffold → make test → make component → gang sign →
#   deploy and `gang run` over a localhost relay on a default-deny robot
#   (set up as in docs/QUICKSTART.md step 6).
#
# Usage: scripts/check-scaffold.sh [path/to/gang]   (default: target/debug/gang)
# The scaffold builds with the toolchain its own rust-toolchain.toml pins;
# rustup installs it if needed.
set -euo pipefail

GANG=$(realpath "${1:-target/debug/gang}")
WORK=$(mktemp -d)
PIDS=()
cleanup() {
    for pid in "${PIDS[@]}"; do kill "$pid" 2>/dev/null || true; done
    wait 2>/dev/null || true
}
trap cleanup EXIT

export DO_NOT_TRACK=1 GANG_TELEMETRY=off NO_COLOR=1
# A toolchain override (as the MSRV job sets) would bypass the scaffold's pin.
unset RUSTUP_TOOLCHAIN CARGO_TARGET_DIR

as() { local who=$1; shift; HOME="$WORK/$who" GANG_HOME="$WORK/$who/.gang" "$GANG" "$@"; }
step() { echo "=== check-scaffold: $* ==="; }
wait_for() { # file pattern
    for _ in $(seq 1 60); do grep -q "$2" "$1" && return 0; sleep 1; done
    echo "timed out waiting for '$2' in $1:"; cat "$1"; return 1
}
mkdir -p "$WORK/op" "$WORK/robot" "$WORK/relay"

step "scaffold"
as op capability scaffold my-tool --language rust --output-dir "$WORK"
cd "$WORK/my-tool"

step "make test"
make test

step "make component (toolchain $(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml))"
make component
test -s my-tool.component.wasm

step "sign"
as op identity generate >/dev/null
as op config set host_key_policy tofu >/dev/null
# What `make sign` runs, with the operator's GANG_HOME.
as op sign my-tool.component.wasm --name my-tool --version 0.1.0 --capabilities none

step "relay and default-deny robot"
PORT=$((20000 + RANDOM % 20000))
as relay identity generate >/dev/null
as relay relay --port "$PORT" > "$WORK/relay.log" 2>&1 &
PIDS+=($!)
wait_for "$WORK/relay.log" "Peer ID (libp2p/dial):"
RELAY=/ip4/127.0.0.1/tcp/$PORT/p2p/$(awk '/Peer ID \(libp2p\/dial\):/ {print $NF; exit}' "$WORK/relay.log")
OPERATOR=$(as op identity show | grep -oE '12D3-[0-9a-f]{32}' | head -1)

as robot init --yes >/dev/null
as robot policy allow-peer "$OPERATOR" >/dev/null
as robot agent -r "$RELAY" > "$WORK/agent.log" 2>&1 &
PIDS+=($!)
wait_for "$WORK/agent.log" "Relay circuit reservation established"
ROBOT=$(awk '/Peer ID \(libp2p\/dial\):/ {print $NF; exit}' "$WORK/agent.log")
as op peer add scaffold-robot "$ROBOT" --relay "$RELAY" >/dev/null

step "deploy and run"
as op deploy scaffold-robot my-tool.component.wasm --timeout 60
OUT=$(as op run scaffold-robot my-tool --timeout 30)
echo "$OUT"
grep -q '"status": "ok"' <<<"$OUT" || { echo "unexpected output from gang run"; exit 1; }

step "ok"
