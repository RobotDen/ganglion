#!/usr/bin/env bash
# Build the test diagnostics WASM component for e2e testing.
# Requires: cargo-component
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
COMPONENT_DIR="$SCRIPT_DIR/test-component"
OUT_DIR="$SCRIPT_DIR/test-data"

# Placeholder mode (GANG_E2E_PLACEHOLDER=1): skip the WASM toolchain and ship
# placeholder bytes instead of a real component. The robot agent executes
# these via the direct-broker path, so the RELAY TRANSPORT semantics the
# degraded-link matrix exercises are identical — only the in-sandbox WASM
# execution (which netem never touches) is skipped. For environments without
# cargo-component/wasm targets. GANG_BIN can point at a prebuilt gang binary
# to skip cargo run for signing.
if [ -n "${GANG_E2E_PLACEHOLDER:-}" ]; then
    echo "Placeholder mode: skipping WASM build (transport semantics unchanged)"
    mkdir -p "$OUT_DIR"
    printf "placeholder-not-wasm" > "$OUT_DIR/diagnostics.wasm"
    touch "$OUT_DIR/placeholder-mode"
    cd "$SCRIPT_DIR/../.."
    GANG="${GANG_BIN:-cargo run --bin gang --}"
    $GANG sign "$OUT_DIR/diagnostics.wasm" --name diagnostics --component-version 0.1.0 \
        --capabilities diagnostics
    echo "Signed placeholder: $OUT_DIR/diagnostics.wasm"
    exit 0
fi
rm -f "$OUT_DIR/placeholder-mode"

# The workspace rust-toolchain.toml only installs the wasm32-wasip2 target,
# but cargo-component builds a wasm32-wasip1 core module and adapts it into
# a component (wasip2 support in cargo-component is not something we can
# count on). Install the wasip1 target here so this script works regardless
# of what the toolchain file provisions.
echo "Ensuring wasm32-wasip1 target is installed..."
rustup target add wasm32-wasip1

echo "Building test diagnostics WASM component..."
cd "$COMPONENT_DIR"
cargo component build --release

mkdir -p "$OUT_DIR"
cp target/wasm32-wasip1/release/test_diagnostics_component.wasm "$OUT_DIR/diagnostics.wasm"

echo "Signing component..."
cd "$SCRIPT_DIR/../.."

# Generate a test keypair if not present
TEST_KEY="$OUT_DIR/test-operator.key"
if [ ! -f "$TEST_KEY" ]; then
    cargo run --bin gang -- identity generate 2>/dev/null || true
fi

cargo run --bin gang -- sign "$OUT_DIR/diagnostics.wasm" \
    --name diagnostics \
    --version 0.1.0

echo "Built and signed: $OUT_DIR/diagnostics.wasm"
echo "Manifest: $OUT_DIR/diagnostics.manifest.cbor"
ls -la "$OUT_DIR/"
