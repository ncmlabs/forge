# FORGE Language — POC Implementation Plan

> Feed this file to Claude Code. It contains everything needed to build a working
> proof-of-concept interpreter for the FORGE language in Rust.
> Work through phases in order. Each phase is independently runnable and testable.

---

## What FORGE is (30-second summary)

FORGE is a programming language for LLM agent fleets. Key ideas:
- `agent` — a supervised, stateful process (Erlang GenServer-inspired)
- `think` — a stochastic LLM call that returns `uncertain<T>` (not a plain value)
- `plan` — a multi-step pipeline where the runtime auto-parallelizes independent steps
- `supervise` — an OTP-style supervision tree with restart strategies
- `tool` — a typed side-effect declaration with cost/rate-limit metadata
- `uncertain<T>` — first-class type for LLM outputs; forces the programmer to handle uncertainty before using the value

End goal of this POC: a CLI that parses a `.forge` file, type-checks it, and executes it against a real LLM (Anthropic API), printing structured traces of every step.

---

## Tech stack

| Concern | Choice | Why |
|---|---|---|
| Language | Rust | Memory safety, fast, great parser ecosystem |
| Parser | `pest` crate (PEG grammar) | Clean grammar files, good error messages |
| LLM backend | Anthropic API (`reqwest` + `serde_json`) | Claude is best at structured output |
| Async runtime | `tokio` | Needed for parallel `plan` steps |
| CLI | `clap` | Standard Rust CLI |
| Serialization | `serde` + `serde_json` | Trace output |
| Testing | `cargo test` + inline fixtures | No extra framework needed |

---

## Project structure

```
forge/
├── Cargo.toml
├── README.md
├── grammar/
│   └── forge.pest              # PEG grammar for the language
├── src/
│   ├── main.rs                 # CLI entry point
│   ├── lexer.rs                # Token types (generated from pest)
│   ├── ast.rs                  # AST node definitions
│   ├── parser.rs               # pest → AST transformation
│   ├── types.rs                # Type system: uncertain<T>, restricted<T>, etc.
│   ├── checker.rs              # Type checker / semantic analysis
│   ├── runtime/
│   │   ├── mod.rs              # Runtime coordinator
│   │   ├── agent.rs            # Agent process model
│   │   ├── supervisor.rs       # Supervision tree + restart strategies
│   │   ├── context.rs          # Context scoping and compaction
│   │   ├── plan.rs             # DAG builder + parallel executor
│   │   └── channel.rs          # Typed channel implementation
│   ├── llm/
│   │   ├── mod.rs              # LLM backend abstraction
│   │   ├── anthropic.rs        # Anthropic API client
│   │   └── mock.rs             # Mock LLM for tests (no API key needed)
│   ├── tools/
│   │   ├── mod.rs              # Tool registry
│   │   └── builtin.rs          # Built-in tools: web_search stub, echo, file_read
│   └── tracer.rs               # Structured trace emitter (JSON)
├── examples/
│   ├── hello_agent.forge       # Simplest possible agent
│   ├── uncertain_match.forge   # uncertain<T> pattern matching
│   ├── parallel_plan.forge     # plan with auto-parallelization
│   └── supervised.forge        # Supervision tree with restart
└── tests/
    ├── parser_tests.rs
    ├── type_checker_tests.rs
    └── runtime_tests.rs
```

---

## Phase 1 — Lexer + Parser (no execution)

**Goal:** Parse a `.forge` file into an AST and pretty-print it. No execution yet.

**Deliverable:** `forge parse examples/hello_agent.forge` prints the AST.

### 1.1 — Bootstrap the project

```bash
cargo new forge
cd forge
# Add to Cargo.toml:
# pest = "2"
# pest_derive = "2"
# tokio = { version = "1", features = ["full"] }
# serde = { version = "1", features = ["derive"] }
# serde_json = "1"
# reqwest = { version = "0.11", features = ["json"] }
# clap = { version = "4", features = ["derive"] }
# anyhow = "1"
# thiserror = "1"
```

### 1.2 — Grammar file (`grammar/forge.pest`)

Implement PEG grammar for these constructs **in this order** (each builds on prior):

```
1. Identifiers, keywords, string literals, number literals, comments
2. Type expressions: String, u64, f32, Bool, uncertain<T>, restricted<T>
3. Expressions: literals, identifiers, function calls, pipe operator |>
4. Statements: let bindings, match expressions, return
5. think call: `think expr { budget: N, model: "..." }`
6. tool declaration
7. agent declaration with state block and handle methods
8. plan declaration with step assignments
9. supervise block with strategy and channel declarations
10. context block with compaction policy
11. interface declaration
12. Top-level fn main()
```

Important grammar rules to get right:
- `think` is a keyword-prefixed expression, not a function call
- `uncertain<T>` is a parameterized type — handle the `<>` carefully in PEG
- `|>` pipe operator is left-associative
- `match` branches use `=>` not `:`
- `channel<T>` in supervise blocks is a type annotation on the pipe

### 1.3 — AST types (`src/ast.rs`)

```rust
// Core AST nodes — implement these
pub enum Expr {
    Literal(Literal),
    Ident(String),
    Call { name: String, args: Vec<Expr> },
    Think { expr: Box<Expr>, params: ThinkParams },
    Pipe { left: Box<Expr>, right: Box<Expr> },
    Match { subject: Box<Expr>, arms: Vec<MatchArm> },
    Using { think: Box<Expr>, context_items: Vec<Expr> },
}

pub enum Stmt {
    Let { name: String, ty: Option<TypeExpr>, value: Expr },
    Expr(Expr),
    Return(Expr),
}

pub struct AgentDecl {
    pub name: String,
    pub implements: Option<String>,
    pub state: Vec<StateField>,
    pub failure_policies: Vec<FailurePolicy>,
    pub handles: Vec<HandleDecl>,
}

pub struct PlanDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: TypeExpr,
    pub steps: Vec<PlanStep>,   // name = expr
}

pub struct SuperviseDecl {
    pub name: String,
    pub agents: Vec<AgentBinding>,
    pub strategy: RestartStrategy,
    pub channels: Vec<ChannelDecl>,
}

pub enum TypeExpr {
    Simple(String),
    Uncertain(Box<TypeExpr>),
    Restricted(Box<TypeExpr>),
    Grounded(Box<TypeExpr>, Box<TypeExpr>),
    Classified(Box<TypeExpr>, Vec<String>),
    Channel(Box<TypeExpr>),
    Array(Box<TypeExpr>),
}
```

### 1.4 — Parser (`src/parser.rs`)

Transform `pest::Pairs` into the AST. Use a recursive descent approach over the pest output. Return `anyhow::Result<Program>`.

### 1.5 — Minimum test fixture

Create `examples/hello_agent.forge`:

```forge
agent Greeter {
    state {
        model: "claude-haiku-3"
    }

    on_hallucination: restart

    handle greet(name: String) -> String {
        let reply = think respond(name)
        reply
    }
}

fn main() {
    let g = Greeter()
    let result = g.greet("world")
    print(result)
}
```

**Test:** `cargo run -- parse examples/hello_agent.forge` should print AST without errors.

---

## Phase 2 — Type System

**Goal:** Type-check a parsed program. Catch: unhandled `uncertain<T>`, `restricted<T>` flowing into log calls, missing match arms.

**Deliverable:** `forge check examples/uncertain_match.forge` prints type errors or "OK".

### 2.1 — Type definitions (`src/types.rs`)

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum ForgeType {
    // Primitives
    String, Bool, U64, F32, I64, Unit,
    // Parameterized
    Uncertain(Box<ForgeType>),
    Restricted(Box<ForgeType>),
    Grounded(Box<ForgeType>, GroundSource),
    Classified(Box<ForgeType>, Vec<String>),
    Array(Box<ForgeType>),
    Channel(Box<ForgeType>),
    // User-defined
    Named(String),
    // Unknown during inference
    Unknown,
}

impl ForgeType {
    pub fn is_uncertain(&self) -> bool { matches!(self, ForgeType::Uncertain(_)) }
    pub fn is_restricted(&self) -> bool { matches!(self, ForgeType::Restricted(_)) }
    pub fn inner(&self) -> Option<&ForgeType> { /* unwrap one layer */ }
}
```

### 2.2 — Type checker (`src/checker.rs`)

Implement these rules **strictly** — these are the language's safety guarantees:

**Rule 1: Uncertain must be matched**
```
If expr has type uncertain<T>, it cannot be used as T directly.
It must appear as the subject of a match expression.
ERROR: let x: String = think foo()   // think returns uncertain<String>
OK:    match think foo() { certain(s) => use(s), ... }
```

**Rule 2: Restricted cannot flow into log/print**
```
If a function is marked as a sink (log, print, debug, trace),
restricted<T> values cannot be passed to it.
ERROR: print(user.email)   // email: restricted<String>
```

**Rule 3: Think always returns uncertain<T>**
```
The return type of any `think` expression is always uncertain<T>
where T is the declared return type of the think target.
```

**Rule 4: Match arms must cover uncertain spectrum**
```
A match on uncertain<T> must have at least:
- one arm for certain(_) or certain(x) if ...
- one arm for uncertain(_) or failed(_) or a wildcard _
WARN: missing arm for hallucinated(_) [not error, just warn]
```

**Rule 5: Plan step types are inferred from their expressions**
```
In a plan block, each step name gets the type of its RHS expression.
Steps can reference other steps — this builds the dependency graph.
```

### 2.3 — Test fixture (`examples/uncertain_match.forge`)

```forge
agent Classifier {
    state { model: "claude-haiku-3" }

    handle classify(text: String) -> String {
        // This should TYPE ERROR — using uncertain<String> as String directly
        let label: String = think categorize(text)
        label
    }
}
```

And the correct version that should pass:

```forge
agent Classifier {
    state { model: "claude-haiku-3" }

    handle classify(text: String) -> String {
        let result = think categorize(text)
        match result {
            certain(label) if label.confidence > 0.8 => label.value,
            certain(label)                            => label.value,
            uncertain(_)                              => "unknown",
            failed(_)                                 => "error"
        }
    }
}
```

---

## Phase 3 — Runtime: Agent + Think

**Goal:** Execute a simple agent against the real Anthropic API. `uncertain<T>` is a real value at runtime.

**Deliverable:** `forge run examples/hello_agent.forge` calls Claude and prints the response with a trace.

### 3.1 — LLM backend abstraction (`src/llm/mod.rs`)

```rust
#[async_trait]
pub trait LLMBackend: Send + Sync {
    async fn complete(
        &self,
        prompt: &str,
        params: &ThinkParams,
    ) -> anyhow::Result<LLMResponse>;
}

pub struct LLMResponse {
    pub content: String,
    pub confidence: f32,        // estimated from logprobs or heuristic
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub cost_usd: f32,
    pub model: String,
    pub latency_ms: u64,
}
```

### 3.2 — Anthropic client (`src/llm/anthropic.rs`)

- Read `ANTHROPIC_API_KEY` from env
- POST to `https://api.anthropic.com/v1/messages`
- Model: configurable, default `claude-haiku-4-5`
- Map response to `LLMResponse`
- Estimate confidence: use a simple heuristic (output length / max_tokens ratio inverted, or just 0.85 as default for POC)

### 3.3 — Mock backend (`src/llm/mock.rs`)

For tests — returns deterministic responses without API calls.

```rust
pub struct MockLLM {
    pub responses: HashMap<String, String>,  // prompt substring → response
    pub default: String,
}
```

### 3.4 — Uncertain<T> runtime value (`src/types.rs`)

```rust
#[derive(Debug, Clone)]
pub enum UncertainValue {
    Certain   { value: Value, confidence: f32 },
    Uncertain { value: Value, confidence: f32 },
    Hallucinated { raw: String },
    Failed    { reason: String },
}

impl UncertainValue {
    pub fn from_response(resp: LLMResponse, threshold_certain: f32, threshold_uncertain: f32) -> Self {
        if resp.confidence >= threshold_certain {
            Self::Certain { value: Value::String(resp.content), confidence: resp.confidence }
        } else if resp.confidence >= threshold_uncertain {
            Self::Uncertain { value: Value::String(resp.content), confidence: resp.confidence }
        } else {
            Self::Hallucinated { raw: resp.content }
        }
    }
}
```

### 3.5 — Agent executor (`src/runtime/agent.rs`)

```rust
pub struct AgentProcess {
    pub name: String,
    pub state: HashMap<String, Value>,
    pub decl: AgentDecl,
    pub llm: Arc<dyn LLMBackend>,
    pub failure_policy: FailurePolicy,
}

impl AgentProcess {
    pub async fn send(&mut self, handle_name: &str, args: Vec<Value>) 
        -> anyhow::Result<Value>;
    
    async fn execute_think(&self, expr: &Expr, context: &ExecutionContext)
        -> anyhow::Result<UncertainValue>;
    
    async fn execute_match(&self, subject: UncertainValue, arms: &[MatchArm])
        -> anyhow::Result<Value>;
}
```

**Key:** when executing a `think` call, build the prompt from the expression context, call the LLM backend, wrap the response in `UncertainValue`. The `match` on `uncertain<T>` then dispatches to the correct arm based on the variant.

### 3.6 — Tracer (`src/tracer.rs`)

Every `think` call emits a structured JSON trace to stderr (or a trace file):

```json
{
  "step": "Greeter.greet.think_respond",
  "input_tokens": 312,
  "output_tokens": 45,
  "latency_ms": 890,
  "cost_usd": 0.00008,
  "confidence": 0.87,
  "model": "claude-haiku-4-5",
  "result_variant": "Certain",
  "timestamp": "2026-03-31T12:00:00Z"
}
```

---

## Phase 4 — Runtime: Plan + Parallelism

**Goal:** Execute a `plan` block where independent steps run in parallel via tokio.

**Deliverable:** `forge run examples/parallel_plan.forge` runs 3 searches concurrently and shows the timing difference vs sequential.

### 4.1 — Dependency graph builder (`src/runtime/plan.rs`)

```rust
pub struct DependencyGraph {
    pub nodes: HashMap<String, PlanStep>,
    pub edges: HashMap<String, Vec<String>>,  // step → steps it depends on
}

impl DependencyGraph {
    pub fn from_plan(plan: &PlanDecl) -> Self {
        // Walk each step's RHS expression
        // Collect all ident references that are also step names
        // Those are dependencies
        // Build adjacency list
    }
    
    pub fn execution_waves(&self) -> Vec<Vec<String>> {
        // Topological sort → return waves of steps that can run in parallel
        // Wave 0: steps with no dependencies
        // Wave 1: steps whose only dependencies are in wave 0
        // etc.
    }
}
```

### 4.2 — Plan executor

```rust
pub async fn execute_plan(
    plan: &PlanDecl,
    args: Vec<Value>,
    agent_registry: &AgentRegistry,
    llm: Arc<dyn LLMBackend>,
    tracer: &Tracer,
) -> anyhow::Result<Value> {
    let graph = DependencyGraph::from_plan(plan);
    let waves = graph.execution_waves();
    let mut results: HashMap<String, Value> = HashMap::new();
    
    for wave in waves {
        // Spawn all steps in this wave as tokio tasks
        let mut handles = vec![];
        for step_name in wave {
            let step = plan.steps.get(&step_name).unwrap();
            let step_args = resolve_args(step, &results);
            let handle = tokio::spawn(execute_step(step, step_args, llm.clone(), tracer.clone()));
            handles.push((step_name, handle));
        }
        // Await all in this wave before proceeding
        for (name, handle) in handles {
            results.insert(name, handle.await??);
        }
    }
    
    // Return value of last step
    results.get(&plan.final_step()).cloned().ok_or(...)
}
```

### 4.3 — Parallel plan test fixture (`examples/parallel_plan.forge`)

```forge
plan research(topic: String) -> String {
    // These three are independent — should run in parallel
    web_results   = search_web(topic)
    paper_results = search_papers(topic)
    news_results  = search_news(topic)

    // This depends on all three — waits
    synthesis = think synthesize(web_results, paper_results, news_results)

    synthesis
}

fn main() {
    let report = research("quantum computing breakthroughs 2025")
    print(report)
}
```

The tracer should show `web_results`, `paper_results`, `news_results` with overlapping timestamps and `synthesis` starting only after all three complete.

---

## Phase 5 — Runtime: Supervision Tree

**Goal:** Supervision tree with `one_for_one` and `rest_for_one` restart strategies. Simulate a crash and verify the correct agents restart.

**Deliverable:** `forge run examples/supervised.forge` shows an agent crash being recovered by the supervisor.

### 5.1 — Supervisor (`src/runtime/supervisor.rs`)

```rust
pub struct Supervisor {
    pub name: String,
    pub strategy: RestartStrategy,
    pub max_restarts: u32,
    pub period_secs: u64,
    pub children: Vec<ChildSpec>,
    restart_log: Vec<Instant>,
}

pub enum RestartStrategy { OneForOne, OneForAll, RestForOne }

impl Supervisor {
    pub async fn start(&mut self) -> anyhow::Result<()>;
    pub async fn handle_crash(&mut self, crashed_child: &str) -> anyhow::Result<()>;
    pub fn restart_targets(&self, crashed: &str) -> Vec<String>;
    // one_for_one: [crashed]
    // one_for_all: all children
    // rest_for_one: crashed + all children started after it
}
```

### 5.2 — Failure injection for testing

Add a `@fail_after(n)` annotation to agents for testing:

```forge
agent UnreliableSearcher {
    @fail_after(2)   // crashes after 2 successful calls — for POC testing only
    state { model: "claude-haiku-3" }
    on_hallucination: restart
    handle search(query: String) -> String {
        think web_search(query)
    }
}
```

### 5.3 — Supervised example (`examples/supervised.forge`)

```forge
agent Searcher {
    state { model: "claude-haiku-3" }
    on_hallucination: restart
    on_timeout: retry(max: 2) | fallback(FallbackSearcher)

    handle search(q: String) -> uncertain<String> {
        think find(q)
    }
}

agent FallbackSearcher {
    state { model: "claude-haiku-3" }
    handle search(q: String) -> uncertain<String> {
        certain("no results found")
    }
}

agent Summarizer {
    state { model: "claude-haiku-3" }
    handle summarize(text: String) -> uncertain<String> {
        think condense(text)
    }
}

supervise research_pipeline {
    agent searcher   = Searcher()
    agent summarizer = Summarizer()
    agent fallback   = FallbackSearcher()

    strategy: rest_for_one

    searcher |> channel<String> |> summarizer
}

fn main() {
    research_pipeline.start()
    research_pipeline.send("search", ["FORGE programming language agents"])
}
```

---

## Phase 6 — CLI + Polish

**Goal:** A usable CLI tool. Clean error messages. Trace output. Help text.

### 6.1 — CLI commands (`src/main.rs`)

```bash
forge parse  <file>              # Parse and print AST
forge check  <file>              # Type-check only
forge run    <file> [--trace]    # Execute with optional trace output
forge cost   <file>              # Estimate token cost (static analysis)
forge fleet  <spec>              # Stub: print what modules would be generated
```

### 6.2 — Error messages

FORGE errors should include:
- File name + line number + column
- The offending source line with a `^` pointer
- A plain-English explanation
- A suggestion when possible

```
error[E001]: uncertain value used without match
  --> examples/hello_agent.forge:8:20
   |
 8 |         let label: String = think categorize(text)
   |                             ^^^^^ this has type uncertain<String>
   |
   = help: wrap this in a match expression to handle all confidence levels
   = note: uncertain<T> cannot be assigned to T directly
```

### 6.3 — Environment variables

```bash
ANTHROPIC_API_KEY=sk-...   # Required for real LLM execution
FORGE_MOCK=1               # Use mock LLM (no API key needed)
FORGE_TRACE=1              # Enable trace output to stderr
FORGE_MODEL=claude-haiku-4-5  # Override default model
```

---

## Phase 7 — Stretch goals (do these if phases 1-6 are solid)

These are not required for the POC but are high-value if time allows:

### 7.1 — Context compaction
Implement the `sliding_window` compaction policy for `ConversationHistory`. After N turns, summarize the oldest M turns using a cheap LLM call and replace them with the summary.

### 7.2 — Token cost estimator
Static analysis pass that walks the plan DAG and estimates token cost per step based on input size heuristics. Output a cost table before execution.

### 7.3 — WASM compilation stub
Don't implement full LLVM IR generation — just demonstrate the pipeline:
- Parse a simple `plan` block
- Emit a skeleton Rust file that matches the plan structure
- Compile it with `rustc --target wasm32-wasi`
- This proves the "agent fleet → binary" concept even without full IR generation

### 7.4 — Agent interface checker
Verify at compile time that an agent declared as `implements Researcher` actually has all the `handle` methods required by the `Researcher` interface with matching signatures.

---

## Implementation notes for Claude Code

### Start here — minimum viable first run

The fastest path to something runnable:

1. `cargo new forge` + add dependencies
2. Write `grammar/forge.pest` for just `agent` + `handle` + basic expressions
3. Parse `examples/hello_agent.forge` into an AST
4. Execute the `handle greet` by calling Anthropic API directly (skip type checker)
5. Print the result

This gives you an end-to-end run in ~200 lines. Then layer in the type system, plan parallelism, and supervision on top.

### The single most important thing to get right

The `uncertain<T>` runtime value. Everything else in FORGE depends on this. Get the type, the match execution, and the confidence thresholds working correctly before anything else. If `uncertain<T>` is right, the rest of the language follows naturally.

### Confidence scoring in the POC

For the POC, use this simple heuristic (replace with logprobs later):
- Response contains hedging phrases ("I think", "might be", "possibly") → 0.6
- Response is short and direct → 0.88
- Response contains self-contradiction → 0.4 (detect via second LLM call)
- Default → 0.85

### Test without an API key

Set `FORGE_MOCK=1` to use the mock LLM backend. All tests in `tests/` should use the mock backend. Only integration tests need the real API key.

### Parallelism gotcha

When executing a `plan` wave with `tokio::spawn`, each spawned task needs its own clone of the LLM backend (use `Arc<dyn LLMBackend>`). The `AgentProcess` struct should not be `Send` if it contains non-Send fields — use `Arc<Mutex<AgentState>>` for state.

---

## Acceptance criteria for POC completion

The POC is done when all of these work:

- [ ] `forge parse examples/hello_agent.forge` → prints AST, no errors
- [ ] `forge check examples/uncertain_match.forge` → prints type error for unhandled uncertain
- [ ] `forge run examples/hello_agent.forge` → calls Claude, prints response
- [ ] `forge run examples/parallel_plan.forge` → shows parallel execution in trace
- [ ] `forge run examples/supervised.forge` → shows supervision restart in trace
- [ ] All unit tests pass with mock LLM (`FORGE_MOCK=1 cargo test`)
- [ ] Type checker catches: unhandled `uncertain<T>`, `restricted<T>` in log calls

---

## Reference: FORGE syntax summary

```forge
// Keywords
agent  think  plan  supervise  tool  context  interface  handle
fn  let  match  with  using  on_hallucination  on_timeout  on_cost_exceed

// Types
String  Bool  u64  f32  i64  Unit
uncertain<T>  restricted<T>  grounded<T, Source>
classified<T, ["a","b"]>  channel<T>  T[]

// Operators
|>   // pipe: left |> right  ≡  right(left)
=>   // match arm
->   // return type annotation
:    // type annotation on let bindings and params

// Match arms for uncertain<T>
match expr {
    certain(x) if x.confidence > 0.9 => ...,
    certain(x)                        => ...,
    uncertain(x)                      => ...,
    hallucinated(_)                   => ...,
    failed(reason)                    => ...,
    _                                 => ...   // catch-all
}

// Failure policies on agents
on_hallucination: restart | fallback(AgentName) | escalate
on_timeout:       retry(max: N) | fallback(AgentName) | fail
on_cost_exceed:   fallback(AgentName) | downgrade(model: "cheaper") | fail

// Restart strategies in supervise
strategy: one_for_one | one_for_all | rest_for_one
```

---

*FORGE Language POC Plan · v0.1 · Ready for Claude Code*
