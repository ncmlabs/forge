#!/usr/bin/env bash
# FORGE derived-surface drift audit (issue #229, Layer 2).
#
# Invokes the claude CLI non-interactively with the audit protocol at
# scripts/audit-forge-surface.prompt.md, runs the repo's verification gates
# against the resulting edits, enforces the path-guard (no edits outside the
# allowed derived surfaces), and — if diff is non-empty — opens a draft PR.
#
# Intended callers:
#   - .github/workflows/forge-surface-audit.yml (weekly cron)
#   - humans running `gh workflow run forge-surface-audit.yml`
#   - local dry-run via `AUDIT_DRY_RUN=1 scripts/audit-forge-surface.sh`
#
# Required env:
#   ANTHROPIC_API_KEY  — for the claude CLI
# Optional env:
#   AUDIT_DRY_RUN=1    — run audit + gates but skip commit/push/PR
#   AUDIT_BRANCH       — override branch name (default: chore/surface-audit-YYYY-Www)

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

ISO_WEEK="$(date -u +%G-W%V)"
BRANCH="${AUDIT_BRANCH:-chore/surface-audit-${ISO_WEEK}}"
PROMPT_FILE="scripts/audit-forge-surface.prompt.md"
REPORT_FILE="$(mktemp -t forge-audit-report-XXXXXX.txt)"
trap 'rm -f "$REPORT_FILE"' EXIT

if [ ! -f "$PROMPT_FILE" ]; then
  echo "audit: missing $PROMPT_FILE" >&2
  exit 2
fi

if ! command -v claude >/dev/null 2>&1; then
  echo "audit: claude CLI not found in PATH" >&2
  exit 2
fi

# Require a clean worktree before editing so the diff we capture later is
# attributable to the audit alone.
if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "audit: worktree is dirty; refusing to run" >&2
  exit 2
fi

echo "audit: running claude with $PROMPT_FILE (week $ISO_WEEK)"
# acceptEdits lets Claude apply edits without an interactive prompt; the
# prompt itself enforces the authoritative-vs-derived path split, and the
# post-run path-guard below is the authoritative check.
claude -p \
  --permission-mode acceptEdits \
  --output-format text \
  "$(cat "$PROMPT_FILE")" | tee "$REPORT_FILE"

# Path-guard: every changed path must live under an allowed prefix.
ALLOWED='^(docs/|skills/|examples/|workflows/|CHANGELOG\.md$)'
CHANGED="$(git status --porcelain | awk '{print $2}')"
if [ -n "$CHANGED" ]; then
  OFFENDERS="$(echo "$CHANGED" | grep -Ev "$ALLOWED" || true)"
  if [ -n "$OFFENDERS" ]; then
    echo "audit: path-guard violation — edits outside allowed derived surfaces:" >&2
    echo "$OFFENDERS" >&2
    exit 3
  fi
fi

# If nothing changed, stop here — that's the "no drift detected" outcome.
if [ -z "$CHANGED" ]; then
  echo "audit: no drift detected (no files changed)"
  exit 0
fi

echo "audit: running verification gates"
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
FORGE_MOCK=1 bash scripts/check-forge-examples.sh

if [ "${AUDIT_DRY_RUN:-0}" = "1" ]; then
  echo "audit: AUDIT_DRY_RUN=1 — skipping commit/push/PR"
  echo "audit: changed paths:"
  echo "$CHANGED"
  exit 0
fi

echo "audit: committing on $BRANCH"
git checkout -B "$BRANCH"
git add docs skills examples workflows CHANGELOG.md 2>/dev/null || true
git commit -m "chore: FORGE surface audit (${ISO_WEEK}) (#229)"

echo "audit: pushing $BRANCH"
git push -u origin "$BRANCH"

PR_BODY_FILE="$(mktemp -t forge-audit-pr-XXXXXX.md)"
trap 'rm -f "$REPORT_FILE" "$PR_BODY_FILE"' EXIT

{
  echo "Automated weekly FORGE derived-surface audit (issue #229, Layer 2)."
  echo
  echo "## Drift report"
  echo
  if grep -q '^AUDIT REPORT:' "$REPORT_FILE"; then
    awk '/^AUDIT REPORT:/{flag=1} flag; /^AUDIT REPORT END/{flag=0}' "$REPORT_FILE"
  else
    echo "_(audit did not emit a report block; see workflow logs)_"
  fi
  echo
  echo "## Verification"
  echo
  echo "- \`cargo fmt --check\` ✓"
  echo "- \`cargo clippy --all-targets -- -D warnings\` ✓"
  echo "- \`cargo test\` ✓"
  echo "- \`scripts/check-forge-examples.sh\` ✓"
  echo
  echo "## Scope"
  echo
  echo "Path-guard confirmed: edits stay within \`docs/\`, \`skills/\`, \`examples/\`, \`workflows/\`, \`CHANGELOG.md\`."
  echo
  echo "Closes part of #229 (ongoing — this PR does not close the issue)."
} > "$PR_BODY_FILE"

gh pr create \
  --base development \
  --head "$BRANCH" \
  --draft \
  --title "chore: FORGE surface audit (${ISO_WEEK})" \
  --body-file "$PR_BODY_FILE" \
  --label surface-audit \
  --label docs

echo "audit: draft PR opened"
