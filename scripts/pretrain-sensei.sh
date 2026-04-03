#!/usr/bin/env bash
# pretrain-sensei.sh — Bulk pre-training pipeline for forge-sensei
# Ingests all FORGE documentation, examples, conformance tests, and key source files.
set -euo pipefail

FORGE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SENSEI_BIN="${SENSEI_BIN:-$FORGE_ROOT/bin/forge-sensei}"
COUNT=0

if [ ! -x "$SENSEI_BIN" ]; then
  echo "Error: forge-sensei binary not found at $SENSEI_BIN"
  echo "Run: bash scripts/build-sensei.sh"
  exit 1
fi

ingest() {
  local path="$1"
  if [ -f "$path" ]; then
    "$SENSEI_BIN" ingest "$path" 2>/dev/null || true
    COUNT=$((COUNT + 1))
    echo "  [$COUNT] $path"
  fi
}

ingest_fact() {
  local category="$1"
  local fact="$2"
  "$SENSEI_BIN" ingest-fact "$category" "$fact" 2>/dev/null || true
  COUNT=$((COUNT + 1))
}

echo "=== forge-sensei Pre-Training Pipeline ==="
echo ""

# ── Phase 1: Core Documentation ──────────────────────────────
echo "Phase 1: Core documentation..."
ingest "$FORGE_ROOT/docs/forge-reference.md"
ingest "$FORGE_ROOT/forge-principles.md"
ingest "$FORGE_ROOT/README.md"
ingest "$FORGE_ROOT/roadmap.md"
ingest "$FORGE_ROOT/providers.md"
ingest "$FORGE_ROOT/CONTRIBUTING.md"

# ── Phase 2: Example Programs ────────────────────────────────
echo ""
echo "Phase 2: Example programs..."
for f in "$FORGE_ROOT"/examples/*.forge; do
  ingest "$f"
done
for f in "$FORGE_ROOT"/examples/tictactoe/*.forge; do
  ingest "$f"
done

# ── Phase 3: Workflow Definitions ────────────────────────────
echo ""
echo "Phase 3: Workflows..."
for f in "$FORGE_ROOT"/workflows/*.forge; do
  # Skip sensei itself to avoid circular learning
  [ "$(basename "$f")" = "forge-sensei.forge" ] && continue
  ingest "$f"
done

# ── Phase 4: Conformance Tests ───────────────────────────────
echo ""
echo "Phase 4: Conformance tests..."
if command -v python3 &>/dev/null; then
  for test_file in "$FORGE_ROOT"/conformance/**/*.json; do
    [ "$(basename "$test_file")" = "schema.json" ] && continue
    [ ! -f "$test_file" ] && continue

    info=$(python3 -c "
import json, sys
d = json.load(open('$test_file'))
name = d.get('name', 'unknown')
cat = d.get('category', 'unknown')
desc = d.get('description', '')
outcome = d.get('expected', {}).get('outcome', 'unknown')
print(f'{name}|{cat}|{outcome}|{desc}')
" 2>/dev/null || echo "")

    if [ -n "$info" ]; then
      IFS='|' read -r name cat outcome desc <<< "$info"
      ingest_fact "CONFORMANCE" "Test '$name' (category: $cat) expects $outcome. $desc"
    fi
  done
else
  echo "  (skipping: python3 not found for JSON parsing)"
fi

# ── Phase 5: Design Specifications ───────────────────────────
echo ""
echo "Phase 5: Design specifications..."
for spec in "$FORGE_ROOT"/docs/superpowers/specs/*.md; do
  ingest "$spec"
done

# ── Phase 6: Key Source Modules ──────────────────────────────
echo ""
echo "Phase 6: Key source modules..."
for src in \
  "$FORGE_ROOT/src/checker/pure_checker.rs" \
  "$FORGE_ROOT/src/checker/uncertain_checker.rs" \
  "$FORGE_ROOT/src/checker/states_checker.rs" \
  "$FORGE_ROOT/src/checker/boundary_checker.rs" \
  "$FORGE_ROOT/src/checker/warden_checker.rs" \
  "$FORGE_ROOT/src/checker/requires_checker.rs" \
  "$FORGE_ROOT/src/runtime/knowledge_store.rs"; do
  ingest "$src"
done

echo ""
echo "=== Pre-training complete: $COUNT items ingested ==="
