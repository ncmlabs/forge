# FORGE

[![CI](https://github.com/ncmlabs/forge/actions/workflows/ci.yml/badge.svg)](https://github.com/ncmlabs/forge/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Status](https://img.shields.io/badge/status-Layer%201%20Active-brightgreen.svg)]()

**The programming language for oracle-augmented computation.**

FORGE treats LLM calls as oracle queries — not function calls — with their own type system, execution model, and failure semantics. Uncertainty is a compile-time type. Deterministic logic is structurally separated from stochastic logic. Agents are first-class citizens with lifecycle management, knowledge systems, and supervision trees.

---

## The problem we're solving

Every team building agent systems today is solving the same five problems independently, from scratch, in languages that don't know LLMs exist:

**Uncertainty is invisible.** An LLM returns a string. The language treats it identically to the result of `1 + 1`. There is no structural difference between a confident answer and a hallucinated one. The hallucination propagates downstream, silently, until it causes damage.

**Cost is a surprise.** Token usage is tracked by billing dashboards, not by the programs that generate it. There is no way to estimate the cost of a pipeline before running it. There is no budget enforcement at the language level. Teams discover they've spent their monthly budget on a single bad loop at 3am.

**Parallelism is manual work.** Steps that could run simultaneously run sequentially because making them parallel requires async code, thread management, and careful dependency tracking. The parallelism is always the programmer's problem.

**Failure recovery is bolted on.** When an agent crashes or returns garbage, the recovery logic is written in the same imperative style as the happy path — nested try-catch blocks, manual retry loops, ad-hoc fallback logic scattered across the codebase. The failure architecture is an afterthought.

**Deterministic and stochastic logic mix.** Game rules, compliance checks, and dosage calculations sit in the same language level as AI reasoning. The compiler cannot distinguish between "this must be correct" and "this might be uncertain." The result: hallucinations reach places they must never reach.

These are not hard problems. They are problems that exist because every existing language was designed before LLMs existed. They treat LLM calls as function calls. They are not.

---

## Quick start

```bash
# Build from source
git clone https://github.com/ncmlabs/forge.git
cd forge
cargo build --release

# Validate a FORGE program
cargo run -- check examples/hello.forge

# Run with configured LLM provider
cargo run -- run examples/hello.forge

# Build a standalone agent binary
cargo run -- build workflows/forge-sensei.forge -o bin/forge-sensei

# Run all tests (no API calls — uses mock provider)
cargo test
```

---

## What FORGE is

FORGE is built on a foundational insight: **an LLM call is not a function call. It is an oracle query.**

A function call is deterministic. Same input, same output, every time. An oracle query is probabilistic. Same input, different output each time, drawn from a distribution the caller cannot inspect. These are different operations. A language that treats them identically is lying about the nature of computation.

FORGE is the first language that treats oracle calls as the first-class primitives they are — with their own syntax, their own type system, their own execution model, and their own failure semantics.

```
  Traditional Code                    FORGE
  ─────────────────                   ─────────────────
  result = llm.chat(prompt)           result = reason prompt
  # result is a string                # result is uncertain<T>
  # confidence? unknown               # confidence: tracked
  # hallucination? invisible          # hallucination: type error
  # cost? check dashboard later       # cost: forge cost <file>
  # failure? try/catch somewhere      # failure: declared policy
  return result                       when result.sure -> give result
                                      else -> escalate to human
```

The result is a language where:
- Uncertainty is tracked at compile time, not discovered at runtime
- Deterministic logic is structurally separated from stochastic logic
- Parallelism is inferred by the compiler, not written by the programmer
- Failure recovery is declared, not coded
- Every decision is traced automatically
- The same program runs on any LLM provider by changing a config line
- Agent fleets build systems from specifications, producing native binaries

---

## The language at a glance

```forge
# Declare what capabilities you need — not which provider
use
  llm.reason
  llm.classify
  web.search

# A task with confidence-aware output
task classify_intent
  needs message: Text
  gives Intent

  do
    result = classify message into ["buy", "support", "cancel", "other"]

    when result.sure(above: 0.85)  -> give result
    when result.sure               -> give result with flag("low-confidence")
    when result.unsure             -> give ask_for_clarification(message)
    else                           -> escalate to human

# Deterministic logic — compiler enforces: no oracle calls allowed here
pure check_winner
  needs board: Text[9]
  gives WinResult
  do
    for line in WIN_LINES
      sym = board[line[0]]
      if sym != "" and sym == board[line[1]] and sym == board[line[2]]
        give Winner(sym)
    give Ongoing

# A multi-stage pipeline — parallel stages inferred automatically
flow research
  needs topic: Text
  gives Report

  stage gather
    web    = search topic
    papers = search "{topic} research"
    news   = search "{topic} news"
    # compiler sees no dependencies between these — runs all three in parallel

  stage synthesize
    needs gather.*
    draft = reason "synthesize: {gather.*}"

  stage verify
    needs synthesize.draft
    give reason "fact-check: {synthesize.draft}"

# A stateful agent with lifecycle enforcement
agent RoomAgent
  lifecycle: RoomLifecycle    # illegal state transitions = compile error

  memory
    board:   Text[9]
    players: Player[]
    turn:    Number

  timer turn_limit: 15s       # fires on_turn_limit_expired automatically

  on move(player: Player, cell: Number)
    requires lifecycle == playing              on fail: silent
    requires valid_move(memory.board, cell)   on fail: give InvalidCell

    memory.board[cell] = player.symbol
    emit MoveEvent(memory.id, player, cell)
    reset turn_limit

  on_hallucination: restart
  on_timeout:       forfeit(idle_player)
```

---

## How it works

### The compiler pipeline

```
                          ┌─────────────────┐
                          │  source.forge    │
                          └────────┬────────┘
                                   │ parse
                                   ▼
                          ┌─────────────────┐
                          │    AST (typed,   │
                          │    spanned)      │
                          └────────┬────────┘
                                   │ check
                    ┌──────────────┼──────────────┐
                    ▼              ▼               ▼
             ┌───────────┐ ┌───────────┐  ┌────────────┐
             │   pure     │ │  states   │  │  boundary  │
             │  checker   │ │  checker  │  │  checker   │
             └───────────┘ └───────────┘  └────────────┘
                    ▼              ▼               ▼
             ┌───────────┐ ┌───────────┐  ┌────────────┐
             │ uncertain  │ │  requires │  │   spawn    │
             │  checker   │ │  checker  │  │  checker   │
             └───────────┘ └───────────┘  └────────────┘
                    │              │          │
                    └──────────────┼──────────┘
                                   │    ┌────────────┐
                                   ├───▶│  warden    │
                                   │    │  checker   │
                                   │    └────────────┘
                                   ▼
                    ┌──────────────────────────┐
                    │     Runtime Executor      │
                    │  ┌──────┐ ┌──────┐       │
                    │  │agents│ │flows │ ...    │
                    │  └──────┘ └──────┘       │
                    └─────────────┬────────────┘
                                  │
                    ┌─────────────┼─────────────┐
                    ▼             ▼              ▼
              ┌──────────┐ ┌──────────┐  ┌───────────┐
              │forge run │ │forge     │  │forge build│
              │ (interpret│ │ serve   │  │ (binary)  │
              └──────────┘ └──────────┘  └───────────┘
```

Seven semantic checkers catch errors at compile time — before any LLM call is made, before any token is spent.

### The confidence flow

The core of FORGE: every oracle call returns `uncertain<T>`. You must handle it.

```
  reason "analyze this document"
         │
         ▼
  ┌──────────────────────────┐
  │    uncertain<T>           │
  │    confidence: 0.0 - 1.0  │
  └──────────┬───────────────┘
             │
    ┌────────┼─────────┬──────────────┐
    ▼        ▼         ▼              ▼
  ┌──────┐ ┌──────┐ ┌──────────┐ ┌────────┐
  │ .sure │ │.unsure│ │.unreliable│ │  else  │
  │ ≥0.8  │ │ ≥0.5 │ │   <0.5   │ │        │
  └──┬───┘ └──┬───┘ └─────┬────┘ └───┬────┘
     │        │            │          │
     ▼        ▼            ▼          ▼
   act     ask for      discard    escalate
            more info               to human
```

No `force_unwrap`. No implicit promotion. You cannot pretend to know what you don't know.

### Flow parallelism

The compiler analyzes stage dependencies and builds a DAG. Independent stages run in parallel automatically.

```
  flow research
    stage gather          stage synthesize       stage verify
    ┌───────────────┐     ┌───────────────┐     ┌──────────────┐
    │ web = search  │     │               │     │              │
    │ papers = search├────▶│ draft = reason├────▶│ give reason  │
    │ news = search │     │ "synthesize"  │     │ "fact-check" │
    └───────────────┘     └───────────────┘     └──────────────┘

    Wave 1 (parallel)      Wave 2 (sequential)   Wave 3 (sequential)
    ┌─────┐ ┌──────┐      ┌──────────────┐      ┌──────────────┐
    │ web │ │papers│      │  synthesize   │      │    verify     │
    │     │ │      │      │              │      │              │
    │     │ │      │      └──────────────┘      └──────────────┘
    │     │ │      │
    │     │ └──────┘
    │     │ ┌─────┐
    │     │ │news │
    └─────┘ └─────┘
      3 calls parallel       1 call                1 call
      = 1x latency           = 1x latency          = 1x latency
```

You write stages and declare dependencies. The compiler figures out the parallelism.

### The agent lifecycle

```
  ┌─────────┐    spawn specialist as "syntax_expert"
  │  Sensei  │    with knowledge where category == "SYNTAX"
  │  Agent   │──────────────────────────────────────────────┐
  └────┬─────┘                                              │
       │ find "syntax_expert"                               ▼
       │◄──────────────────────────────── ┌──────────────────────┐
       │                                  │  Specialist Agent     │
       │ emit LearnedInsight ────────────▶│  ┌────────────────┐  │
       │                                  │  │ knowledge store │  │
       │                                  │  │ learn + recall  │  │
       │                                  │  └────────────────┘  │
       │                                  │  subscribe            │
       │                                  │   LearnedInsight      │
       │                                  └──────────┬───────────┘
       │                                             │
       │             ┌─────────────┐                 │ retire
       │◄────────────│   Warden    │◄────────────────┘ with knowledge
       │  supervises │  on stuck   │  monitors         export
       │             │  on crash   │
       │             │  on timeout │
       │             └─────────────┘
```

Agents are born (`spawn`), learn (`learn`/`recall`), discover each other (`find`), communicate (`emit`/`subscribe`), get supervised (`warden`), and gracefully exit (`retire`) — preserving their knowledge.

### System orchestration

```
  ┌─────────────────────────────────────────────────────┐
  │  system customer_support                             │
  │                                                      │
  │  ┌──────────┐    events    ┌──────────────────┐     │
  │  │ triage   │─────────────▶│ tech_specialist   │     │
  │  │ agent    │              └──────────────────┘     │
  │  │          │    events    ┌──────────────────┐     │
  │  │          │─────────────▶│ billing_specialist│     │
  │  └──────────┘              └──────────────────┘     │
  │       │                           │                  │
  │       └───────────┬───────────────┘                  │
  │                   ▼                                  │
  │          ┌─────────────────┐                         │
  │          │   Shared Event   │                         │
  │          │      Bus         │                         │
  │          └─────────────────┘                         │
  │                   │                                  │
  │          ┌────────┴────────┐                         │
  │          ▼                 ▼                          │
  │  ┌──────────────┐ ┌───────────────┐                  │
  │  │ Instance     │ │ Knowledge     │                  │
  │  │ Registry     │ │ Stores        │                  │
  │  │ (find/spawn) │ │ (learn/recall)│                  │
  │  └──────────────┘ └───────────────┘                  │
  └─────────────────────────────────────────────────────┘
```

A `system` declaration wires agents together with a shared event bus, instance registry, and knowledge infrastructure. Wardens supervise the fleet.

---

## The agent ecosystem

Agents in FORGE are born, learn, specialize, discover each other, and gracefully retire — all as language primitives.

```forge
# A specialist agent that learns within a domain
agent specialist
  memory
    topic: Text
    query_count: Number
  knowledge store: ".forge-knowledge/specialist"
    max_entries: 10000
    retention: 180d
  subscribe LearnedInsight where category == memory.topic

  on query(question: Text)
    memory.query_count = memory.query_count + 1
    prior = recall "{memory.topic} {question}"
    answer = answer_forge_question(question, prior)
    learn from interaction(question, answer, 0.6) category: "{memory.topic}"
    give answer

# The sensei spawns specialists on demand
exportable agent forge_sensei
  # ...
  on deep_dive(topic: Text)
    existing = find "specialist_{topic}"
    if existing
      give existing
    child = spawn specialist as "specialist_{topic}"
      with knowledge where category == topic
      with confidence_cap: 0.8
      with memory topic: topic
    give child

# Supervision — the warden watches everything
warden sensei_warden
  manages [forge_sensei, specialist]
  on stuck: nudge, self
    after 3: escalate
  on hallucination: restart, self
  on crash: restart, self
  max_retries 3 per 1h then escalate
```

This is not a toy example — it is the actual `forge-sensei` program that teaches the FORGE language, compiled to a standalone binary at `bin/forge-sensei`.

---

## See it in action — the FORGE Wiki

The [`examples/wiki/`](examples/wiki/) directory contains a complete documentation wiki built in FORGE — ~580 lines that exercise all 14 language primitives in a real, working application. Browse docs, search with LLM-powered confidence gating, ask questions, and auto-generate verified reference documentation.

What the wiki demonstrates:

- **Agents** with persistent memory and typed state machines (`content_manager`, `search_agent`, `qa_agent`)
- **Flows** with parallel DAG execution (3 LLM extractions in parallel, then generation, then fact-checking)
- **Pools** with majority-vote verification (3 independent checkers per claim)
- **Warden** supervision with 5 failure policies and escalation chains
- **Events** for reactive cross-agent communication (content changes trigger search re-indexing)
- **Confidence gating** from LLM output through to color-coded UI badges
- **Pure** rendering functions enforcing the determinism boundary
- **System** composition wiring agents with `>>`

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
forge serve examples/wiki/server.forge -s examples/wiki/shared.forge --watch
# Open http://127.0.0.1:3000/home
```

See the [Wiki README](examples/wiki/README.md) for the full guide and [Architecture](examples/wiki/ARCHITECTURE.md) for system diagrams and the complete feature map.

---

## The nine principles

Every feature in FORGE traces to one of nine principles. Every design decision is a refusal or an affirmation of these beliefs.

```
  ┌─────────────────────────────────────────────────────────────────┐
  │                    THE NINE PRINCIPLES                          │
  │                                                                 │
  │  SAFETY                    ARCHITECTURE          OPERATIONS     │
  │  ┌───────────────────┐     ┌──────────────┐     ┌───────────┐  │
  │  │ I   Honesty       │     │ IV  Compose  │     │ VII Human  │  │
  │  │ II  Determinism   │     │ V   Supervise│     │     Ceiling│  │
  │  │ III Token Economy │     │ VI  Self-Ref │     │VIII Account│  │
  │  │ IX  Boundary      │     │              │     │     ability│  │
  │  └───────────────────┘     └──────────────┘     └───────────┘  │
  │         │                        │                     │        │
  │         ▼                        ▼                     ▼        │
  │  "What can go wrong?"    "How does it fit?"    "Who's watching?"│
  └─────────────────────────────────────────────────────────────────┘
```

### I — The Honesty Principle
*A system that hides uncertainty is more dangerous than one that knows nothing.*

Every oracle call returns `uncertain<T>`. The compiler enforces that `uncertain<T>` cannot be used as `T` without handling the uncertainty first. There is no `force_unwrap`. There is no implicit confidence promotion. You cannot pretend to know what you don't know.

### II — The Determinism Boundary
*Two kinds of computation exist. They must never be mixed invisibly.*

```
  ┌──────────────────────────┐    ┌──────────────────────────┐
  │   DETERMINISTIC ZONE     │    │    STOCHASTIC ZONE       │
  │   (pure functions)       │    │    (tasks with LLM)      │
  │                          │    │                          │
  │   game rules             │    │   intent classification  │
  │   compliance checks      │    │   summarization          │
  │   dosage calculations    │    │   code generation        │
  │   validation logic       │    │   reasoning              │
  │                          │    │                          │
  │   confidence: always 1.0 │    │   confidence: 0.0 - 1.0  │
  │   hallucination: IMPOSSIBLE│   │   hallucination: possible │
  │                          │    │                          │
  │   pure check_winner      │    │   task classify_intent   │
  │     gives WinResult  ◄───┼─NO─┼──  reason "..."         │
  │                          │    │                          │
  └──────────────────────────┘    └──────────────────────────┘
         compile error if pure calls an oracle ───▶ ✕
```

`pure` functions cannot call oracles. `think` tasks cannot be guaranteed deterministic. This boundary is enforced at compile time. The deterministic core of any FORGE system — rules, constraints, validations — is provably free of hallucination.

### III — The Token Economy
*Token is the fundamental unit of oracle computation.*

The compiler is a token optimizer. `forge cost` is a first-class command. Budget limits are language-level constructs. Context compaction policies are primitives. Sending 10,000 tokens where 1,000 suffice is a categorical architectural error, not a performance issue.

### IV — The Composition Completeness
*Any primitive that cannot compose with any other primitive is not a primitive.*

The `>>` operator connects everything. `ConfidentValue` is the universal interface. A new primitive added to FORGE must pass one test: can it appear on both sides of `>>`? If not, it belongs in a library, not the language.

### V — The Supervision Principle
*Write the happy path. Declare failure policy. Let the supervisor handle the rest.*

Agents declare their failure policies — what happens on hallucination, timeout, cost overrun. The supervision tree enforces them. Crash recovery is a declaration, not code. A FORGE agent with no failure declarations is architecturally incomplete.

### VI — The Self-Reference Principle
*A language built for agents must be writable by agents.*

`forge_check(code)` and `forge_run(code)` are first-class tools available inside FORGE programs. The conformance suite is not documentation — it is how the oracle learns to speak FORGE. Every error message is structured data designed for an agent's repair loop, not just for a human's eyes.

### VII — The Human Ceiling
*The most valuable FORGE agents know when to stop.*

`escalate to human` is as simple to write as any other path. The `uncertain<T>` type has a `hallucinated` variant that forces escalation. A FORGE system that never escalates is not more capable — it is less trustworthy. The language makes the safe path the path of least resistance.

### VIII — The Accountability Principle
*Every decision must be traceable to its cause.*

Observability is compiled in, not bolted on. Every `think` call, every `transition`, every `emit`, every `requires` guard produces a structured trace event automatically. The trace exists by default, always — not optionally, not with a flag.

### IX — The Boundary Principle
*Code that must be correct must be separated from code that might be wrong.*

The `boundary` primitive enforces code partition at the compiler level. Server code cannot leak into client bundles. Values from external sources are typed `untrusted<T>` and cannot flow into privileged operations without sanitization. This is prompt injection prevention at the language level.

---

## The architecture — four layers

FORGE is designed as a self-building system. Each layer uses the layer below to construct itself.

```
┌─────────────────────────────────────────────────────┐
│  LAYER 4 — SELF-IMPROVEMENT                          │
│  Agents watch running systems, identify bottlenecks, │
│  rewrite slow modules, shadow-deploy improvements    │
├─────────────────────────────────────────────────────┤
│  LAYER 3 — AUTOMATION FACTORY                       │
│  Specification in → running system out              │
│  No human writes FORGE code                         │
├─────────────────────────────────────────────────────┤
│  LAYER 2 — TOOLKIT AGENTS                           │
│  Agents that generate FORGE code from descriptions  │
│  TaskGenerator, FlowGenerator, AgentGenerator       │
├─────────────────────────────────────────────────────┤
│  LAYER 1 — THE SUBSTRATE                            │
│  The language itself. Written by humans once.       │
│  Everything above is built on top of this.          │
└─────────────────────────────────────────────────────┘
```

Layer 1 is the only layer humans build from scratch. Everything above it is built by agents using FORGE itself.

---

## The build system

FORGE programs compile to standalone native binaries. Each agent becomes a CLI with subcommands for its handlers and an interactive REPL.

```bash
# Compile an agent to a binary
forge build workflows/forge-sensei.forge -o bin/forge-sensei

# The binary has subcommands for each handler
bin/forge-sensei query "how do flows work?"
bin/forge-sensei review "pure bad_fn\n  do\n    reason 'hello'"
bin/forge-sensei status
bin/forge-sensei repl    # interactive session
```

Multi-file projects use a manifest:

```toml
# forge.project.toml
[project]
name = "my-system"
version = "0.1.0"

[sources]
main = "src/main.forge"
agents = "src/agents/*.forge"
```

```bash
forge build --manifest forge.project.toml -o bin/my-system
```

**Future targets:** WASM compilation via Cranelift is planned — the same binary running on server, browser, and edge. See the [roadmap](roadmap.md) for details.

---

## Provider independence

FORGE code never names a provider. Switching from one LLM to another is a single config line.

```toml
# forge.config.toml

[llm]
default = "primary"

[providers.primary]
type    = "anthropic"
model   = "claude-haiku-4-5-20251001"
api_key = "${ANTHROPIC_API_KEY}"
fallback = "local"

[providers.local]
type     = "openai-compat"    # covers any OpenAI-compatible endpoint
model    = "llama3.2"
base_url = "http://localhost:11434/v1"
api_key  = "not-required"
```

The same config pattern covers: any major cloud provider, locally hosted models via standard runtimes, self-hosted GPU servers, fast inference APIs, and future providers that haven't been built yet — because the OpenAI-compatible API has become the universal standard.

Capability hints let the runtime route automatically:

```forge
task analyze_contract
  needs doc: Text
  do
    reason "analyze: {doc}" with
      quality: high        # route to best available model
      local_only: true     # never leave the machine (privacy-sensitive)
```

---

## What's built — and what's next

### v0.1.0 — Layer 1 complete (Phase 1 shipped)

```
✅ task + reason + when         — oracle reasoning with uncertainty handling
✅ pure                         — deterministic logic, provably hallucination-free
✅ flow + stage + needs          — parallel pipelines, automatic DAG inference
✅ agent + states + requires     — stateful systems with lifecycle enforcement
✅ event + timer                 — broadcast coordination and time-aware behavior
✅ boundary                      — server/client separation, prompt injection prevention
✅ >> composition                — universal wiring between all primitives
✅ spawn + find + retire         — agent lifecycle: birth, discovery, graceful shutdown
✅ learn + recall + knowledge    — progressive learning with categorized knowledge stores
✅ warden supervision            — crash recovery, stuck detection, escalation policies
✅ system orchestration          — multi-agent wiring with shared event bus
✅ forge build                   — standalone native binaries with CLI + REPL
✅ Web runtime                   — HTML templates, static serving, hot-reload, markdown
✅ HTTP client + webhooks        — web.fetch, web.post, HMAC-verified webhooks
✅ exec + skill bridge           — CLI execution, SKILL.md ecosystem, tool-use
✅ data operations               — persistent KV storage (redb), vector embeddings, semantic search
✅ Observer                      — live SSE tracing, introspection API, D3 topology, cost dashboard
✅ Wiki showcase                 — documentation wiki using all 14 primitives (52 tests)
✅ Sentinel                      — AI-powered repo health dashboard (17 primitives)
✅ forge-sensei                  — self-referential learning agent (FORGE teaching FORGE)
```

### Coming next

```
⬜ WASM compilation              — Cranelift backend, browser target (v0.2.0)
```

### Future layers

```
⬜ Layer 2 — Toolkit agents      — agents that generate FORGE code from descriptions
⬜ Layer 3 — Automation factory   — spec in → running system out
⬜ Layer 4 — Self-improvement     — factory watches and optimizes itself
```

When Layer 1 is complete, Layer 2 becomes possible. When Layer 2 exists, the factory runs. When the factory runs, the system builds itself.

---

## Deployment targets

**Today:** FORGE programs compile to native binaries via `forge build`. Each agent becomes a standalone CLI with handler subcommands and interactive REPL.

**Planned:** WASM compilation via Cranelift will enable three deployment environments from a single source:

| Target | Runtime | Use case |
|--------|---------|----------|
| `--target native` | OS directly | CLI tools, servers, desktop apps |
| `--target wasm32-wasi` | wasmtime / wasmer | Cloud functions, edge, embedded |
| `--target wasm32-browser` | Browser WASM | Web applications |

The FORGE source code does not change between targets. One compiler flag selects the deployment environment.

---

## The type system

FORGE's type system extends beyond structure to include epistemic state.

| Type | Meaning |
|---|---|
| `uncertain<T>` | Value from an oracle call — must be handled before use |
| `pure T` | Value from deterministic computation — confidence always 1.0 |
| `restricted<T>` | Sensitive value — compiler prevents logging, leaking |
| `untrusted<T>` | Value from external source — cannot reach privileged operations |
| `grounded<T>` | Value with mandatory source citation — hallucination guard |
| `classified<T, Labels>` | Classification with bounded output set |

The type checker enforces:
- `uncertain<T>` cannot be used as `T` without a `when` handler
- `restricted<T>` cannot flow into `log()`, `say()`, or network operations
- `untrusted<T>` cannot flow into `boundary server` operations without sanitization
- `pure` functions cannot contain `think`, `reason`, or any tool call
- State transitions that aren't declared in a `states` block are compile errors

---

## The trace format

Every FORGE execution produces structured traces automatically. No instrumentation required.

```json
{
  "step": "research.synthesize.reason",
  "agent": "ResearchAgent#042",
  "tokens_in": 2847,
  "tokens_out": 312,
  "latency_ms": 1240,
  "cost_usd": 0.0039,
  "confidence": 0.87,
  "variant": "certain",
  "when_branch": "sure",
  "provider": "primary",
  "model": "claude-haiku-4-5-20251001",
  "caused_by": "flow.research",
  "parallel_with": ["research.gather.web", "research.gather.papers"],
  "timestamp": "2026-04-01T12:00:00Z"
}
```

The trace answers: what was decided, by which agent, with what confidence, at what cost, in what time, caused by what, running alongside what. This is the minimum information needed to audit any FORGE decision — including decisions that were wrong.

---

## The domains where FORGE changes outcomes

FORGE is most valuable in systems where:
- Wrong AI answers have real consequences
- Confidence level determines whether to act or escalate
- Deterministic rules must coexist with AI reasoning
- Multiple agents coordinate on long-horizon tasks
- Cost at scale is a real constraint
- Decisions must be auditable

**Healthcare.** Clinical decision support where a low-confidence diagnosis must route to physician review, not automatic treatment. Drug interaction checking that is provably deterministic. Patient record processing that never logs PII.

**Financial services.** Credit decisions where confidence below threshold triggers human review. Fraud detection where the deterministic rules run as compiled native code and the AI handles the edge cases. Compliance checking where every decision carries an audit trail.

**Call center operations.** Campaign routing where confidence determines automatic vs manual handling. Quality scoring that accumulates evidence before rendering judgment. Compliance auditing that is `pure` and cannot hallucinate.

**Scientific research.** Hypothesis-driven discovery loops where AI proposes experiments and deterministic simulators evaluate them. The research pipeline runs continuously at scale.

**Software development.** Feature generation from specifications. Agents that write, verify, and deploy code. Systems that maintain themselves by identifying bottlenecks and rewriting slow modules.

---

## The impact model

**Immediate:** Teams building agent systems get structural guarantees they currently lack. Hallucination propagation becomes a compile error. Parallelism becomes automatic. Provider switching becomes a config change. Systems that currently fail silently in production fail loudly at compile time instead.

**Two years:** The factory model means specifications become the unit of software delivery. An organization describes what it needs. The factory builds it. Engineers review the specification and the deployed behavior — not the code. The bottleneck shifts from "how fast can we write code" to "how clearly can we specify intent."

**Five years:** The primitives migrate. The ideas that make FORGE coherent — uncertainty types, determinism boundaries, supervision trees for agents, token-aware compilation — appear in mainstream frameworks regardless of whether FORGE achieves mass adoption. The vocabulary becomes standard.

**The ceiling:** Consequential systems — the ones making decisions about healthcare, credit, justice, safety — gain a structural property they currently lack: the inability to silently propagate a confident hallucination into an irreversible action. This is not a policy improvement. It is an architectural guarantee. It changes what responsible AI deployment looks like at the foundation.

---

## The honest constraints

Three things determine whether this reaches its potential:

**Engineering:** Layer 1 shipped as v0.1.0 — parser (609 PEG rules), seven semantic checkers, async runtime with 15 modules, agent lifecycle (spawn/find/retire), knowledge system, system orchestration, web runtime, observer, and 883 tests are operational. Layer 2 (toolkit agents that generate FORGE code) is the next frontier. Error messages are structured data designed for agent repair loops.

**Capability:** The factory model depends on LLMs that can write reliable FORGE code. Today's models can handle simple programs. Complex multi-agent systems with subtle invariants require the repair loop to run multiple times. The factory becomes more powerful as models improve.

**Timing:** The AI agent market is at exactly the moment where the right foundation matters most. The 40% project cancellation rate is a problem waiting for a structural solution. FORGE is that solution — but only if it reaches teams before they've already committed to architectures that can't be revised.

---

## Document index

| Document | Contents |
|---|---|
| `forge-principles.md` | The nine principles — what FORGE believes and why |
| `docs/forge-reference.md` | Complete language reference — syntax, semantics, and compiler enforcement |
| `roadmap.md` | Architecture, milestones, track progress, and layer model |
| `CHANGELOG.md` | All notable changes in Keep a Changelog format |
| `providers.md` | Provider abstraction: trait, registry, implementations, config |
| `examples/` | 30 example programs and 4 showcase apps demonstrating all language primitives |
| `workflows/` | Real FORGE programs: dev-cycle workflow, forge-sensei learning agent |
| `conformance/` | Language-agnostic JSON test suite — 84 tests covering parser, checkers, and runtime |

---

## Current status

**v0.1.0 released.** Layer 1 — the substrate — is complete. 63 of 72 tracked issues are closed.

- **Parser**: PEG grammar (609 rules) covering all 14 language primitives with comprehensive error diagnostics
- **Semantic checkers** (7): purity, boundary, states, requires, uncertain, spawn, warden
- **Runtime**: 15-module async execution engine — agents, flows, pools, events, timers, knowledge, supervision
- **Agent ecosystem**: spawn/find/retire lifecycle, knowledge stores with categories, instance registry, system orchestration
- **Web runtime**: HTML templates, HTTP client/server, static serving, hot-reload, webhooks, markdown
- **Data operations**: Persistent KV storage (redb), vector embeddings, semantic search
- **Observer**: Live SSE tracing, introspection API, D3 topology visualization, cost dashboard, failure injection
- **Build system**: `forge build` compiles agents to standalone CLI binaries with handler subcommands and interactive REPL
- **Providers**: Anthropic, OpenAI-compatible, Ollama, Groq — swap with a config line
- **Showcase apps**: Wiki (52 tests), Sentinel (17 primitives), Observer (standalone SPA), forge-sensei
- **Test suite**: 883 tests — unit, conformance, integration, E2E (all core tests run with mock provider — no API calls)
- **CLI**: `forge parse`, `forge check`, `forge run`, `forge build`, `forge serve`, `forge trace`, `forge cost`

```bash
# Try it
cargo build
cargo run -- check examples/hello.forge       # semantic validation
cargo run -- run examples/hello.forge          # execute with LLM
cargo run -- serve examples/wiki/ --watch      # web app with hot-reload
cargo run -- build examples/hello.forge -o bin/hello  # standalone binary
cargo test                                     # 883 tests, no API calls
```

See the [roadmap](roadmap.md) for milestone tracking and detailed progress.

---

## The single sentence

FORGE is the language that makes it harder to build systems that are confidently wrong than systems that are honestly uncertain — because the most dangerous agent is not the one that can do everything, but the one that thinks it can.

---

## Contributing

FORGE is open source under the Apache 2.0 license. We welcome contributions — see the [issues](https://github.com/ncmlabs/forge/issues) for what's being worked on.

```bash
# Development workflow
cargo fmt --check && cargo clippy -- -D warnings && cargo test
```

---

## License

[Apache License 2.0](LICENSE)

---

*FORGE v0.1.0 is released. Layer 1 (the substrate) is complete with 63/72 issues closed and 883 tests passing. Tracks A, B, C, and E are 100% done. WASM compilation (Track D) is planned for v0.2.0. See the [roadmap](roadmap.md) for details.*
