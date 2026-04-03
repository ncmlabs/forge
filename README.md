# FORGE

**The first programming language built for oracle-augmented computation.**

FORGE is a language where agents are the primary citizens, uncertainty is a type, and systems build themselves. You describe intent. The language handles the rest.

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

## What FORGE is

FORGE is built on a foundational insight: **an LLM call is not a function call. It is an oracle query.**

A function call is deterministic. Same input, same output, every time. An oracle query is probabilistic. Same input, different output each time, drawn from a distribution the caller cannot inspect. These are different operations. A language that treats them identically is lying about the nature of computation.

FORGE is the first language that treats oracle calls as the first-class primitives they are — with their own syntax, their own type system, their own execution model, and their own failure semantics.

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

## The nine principles

Every feature in FORGE traces to one of nine principles. Every design decision is a refusal or an affirmation of these beliefs.

### I — The Honesty Principle
*A system that hides uncertainty is more dangerous than one that knows nothing.*

Every oracle call returns `uncertain<T>`. The compiler enforces that `uncertain<T>` cannot be used as `T` without handling the uncertainty first. There is no `force_unwrap`. There is no implicit confidence promotion. You cannot pretend to know what you don't know.

### II — The Determinism Boundary
*Two kinds of computation exist. They must never be mixed invisibly.*

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

## The compilation model

FORGE programs compile to native binaries via LLVM. The fleet compilation mode is the factory: describe a system in plain language, an agent fleet writes the LLVM IR modules in parallel, a verification layer gates compilation, and the result is a portable native binary.

```bash
forge fleet
  spec: """
    Real-time analytics pipeline for call center data.
    Ingest at 50,000 events/sec. Aggregate by campaign.
    Detect anomalies. Expose via REST API. HIPAA-compliant.
  """
  agents: 6
  target: "wasm32-wasi"
  verify: strict
  budget: $2.00
```

The agents write the code. The compiler verifies it. LLVM optimizes it. The binary runs forever with zero ongoing LLM cost.

**LLM cost: one-time, ~$1-2 per system**
**Runtime LLM cost: $0.00 (except agents that reason at runtime)**
**Binary runs on: any WASM runtime — server, browser, edge, embedded**

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

## What gets built first

The language primitives needed to unlock the factory model, in order:

```
task + think + when          →  oracle reasoning with uncertainty
pure                         →  deterministic logic, provably hallucination-free
flow + stage + needs         →  parallel pipelines, automatic
agent + states + requires    →  stateful systems with lifecycle enforcement
event + timer                →  broadcast coordination and time-aware behavior
boundary                     →  server/client separation, prompt injection prevention
>> composition               →  universal wiring between all primitives
forge_check + forge_run      →  agents verify and execute their own output
```

When these eight capabilities exist, Layer 2 becomes possible. When Layer 2 exists, the factory runs. When the factory runs, the system builds itself.

---

## The deployment targets

One FORGE binary. Three environments.

**Terminal/server:** Any WASM runtime executes the binary. Stdin, stdout, filesystem, sockets — standard OS interfaces. Deploy like any native binary.

**Browser:** The browser's built-in WASM runtime loads the binary. Canvas, WebGL, WebSockets for real-time. The same game logic that runs on the server runs in the browser from the identical binary.

**Desktop:** A thin native shell wraps the WASM binary. Native window, GPU access, filesystem — desktop feel with a single portable core.

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

**Engineering:** Layer 1 is substantially complete — parser, six semantic checkers, async runtime, LLM providers, and a 27-test conformance suite are operational. Layer 2 (toolkit agents that generate FORGE code) is the next frontier. Error messages are structured data designed for agent repair loops.

**Capability:** The factory model depends on LLMs that can write reliable FORGE code. Today's models can handle simple programs. Complex multi-agent systems with subtle invariants require the repair loop to run multiple times. The factory becomes more powerful as models improve.

**Timing:** The AI agent market is at exactly the moment where the right foundation matters most. The 40% project cancellation rate is a problem waiting for a structural solution. FORGE is that solution — but only if it reaches teams before they've already committed to architectures that can't be revised.

---

## Document index

| Document | Contents |
|---|---|
| `forge-principles.md` | The nine principles — what FORGE believes and why |
| `docs/forge-reference.md` | Complete language reference — syntax, semantics, and compiler enforcement for all 14 primitives |
| `providers.md` | Provider abstraction: trait, registry, implementations, config |
| `roadmap.md` | Architecture, requirements, and layer model |
| `conformance/` | Language-agnostic JSON test suite — 27 tests covering parser, checkers, and runtime |

---

## Current status

Layer 1 — the substrate — is substantially complete:

- **Parser**: PEG grammar covering all 14 language primitives with comprehensive error diagnostics
- **Semantic checkers** (6): purity, boundary, states, requires, uncertain value taint tracking, warden supervision
- **Runtime**: Async executor with LLM provider support (Anthropic, OpenAI-compatible, Ollama, Groq)
- **Conformance suite**: 27 language-agnostic JSON tests covering parser, checkers, and runtime
- **Acceptance tests**: End-to-end tests proving Layer 1 completeness, including a multi-agent tic-tac-toe game
- **Token tracking**: Cost estimation and budget enforcement
- **CLI**: `forge parse`, `forge check`, `forge run`, `forge trace`

```bash
# Try it
cargo build
cargo run -- parse examples/hello.forge    # parse and print AST
cargo run -- check examples/hello.forge    # run semantic checkers
cargo run -- run examples/hello.forge      # execute with configured LLM
cargo test                                 # run all tests including conformance
```

---

## The single sentence

FORGE is the language that makes it harder to build systems that are confidently wrong than systems that are honestly uncertain — because the most dangerous agent is not the one that can do everything, but the one that thinks it can.

---

*FORGE is under active development. Layer 1 (the substrate) is substantially complete. All documents in the repository are working drafts.*
