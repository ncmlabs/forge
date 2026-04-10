---
name: ollama
description: Use Ollama local inference for text generation, classification, and analysis via local GPU models.
timeout: 120
allowed-tools:
  - Bash(ollama:*)
capabilities:
  - name: generate
    inputs: [Text, Text]
    output: Text
  - name: classify
    inputs: [Text, Text, Text]
    output: Text
  - name: analyze
    inputs: [Text, Text]
    output: Text
---

# Ollama Local Inference Skill

Use this skill when FORGE needs local LLM inference without external API calls. Ollama runs on the local GPU server (RTX 3090) and provides zero-cost text generation, classification, and analysis.

Default operating pattern:

1. Select the model based on task complexity (fast, balanced, or high).
2. Pipe the prompt to `ollama run` via stdin for multi-line prompts.
3. Capture stdout as the complete response.
4. Parse or post-process the text output as needed.

Ollama is not a coding agent. It does not edit files, manage sessions, or run tools. Use it for inference tasks: classification, summarisation, text generation, and structured analysis.

## Model Selection

Choose the model based on the task:

| Routing tier | Model | Use case |
|---|---|---|
| `fast` | `gpt-oss:latest` | Short answers, quick classification, low-latency checks |
| `balanced` | `qwen3-coder:latest` | Code analysis, structured classification, moderate reasoning |
| `high` | `qwen3.5:27b` | Complex reasoning, long-form analysis, nuanced generation |

When the caller does not specify a model, default to `qwen3-coder:latest`.

## Capabilities

### `generate(prompt, model)`

General-purpose text generation.

Preferred command:

```bash
printf '%s\n' "$PROMPT" | ollama run "$MODEL"
```

For short single-line prompts:

```bash
ollama run "$MODEL" "$PROMPT"
```

Expected behavior:

- stdout contains the full generated text
- no side effects (no file writes, no network calls)
- exit code 0 on success

### `classify(text, categories, model)`

Classify input text into one of the provided categories.

Preferred command:

```bash
printf 'Classify the following text into exactly one of these categories: %s\n\nText: %s\n\nRespond with only the category name.' "$CATEGORIES" "$TEXT" | ollama run "$MODEL"
```

Expected behavior:

- output is a single category name from the provided list
- use `qwen3-coder:latest` or `gpt-oss:latest` for fast classification
- useful for urgency detection, intent classification, and triage

### `analyze(prompt, model)`

Longer-form analysis such as code review summaries, log analysis, or document review.

Preferred command:

```bash
printf '%s\n' "$PROMPT" | ollama run "$MODEL"
```

Expected behavior:

- output is structured prose (findings, recommendations, risks)
- prefer `qwen3.5:27b` for complex analysis tasks
- prefer `qwen3-coder:latest` for code-specific analysis

## Session Adapter Mapping

| FORGE session field | Ollama CLI |
|---|---|
| `prompt` | stdin pipe or positional arg |
| `agent = "ollama"` | `ollama run` |
| `model` | model name arg (e.g., `qwen3-coder:latest`) |
| `tools` | N/A |
| `workdir` | process working directory |
| `timeout` | managed by FORGE wrapper |
| `budget` | N/A (local inference, no cost) |
| `resume` | N/A |
| `output_mode` | N/A (always plain text) |

## AgentResult Mapping

| Ollama output | AgentResult field |
|---|---|
| stdout (full text) | `output` |
| exit code 0 | success (no error) |
| non-zero exit code | `error` |
| N/A | `session_id` (not supported) |
| N/A | `tokens_in` (not exposed by CLI) |
| N/A | `tokens_out` (not exposed by CLI) |
| 0.00 | `cost_usd` (local, always zero) |

Limitation: `ollama run` does not expose token usage or timing metadata in its CLI output. If token tracking is needed, use the OpenAI-compatible API endpoint (`http://<host>:11434/v1`) instead of the CLI.

## Safety Rules

- Ollama is inference-only. Never expect it to edit files or execute commands.
- Always validate that the target model is pulled before invoking (`ollama list`).
- Prefer stdin piping for prompts containing quotes, newlines, or special characters.
- Set FORGE-level timeouts to guard against slow generation on large models.
- Do not send sensitive data through Ollama unless the server is on a trusted network.
- Cap confidence at 0.80 for Ollama outputs (local models are less capable than frontier APIs).
