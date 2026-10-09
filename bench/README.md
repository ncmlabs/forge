# FORGE authoring benchmark — spec corpus

Part 1 of [#483](https://github.com/ncmlabs/forge/issues/483). This directory is
the **corpus only**: the natural-language specs, a reference solution per spec,
and the offline acceptance fixture. The model-running harness (prompting a model,
collecting `forge check --json` diagnostics, repairing, reporting a table) is
part 2 and lands after [#475](https://github.com/ncmlabs/forge/issues/475).

## What the benchmark measures

Given `docs/forge-card.md` and one `spec.md`, a model writes a single `.forge`
program. Three numbers come out of that:

| Metric | Definition |
| ------ | ---------- |
| **pass@1** | The first draft passes `forge check` with **zero diagnostics** — no errors *and* no warnings. |
| **pass@3-repair** | The program passes `forge check` within 3 attempts total, where each repair is driven by the diagnostics from the previous attempt (the draft → deterministic validate → repair loop of [#471](https://github.com/ncmlabs/forge/pull/471), validated with `forge check --json`). |
| **correctness** | `forge test <model>.forge --expect expected.txt` exits 0 against the spec's recorded fixture: the program reproduces the required output exactly. |

The model never sees `reference.forge`, `reference.forge.fixtures.json`, or
`expected.txt`. Those exist so the benchmark costs nothing to grade: replay never
calls a provider and reports zero tokens and zero cost
([#478](https://github.com/ncmlabs/forge/issues/478)).

## Layout

```
bench/
  README.md                  this file
  mock.config.toml           a copy of config/mock.config.toml (the mock provider)
  specs/
    01-task-greeting/
      spec.md                  the task the model receives (5–15 lines, no FORGE code)
      reference.forge          a correct solution; never shown to the model
      reference.forge.fixtures.json   recorded provider responses
      expected.txt             the reference's stdout during recording
      meta.toml                difficulty, features, oracle_calls
```

`tests/bench_corpus_tests.rs` enforces the corpus contract: exactly 30 specs in
a 10 easy / 12 medium / 8 hard spread, all five files per spec, 5–15 non-blank
lines per `spec.md` (counted there), a reference that checks with zero
diagnostics, and a `forge test --expect` replay that exits 0 for every spec.

`meta.toml`:

```toml
difficulty = "easy"        # easy | medium | hard
features = ["task", "when"] # from the vocabulary below, non-empty
oracle_calls = 1           # provider calls the reference makes
```

Feature vocabulary: `task`, `pure`, `flow`, `when`, `match`, `if`, `for`,
`agent`, `states`, `requires`, `event`, `pool`, `warden`, `contract`, `system`,
`command`, `knowledge`, `spawn`, `file`, `json`.

## How a fixture is produced

Fixtures and `expected.txt` are **never hand-written**. From the repository root:

```sh
cd bench/specs/<NN>-<slug>
FORGE_CONFIG=bench/mock.config.toml forge run reference.forge --record > expected.txt
FORGE_CONFIG=bench/mock.config.toml forge test reference.forge --expect expected.txt
```

The record run writes `reference.forge.fixtures.json` beside the program, and
`forge test` replays it offline and diffs stdout against `expected.txt`.
`tests/bench_corpus_tests.rs` re-runs the replay for every spec in CI.

## What the mock provider makes deterministic

`bench/mock.config.toml` selects the built-in mock provider. It cannot be given
canned responses through config, so every oracle call returns the same text:

- `reason` → `mock response` at confidence 0.85, so `.sure` is the branch that
  fires. Specs state the required output for that branch.
- `classify` → `mock response`, which is never one of the request's labels, so a
  `match` on a classification always lands on its `_` arm. Specs are written so
  that is the meaningful outcome.
- `command` / `exec` really execute, so their stdout, stderr, exit status and
  confidence are real and stable. This is where the deterministic gate
  ([#484](https://github.com/ncmlabs/forge/issues/484)) is exercised: at least
  four specs require a `command` result to be checked before any oracle decision
  is allowed to matter.
- Agent handlers do not run under `forge run`: a program's observable output
  comes from `fn main` and the flows, pools and tasks it calls. Agent, event,
  system and warden specs are therefore checked for structure by `forge check`
  and for their `fn main` output by the fixture.

## Adding a spec

1. Create `bench/specs/<NN>-<slug>/`; `<NN>` is zero-padded and keeps the
   directory listing in difficulty order.
2. Write `spec.md`: 5–15 lines of plain language. State the inputs, the exact
   required output text, and which FORGE features are expected. No FORGE code,
   and do not paraphrase the reference's structure.
3. Write `reference.forge` from `docs/forge-card.md` and `examples/basics` /
   `examples/llm` only. Run `forge check reference.forge` — it must print `OK`
   with zero diagnostics. If the card lacks something you need, report the gap
   instead of inventing syntax.
4. Record the fixture and `expected.txt` with the two commands above; both must
   exit 0 on a second run.
5. Write `meta.toml` and set `oracle_calls` to the number of entries in
   `reference.forge.fixtures.json`.
6. Run `cargo test --test bench_corpus_tests`.
