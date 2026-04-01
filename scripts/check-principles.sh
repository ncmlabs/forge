#!/usr/bin/env bash
# FORGE Principles Fast Check
# Called by Claude Code PreToolUse hook before git commit.
# Checks staged .rs files for mechanical principle violations.
# Outputs JSON to block commit on violation, exits 0 silently on pass.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
STAGED_RS=$(git diff --cached --name-only --diff-filter=ACM -- '*.rs' 2>/dev/null || true)

if [ -z "$STAGED_RS" ]; then
  exit 0
fi

VIOLATIONS=""

# ── Principle II: Determinism Boundary ──────────────────────────
# Pure-checking code must reject ALL oracle call variants.
# If a staged file touches pure-checking logic, verify it handles
# Reason, Classify, AND Search — not just some of them.

for f in $STAGED_RS; do
  filepath="$REPO_ROOT/$f"
  [ -f "$filepath" ] || continue

  if grep -q 'check_pure\|PureUsesLlm\|pure.*checker' "$filepath" 2>/dev/null; then
    has_reason=$(grep -c 'Expr::Reason\|Reason(' "$filepath" 2>/dev/null || echo 0)
    has_classify=$(grep -c 'Expr::Classify\|Classify(' "$filepath" 2>/dev/null || echo 0)
    has_search=$(grep -c 'Expr::Search\|Search(' "$filepath" 2>/dev/null || echo 0)

    if [ "$has_reason" -gt 0 ] || [ "$has_classify" -gt 0 ] || [ "$has_search" -gt 0 ]; then
      if [ "$has_reason" -eq 0 ] || [ "$has_classify" -eq 0 ] || [ "$has_search" -eq 0 ]; then
        missing=""
        [ "$has_reason" -eq 0 ] && missing="${missing}Reason, "
        [ "$has_classify" -eq 0 ] && missing="${missing}Classify, "
        [ "$has_search" -eq 0 ] && missing="${missing}Search, "
        missing="${missing%, }"
        VIOLATIONS="${VIOLATIONS}[PRINCIPLE II] Determinism Boundary\nFile: $f\nIssue: Pure checker does not handle all oracle call variants\nEvidence: Missing checks for: $missing\nRequired: Pure checker must reject Reason, Classify, AND Search\n\n"
      fi
    fi
  fi
done

# ── Principle VI: Self-Reference ────────────────────────────────
# New error enum variants must have structured fields (file, line, col).
# Check staged diffs for error variants missing these fields.

for f in $STAGED_RS; do
  filepath="$REPO_ROOT/$f"
  [ -f "$filepath" ] || continue

  staged_diff=$(git diff --cached -- "$f" 2>/dev/null || true)
  added_lines=$(echo "$staged_diff" | grep '^+' | grep -v '^+++' || true)

  if echo "$added_lines" | grep -q '#\[error(' 2>/dev/null; then
    # Check if new variant definitions (lines with braces) have structured fields
    # Extract hunks containing #[error( and check for file/line/col in the same hunk
    hunk=""
    in_error_hunk=false
    while IFS= read -r line; do
      if echo "$line" | grep -q '^@@' 2>/dev/null; then
        # Process previous hunk
        if $in_error_hunk && [ -n "$hunk" ]; then
          if ! echo "$hunk" | grep -q 'file.*String\|line.*usize\|col.*usize' 2>/dev/null; then
            if echo "$hunk" | grep -q '^\+.*{' 2>/dev/null; then
              VIOLATIONS="${VIOLATIONS}[PRINCIPLE VI] Self-Reference\nFile: $f\nIssue: Error variant missing structured fields\nEvidence: New error variant lacks file/line/col fields\nRequired: All error variants must include file, line, col for machine consumption\n\n"
            fi
          fi
        fi
        hunk=""
        in_error_hunk=false
      fi
      if echo "$line" | grep -q '^+.*#\[error(' 2>/dev/null; then
        in_error_hunk=true
      fi
      hunk="${hunk}${line}\n"
    done <<< "$staged_diff"
    # Process last hunk
    if $in_error_hunk && [ -n "$hunk" ]; then
      if ! echo "$hunk" | grep -q 'file.*String\|line.*usize\|col.*usize' 2>/dev/null; then
        if echo "$hunk" | grep -q '^\+.*{' 2>/dev/null; then
          VIOLATIONS="${VIOLATIONS}[PRINCIPLE VI] Self-Reference\nFile: $f\nIssue: Error variant missing structured fields\nEvidence: New error variant lacks file/line/col fields\nRequired: All error variants must include file, line, col for machine consumption\n\n"
        fi
      fi
    fi
  fi
done

# ── Principle VIII: Accountability ──────────────────────────────
# CompletionRequest construction must have adjacent tracer calls.

for f in $STAGED_RS; do
  filepath="$REPO_ROOT/$f"
  [ -f "$filepath" ] || continue

  if grep -q 'CompletionRequest' "$filepath" 2>/dev/null; then
    line_nums=$(grep -n 'CompletionRequest' "$filepath" 2>/dev/null | cut -d: -f1 || true)
    for line_num in $line_nums; do
      start=$((line_num > 10 ? line_num - 10 : 1))
      end=$((line_num + 10))
      context=$(sed -n "${start},${end}p" "$filepath" 2>/dev/null || true)
      if ! echo "$context" | grep -qi 'trace\|tracer\|Tracer\|span\|instrument' 2>/dev/null; then
        VIOLATIONS="${VIOLATIONS}[PRINCIPLE VIII] Accountability\nFile: $f:$line_num\nIssue: CompletionRequest without adjacent tracer call\nEvidence: No trace/tracer/span reference within 10 lines of CompletionRequest\nRequired: Every oracle call must be traced for accountability\n\n"
      fi
    done
  fi
done

# ── Principle IX: Boundary ──────────────────────────────────────
# Checker modules must not import from runtime modules.

for f in $STAGED_RS; do
  filepath="$REPO_ROOT/$f"
  [ -f "$filepath" ] || continue

  if echo "$f" | grep -q 'checker\|boundary' 2>/dev/null; then
    if grep -q 'use crate::runtime\|mod runtime' "$filepath" 2>/dev/null; then
      VIOLATIONS="${VIOLATIONS}[PRINCIPLE IX] Boundary\nFile: $f\nIssue: Checker module imports from runtime\nEvidence: Found runtime import in checker code\nRequired: Checker modules must not depend on runtime modules (compile-time vs runtime separation)\n\n"
    fi
  fi
done

# ── Output ──────────────────────────────────────────────────────

if [ -n "$VIOLATIONS" ]; then
  reason=$(printf '%b' "$VIOLATIONS" | head -c 2000)
  reason_escaped=$(echo "$reason" | python3 -c 'import sys,json; print(json.dumps(sys.stdin.read()))' 2>/dev/null || echo '"Principle violation detected"')
  printf '{"continue": false, "stopReason": %s}\n' "$reason_escaped"
  exit 0
fi

exit 0
