#!/usr/bin/env bash
# Seed the open capability registry with the eight in-tree capability crates.
#
#   ./scripts/seed-registry.sh [--sign-key <path>] [--dry-run]
#
# For each `crates/gang-capability-*` crate this script:
#   1. builds it as a WASM component (cargo-component, wasm32-wasip2),
#   2. signs it with your identity key (`gang sign`), which writes the
#      `.manifest.cbor` with the crate's declared capabilities, and
#   3. publishes it to the local open registry (`gang registry publish`),
#      making it discoverable via `gang registry search`/`info`/`install`.
#
# Prerequisites (one-time):
#   rustup target add wasm32-wasip2
#   cargo install cargo-component
#   gang identity generate          # if ~/.gang/identity.key does not exist
#
# The author recorded on every published entry is the signing identity, so run
# this on the machine that holds the project's operator key.
set -eu

say()  { printf '\033[1;36m==>\033[0m %s\n' "$1"; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$1" >&2; exit 1; }

SIGN_KEY=""
DRY_RUN=0
while [ $# -gt 0 ]; do
  case "$1" in
    --sign-key) SIGN_KEY="$2"; shift 2 ;;
    --dry-run)  DRY_RUN=1; shift ;;
    *) die "unknown argument: $1" ;;
  esac
done

if [ "$DRY_RUN" -eq 0 ]; then
  command -v cargo-component >/dev/null 2>&1 || die "cargo-component not found (cargo install cargo-component)"
  command -v gang >/dev/null 2>&1 || die "gang not found on PATH (cargo install --path crates/gang-cli)"
  # cargo-component builds for wasm32-wasip1 by default (wasip2 also works on
  # newer versions); accept either being installed.
  rustup target list --installed 2>/dev/null | grep -qE 'wasm32-wasip[12]|wasm32-wasi$' \
    || die "no wasm32-wasi* target installed (rustup target add wasm32-wasip1)"
  # A crate can only produce a .wasm if it builds as a cdylib. The capability
  # crates are currently rlib-only logic libraries — componentization (cdylib +
  # WIT guest bindings) is tracked in issue #28. Fail with the real story
  # rather than a confusing empty-find later.
  if ! grep -q 'cdylib' crates/gang-capability-diagnostics/Cargo.toml 2>/dev/null; then
    die "capability crates are not componentized yet (rlib-only; no cdylib/WIT \
guest bindings) — cargo-component emits .rlib, never .wasm. See issue #28."
  fi
fi

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

# Declared capability groups per crate — must match each crate's manifest
# intent. Passed to `gang sign --capabilities` so the signed manifest declares
# exactly what the component needs, nothing more.
caps_for() {
  case "$1" in
    gang-capability-diagnostics)       echo "diagnostics" ;;
    gang-capability-param-inspect)     echo "ros" ;;
    gang-capability-diagnostic-bundle) echo "diagnostics,logs,artifacts" ;;
    gang-capability-network-archetype) echo "network,diagnostics" ;;
    gang-capability-log-normalize)     echo "logs" ;;
    gang-capability-topic-echo)        echo "ros,artifacts" ;;
    gang-capability-canary-probe)      echo "ros,metrics" ;;
    gang-capability-rosbag-slice)      echo "fs,process,artifacts" ;;
    *) die "no capability mapping for $1" ;;
  esac
}

version="$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)"/\1/')"
outdir="$repo_root/target/registry-seed"
mkdir -p "$outdir"

published=0
for dir in crates/gang-capability-*/; do
  crate="$(basename "$dir")"
  caps="$(caps_for "$crate")"
  say "$crate (capabilities: $caps)"

  if [ "$DRY_RUN" -eq 1 ]; then
    continue
  fi

  # 1. Build the component.
  (cd "$dir" && cargo component build --release --quiet)
  # cargo-component's output target dir varies by version (wasip1 is the
  # long-standing default; wasip2 on newer releases; wasm32-wasi historically),
  # and in a workspace the artifact lands in the WORKSPACE root target/ (or
  # $CARGO_TARGET_DIR), never the crate-local one. Search all candidates.
  wasm=""
  base="${CARGO_TARGET_DIR:-$repo_root/target}"
  for tgt in wasm32-wasip2 wasm32-wasip1 wasm32-wasi; do
    for root in "$base" "$dir/target"; do
      candidate="$(find "$root/$tgt/release" -maxdepth 1 -name "${crate//-/_}.wasm" 2>/dev/null | head -1)"
      if [ -n "$candidate" ]; then wasm="$candidate"; break 2; fi
    done
  done
  [ -n "$wasm" ] || die "no component produced for $crate (searched $base/wasm32-wasip{1,2}/release)"
  cp "$wasm" "$outdir/$crate.component.wasm"

  # 2. Sign (writes $outdir/$crate.component.manifest.cbor).
  sign_args=(sign "$outdir/$crate.component.wasm"
             --name "${crate#gang-capability-}"
             --component-version "$version"
             --capabilities "$caps")
  [ -n "$SIGN_KEY" ] && sign_args+=(--key "$SIGN_KEY")
  gang "${sign_args[@]}"

  # 3. Publish to the open registry.
  gang registry publish "$outdir/$crate.component.wasm" \
    --description "$(grep -m1 '^description' "$dir/Cargo.toml" | sed 's/.*"\(.*\)"/\1/')" \
    --tags "seed,official"
  published=$((published + 1))
done

if [ "$DRY_RUN" -eq 1 ]; then
  say "dry run complete — 8 crates mapped, nothing built"
else
  say "published $published capabilities. Verify: gang registry list"
fi
