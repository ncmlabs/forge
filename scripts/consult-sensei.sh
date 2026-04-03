#!/usr/bin/env bash
# consult-sensei.sh — PreToolUse hook: consults forge-sensei before .forge file edits
# Reads the tool input from stdin (Claude Code hook protocol).
set -euo pipefail

FORGE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SENSEI_BIN="$FORGE_ROOT/bin/forge-sensei"

# Skip if binary not built yet
if [ ! -x "$SENSEI_BIN" ]; then
  exit 0
fi

# Read the hook input JSON from stdin
INPUT=$(cat)

# Extract the file path from the tool input
FILE_PATH=$(echo "$INPUT" | python3 -c "
import json, sys
data = json.load(sys.stdin)
inp = data.get('tool_input', {})
print(inp.get('file_path', ''))
" 2>/dev/null || echo "")

# Only consult sensei for .forge files
if [[ "$FILE_PATH" != *.forge ]]; then
  exit 0
fi

# Extract what's being written/edited for context
CONTENT=$(echo "$INPUT" | python3 -c "
import json, sys
data = json.load(sys.stdin)
inp = data.get('tool_input', {})
text = inp.get('new_string', inp.get('content', ''))
print(text[:500])
" 2>/dev/null || echo "")

if [ -z "$CONTENT" ]; then
  exit 0
fi

# Query sensei for advice on this code
ADVICE=$("$SENSEI_BIN" review "$CONTENT" 2>/dev/null || echo "")

if [ -n "$ADVICE" ] && [ "$ADVICE" != "null" ]; then
  # Inject sensei's advice as a non-blocking note
  ESCAPED=$(echo "$ADVICE" | python3 -c 'import sys,json; print(json.dumps(sys.stdin.read().strip()))' 2>/dev/null || echo '""')
  printf '{"message": "forge-sensei notes: %s"}\n' "$ESCAPED"
fi

exit 0
