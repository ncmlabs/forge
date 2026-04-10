---
name: codex
description: Use Codex CLI as a bounded coding agent for repo exploration, implementation, review, and session resume.
timeout: 1800
allowed-tools:
  - Bash(codex:*)
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

# Codex CLI Skill

Use this skill when FORGE needs to delegate coding work to Codex CLI.

Default operating pattern:

1. Explore in read-only sandbox mode.
2. Implement with bounded automatic execution.
3. Verify with deterministic checks.
4. Resume by session id when follow-up is needed.

Codex currently exposes its machine-readable interface as JSONL events. Treat the event stream as canonical and derive the final result from the last assistant message plus terminal usage events.

## Capabilities

### `explore(prompt, repo_path)`

Use for read-only planning and repo analysis.

Preferred command:

```bash
codex exec --json --sandbox read-only "$PROMPT"
```

Expected behavior:

- no file edits
- command execution restricted by sandbox mode
- progress available via `item.started` and `item.completed`

### `implement(prompt, repo_path, verify_cmd)`

Use for bounded code changes.

Preferred command:

```bash
codex exec --json --full-auto "$PROMPT"
```

Prompt requirements:

- specify the target repo and allowed scope
- require minimal edits
- name the verification command
- require a final summary of what changed and what passed

Expected behavior:

- Codex may edit files and run shell commands
- progress is visible through JSONL events
- final answer is delivered as the last assistant message

### `review(prompt, repo_path)`

Use for read-heavy verification.

Preferred command:

```bash
codex exec --json --sandbox read-only "$PROMPT"
```

Review prompts should ask for:

- findings first
- concrete file references
- missing verification
- residual risk

### `resume_session(session_id, prompt)`

Use to continue a prior noninteractive session.

Preferred command:

```bash
codex exec resume "$SESSION_ID" "$PROMPT" --json
```

The CLI also supports:

```bash
codex exec resume --last "$PROMPT" --json
```

## Session Adapter Mapping

| FORGE session field | Codex CLI |
|---|---|
| `prompt` | positional prompt |
| `agent = "codex"` | `codex exec` |
| `tools` | no direct allowlist flag |
| `workdir` | process working directory or `--cd` |
| `timeout` | managed by FORGE wrapper |
| `budget` | no direct cost-cap flag observed |
| `resume` | `codex exec resume <session-id> [prompt] --json` |
| `output_mode` | `--json` |
| analysis mode | `--sandbox read-only` |
| bounded edit mode | `--full-auto` |

## AgentResult Mapping

Map the JSONL stream into `AgentResult` as follows:

| Codex event or field | AgentResult field |
|---|---|
| `thread.started.thread_id` | `session_id` |
| final assistant `item.completed` | `output` |
| `turn.completed.usage.input_tokens` | `tokens_in` |
| `turn.completed.usage.output_tokens` | `tokens_out` |
| command execution items | `progress_events` |
| failed command items | progress metadata or `error` if terminal |

Important limitation:

- Current `codex exec --json` does not emit a single terminal object equivalent to Claude's `result` schema.
- Cost is not directly exposed in the observed JSONL event stream.

## Safety Rules

- Use `--sandbox read-only` for exploration and review.
- Use `--full-auto` only for bounded implementation tasks.
- Do not rely on prompt-only tool restrictions; Codex does not currently expose a Claude-style `--allowedTools` surface.
- Prefer one implementer session plus a separate reviewer session over parallel editing sessions.
