#!/usr/bin/env bash
# sensei-smoke-test.sh — Quick end-to-end integration test for forge-sensei
set -euo pipefail

FORGE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SENSEI_BIN="$FORGE_ROOT/bin/forge-sensei"
PASSED=0
TOTAL=0

check() {
  TOTAL=$((TOTAL + 1))
  local name="$1"
  shift
  if output=$("$@" 2>&1); then
    PASSED=$((PASSED + 1))
    echo "  PASS: $name"
  else
    echo "  FAIL: $name"
    echo "    $output"
  fi
}

echo "=== forge-sensei Smoke Test ==="
echo ""

# Build if needed
if [ ! -x "$SENSEI_BIN" ]; then
  echo "Building forge-sensei..."
  bash "$FORGE_ROOT/scripts/build-sensei.sh"
fi

# Test each core operation
check "status" "$SENSEI_BIN" status
check "ingest" "$SENSEI_BIN" ingest "$FORGE_ROOT/README.md"
check "query" "$SENSEI_BIN" query "What is FORGE?"
check "review" "$SENSEI_BIN" review "pure bad\n  do\n    give 42"
check "ingest-fact" "$SENSEI_BIN" ingest-fact "TEST" "This is a smoke test fact"

echo ""
echo "=== Results: $PASSED/$TOTAL passed ==="
[ "$PASSED" -eq "$TOTAL" ] && exit 0 || exit 1
