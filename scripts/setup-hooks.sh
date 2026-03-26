#!/usr/bin/env bash
# Configure git to use the project's hooks directory.
# Run once after cloning: ./scripts/setup-hooks.sh

set -e

REPO_ROOT="$(git rev-parse --show-toplevel)"
git config core.hooksPath "${REPO_ROOT}/.githooks"
echo "Git hooks configured: ${REPO_ROOT}/.githooks"
echo "Pre-commit hook will run: fmt check, clippy, tests"
