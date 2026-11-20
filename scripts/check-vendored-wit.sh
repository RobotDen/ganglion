#!/usr/bin/env bash
# Check that every capability crate's vendored wit/ganglion.wit matches the
# canonical copy in gang-wasm-host, ignoring the vendoring header (leading
# `//` comment lines). Run by .githooks/pre-commit and by CI.
#
# To fix a drift, re-copy the canonical WIT below each vendored file's
# 3-line "VENDORED from …" header.
set -euo pipefail

cd "$(dirname "$0")/.."
canonical="crates/gang-wasm-host/wit/ganglion.wit"
status=0
for vendored in crates/gang-capability-*/wit/ganglion.wit; do
    if ! diff -u <(grep -v '^//' "$canonical") <(grep -v '^//' "$vendored") \
        --label "$canonical" --label "$vendored"; then
        echo "Vendored WIT out of sync: $vendored differs from $canonical." >&2
        status=1
    fi
done
if [ "$status" -ne 0 ]; then
    echo "Re-copy the canonical WIT (keep the vendoring header)." >&2
fi
exit "$status"
