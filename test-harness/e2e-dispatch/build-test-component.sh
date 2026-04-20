#!/usr/bin/env bash
# Build the test diagnostics WASM component for e2e testing.
# Requires: cargo-component, wasm32-wasip1 target
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
COMPONENT_DIR="$SCRIPT_DIR/test-component"
OUT_DIR="$SCRIPT_DIR/test-data"

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
