# Agent-to-CLI Delegation Research

Issue: `#165`  
Date: April 10, 2026  
Status: Implemented research + reference assets for `#178`, `#179`, and `#191`

## Summary

This document defines the recommended FORGE pattern for delegating coding work to external CLIs such as Claude Code and Codex.

The main conclusion is simple:

- Default to **one bounded implementer agent** for coding tasks.
- Add an **independent evaluator/reviewer pass** after implementation.
- Use parallel agents only for **read-only exploration** or **disjoint write scopes**.

This matches three independent signals:

- Anthropic recommends structured harnesses for long-running agents and treats parallel agents as workload-specific rather than a universal default.
- The current Claude Code and Codex CLIs both work well as single-session coding engines with resumable session state and machine-readable output.
- OpenClaw's local testing guidance favors deterministic workflow-contract tests first, then narrow live probes for real adapter behavior.

## Research Inputs

### External references

- Anthropic: <https://www.anthropic.com/engineering/harness-design-long-running-apps>
- Anthropic: <https://www.anthropic.com/engineering/multi-agent-research-system>
- Anthropic: <https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents>
- Claude Code product page: <https://www.anthropic.com/product/claude-code>

### Local references

- [roadmap.md](/Users/claudiu/Work/ncmlabs/forge-165/roadmap.md)
- [workflows/dev-cycle.forge](/Users/claudiu/Work/ncmlabs/forge-165/workflows/dev-cycle.forge)
- [docs/2026-04-10-forge-confidence-verification-paper.md](/Users/claudiu/Work/ncmlabs/forge-165/docs/2026-04-10-forge-confidence-verification-paper.md)
- [Claude Code SDK result schema](/Users/claudiu/Work/marci/claude-code/src/entrypoints/sdk/coreSchemas.ts)
- [Claude Code structured I/O](/Users/claudiu/Work/marci/claude-code/src/cli/structuredIO.ts)
- [OpenClaw testing guidance](/Users/claudiu/Work/tools/openclaw/docs/testing.md)
- [OpenClaw session/security model](/Users/claudiu/Work/tools/openclaw/README.md)

## Recommended Coordination Pattern

### Default harness

1. **Plan/inspect**
   - Run the external CLI in read-only mode.
   - Require it to identify target files, constraints, and verification commands.
   - Do not mutate files in this phase.

2. **Implement**
   - Run one coding session with a bounded scope.
   - Allow edits and command execution only as needed for the declared task.
   - Prefer the smallest verification loop that proves correctness.

3. **Evaluate**
   - Run deterministic checks first.
   - Optionally run an independent reviewer session after verification.
   - Treat reviewer output as evidence, not as authority over passing checks.

4. **Resume**
   - If the session is interrupted, resume the same session where the CLI supports it.
   - If resume is unavailable or unreliable, restart from a structured handoff artifact.

### When to use multiple agents

Use multiple external coding agents only when one of these is true:

- work is read-only and parallelizable, such as repo exploration or evidence gathering
- write scopes are disjoint and can be assigned to separate worktrees
- a reviewer is auditing a finished implementation rather than editing in parallel

Do not default to multi-agent coding when:

- the same files are likely to be edited
- the task is a small bugfix or narrow feature
- the next step depends on the previous result
- the expected benefit is “more intelligence” rather than real parallelism

## Useful End-State Agent

The recommended useful tested agent for Phase 2 is **Repo Implementer**.

Inputs:

- task prompt
- repo path
- declared scope
- verification commands
- optional budget and timeout

Outputs:

- concise human summary
- structured machine-readable result
- progress events while running
- verification outcome

Behavior contract:

- inspect first
- change the minimum necessary files
- run only relevant checks
- report what changed, what passed, and what remains risky

## Invocation Patterns

### Claude Code

#### Safe analysis

Observed working command on April 10, 2026:

```bash
printf '%s\n' 'Inspect this repo, identify the bug, and return a 3-step fix plan without editing files or running commands.' \
  | claude --print --bare --output-format json \
      --permission-mode plan \
      --allowedTools Read,Glob,Grep
```

Use this when:

- repo inspection is needed
- edits must be blocked
- a structured final result is enough

#### Streaming analysis

Observed working command:

```bash
printf '%s\n' 'Inspect this repo, identify the bug, and emit stream-json events while planning a fix without editing files.' \
  | claude --print --verbose --bare --output-format stream-json \
      --permission-mode plan \
      --allowedTools Read,Glob,Grep
```

Use this when:

- the adapter must parse intermediate progress
- the caller needs `session_id`, tool activity, and turn-state events

Important finding:

- `stream-json` requires `--verbose`.
- In plan mode, Claude still surfaced tool metadata including `Bash` and `Edit` in the init event, but actual execution of a mutating or command-running step was blocked by permission checks.

#### Code modification

Observed working command:

```bash
printf '%s\n' 'Fix the bug in this repo so the existing unittest passes. Keep the change minimal, run the relevant verification, and summarize what you changed.' \
  | claude --print --bare --output-format json \
      --permission-mode acceptEdits \
      --allowedTools Read,Edit,Glob,Grep,Bash
```

Use this when:

- the task is a bounded code change
- the session needs to edit files and run verification commands

#### Resume

Observed working command:

```bash
printf '%s\n' 'Now give me just the final changed file path and the verification command that passed.' \
  | claude --print --bare --output-format json \
      --resume <session-id>
```

### Codex

#### Safe analysis

Observed working command:

```bash
codex exec --json --sandbox read-only \
  "Inspect this repo, identify the bug, and return a concise fix plan without editing files."
```

Use this when:

- read-only exploration is enough
- event streaming is required
- sandbox isolation should prevent edits

Important finding:

- `codex exec` emits JSONL events rather than a single final result object.
- The stable integration surface is the event stream plus the final assistant message, not a single typed result record.

#### Code modification

Observed working command:

```bash
codex exec --json --full-auto \
  "Fix the bug in this repo so the existing unittest passes. Keep the change minimal, run the relevant verification, and summarize what you changed."
```

Use this when:

- the task is bounded and coding-oriented
- workspace-write sandboxing is acceptable
- the adapter can consume event-stream progress rather than a single result object

#### Resume

Observed available command surface:

```bash
codex exec resume [SESSION_ID] [PROMPT] --json
codex exec resume --last [PROMPT] --json
```

This issue verified the command contract from the live CLI help surface. Resume behavior itself was tested live for Claude and remains ready for direct adapter wiring for Codex in `#191`.

## Session Adapter Mapping

These mappings are normative inputs to `#191`.

### Target `SessionConfig` fields

| FORGE field | Meaning |
|---|---|
| `prompt` | User task or follow-up prompt |
| `agent` | Adapter selector such as `claude` or `codex` |
| `tools` | Allowed tool subset for the external agent |
| `workdir` | Working directory for the session |
| `timeout` | FORGE-managed timeout |
| `budget` | Spending cap when the adapter supports it |
| `resume` | Continue a prior session |
| `output_mode` | Final JSON vs streaming JSON/events |

### Claude adapter

| FORGE field | Claude CLI mapping |
|---|---|
| `prompt` | stdin payload or positional prompt when using `--print` |
| `agent` | `claude` binary |
| `tools` | `--allowedTools ...` |
| `workdir` | process working directory |
| `timeout` | managed by FORGE wrapper, not a native flag |
| `budget` | `--max-budget-usd ...` when budgeted runs are required |
| `resume` | `--resume <session-id>` or `--continue` for recent local session |
| `output_mode = final_json` | `--output-format json` |
| `output_mode = stream` | `--output-format stream-json --verbose` |
| bounded noninteractive mode | `--print --bare` |
| read-only planning | `--permission-mode plan` |
| bounded editing | `--permission-mode acceptEdits` |

### Codex adapter

| FORGE field | Codex CLI mapping |
|---|---|
| `prompt` | positional prompt to `codex exec` |
| `agent` | `codex` binary |
| `tools` | no direct allowlist flag; enforce indirectly via sandbox/approval policy and prompt contract |
| `workdir` | process working directory or `--cd` |
| `timeout` | managed by FORGE wrapper |
| `budget` | no direct CLI cost cap observed in `codex exec`; enforce via FORGE timeout/policy |
| `resume` | `codex exec resume <session-id> [prompt] --json` |
| `output_mode` | `--json` |
| read-only planning | `--sandbox read-only` |
| bounded editing | `--full-auto` or explicit sandbox/approval configuration |

## AgentResult Mapping

`AgentResult` is not implemented yet in this branch. These tables define the target mapping for `#191`.

### Provisional `AgentResult` shape

| Field | Meaning |
|---|---|
| `success` | Overall session success |
| `output` | Final human-readable result text |
| `structured_output` | Structured final payload if provided |
| `session_id` | Resumable session identifier |
| `num_turns` | Turns used |
| `duration_ms` | End-to-end duration |
| `duration_api_ms` | Provider/API time if available |
| `cost_usd` | Total cost if exposed |
| `tokens_in` | Input tokens |
| `tokens_out` | Output tokens |
| `stop_reason` | End condition |
| `permission_denials` | Blocked actions |
| `progress_events` | Parsed intermediate events |
| `artifacts` | Output files or paths when present |
| `claims` | Structured claims for future verification |
| `error` | Terminal error details |

### Claude JSON result mapping

Observed final JSON object from `--output-format json`:

| Claude field | `AgentResult` field |
|---|---|
| `type = result` + `subtype = success` | `success = true` |
| `type = result` + error subtype | `success = false`, `error` |
| `result` | `output` |
| `structured_output` | `structured_output` |
| `session_id` | `session_id` |
| `num_turns` | `num_turns` |
| `duration_ms` | `duration_ms` |
| `duration_api_ms` | `duration_api_ms` |
| `total_cost_usd` | `cost_usd` |
| `usage.input_tokens` | `tokens_in` |
| `usage.output_tokens` | `tokens_out` |
| `stop_reason` | `stop_reason` |
| `permission_denials` | `permission_denials` |
| `uuid` | adapter metadata, not core `AgentResult` |

### Claude stream-json event mapping

Observed event types:

- `system.init`
- `assistant`
- `user` with tool results
- `tool_use` content blocks inside assistant messages
- `result`

Useful progress extraction:

| Claude event | `AgentResult.progress_events` |
|---|---|
| `system.init` | session start metadata |
| assistant `tool_use` block | planned tool action |
| `user` tool result | tool completion payload |
| `result` | terminal event |

Additional fields visible in the local Claude Code schema and/or live stream surface:

- `tool_progress`
- `system.status`
- `system.post_turn_summary`
- `system.task_started`
- `system.task_notification`
- `system.session_state_changed`

These should map into future FORGE lifecycle tracing for Principle VIII.

### Codex JSONL mapping

Observed event types from `codex exec --json`:

- `thread.started`
- `turn.started`
- `item.started`
- `item.completed`
- `turn.completed`

Recommended mapping:

| Codex event/data | `AgentResult` field |
|---|---|
| `thread.started.thread_id` | `session_id` |
| final `item.completed` with `type = agent_message` | `output` |
| `turn.completed.usage.input_tokens` | `tokens_in` |
| `turn.completed.usage.output_tokens` | `tokens_out` |
| missing explicit cost | `cost_usd = None` |
| `item.started` / `item.completed` command executions | `progress_events` |
| failed command-execution items | progress metadata, not terminal failure by themselves |

Important limitation:

- Current Codex JSONL does not expose a single final typed result object analogous to Claude's `result` schema.
- The adapter should therefore treat the event stream as canonical and derive final output from the last assistant message plus terminal usage event.

## Security Findings

- Prefer argv/process invocation in FORGE adapters whenever possible. Avoid `sh -c` unless shell composition is explicitly required.
- Treat prompt construction as untrusted input. Build prompts as raw strings, not shell fragments.
- Claude supports explicit permission modes and tool allowlists. Use them.
- Codex currently relies more on sandbox and approval configuration than on tool allowlists.
- Cost caps are natively available in Claude CLI and should be used for bounded tasks.
- For Codex, enforce budget indirectly with timeout, sandbox, and future FORGE policy gates.
- OpenClaw's local model is directionally correct for FORGE: deterministic evals first, live probes second, and per-session isolation for non-main work.

## Live Validation

All live probes below were run on April 10, 2026.

### Test repo

Disposable repo: `/private/tmp/forge165-agent-repo`

Files:

- `app.py` with a one-line arithmetic bug
- `test_app.py` with a failing `unittest`

Baseline failure:

```bash
python3 -m unittest -v test_app.py
```

Result: failed because `add(2, 3)` returned `-1` instead of `5`.

### Pattern 1: Claude read-only planning

Status: passed

- Command: read-only `claude --print --bare --output-format json --permission-mode plan`
- Result: returned a final JSON object with `session_id`, `num_turns`, `total_cost_usd`, `usage`, and a valid 3-step plan

### Pattern 2: Claude bounded implementation

Status: passed

- Command: `claude --print --bare --output-format json --permission-mode acceptEdits`
- Result: changed `app.py` from `a - b` to `a + b`
- Verification: `python3 -m unittest -v test_app.py` passed

### Pattern 3: Claude resume

Status: passed

- Command: `claude --print --bare --output-format json --resume <session-id>`
- Result: resumed the same `session_id` and returned the changed file path and verification command

### Pattern 4: Codex read-only planning

Status: passed

- Command: `codex exec --json --sandbox read-only`
- Result: emitted JSONL events, inspected the repo, and returned a valid fix plan in the terminal assistant message

### Pattern 5: Codex bounded implementation

Status: passed

- Command: `codex exec --json --full-auto`
- Result: changed `app.py` from `a - b` to `a + b`
- Verification: `python3 -m unittest -v test_app.py` passed

## Recommended Adapter Defaults

### Claude

- analysis: `--print --bare --output-format json --permission-mode plan`
- streaming analysis: `--print --bare --verbose --output-format stream-json --permission-mode plan`
- implementation: `--print --bare --output-format json --permission-mode acceptEdits`
- resume: `--resume <session-id>`

### Codex

- analysis: `codex exec --json --sandbox read-only`
- implementation: `codex exec --json --full-auto`
- resume: `codex exec resume <session-id> --json`

## Principles Audit

- Principle I: this document records observed CLI behavior directly and marks inferred behavior explicitly
- Principle III: cost/token availability is documented adapter by adapter
- Principle VIII: progress and lifecycle events are captured as first-class adapter outputs
- Principle IX: this issue stays at the research/spec layer and does not implement runtime session adapters
