#!/usr/bin/env bash
# Build the vgi-symbols worker and run the SQLLogic E2E tests against it using
# the haybarn DuckDB unittest runner (which ships the `vgi` community extension).
#
# Prerequisites (one-time):
#   uv tool install haybarn-unittest
#   echo "INSTALL vgi FROM community;" | uvx haybarn-cli
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$REPO_ROOT"

UNITTEST="${VGI_UNITTEST:-$(command -v haybarn-unittest || true)}"
if [[ -z "$UNITTEST" || ! -x "$UNITTEST" ]]; then
    echo "ERROR: haybarn-unittest not found. Install it with:" >&2
    echo "       uv tool install haybarn-unittest" >&2
    exit 1
fi

# Ensure the vgi community extension is installed for this haybarn version.
if ! echo "LOAD vgi;" | uvx haybarn-cli >/dev/null 2>&1; then
    echo "==> Installing vgi extension from community repository"
    echo "INSTALL vgi FROM community;" | uvx haybarn-cli
fi

echo "==> Building symbols-worker (release)"
cargo build --release --bin symbols-worker

WORKER="$REPO_ROOT/target/release/symbols-worker"
SYMBOL_DIR="$REPO_ROOT/test/symbols"
# NOTE: Catch2 test-name filter, not a shell glob; only a trailing `*` works.
TEST_GLOB="${1:-test/sql/*}"

echo "==> Running SQLLogic tests"
echo "    worker:     $WORKER"
echo "    symbol dir: $SYMBOL_DIR"
echo "    unittest:   $UNITTEST"
echo "    tests:      $TEST_GLOB"

VGI_SYMBOLS_WORKER="$WORKER" \
VGI_SYMBOLS_DIR="$SYMBOL_DIR" \
VGI_WORKER_CATALOG_NAME="symbols" \
    "$UNITTEST" --test-dir "$REPO_ROOT" "$TEST_GLOB"
