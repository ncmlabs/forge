# FORGE Language — POC Implementation Plan v2

> Feed this to Claude Code. This is the implementation plan for a **genuinely new**
> programming language — not Elixir with agent keywords, not Go with `think`.
> FORGE is designed from the ground up so that agents can easily build on top of it.
>
> The question that drives every design decision:
> **"Would an LLM agent find this natural to read, write, and compose?"**

---

## The design philosophy reboot

Most "agent languages" make the same mistake: they take an existing language
and add agent primitives on top. The result feels like a human language with
costumes on.

FORGE starts from the opposite direction:

> **Agents are the primary citizens. Humans are welcome guests.**

This changes everything:

| Human-first language | Agent-first language |
|---|---|
| Syntax optimized for typing | Syntax optimized for generation + parsing |
| Errors explained for humans | Errors are data structures agents can act on |
| Libraries you import | Capabilities you declare and compose |
| State you manage | State that flows |
| You orchestrate | You describe intent, the runtime orchestrates |
| Fixed control flow | Adaptive control flow based on confidence |

The key insight: **an agent doesn't write code the way a human does**.
An agent generates structure. FORGE should feel like structured intent,
not structured text.

---

## Core design principles

### 1. Structure over syntax

FORGE has minimal punctuation. No curly braces, no semicolons, no colons on every line.
The structure comes from indentation + keywords, but the keywords are SEMANTIC, not
syntactic. Every keyword tells you WHAT KIND OF THING this is, not just where it starts.

```forge
task greet
  needs name: Text
  gives Text

  do
    say "hello {name}"
```

An agent reading this knows immediately: this is a task, it needs input, it produces output,
it does one thing. No parsing of brackets. No ambiguity.

### 2. Capabilities, not imports

You don't import libraries. You declare capabilities. The runtime resolves them.

```forge
use
  web.search
  llm.reason
  data.store
```

An agent composing a new task just declares what it needs. It doesn't need to know
which library, which version, which module path. The runtime figures it out.

### 3. Confidence is in the execution model, not the type system

Instead of a type `uncertain<T>` that wraps values, FORGE has a confidence-aware
execution model. Every value in FORGE has an implicit confidence level.
You query it when you need it, ignore it when you don't.

```forge
task analyze
  needs doc: Text
  gives Insight

  do
    result = reason "what is the main claim in {doc}"

    when result.sure        -> give result
    when result.unsure      -> give clarify(result)
    when result.unreliable  -> give ask_human(doc)
```

`sure`, `unsure`, `unreliable` are built-in confidence predicates.
No type annotations needed. No match expressions. The language reads like intent.

### 4. Composition is the primary abstraction

FORGE has no classes, no objects, no modules in the traditional sense.
The primary abstraction is **composition of tasks**.

```forge
task research_and_report
  is search >> summarize >> format_report
```

`>>` is the composition operator. Three tasks become one.
The runtime handles data passing, error propagation, and confidence tracking.

### 5. Failure is a first-class output, not an exception

Every task can produce a result OR a failure. Failures are values, not exceptions.
You handle them in the same place you handle results.

```forge
task safe_search
  needs query: Text
  gives Results or Failure

  do
    result = web.search(query)
    give result
  
  if fails
    give Failure("search unavailable", retry: true)
```

---

## The language at a glance

```forge
# FORGE v0.1 — everything in the language demonstrated

# --- Capabilities ---
use
  llm.reason
  llm.classify
  web.search
  data.embed

# --- A simple task ---
task greet
  needs name: Text
  gives Text
  do
    say "Hello, {name}"

# --- A task with confidence-aware output ---
task classify_intent
  needs message: Text
  gives Intent

  do
    result = classify message into ["buy", "support", "cancel", "other"]

    when result.sure(above: 0.85)  -> give result
    when result.sure               -> give result with flag("low-confidence")
    when result.unsure             -> give ask_for_clarification(message)
    else                           -> give Intent.unknown

# --- Composition: three tasks become one ---
task process_message
  is classify_intent >> route_to_handler >> send_response

# --- A flow: multi-step with named stages ---
flow research
  needs topic: Text
  gives Report

  stage gather
    web_results  = search topic
    paper_results = search "{topic} research paper"
    news         = search "{topic} news"
    # stages run in parallel by default — no annotation needed

  stage synthesize
    needs gather.*              # depends on everything from gather
    draft = reason "synthesize these sources into a report: {gather.*}"

  stage verify
    needs synthesize.draft
    checked = reason "fact-check this: {synthesize.draft}"
    give Report(checked)

# --- A pool: supervise multiple workers ---
pool search_workers
  workers: SearchAgent * 3        # 3 instances
  strategy: fastest               # return first result that succeeds
  fallback: CachedSearch          # if all fail

# --- An agent: stateful, long-running ---
agent support_bot
  memory
    history: Conversation
    user: Profile

  on message: Text
    intent = classify_intent(message)
    response = route_to_handler(intent, history: memory.history)
    memory.history = memory.history + (message, response)
    give response

  on reset
    memory.history = Conversation.empty

  if stuck            # agent hasn't made progress in 3 turns
    escalate to human

# --- A contract: what an agent must implement ---
contract Researcher
  can search(query: Text) -> Results
  can summarize(sources: Results) -> Summary

# --- Wire things together ---
system analytics_pipeline
  use
    ingestion: DataIngestor
    analysis:  Researcher
    reporting: ReportWriter

  ingestion >> analysis >> reporting
```

---

## What's genuinely new here

### The `flow` primitive

A `flow` is not a function and not a class. It's a named, multi-stage computation
where stages are the unit of parallelism. The compiler analyzes `needs` declarations
to build the dependency graph. Stages with no shared `needs` run in parallel.

This is more expressive than a plan/pipeline because:
- Stages are named and inspectable
- Data flows between stages by reference, not copying
- A flow can be paused, resumed, and inspected mid-execution
- Other agents can subscribe to stage outputs

### The `pool` primitive

A pool is not a supervisor. It's a declarative worker group with a resolution strategy.
`fastest` (return first success), `all` (wait for all), `majority` (return when >50% agree),
`quorum(n)` (return when n agree). This maps to how you'd actually use multiple LLMs.

```forge
pool fact_checkers
  workers: FactChecker * 5
  strategy: majority          # claim is true if 3+ agree
  timeout: 10s
```

### Confidence predicates, not confidence types

Instead of encoding confidence in the type system (verbose, hard to compose),
FORGE treats confidence as a runtime property of every value. You query it
with readable predicates:

```forge
when result.sure              # above default threshold (0.8)
when result.sure(above: 0.9)  # explicit threshold
when result.unsure            # below threshold
when result.unreliable        # multiple models disagree
when result.conflicted        # internal contradiction detected
```

These predicates work on any value, not just LLM outputs. A database query
can be `conflicted` if two replicas disagree.

### The `>>` composition chain

Every task is composable. The type system ensures the output of A matches
the input of B. But you don't annotate this explicitly — the compiler infers
and validates it.

```forge
# These all mean the same thing:
task pipeline is A >> B >> C

task pipeline
  is A >> B >> C

task pipeline
  do
    A >> B >> C

# Fan-out and fan-in:
task multi is (A | B | C) >> merge >> D
# A, B, C run in parallel, merge collects their outputs, D processes the merged result
```

### `if stuck` — adaptive control flow

FORGE has a built-in concept of "stuck" — when an agent or flow hasn't made
meaningful progress. This is automatically detected (repeated similar outputs,
loop detection, timeout). You declare what to do when it happens.

```forge
agent researcher
  if stuck for 3 turns
    try different_approach
  if stuck for 5 turns
    escalate to supervisor
  if stuck for 10 turns
    give Failure("could not complete research", context: memory)
```

### `memory` is structured, not a dictionary

Agent memory in FORGE is declared with a schema. The runtime handles:
- Persistence across calls
- Automatic summarization when memory gets large
- Context window management (only relevant memory enters the LLM context)

```forge
agent assistant
  memory
    conversation: Conversation   # auto-summarized when > 20 turns
    preferences:  UserPrefs      # always in context
    scratch:      Text           # evicted when not used for 5 turns
```

---

## FORGE vs the alternatives — syntax comparison

### Classifying user intent

**Python + LangChain:**
```python
from langchain.chat_models import ChatOpenAI
from langchain.schema import HumanMessage

llm = ChatOpenAI(model="gpt-4")
result = llm([HumanMessage(content=f"Classify: {message}")])
# result.content is a string — you parse it yourself
# no confidence, no structure, no safety
```

**FORGE:**
```forge
task classify_intent
  needs message: Text
  gives Intent

  do
    classify message into ["buy", "support", "cancel"]
```

No boilerplate. Structured output guaranteed. Confidence tracked automatically.

---

### Multi-step pipeline

**Python:**
```python
async def research(topic):
    web = await search_web(topic)       # sequential — you have to think about async
    papers = await search_papers(topic)  # or use asyncio.gather manually
    news = await search_news(topic)
    synthesis = await llm.complete(f"Synthesize: {web} {papers} {news}")
    return synthesis
```

**FORGE:**
```forge
flow research
  needs topic: Text

  stage gather
    web    = search topic
    papers = search "{topic} paper"
    news   = search "{topic} news"   # all three parallel, automatically

  stage synthesize
    needs gather.*
    reason "synthesize: {gather.*}"
```

Parallel by default. No async/await. The stages communicate intent, not mechanics.

---

### Handling failure

**Python:**
```python
try:
    result = llm.classify(text)
    if result.confidence < 0.8:
        # you have to remember to check this
        result = fallback_classify(text)
except Exception as e:
    # what do you do here?
    pass
```

**FORGE:**
```forge
task safe_classify
  needs text: Text

  do
    classify text into ["a", "b", "c"]

  when unsure
    try fallback_classifier(text)

  if fails
    give Classification.unknown
```

Failure and uncertainty are part of the task definition. Not bolted on.

---

## Project structure

```
forge/
├── Cargo.toml
├── grammar/
│   └── forge.pest              # PEG grammar
├── src/
│   ├── main.rs                 # CLI
│   ├── ast.rs                  # AST: Task, Flow, Agent, Pool, System, Contract
│   ├── parser.rs               # pest → AST
│   ├── resolver.rs             # capability resolution + composition type check
│   ├── planner.rs              # DAG builder from flow stages
│   ├── runtime/
│   │   ├── mod.rs
│   │   ├── executor.rs         # task + flow execution
│   │   ├── agent.rs            # stateful agent process
│   │   ├── pool.rs             # worker pool with strategies
│   │   ├── confidence.rs       # confidence model + predicates
│   │   └── memory.rs           # agent memory with auto-compaction
│   ├── llm/
│   │   ├── mod.rs              # LLMBackend trait
│   │   ├── anthropic.rs        # Anthropic API
│   │   └── mock.rs             # deterministic mock for tests
│   └── tracer.rs               # structured execution traces
├── examples/
│   ├── hello.forge             # simplest task
│   ├── classify.forge          # confidence-aware classification
│   ├── research.forge          # multi-stage flow with parallelism
│   ├── support_bot.forge       # stateful agent with memory
│   └── fact_check_pool.forge   # pool with majority strategy
└── tests/
```

---

## AST design

The AST should reflect the language's primitives directly:

```rust
// src/ast.rs

pub enum TopLevel {
    Use(UseDecl),
    Task(TaskDecl),
    Flow(FlowDecl),
    Agent(AgentDecl),
    Pool(PoolDecl),
    System(SystemDecl),
    Contract(ContractDecl),
}

// task greet
//   needs name: Text
//   gives Text
//   do ...
pub struct TaskDecl {
    pub name: String,
    pub needs: Vec<Param>,
    pub gives: Vec<OutputType>,      // can give multiple types (T or Failure)
    pub body: TaskBody,
    pub failure_handler: Option<FailureHandler>,
}

pub enum TaskBody {
    Do(Vec<Stmt>),                   // imperative do block
    Is(CompositionExpr),             // declarative: is A >> B >> C
}

// flow research
//   needs topic: Text
//   stages: [gather, synthesize, verify]
pub struct FlowDecl {
    pub name: String,
    pub needs: Vec<Param>,
    pub gives: Vec<OutputType>,
    pub stages: Vec<StageDecl>,
}

pub struct StageDecl {
    pub name: String,
    pub needs: Vec<StageNeed>,       // needs gather.* or needs stage.field
    pub body: Vec<Stmt>,
}

// agent support_bot
//   memory { ... }
//   on message: Text ...
//   if stuck ...
pub struct AgentDecl {
    pub name: String,
    pub implements: Option<String>,  // contract name
    pub memory: Vec<MemoryField>,
    pub handlers: Vec<MessageHandler>,
    pub stuck_policy: Option<StuckPolicy>,
}

// pool fact_checkers
//   workers: FactChecker * 3
//   strategy: majority
pub struct PoolDecl {
    pub name: String,
    pub worker: String,
    pub count: u32,
    pub strategy: PoolStrategy,
    pub timeout: Option<Duration>,
    pub fallback: Option<String>,
}

pub enum PoolStrategy {
    Fastest,
    All,
    Majority,
    Quorum(u32),
    First(u32),    // first N to agree
}

// Expressions
pub enum Expr {
    Literal(Literal),
    Ident(String),
    Template(Vec<TemplatePart>),      // "hello {name}"
    Call(String, Vec<Expr>),
    Reason(Box<Expr>),                // reason "..."
    Classify(Box<Expr>, Vec<String>), // classify x into ["a","b"]
    Search(Box<Expr>),                // search "query"
    Compose(Box<Expr>, Box<Expr>),    // A >> B
    FanOut(Vec<Expr>),                // (A | B | C)
    Await(Box<Expr>),                 // explicit await (usually implicit)
    FieldAccess(Box<Expr>, String),   // result.sure, gather.web
    GlobAccess(Box<Expr>),            // gather.*
}

// Confidence predicates
pub enum ConfidencePred {
    Sure(Option<f32>),       // .sure or .sure(above: 0.9)
    Unsure,
    Unreliable,
    Conflicted,
    Custom(f32, f32),        // between(0.6, 0.8)
}

// when/else branching (replaces match on uncertain<T>)
pub struct WhenClause {
    pub predicate: ConfidencePred,
    pub body: Vec<Stmt>,
}

// Statements
pub enum Stmt {
    Bind(String, Expr),              // name = expr
    Give(Expr),                      // give result
    Say(Expr),                       // say "message" (print)
    When(Vec<WhenClause>, Option<Vec<Stmt>>),  // when/else
    Try(Expr, Option<Expr>),         // try X or Y
    Escalate(EscalateTarget),        // escalate to human/supervisor
    MemoryUpdate(String, Expr),      // memory.field = expr
}
```

---

## Grammar sketch

```pest
// grammar/forge.pest

WHITESPACE = _{ " " | "\t" }
NEWLINE    = _{ "\n" | "\r\n" }
COMMENT    = _{ "#" ~ (!NEWLINE ~ ANY)* }
indent     = _{ "  " }   // 2-space indent

// Identifiers and literals
ident   = @{ ASCII_ALPHA ~ (ASCII_ALPHANUMERIC | "_")* }
text_lit = @{ "\"" ~ (!"\"" ~ ANY)* ~ "\"" }
num_lit  = @{ ASCII_DIGIT+ ~ ("." ~ ASCII_DIGIT+)? }
template = ${ "\"" ~ (template_expr | template_text)* ~ "\"" }
template_expr = !{ "{" ~ expr ~ "}" }
template_text = @{ (!("\"" | "{") ~ ANY)+ }

// Types
type_expr = { "Text" | "Number" | "Bool" | "Conversation"
            | "Profile" | "Results" | "Report" | "Intent"
            | "Summary" | "Failure" | "Classification" | ident }

// Capability use
use_decl = { "use" ~ NEWLINE ~ (indent ~ cap_path ~ NEWLINE)+ }
cap_path = @{ ident ~ ("." ~ ident)* }

// Task
task_decl = {
    "task" ~ ident ~ NEWLINE ~
    (indent ~ "needs" ~ param_list ~ NEWLINE)? ~
    (indent ~ "gives" ~ output_type ~ NEWLINE)? ~
    (indent ~ "is" ~ compose_expr ~ NEWLINE |
     indent ~ "do" ~ NEWLINE ~ stmt_block) ~
    (indent ~ "when" ~ when_block)* ~
    (indent ~ "if" ~ "fails" ~ NEWLINE ~ stmt_block)?
}

// Composition
compose_expr = { compose_term ~ (">>" ~ compose_term)* }
compose_term = { "(" ~ compose_expr ~ ("|" ~ compose_expr)* ~ ")"
               | ident ~ call_args? }

// Flow
flow_decl = {
    "flow" ~ ident ~ NEWLINE ~
    (indent ~ "needs" ~ param_list ~ NEWLINE)? ~
    (indent ~ "gives" ~ output_type ~ NEWLINE)? ~
    stage_decl+
}
stage_decl = {
    indent ~ "stage" ~ ident ~ NEWLINE ~
    (indent{2} ~ "needs" ~ needs_list ~ NEWLINE)? ~
    stmt_block
}
needs_list = { needs_item ~ ("," ~ needs_item)* }
needs_item = { ident ~ "." ~ ("*" | ident) }

// Agent
agent_decl = {
    "agent" ~ ident ~ NEWLINE ~
    (indent ~ "memory" ~ NEWLINE ~ memory_block)? ~
    msg_handler+ ~
    (indent ~ "if" ~ "stuck" ~ stuck_policy)*
}

// Pool
pool_decl = {
    "pool" ~ ident ~ NEWLINE ~
    indent ~ "workers" ~ ":" ~ ident ~ "*" ~ num_lit ~ NEWLINE ~
    indent ~ "strategy" ~ ":" ~ pool_strategy ~ NEWLINE ~
    (indent ~ "timeout" ~ ":" ~ duration)? ~
    (indent ~ "fallback" ~ ":" ~ ident)?
}
pool_strategy = { "fastest" | "all" | "majority" | "quorum(" ~ num_lit ~ ")" }

// Statements
stmt       = { bind_stmt | give_stmt | say_stmt | when_stmt | try_stmt | escalate_stmt }
bind_stmt  = { ident ~ "=" ~ expr ~ NEWLINE }
give_stmt  = { "give" ~ expr ~ NEWLINE }
say_stmt   = { "say" ~ expr ~ NEWLINE }
try_stmt   = { "try" ~ expr ~ ("or" ~ expr)? ~ NEWLINE }
escalate_stmt = { "escalate" ~ "to" ~ ident ~ NEWLINE }
when_stmt  = { when_clause+ ~ (indent ~ "else" ~ NEWLINE ~ stmt_block)? }
when_clause = { indent ~ "when" ~ conf_pred ~ "->" ~ stmt ~ NEWLINE }

// Confidence predicates
conf_pred = {
    "result.sure(above:" ~ num_lit ~ ")" |
    "result.sure" | "result.unsure" | "result.unreliable" |
    "result.conflicted" | ident ~ "." ~ ident
}

// Expressions
expr = { compose_expr | call_expr | reason_expr | classify_expr
       | search_expr | template | text_lit | num_lit | ident }
reason_expr   = { "reason" ~ expr }
classify_expr = { "classify" ~ expr ~ "into" ~ "[" ~ text_list ~ "]" }
search_expr   = { "search" ~ expr }
call_expr     = { ident ~ "(" ~ arg_list? ~ ")" }
```

---

## Phase 1 — Core parsing

**Goal:** `forge parse examples/hello.forge` prints AST.

### Implement in order:

1. `use` declarations
2. `task` with `do` block (no composition yet)
3. Basic statements: bind, give, say
4. String templates: `"hello {name}"`
5. Builtins: `reason`, `search`, `classify ... into`
6. `when`/`else` confidence predicates
7. `task is A >> B >> C` composition form
8. `flow` with stages and `needs` references
9. `agent` with memory and `on` handlers
10. `pool` declaration
11. `system` wiring

### Minimum test file (`examples/hello.forge`):

```forge
task greet
  needs name: Text
  gives Text

  do
    say "Hello, {name}!"
```

### Second test (`examples/classify.forge`):

```forge
use
  llm.classify

task classify_intent
  needs message: Text
  gives Text

  do
    result = classify message into ["buy", "support", "cancel", "other"]

    when result.sure        -> give result
    when result.unsure      -> give "unclear"
    else                    -> give "unknown"
```

---

## Phase 2 — Capability resolver

**Goal:** Resolve `use` declarations to runtime capabilities. Type-check compositions.

### Capability registry (`src/resolver.rs`)

```rust
pub struct Capability {
    pub name: String,               // "llm.reason"
    pub inputs: Vec<ForgeType>,
    pub outputs: Vec<ForgeType>,
    pub cost_estimate: CostHint,
}

pub struct CapabilityRegistry {
    pub caps: HashMap<String, Capability>,
}

impl CapabilityRegistry {
    pub fn builtin() -> Self {
        // Register built-in capabilities:
        // llm.reason: Text -> Text
        // llm.classify: (Text, Labels) -> Classification
        // web.search: Text -> Results
        // data.store: (Key, Value) -> ()
        // data.retrieve: Key -> Value
    }
    
    pub fn resolve(&self, name: &str) -> Option<&Capability>;
    
    pub fn check_composition(&self, a: &str, b: &str) -> Result<ForgeType, TypeError>;
    // output type of A must be compatible with input type of B
}
```

### Composition type checking

The `>>` operator requires that output type of left side matches input type of right side.
For the POC, be permissive: `Text >> anything` always works. Add strict checking later.

```rust
pub fn check_compose(left: ForgeType, right: &TaskDecl) -> Result<ForgeType, TypeError> {
    // For POC: if left is Text and right needs Text, ok
    // Return right's output type
    // Error if types clearly incompatible
}
```

---

## Phase 3 — Execution: tasks and confidence

**Goal:** Execute tasks. `reason` and `classify` call the real Anthropic API.
Confidence predicates work. `give` returns values. `say` prints.

### Confidence model (`src/runtime/confidence.rs`)

```rust
#[derive(Debug, Clone)]
pub struct ConfidentValue {
    pub value: Value,
    pub confidence: f32,
    pub source: ConfidenceSource,
}

pub enum ConfidenceSource {
    LLMDirect(f32),           // from model logprobs or heuristic
    ConsensusAgreement(f32),  // multiple agents agreed
    Deterministic,            // from code, not LLM — always 1.0
    Derived(f32),             // propagated from upstream value
}

impl ConfidentValue {
    pub fn sure(&self) -> bool { self.confidence >= 0.8 }
    pub fn sure_above(&self, threshold: f32) -> bool { self.confidence >= threshold }
    pub fn unsure(&self) -> bool { self.confidence >= 0.5 && self.confidence < 0.8 }
    pub fn unreliable(&self) -> bool { self.confidence < 0.5 }
    pub fn conflicted(&self) -> bool {
        matches!(self.source, ConfidenceSource::ConsensusAgreement(f) if f < 0.6)
    }
}
```

### Built-in capability implementations

```rust
// reason "summarize this: {text}"
// → calls Anthropic API, returns ConfidentValue
async fn builtin_reason(prompt: ConfidentValue, llm: &dyn LLMBackend) 
    -> anyhow::Result<ConfidentValue>;

// classify text into ["a", "b", "c"]  
// → system prompt constrains output to one of the labels
// → confidence is higher because output space is bounded
async fn builtin_classify(
    text: ConfidentValue, 
    labels: Vec<String>,
    llm: &dyn LLMBackend
) -> anyhow::Result<ConfidentValue>;

// search "query"
// → stub: returns mock results for POC
// → real implementation: web search API
async fn builtin_search(query: ConfidentValue) -> anyhow::Result<ConfidentValue>;
```

### Task executor

```rust
pub struct TaskExecutor {
    pub registry: CapabilityRegistry,
    pub llm: Arc<dyn LLMBackend>,
    pub tracer: Tracer,
}

impl TaskExecutor {
    pub async fn run_task(
        &self,
        task: &TaskDecl,
        args: HashMap<String, ConfidentValue>,
    ) -> anyhow::Result<ConfidentValue> {
        let mut scope = Scope::new(args);
        
        match &task.body {
            TaskBody::Do(stmts) => self.run_stmts(stmts, &mut scope).await,
            TaskBody::Is(compose) => self.run_compose(compose, &mut scope).await,
        }
    }
    
    async fn run_stmt(&self, stmt: &Stmt, scope: &mut Scope) -> anyhow::Result<Option<ConfidentValue>>;
    async fn run_when(&self, clauses: &[WhenClause], subject: &ConfidentValue, scope: &mut Scope) -> anyhow::Result<Option<ConfidentValue>>;
    async fn eval_expr(&self, expr: &Expr, scope: &Scope) -> anyhow::Result<ConfidentValue>;
}
```

---

## Phase 4 — Flow execution with automatic parallelism

**Goal:** Execute a `flow`. Stages with no shared `needs` run concurrently.

### Stage dependency analysis (`src/planner.rs`)

```rust
pub struct FlowPlanner;

impl FlowPlanner {
    pub fn dependency_graph(flow: &FlowDecl) -> DependencyGraph {
        // For each stage, collect its `needs` declarations
        // needs gather.*   → depends on stage "gather"
        // needs synthesize.draft → depends on stage "synthesize"
        // No needs declaration → depends on nothing → runs in wave 0
        
        // Build: stage_name → Vec<stage_name it depends on>
    }
    
    pub fn execution_waves(graph: &DependencyGraph) -> Vec<Vec<String>> {
        // Kahn's algorithm topological sort
        // Returns: [[stage_a, stage_b], [stage_c], [stage_d]]
        // Stages in the same inner Vec run in parallel
    }
}
```

### Flow executor

```rust
pub async fn execute_flow(
    flow: &FlowDecl,
    args: HashMap<String, ConfidentValue>,
    executor: &TaskExecutor,
) -> anyhow::Result<ConfidentValue> {
    let graph = FlowPlanner::dependency_graph(flow);
    let waves = FlowPlanner::execution_waves(&graph);
    
    let mut stage_outputs: HashMap<String, HashMap<String, ConfidentValue>> = HashMap::new();
    
    for wave in waves {
        let handles: Vec<_> = wave.into_iter().map(|stage_name| {
            let stage = flow.stage(&stage_name);
            let inputs = resolve_stage_inputs(stage, &stage_outputs);
            let exec = executor.clone();
            tokio::spawn(async move {
                exec.run_stage(stage, inputs).await
            })
        }).collect();
        
        for (stage_name, handle) in wave.iter().zip(handles) {
            stage_outputs.insert(stage_name.clone(), handle.await??);
        }
    }
    
    // Return the output of the last stage
    resolve_flow_output(flow, &stage_outputs)
}
```

### Test (`examples/research.forge`):

```forge
use
  llm.reason
  web.search

flow research
  needs topic: Text
  gives Text

  stage gather
    web    = search topic
    papers = search "{topic} academic paper"
    news   = search "{topic} latest news"

  stage synthesize
    needs gather.*
    draft = reason "synthesize into a brief report: {gather.web} {gather.papers} {gather.news}"

  stage verify
    needs synthesize.draft
    give reason "is this factually consistent? if not, correct it: {synthesize.draft}"
```

The tracer should show `gather.web`, `gather.papers`, `gather.news` with overlapping timestamps.

---

## Phase 5 — Agent with memory

**Goal:** A stateful agent that retains conversation history across messages.

### Memory model (`src/runtime/memory.rs`)

```rust
pub struct AgentMemory {
    pub fields: HashMap<String, MemoryField>,
}

pub struct MemoryField {
    pub value: Value,
    pub schema: MemorySchema,
    pub access_count: u32,
    pub last_accessed: Instant,
}

pub enum MemorySchema {
    Conversation { max_turns: u32, compaction: CompactionStrategy },
    Always,          // always in context (preferences, profile)
    Evictable { ttl_unused_turns: u32 },  // scratch pad
}

impl AgentMemory {
    pub fn to_context_string(&self, budget_tokens: u32) -> String {
        // Serialize memory fields into a compact context string
        // Respect token budget — evict Evictable fields first
        // Summarize Conversation if over max_turns
    }
}
```

### Agent process

```rust
pub struct AgentProcess {
    pub name: String,
    pub decl: AgentDecl,
    pub memory: AgentMemory,
    pub executor: TaskExecutor,
    pub stuck_detector: StuckDetector,
}

impl AgentProcess {
    pub async fn handle(&mut self, event: &str, payload: ConfidentValue)
        -> anyhow::Result<ConfidentValue>;
    
    async fn check_stuck(&mut self, response: &ConfidentValue) -> bool;
    // Stuck = last 3 responses are semantically similar (cosine sim > 0.95)
    // Or: response confidence has been < 0.5 for 3 turns
    
    async fn apply_stuck_policy(&mut self) -> anyhow::Result<ConfidentValue>;
}
```

### Test (`examples/support_bot.forge`):

```forge
use
  llm.reason
  llm.classify

agent support_bot
  memory
    history: Conversation
    user: Profile

  on message: Text
    intent = classify message into ["billing", "technical", "general", "complaint"]
    
    response = reason "
      You are a support agent. User history: {memory.history}
      User said: {message}
      Their intent is: {intent}
      Give a helpful, concise response.
    "
    
    memory.history = memory.history + (message, response)
    give response

  on reset
    memory.history = Conversation.empty

  if stuck for 3 turns
    give "I'm having trouble helping with this. Let me connect you with a human agent."
```

---

## Phase 6 — Pool with strategies

**Goal:** Multiple workers, `fastest` and `majority` strategies.

### Pool executor (`src/runtime/pool.rs`)

```rust
pub struct Pool {
    pub decl: PoolDecl,
    pub workers: Vec<AgentProcess>,
}

impl Pool {
    pub async fn send(&mut self, event: &str, payload: ConfidentValue)
        -> anyhow::Result<ConfidentValue> {
        match &self.decl.strategy {
            PoolStrategy::Fastest => self.run_fastest(event, payload).await,
            PoolStrategy::Majority => self.run_majority(event, payload).await,
            PoolStrategy::All => self.run_all(event, payload).await,
            PoolStrategy::Quorum(n) => self.run_quorum(*n, event, payload).await,
        }
    }
    
    async fn run_fastest(&mut self, event: &str, payload: ConfidentValue)
        -> anyhow::Result<ConfidentValue> {
        // Race all workers — return first ConfidentValue where .sure()
        let mut handles = vec![];
        for worker in &mut self.workers {
            handles.push(tokio::spawn(worker.handle(event, payload.clone())));
        }
        // Use tokio::select! or futures::future::select_ok
        // Return first success, cancel the rest
    }
    
    async fn run_majority(&mut self, ...) -> anyhow::Result<ConfidentValue> {
        // Run all workers, wait for majority
        // If responses are semantically similar → high confidence
        // If responses conflict → conflicted confidence
        // Use embedding similarity or simple string comparison for POC
    }
}
```

### Test (`examples/fact_check_pool.forge`):

```forge
use
  llm.reason

pool fact_checkers
  workers: FactChecker * 3
  strategy: majority
  timeout: 15s

task FactChecker
  needs claim: Text
  gives Text

  do
    result = reason "Is this claim factually accurate? Answer yes or no and explain briefly: {claim}"

    when result.sure        -> give result
    when result.unsure      -> give "uncertain"
    else                    -> give "could not verify"

fn main
  pool = fact_checkers()
  verdict = pool.send("check", "The speed of light is 299,792 km/s")
  say "Verdict: {verdict}"
```

---

## Phase 7 — CLI

```bash
forge run    <file>                 # Execute
forge parse  <file>                 # Print AST
forge check  <file>                 # Resolve + type check
forge trace  <file>                 # Execute with full trace to stderr
forge cost   <file>                 # Estimate token cost
```

Environment:
```bash
ANTHROPIC_API_KEY=sk-...
FORGE_MOCK=1              # Use mock LLM
FORGE_TRACE=1             # Structured JSON traces to stderr
FORGE_MODEL=claude-haiku-4-5-20251001  # Override model
```

---

## What to build first — minimum viable FORGE

```
Day 1: Parse hello.forge → print AST
Day 2: Execute reason and say → prints LLM response
Day 3: when/else confidence predicates work
Day 4: >> composition chains work
Day 5: flow with parallel stages works
Day 6: agent with memory works
Day 7: pool with majority strategy works
```

Each day is runnable and demonstrable. Each builds on the prior.

---

## The single most important design invariant

Every primitive in FORGE should be composable with `>>`.

```forge
task A >> task B             # works
flow X >> task B             # works
agent Y >> flow X            # works (sends message to agent, pipes output)
pool P >> task B             # works (pool output flows into task)
```

This is what makes FORGE "easy to build on top of." Any FORGE construct is a
composable unit. New primitives added in future versions automatically work with
the existing ones because they all speak the same interface: `ConfidentValue` in,
`ConfidentValue` out.

That's the foundation. Everything else is built on top of it.

---

*FORGE Language POC Plan v2 · Agent-native design · Ready for Claude Code*
