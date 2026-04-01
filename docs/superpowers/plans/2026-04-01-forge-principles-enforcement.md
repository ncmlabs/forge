# FORGE Principles Enforcement System — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Automated enforcement of FORGE's 9 first principles against the Rust implementation code, via a fast pre-commit hook and a deep LLM-powered review skill.

**Architecture:** Two components — (1) `scripts/check-principles.sh` for fast deterministic pattern checks on `git commit`, wired via Claude Code `PreToolUse` hook; (2) a Claude Code skill at `~/.claude/skills/forge-principles/` with `SKILL.md`, `rules.md`, and `checklist.md` for deep 9-principle audits.

**Tech Stack:** Bash/grep for hook, Claude Code skills + hooks system, YAML frontmatter for skill definition.

**Spec:** `docs/superpowers/specs/2026-04-01-forge-principles-enforcement-design.md`

**Principles doc:** `forge-principles.md` (the source of truth for all 9 principles)

---

## File Structure

| File | Responsibility |
|---|---|
| `scripts/check-principles.sh` | Fast pre-commit checker. Reads staged `.rs` files, runs 4 pattern checks, outputs JSON to block or passes silently. |
| `.claude/settings.json` | Project-level Claude Code settings. Wires the hook to `PreToolUse` on `Bash(git commit*)`. |
| `~/.claude/skills/forge-principles/SKILL.md` | Skill definition. Instructions for the LLM agent to run a full 9-principle audit. |
| `~/.claude/skills/forge-principles/rules.md` | The 9 principles as concrete, checkable rules with file paths and grep patterns. |
| `~/.claude/skills/forge-principles/checklist.md` | Per-principle evaluation checklist the agent walks through. |

---

### Task 1: Create the pre-commit hook script

**Files:**
- Create: `scripts/check-principles.sh`

- [ ] **Step 1: Create `scripts/check-principles.sh`**

This script is called by the Claude Code hook before `git commit`. It checks staged `.rs` files for mechanical principle violations and outputs JSON to block if violations are found.

```bash
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

  # Only check files that contain pure-checking logic
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
# Check staged files for error enum definitions missing these fields.

for f in $STAGED_RS; do
  filepath="$REPO_ROOT/$f"
  [ -f "$filepath" ] || continue

  # Find error enum blocks in staged content
  staged_content=$(git diff --cached -- "$f" | grep '^+' | grep -v '^+++' || true)

  if echo "$staged_content" | grep -q 'enum.*Error'; then
    # Check if there are variant definitions without structured fields
    # Look for variants that have braces but lack file/line/col
    if echo "$staged_content" | grep -qP '#\[error\(' 2>/dev/null; then
      if ! echo "$staged_content" | grep -q 'file.*:.*String\|line.*:.*usize\|col.*:.*usize' 2>/dev/null; then
        # Only flag if there are new variant definitions with braces
        if echo "$staged_content" | grep -qP '^\+\s+\w+\s*\{' 2>/dev/null; then
          VIOLATIONS="${VIOLATIONS}[PRINCIPLE VI] Self-Reference\nFile: $f\nIssue: Error variant missing structured fields\nEvidence: New error variant lacks file/line/col fields\nRequired: All error variants must include file, line, col, and error code for machine consumption\n\n"
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
    # For each CompletionRequest, check if there's a trace/tracer call within 10 lines
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
  # Format as JSON for Claude Code hook protocol
  reason=$(echo -e "$VIOLATIONS" | head -c 2000)
  # Escape for JSON
  reason_escaped=$(echo "$reason" | python3 -c 'import sys,json; print(json.dumps(sys.stdin.read()))' 2>/dev/null || echo "\"Principle violation detected\"")
  echo "{\"continue\": false, \"stopReason\": $reason_escaped}"
  exit 0
fi

# Pass silently
exit 0
```

- [ ] **Step 2: Make the script executable**

Run: `chmod +x scripts/check-principles.sh`

- [ ] **Step 3: Test the script runs cleanly with no staged files**

Run: `bash scripts/check-principles.sh`
Expected: exits 0 with no output (no staged .rs files)

- [ ] **Step 4: Test the script detects a real pattern**

Run: `echo '{"tool_name":"Bash","tool_input":{"command":"git commit -m test"}}' | bash scripts/check-principles.sh`
Expected: exits 0 with no output (no staged files, so passes)

- [ ] **Step 5: Commit**

```bash
git add scripts/check-principles.sh
git commit -m "feat: add FORGE principles fast-check script for pre-commit hook"
```

---

### Task 2: Wire the pre-commit hook in Claude Code settings

**Files:**
- Create: `.claude/settings.json`

- [ ] **Step 1: Create `.claude/settings.json` with the hook**

The project already has `.claude/settings.local.json` with permissions. The project-level `settings.json` (committed to repo) gets the hook so all contributors get it.

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "if": "Bash(git commit*)",
            "command": "bash scripts/check-principles.sh",
            "timeout": 10,
            "statusMessage": "Checking FORGE principles..."
          }
        ]
      }
    ]
  }
}
```

- [ ] **Step 2: Validate the JSON is well-formed**

Run: `jq -e '.hooks.PreToolUse[] | select(.matcher == "Bash") | .hooks[] | select(.type == "command") | .command' .claude/settings.json`
Expected: prints `"bash scripts/check-principles.sh"` and exits 0.

- [ ] **Step 3: Commit**

```bash
git add .claude/settings.json
git commit -m "feat: wire FORGE principles hook to PreToolUse on git commit"
```

---

### Task 3: Create the skill definition (SKILL.md)

**Files:**
- Create: `~/.claude/skills/forge-principles/SKILL.md`

- [ ] **Step 1: Create the skill directory**

Run: `mkdir -p ~/.claude/skills/forge-principles`

- [ ] **Step 2: Create `SKILL.md`**

```markdown
---
name: forge-principles
description: Deep audit of FORGE Rust implementation against the 9 first principles. Checks that the compiler, runtime, and type system correctly enforce honesty, determinism boundary, token economy, composition, supervision, self-reference, human ceiling, accountability, and boundary separation.
triggers:
  - "check principles"
  - "principles audit"
  - "forge principles"
  - "principle review"
  - "forge-principles"
---

# FORGE Principles Audit

You are auditing the FORGE Rust implementation at `src/**/*.rs` against the 9 first principles defined in `forge-principles.md`.

## Instructions

1. Read `~/.claude/skills/forge-principles/rules.md` for the concrete rules per principle.
2. Read `~/.claude/skills/forge-principles/checklist.md` for the evaluation checklist.
3. For each principle in the checklist, evaluate the current Rust code:
   - Use Grep and Glob to find relevant code
   - Check each rule against what exists
   - Mark as PASS, VIOLATION, or NOT_YET_APPLICABLE
4. Produce a structured report.

## Evaluation Rules

- **PASS** — The rule is implemented correctly in the Rust code.
- **VIOLATION** — The rule is implemented but incorrectly, or a required component exists but doesn't follow the principle.
- **NOT_YET_APPLICABLE** — The relevant component doesn't exist yet (e.g., no `src/llm/cost_tracker.rs` means Principle III can't be checked). This is NOT a failure — it tracks what's missing.

## Output Format

For each principle, output:

```
[PRINCIPLE {number}] {name}
Status: PASS | VIOLATION | NOT_YET_APPLICABLE
File: {path}:{line} (if applicable)
Issue: {description} (if VIOLATION)
Evidence: {what was found} (if VIOLATION)
Required: {what the principle demands} (if VIOLATION)
```

Then produce a summary:

```
## Principles Report
- PASS: N/9
- VIOLATION: N/9
- NOT_YET_APPLICABLE: N/9

### Violations (if any)
[details]

### Not Yet Applicable (if any)
[list of principles and what component is missing]
```

## Important

- Do NOT false-pass. If a component doesn't exist, mark NOT_YET_APPLICABLE.
- Do NOT flag components that are correctly stubbed with issue references — those are tracked in the roadmap.
- Focus on the Rust implementation, not `.forge` files.
- Read the actual code before making judgments. Don't guess from file names.
```

- [ ] **Step 3: Verify the file exists and has correct frontmatter**

Run: `head -10 ~/.claude/skills/forge-principles/SKILL.md`
Expected: Shows YAML frontmatter with `name: forge-principles`

- [ ] **Step 4: Commit** (nothing to commit — skill is outside repo, in user's home dir)

No commit needed. Skill files live in `~/.claude/skills/` which is not part of the repo.

---

### Task 4: Create the rules reference (rules.md)

**Files:**
- Create: `~/.claude/skills/forge-principles/rules.md`

- [ ] **Step 1: Create `rules.md`**

```markdown
# FORGE Principles — Concrete Rules for Rust Implementation

Source of truth: `forge-principles.md` in the repo root.

These rules translate each principle into specific, checkable assertions against `src/**/*.rs`.

---

## Principle I — Honesty
> "A system that hides uncertainty is more dangerous than one that knows nothing"

### Rules
1. The AST must have expression nodes for oracle calls (`Reason`, `Classify`, `Search`) that are typed distinctly from deterministic expressions.
   - Check: `src/ast.rs` — look for `Expr::Reason`, `Expr::Classify`, `Expr::Search` variants.
2. The type system must represent confidence/uncertainty as a first-class concept.
   - Check: `src/types.rs` — look for confidence-related types (`ConfidenceSource`, confidence predicates, or `uncertain<T>` equivalent).
3. If an uncertain checker exists, it must reject code paths where uncertain values are used without `when`/`match`.
   - Check: `src/checker/uncertain_checker.rs` or equivalent in `src/resolver.rs`.

### Grep patterns
```
src/ast.rs: Expr::Reason, Expr::Classify, Expr::Search
src/types.rs: ConfidenceSource, Confidence, uncertain
src/resolver.rs: uncertain, confidence, unhandled
```

---

## Principle II — Determinism Boundary
> "Two kinds of computation exist. They must never be mixed invisibly."

### Rules
1. A purity checker must exist and reject ALL oracle call variants inside `pure` functions.
   - Check: `src/resolver.rs` function `contains_llm_operation` or equivalent.
2. The check must be recursive — a pure function calling another function that calls `think` must also be rejected.
   - Check: look for recursive traversal in the purity check (does it follow function calls?).
3. The three oracle variants (`Reason`, `Classify`, `Search`) must ALL be checked, not just some.
   - Check: the match arms in the purity check.

### Grep patterns
```
src/resolver.rs: check_pure, contains_llm_operation, PureUsesLlm
src/resolver.rs: Expr::Reason, Expr::Classify, Expr::Search (in same function)
```

---

## Principle III — Token Economy
> "Token is the fundamental unit of oracle computation"

### Rules
1. A cost tracking component must exist.
   - Check: `src/llm/cost_tracker.rs` or cost-related structs.
2. The CLI must have a `cost` subcommand.
   - Check: `src/main.rs` for `cost` in the CLI enum.
3. `CompletionRequest` (or equivalent) must carry token budget fields.
   - Check: any LLM request struct for `max_tokens`, `budget`, or `cost` fields.

### Grep patterns
```
src/llm/: cost, budget, token, CostTracker
src/main.rs: Command::Cost, "cost"
```

---

## Principle IV — Composition Completeness
> "Any primitive that cannot compose with any other primitive is not a primitive"

### Rules
1. The `>>` (compose) operator must be in the AST.
   - Check: `src/ast.rs` for `Expr::Compose` or similar.
2. Type compatibility checking must exist for composition.
   - Check: `src/resolver.rs` or `src/types.rs` for compose type checking.
3. Every primitive type must be compatible with `>>` — no special adapters.
   - Check: `src/types.rs` `is_compatible` function covers all type pairs.

### Grep patterns
```
src/ast.rs: Compose, compose_expr
src/types.rs: is_compatible
src/resolver.rs: check_compose, composition
```

---

## Principle V — Supervision
> "Write the happy path. Declare failure policy. Let the supervisor handle the rest."

### Rules
1. Agent AST nodes must support failure policy declarations.
   - Check: `src/ast.rs` agent-related structs for `on_hallucination`, `on_timeout`, `failure_policy`, or `stuck` fields.
2. Supervisor strategies must be defined.
   - Check: `src/runtime/supervisor.rs` for `one_for_one`, `one_for_all`, `rest_for_one`.
3. The grammar must parse failure policy declarations.
   - Check: `grammar/forge.pest` for `on_hallucination`, `on_timeout`, `if_stuck`, `failure_policy`.

### Grep patterns
```
src/ast.rs: failure, hallucination, timeout, stuck, supervisor
src/runtime/supervisor.rs: one_for_one, one_for_all, rest_for_one
grammar/forge.pest: on_hallucination, on_timeout, if_stuck
```

---

## Principle VI — Self-Reference
> "A language built for agents must be writable by agents"

### Rules
1. Error types must be structured with file, line, col, and error code.
   - Check: all `enum.*Error` in `src/` — every variant with `#[error(` must have `file: String, line: usize, col: usize`.
2. Error formatting must be machine-consumable (not just Display strings).
   - Check: error types derive `Serialize` or have structured output in `main.rs`.
3. The grammar's complexity must be bounded — check the pest file isn't growing unbounded.
   - Check: `grammar/forge.pest` line count (flag if >1000 lines).

### Grep patterns
```
src/: enum.*Error, #[error(, file:.*String, line:.*usize, col:.*usize
src/: Serialize (on error types)
grammar/forge.pest: (line count)
```

---

## Principle VII — Human Ceiling
> "The most valuable FORGE agents know when to stop"

### Rules
1. `escalate` must be a first-class AST node.
   - Check: `src/ast.rs` for `Stmt::Escalate` or `Expr::Escalate`.
2. The grammar must parse `escalate` statements.
   - Check: `grammar/forge.pest` for `escalate_stmt` or `escalate`.
3. `requires` guards must support `on fail: escalate` as a policy.
   - Check: `src/ast.rs` `FailPolicy` or `OnFail` enum for an `Escalate` variant.

### Grep patterns
```
src/ast.rs: Escalate, escalate
grammar/forge.pest: escalate
src/ast.rs: FailPolicy, OnFail, Escalate
```

---

## Principle VIII — Accountability
> "Every decision must be traceable to its cause"

### Rules
1. A tracer module must exist.
   - Check: `src/tracer.rs` is more than a stub.
2. Runtime execution paths for `think`, `transition`, `emit` must call the tracer.
   - Check: `src/runtime/` files for tracer invocations.
3. Trace output must be structured JSON.
   - Check: `src/tracer.rs` for `serde_json`, `Serialize`, or JSON formatting.

### Grep patterns
```
src/tracer.rs: (check if >10 lines, i.e. not just a stub comment)
src/runtime/: tracer, trace, Tracer
src/tracer.rs: serde_json, Serialize, json
```

---

## Principle IX — Boundary
> "Code that must be correct must be separated from code that might be wrong"

### Rules
1. The AST must support boundary directives.
   - Check: `src/ast.rs` for `BoundaryDirective` or `Boundary`.
2. A boundary checker must exist (or be planned).
   - Check: `src/checker/boundary_checker.rs` or boundary logic in `src/resolver.rs`.
3. Checker modules must NOT import runtime modules.
   - Check: `src/resolver.rs` and any `src/checker/*.rs` for `use crate::runtime`.

### Grep patterns
```
src/ast.rs: Boundary, BoundaryDirective
src/checker/: boundary
src/resolver.rs: use crate::runtime (should NOT exist)
```
```

- [ ] **Step 2: Verify the file**

Run: `wc -l ~/.claude/skills/forge-principles/rules.md`
Expected: ~170-180 lines

---

### Task 5: Create the evaluation checklist (checklist.md)

**Files:**
- Create: `~/.claude/skills/forge-principles/checklist.md`

- [ ] **Step 1: Create `checklist.md`**

```markdown
# FORGE Principles Evaluation Checklist

Walk through each item. For each, use Grep/Glob to check the Rust code, then mark the status.

## How to evaluate

For each check item:
1. Run the grep pattern listed in `rules.md` for this principle
2. Read the matching code (if any) to verify it meets the rule
3. If no matching code exists and the component is stubbed/planned, mark NOT_YET_APPLICABLE
4. If matching code exists but doesn't meet the rule, mark VIOLATION with evidence
5. If matching code exists and meets the rule, mark PASS

---

## Checklist

### Principle I — Honesty
- [ ] Oracle calls (Reason/Classify/Search) exist as distinct AST expression variants
- [ ] Type system has confidence/uncertainty representation
- [ ] Uncertain values cannot bypass when/match (if checker exists)

### Principle II — Determinism Boundary
- [ ] Purity checker exists and is wired into the check pipeline
- [ ] Purity checker rejects Reason inside pure
- [ ] Purity checker rejects Classify inside pure
- [ ] Purity checker rejects Search inside pure
- [ ] Purity check is recursive (follows function calls, not just direct expressions)

### Principle III — Token Economy
- [ ] Cost tracking component exists (not just a stub)
- [ ] CLI has a `cost` subcommand
- [ ] LLM request structs carry budget/token fields

### Principle IV — Composition Completeness
- [ ] `>>` compose operator exists in AST
- [ ] Type compatibility checking exists for composition
- [ ] All primitive types are covered by compatibility rules

### Principle V — Supervision
- [ ] Agent AST supports failure policy declarations
- [ ] Supervisor strategies are defined (one_for_one/all/rest_for_one)
- [ ] Grammar parses failure policy declarations

### Principle VI — Self-Reference
- [ ] All error enum variants have structured fields (file, line, col)
- [ ] Error output is machine-consumable (not just human-readable strings)
- [ ] Grammar complexity is bounded (<1000 lines in forge.pest)

### Principle VII — Human Ceiling
- [ ] `escalate` is a first-class AST node
- [ ] Grammar parses `escalate` statements
- [ ] Requires guards support `on fail: escalate`

### Principle VIII — Accountability
- [ ] Tracer module exists (not just a stub)
- [ ] Runtime execution paths invoke the tracer
- [ ] Trace format is structured JSON

### Principle IX — Boundary
- [ ] AST supports boundary directives
- [ ] Boundary checker exists (or is clearly planned with issue reference)
- [ ] Checker modules do NOT import runtime modules
```

- [ ] **Step 2: Verify the file**

Run: `head -5 ~/.claude/skills/forge-principles/checklist.md`
Expected: Shows the header

---

### Task 6: Test the hook end-to-end

**Files:**
- None (verification only)

- [ ] **Step 1: Pipe-test the hook script with a simulated git commit payload**

Run: `echo '{"tool_name":"Bash","tool_input":{"command":"git commit -m test"}}' | bash scripts/check-principles.sh`
Expected: exits 0 with no output (no staged .rs files with violations)

- [ ] **Step 2: Validate the settings.json is picked up**

Run: `jq -e '.hooks.PreToolUse[] | select(.matcher == "Bash") | .hooks[] | select(.type == "command") | .command' .claude/settings.json`
Expected: `"bash scripts/check-principles.sh"` and exit 0

- [ ] **Step 3: Test the skill is discoverable**

Run: `ls ~/.claude/skills/forge-principles/SKILL.md`
Expected: file exists

- [ ] **Step 4: Verify no uncommitted repo changes remain**

Run: `git status`
Expected: All repo files (scripts/check-principles.sh, .claude/settings.json) were committed in Tasks 1 and 2. Working tree should be clean (except untracked files outside the repo like the skill files in ~/.claude/).

---

### Task 7: Run the principles audit to establish baseline

**Files:**
- None (verification only)

- [ ] **Step 1: Invoke the skill manually**

Run: `/forge-principles`

Expected output: A 9-principle report. Based on current codebase state, expected results:
- Principle I (Honesty): PASS — Reason/Classify/Search exist as AST variants, ConfidenceSource exists in types.rs
- Principle II (Determinism Boundary): PASS — purity checker exists in resolver.rs, checks all 3 variants
- Principle III (Token Economy): NOT_YET_APPLICABLE — no cost tracker, no providers yet
- Principle IV (Composition Completeness): PASS — Compose expr exists, is_compatible exists
- Principle V (Supervision): NOT_YET_APPLICABLE — agent runtime is stubbed
- Principle VI (Self-Reference): PASS — error variants have file/line/col fields
- Principle VII (Human Ceiling): PASS — Escalate exists as AST node, grammar parses it
- Principle VIII (Accountability): NOT_YET_APPLICABLE — tracer is a stub
- Principle IX (Boundary): PASS — BoundaryDirective in AST, no checker-runtime imports

Expected summary: ~5 PASS, 0 VIOLATION, ~4 NOT_YET_APPLICABLE
