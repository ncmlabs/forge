---
name: write-forge
description: Writing, checking or running FORGE `.forge` programs — the authoring loop, the CLI's JSON envelope and exit codes, and the syntax and checker mistakes that break agents. Use when a task asks for a FORGE program or `.forge` file, names FORGE tasks/flows/agents/pools/wardens, or when the project has a forge.project.toml.
---

# Writing FORGE

FORGE is a language for oracle-augmented computation: LLM calls are primitives, uncertainty is a
compile-time type, and independent work runs in parallel. Deterministic logic lives in `pure`,
stochastic logic in `task`, and agents are first-class with memory, lifecycle states, events and
supervision.

Read [`forge-card.md`](forge-card.md) before writing code — it ships with this skill: the syntax
that trips agents, one idiom per primitive, and the checker errors you will actually hit.
`docs/forge-reference.md` in the FORGE repo is the full authority, and `grammar/forge.pest` wins
any disagreement.

## When to choose FORGE

Choose it when the task is an orchestration of LLM work:

- several LLM steps, where independent ones should run in the same wave (a `flow` stage without
  `needs`, or `pool` workers);
- output whose confidence matters — `uncertain<T>` is a type and `when x.sure` is the dispatch;
- a long-lived agent: memory, lifecycle states, events, supervision by a `warden`;
- a visible boundary between deterministic logic (`pure`) and oracle calls (`task`).

Do not choose it for plain data munging with no LLM or effect (write a script), for a single API
call, or when the task needs a library ecosystem FORGE does not have — reach for
`command ["argv", "array"]` or `exec` instead of inventing a capability.

## The authoring loop

Run these in order. A program is not ready until it has been through all six.

**1. Scaffold — `forge init`**

```bash
forge init my-app --template pipeline   # templates: pipeline, agent, webhook-bot
forge init my-app --dry-run             # list the files, write nothing
```

Writes `forge.project.toml`, `forge.config.toml` (mock provider by default), `main.forge`,
`main.forge.fixtures.json`, `expected.txt`, `.gitignore` and `AGENTS.md`.

**2. Check — `forge check --json`**

```bash
forge check --json main.forge
```

Parse + resolve + check, one JSON envelope on stdout. Read `data.diagnostics[]` and apply each
`help` — that key carries the fix text (there is no separate `fix` key) — then re-check. Loop until
`data.diagnostics` is empty and the exit code is `0`. Inside a project, check the manifest instead
of the file: `forge check --manifest forge.project.toml` registers the declared `skill.*`
capabilities and checks every source.

**3. Explain — `forge explain <code>`**

```bash
forge explain E020
forge explain --list
```

Every diagnostic has a stable code. `explain` prints what it means with a wrong/right example;
`next_steps` in the check envelope already hands you `forge explain <code>` for each distinct error.

**4. Replay — `forge test`**

```bash
forge run main.forge --record   # once: record every provider response as fixtures
forge test main.forge           # replay them — no provider, no tokens, $0
```

`test` replays `<file>.fixtures.json` beside the program. With no fixtures it exits `1` and tells
you the `--record` command to run.

**5. Plan — `forge run --dry-run`**

```bash
forge run --dry-run main.forge
```

Prints the static execution plan — the cost table of every LLM operation, then the effect sites
(`command`, `exec`, `http`, `file.write`, `session`, `spawn`) — and the line `nothing was executed`.
Nothing runs, no provider is built. Read it as the cost and blast-radius review before step 6.

**6. Run — `forge run --json`**

```bash
FORGE_MOCK=1 forge run main.forge   # whole program on the mock provider, still $0
forge run --json main.forge         # the real run
```

`data.output` holds the program's `say` lines (in JSON mode nothing is printed to stdout),
`data.result` the returned value, and `data.summary` the run's economics.

## Exit codes

Semantic in both modes — `--json` never changes them.

| Code | Meaning |
|------|---------|
| `0` | success — clean check, completed run |
| `1` | error — diagnostics, a blocked run, or an invalid command line |
| `2` | warnings only — `check` (or `run --dry-run`) found warnings but no errors, and nothing else |

`3` (partial), `10` (needs input) and `11` (async pending) are reserved. In a loop, treat `2` as
"read `data.warnings`, then keep going"; treat `1` as "stop and fix".

## The JSON envelope

Every in-scope command emits one document — `--json`, or `FORGE_OUTPUT=json` for a whole session:

| Field | What an agent reads it for |
|-------|----------------------------|
| `status` | `success`, `error`, `warning` or `pending` |
| `command` | which subcommand produced it |
| `data` | the payload; `--fields diagnostics` keeps only what you need |
| `context` | one sentence on what happened |
| `next_steps` | the commands to run next (`forge explain <code>`) |
| `warnings` | non-fatal findings; for `check` they are also in `data.diagnostics` |
| `cost` | USD this command spent; `0.0` when it made no LLM call |
| `error` | `null`, else `{type, code, message, suggestion}` |
| `duration_ms` | wall-clock time |

Payloads worth knowing: `check` → `{files, diagnostics[]}` where each diagnostic has
`code`, `severity`, `file`, `line`, `col`, `message`, `label`, `help`; `run`/`test` →
`{file, output, result, summary}`; `run --dry-run` → `{dry_run, file, operations, effects,
total_tokens_in, total_tokens_out, estimated_cost_usd}`; `explain` → `{code, title, explain}`.
`line`/`col` are `null` when the diagnostic names a file that was not parsed — never a fake `1:1`.

## The five mistakes that break most FORGE programs

1. **`when` takes an arrow, not a block.** `when x.sure -> give x`, with `else -> ...` as a sibling
   at the same indentation. Only `if`, `for` and `match` take an indented body.
2. **`fn main` has no parentheses and no return type.** Write `fn main`, never `fn main()`, and its
   body sits at i1 with no `needs`/`gives`/`do`. Keep it thin — put branching in a `task` or `pure`.
3. **Indentation is exact: 2 spaces per level** (i1–i4, nesting stops at i4), never a tab, and a
   blank line inside a body ends the block — the next indented line then fails to parse.
4. **`pure` cannot call LLMs.** No `reason`, `classify`, `search`, `recall`, `exec`, `command`,
   `session`, `file.read`, `skill.*`, `escalate`, `try ... or`, and no call to a `task`. Move the
   line into a `task`; `pure` is deterministic and always confidence `1.0`.
5. **`uncertain<T>` is handled by binding, then dispatching.** `reason`, `classify`, `search`,
   `recall`, `exec`, `command`, `session`, `file.read`, `toml.parse`, `json.parse` and every
   `skill.*` call return an uncertain value, and taint survives assignment and field access. Never
   `give` the raw result and never inline an oracle in `give` (that is the E020 error): bind it,
   then `when result.sure -> ...` / `.unsure` / `else`, or `match`. Thresholds: `sure` ≥ 0.8,
   `unsure` 0.5–0.8, `unreliable` < 0.5; override with `when result.sure(above: 0.9)`.
