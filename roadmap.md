# FORGE — Making It Real
## Master Roadmap & Requirements Document

> This is the single document that captures everything needed to bring FORGE
> from concept to reality. It is organized as four layers — the Satisfactory model.
> Each layer uses the output of the layer below to build itself.
> Humans build Layer 1 by hand, once. Agents build everything after that.

---

## The north star

FORGE is a programming language where agents are first-class citizens.
Its ultimate purpose is not to be written by humans — it is to be written by agents,
for agents, building systems that build more systems.

The measure of success is not "can a developer write FORGE?"
It is "can an agent write FORGE that builds a production system with no human
writing a line of code?"

Everything in this document exists to reach that moment.

---

## The four layers

```
┌─────────────────────────────────────────────────────────────┐
│  LAYER 4 — SELF-IMPROVEMENT                                 │
│  The system watches itself, identifies bottlenecks,         │
│  rewrites slow modules, deploys improvements.               │
│  The factory gets better while it runs.                     │
├─────────────────────────────────────────────────────────────┤
│  LAYER 3 — AUTOMATION FACTORY                               │
│  Spec in → running system out.                              │
│  No human writes FORGE. Agents do all of it.                │
├─────────────────────────────────────────────────────────────┤
│  LAYER 2 — FORGE TOOLKIT                                    │
│  Agents that generate FORGE primitives.                     │
│  Humans describe intent. Agents write FORGE.                │
├─────────────────────────────────────────────────────────────┤
│  LAYER 1 — FORGE SUBSTRATE                                  │
│  The language itself. Built by hand, once.                  │
│  The bedrock everything else stands on.                     │
└─────────────────────────────────────────────────────────────┘
```

---

## Milestones

### Layer 1 — Substrate (v0.1.0 shipped 2026-04-09)

| Milestone | Description | Status | Date |
|-----------|-------------|--------|------|
| M1 — Bootstrap | Grammar, parser, AST, CLI scaffold | Done | 2026-03-31 |
| M2 — Semantic Safety | 7 checkers: pure, boundary, states, requires, uncertain, spawn, warden | Done | 2026-04-01 |
| M3 — Core Runtime | Async executor, flows, pools, agents, events, timers, providers | Done | 2026-04-02 |
| M4 — Build System | `forge build` standalone binaries, multi-file composition, agent REPL | Done | 2026-04-03 |
| M5 — Ecosystem Core | Persistent memory, knowledge categories, instance registry, spawn/find/retire, system runtime, forge-sensei specialist spawning | Done | 2026-04-04 |
| M6 — Web Runtime | HTML templates, static serving, hot-reload, HTTP client, markdown, webhooks | Done | 2026-04-05 |
| M7 — Skill Bridge | exec primitive, host skill bridge, tool-use providers, skill E2E test | Done | 2026-04-07 |
| M8 — Wiki Showcase | First real FORGE app: wiki with search agent, fact-checking pool, warden supervision, 52 tests | Done | 2026-04-07 |
| M9 — Sentinel | Killer app: AI-powered repo health dashboard showcasing all 17 primitives | Done | 2026-04-08 |
| M10 — Observer | Elixir-style real-time agent traceability: SSE stream, inspect API, topology viz, cost tracking | Done | 2026-04-09 |
| M11 — WASM Compilation | Cranelift backend for pure functions, browser WASM target | Deferred | — |
| M12 — Release | All core tracks done, conformance suite, documentation, v0.1.0 | Done | 2026-04-09 |

### Layer 2 — Toolkit (Phase 2, in progress)

| Milestone | Description | Issues | Status |
|-----------|-------------|--------|--------|
| P2.M1 — Command | `command` primitive: grammar, sync execution, background mode | #160 ✅ #161 ✅ #162 ✅ | Done |
| P2.M2 — Session Core | `session` primitive: grammar and lifecycle manager | #189 ✅, #190 ✅ | Done |
| P2.M3 — AgentResult + Contract | Typed result contract plus claims/evidence/verification model | #193 ✅ #203 ✅ | Done |
| P2.M4 — Session Adapters + Events | Agent adapters, polling, and event integration | #191 ✅ #192 ✅ | Done |
| P2.M5 — Verification + Contradictions | Verification engine and contradiction/warden integration | #204 ✅ #205 ✅ | Done |
| P2.M6 — Sandbox | `sandbox` isolation: spawn modifier + worktree | #194 ✅ | Done |
| P2.M7 — Skills Foundation | Pluggable skill architecture, capability types, CLI delegation research, deterministic execution | #163 ✅ #164 ✅ #165 ✅ #237 ✅ | Done |
| P2.M8 — CLI Skills | GitHub, Slack, Claude Code, Codex/Ollama SKILL.md files | #176 ✅ #177 ✅ #178 ✅ #179 ✅ | Done |
| P2.M9 — Orchestration | Slack Monitor, Inbound Triager, approval gate | #180 ✅, #181 ✅, #182 | In Progress |
| P2.M10 — Dev System | ProjectAgent, Executor, FORGE + forge-wiki two-project proof | #183-#185 | Frozen |
| P2.M11 — Knowledge School | forge-sensei curriculum + toolkit knowledge transfer + operational readiness | #166 ✅, #167 ✅, #240, #249 ✅, #256 ✅, #257 ✅ | Near Done |
| P2.M12 — Toolkit Agents | Generator contract, Task/Flow/Agent/System generators, Repair, Test, SpecAnalyzer | #168, #169-#175 (frozen) | Frozen (except #168) |
| P2.M13 — Polish | CostEstimator, DocumentationAgent, reference/skill refresh, example validation | #186 (frozen), #187 (frozen), #229, #231 ✅ | In Progress |

### Future Layers

| Milestone | Description | Status |
|-----------|-------------|--------|
| Layer 3 — Automation Factory | Spec in → running system out. No human writes code. | Future |
| Layer 4 — Self-Improvement | Factory watches itself and optimizes deployed systems | Future |

## Implementation tracks

Layer 1 is organized into four parallel tracks:

### Track A — Ecosystem Core: 9/9 COMPLETE

The substrate for agent birth, learning, specialization, discovery, communication, supervision, and graceful retirement.

| Issue | Feature | Status |
|-------|---------|--------|
| #57 | `memory persistent` — ACID storage for agent state | Done |
| #81 | Knowledge categories — domain-specific tagging and export | Done |
| #82 | Instance registry — runtime agent tracking and discovery | Done |
| #85 | `learn` with `category:` suffix — categorized knowledge ingestion | Done |
| #83 | `spawn` statement — create child agents with filtered knowledge | Done |
| #84 | `find` expression — discover running agents by alias or template | Done |
| #86 | `retire` statement — graceful shutdown with knowledge export | Done |
| #87 | `system` declaration runtime — orchestration, event routing, warden integration | Done |
| #88 | Specialist spawning — forge-sensei creates domain expert apprentices | Done |

### Track B — Web & Capabilities: 9/9 COMPLETE

Web runtime, HTTP client, data persistence, and external integrations.

| Issue | Feature | Status |
|-------|---------|--------|
| #44 | HTTP request/response types | Done |
| #45 | HTML template rendering | Done |
| #46 | Static file serving with Tailwind CSS | Done |
| #47 | Hot-reload development mode | Done |
| #49 | Markdown rendering | Done |
| #51 | HTTP client: `web.fetch`, `web.post` | Done |
| #52 | Webhook and callback support | Done |
| #40 | `exec` primitive, host skill bridge, tool-use providers | Done |
| #50 | Vector embeddings and semantic search | Open |

### Track C — Wiki Showcase: 7/7 COMPLETE

The first real FORGE application — a wiki powered by agents, pools, and wardens.

| Issue | Feature | Status |
|-------|---------|--------|
| #59 | Wiki architecture and system wiring | Done |
| #60 | Content agent with lifecycle states | Done |
| #61 | Search agent with confidence gating | Done |
| #62 | Doc generation flow from source | Done |
| #63 | Fact-checking pool with majority vote | Done |
| #64 | Warden supervision for wiki agents | Done |
| #65 | End-to-end wiki acceptance tests (52 tests) | Done |

### Track E — Observer: 7/7 COMPLETE

Elixir-style real-time agent traceability — tap into any running agent, see memory, trace decisions, inspect supervision tree.

| Issue | Feature | Status |
|-------|---------|--------|
| #138 | Live trace stream: `/__forge/events` SSE endpoint | Done |
| #139 | Runtime introspection API: `/__forge/inspect/*` | Done |
| #140 | Sentinel live scan UX: SSE-driven progress | Done |
| #141 | Agent topology visualization: live graph with tap-to-inspect | Done |
| #142 | Cost and confidence dashboard: token economy visibility | Done |
| #143 | Failure injection: test warden policies | Done |
| #144 | Standalone Observer app | Done |

### Track D — WASM Compilation: 0/4

Cranelift backend for compiling FORGE to WebAssembly.

| Issue | Feature | Status |
|-------|---------|--------|
| #53 | WASM codegen foundation: Cranelift for pure functions | Open |
| #54 | WASM codegen for tasks with LLM host imports | Open |
| #55 | Browser WASM target: `--target wasm32-browser` | Open |
| #56 | Boundary compilation: split server/client/shared bundles | Open |

### Progress summary

```
Track A ████████████████████ 9/9  (100%)  — Ecosystem Core
Track B ████████████████████ 9/9  (100%)  — Web & Capabilities
Track C ████████████████████ 7/7  (100%)  — Wiki Showcase
Track D ░░░░░░░░░░░░░░░░░░░░ 0/4  (  0%)  — WASM Compilation (deferred)
Track E ████████████████████ 7/7  (100%)  — Observer
─────────────────────────────────────────
Overall ██████████████████░░ 32/36 ( 89%)
```

---

# LAYER 1 — The substrate

## What it is

The FORGE language runtime, compiler, and tooling. Written in Rust by hand.
This is the only layer humans write directly. It is built once and then the
layers above take over.

The design principle for Layer 1: **every primitive must connect to every
other primitive the same way.** `ConfidentValue` in, `ConfidentValue` out.
The `>>` operator connects everything. This uniformity is what makes
automation possible in Layer 2 and above.

---

## L1.1 — Language primitives

Everything in this section is a language-level construct, not a library.
If it cannot be expressed without a keyword, it goes here.

### Core computation

| Primitive | Keyword | What it does |
|---|---|---|
| Stochastic call | `think` / `reason` | Calls an LLM, returns `uncertain<T>` |
| Deterministic fn | `pure` | No LLM, no side effects, compiler-enforced |
| Confidence branch | `when` | Branches on `.sure`, `.unsure`, `.unreliable` |
| Pattern match | `match` | Structural pattern matching on typed values |
| Boolean branch | `if/else` | Plain boolean conditions |
| Composition | `>>` | Connects any two FORGE primitives |
| Fan-out/in | `(A \| B \| C) >> merge` | Parallel branches, collected result |

### Process model

| Primitive | Keyword | What it does |
|---|---|---|
| Stateful process | `agent` | Long-running supervised process with memory |
| Lifecycle machine | `states` | Typed state transitions, illegal ones = compile error |
| Message guard | `requires` | Preconditions on handlers with explicit fail policies |
| Named timer | `timer` | Fires a handler after duration, cancellable |
| Pipeline | `flow` | Multi-stage computation, auto-parallelized by deps |
| Worker group | `pool` | N agents with resolution strategy (fastest/majority/all) |
| Broadcast | `event` | Typed emit/subscribe streams, one-to-many |
| System wiring | `system` | Top-level composition of all components |

### Safety and structure

| Primitive | Keyword | What it does |
|---|---|---|
| Capability decl | `use` | Declare what the code needs, runtime resolves who provides it |
| Code partition | `boundary` | server / client / shared — compiler-enforced separation |
| Interface contract | `contract` | What an agent must implement |
| Semantic types | type modifiers | `uncertain<T>`, `restricted<T>`, `grounded<T>`, `classified<T>` |

### Failure model

Every agent declares its failure policy. These are not library functions —
they are declarations the compiler understands and the supervisor enforces.

```
on_hallucination: restart | fallback(AgentName) | escalate
on_timeout:       retry(max: N) | fallback(AgentName) | fail
on_cost_exceed:   fallback(CheaperAgent) | downgrade | fail
if stuck for N turns: try X | escalate to supervisor | give Failure(...)
```

---

## L1.2 — Type system

The type system is the primary safety mechanism. It enforces rules at compile
time that would otherwise be runtime surprises.

### Rules the compiler enforces

| Rule | What it prevents |
|---|---|
| `uncertain<T>` must be matched before use | Using a low-confidence LLM output as fact |
| `restricted<T>` cannot flow into log/print | PII leaking into logs |
| `grounded<T>` requires a source | Hallucinated facts presented as citations |
| `pure` cannot contain `think` or tool calls | Deterministic logic calling stochastic LLMs |
| `states` transitions must be declared | Invalid lifecycle transitions at runtime |
| `requires` guards must be checkable | Guards that always pass or always fail |
| `boundary server` cannot reference `boundary client` | Server logic leaking into client bundles |
| `>>` operands must be type-compatible | Mismatched pipeline connections |

### Confidence predicates

These work on any `uncertain<T>` value. They are not methods — they are
language-level predicates the compiler understands.

```
result.sure                  # above default threshold (0.8)
result.sure(above: 0.9)      # explicit threshold
result.unsure                # below threshold, above floor (0.5)
result.unreliable            # below floor — do not trust
result.conflicted            # internal contradiction detected
```

---

## L1.3 — Grammar

The grammar uses PEG (pest crate). Key design decisions:

**Indentation-sensitive, brace-free.** Two spaces per indent level.
Only `boundary` uses an alternative — file-level directive (`#! boundary: server`)
rather than a block, to avoid the dynamic indentation problem.

**`when` is confidence-only.** Takes a `conf_pred` expression, nothing else.
The compiler errors if you use `when` on a non-confident value.

**`match` is structural only.** Takes patterns, not confidence predicates.
The compiler errors if you use `match` on an `uncertain<T>` without unwrapping.

**`if/else` is boolean only.** Plain expressions evaluating to `Bool`.

These three are distinct. A reader always knows which kind of branch they are looking at.

The grammar file: `grammar/forge.pest`
Full grammar specification: see `FORGE_POC_v3.md`

---

## L1.4 — Compiler pipeline

```
Source (.forge)
    ↓
Lexer + Parser (pest PEG)
    ↓
AST
    ↓
Resolver (capabilities, imports, contracts)
    ↓
Type Checker
  ├── uncertain_checker.rs    — uncertain<T> must be matched
  ├── pure_checker.rs         — no LLM calls inside pure
  ├── states_checker.rs       — invalid transitions = error
  ├── boundary_checker.rs     — no cross-boundary leakage
  ├── requires_checker.rs     — guards are satisfiable
  ├── spawn_checker.rs        — failure policy warnings for spawned agents
  └── warden_checker.rs       — supervision coverage validation
    ↓
Cost Estimator (optional — forge cost <file>)
    ↓
Code Generator
  ├── --target wasm32-wasi    → WASM binary (server/CLI)
  ├── --target wasm32-browser → WASM bundle (web)
  ├── --target native         → native binary
  └── --boundary              → filter by boundary layer
    ↓
LLVM IR (via inkwell or cranelift)
    ↓
Optimized binary / WASM
```

### CLI commands

```bash
forge parse  <file>                  # print AST — debug
forge check  <file>                  # type check only, no execution
forge run    <file>                  # execute against configured providers
forge cost   <file>                  # static token/cost estimate
forge build  <file> --target <t>     # compile to binary or WASM
forge fleet  --spec "<text>"         # agent fleet builds from spec
forge test   <file>                  # run with mock provider
```

---

## L1.5 — Runtime

### Execution model

FORGE programs are not scripts. They are long-running supervised systems.
The runtime is responsible for:

- Spawning and monitoring agent processes
- Routing messages between agents through typed channels
- Running the supervision tree (restart on crash)
- Managing the timer engine
- Publishing and delivering events via the event bus
- Enforcing budget limits on LLM calls
- Emitting structured traces for every `think` call

### Key runtime components

| Component | File | What it does |
|---|---|---|
| Task executor | `runtime/executor.rs` | Expression evaluation, environment management |
| Agent process | `runtime/agent.rs` | Event loop, stuck detection, handler dispatch |
| System orchestrator | `runtime/system.rs` | Multi-agent wiring, shared event bus, resource limits |
| Event bus | `runtime/event_bus.rs` | Typed pub/sub with filtered subscriptions |
| Instance registry | `runtime/instance_registry.rs` | Track and discover living agent instances |
| Knowledge store | `runtime/knowledge_store.rs` | Learn, recall, categorize, export knowledge |
| Warden runtime | `runtime/warded.rs` | Supervision with retry, restart, escalate policies |
| Warden policy | `runtime/warden.rs` | Policy enforcement and circuit breaking |
| Pool executor | `runtime/pool.rs` | Worker fleet with fastest/round-robin strategies |
| Storage engine | `runtime/storage.rs` | ACID key-value store (redb) for persistent memory |
| Memory manager | `runtime/memory.rs` | Agent memory with persistent write-through |
| Timer engine | `runtime/timer_engine.rs` | Named timers, async fire, cancellation |
| State machine | `runtime/state_machine.rs` | Lifecycle enforcement at runtime |
| Confidence | `runtime/confidence.rs` | ConfidentValue — universal uncertain<T> wrapper |
| HTTP server | `runtime/http_server.rs` | Endpoint routing for agent-backed APIs |
| Tracer | `tracer.rs` | Structured JSON traces per operation |

### Supervision strategies

```
one_for_one    restart only the crashed agent
one_for_all    restart all agents in the group
rest_for_one   restart crashed agent + all started after it
```

---

## L1.6 — Provider abstraction

Full specification: `FORGE_PROVIDERS.md`

Summary of what is needed:

### The trait

```rust
trait LLMProvider {
    fn name(&self)         -> &str;
    fn capabilities(&self) -> &ProviderCapabilities;
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ProviderError>;
    async fn health_check(&self) -> Result<(), ProviderError>;
    fn estimate_cost(&self, prompt_tokens: u32, max_output: u32) -> f32;
}
```

### Provider implementations needed

| Implementation | Covers |
|---|---|
| `AnthropicProvider` | claude-haiku, claude-sonnet, claude-opus |
| `OpenAICompatProvider` | OpenAI, Ollama, vLLM, LM Studio, Groq, Together, Mistral, Fireworks — anything with an OpenAI-compatible API |
| `MockProvider` | All tests — deterministic, instant, no API key |

The `OpenAICompatProvider` is the most important. It covers the majority of
all current and future providers through one implementation. Every new provider
that launches will almost certainly implement the OpenAI spec.

### Provider registry

- Builds from `forge.config.toml` at startup
- Resolves capability hints to the cheapest satisfying provider
- Follows fallback chains automatically on failure
- Health-checks all providers at startup (optional, warn-only)

### Configuration (`forge.config.toml`)

```toml
[llm]
default = "claude-haiku"

[providers.claude-haiku]
type    = "anthropic"
model   = "claude-haiku-4-5-20251001"
api_key = "${ANTHROPIC_API_KEY}"
fallback = "ollama"

[providers.ollama]
type     = "openai-compat"
model    = "llama3.2"
base_url = "http://localhost:11434/v1"
api_key  = "not-required"

[providers.mock]
type = "mock"
```

### Environment variables

```bash
ANTHROPIC_API_KEY=sk-ant-...
OPENAI_API_KEY=sk-...
FORGE_MOCK=1              # use mock provider, no API calls
FORGE_PROVIDER=ollama     # override default from env
FORGE_TRACE=1             # structured JSON traces to stderr
FORGE_CONFIG=path/to/forge.config.toml
```

---

## L1.7 — Deployment targets

| Target flag | Runtime | Use case |
|---|---|---|
| `--target wasm32-wasi` | wasmtime / wasmer | Server, CLI, cloud functions |
| `--target wasm32-browser` | Browser WebAssembly | Web applications |
| `--target native` | OS directly | Desktop apps, performance-critical services |

The Tauri framework wraps a WASM binary in a native desktop shell with full OS
access (filesystem, GPU, native window). This gives FORGE a desktop GUI path
without a separate compilation target.

---

## L1.8 — Testing requirements

All tests run with the mock provider. `cargo test` should never make an API call.

### Test categories

| Category | File | What it covers |
|---|---|---|
| Parser | `tests/parser_tests.rs` | Every grammar construct parses correctly |
| Uncertain checker | `tests/uncertain_tests.rs` | Unhandled uncertain<T> = compile error |
| Pure checker | `tests/pure_tests.rs` | think inside pure = compile error |
| States checker | `tests/states_tests.rs` | Invalid transitions = compile error |
| Boundary checker | `tests/boundary_tests.rs` | Cross-boundary refs = compile error |
| Requires checker | `tests/requires_tests.rs` | Guard evaluation and fail policies |
| Provider abstraction | `tests/provider_tests.rs` | Trait, registry, fallback, cost tracker |
| Agent runtime | `tests/agent_tests.rs` | Message dispatch, state machine, timers |
| Flow parallelism | `tests/flow_tests.rs` | DAG analysis, wave execution, ordering |
| Event bus | `tests/event_tests.rs` | Emit, subscribe, filter |
| End-to-end | `tests/e2e_tests.rs` | Full programs with mock provider |

### Acceptance tests

The Layer 1 is complete when all of these pass:

```bash
forge check examples/errors/uncertain_error.forge   # should error: unhandled uncertain
forge check examples/errors/pure_error.forge        # should error: think inside pure
forge check examples/errors/states_error.forge      # should error: illegal transition
forge check examples/errors/boundary_error.forge    # should error: cross-boundary ref
forge run   examples/basics/hello.forge             # should print response from LLM
forge run   examples/llm/research.forge          # should show parallel stage timing
forge run   examples/tictactoe/platform.forge # should run full game system
```

---

## L1.9 — Dependencies (Cargo.toml)

```toml
[dependencies]
# Language
pest            = "2"
pest_derive     = "2"

# Runtime
tokio           = { version = "1", features = ["full"] }
async-trait     = "0.1"

# Code generation (choose one)
inkwell         = "0.4"    # LLVM bindings — production quality
# OR
cranelift-codegen = "0.100" # lighter alternative

# Provider HTTP
reqwest         = { version = "0.11", features = ["json"] }
serde           = { version = "1", features = ["derive"] }
serde_json      = "1"

# Config
toml            = "0.8"
dirs            = "5"

# CLI
clap            = { version = "4", features = ["derive"] }

# Error handling
anyhow          = "1"
thiserror       = "1"
```

---

## L1.10 — Build schedule

| Week | Deliverable | Acceptance criterion | Actual |
|---|---|---|---|
| 1 | Parser + AST | `forge parse hello.forge` prints AST | Done 2026-03-31 |
| 2 | Type checker skeleton | `forge check uncertain_error.forge` errors correctly | Done 2026-04-01 |
| 3 | Provider abstraction | Mock + Anthropic providers work | Done 2026-04-01 |
| 4 | Agent runtime + think | `forge run hello.forge` calls real LLM | Done 2026-04-02 |
| 5 | Pure checker | `forge check pure_error.forge` errors correctly | Done 2026-04-01 |
| 6 | States + requires | Lifecycle enforcement works in agent | Done 2026-04-01 |
| 7 | Flow + parallelism | Research example shows parallel timing | Done 2026-04-02 |
| 8 | Event bus + timers | Tic-tac-toe reconnect timer works | Done 2026-04-02 |
| 9 | Boundary enforcement | Cross-boundary ref = compile error | Done 2026-04-01 |
| 10 | Build system | `forge build` produces standalone binary | Done 2026-04-03 |
| 11 | Agent ecosystem | spawn/find/retire/system runtime works | Done 2026-04-04 |
| 12 | Cleanup + documentation | All tests pass, README complete | Done 2026-04-09 |

**Estimated 12 weeks of work delivered in 10 days. Layer 1 shipped as v0.1.0 on 2026-04-09.**

---

# LAYER 2 — The toolkit agents

## What it is

FORGE programs that generate other FORGE programs. These are the first real
FORGE applications — and they prove the language works for its intended purpose.

Layer 2 is written in FORGE (once Layer 1 exists). It is the first moment
the recursive property appears: FORGE agents writing FORGE code.

Phase 2 also builds a complete dev orchestration system — proving FORGE
can manage real multi-project workflows with Slack integration, GitHub
automation, approval gates, and agent delegation to Claude Code and Codex.

---

## L2.0 — Architectural foundation: three-tier primitive stack

Phase 2 introduces three new language primitives that form a progression
from simple shell commands to fully isolated agent delegation:

```
command                  session                    sandbox
├─ sync shell exec       ├─ long-running process     ├─ worktree isolation
├─ background mode       ├─ Claude/Codex adapters    ├─ spawn modifier
├─ structured result     ├─ polling + events         └─ filesystem safety
└─ timeout/env/dir       ├─ AgentResult return
                         └─ budget/permission control
```

### command (done — #160, #161, #162)

First-class `command` expression with dual invocation modes:
- **String mode**: `command "git status"` → shell-interpreted via `sh -c`
- **Argv mode**: `command ["git", "commit", "-m", msg]` → no shell, no injection
- Modifiers: `in` (working dir), `timeout`, `background`, `env`
- Structured return: `result.stdout`, `result.stderr`, `result.exit_code`, `result.success`
- Confidence: 0.9 on exit 0, 0.3 otherwise

### session (new — #189, #190, #191, #192)

Long-running agent delegation to Claude Code, Codex, or generic processes:
```forge
result = session "implement the login page" via claude in "/repo" timeout 30m
```
- Lifecycle: spawn → running → completed | failed | timed_out
- Agent-specific adapters (Claude flags, Codex flags, generic process)
- Polling integration with FORGE event bus
- Returns typed `AgentResult`

### AgentResult (new — #193)

Built-in typed result contract for session returns:
- Fields: `plan`, `patch_summary`, `files_changed`, `tests_run`, `tests_passed`, `cost_usd`, `confidence`, `approval_needed`, `metadata`
- Replaces ad-hoc text parsing of agent output
- Enables confident branching: `when result.sure -> ...`
- `metadata: Map` extensibility hook for #203 (claim/evidence/verification contract)

### sandbox (new — #194)

Worktree isolation for safe agent execution:
- Spawn modifier: `spawn worker in sandbox`
- Creates a git worktree so agents cannot step on each other's filesystem
- Automatic cleanup on agent retire

---

## L2.1 — Phase 2 milestones

### P2.M1: Command (Foundation) — DONE

| Issue | Title | Status |
|-------|-------|--------|
| #160 | `command` primitive — grammar, AST, and parser | Done |
| #161 | `command` runtime — synchronous execution | Done |
| #162 | `command` background mode — process manager | Done |

### P2.M2: Session Core

| Issue | Title | Status |
|-------|-------|--------|
| #189 | session primitive — grammar, AST, and parser | Done |
| #190 | session runtime — lifecycle manager | Done |

### P2.M3: AgentResult + Reliability Contract

| Issue | Title | Status |
|-------|-------|--------|
| #193 | AgentResult built-in type | Done ✅ |
| #203 | Phase 2 reliability: claim/evidence/verification contract | Done ✅ |

### P2.M4: Session Adapters + Events

| Issue | Title | Status |
|-------|-------|--------|
| #191 | session agent adapters — Claude, Codex, generic | Done ✅ |
| #192 | session polling + event integration | Done ✅ |

### P2.M5: Verification + Contradictions

| Issue | Title | Status |
|-------|-------|--------|
| #204 | Phase 2 reliability: verification engine for coding sessions | Done ✅ |
| #205 | Phase 2 reliability: contradiction events and warden integration | Done ✅ |

### P2.M6: Sandbox Isolation

| Issue | Title | Status |
|-------|-------|--------|
| #194 | sandbox isolation — spawn modifier + worktree | Done ✅ |

### P2.M7: Skills Foundation

| Issue | Title | Status |
|-------|-------|--------|
| #163 | Pluggable skill architecture — project-level declarations | Done |
| #164 | Skill capability type system — rich signatures | Done |
| #165 | Explore agent-to-CLI delegation patterns (Claude Code, Codex) | Done |
| #237 | Hybrid deterministic skill execution for simple capabilities | Done |

### P2.M8: CLI Skills (SKILL.md)

| Issue | Title | Status |
|-------|-------|--------|
| #176 | GitHub skill — gh CLI wrapper | Done ✅ |
| #177 | Slack skill — bidirectional Web API via curl | Done ✅ |
| #178 | Claude Code skill — claude CLI wrapper | Done ✅ |
| #179 | Codex + Ollama skills | Done ✅ |

### P2.M9: Orchestration Infrastructure

| Issue | Title | Status |
|-------|-------|--------|
| #180 | Slack Monitor agent — poll channels, detect mentions | Done ✅ |
| #181 | Inbound Triager agent — classify, route, or escalate | Done ✅ |
| #182 | Approval gate pattern — events + Slack + webhook | Open |

### P2.M10: Dev Orchestration System

| Issue | Title | Status |
|-------|-------|--------|
| #183 | ProjectAgent template — per-project config and routing | Open |
| #184 | Executor agent — full issue lifecycle (explore→merge) | Open |
| #185 | Dev orchestration system assembly — FORGE + forge-wiki proof | Open |

### P2.M11: Knowledge School

| Issue | Title | Status |
|-------|-------|--------|
| #166 | forge-sensei code-generation curriculum | Done ✅ |
| #167 | Toolkit agent knowledge transfer infrastructure | Done ✅ |
| #240 | forge-sensei server/client deployment path | In Progress |
| #249 | Epic: Foundation hardening for forge-sensei (9/9 sub-issues closed) | Done ✅ |
| #256 | Split sensei into shared/server/client FORGE sources | Done ✅ |
| #257 | Cross-platform install scripts (macOS, Linux, WSL) | Done ✅ |

### P2.M12: Toolkit Agents

| Issue | Title | Status |
|-------|-------|--------|
| #168 | Generator contract and shared validation infrastructure | Open |
| #169 | TaskGenerator agent | Open |
| #170 | FlowGenerator agent | Open |
| #171 | AgentGenerator agent | Open |
| #172 | SystemAssembler agent | Open |
| #173 | RepairAgent | Open |
| #174 | TestGenerator agent | Open |
| #175 | SpecAnalyzer — capstone agent | Open |

### P2.M13: Polish

| Issue | Title | Status |
|-------|-------|--------|
| #186 | CostEstimator agent | Open |
| #187 | DocumentationAgent | Open |
| #229 | Reference, skill, and example refresh | In Progress |
| #231 | Mock-only FORGE example validation CI gate | Done ✅ |

---

## L2.1a — Phase 2 critical path

Phase 2 is tracked around the trustworthy dev-automation backbone:

1. `P2.M2` Session Core — `#189`, `#190`
2. `P2.M3` AgentResult + Reliability Contract — `#193`, `#203`
3. `P2.M4` Session Adapters + Events — `#191`, `#192`
4. `P2.M5` Verification + Contradictions — `#204`, `#205`
5. `P2.M6` Sandbox — `#194`
6. `P2.M9` Orchestration — `#180-#182`
7. `P2.M10` Dev System — `#183-#185`

Current execution rule:

- `sure/unsure` remains a language branching primitive
- verification establishes trust
- policy and approval gates establish actionability
- commit / PR / merge flows must key off verification and policy, not raw confidence

Done:
- `P2.M1` Command ✅
- `P2.M2` Session Core ✅
- `P2.M3` AgentResult + Reliability Contract ✅
- `P2.M4` Session Adapters + Events ✅
- `P2.M5` Verification + Contradictions ✅
- `P2.M6` Sandbox ✅
- `P2.M7` Skills Foundation ✅
- `P2.M8` CLI Skills ✅

Now:
- `P2.M9` Orchestration — Approval gate (#182)
- `P2.M11` Knowledge School — operational readiness (#240)

Blocked next by:
- `P2.M10` Dev System — ProjectAgent, Executor, two-project proof

Then:
- `P2.M12` Toolkit Agents — Generator contract (#168) unblocks all generators
- `P2.M13` Polish

---

## L2.2 — The full orchestration picture

```
INBOUND MONITORS                          OUTBOUND
┌─────────────────────┐                   ┌──────────────────────┐
│ Slack Monitor       │──→ SlackMention   │ Slack Notifier       │
│ (polls channels,    │                   │ (chat.postMessage,   │
│  detects @mentions) │                   │  approval requests,  │
│                     │                   │  status updates)     │
├─────────────────────┤                   ├──────────────────────┤
│ GitHub Monitor      │──→ IssueCreated   │ GitHub Actor         │
│ (watches repos for  │   PRReviewNeeded  │ (gh issue create,    │
│  new issues, PRs)   │                   │  gh pr create, etc.) │
└─────────┬───────────┘                   └──────────────────────┘
          │                                        ▲
          ▼                                        │
┌─────────────────────────────────────────────────────────────────┐
│                        TRIAGER AGENT                            │
│  Classifies inbound → routes to project agents or escalates    │
│  "Can I handle this?" → yes: route / no: notify human          │
└──────────┬──────────────────────────────────┬───────────────────┘
           │                                  │
           ▼                                  ▼
┌──────────────────────┐           ┌──────────────────────┐
│ ProjectAgent(forge)  │           │ ProjectAgent(wiki)   │
│ tool: claude         │           │ tool: codex/ollama   │
│ repo: ncmlabs/forge  │           │ repo: ncmlabs/wiki   │
└──────────┬───────────┘           └──────────┬───────────┘
           │                                  │
           ▼                                  ▼
┌──────────────────────┐           ┌──────────────────────┐
│ Executor (per issue) │           │ Executor (per issue) │
│ branch→implement→    │           │ branch→implement→    │
│ test→PR→CI→merge     │           │ test→PR→CI→merge     │
└──────────┬───────────┘           └──────────┬───────────┘
           │                                  │
           ▼                                  ▼
┌─────────────────────────────────────────────────────────────────┐
│                    APPROVAL GATE                                │
│  emit ApprovalRequest → Slack notification with approve/reject │
│  webhook callback → ApprovalResponse event → agent resumes     │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│  Warden: DevLead — supervises all agents, escalation ladder    │
└─────────────────────────────────────────────────────────────────┘
```

---

## L2.3 — Design decisions

1. **Command first** — `command` is the foundation; skills and sessions depend on it
2. **Three-tier primitive stack** — `command` (shell) → `session` (agent delegation) → `sandbox` (isolation)
3. **Skills are pluggable, not built-in** — declared in `forge.project.toml`, resolved at load time, swappable without changing agent code
4. **Knowledge via school** — forge-sensei (teacher) + forge-wiki (textbook) → toolkit agents (students)
5. **Two-project proof** — FORGE (claude) + forge-wiki (codex/ollama) with different tool preferences
6. **Slack via Web API** — curl + bot token for `chat.postMessage`, `conversations.history`, not the Slack Platform CLI

---

## L2.4 — Core toolkit agents

All toolkit agents implement the same Generator contract:

```forge
contract Generator
  on generate(spec: Text) -> ForgeCode or GenerationError
  on validate(code: ForgeCode) -> ValidationResult
  on repair(code: ForgeCode, errors: CompilerError[]) -> ForgeCode or GenerationError
```

Any agent that implements `Generator` can be swapped into the factory.
When a better TaskGenerator is written, it replaces the old one transparently.

### Reference pattern: TaskGenerator

```forge
agent TaskGenerator
  memory
    examples: ForgeCode[]    # few-shot examples of good tasks
    common_errors: Error[]   # errors seen before, how they were fixed

  on generate(spec: Text) -> ForgeCode or GenerationError
    requires spec.length > 10    on fail: give GenerationError("spec too vague")

    code = reason """
      Write a FORGE task for this requirement:
      {spec}

      Rules:
      - Use `pure` for any deterministic computation
      - All LLM calls return uncertain<T> — handle with when/match
      - Add requires guards with explicit on fail policies
      - No provider-specific code — use capability hints only

      Examples of well-written FORGE tasks:
      {memory.examples}

      Return only the FORGE code. No explanation.
    """

    verified = forge_compiler.check(code)

    when verified.passed
      give code

    when verified.failed
      fixed = fix_errors(code, verified.errors, memory.common_errors)
      memory.common_errors += (verified.errors, fixed)
      give fixed

  pure fix_errors
    needs code: ForgeCode, errors: CompilerError[], known: Error[]
    gives ForgeCode
    do
      # Pattern-match against known error signatures
      # Apply mechanical fixes where possible
      # Return fixed code for retry
```

### Support agents

- **FlowGenerator** — pipeline description → FORGE flow with correct `needs` dependencies and parallel stages
- **AgentGenerator** — agent purpose + lifecycle → complete agent with memory, timers, requires, failure policies
- **SystemAssembler** — generated components → runnable `system` declaration with typed channels and supervision
- **RepairAgent** — FORGE code + compiler errors → fixed code, learns from every failure across the toolkit
- **TestGenerator** — FORGE system → test cases covering every `when` branch, `requires` guard, state transition
- **SpecAnalyzer** — natural language → structured decomposition → orchestrates all generators → runnable system
- **CostEstimator** — FORGE system → per-step cost estimate, flags expensive operations
- **DocumentationAgent** — FORGE system → human-readable markdown docs, auto-updated on changes

---

## L2.5 — Dependency chain and schedule

```
command (#160-162) ✅
  ├─→ session (#189-192) → AgentResult (#193) → sandbox (#194)
  └─→ pluggable skills (#163-164)
        ├─→ CLI delegation research (#165) → CLI skills (#176-179)
        └─→ knowledge school (#166-167) → toolkit agents (#168-175)
                                                └─→ orchestration (#180-182) → dev system (#183-185)
                                                                                    └─→ polish (#186-187)
```

### Phase 2 progress

```
P2.M1  Command         ████████████████████  3/3  (100%)  DONE
P2.M2  Session Core    ████████████████████  2/2  (100%)  DONE
P2.M3  Result+Contract ████████████████████  2/2  (100%)  DONE
P2.M4  Adapters+Events ████████████████████  2/2  (100%)  DONE
P2.M5  Verify+Contra   ████████████████████  2/2  (100%)  DONE
P2.M6  Sandbox         ████████████████████  1/1  (100%)  DONE
P2.M7  Skills          ████████████████████  4/4  (100%)  DONE
P2.M8  CLI Skills      ████████████████████  4/4  (100%)  DONE
P2.M9  Orchestration   █████████████░░░░░░░  2/3  ( 67%)  IN PROGRESS
P2.M10 Dev System      ░░░░░░░░░░░░░░░░░░░░  0/3  (  0%)  FROZEN
P2.M11 Knowledge       ████████████████░░░░  5/6  ( 83%)  NEAR DONE — #240 remaining
P2.M12 Toolkit         ░░░░░░░░░░░░░░░░░░░░  0/8  (  0%)  FROZEN
P2.M13 Polish          ██████████░░░░░░░░░░  2/4  ( 50%)  IN PROGRESS
─────────────────────────────────────────────────────────
Phase 2 Overall         █████████████░░░░░░░  28/44 ( 64%)
```

---

# LAYER 3 — The automation factory

## What it is

The full pipeline from business requirement to deployed running system.
No human writes FORGE. The toolkit agents do all of it.

Layer 3 is built from Layer 2 components assembled by SystemAssembler.
The factory is itself a FORGE system.

---

## L3.1 — The factory flow

```forge
flow build_system
  needs spec: Text
  needs target: DeploymentTarget
  gives DeployedSystem or BuildFailure

  stage analyze
    decomposition = SpecAnalyzer.analyze(spec)
    cost_estimate = CostEstimator.estimate(decomposition)
    when cost_estimate.exceeds_budget -> give BuildFailure("over budget")

  stage generate
    needs analyze.decomposition
    # All generators run in parallel
    tasks   = TaskGenerator.generate(analyze.decomposition.tasks)
    flows   = FlowGenerator.generate(analyze.decomposition.flows)
    agents  = AgentGenerator.generate(analyze.decomposition.agents)
    types   = TypeGenerator.generate(analyze.decomposition.shared_types)

  stage assemble
    needs generate.*
    system_code = SystemAssembler.assemble(generate.*)
    docs        = DocumentationAgent.document(system_code)

  stage verify
    needs assemble.system_code
    check = forge_compiler.check(assemble.system_code)
    when check.failed
      repaired = RepairAgent.repair(assemble.system_code, check.errors)
      # Re-run verify with repaired code
      give build_system(spec, target)

  stage test
    needs verify.*
    tests   = TestGenerator.generate(assemble.system_code)
    results = forge_runner.test(assemble.system_code, tests)
    when results.any_failed
      give BuildFailure("tests failed", results.failures)

  stage deploy
    needs test.*
    binary   = forge_compiler.build(assemble.system_code, target)
    deployed = deployer.ship(binary, target)
    monitor.watch(deployed)

  give DeployedSystem(
    url:       target.url,
    docs:      assemble.docs,
    dashboard: monitor.dashboard_url,
    cost_est:  analyze.cost_estimate
  )
```

---

## L3.2 — Infrastructure components

These are not FORGE code — they are the services the factory depends on.

| Component | What it is | Why needed |
|---|---|---|
| FORGE compiler service | HTTP service wrapping `forge check` + `forge build` | Toolkit agents need to verify code |
| FORGE runner service | HTTP service wrapping `forge run` | Factory runs test suites |
| Binary registry | S3-compatible storage | Store built WASM binaries |
| Deployment service | Kubernetes / fly.io / Railway integration | Ship binaries to production |
| Monitoring service | Metrics + traces collector | Feed data to Layer 4 |
| Secret store | Vault / environment service | Provider API keys for deployed systems |

---

## L3.3 — The factory API

The factory exposes one endpoint to the outside world:

```
POST /build
  body: { spec: "...", target: "wasm32-wasi", budget: 5.00 }
  
  returns: {
    status: "deployed" | "failed",
    url: "https://...",
    docs_url: "https://...",
    dashboard_url: "https://...",
    build_cost_usd: 1.24,
    estimated_runtime_cost_per_call: 0.0003
  }
```

That is the entire interface. Spec in, system out.

---

## L3.4 — Factory build schedule

| Week | Deliverable | Acceptance criterion |
|---|---|---|
| 22-24 | Compiler + runner services | Toolkit agents can check and run code via HTTP |
| 25-26 | Deployment service | Built binary deploys to cloud target |
| 27-28 | Full factory pipeline | `POST /build` with a spec returns a deployed URL |
| 29-30 | Monitoring integration | Deployed systems emit traces the factory can see |

**Total: 2 additional months.**

---

# LAYER 4 — Self-improvement

## What it is

The factory that improves itself. Agents that watch running systems, identify
inefficiencies, rewrite the problematic modules, shadow-deploy the rewrites,
and promote improvements automatically.

---

## L4.1 — The optimizer agent

```forge
agent SystemOptimizer
  memory
    watched:  WatchedSystem[]
    history:  OptimizationRecord[]

  subscribe PerformanceEvent where event.is_anomaly

  on PerformanceEvent(e)
    analysis = reason """
      This FORGE module is underperforming:
      
      Module: {e.module}
      P95 latency: {e.latency_p95}ms
      Cost per call: ${e.cost_per_call}
      Token profile: {e.token_profile}
      Error rate: {e.error_rate}%
      
      Source code:
      {e.source_code}
      
      Identify the root cause and suggest a specific fix.
    """

    when analysis.sure(above: 0.85)
      improvement = reason """
        Rewrite this FORGE module to fix the identified issue:
        {analysis}
        
        Original code:
        {e.source_code}
        
        The rewrite must:
        - Keep the same interface (inputs and outputs unchanged)
        - Fix the identified problem
        - Not introduce new ones
        
        Return only valid FORGE code.
      """

      verified = forge_compiler.check(improvement)
      when verified.passed
        shadow   = deployer.shadow(improvement, e.module)
        results  = monitor.compare(e.module, shadow, duration: 15min)

        when results.improvement_confirmed
          deployer.promote(shadow)
          record = OptimizationRecord(
            module:     e.module,
            problem:    analysis,
            fix:        improvement,
            latency_delta: results.latency_delta,
            cost_delta:    results.cost_delta,
            timestamp:     now()
          )
          memory.history += record
          emit OptimizationApplied(record)

  if stuck for 5 attempts
    escalate to human_engineer
```

---

## L4.2 — What the optimizer watches

| Metric | Threshold | Action |
|---|---|---|
| P95 latency | > 2× baseline | Analyze + suggest optimization |
| Cost per call | > 1.5× baseline | Try cheaper model or context reduction |
| Error rate | > 5% | Analyze failure patterns, suggest fixes |
| Token usage | > 2× expected | Identify context bloat, suggest compaction |
| Confidence scores | < 0.7 average | Suggest prompt improvements |
| Stuck detection rate | > 10% | Analyze patterns, suggest clearer prompts |

---

## L4.3 — The self-improvement loop

```
1. Deployed system runs
2. Monitoring collects latency, cost, errors, confidence scores
3. PerformanceEvent emitted when anomaly detected
4. SystemOptimizer analyzes the underperforming module
5. Optimizer generates improved FORGE code
6. Compiler verifies the improvement
7. Shadow deploy — both versions run simultaneously
8. Monitor compares the two versions for 15 minutes
9. If new version is better → promote automatically
10. If new version is worse → discard, try again
11. Optimization recorded in history
12. Factory toolkit agents learn from the history
13. Future generated code avoids the patterns that needed fixing
```

---

# Cross-cutting requirements

These apply to all four layers.

---

## Security

| Requirement | How |
|---|---|
| API keys never in code | `${ENV_VAR}` references in config only |
| PII cannot log | `restricted<T>` type, compiler-enforced |
| Server code cannot reach client | `boundary` checker |
| Prompt injection detection | FORGE code that processes external input must sanitize before `reason` |
| Agent isolation | Each agent runs in its own process, no shared memory |
| Tool call confirmation | `irreversible: true` on tool declarations triggers human confirmation |
| Budget limits | Hard caps in config, enforced by cost tracker before each call |

---

## Observability

Every FORGE system emits structured traces automatically. No instrumentation code.

```json
{
  "timestamp": "2026-04-01T12:00:00Z",
  "step": "ResearchFlow.gather.web_search",
  "type": "think",
  "provider": "claude-haiku",
  "tokens_in": 312,
  "tokens_out": 891,
  "latency_ms": 1240,
  "cost_usd": 0.00031,
  "confidence": 0.87,
  "result_variant": "Certain",
  "agent_id": "researcher-001",
  "flow_stage": "gather",
  "parallel_with": ["gather.paper_search", "gather.news_search"]
}
```

The tracer emits one JSON object per `think` call, per tool call, per state
transition, and per agent restart. This trace stream feeds Layer 4's monitoring.

---

## Economics — the cost model

FORGE systems have two cost phases:

**Build phase (one-time):**
- Layer 2 toolkit agent calls generate the system
- Typically $0.50 – $5.00 for a complete business system
- Paid once

**Runtime phase (ongoing):**
- Only `think` and `reason` calls cost money
- `pure` functions, state machines, routing, composition — all free
- Compiled WASM logic — zero AI cost per execution
- AI opponent in a game — small cost per move
- Complex reasoning tasks — cost per call depends on model

The economics improve dramatically as more logic moves into `pure` functions
and compiled code. A well-designed FORGE system spends AI tokens on decisions,
not on computation.

---

## Conformance suite

A language-agnostic test suite that proves a FORGE implementation is correct.
This is critical for adoption — AI coding tools can implement FORGE correctly
from the conformance suite without needing FORGE to appear in their training data.

The conformance suite consists of:
- Input `.forge` files
- Expected compiler output (error or success)
- For runnable programs: expected trace shape (not content)
- For type errors: expected error message patterns

Format: JSON files that can be consumed by any test runner.

```json
{
  "name": "uncertain_must_be_matched",
  "input": "agent A\n  on handle(x: Text)\n    result = reason x\n    give result\n",
  "expected": {
    "outcome": "compile_error",
    "error_code": "E012",
    "error_contains": "uncertain value used without handling"
  }
}
```

Any agent given the conformance suite can generate a FORGE implementation.
Any FORGE implementation can verify itself against it. This is the adoption lever.

---

## Documentation

| Document | Purpose | Status |
|---|---|---|
| `FORGE_POC_v3.md` | POC implementation plan | Done |
| `FORGE_PROVIDERS.md` | Provider abstraction spec | Done |
| `FORGE_LANGUAGE_SPEC.html` | Visual language reference | Done |
| `FORGE_PROVIDERS.md` | Provider implementation | Done |
| `FORGE_MAKING_IT_REAL.md` | This document | Done |
| `FORGE_GRAMMAR.pest` | Complete grammar file | Layer 1 |
| `FORGE_STDLIB.md` | Standard capabilities reference | Layer 1 |
| `FORGE_CONFORMANCE.json` | Conformance test suite | Layer 1 |
| `FORGE_TOOLKIT.md` | Toolkit agent reference | Layer 2 |
| `FORGE_FACTORY_API.md` | Factory HTTP API reference | Layer 3 |

---

# Summary — what needs to exist in order

Phase 1 (Layer 1): Build FORGE — **SHIPPED v0.1.0 on 2026-04-09**
  ✅ Grammar + parser + AST
  ✅ Type checker (7 checker modules)
  ✅ Provider abstraction + Anthropic + OpenAI-compat + Ollama + Groq + Mock
  ✅ Agent runtime + supervision + event bus + timers
  ✅ Flow executor with parallel DAG
  ✅ Agent ecosystem: spawn, find, retire, system orchestration
  ✅ Knowledge system: learn, recall, categories, export
  ✅ Build system: standalone binaries with CLI + REPL
  ✅ forge-sensei: self-referential learning agent
  ✅ CLI: parse / check / run / cost / build / trace
  ✅ Test suite — 919 tests, no real API calls
  ✅ Web runtime (HTML, HTTP client, static serving, webhooks)
  ✅ Host skill bridge (exec, SKILL.md, tool-use providers)
  ✅ Wiki showcase (7 issues, 52 tests, full documentation)
  ✅ Sentinel killer app (17 primitives, AI-powered repo health)
  ✅ Observer (real-time agent tracing, topology viz, cost tracking)
  ⬜ WASM compilation (Cranelift backend) — deferred, not blocking Phase 2
  Goal: tic-tac-toe system runs end-to-end ✅

Phase 2 (Layer 2): Build the toolkit — **IN PROGRESS (9/39 issues done)**
  ✅ P2.M1: `command` primitive — grammar, sync, background (#160-#162)
  ✅ P2.M2: `session` core — grammar and lifecycle manager done (#189 ✅, #190 ✅)
  ⬜ P2.M3: `AgentResult` + reliability contract — claims, evidence, verification (#193, #203)
  ⬜ P2.M4: session adapters + events — Claude/Codex/generic and event hooks (#191-#192)
  ⬜ P2.M5: verification + contradictions — trusted gating before repo mutations (#204-#205)
  ✅ P2.M6: `sandbox` — worktree isolation for safe execution (#194 ✅)
  ✅ P2.M7: Skills foundation — pluggable architecture, rich types (#163 ✅, #164 ✅, #165 ✅)
  ⬜ P2.M8: CLI skills — GitHub, Slack, Claude Code, Codex/Ollama (#176-#179)
  ⬜ P2.M9: Orchestration — Slack Monitor, Triager, Approval gate (#180-#182)
  ⬜ P2.M10: Dev system — ProjectAgent, Executor, two-project proof (#183-#185)
  ⬜ P2.M11: Knowledge school — sensei curriculum, knowledge transfer (#166-#167)
  ⬜ P2.M12: Toolkit agents — Generator contract, 7 generators (#168-#175)
  ⬜ P2.M13: Polish — CostEstimator, DocumentationAgent, reference/skill refresh, example validation (#186-#187, #229, #231 ✅)
  Goal: describe a system → get runnable FORGE code; dev orchestration for FORGE + forge-wiki

Phase 3 (Layer 3): Build the factory
  ⬜ Compiler service (HTTP wrapper)
  ⬜ Runner service (HTTP wrapper)
  ⬜ Binary registry (S3)
  ⬜ Deployment service
  ⬜ Monitoring + trace collector
  ⬜ Factory flow (build_system.forge)
  ⬜ Factory API (POST /build)
  Goal: describe a system, get a deployed URL

Phase 4 (Layer 4): Self-improvement
  ⬜ SystemOptimizer.forge
  ⬜ Shadow deploy infrastructure
  ⬜ Performance anomaly detection
  ⬜ Automatic promotion pipeline
  Goal: system improves itself while running

---

## The moment it becomes real

The proof that FORGE works is not when a developer writes a FORGE program.

It is when you give the factory a plain-language description of a business
problem, it produces a running system, that system handles real traffic,
the optimizer identifies a slow module, rewrites it, shadow-deploys the
improvement, and promotes it — all without a human writing a single line
of code.

That moment is closer than you think.

Everything in this document exists to reach that moment.

---

*FORGE — Making It Real · Master Roadmap v3.0*
*Layers: Substrate → Toolkit → Factory → Self-improvement*
*Layer 1: v0.1.0 shipped (32/36 tracks, 89%) · Phase 2: 10/40 issues (25%) · Next: session adapters/events → AgentResult/reliability contract → verification*
*Last updated: 2026-04-11*
