#!/usr/bin/env bash
# consult-sensei.sh — PreToolUse hook: consults forge-sensei before .forge file edits
# Reads the tool input from stdin (Claude Code hook protocol).
set -euo pipefail
trap 'exit 0' ERR

FORGE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SENSEI_BIN="$FORGE_ROOT/bin/forge-sensei"
CACHE_DIR="/tmp/forge-sensei-cache"

# Skip if binary not built yet
if [ ! -x "$SENSEI_BIN" ]; then
  exit 0
fi

# Skip if jq not available
if ! command -v jq &>/dev/null; then
  exit 0
fi

# Read the hook input JSON from stdin
INPUT=$(cat)

# Extract the file path from the tool input
FILE_PATH=$(echo "$INPUT" | jq -r '.tool_input.file_path // ""')

# Only consult sensei for .forge files
if [[ "$FILE_PATH" != *.forge ]]; then
  exit 0
fi

# Extract what's being written/edited for context
CONTENT=$(echo "$INPUT" | jq -r '(.tool_input.new_string // .tool_input.content // "")[:500]')

# Skip trivial edits (less than 20 chars)
if [ ${#CONTENT} -lt 20 ]; then
  exit 0
fi

# Check cache (5-minute TTL)
mkdir -p "$CACHE_DIR"
HASH=$(echo "$CONTENT" | shasum -a 256 | cut -d' ' -f1)
CACHE_FILE="$CACHE_DIR/$HASH"
if [ -f "$CACHE_FILE" ]; then
  AGE=$(( $(date +%s) - $(stat -f%m "$CACHE_FILE" 2>/dev/null || stat -c%Y "$CACHE_FILE" 2>/dev/null || echo 0) ))
  if [ "$AGE" -lt 300 ]; then
    ADVICE=$(cat "$CACHE_FILE")
    if [ -n "$ADVICE" ] && [ "$ADVICE" != "null" ]; then
      ESCAPED=$(echo "$ADVICE" | jq -Rs '.')
      printf '{"message": "forge-sensei notes: %s"}\n' "$ESCAPED"
    fi
    exit 0
  fi
fi

# Query sensei for advice on this code
ADVICE=$("$SENSEI_BIN" review "$CONTENT" 2>/dev/null || echo "")

# Cache the result
if [ -n "$ADVICE" ]; then
  echo "$ADVICE" > "$CACHE_FILE"
fi

if [ -n "$ADVICE" ] && [ "$ADVICE" != "null" ]; then
  ESCAPED=$(echo "$ADVICE" | jq -Rs '.')
  printf '{"message": "forge-sensei notes: %s"}\n' "$ESCAPED"
fi

exit 0
