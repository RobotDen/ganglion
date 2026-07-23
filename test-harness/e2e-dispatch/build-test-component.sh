#!/usr/bin/env bash
# Build the test diagnostics WASM component for e2e testing.
# Requires: cargo-component
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
COMPONENT_DIR="$SCRIPT_DIR/test-component"
OUT_DIR="$SCRIPT_DIR/test-data"

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
