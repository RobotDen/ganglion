#!/usr/bin/env bash
# Seed the open capability registry with the eight in-tree capability crates.
#
#   ./scripts/seed-registry.sh [--sign-key <path>] [--dry-run]
#
# For each `crates/gang-capability-*` crate this script:
#   1. builds it as a WASM component (`cargo build --target wasm32-wasip2`;
#      rustc emits a component directly for cdylib crates — no cargo-component),
#   2. signs it with your identity key (`gang sign`), which writes the
#      `.manifest.cbor` with the crate's declared capabilities, and
#   3. publishes it to the local open registry (`gang registry publish`),
#      making it discoverable via `gang registry search`/`info`/`install`.
#
# Prerequisites (one-time):
#   rustup target add wasm32-wasip2
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
  command -v gang >/dev/null 2>&1 || die "gang not found on PATH (cargo install --path crates/gang-cli)"
  # The wasm32-wasip2 target produces components directly from cdylib crates;
  # no cargo-component needed.
  rustup target list --installed 2>/dev/null | grep -q 'wasm32-wasip2' \
    || die "wasm32-wasip2 target missing (rustup target add wasm32-wasip2)"
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
    gang-capability-canary-probe)      echo "diagnostics,ros,metrics" ;;
    gang-capability-rosbag-slice)      echo "fs,process,artifacts" ;;
    *) die "no capability mapping for $1" ;;
  esac
}

version="$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)"/\1/')"
outdir="$repo_root/target/registry-seed"
mkdir -p "$outdir"

published=0
skipped=0
for dir in crates/gang-capability-*/; do
  crate="$(basename "$dir")"
  caps="$(caps_for "$crate")"
  say "$crate (capabilities: $caps)"

  if [ "$DRY_RUN" -eq 1 ]; then
    continue
  fi

  # 1. Build the component. The wasm32-wasip2 target emits a WASM *component*
  # directly for cdylib crates (rustc links via wasm-component-ld) — no
  # cargo-component or adapter step needed.
  cargo build -p "$crate" --release --target wasm32-wasip2 --quiet
  base="${CARGO_TARGET_DIR:-$repo_root/target}"
  wasm="$base/wasm32-wasip2/release/${crate//-/_}.wasm"
  [ -f "$wasm" ] || die "no component produced for $crate (expected $wasm)"
  cp "$wasm" "$outdir/$crate.component.wasm"

  short="${crate#gang-capability-}"

  # Idempotency: a re-run after a partial failure must not die on crates that
  # already made it into the registry.
  if gang registry info "$short" 2>/dev/null | grep -q "v${version}"; then
    say "  $short@$version already published — skipping"
    skipped=$((skipped + 1))
    continue
  fi

  # 2. Sign (writes $outdir/$crate.component.manifest.cbor).
  sign_args=(sign "$outdir/$crate.component.wasm"
             --name "$short"
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
  say "published $published capabilities (skipped $skipped already-published). Verify: gang registry list"
fi
