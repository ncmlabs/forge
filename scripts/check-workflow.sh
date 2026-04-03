#!/usr/bin/env bash
# FORGE Workflow Gate Check
# Called by Claude Code PreToolUse hook before git commit.
# Enforces: branch from development, changelog staged, tests pass.
# Outputs JSON to block commit on violation, exits 0 silently on pass.

set -euo pipefail

# Allow bypass for emergencies
if [ "${FORGE_SKIP_WORKFLOW:-0}" = "1" ]; then
  exit 0
fi

VIOLATIONS=""

# ── Gate 1: Branch Ancestry ──────────────────────────────────────
# Current branch must descend from development.
# Skip check if on development or main directly.

BRANCH=$(git branch --show-current 2>/dev/null || echo "")

if [ -n "$BRANCH" ] && [ "$BRANCH" != "development" ] && [ "$BRANCH" != "main" ]; then
  if ! git merge-base --is-ancestor origin/development HEAD 2>/dev/null; then
    VIOLATIONS="${VIOLATIONS}[WORKFLOW] Branch Ancestry\nIssue: Branch '$BRANCH' does not descend from development\nRequired: Create branches from development (git checkout -b <branch> origin/development)\n\n"
  fi
fi

# ── Gate 2: Changelog Staged ─────────────────────────────────────
# CHANGELOG.md must be in the staging area.

if ! git diff --cached --name-only 2>/dev/null | grep -q "CHANGELOG.md"; then
  VIOLATIONS="${VIOLATIONS}[WORKFLOW] Changelog Missing\nIssue: CHANGELOG.md is not staged for commit\nRequired: Update CHANGELOG.md under [Unreleased] and stage it\n\n"
fi

# ── Gate 3: Tests Pass ───────────────────────────────────────────
# cargo test must exit 0.

if ! cargo test --quiet 2>/dev/null; then
  VIOLATIONS="${VIOLATIONS}[WORKFLOW] Tests Failing\nIssue: cargo test did not pass\nRequired: All tests must pass before committing\n\n"
fi

# ── Output ───────────────────────────────────────────────────────

if [ -n "$VIOLATIONS" ]; then
  reason=$(printf '%b' "$VIOLATIONS" | head -c 2000)
  reason_escaped=$(echo "$reason" | python3 -c 'import sys,json; print(json.dumps(sys.stdin.read()))' 2>/dev/null || echo '"Workflow gates failed"')
  printf '{"continue": false, "stopReason": %s}\n' "$reason_escaped"
  exit 0
fi

exit 0
