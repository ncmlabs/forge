# Conformance Suite: JSON-Format Language-Agnostic Test Cases

**Issue:** #27
**Date:** 2026-04-02

## Problem

FORGE needs a machine-readable test suite that proves any implementation is correct. AI coding tools should be able to implement FORGE correctly from the conformance suite alone. This is the adoption lever for multi-implementation support.

## Scope

**In scope:**
- JSON schema for conformance test format
- Test files covering: parser, type checker (all 6 checkers), runtime (trace shape), errors
- Rust validation harness that runs all JSON tests via `cargo test`
- At least one positive and one negative test per category
- At least one test per checker module

**Out of scope:**
- Numeric error code system (tests match on message substrings)
- CLI integration tests (runner calls library functions directly)
- Exhaustive coverage of every edge case (this is the minimum conformance set)

## Design

### JSON Test Format

Each test is a single JSON file with this structure:

```json
{
  "name": "unique_test_name",
  "category": "parser | checker | runtime | errors",
  "subcategory": "optional — e.g. pure, states, uncertain",
  "description": "Human-readable description of what this tests",
  "input": "forge source code as string",
  "expected": {
    "outcome": "parse_ok | parse_error | compile_ok | compile_error | run_ok | run_error",
    "error_contains": ["substring1", "substring2"],
    "error_kind": "error | warning",
    "trace_shape": ["event_type1", "event_type2"]
  },
  "mock_responses": [
    {"text": "response text", "confidence": 0.95}
  ]
}
```

**Field rules:**
- `name`, `category`, `description`, `input`, `expected`, `expected.outcome` — always required
- `input` — string for single-file tests; array of `{"file": "name.forge", "source": "..."}` for multi-file (boundary) tests
- `error_contains` — required when outcome is `*_error`; all substrings must appear in at least one diagnostic message
- `error_kind` — optional, defaults to `"error"`; set to `"warning"` for requires checker warnings
- `trace_shape` — required when outcome is `run_ok` or `run_error`; ordered subsequence of expected trace event types
- `mock_responses` — required when outcome is `run_ok` or `run_error`; scripted LLM responses consumed in order

### Outcome Types

| Outcome | What the runner does |
|---------|---------------------|
| `parse_ok` | Parse input, expect success |
| `parse_error` | Parse input, expect failure, check `error_contains` |
| `compile_ok` | Parse + run all checkers, expect zero errors |
| `compile_error` | Parse + run all checkers, expect at least one matching diagnostic |
| `run_ok` | Parse + check + execute with mock provider, verify `trace_shape` |
| `run_error` | Parse + check + execute, expect runtime error matching `error_contains` |

### Trace Shape Matching

Trace shape uses **subsequence matching**: the expected event types must appear in order within the actual trace, but extra implementation-specific events between them are allowed. This makes tests resilient across implementations.

Valid trace event types (from tracer.rs): `llm_request`, `llm_response`, `when_dispatch`, `task_call`, `task_return`, `flow_start`, `flow_complete`, `wave_complete`, `agent_start`, `agent_handle`, `state_transition`, `event_emit`, `timer_start`, `timer_fire`, `escalate`.

### Directory Layout

```
conformance/
  schema.json                          # JSON Schema (draft 2020-12)
  parser/
    valid_task.json                    # task declaration parses
    valid_pure.json                    # pure function parses
    valid_flow.json                    # flow declaration parses
    valid_agent.json                   # agent with handlers parses
    valid_states.json                  # states declaration parses
    valid_warden.json                  # warden declaration parses
    invalid_syntax.json                # malformed syntax fails
    invalid_indent.json                # bad indentation fails
  checker/
    pure_no_reason.json                # pure + reason → error
    pure_no_classify.json              # pure + classify → error
    pure_no_try_or.json                # pure + try/or → error
    pure_no_escalate.json              # pure + escalate → error
    states_illegal_transition.json     # transition not in declared paths → error
    states_unknown_target.json         # transition to undeclared state → error
    requires_llm_warning.json          # reason in requires → warning
    uncertain_inline_oracle.json       # oracle in give → error
    uncertain_unhandled.json           # tainted value without when → error
    boundary_endpoint_in_shared.json   # endpoint outside server → error
    boundary_cross_ref.json            # server symbol from client → error (multi-file)
    warden_unknown_managed.json        # managing undeclared agent → error
    warden_escalation_order.json       # wrong escalation ladder order → error
    compile_ok_clean_program.json      # valid program compiles cleanly
  runtime/
    hello_task.json                    # simple task executes
    confidence_dispatch.json           # when/sure branch taken
    when_unsure_branch.json            # unsure path taken on low confidence
    task_chain.json                    # sequential task calls
  errors/
    error_message_format.json          # error messages contain file/line info
```

### Multi-File Test Format

For boundary checker tests requiring multiple files:

```json
{
  "name": "boundary_cross_ref",
  "category": "checker",
  "subcategory": "boundary",
  "description": "Server-only symbol cannot be referenced from client boundary",
  "input": [
    {"file": "server.forge", "source": "#! boundary: server\ntask internal needs x: Text gives Text do\n  give x\n"},
    {"file": "client.forge", "source": "#! boundary: client\ntask use_it needs x: Text gives Text do\n  result = internal(x)\n  give result\n"}
  ],
  "expected": {
    "outcome": "compile_error",
    "error_contains": ["cannot reference", "server"]
  }
}
```

### Rust Validation Harness

File: `tests/conformance_runner.rs`

**Approach:** Use the `datatest-stable` crate (or manual glob + macro) to generate one `#[test]` per JSON file. Each test:

1. Read and deserialize the JSON file into `ConformanceTest` struct
2. Dispatch based on `expected.outcome`:
   - **parse_ok/parse_error**: Call `forge::parser::parse()` on input
   - **compile_ok/compile_error**: Parse, then call `forge::checker::check_all()`. For multi-file input, call `forge::checker::boundary_checker::check_boundary()` with all programs
   - **run_ok/run_error**: Parse, check, build mock provider from `mock_responses`, execute via `TaskExecutor`, capture trace events
3. Assert:
   - For `*_ok` outcomes: no errors (or no errors of matching kind)
   - For `*_error` outcomes: at least one diagnostic where all `error_contains` substrings appear in the message, and `error_kind` matches
   - For `run_*` outcomes: trace events contain `trace_shape` as an ordered subsequence

**Key types:**
```rust
struct ConformanceTest {
    name: String,
    category: String,
    subcategory: Option<String>,
    description: String,
    input: InputKind,  // String or Vec<FileInput>
    expected: Expected,
    mock_responses: Option<Vec<MockResponse>>,
}

enum InputKind {
    Single(String),
    Multi(Vec<FileInput>),
}

struct FileInput { file: String, source: String }

struct Expected {
    outcome: Outcome,
    error_contains: Option<Vec<String>>,
    error_kind: Option<String>,
    trace_shape: Option<Vec<String>>,
}

struct MockResponse { text: String, confidence: f64 }
```

### JSON Schema (schema.json)

JSON Schema draft 2020-12 validating the test format. Key constraints:
- Required: `name`, `category`, `description`, `input`, `expected`
- `category` enum: `["parser", "checker", "runtime", "errors"]`
- `outcome` enum: `["parse_ok", "parse_error", "compile_ok", "compile_error", "run_ok", "run_error"]`
- Conditional: `error_contains` required when outcome contains `_error`
- Conditional: `mock_responses` required when outcome starts with `run_`
- `input` is oneOf: string, array of {file, source} objects

## Files to Create/Modify

**Create:**
- `conformance/schema.json` — JSON Schema definition
- `conformance/parser/*.json` — ~8 parser tests
- `conformance/checker/*.json` — ~15 checker tests
- `conformance/runtime/*.json` — ~4 runtime tests
- `conformance/errors/*.json` — ~1 error format test
- `tests/conformance_runner.rs` — Rust validation harness

**Modify:**
- `Cargo.toml` — add `datatest-stable` or `glob` dev-dependency if needed

## Verification

1. `cargo test conformance` — all JSON tests pass
2. Manually corrupt a test expectation → verify it fails with clear message
3. Validate all JSON files against `conformance/schema.json` (can use `jsonschema` CLI or add schema validation to the runner)
4. Each checker module has at least one test
5. Each category has at least one positive and one negative test
