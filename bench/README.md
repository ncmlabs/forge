# FORGE authoring benchmark — spec corpus

Part 1 of [#483](https://github.com/ncmlabs/forge/issues/483) is the **corpus**:
the natural-language specs, a reference solution and an offline acceptance
fixture per spec. Part 2 is `bench/run.sh` — the harness that prompts a model,
checks the reply with `forge check --json`, repairs from the diagnostics, and
reports the table; see [Running the harness](#running-the-harness).

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

## Running the harness

`bench/run.sh` (bash + curl + jq) sends `docs/forge-card.md` as the system
prompt and `spec.md` plus one fixed instruction as the user turn — *reply with
one complete `.forge` program in a single fenced block* — extracts the first
fenced block, and checks it with `forge check --json`. Diagnostics mean not
clean: the harness repairs at most twice, each time sending the previous
program and the diagnostics array back unchanged, then grades the surviving
program with `forge test --expect` against the spec's fixture. Every spec is
reported, never a best-of.

| Variable | Meaning |
| -------- | ------- |
| `BENCH_BASE_URL` | OpenAI-compatible base URL, e.g. `https://host/v1`. Required unless `BENCH_FAKE` is set. |
| `BENCH_API_KEY` | Bearer token for that endpoint. Optional — a local vLLM needs none. |
| `BENCH_MODEL` | Provider model id sent in the request. Required unless `BENCH_FAKE` is set. |
| `BENCH_FAKE` | `reference` or `broken`: answer locally instead of calling the endpoint. |
| `FORGE_BIN` | The forge command, default `cargo run -q --`; CI uses `./target/release/forge`. |

```sh
BENCH_BASE_URL=https://host/v1 BENCH_API_KEY=... BENCH_MODEL=my-model \
  bench/run.sh --model my-model --specs 01,02,10 --out bench/results/2026-01-05-my-model.json
```

`--specs` takes two-digit ids and defaults to the whole corpus. The results file
is `{model, date, forge_rev, totals: {pass1_rate, pass3_rate, correct_rate,
tokens_in, tokens_out}, specs: [...]}`, and each spec carries `{spec,
difficulty, pass1, pass3, attempts, correct, note, reason, tokens_in,
tokens_out, final_codes, final_diagnostics}`:

- `pass1` / `pass3` — the first draft, and any of the three attempts, checked
  with zero diagnostics.
- `attempts` — 1 to 3. A spec still unclean after 3 escalates: `pass3` is false,
  `note` says so, and `final_diagnostics` holds the surviving array.
- `correct` and `reason` — the `forge test --expect` replay result and, when it
  fails, why: `replay miss: ...` for calls the fixture lacks, an output
  mismatch, or a runtime error.
- `tokens_in` / `tokens_out` — summed over the attempts the spec took, read from
  the endpoint's `usage` field.

One markdown table goes to stdout and everything else to stderr, so
`bench/run.sh ... >> "$GITHUB_STEP_SUMMARY"` appends a clean table. A reply with
no program — or an empty fenced block — is never a pass, and a spec that never
checks clean is still graded for correctness rather than hidden.

Offline, with no key and no cost:

```sh
bench/run.sh --self-test
```

It runs specs 01 and 02 against both fake models and fails unless the numbers
match: `BENCH_FAKE=reference` answers with the spec's own reference (pass@1 =
pass@3 = correct = 2/2, one attempt each) and `BENCH_FAKE=broken` answers with
the reference minus its first block-opening `do` (a parse error first, the
reference on repair: pass@1 = 0, pass@3 = 2/2, two attempts each).
`tests/bench_corpus_tests.rs` runs it as part of `cargo test`.

`.github/workflows/bench.yml` runs the whole corpus weekly (Mondays 05:00 UTC)
and on `workflow_dispatch` over a matrix of models, uploading each results JSON
as an artifact and appending its table to the run summary. The coordinator
commits the JSON under `bench/results/`.

## Layout

```
bench/
  README.md                  this file
  run.sh                     the model-running harness (part 2)
  mock.config.toml           a copy of config/mock.config.toml (the mock provider)
  results/                   one JSON per run (the coordinator commits them)
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
