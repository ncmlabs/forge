#!/usr/bin/env bash
# Unit test for the path-guard / drift-detection pipeline used by
# scripts/audit-forge-surface.sh. Reproduces the awk/grep stages in isolation
# and feeds synthetic `git status --porcelain` strings, asserting that:
#
#   - untracked artifacts outside the allow-list never appear in OFFENDERS
#     (regression test for issue #348)
#   - tracked edits outside the allow-list always appear in OFFENDERS
#   - tracked edits inside the allow-list never appear in OFFENDERS, do appear
#     in DRIFT
#   - untracked new files inside the allow-list appear in DRIFT (so audits
#     creating fresh allowed-surface docs are still detected as drift)
#   - empty porcelain produces no OFFENDERS and no DRIFT
#
# Usage: bash scripts/test-audit-path-guard.sh
# Exit:  0 = all assertions pass, 1 = at least one regression

set -uo pipefail

# Mirror of the regex/pipeline in scripts/audit-forge-surface.sh. If you change
# one, change the other — that is the invariant this test exists to enforce.
ALLOWED='^(docs/|skills/|examples/|workflows/|CHANGELOG\.md$)'

offenders_of() {
  echo "$1" | awk '$1 != "??" {print $2}' | grep -Ev "$ALLOWED" || true
}

drift_of() {
  echo "$1" | awk '{print $2}' | grep -E "$ALLOWED" || true
}

PASS=0
FAIL=0
FAILURES=""

assert_offenders() {
  local name="$1" porcelain="$2" expected="$3"
  local actual
  actual="$(offenders_of "$porcelain")"
  if [ "$actual" = "$expected" ]; then
    PASS=$((PASS + 1))
    printf "  \033[32m✓\033[0m OFFENDERS  %s\n" "$name"
  else
    FAIL=$((FAIL + 1))
    FAILURES="${FAILURES}    - ${name}\n      expected: $(printf '%q' "$expected")\n      actual:   $(printf '%q' "$actual")\n"
    printf "  \033[31m✗\033[0m OFFENDERS  %s\n" "$name"
  fi
}

assert_drift() {
  local name="$1" porcelain="$2" expected="$3"
  local actual
  actual="$(drift_of "$porcelain")"
  if [ "$actual" = "$expected" ]; then
    PASS=$((PASS + 1))
    printf "  \033[32m✓\033[0m DRIFT      %s\n" "$name"
  else
    FAIL=$((FAIL + 1))
    FAILURES="${FAILURES}    - ${name}\n      expected: $(printf '%q' "$expected")\n      actual:   $(printf '%q' "$actual")\n"
    printf "  \033[31m✗\033[0m DRIFT      %s\n" "$name"
  fi
}

echo ""
echo "── path-guard invariants ─────────────────────────────────────"

# Issue #348 regression: an untracked runtime artifact at the repo root
# outside any allowed surface must NOT trip the guard.
P1='?? .forge-data/v0/sessions.redb'
assert_offenders "issue #348 regression: untracked .forge-data/" "$P1" ""
assert_drift     "issue #348 regression: drift ignores .forge-data/" "$P1" ""

# Tracked modification inside an allowed surface: the canonical happy path.
P2=' M docs/forge-reference.md'
assert_offenders "tracked edit inside docs/ — no offenders" "$P2" ""
assert_drift     "tracked edit inside docs/ — counts as drift" "$P2" "docs/forge-reference.md"

# Tracked modification outside an allowed surface: the safety case.
P3=' M src/main.rs'
assert_offenders "tracked edit outside allow-list — flagged" "$P3" "src/main.rs"
assert_drift     "tracked edit outside allow-list — not drift" "$P3" ""

# Untracked new file inside an allowed surface: audit may legitimately create
# a fresh doc; we want it detected as drift even though it's untracked.
P4='?? docs/new-page.md'
assert_offenders "untracked new file inside docs/ — not flagged" "$P4" ""
assert_drift     "untracked new file inside docs/ — counts as drift" "$P4" "docs/new-page.md"

# Mixed: real audit drift + a stray runtime artifact.
P5=$' M docs/forge-reference.md\n M CHANGELOG.md\n?? .forge-data/v0/sessions.redb'
assert_offenders "mixed: drift + stray artifact — no offenders" "$P5" ""
assert_drift     "mixed: drift + stray artifact — only allow-list paths" \
  "$P5" $'docs/forge-reference.md\nCHANGELOG.md'

# Mixed with violation: audit edited something it shouldn't have.
P6=$' M docs/forge-reference.md\n M src/runtime.rs\n?? .forge-data/v0/sessions.redb'
assert_offenders "mixed with violation — runtime edit flagged, artifact ignored" \
  "$P6" "src/runtime.rs"

# Staged-add inside allow-list (status 'A '): treated as tracked, must be drift.
P7='A  examples/new-flow.forge'
assert_offenders "staged add inside examples/ — not flagged" "$P7" ""
assert_drift     "staged add inside examples/ — counts as drift" "$P7" "examples/new-flow.forge"

# Empty porcelain: no changes at all.
P8=''
assert_offenders "empty porcelain — no offenders" "$P8" ""
assert_drift     "empty porcelain — no drift" "$P8" ""

# CHANGELOG.md anchor: must match exactly, not a prefix-collision elsewhere.
P9=' M CHANGELOG.md'
assert_offenders "CHANGELOG.md tracked edit — not flagged" "$P9" ""
assert_drift     "CHANGELOG.md tracked edit — counts as drift" "$P9" "CHANGELOG.md"

# Negative: a path that contains 'docs/' deeper in the tree must NOT match
# (the regex is anchored to the start of the path).
P10=' M src/docs/legacy.rs'
assert_offenders "non-anchored docs/ — flagged as offender" "$P10" "src/docs/legacy.rs"
assert_drift     "non-anchored docs/ — not drift" "$P10" ""

echo ""
echo "──────────────────────────────────────────────────────────────"
echo "  Passed: $PASS    Failed: $FAIL"
echo "──────────────────────────────────────────────────────────────"

if [ $FAIL -gt 0 ]; then
  echo ""
  echo "Failures:"
  printf "%b" "$FAILURES"
  exit 1
fi

exit 0
