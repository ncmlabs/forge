---
name: claude
description: Use Claude Code CLI as a bounded coding agent for repo exploration, implementation, review, and session resume.
timeout: 1800
allowed-tools:
  - Bash(claude:*)
capabilities:
  - name: explore
    inputs: [Text, Text]
    output: Text
  - name: implement
    inputs: [Text, Text, Text]
    output: Text
  - name: review
    inputs: [Text, Text]
    output: Text
  - name: resume_session
    inputs: [Text, Text]
    output: Text
---

# Claude Code Skill

Use this skill when FORGE needs to delegate coding work to Claude Code CLI.

Default operating pattern:

1. Explore in read-only mode.
2. Implement with bounded edit permissions.
3. Verify with deterministic checks.
4. Resume the same session when follow-up is needed.

Do not use multiple Claude coding sessions in parallel unless the worktrees or file scopes are disjoint.

## FORGE Language Preamble

When the delegated task involves FORGE language code, `.forge` files, language docs, examples, or runtime semantics, prepend this grounding block to the Claude prompt:

```text
You are working in the FORGE repository. Before answering or editing, read the repo truth for this checkout:
- docs/training-development-workflow.md
- workflows/dev-cycle.forge
- docs/forge-reference.md
- grammar/forge.pest
- src/ast.rs
- src/parser.rs
- relevant checker, resolver, runtime, CLI, example, and test files

Treat source and tests as authoritative when docs disagree. Follow the GitHub issue acceptance criteria and FORGE principles.

Current FORGE surfaces to account for include:
- uncertainty handling: do not directly `give` oracle/runtime results; bind then use `when result.sure`, `when result.unsure`, and `else`
- `exec`, `command`, background `command.status/output/cancel`
- `file.read` (server-only) plus schema-driven `toml.parse(text, "TypeName")` / `json.parse(text, "TypeName")` built-ins; `TypeName` is a string literal naming a `type` in the same program and refines the return type to `Named(TypeName)`
- text built-ins: `text.to_number(s)`, `text.replace(s, find, replacement)` (runtime template substitution since `{var}` interpolation is parse-time only), `text.short_id()` (8-char hex from v4 UUID, used for collision-resistant suffixes like dev-cycle workdirs)
- template-string escapes: `\n`, `\r`, `\t`, `\"`, `\\`, `\{`, `\}` — the brace escapes carry literal `{` / `}` past the parser, required when passing placeholder text (e.g. `"\{issue_id\}"`) to runtime helpers like `text.replace`
- `session`, `on progress`/`on complete` hooks, `isolate worktree`, and `gives AgentResult`
- `AgentResult` fields and `metadata.verification`
- `knowledge store`, `recall`, `learn`
- `spawn`, `find`, `retire`, `exportable agent`
- project skills via `forge.project.toml` and `skill.<namespace>.<capability>(...)`
- `schedule` blocks with `mode: spawn` (prompt-driven stateless turn) and `mode: wake` (memory rehydration + event emit); both modes are runtime-dispatched via `WakeService`/`CronDriver`
- server-only `search`, boundary directives, and raw Html interpolation with `{!expr}`

Validation notes:
- Positive single-file examples should pass `cargo run -- check <file>`.
- Manifest/project skill examples must be validated through their `forge.project.toml`.
- Multi-file examples may need manifest or merged-source validation; do not assume each dependent file checks in isolation.
- Known checker limitation: multi-state lifecycle guards such as `lifecycle == a or lifecycle == b` are currently opaque warnings and may make `forge check` exit nonzero.
```

## Capabilities

### `explore(prompt, repo_path)`

Use for read-only planning and repo analysis.

Preferred command:

```bash
printf '%s\n' "$PROMPT" | claude --print --bare --output-format json \
  --permission-mode plan \
  --allowedTools Read,Glob,Grep
```

Use stream output when progress events are needed:

```bash
printf '%s\n' "$PROMPT" | claude --print --verbose --bare --output-format stream-json \
  --permission-mode plan \
  --allowedTools Read,Glob,Grep
```

Expected behavior:

- no file edits
- no shell commands should succeed in plan mode
- output should identify target files, likely fix, and verification commands

### `implement(prompt, repo_path, verify_cmd)`

Use for bounded code changes.

Preferred command:

```bash
printf '%s\n' "$PROMPT" | claude --print --bare --output-format json \
  --permission-mode acceptEdits \
  --allowedTools Read,Edit,Glob,Grep,Bash
```

Required prompt constraints:

- state the allowed scope
- name the verification command
- require minimal edits
- require a final summary of changed files and checks run

Expected behavior:

- Claude may edit files
- Claude may run bounded verification commands
- final output should summarize the change and verification

### `review(prompt, repo_path)`

Use for read-heavy verification after implementation.

Preferred command:

```bash
printf '%s\n' "$PROMPT" | claude --print --bare --output-format json \
  --permission-mode plan \
  --allowedTools Read,Glob,Grep
```

Review prompts should ask for:

- concrete findings first
- file references
- missing verification
- residual risks

### `resume_session(session_id, prompt)`

Use to continue an interrupted or previously successful session.

Preferred command:

```bash
printf '%s\n' "$PROMPT" | claude --print --bare --output-format json \
  --resume "$SESSION_ID"
```

Use resume when:

- the same task continues
- the same repo state is still relevant
- preserving session context is cheaper than rebuilding it

## Session Adapter Mapping

| FORGE session field | Claude CLI |
|---|---|
| `prompt` | stdin payload or positional prompt |
| `agent = "claude"` | `claude` |
| `tools` | `--allowedTools` |
| `workdir` | process working directory |
| `timeout` | managed by FORGE wrapper |
| `budget` | `--max-budget-usd` |
| `resume` | `--resume <session-id>` |
| `output_mode = final_json` | `--output-format json` |
| `output_mode = stream` | `--output-format stream-json --verbose` |
| analysis mode | `--permission-mode plan` |
| bounded edit mode | `--permission-mode acceptEdits` |

## AgentResult Mapping

When `--output-format json` is used, map:

| Claude JSON field | AgentResult field |
|---|---|
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

When `stream-json` is used, capture:

- `system.init`
- `tool_progress`
- `system.status`
- `system.task_started`
- `system.task_notification`
- `system.session_state_changed`
- final `result`

## Safety Rules

- Prefer read-only explore before implementation.
- Always set an explicit permission mode.
- Always run in the target repo working directory.
- Use budget caps for nontrivial implementation runs.
- Prefer one implementer session plus one reviewer session over parallel editing sessions.
