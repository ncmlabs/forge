#!/usr/bin/env bash
# FORGE derived-surface drift audit (issues #229, #449 — Layer 2).
#
# Rework (2026-09-03, #449): the audit protocol is now executed by the FORGE
# agent in workflows/surface-audit.forge via `forge send`, on a zero-cost
# OpenAI-compatible provider selected with FORGE_CONFIG + FORGE_PROVIDER
# (the same pattern the Daily Frame pipeline uses). The retired `claude -p`
# invocation required an Anthropic key whose billing died in July 2026 and
# kept the weekly cron red; this path runs on local/LAN budget instead.
#
# Flow:
#   1. forge send workflows/surface-audit.forge audit <repo-root>
#      -> agent reads the protocol, audits, emits AUDIT REPORT block and
#         applies allowed-surface edits via file.write + git apply.
#   2. Path-guard: tracked edits must live under allowed derived surfaces.
#   3. Verification gates (fmt/clippy/test/example validation).
#   4. If diff non-empty: commit, push, open a draft PR.
#
# Intended callers:
#   - .github/workflows/forge-surface-audit.yml (cron + manual dispatch)
#   - local dry-run via AUDIT_DRY_RUN=1 scripts/audit-forge-surface.sh
#
# Required env (provider selection — zero-cost pattern):
#   FORGE_CONFIG       — path to a forge config declaring the audit provider
#   FORGE_PROVIDER     — provider name inside that config (e.g. vllm-local)
#   FORGE_BINARY       — forge binary to drive the audit agent (default: forge)
# Optional env:
#   AUDIT_DRY_RUN=1    — run audit + gates but skip commit/push/PR
#   AUDIT_BRANCH       — override branch name (default: chore/surface-audit-YYYY-Www)

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

ISO_WEEK="$(date -u +%G-W%V)"
BRANCH="${AUDIT_BRANCH:-chore/surface-audit-${ISO_WEEK}}"
PROMPT_FILE="scripts/audit-forge-surface.prompt.md"
AGENT_FILE="workflows/surface-audit.forge"
# Allowed derived surfaces (issue #229) — used by the patch scope pre-check
# and the post-run path-guard.
ALLOWED='^(docs/|skills/|examples/|workflows/|CHANGELOG\.md$)'
REPORT_FILE="$(mktemp -t forge-audit-report-XXXXXX.txt)"
trap 'rm -f "$REPORT_FILE"' EXIT

FORGE_BIN="${FORGE_BINARY:-forge}"

if [ ! -f "$PROMPT_FILE" ]; then
  echo "audit: missing $PROMPT_FILE" >&2
  exit 2
fi
if [ ! -f "$AGENT_FILE" ]; then
  echo "audit: missing $AGENT_FILE" >&2
  exit 2
fi
if ! command -v "$FORGE_BIN" >/dev/null 2>&1; then
  echo "audit: forge binary not found: $FORGE_BIN (set FORGE_BINARY)" >&2
  exit 2
fi

# Refuse to run without a real provider selected. A silent mock fallback used
# to be indistinguishable from a real audit (see #437-era footguns); make the
# failure loud instead.
if [ "${FORGE_MOCK:-0}" = "1" ]; then
  echo "audit: refusing to run with FORGE_MOCK=1 — the audit would be a no-op" >&2
  exit 2
fi
if [ -z "${FORGE_CONFIG:-}" ] || [ -z "${FORGE_PROVIDER:-}" ]; then
  echo "audit: FORGE_CONFIG and FORGE_PROVIDER are required (zero-cost provider selection; see #449)" >&2
  exit 2
fi

# Require a clean worktree before editing so the diff we capture later is
# attributable to the audit alone.
if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "audit: worktree is dirty; refusing to run" >&2
  exit 2
fi

echo "audit: driving $AGENT_FILE with $FORGE_BIN (provider $FORGE_PROVIDER, week $ISO_WEEK)"
"$FORGE_BIN" send "$AGENT_FILE" audit "$REPO_ROOT" "$ISO_WEEK" | tee "$REPORT_FILE"

# The agent materializes its proposed fix at .forge-audit.patch. Everything
# below is deterministic: normalize, scope-check, apply — the agent never
# touches git itself.
PATCH_FILE="$REPO_ROOT/.forge-audit.patch"
if [ ! -f "$PATCH_FILE" ]; then
  echo "audit: agent wrote no patch file — treating as no drift"
  if grep -q '^AUDIT REPORT:' "$REPORT_FILE"; then
    awk '/^AUDIT REPORT:/{flag=1} flag; /^AUDIT REPORT END/{flag=0}' "$REPORT_FILE"
  fi
  exit 0
fi

# Normalize: keep only diff-structural lines, from the first diff header
# onwards. Drops code fences, leading/trailing prose and other wrapper noise
# the model may still have emitted despite the prompt. Empty lines are kept:
# inside a hunk they are blank context lines.
awk '/^diff --git /{found=1} found' "$PATCH_FILE" \
  | grep -E '^(diff --git |index |--- |\+\+\+ |@@|[ +-]|$)' > "$PATCH_FILE.norm" || true
mv "$PATCH_FILE.norm" "$PATCH_FILE"
if grep -q '^NODRIFT$' "$PATCH_FILE" || [ ! -s "$PATCH_FILE" ]; then
  echo "audit: agent reports no drift"
  rm -f "$PATCH_FILE"
  exit 0
fi

# Scope pre-check: every path in the patch must be an allowed surface —
# reject BEFORE the worktree is touched.
BAD_PATHS="$(grep '^diff --git ' "$PATCH_FILE" | sed -E 's|^diff --git a/(.*) b/(.*)$|\1\n\2|' | grep -Ev "$ALLOWED" || true)"
if [ -n "$BAD_PATHS" ]; then
  echo "audit: patch touches disallowed paths:" >&2
  echo "$BAD_PATHS" >&2
  exit 3
fi

# Apply (with --recount to tolerate slightly-off hunk headers from the LLM).
if git apply --check --recount "$PATCH_FILE" 2>"$PATCH_FILE.err"; then
  git apply --recount "$PATCH_FILE"
  rm -f "$PATCH_FILE" "$PATCH_FILE.err"
  echo "audit: agent patch applied"
else
  echo "audit: agent diff does not apply cleanly (kept for inspection at $PATCH_FILE):" >&2
  cat "$PATCH_FILE.err" >&2
  exit 4
fi

# The agent also reports inside its stdout; keep both the agent's own
# AUDIT REPORT block and the applied-patch status visible in the log.

# Path-guard: every *tracked* change must live under an allowed prefix.
# Untracked entries ('??' in porcelain) are out-of-scope — the `git add` below
# only stages allow-listed surfaces, so a stray runtime artifact (e.g.
# `.forge-data/`, a session cache, a tmp dir) outside those prefixes can never
# be committed and must not trip the guard. See issue #348. Porcelain lines
# carry a fixed 3-char prefix (XY + space), so the path starts at column 4.
PORCELAIN="$(git status --porcelain)"
OFFENDERS="$(echo "$PORCELAIN" | grep -Ev '^\?\? ' | cut -c4- | grep -Ev "$ALLOWED" || true)"
if [ -n "$OFFENDERS" ]; then
  echo "audit: path-guard violation — tracked edits outside allowed derived surfaces:" >&2
  echo "$OFFENDERS" >&2
  exit 3
fi

# Drift = any change (tracked edit or new file) under an allowed surface.
# Untracked artifacts outside ALLOWED are intentionally excluded so they can't
# masquerade as drift; new files the audit creates inside an allowed surface
# (e.g. a fresh `docs/foo.md`) appear here as '??' and are still detected.
CHANGED="$(echo "$PORCELAIN" | cut -c4- | grep -E "$ALLOWED" || true)"
if [ -z "$CHANGED" ]; then
  echo "audit: no drift detected (no changes within allowed surfaces)"
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
  echo "Automated weekly FORGE derived-surface audit (issues #229, #449 — Layer 2)."
  echo
  echo "Executed by \`forge send workflows/surface-audit.forge audit\` on a zero-cost"
  echo "OpenAI-compatible provider (no Anthropic billing involved)."
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

# Ensure labels exist — gh pr create --label errors if a label is missing.
# Idempotent: --force would bump existing labels, so only create when absent.
for label in surface-audit docs; do
  if ! gh label list --limit 200 --json name --jq '.[].name' | grep -Fxq "$label"; then
    case "$label" in
      surface-audit) gh label create surface-audit --color ededed --description "Automated derived-surface drift audit (issue #229)" ;;
      docs)          gh label create docs --color 0075ca --description "Documentation" ;;
    esac
  fi
done

gh pr create \
  --base development \
  --head "$BRANCH" \
  --draft \
  --title "chore: FORGE surface audit (${ISO_WEEK})" \
  --body-file "$PR_BODY_FILE" \
  --label surface-audit \
  --label docs

echo "audit: draft PR opened"
