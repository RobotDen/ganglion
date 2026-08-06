#!/usr/bin/env bash
# Configure git to use the project's hooks directory.
# Run once after cloning: ./scripts/setup-hooks.sh

set -e

REPO_ROOT="$(git rev-parse --show-toplevel)"
git config core.hooksPath "${REPO_ROOT}/.githooks"
echo "Git hooks configured: ${REPO_ROOT}/.githooks"
echo "  pre-commit: fmt check, clippy, tests (fast, per commit)"
echo "  pre-push:   the full CI gate — fmt, clippy -Dwarnings, rustdoc -Dwarnings,"
echo "              tests, cargo-deny; opt-in docker harness (GANG_PREPUSH_HARNESS=1)."
echo "              Skip once with GANG_SKIP_PREPUSH=1 git push (CI still gates)."
