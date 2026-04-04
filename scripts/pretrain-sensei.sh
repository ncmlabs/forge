#!/usr/bin/env bash
# pretrain-sensei.sh — Bulk pre-training pipeline for forge-sensei
# Usage: bash scripts/pretrain-sensei.sh [--force] [--dry-run]
set -euo pipefail

FORGE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SENSEI_BIN="${SENSEI_BIN:-$FORGE_ROOT/bin/forge-sensei}"

# Auto-detect mock mode
if [ "${FORGE_MOCK:-}" = "1" ] && [ -z "${FORGE_CONFIG:-}" ]; then
  export FORGE_CONFIG="$FORGE_ROOT/config/mock.config.toml"
fi
MANIFEST_FILE="$FORGE_ROOT/.forge-knowledge/pretrain-manifest.sha256"
FORCE=false
DRY_RUN=false

for arg in "$@"; do
  case "$arg" in
    --force) FORCE=true ;;
    --dry-run) DRY_RUN=true ;;
  esac
done

if [ ! -x "$SENSEI_BIN" ] && [ "$DRY_RUN" = false ]; then
  echo "Error: forge-sensei binary not found at $SENSEI_BIN"
  echo "Run: bash scripts/build-sensei.sh"
  exit 1
fi

if ! command -v jq &>/dev/null; then
  echo "Error: jq required for JSON parsing. Install: brew install jq"
  exit 1
fi

# ── Collect all target files ──────────────────────────────────
FILES=()
collect() { [ -f "$1" ] && FILES+=("$1"); }

for f in docs/forge-reference.md forge-principles.md README.md roadmap.md providers.md CONTRIBUTING.md; do
  collect "$FORGE_ROOT/$f"
done
for f in "$FORGE_ROOT"/examples/*.forge; do collect "$f"; done
for f in "$FORGE_ROOT"/examples/tictactoe/*.forge; do collect "$f"; done
for f in "$FORGE_ROOT"/workflows/*.forge; do
  [ "$(basename "$f")" = "forge-sensei.forge" ] && continue
  collect "$f"
done
for f in "$FORGE_ROOT"/docs/superpowers/specs/*.md; do collect "$f"; done
for src in \
  "$FORGE_ROOT/src/checker/pure_checker.rs" \
  "$FORGE_ROOT/src/checker/uncertain_checker.rs" \
  "$FORGE_ROOT/src/checker/states_checker.rs" \
  "$FORGE_ROOT/src/checker/boundary_checker.rs" \
  "$FORGE_ROOT/src/checker/warden_checker.rs" \
  "$FORGE_ROOT/src/checker/requires_checker.rs" \
  "$FORGE_ROOT/src/runtime/knowledge_store.rs"; do
  collect "$src"
done

TOTAL_FILES=${#FILES[@]}
CONF_FILES=()
for f in "$FORGE_ROOT"/conformance/**/*.json; do
  [ "$(basename "$f")" = "schema.json" ] && continue
  [ -f "$f" ] && CONF_FILES+=("$f")
done
TOTAL_CONF=${#CONF_FILES[@]}
TOTAL=$((TOTAL_FILES + TOTAL_CONF))

# ── Idempotency check ────────────────────────────────────────
if [ "$FORCE" = false ] && [ "$DRY_RUN" = false ]; then
  CURRENT_MANIFEST=$(printf '%s\n' "${FILES[@]}" "${CONF_FILES[@]}" | sort | xargs shasum -a 256 2>/dev/null | shasum -a 256 | cut -d' ' -f1)
  if [ -f "$MANIFEST_FILE" ]; then
    CACHED_MANIFEST=$(cat "$MANIFEST_FILE")
    if [ "$CURRENT_MANIFEST" = "$CACHED_MANIFEST" ]; then
      echo "Knowledge base up to date (no changes since last pretrain)."
      echo "Use --force to re-ingest."
      exit 0
    fi
  fi
fi

if [ "$DRY_RUN" = true ]; then
  echo "=== Dry Run: $TOTAL items would be ingested ==="
  echo ""
  echo "Files ($TOTAL_FILES):"
  printf '  %s\n' "${FILES[@]}"
  echo ""
  echo "Conformance tests ($TOTAL_CONF):"
  printf '  %s\n' "${CONF_FILES[@]}"
  exit 0
fi

# ── Ingest functions ──────────────────────────────────────────
COUNT=0
FAILED=0
FAIL_LOG=()
PHASE_START=$SECONDS

ingest() {
  local path="$1"
  COUNT=$((COUNT + 1))
  local err
  if err=$("$SENSEI_BIN" ingest "$path" 2>&1); then
    printf "  [%d/%d] %s\n" "$COUNT" "$TOTAL" "$path"
  else
    FAILED=$((FAILED + 1))
    FAIL_LOG+=("FAIL: $path -- $err")
    printf "  [%d/%d] FAIL: %s\n" "$COUNT" "$TOTAL" "$path"
  fi
}

ingest_fact() {
  local category="$1"
  local fact="$2"
  COUNT=$((COUNT + 1))
  local err
  if err=$("$SENSEI_BIN" ingest-fact "$category" "$fact" 2>&1); then
    :
  else
    FAILED=$((FAILED + 1))
    FAIL_LOG+=("FAIL: [$category] -- $err")
  fi
}

phase_time() {
  local now=$SECONDS
  local elapsed=$((now - PHASE_START))
  PHASE_START=$now
  echo "  (${elapsed}s)"
}

echo "=== forge-sensei Pre-Training Pipeline ==="
echo "Total items: $TOTAL"
echo ""

# ── Phase 1: Core Documentation ──────────────────────────────
echo "Phase 1: Core documentation..."
for f in docs/forge-reference.md forge-principles.md README.md roadmap.md providers.md CONTRIBUTING.md; do
  [ -f "$FORGE_ROOT/$f" ] && ingest "$FORGE_ROOT/$f"
done
phase_time

# ── Phase 2: Example Programs ────────────────────────────────
echo "Phase 2: Example programs..."
for f in "$FORGE_ROOT"/examples/*.forge "$FORGE_ROOT"/examples/tictactoe/*.forge; do
  [ -f "$f" ] && ingest "$f"
done
phase_time

# ── Phase 3: Workflow Definitions ────────────────────────────
echo "Phase 3: Workflows..."
for f in "$FORGE_ROOT"/workflows/*.forge; do
  [ "$(basename "$f")" = "forge-sensei.forge" ] && continue
  [ -f "$f" ] && ingest "$f"
done
phase_time

# ── Phase 4: Conformance Tests ───────────────────────────────
echo "Phase 4: Conformance tests..."
for test_file in "${CONF_FILES[@]}"; do
  name=$(jq -r '.name // "unknown"' "$test_file")
  cat_val=$(jq -r '.category // "unknown"' "$test_file")
  outcome=$(jq -r '.expected.outcome // "unknown"' "$test_file")
  desc=$(jq -r '.description // ""' "$test_file")
  ingest_fact "CONFORMANCE" "Test '$name' (category: $cat_val) expects $outcome. $desc"
done
phase_time

# ── Phase 5: Design Specifications ───────────────────────────
echo "Phase 5: Design specifications..."
for spec in "$FORGE_ROOT"/docs/superpowers/specs/*.md; do
  [ -f "$spec" ] && ingest "$spec"
done
phase_time

# ── Phase 6: Key Source Modules ──────────────────────────────
echo "Phase 6: Key source modules..."
for src in \
  "$FORGE_ROOT/src/checker/pure_checker.rs" \
  "$FORGE_ROOT/src/checker/uncertain_checker.rs" \
  "$FORGE_ROOT/src/checker/states_checker.rs" \
  "$FORGE_ROOT/src/checker/boundary_checker.rs" \
  "$FORGE_ROOT/src/checker/warden_checker.rs" \
  "$FORGE_ROOT/src/checker/requires_checker.rs" \
  "$FORGE_ROOT/src/runtime/knowledge_store.rs"; do
  [ -f "$src" ] && ingest "$src"
done
phase_time

# ── Summary ───────────────────────────────────────────────────
SUCCEEDED=$((COUNT - FAILED))
PCTVAL=0
if [ "$COUNT" -gt 0 ]; then
  PCTVAL=$((SUCCEEDED * 100 / COUNT))
fi
echo ""
echo "=== Pre-training complete ==="
echo "Ingested: $SUCCEEDED/$COUNT ($PCTVAL%)"

if [ "$FAILED" -gt 0 ]; then
  echo ""
  echo "Failures ($FAILED):"
  for entry in "${FAIL_LOG[@]}"; do
    echo "  $entry"
  done
fi

# Save manifest for idempotency
mkdir -p "$(dirname "$MANIFEST_FILE")"
printf '%s\n' "${FILES[@]}" "${CONF_FILES[@]}" | sort | xargs shasum -a 256 2>/dev/null | shasum -a 256 | cut -d' ' -f1 > "$MANIFEST_FILE"
