#!/usr/bin/env bash
# Install local git hooks for FORGE.
# Run once after cloning: bash scripts/install-hooks.sh
#
# Why this exists:
# - .claude/settings.json wires check-security.sh to commits made via Claude Code.
# - That hook does NOT fire when you commit from a regular terminal.
# - This script installs a real .git/hooks/pre-commit that calls the same script,
#   so the protection works regardless of how you commit.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
HOOK_DIR="$REPO_ROOT/.git/hooks"
HOOK_FILE="$HOOK_DIR/pre-commit"
SCANNER="$REPO_ROOT/scripts/check-security.sh"

if [ ! -f "$SCANNER" ]; then
  echo "error: scanner not found at $SCANNER" >&2
  exit 1
fi

mkdir -p "$HOOK_DIR"

cat > "$HOOK_FILE" <<'HOOK'
#!/usr/bin/env bash
# Auto-installed by scripts/install-hooks.sh — do not edit by hand.
# Runs the FORGE security scanner on staged content. Blocks commit on violation.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
SCANNER="$REPO_ROOT/scripts/check-security.sh"

if [ ! -x "$SCANNER" ]; then
  echo "warning: $SCANNER missing or not executable — skipping security scan" >&2
  exit 0
fi

# The scanner emits {"continue": false, "stopReason": "..."} on violation, exits 0.
# We parse that and block the commit ourselves.
output="$("$SCANNER" 2>&1 || true)"

if echo "$output" | grep -q '"continue": false'; then
  echo "" >&2
  echo "──── COMMIT BLOCKED — security scanner found violations ────" >&2
  reason=$(echo "$output" | python3 -c 'import sys,json; print(json.load(sys.stdin).get("stopReason",""))' 2>/dev/null || echo "$output")
  echo "$reason" >&2
  echo "" >&2
  echo "If this is a false positive, add the file to .forge-security-allow" >&2
  echo "or use 'git commit --no-verify' (NOT recommended)." >&2
  exit 1
fi

exit 0
HOOK

chmod +x "$HOOK_FILE"

echo "✓ Installed pre-commit hook at $HOOK_FILE"
echo "  → runs $SCANNER on every commit"
echo ""
echo "To verify: bash scripts/test-check-security.sh"
