# The FORGE Card

> One page of FORGE for agents: the syntax that trips you up, one idiom per primitive, and the errors you will actually hit. `docs/forge-reference.md` is the full authority; `grammar/forge.pest` wins any disagreement.

## 1. What FORGE is

FORGE is a language for oracle-augmented computation: LLM calls are primitives, uncertainty is a compile-time type, and independent work runs in parallel. Deterministic logic lives in `pure`, stochastic logic in `task`, and agents are first-class with memory, lifecycle states, events, and supervision.

Authoring loop, in this order: `forge check app.forge` (parse + resolve + check — fix every error first) → `FORGE_MOCK=1 forge run app.forge` (the whole program on the mock provider: no API key, no tokens, no cost) → `forge run app.forge` (only once the mock run is clean). A program driving FORGE should prefer `forge check --json app.forge`: one JSON envelope on stdout, diagnostics with `code`/`line`/`col`, and semantic exit codes (`0` clean, `1` errors, `2` warnings only) — see `docs/forge-reference.md` §27.
New project: `forge init <dir> --template pipeline|agent|webhook-bot`.

## 2. Syntax rules that trip agents

- **Exact 2-space indentation.** Levels are 2 / 4 / 6 / 8 spaces (i1–i4) and nesting stops at i4; 3 spaces or a tab is a parse error, and a block nested inside `fn main` (whose own body is i1) still puts its body at i3, `else` at i2 — keep `fn main` thin and put branching in a `task` or `pure`.
- **No braces, no semicolons, no parentheses around conditions.** Newlines separate statements.
- **No blank lines inside a body.** A blank line ends the block, and the indented line after it fails to parse.
- **`needs` / `gives` / `do`.** `needs name: Type` (the type is required), `gives Type`, body inside `do` at i1 → i2. `fn main` has none of them: its body sits at i1.
- **Template strings.** `"{x}"` interpolates at parse time; `\{` and `\}` are literal braces; `\n`, `\t`, `\"`, `\\` are the other escapes.
- **`when` takes an arrow, not a block.** `when x.sure -> give x` is one line, with `else -> ...` as a sibling; only `if`, `for`, and `match` take an indented body.

## 3. The determinism boundary

`task` may call oracles and perform effects; `pure` may not — it is deterministic and always returns confidence 1.0. Forbidden in `pure`: `reason`, `classify`, `search`, `recall`, `exec`, `command`, `session`, `file.read`, `skill.*`, `escalate`, `try ... or`, and any call to a `task`.

Wrong — `reason` in a pure function; the fix is to move the oracle call into a `task` (§5 shows both):
```forge-error
pure summarize
  needs text: Text
  gives Text
  do
    give reason "summarize {text}"
```

## 4. Uncertainty: bind, then dispatch

`reason`, `classify`, `search`, `recall`, `exec`, `command`, `session`, `file.read`, `toml.parse`, `json.parse`, and every `skill.*` call return an uncertain value; taint survives assignment and field access. Bind the result to a name, then dispatch it with `when <name>.sure -> ...` / `.unsure` / `else`, or with `match`. Never `give` the raw result, and never inline an oracle in `give`: `give reason "..."` fails with "unhandled uncertain: oracle result given without confidence dispatch". Levels: `sure` (≥ 0.8), `unsure` (0.5–0.8), `unreliable` (< 0.5), `conflicted`; `when x.sure(above: 0.9)` overrides the threshold.

Wrong — `give` of a raw oracle result:
```forge-error
task analyze
  needs topic: Text
  gives Text
  do
    result = reason "analyze {topic}"
    give result
```
Right — every path acknowledges the confidence:
```forge
task analyze
  needs topic: Text
  gives Text
  do
    result = reason "analyze {topic}"
    when result.sure -> give result
    when result.unsure -> give "uncertain: {result}"
    else -> give "unknown"
```

## 5. One idiom per primitive

**`pure` / `task` / `when`**
```forge
pure slugify
  needs title: Text
  gives Text
  do
    give "post-{title}"

task summarize
  needs topic: Text
  gives Text
  do
    draft = reason "One sentence about {topic}"
    when draft.sure -> give slugify(draft)
    when draft.unsure -> give "uncertain: {draft}"
    else -> give "unknown"
```
**`flow`** — stages without `needs` run in the same wave, in parallel:
```forge
flow brief
  needs topic: Text
  gives Text
  stage gather_web
    notes = reason "Summarize {topic}"
  stage gather_papers
    notes = reason "Cite papers on {topic}"
  stage synthesize
    needs gather_web.notes, gather_papers.notes
    merged = reason "Combine: {gather_web.notes} {gather_papers.notes}"
    when merged.sure -> give merged
    else -> give "synthesis failed"
```
**`match`** — arms are tag patterns: `Buy` matches the `"Buy"` label returned by `classify`, `_` catches everything else
```forge
task route
  needs message: Text
  gives Text
  do
    result = classify message into ["Buy", "Support", "Other"]
    match result
      Buy -> give "sales"
      Support -> give "helpdesk"
      _ -> give "triage"
```
**`if` / `else if` / `for`**
```forge
pure rate
  needs scores: Number[]
  gives Text
  do
    total = 0
    for score in scores
      total = total + score
    if total > 10
      give "high"
    else if total > 5
      give "medium"
    else
      give "low"
```
**`event` / `subscribe` / `emit`** — never emit the event you subscribe to; declare a second event for the reply
```forge
event Mention
  thread: Text

event Replied
  thread: Text

agent listener
  subscribe Mention where thread == "main"
  on Mention(thread: Text)
    emit Replied(thread: thread)
    say "echo {thread}"
```
**`states` / `agent` / `memory` / `requires` / `transition`**
```forge
states Phase
  waiting -> active
  active -> waiting

agent responder
  lifecycle: Phase
  memory
    thread: Text
  on Mention(thread: Text)
    requires lifecycle == waiting on fail: give "busy"
    memory.thread = thread
    transition to active
```
**`pool`**
```forge
task FactChecker
  needs claim: Text
  gives Text
  do
    verdict = reason "Is this true? {claim}"
    when verdict.sure -> give verdict
    else -> give "unverified"

pool checkers
  workers: FactChecker * 3
  strategy: majority
  timeout: 15s

fn main
  say checkers.send("check", "the sky is blue")
```
**`warden`** — cover all six failure types; `manages` must name a declared agent/pool/flow:
```forge
agent bot
  on start
    say "ready"

warden supervisor
  manages [bot]
  on stuck: nudge, self
  on crash: restart, all
  on hallucination: replace, downstream
  on contradiction: escalate, self
  on budget: downgrade, self
  on timeout: restart, self
```
**`contract` / `system`**
```forge
contract GameRoom
  can join(player: Text) -> Text
agent room_agent
  on start
    say "room ready"
agent lobby_agent
  on start
    say "lobby ready"
system lobby
  use
    game: room_agent
    front: lobby_agent
  game >> front
```
**`command`** — `["argv", "array"]` beats a shell string when interpolating values; always branch on `result.success` before using the output — a failed command must never be reinterpreted (E150)
```forge
task run_tests
  gives Text
  do
    result = command ["cargo", "test"] in "." timeout 10m
    if result.success
      give result.stdout
    else
      give result.stderr
```
**`session`** — external agent sessions; handle the result like any oracle:
```forge
task review_patch
  gives AgentResult
  do
    result = session "code-review" agent "claude" prompt "Review this patch" timeout 5m gives AgentResult
    when result.sure -> give result
    else -> give AgentResult(plan: "failed", confidence: 0.0)
```
**`knowledge` / `learn` / `recall`**
```forge
agent librarian
  knowledge store: ".forge-knowledge/librarian"
    max_entries: 5000
  on ingest(fact: Text)
    learn "{fact}" category: "FACTS"
  on ask(question: Text)
    prior = recall "{question}"
    when prior.sure -> give prior
    else -> escalate to human
```
**`spawn` / `find` / `retire`** — a spawned agent needs a failure policy or `forge check` warns:
```forge
agent specialist
  on start
    say "specialist up"
  if stuck for 3 turns
    escalate to human

agent foreman
  on dispatch(topic: Text)
    child = spawn specialist as "spec_{topic}"
    say "spawned {child}"
  on cleanup(topic: Text)
    existing = find "spec_{topic}"
    retire "spec_{topic}"
```
**`skill.*`** — resolves only through a project manifest, so validate it with `forge check --manifest forge.project.toml`, not a bare `forge check` (the block below is not extracted by CI):
```toml
[skills]
repo_check = {}
```
```text
use
  skill.repo_check

fn main
  report = skill.repo_check.analyze()
  say report
```
**`file.read` / `json.parse`** — `toml.parse` has the same shape; `file.read` is server-only:
```forge
#! boundary: server

use
  file.read
  json.parse
type Host
  name: Text
  port: Number
task load_host
  needs path: Text
  gives Text
  do
    host = json.parse(file.read(path), "Host")
    when host.sure -> give "{host.name}:{host.port}"
    else -> give "unreadable config"
```

## 6. Common errors

- `expected statement` — bad indentation (3 spaces, a tab) or `when` without `->`; use exact 2-space levels and one-line `when x.sure -> ...`.
- `expected eoi, top level` — a blank line inside `do`, a handler, or `fn main`; delete the blank line.
- ``unhandled uncertain: ...`` — an oracle result reached `give` raw or inline; bind it, then dispatch with `when x.sure` / `.unsure` / `else`.
- ``command result `x` used before checking `x.success` `` — an unchecked `command`/`exec` result reached `give`, `emit`, or a `reason`/`classify` prompt; branch on `x.success` (or `x.exit_code`) first.
- ``pure function `<f>` cannot use `<op>` `` / ``cannot call task `<t>` `` — an oracle, effect, or `task` call inside `pure`; move that line into a `task` (`pure` may only call `pure`).
- ``illegal transition from `<a>` to `<b>` `` — add that edge to the `states` block, or fix the state name.
- ``unguarded transition to `<s>` in handler `<h>` `` — add `requires lifecycle == <from>` as the handler's first line.
- ``unknown capability `<name>` `` — the `use` list names something that is not built in and not in the project manifest; fix the name.
- ``call to undeclared function `<f>` `` — `f` is not a declared `task`/`pure`/`flow`/`pool` (`asset` and `winning_lines` are the only builtin calls, `main` is not callable, and uppercase names are type constructors); declare it, fix the name, or — if `f` lives in another file of the project — check the files together with `forge check --merge <files>` or `forge check --manifest forge.project.toml`.
- ``pattern `<P>` never matches ...`` — the arm is dead code: `P` is not a declared or builtin type and not in the scrutinee's known values (`classify` labels, literal `give`s), or it differs only in case from a label — tags compare exact text, so `Positive` never matches `"positive"`.
- `file.read() is not allowed in shared boundary` — add `#! boundary: server` as line 1 (`search`, `data.*`, and `endpoint` are server-only too).

Run `forge explain <code>` for any error code (see #474).

## 7. Where to go next

- `docs/forge-reference.md` — the full language reference.
- `examples/` — runnable programs; `examples/errors/` fail on purpose.
- <https://github.com/ncmlabs/forge-examples> — the examples repo.
- `llms.txt` — the machine entry point, and it points back here.
