# FORGE Principles Enforcement System — Design Spec

> Two-layer enforcement: a fast pre-commit hook for mechanical violations,
> and a deep LLM-powered code review agent for architectural principle adherence.
> Both target the Rust implementation code that builds FORGE.

---

## Problem

FORGE has 9 first principles (in `forge-principles.md`) that every design decision must trace back to. Currently, principle adherence is checked manually. As agents build more of the system, we need automated enforcement that:

1. Blocks commits that introduce mechanical principle violations
2. Provides deep architectural review at milestone boundaries
3. Produces structured output that agents can consume and act on

---

## Components

### 1. Pre-commit Hook (Fast Path)

**Trigger:** Before every `git commit` via Claude Code hook in `.claude/settings.json`
**Speed:** <2 seconds
**Method:** Deterministic pattern matching (grep/shell), no LLM
**Scope:** Only staged `.rs` files
**Action:** Block commit + print structured violation

**What it checks:**

| Principle | Pattern Check |
|---|---|
| II. Determinism Boundary | New code in pure-checking paths must reject all oracle call variants (`Reason`, `Classify`, `Search`). Check that `check_pure_body` or equivalent handles all `Expr` variants. |
| VI. Self-Reference | New error enum variants must include structured fields. Grep for `enum.*Error` additions that lack `file`, `line`, `code` fields. |
| VIII. Accountability | New `CompletionRequest` construction sites must have adjacent tracer calls. Grep for `CompletionRequest` without `trace`/`tracer`/`Tracer` within 10 lines. |
| IX. Boundary | Checker modules must not import from other boundary domains. `boundary_checker.rs` must not import from `runtime/agent.rs`. |

**Implementation:** `scripts/check-principles.sh`
- Receives list of staged `.rs` files
- Runs each pattern check
- Exits 0 (pass) or 1 (violation)
- Outputs structured violation format to stderr

**Hook configuration:** `.claude/settings.json`

The hook uses Claude Code's `PreToolUse` event on `Bash` with an `if` filter matching `git commit` commands. When a `git commit` is about to run, the hook executes the principles checker on staged `.rs` files and blocks the commit if violations are found.

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

The script outputs JSON to block: `{"continue": false, "stopReason": "[PRINCIPLE II] ..."}` on violation, or exits 0 silently on pass.

---

### 2. Code Review Agent (Deep Path)

**Trigger:**
- Automatically via `superpowers:code-reviewer` after implementation steps
- Manually via `/forge-principles` slash command

**Speed:** 30-60 seconds
**Method:** LLM-powered analysis reading Rust source files
**Scope:** All `src/**/*.rs` files, full codebase reasoning
**Action:** Block PR + structured violation report

**Skill location:** `.claude/skills/forge-principles/`

**Skill files:**

#### `skill.md`
The main skill definition. Instructs the agent to:
1. Read `rules.md` for the principle-to-rule mapping
2. Use Grep/Glob to scan `src/**/*.rs`
3. Evaluate each principle against current implementation
4. Produce structured report

#### `rules.md`
The 9 principles distilled into concrete, checkable rules:

**Principle I — Honesty**
- Every `think`/`reason` AST node must produce a type that carries confidence
- No code path allows `uncertain<T>` to be used as `T` without `when`/`match`
- Check: `src/ast.rs` expression types, `src/resolver.rs` type checking, any uncertain checker

**Principle II — Determinism Boundary**
- `pure` checker must reject ALL oracle call variants (Reason, Classify, Search)
- Rejection must be recursive through function calls, not just direct expressions
- Check: `src/resolver.rs` purity checking, any `src/checker/pure_checker.rs`

**Principle III — Token Economy**
- Every `CompletionRequest` path must have cost tracking
- Budget limits must be enforceable at the language level
- `forge cost` command must exist and produce per-step estimates
- Check: `src/llm/cost_tracker.rs`, `src/main.rs` CLI commands

**Principle IV — Composition Completeness**
- Every primitive's output must be compatible with `>>` operator
- `ConfidentValue` (or equivalent) must be the universal interface
- No special adapters needed for any primitive
- Check: `src/types.rs` compatibility rules, `src/resolver.rs` compose checking

**Principle V — Supervision**
- Agent declarations must support failure policy declarations
- Supervisor strategies (one_for_one, one_for_all, rest_for_one) must be implemented
- Missing failure policies should generate warnings/errors
- Check: `src/ast.rs` agent nodes, `src/runtime/supervisor.rs`, `src/runtime/agent.rs`

**Principle VI — Self-Reference**
- Error types must be structured: file, line, col, error code, human message
- Errors must be machine-consumable (structured data, not just strings)
- Compiler must be callable as a tool from within FORGE
- Check: all error enums in `src/`, error formatting in `src/main.rs`

**Principle VII — Human Ceiling**
- `escalate` must be a first-class construct in the AST
- `stuck` detection must be implementable
- `requires` guards must support `on fail: escalate`
- Escalation path must not be harder to write than other paths
- Check: `src/ast.rs` escalate nodes, `src/parser.rs` escalate parsing

**Principle VIII — Accountability**
- `think`, `transition`, `emit`, `requires` guard events must produce traces
- Tracing must be default-on, not opt-in
- Trace format must be structured JSON with defined fields
- Check: `src/tracer.rs`, runtime execution paths

**Principle IX — Boundary**
- Boundary checker must prevent cross-boundary references
- `restricted<T>` must not flow to log/print operations
- Server code must not leak into client bundles
- Check: `src/checker/boundary_checker.rs`, type flow analysis

#### `checklist.md`
Per-principle checklist the agent walks through during review. Each item is either PASS, VIOLATION, or NOT_YET_APPLICABLE.

---

### 3. Output Format

Both components produce the same structured format:

```
[PRINCIPLE {number}] {name}
Status: PASS | VIOLATION | NOT_YET_APPLICABLE
File: {path}:{line}
Issue: {description}
Evidence: {what was found in the code}
Required: {what the principle demands}
```

**NOT_YET_APPLICABLE** — The principle can't be checked because the relevant component doesn't exist yet (e.g., token economy checks before providers are implemented). This prevents false passes on unimplemented features.

The code review agent additionally produces a summary:

```
## Principles Report
- PASS: 5/9
- VIOLATION: 1/9
- NOT_YET_APPLICABLE: 3/9

### Violations
[PRINCIPLE II] Determinism Boundary
...
```

---

## File Layout

```
forge/
  .claude/
    settings.json              # hook configuration
  scripts/
    check-principles.sh        # fast pre-commit checker
  .claude/skills/              # (user's skill directory, not in repo)
    forge-principles/
      skill.md                 # skill definition
      rules.md                 # 9 principles as structured rules
      checklist.md             # per-principle evaluation checklist
```

Note: The skill files live in the user's `.claude/skills/` directory (not in the repo) since they're Claude Code tooling. The hook script lives in `scripts/` (in the repo) since it's versioned with the codebase.

---

## Execution Flow

### On every commit:
```
git commit
  → .claude/settings.json hook triggers
  → scripts/check-principles.sh runs on staged .rs files
  → Pattern checks for Principles II, VI, VIII, IX
  → PASS → commit proceeds
  → VIOLATION → commit blocked, structured error printed
```

### On milestone review:
```
Agent completes implementation step
  → superpowers:code-reviewer triggers
  → forge-principles skill activated
  → Reads rules.md, scans all src/**/*.rs
  → Evaluates all 9 principles
  → Produces structured report
  → PASS → review passes
  → VIOLATION → review blocked, report with fixes
```

### On manual audit:
```
User runs /forge-principles
  → Same as milestone review
  → Full 9-principle audit
```

---

## What This Does NOT Do

- Does not check `.forge` source files (the FORGE compiler itself handles that)
- Does not check design docs or issues for principle alignment (scope limited to Rust code)
- Does not replace the FORGE compiler's own checkers — it checks that the checkers are correctly implemented
- Does not run tests — it analyzes code structure and patterns

---

## Verification

After implementation:
- `git commit` on a staged file that adds a pure function calling think → blocked
- `git commit` on a clean file → passes
- `/forge-principles` produces a 9-principle report with correct PASS/VIOLATION/NOT_YET_APPLICABLE
- `superpowers:code-reviewer` automatically includes principles check
