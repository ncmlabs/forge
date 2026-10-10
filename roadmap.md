# FORGE Roadmap — v0.3

> Reset 2026-10-09. The previous roadmap (v3) is archived at
> [`docs/archive/roadmap-v3.md`](docs/archive/roadmap-v3.md). See
> [Deprecated in the 2026-10-09 reset](#deprecated-in-the-2026-10-09-reset)
> for what was cut and why.

## North star

**An agent should be able to write FORGE that builds a production system.**

That is [Principle VI — Self-Reference](forge-principles.md) as a measurable
claim: not "FORGE can express this", but "an agent, given the language and the
CLI contract, writes it correctly and cheaply".

## What FORGE is and is not

FORGE owns **orchestration semantics**: agents, systems, events, wardens,
skills, state machines, tracing, uncertainty and the determinism boundary.
Those are language surface, and they stay here.

Apps own **delivery surfaces**: web UI, Slack, TUI. They are built *with*
FORGE and live outside this repository.

Two consequences of that split are the Lean core track below: clone-dev moves
to its own repository (#487), and the showcases — wiki, sentinel, tictactoe —
move to their own repository (#488).

## Where we are

- **v0.2.0 shipped 2026-08-31.** Layer 1 (substrate) and the Phase 2 toolkit
  are in the runtime; the language, compiler and CLI work.
- **Clone-dev v1 proved the pipeline one issue deep.** The reference run took
  one real GitHub issue from signed wake to merged PR with first-try green CI,
  but could not complete the intended ten-issue sweep. That result is the
  Layer 3 v1 outcome: [`docs/clone-dev-v1-retrospective.md`](docs/clone-dev-v1-retrospective.md).
- What the retrospective did **not** show is that an agent can author FORGE
  reliably. That is the gap v0.3 exists to close.

## Current milestone: v0.3 — Agent-Native FORGE

Make FORGE **learnable in one page** and **exactly machine-readable through the
CLI contract**, so an agent can author it without a human translating. Concretely
that means: a one-page language card, a stable JSON envelope for every command,
structured errors with semantic exit codes, schema introspection, dry-run
planning, and record/replay tests that cost nothing to run.

**Exit metric.** The LLM authoring benchmark (#483) reports **pass@1** and
**pass@3-repair** for Claude, an OpenAI-compatible model and a local model,
tracked weekly. v0.3 is done when those numbers are published and improve.

## Tracks

### Lean core — remove what is not language

| Issue | Title | Status |
| ----- | ----- | ------ |
| [#473](https://github.com/ncmlabs/forge/issues/473) | docs: reset roadmap to v0.3 — Agent-Native FORGE | Done ✅ |
| [#485](https://github.com/ncmlabs/forge/issues/485) | lean: remove web app stack from core (templates, static/Tailwind, hot-reload, markdown) | Open |
| [#486](https://github.com/ncmlabs/forge/issues/486) | lean: close WASM/browser targets; keep server/shared boundary checker | Done ✅ |
| [#487](https://github.com/ncmlabs/forge/issues/487) | lean: extract clone-dev into ncmlabs/forge-clone-dev | Open |
| [#488](https://github.com/ncmlabs/forge/issues/488) | examples: create public ncmlabs/forge-examples and move showcases | Done ✅ |
| [#503](https://github.com/ncmlabs/forge/issues/503) | llm: `[llm.routing]` accepts provider chains per phase (primary + fallbacks) | Done ✅ |

### CLI contract — the interface an agent actually uses

| Issue | Title | Status |
| ----- | ----- | ------ |
| [#474](https://github.com/ncmlabs/forge/issues/474) | diagnostics: stable error codes, real file path, plain output off-TTY, forge explain | Done ✅ |
| [#475](https://github.com/ncmlabs/forge/issues/475) | cli: JSON output envelope, semantic exit codes and --fields for every command | Open |
| [#476](https://github.com/ncmlabs/forge/issues/476) | cli: forge schema and forge help --json introspection | Open |
| [#477](https://github.com/ncmlabs/forge/issues/477) | cli: forge run --dry-run static execution plan (LLM sites, cost, effects) | Open |
| [#478](https://github.com/ncmlabs/forge/issues/478) | test: forge run --record and forge test replay through MockProvider | Open |
| [#479](https://github.com/ncmlabs/forge/issues/479) | runtime: end-of-run summary (calls, tokens, cost, confidence, warden events) | Open |
| [#489](https://github.com/ncmlabs/forge/issues/489) | ci: clippy -D warnings fails on stable 1.99 (double_must_use via async-trait 0.1.89) | Done ✅ |
| [#507](https://github.com/ncmlabs/forge/issues/507) | runtime: background command status()/output() confidence follows exit status | Done ✅ |

### Authorability — make the language writable from one page

| Issue | Title | Status |
| ----- | ----- | ------ |
| [#480](https://github.com/ncmlabs/forge/issues/480) | docs: forge-card.md + llms.txt — the one-page language for agents | Done ✅ |
| [#496](https://github.com/ncmlabs/forge/issues/496) | checker: undefined calls, undeclared match variants and case-mismatched tag patterns pass forge check | Done ✅ |
| [#481](https://github.com/ncmlabs/forge/issues/481) | cli: forge init <template> project scaffolding | Open |
| [#482](https://github.com/ncmlabs/forge/issues/482) | skills: installable write-forge skill for Claude Code and Codex | Open |
| [#229](https://github.com/ncmlabs/forge/issues/229) | Automate FORGE derived-surface drift detection (continuous #229) | Open |

### Proof — measure that agents choose and write FORGE

| Issue | Title | Status |
| ----- | ----- | ------ |
| [#483](https://github.com/ncmlabs/forge/issues/483) | bench: LLM authoring benchmark — pass@1 / pass@3-repair across models (supersedes #168) (part 1 corpus done) | Open |
| [#484](https://github.com/ncmlabs/forge/issues/484) | checker: deterministic gate beats oracle — failed command cannot be overruled by reason | Done ✅ |

## Suggested order

1. #489 — unblock CI.
2. #473 — this reset.
3. #474 — structured errors first: everything else reports through them.
4. #475 — the JSON envelope and exit codes.
5. #476, #478, #480 — introspection, record/replay, the one-page card.
6. #487, #488, #485, #486 — the lean core removals.
7. #477, #479, #481, #482 — dry-run, run summary, scaffolding, skill.
8. #484 — the deterministic gate.
9. #483 — the benchmark, last, because it measures everything above it.

## Deprecated in the 2026-10-09 reset

| Issues | What | Why |
| ------ | ---- | --- |
| #168–#175 | Generator/toolkit agents | Superseded by #483 — agents author FORGE directly instead of generating it through a frozen toolkit |
| #183, #185 | Dev orchestration | Moving to `forge-clone-dev` (#487) — app work, not language surface |
| #186, #187 | Cost/Doc agents | Superseded by #477, #479, #480 — cost and docs reporting belong in the CLI contract and the one-page card |
| #53–#56, #67 | WASM/browser targets | Cut (#486) — browsers are a delivery surface, not a FORGE target |
| #242, #280, #301, #305 | App work or research | Parked — out of scope for v0.3 |

The full detail, including the milestone structure and Phase progress counters
these issues were tracked in, is preserved in
[`docs/archive/roadmap-v3.md`](docs/archive/roadmap-v3.md).

## Updating this file

When an issue merges, flip its status to `Done ✅` in the same PR closeout.
Status values are `Open`, `In progress` and `Done ✅`. Keep the track tables and
the milestone above in step with what actually shipped.
