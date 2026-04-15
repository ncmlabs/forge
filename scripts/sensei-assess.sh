#!/usr/bin/env bash
# sensei-assess.sh — Run forge-sensei through all conformance tests and report mastery
#
# Single source of truth for the mastery assessment pipeline. Moved from
# ~/.claude/skills/forge-sensei/assess.sh as part of #240 closure so a fresh
# checkout can reproduce the novice → ingest → assess → apprentice flow.
#
# Usage: bash scripts/sensei-assess.sh [--json]
#
# Env:
#   FORGE_ROOT    Repo root (auto-detected via git or script location)
#   SENSEI_BIN    forge-sensei CLI (defaults to installed wrapper or repo binary)
#   SENSEI_DIR    Sensei runtime dir (defaults to ~/.forge/sensei)
set -uo pipefail

# ── Path resolution ───────────────────────────────────────────
FORGE_ROOT="${FORGE_ROOT:-}"
if [ -z "$FORGE_ROOT" ]; then
  FORGE_ROOT=$(git rev-parse --show-toplevel 2>/dev/null || true)
fi
if [ -z "$FORGE_ROOT" ]; then
  FORGE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
fi
if [ ! -d "$FORGE_ROOT/conformance" ]; then
  echo "Error: Cannot locate FORGE root (no conformance/ dir). Set FORGE_ROOT env var."
  exit 1
fi

# Prefer installed wrapper (has FORGE_CONFIG + server routing), fallback to repo binary
if [ -z "${SENSEI_BIN:-}" ]; then
  if [ -x "$HOME/.forge/bin/forge-sensei" ]; then
    SENSEI_BIN="$HOME/.forge/bin/forge-sensei"
  else
    SENSEI_BIN="$FORGE_ROOT/bin/forge-sensei"
  fi
fi
JSON_FLAG=false
for arg in "$@"; do
  [ "$arg" = "--json" ] && JSON_FLAG=true
done

if [ ! -x "$SENSEI_BIN" ]; then
  echo "Error: forge-sensei binary not found at $SENSEI_BIN"
  echo "Run: bash scripts/build-sensei.sh"
  exit 1
fi

if ! command -v jq &>/dev/null; then
  echo "Error: jq required. Install: brew install jq"
  exit 1
fi

PASSED=0
TOTAL=0
GAPS=""

# Per-category tracking (bash 4+ associative arrays)
declare -A CAT_PASSED CAT_TOTAL

echo "=== forge-sensei Mastery Assessment ==="
echo ""

for test_file in "$FORGE_ROOT"/conformance/**/*.json; do
  [ "$(basename "$test_file")" = "schema.json" ] && continue
  [ ! -f "$test_file" ] && continue
  TOTAL=$((TOTAL + 1))

  name=$(jq -r '.name' "$test_file")
  cat_val=$(jq -r '.category' "$test_file")
  input=$(jq -r 'if .input | type == "string" then .input else (.input | tostring) end' "$test_file" | head -c 1000)
  expected=$(jq -r '.expected.outcome' "$test_file")

  # Track per-category
  CAT_TOTAL[$cat_val]=$(( ${CAT_TOTAL[$cat_val]:-0} + 1 ))

  if [ -z "$input" ]; then
    echo "  SKIP: $name (no input)"
    continue
  fi

  result=$("$SENSEI_BIN" assess-detailed "$input" "$expected" 2>/dev/null || echo "GAP: runtime error")

  if echo "$result" | grep -qi "GAP:"; then
    gap_topic=$(echo "$result" | grep -oi "GAP: [^.]*" | head -1)
    GAPS="${GAPS}\n  - ${name}: ${gap_topic}"
    echo "  MISS: $name"
  else
    PASSED=$((PASSED + 1))
    CAT_PASSED[$cat_val]=$(( ${CAT_PASSED[$cat_val]:-0} + 1 ))
    echo "  PASS: $name"
  fi
done

if [ "$TOTAL" -eq 0 ]; then
  echo "No conformance tests found."
  exit 1
fi

SCORE=$((PASSED * 100 / TOTAL))

if [ "$SCORE" -ge 90 ]; then LEVEL="expert"
elif [ "$SCORE" -ge 70 ]; then LEVEL="journeyman"
elif [ "$SCORE" -ge 40 ]; then LEVEL="apprentice"
else LEVEL="novice"
fi

# ── Update mastery in the agent ───────────────────────────────
# Advance through mastery levels iteratively (FSM allows one step per call)
for _step in 1 2 3; do
  "$SENSEI_BIN" update-mastery "$SCORE" 2>/dev/null || true
done

# ── Trend tracking ────────────────────────────────────────────
SENSEI_DIR="${SENSEI_DIR:-$HOME/.forge/sensei}"
HISTORY_FILE="$SENSEI_DIR/assessment-history.jsonl"
mkdir -p "$(dirname "$HISTORY_FILE")"
TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

# Build category JSON
CAT_JSON="{"
for cat in "${!CAT_TOTAL[@]}"; do
  cat_p=${CAT_PASSED[$cat]:-0}
  cat_t=${CAT_TOTAL[$cat]}
  CAT_JSON+="\"$cat\":$((cat_p * 100 / cat_t)),"
done
CAT_JSON="${CAT_JSON%,}}"

echo "{\"timestamp\":\"$TIMESTAMP\",\"score\":$SCORE,\"passed\":$PASSED,\"total\":$TOTAL,\"level\":\"$LEVEL\",\"by_category\":$CAT_JSON}" >> "$HISTORY_FILE"

# Show trend
TREND_MSG=""
if [ -f "$HISTORY_FILE" ] && [ "$(wc -l < "$HISTORY_FILE")" -gt 1 ]; then
  PREV_SCORE=$(tail -2 "$HISTORY_FILE" | head -1 | jq -r '.score')
  DELTA=$((SCORE - PREV_SCORE))
  if [ "$DELTA" -ge 0 ]; then
    TREND_MSG="Trend: ${PREV_SCORE}% -> ${SCORE}% (+${DELTA}%)"
  else
    TREND_MSG="Trend: ${PREV_SCORE}% -> ${SCORE}% (${DELTA}%)"
  fi
fi

# ── Report ────────────────────────────────────────────────────
echo ""
echo "=== Assessment Report ==="
echo "Score: ${PASSED}/${TOTAL} (${SCORE}%)"
echo "Level: ${LEVEL}"
[ -n "${TREND_MSG}" ] && echo "$TREND_MSG"

echo ""
echo "Category Scores:"
for cat in $(echo "${!CAT_TOTAL[@]}" | tr ' ' '\n' | sort); do
  cat_p=${CAT_PASSED[$cat]:-0}
  cat_t=${CAT_TOTAL[$cat]}
  cat_s=$((cat_p * 100 / cat_t))
  printf "  %-12s %d/%d (%d%%)\n" "$cat:" "$cat_p" "$cat_t" "$cat_s"
done

if [ -n "$GAPS" ]; then
  printf "\nKnowledge Gaps:%b\n" "$GAPS"
fi
echo ""
echo "Thresholds: novice(<40%) | apprentice(40-69%) | journeyman(70-89%) | expert(90%+)"

# ── JSON output ───────────────────────────────────────────────
if [ "$JSON_FLAG" = true ]; then
  echo ""
  echo "--- JSON ---"
  echo "{\"timestamp\":\"$TIMESTAMP\",\"score\":$SCORE,\"passed\":$PASSED,\"total\":$TOTAL,\"level\":\"$LEVEL\",\"by_category\":$CAT_JSON}"
fi

# ── Write state.json for health check ────────────────────────
# state.json is an advisory summary of the last assessment. redb (owned by the
# daemon) is the source of truth for mastery. We prefer the daemon's reported
# level, but when it contradicts the score we just pushed (e.g., daemon down
# during update_mastery), we fall back to the score-derived level so state.json
# stays internally consistent (score + level always agree).
ENTRIES=$(python3 -c "import json; print(len(json.load(open('$SENSEI_DIR/knowledge.json'))))" 2>/dev/null || echo 0)
ACTUAL_LEVEL=$("$SENSEI_BIN" status 2>/dev/null | grep -oi 'novice\|apprentice\|journeyman\|expert' | head -1 || echo "")
# Sanity check: if daemon reports novice but score is apprentice+, the update
# didn't stick — trust the score we just computed and pushed.
if [ -z "$ACTUAL_LEVEL" ] || { [ "$SCORE" -ge 40 ] && [ "$ACTUAL_LEVEL" = "novice" ]; }; then
  ACTUAL_LEVEL="$LEVEL"
fi
cat > "$SENSEI_DIR/state.json" <<STATEJSON
{"last_eval":"$(date +%Y-%m-%dT%H:%M:%S)","score":$SCORE,"level":"$ACTUAL_LEVEL","entries":$ENTRIES}
STATEJSON
