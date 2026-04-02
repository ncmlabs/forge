# E2E Acceptance Test Suite — Design Spec

**Issue:** ncmlabs/forge#26
**Date:** 2026-04-02
**Goal:** Prove Layer 1 (the substrate) is complete via end-to-end acceptance tests

---

## Context

Per roadmap L1.8, the acceptance suite is the final gate before Layer 2 (toolkit agents) can begin. All 12 dependency issues (#7-#12, #16-#21) are closed. This issue creates the error example files, implements the missing uncertain checker, builds the test harness, and runs all 7 acceptance tests to completion.

---

## Scope

### Deliverables

1. **`tests/acceptance.rs`** — 7 acceptance tests + 2 CLI smoke tests
2. **`src/checker/uncertain_checker.rs`** — new checker pass (Principle I enforcement)
3. **`examples/pure_error.forge`** — `reason` inside pure function
4. **`examples/uncertain_error.forge`** — `give` of uncertain value without dispatch
5. **`examples/boundary_error_client.forge`** — client code referencing server symbol
6. **`examples/boundary_error_server.forge`** — server file declaring the referenced symbol

### Not in scope

- Multi-file system executor (tic-tac-toe tested via AgentProcess, not `system` block)
- Changes to existing checker passes
- New CLI flags or mock configuration

---

## Part 1: Error Example Files

### `examples/pure_error.forge`

```forge
pure bad
  needs x: Text
  gives Text
  do
    give reason "think about {x}"
```

Triggers: `pure_checker::PureUsesLlm` — `"pure function 'bad' cannot use 'reason'"`

### `examples/uncertain_error.forge`

```forge
use
  llm.reason

task analyze
  needs topic: Text
  gives Text
  do
    result = reason "analyze {topic}"
    give result
```

Triggers: `uncertain_checker` — `result` is tainted by `reason` and given without `when`/`match` dispatch.

### `examples/boundary_error_server.forge`

```forge
#! boundary: server

task server_only_task
  needs input: Text
  gives Text
  do
    give "processed: {input}"
```

### `examples/boundary_error_client.forge`

```forge
#! boundary: client

fn main
  result = server_only_task("hello")
  say result
```

Triggers: `boundary_checker::check_name_ref` — `"client code references server-only symbol 'server_only_task'"`

---

## Part 2: Uncertain Checker

### Principle

Principle I (Honesty): every oracle call returns `uncertain<T>`. The compiler enforces that `uncertain<T>` cannot be used as `T` without explicitly handling uncertainty.

### Detection Strategy

**Taint tracking within function/task/handler scope:**

1. Walk statements in order. When a `Bind(name, expr)` is encountered where `expr` is (or contains) `Reason`, `Classify`, or `Search`, mark `name` as uncertain-tainted.
2. When a `Give(expr)` is encountered, check if `expr` references a tainted name. If so, and if there was no prior `When` or `Match` statement that dispatched on that name, emit an error.
3. `When` and `Match` statements that reference a tainted name clear the taint for subsequent code (the dispatch "handles" the uncertainty).

### Error format

```
error: unhandled uncertain: `result` may be uncertain and must be dispatched with when/match
  --> uncertain_error.forge:7:10
   |
7  |     give result
   |          ^^^^^^ this value came from an oracle call
   |
   = help: use `when result.sure -> ...` or `match result` to handle uncertainty first
```

### Integration

- New file: `src/checker/uncertain_checker.rs`
- Added as Pass 5 in `src/checker/mod.rs` `check_all()`, after requires checker, before warden checker
- Returns `Vec<Diagnostic>` like the other checkers
- Walks: tasks, flows (per-stage), agents (per-handler), fn main

### Edge cases

- **Reassignment:** If a tainted variable is rebound to a non-oracle expression, taint is cleared
- **Nested expressions:** `give reason "..."` (inline oracle in give) is always an error — the result is never dispatched
- **Pure functions:** Already blocked by pure_checker (can't call reason/classify/search). No overlap.
- **Field access on tainted:** `give result.value` is still tainted — field access doesn't dispatch
- **When/match with give:** `when result.sure -> give result` is valid — the when dispatches first

---

## Part 3: Test Harness (`tests/acceptance.rs`)

### Structure

```rust
// ── Helpers ──────────────────────────────────────────────────

fn parse_file(path: &str) -> Program { ... }
fn check_file(path: &str) -> Vec<Diagnostic> { ... }
fn check_files(paths: &[&str]) -> Vec<Diagnostic> { ... }
fn mock_registry(mock: MockProvider) -> Arc<ProviderRegistry> { ... }
fn errors(diags: &[Diagnostic]) -> Vec<&Diagnostic> { ... }

// ── Checker error acceptance tests ───────────────────────────

#[test] fn accept_uncertain_error() { ... }
#[test] fn accept_pure_error() { ... }
#[test] fn accept_states_error() { ... }
#[test] fn accept_boundary_error() { ... }

// ── Runtime acceptance tests ─────────────────────────────────

#[tokio::test] async fn accept_hello_run() { ... }
#[tokio::test] async fn accept_research_run() { ... }
#[tokio::test] async fn accept_tictactoe_game() { ... }

// ── CLI smoke tests ──────────────────────────────────────────

#[test] fn cli_check_valid_exits_zero() { ... }
#[test] fn cli_check_error_exits_nonzero() { ... }
```

### Helper details

**`parse_file`**: reads file, calls `forge::parser::parse`, unwraps.

**`check_file`**: parses file, runs `forge::checker::check_all(program, filename)`. For boundary errors, also runs `boundary_checker::check`.

**`check_files`**: parses multiple files, runs per-file `check_all` on each, then runs `boundary_checker::check` on all programs together. Returns combined diagnostics.

**`mock_registry`**: wraps a `MockProvider` in a `ProviderRegistry` with `"mock"` as default.

**`errors`**: filters diagnostics to `DiagnosticKind::Error` only.

### Test details

#### `accept_uncertain_error`
```rust
let diags = check_file("examples/uncertain_error.forge");
let errs = errors(&diags);
assert!(!errs.is_empty(), "should detect unhandled uncertain");
assert!(errs.iter().any(|d| d.message.contains("unhandled uncertain")));
```

#### `accept_pure_error`
```rust
let diags = check_file("examples/pure_error.forge");
let errs = errors(&diags);
assert!(!errs.is_empty(), "should detect LLM op in pure function");
assert!(errs.iter().any(|d| d.message.contains("cannot use")));
```

#### `accept_states_error`
```rust
let diags = check_file("examples/states_error.forge");
let errs = errors(&diags);
assert!(!errs.is_empty(), "should detect illegal transition");
// States checker flags: "illegal transition from `done` to `playing`" (no such edge in GamePhase)
assert!(errs.iter().any(|d| d.message.contains("illegal transition")));
```

#### `accept_boundary_error`
```rust
let diags = check_files(&[
    "examples/boundary_error_server.forge",
    "examples/boundary_error_client.forge",
]);
let errs = errors(&diags);
assert!(!errs.is_empty(), "should detect cross-boundary reference");
assert!(errs.iter().any(|d| d.message.contains("server-only symbol")));
```

#### `accept_hello_run`
```rust
let source = std::fs::read_to_string("examples/hello.forge").unwrap();
let program = forge::parser::parse(&source).unwrap();
let mock = MockProvider::new("mock").with_default("mock response");
let executor = TaskExecutor::new(program, mock_registry(mock), None);
let result = executor.run().await;
assert!(result.is_ok());
let outputs = executor.outputs();
assert_eq!(outputs, vec!["Hello, World!"]);
```

#### `accept_research_run`
```rust
let mock = MockProvider::new("mock")
    .with_response("search", "Search results for the topic")
    .with_response("synthesize", "A synthesized report")
    .with_response("factually consistent", "Yes, this is consistent")
    .with_default("mock research response");
// parse, create executor, run, assert outputs non-empty
```

#### `accept_tictactoe_game`

Uses `AgentProcess` pattern from quiz_tutor_test.rs:

1. Load and parse `examples/tictactoe/room_agent.forge` and `examples/tictactoe/platform.forge`
2. Extract `AgentDecl` (room_agent) and `StatesDecl` (GamePhase) and pure function declarations
3. Create `AgentProcess` with mock registry
4. Dispatch scripted events:
   - `join("X")` — player 1 joins
   - `join("O")` — player 2 joins, triggers transition to `playing`
   - `move("X", 0)` — X plays top-left
   - `move("O", 3)` — O plays middle-left
   - `move("X", 1)` — X plays top-center
   - `move("O", 4)` — O plays center
   - `move("X", 2)` — X plays top-right → X wins (top row)
5. Assert: last move returns `GameResult` with `winner: "X"`
6. Assert: agent memory `board` reflects all moves
7. Assert: lifecycle transitioned through `waiting -> playing -> finished`

Mock provider is not needed for this test — `room_agent` calls `check_winner` (a pure function) and `next_player` (also pure). No LLM calls in the game loop.

#### CLI smoke tests

```rust
#[test]
fn cli_check_valid_exits_zero() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_forge"))
        .args(["check", "examples/hello.forge"])
        .output()
        .unwrap();
    assert!(output.status.success());
}

#[test]
fn cli_check_error_exits_nonzero() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_forge"))
        .args(["check", "examples/states_error.forge"])
        .output()
        .unwrap();
    assert!(!output.status.success());
}
```

---

## Part 4: Files Modified

| File | Change |
|------|--------|
| `src/checker/uncertain_checker.rs` | **New** — taint-tracking uncertain checker |
| `src/checker/mod.rs` | Add uncertain_checker as Pass 5 in `check_all` |
| `tests/acceptance.rs` | **New** — 9 acceptance/smoke tests |
| `examples/pure_error.forge` | **New** — error example |
| `examples/uncertain_error.forge` | **New** — error example |
| `examples/boundary_error_server.forge` | **New** — error example (server half) |
| `examples/boundary_error_client.forge` | **New** — error example (client half) |

---

## Verification

### All tests pass

```bash
cargo test --test acceptance -- --nocapture
```

Expected: 9 tests, all green.

### Individual verification

```bash
# Checker errors produce diagnostics
cargo run -- check examples/uncertain_error.forge    # exit 1, "unhandled uncertain"
cargo run -- check examples/pure_error.forge         # exit 1, "cannot use"
cargo run -- check examples/states_error.forge       # exit 1, states error
cargo run -- check examples/boundary_error_server.forge examples/boundary_error_client.forge  # exit 1, cross-boundary

# Runtime tests produce output
FORGE_MOCK=1 cargo run -- run examples/hello.forge        # prints "Hello, World!"
FORGE_MOCK=1 cargo run -- run examples/research.forge     # prints synthesized output
```

### Existing tests still pass

```bash
cargo test
```

All existing 17 test files must remain green.
