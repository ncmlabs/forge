# forge-sensei

A self-referential learning agent written in FORGE to teach FORGE.

forge-sensei is the flagship application for the FORGE programming language. It demonstrates that FORGE is expressive enough to build a non-trivial learning system — one that progressively masters its own language through knowledge accumulation, confidence-gated reasoning, and supervised multi-agent coordination.

forge-sensei is both a **showcase** (every major FORGE construct in one program) and a **production tool** (integrated into Claude Code for real-time code review and question answering).

## Architecture

forge-sensei is structured as a multi-file FORGE project in `workflows/forge-sensei/`:

```
workflows/forge-sensei/
  core.forge    — Types, events, states, pure functions, tasks, flows, contract
  agent.forge   — Sensei agent, specialist agent, warden
  web.forge     — HTTP endpoints for web UI
```

The component stack follows FORGE's natural declaration order:

```
┌─ Types ──────────────────────────┐
│  QueryResult, AssessmentResult   │
├─ Events ─────────────────────────┤
│  LearnedInsight                  │
│  AssessmentCompleted             │
│  KnowledgeGapFound               │
├─ States ─────────────────────────┤
│  MasteryLevel (4 levels)         │
│  SpecialistPhase (2 levels)      │
├─ Pure Functions ─────────────────┤
│  compute_mastery_score           │
│  determine_level                 │
│  format_status                   │
│  check_prediction                │
├─ Tasks ──────────────────────────┤
│  answer_forge_question           │
│  review_forge_code               │
│  extract_lesson                  │
│  predict_outcome                 │
│  categorize_for_recall           │
├─ Flows ──────────────────────────┤
│  answer_query                    │
│  review_code                     │
├─ Contract ───────────────────────┤
│  ForgeTutor                      │
├─ Agents ─────────────────────────┤
│  forge_sensei (exportable)       │
│  specialist (spawnable)          │
├─ Warden ─────────────────────────┤
│  sensei_warden                   │
└──────────────────────────────────┘
```

## Types & Events

FORGE types are named records with typed fields. Events are broadcast messages that agents can emit and subscribe to.

```forge
type QueryResult
  answer: Text
  confidence_tier: Text
  sources_used: Number

type AssessmentResult
  passed: Bool
  prediction: Text
  expected: Text
  gap_topic: Text
```

Three events drive the learning system:

```forge
event LearnedInsight
  category: Text
  content: Text
  source: Text
  confidence: Number

event AssessmentCompleted
  level: Text
  score: Number
  passed: Number
  total: Number
  failures: Text

event KnowledgeGapFound
  category: Text
  question: Text
```

- `LearnedInsight` — Emitted whenever sensei or a specialist learns something. Other agents subscribe to this for knowledge propagation.
- `AssessmentCompleted` — Emitted after batch assessment with the score and level.
- `KnowledgeGapFound` — Emitted when a recall query returns low confidence, signaling a knowledge gap that needs filling.

## State Machines

FORGE `states` declarations define finite state machines with conditional transition guards. The runtime enforces that transitions only happen along declared edges.

```forge
states MasteryLevel
  novice -> apprentice when conformance_score >= 40
  apprentice -> journeyman when conformance_score >= 70
  journeyman -> expert when conformance_score >= 90
  expert -> expert
```

forge-sensei progresses through 4 mastery levels based on conformance test scores:

| Level | Score | Unlocks |
|-------|-------|---------|
| novice | 0-39% | Basic query, ingestion |
| apprentice | 40-69% | Code review, assessment |
| journeyman | 70-89% | Deep dive, specialist spawning |
| expert | 90%+ | Full capabilities, self-loop |

Specialists have a simpler 2-state lifecycle:

```forge
states SpecialistPhase
  learning -> ready when absorbed_count >= 5
  ready -> ready
```

A specialist must absorb at least 5 `LearnedInsight` events before it can answer queries.

## Pure Functions

`pure` functions are deterministic — no LLM calls, no side effects, no I/O. The compiler enforces this: using `reason`, `classify`, `exec`, `learn`, `recall`, `spawn`, `find`, `emit`, `escalate`, or `try/or` inside a pure function is a compile error.

```forge
pure compute_mastery_score
  needs passed: Number, total: Number
  gives Number
  do
    if total == 0
      give 0
    give (passed / total) * 100

pure determine_level
  needs score: Number
  gives Text
  do
    if score >= 90
      give "expert"
    if score >= 70
      give "journeyman"
    if score >= 40
      give "apprentice"
    give "novice"

pure format_status
  needs level: Text, score: Number, interactions: Number
  gives Text
  do
    give "forge-sensei | Level: {level} | Mastery: {score}% | Interactions: {interactions}"

pure check_prediction
  needs predicted: Text, expected: Text
  gives Bool
  do
    if predicted.contains(expected)
      give true
    give false
```

Pure functions are the foundation of FORGE's honesty guarantee: their results are always `sure` (confidence = 1.0), never uncertain.

## Tasks

Tasks are the primary compute unit. They can call LLM primitives (`reason`, `classify`, `search`) and must dispatch on confidence using `when/else`.

### answer_forge_question

Answers a FORGE question using prior knowledge context. Note the three-branch confidence dispatch — this is idiomatic FORGE:

```forge
task answer_forge_question
  needs question: Text, context: Text
  gives Text
  do
    result = reason "You are forge-sensei... Answer this FORGE question: {question}\n\nContext: {context}"
    when result.sure -> give result
    when result.unsure -> give "Uncertain: {result}"
    else -> give "I could not answer this question confidently. Consider checking the forge-reference."
```

### review_forge_code

Reviews FORGE code for correctness, style, and principle violations:

```forge
task review_forge_code
  needs code: Text, context: Text
  gives Text
  do
    result = reason "You are forge-sensei, reviewing FORGE code... Review this code:\n{code}"
    when result.sure -> give result
    when result.unsure -> give "Uncertain review: {result}"
    else -> give "Could not review this code confidently."
```

### categorize_for_recall

Uses `classify` to categorize a query into one of 12 knowledge categories:

```forge
task categorize_for_recall
  needs context: Text
  gives Text
  do
    result = classify context into ["SYNTAX", "TASKS", "FLOWS", "CONTROL", "AGENTS",
      "KNOWLEDGE", "SUPERVISION", "PRINCIPLES", "PATTERNS", "ERRORS", "CONFORMANCE", "BOUNDARY"]
    when result.sure -> give result
    when result.unsure -> give result
    else -> give "GENERAL"
```

### predict_outcome

Predicts compiler outcomes for conformance assessment:

```forge
task predict_outcome
  needs test_input: Text, knowledge_context: Text
  gives Text
  do
    result = reason "Given this FORGE program:\n{test_input}\n\nPredict the EXACT compiler outcome:
      parse_ok, parse_error, compile_ok, compile_error, run_ok, or run_error."
    when result.sure -> give result
    when result.unsure -> give "UNCERTAIN: {result}"
    else -> give "CANNOT_PREDICT"
```

### extract_lesson

Distills a reusable lesson from a developer interaction:

```forge
task extract_lesson
  needs question: Text, resolution: Text
  gives Text
  do
    result = reason "Extract a concise, reusable lesson about FORGE from this interaction.
      Format: [CATEGORY] lesson text."
    when result.sure -> give result
    when result.unsure -> give "Tentative lesson: {result}"
    else -> give "Could not extract lesson."
```

## Flows

Flows are multi-stage DAG pipelines. Each stage can depend on outputs from previous stages via `needs stage.field`. Stages without dependencies run in parallel.

### answer_query

The query pipeline: categorize the question → retrieve relevant knowledge → respond with confidence gating.

```forge
flow answer_query
  needs question: Text
  gives QueryResult

  stage categorize
    categories = question >> categorize_for_recall(_pipe)

  stage retrieve
    needs categorize.categories
    prior = recall "{categorize.categories} {question}"

  stage respond
    needs retrieve.prior, categorize.categories
    prior = retrieve.prior
    response = answer_forge_question(question, prior)
    when prior.sure -> give QueryResult(answer: response, confidence_tier: "sure", sources_used: 1)
    when prior.unsure -> give QueryResult(answer: response, confidence_tier: "unsure", sources_used: 1)
    else -> give QueryResult(answer: response, confidence_tier: "none", sources_used: 0)
```

Key FORGE constructs demonstrated:
- `>>` pipe operator: passes question through categorize_for_recall
- `recall` with template string: retrieves from knowledge store
- `when/else` on `prior`: dispatches on recall confidence
- Type constructor: `QueryResult(answer: ..., confidence_tier: ..., sources_used: ...)`

### review_code

The code review pipeline:

```forge
flow review_code
  needs code: Text
  gives Text

  stage categorize
    categories = code >> categorize_for_recall(_pipe)

  stage retrieve
    needs categorize.categories
    prior = recall "{categorize.categories} FORGE syntax patterns errors"

  stage review
    needs retrieve.prior
    prior = retrieve.prior
    when prior.sure -> give review_forge_code(code, prior)
    when prior.unsure -> give review_forge_code(code, prior)
    else -> give review_forge_code(code, "No prior knowledge. Review from first principles.")
```

## Contract

Contracts declare the public API of an exportable agent. Any agent implementing `ForgeTutor` must handle these messages:

```forge
contract ForgeTutor
  can query(question: Text) -> QueryResult
  can review(code: Text) -> Text
  can status() -> Text
  can assess_detailed(test_input: Text, expected: Text) -> AssessmentResult
```

## The Sensei Agent

The main agent is declared as `exportable` (can be built into a standalone binary) with lifecycle, persistent memory, knowledge store, and timer:

```forge
exportable agent forge_sensei
  lifecycle: MasteryLevel
  memory persistent
    interaction_count: Number
    success_count: Number
    last_assessment_score: Number
    current_level: Text
    gap_count: Number
  knowledge store: ".forge-knowledge/sensei"
    max_entries: 50000
    retention: 365d
  timer self_assess: 6h
```

### Handlers

**start** — Initializes memory and starts the self-assessment timer:

```forge
on start
  memory.current_level = "novice"
  start self_assess
  say "forge-sensei initialized. Level: novice. Ready to learn and teach FORGE."
```

**ingest** — Bulk document ingestion for pre-training:

```forge
on ingest(document_path: Text)
  learn from document(document_path)
  say "Ingested: {document_path}"
```

**query** — Answers questions via the `answer_query` flow, then learns from the interaction:

```forge
on query(question: Text)
  memory.interaction_count = memory.interaction_count + 1
  result = answer_query(question)
  learn from interaction(question, result.answer, 0.7)
  emit LearnedInsight(category: "interactions", content: result.answer, source: "query", confidence: 0.7)
  give result
```

Every query has three effects: (1) answer the question, (2) store the Q&A pair as knowledge, (3) emit an event for specialists to absorb. Knowledge compounds with every interaction.

**review** — Code review gated by mastery level:

```forge
on review(code: Text)
  requires lifecycle == apprentice or lifecycle == journeyman or lifecycle == expert
    on fail: give "Sensei is still at novice level. Code review unlocks at apprentice."
  memory.interaction_count = memory.interaction_count + 1
  give review_code(code)
```

**deep_dive** — Spawns a specialist sub-agent for focused domain learning:

```forge
on deep_dive(topic: Text)
  requires lifecycle == journeyman or lifecycle == expert
    on fail: give "Deep dive requires journeyman level."
  existing = find "specialist_{topic}"
  if existing
    say "Specialist for [{topic}] already active."
    give existing
  child = spawn specialist as "specialist_{topic}"
    with knowledge where category == topic
    with memory topic: topic
  say "Spawned specialist for [{topic}]."
  emit LearnedInsight(category: topic, content: "Specialist spawned", source: "deep_dive", confidence: 0.9)
  give child
```

Key constructs: `find` discovers existing agents by alias, `spawn` creates a new agent with filtered knowledge transfer (`where category == topic`) and injected memory.

**batch_assess** — Iterates test arrays with state transitions:

```forge
on batch_assess(tests: Text[], expectations: Text[])
  passed = 0
  total = 0
  for test in tests
    total = total + 1
    result = assess_detailed(test, expectations[total - 1])
    if result.passed
      passed = passed + 1
  score = compute_mastery_score(passed, total)
  memory.last_assessment_score = score
  level = determine_level(score)
  if score >= 90 and lifecycle != expert
    transition to expert
  if score >= 70 and lifecycle == apprentice
    transition to journeyman
  if score >= 40 and lifecycle == novice
    transition to apprentice
  emit AssessmentCompleted(level: level, score: score, passed: passed, total: total, failures: gaps)
```

**self_assess.expired** — Timer-triggered periodic self-assessment:

```forge
on self_assess.expired
  say "Self-assess timer fired. Run 'assess.sh' for full mastery evaluation."
  reset self_assess
```

**stuck detection** — Escalates after 3 consecutive stuck turns:

```forge
if stuck for 3 turns
  say "forge-sensei is stuck. Escalating for human guidance."
  escalate to human
```

## Specialist Agent

Specialists are spawned by sensei for focused domain learning. They subscribe to `LearnedInsight` events filtered by their topic:

```forge
agent specialist
  lifecycle: SpecialistPhase
  memory
    topic: Text
    query_count: Number
    absorbed_count: Number
  knowledge store: ".forge-knowledge/specialist"
    max_entries: 10000
    retention: 180d
  subscribe LearnedInsight where category == memory.topic

  on LearnedInsight(category: Text, content: Text, source: Text, confidence: Number)
    learn "{content}" category: category
    memory.absorbed_count = memory.absorbed_count + 1
    emit LearnedInsight(category: category, content: "Specialist [{memory.topic}] absorbed: {content}",
      source: "specialist", confidence: confidence)

  on query(question: Text)
    requires lifecycle == ready
      on fail: give "Specialist still learning. Queries available after absorbing 5 insights."
    prior = recall "{memory.topic} {question}"
    when prior.sure -> give answer_forge_question(question, prior)
    when prior.unsure -> give answer_forge_question(question, prior)
    else -> give "Specialist [{memory.topic}] has no knowledge on this. Escalating."
```

The specialist pattern demonstrates:
- **Filtered knowledge transfer**: `spawn specialist with knowledge where category == topic` — only relevant knowledge is transferred
- **Event subscription**: `subscribe LearnedInsight where category == memory.topic` — specialists only receive events matching their domain
- **Lifecycle gating**: Must absorb 5 insights before queries unlock
- **Recursive knowledge**: Specialists emit their own `LearnedInsight` events, which can be absorbed by other specialists or sensei itself

## Warden

The warden supervises both agents with failure policies and escalation ladders:

```forge
warden sensei_warden
  manages [forge_sensei, specialist]
  on stuck: nudge, self
    after 3: escalate
  on hallucination: restart, self
  on contradiction: nudge, self
    after 2: escalate
  on crash: restart, self
  on timeout: restart, self
  on budget: nudge, self
    after 1: escalate
  max_retries 3 per 1h then escalate
```

Each failure type has a response ladder:
- **stuck**: nudge first, escalate after 3 occurrences
- **hallucination**: immediate restart
- **contradiction**: nudge first, escalate after 2
- **budget**: nudge once, then escalate immediately
- **Circuit breaker**: max 3 retries per hour before escalating

## Knowledge System

### Store Configuration

```forge
knowledge store: ".forge-knowledge/sensei"
  max_entries: 50000
  retention: 365d
```

The knowledge store persists as JSON at the specified path with a 50,000 entry limit and 365-day retention. When the limit is reached, least-recently-used entries are evicted.

### Learning Modes

forge-sensei learns through 4 channels:

| Mode | FORGE syntax | Confidence | Use case |
|------|-------------|------------|----------|
| Direct | `learn "fact"` | 1.0 | Pre-training facts |
| Interaction | `learn from interaction(q, a, 0.7)` | Variable | Q&A pairs from queries |
| Document | `learn from document(path)` | 1.0 | Bulk file ingestion (500-word chunks) |
| Categorized | `learn "fact" category: "TASKS"` | 1.0 | Categorized pattern injection |

### TF-IDF Recall

Knowledge retrieval uses TF-IDF (Term Frequency × Inverse Document Frequency) scoring:

```forge
prior = recall "{categories} {question}"
```

The tokenizer splits text on non-alphanumeric characters, lowercases, and filters tokens shorter than 2 characters. Queries should include category-specific keywords for best results.

Recall returns a `ConfidentValue` — the result MUST be dispatched with `when/else`:

```forge
when prior.sure -> ...    # Strong match (high TF-IDF score)
when prior.unsure -> ...  # Partial match
else -> ...               # No relevant knowledge found
```

Using a recall result without `when/else` dispatch is a compile error (Principle I: Honesty).

### Knowledge Categories

12 categories organize knowledge for filtered retrieval:

| Category | Domain |
|----------|--------|
| SYNTAX | Language grammar, indentation, keywords |
| TASKS | Task declarations, primitives |
| FLOWS | Flow stages, parallelism, composition |
| CONTROL | when/else, requires, try/or |
| AGENTS | Agent declaration, handlers, lifecycle |
| KNOWLEDGE | Knowledge store, recall, learn |
| SUPERVISION | Wardens, policies, escalation |
| PRINCIPLES | 9 first-principles design rules |
| PATTERNS | Common idioms and best practices |
| ERRORS | Compile errors, runtime failures |
| CONFORMANCE | Compliance test outcomes |
| BOUNDARY | Boundary system semantics (server vs pure) |

### The Compounding Property

forge-sensei's knowledge store is a **persistent, compounding artifact**. Unlike flat RAG systems that re-derive understanding from scratch on every query, sensei's knowledge is compiled once and kept current:

- Every query adds a `learn from interaction` entry
- Every session resolution adds a lesson via `extract_lesson`
- Every specialist absorption emits a `LearnedInsight` event
- Pre-training provides the seed; interaction provides the growth

The more sensei is used, the richer its knowledge becomes. Cross-references accumulate naturally through categorized recall.

## Pre-training Pipeline

Two scripts provide the initial knowledge base:

### pretrain-sensei.sh — Breadth

Ingests whole documents across 6 phases:

| Phase | Sources |
|-------|---------|
| 1 | Core docs: forge-reference.md, forge-principles.md, README.md, roadmap.md |
| 2 | Example programs: examples/*.forge |
| 3 | Workflows: workflows/*.forge (excluding sensei itself) |
| 4 | Conformance tests: conformance/**/*.json (as categorized facts) |
| 5 | Design specifications: docs/superpowers/specs/*.md |
| 6 | Key source modules: checker/*.rs, knowledge_store.rs |

Uses SHA-256 manifest for idempotency — skips if sources haven't changed. Run with `--force` to re-ingest.

```bash
bash scripts/pretrain-sensei.sh [--force] [--dry-run]
```

### pretrain-toolkit.sh — Depth

Ingests ~65 curated code patterns across 6 categories optimized for code generation:

| Phase | Category | Facts | Purpose |
|-------|----------|-------|---------|
| 1 | TASKS | ~16 | Task/pure declarations, LLM calls, exec, compose |
| 2 | FLOWS | ~8 | Multi-stage pipelines, wave parallelism |
| 3 | AGENTS | ~10 | Agent lifecycle, memory, events, timers, guards |
| 4 | SYSTEMS | ~11 | System wiring, pools, wardens, contracts |
| 5 | ERRORS | ~12 | Compiler error → fix mappings |
| 6 | TESTING | ~8 | Conformance test structure and templates |

Each fact is a keyword-rich, self-contained pattern with syntax template, working example, and key rules. Optimized for TF-IDF recall by toolkit agents.

```bash
bash scripts/pretrain-toolkit.sh [--force] [--dry-run] [--verify-only]
```

## Knowledge Categories for Code Generation

The 6 curriculum categories map directly to the toolkit agents that will consume them:

| Category | Toolkit Agent | Recall Query Pattern |
|----------|---------------|---------------------|
| TASKS | TaskGenerator #169 | `recall "TASKS FORGE task declaration needs gives"` |
| FLOWS | FlowGenerator #170 | `recall "FLOWS FORGE flow parallel stages wave"` |
| AGENTS | AgentGenerator #171 | `recall "AGENTS FORGE agent lifecycle memory"` |
| SYSTEMS | SystemAssembler #172 | `recall "SYSTEMS FORGE system warden composition"` |
| ERRORS | RepairAgent #173 | `recall "ERRORS FORGE pure function compiler error"` |
| TESTING | TestGenerator #174 | `recall "TESTING FORGE conformance test structure"` |

### How Toolkit Agents Query Knowledge

A toolkit agent uses `recall` with category keywords to retrieve relevant patterns:

```forge
# In a future TaskGenerator agent:
on generate(spec: Text)
  patterns = recall "TASKS {spec}"
  when patterns.sure
    code = reason "Write a FORGE task for: {spec}\n\nKnown patterns:\n{patterns}"
    give code
  when patterns.unsure
    code = reason "Write a FORGE task for: {spec}\n\nPartial patterns:\n{patterns}"
    emit KnowledgeGapFound(category: "TASKS", question: spec)
    give code
  else
    give "No patterns found for this task type."
```

## Continuous Learning

The pre-training curriculum provides seed knowledge. Continuous learning channels compound knowledge over time:

| Channel | Trigger | Effect |
|---------|---------|--------|
| Query learning | Every `on query` | Stores Q&A pair as interaction knowledge |
| Session learning | `learn_from_session` | Distills lesson from resolved developer issue |
| Event propagation | `LearnedInsight` | Knowledge flows between sensei and specialists |
| Webhook ingestion | HTTP POST to `/webhook_ingest` | External systems push categorized facts |
| Self-assessment | 6h timer | Evaluates knowledge health, identifies gaps |
| Toolkit feedback | Future (#167) | Failed/successful code generation feeds back |

The learning cycle: **Pre-train → Query → Learn from interaction → Assess → Identify gaps → Fill gaps → Re-assess**. Each iteration makes the next query more accurate.

## Claude Code Integration

### Skill Commands

forge-sensei is integrated as a Claude Code skill at `~/.claude/skills/forge-sensei/SKILL.md`:

| Command | Description |
|---------|-------------|
| `/forge-sensei query "question"` | Answer a FORGE question |
| `/forge-sensei review "file_path"` | Review FORGE code |
| `/forge-sensei status` | Show mastery level and interaction count |
| `/forge-sensei assess` | Run full conformance assessment |
| `/forge-sensei learn "question" "resolution"` | Teach from a resolved issue |
| `/forge-sensei deep-dive "topic"` | Spawn specialist for domain learning |
| `/forge-sensei pretrain` | Run full pre-training pipeline |
| `/forge-sensei health` | Verify installation end-to-end |

### Consult Hook

The `consult-sensei.sh` hook fires automatically when editing `.forge` files in Claude Code. It reads the file content, queries sensei for review advice, and returns suggestions with a 5-minute cache (SHA-256 content-based).

## Web Interface

forge-sensei includes a web UI with DaisyUI styling, served via FORGE's built-in HTTP server:

```forge
#! boundary: server

endpoint status() -> Html
  level = determine_level(0)
  card = "<div class=\"card bg-base-100 shadow-xl\">...</div>"
  give render_page("forge-sensei | Status", card)

endpoint ask_form() -> Html
  form = render_ask_form()
  give render_page("forge-sensei | Ask", form)

endpoint ask(question: Text) -> Html
  result = answer_query(question)
  body = render_answer(result.answer, result.confidence_tier)
  give render_page("forge-sensei | Answer", body)

endpoint review_form() -> Html
endpoint review(code: Text) -> Html

endpoint webhook_ingest(category: Text, fact: Text) -> Text
endpoint webhook_learn(question: Text, resolution: Text) -> Text
```

Run the web UI:

```bash
forge serve workflows/forge-sensei/ --watch
# Opens at http://localhost:3030
```

## Testing

forge-sensei is tested at 3 layers:

### Layer 1: Conformance JSON Tests

Location: `conformance/{parser,checker,runtime}/sensei_*.json`

| Category | Tests | Examples |
|----------|-------|---------|
| Parser valid | 34 | Full program parse, type/event/states/pure/task/flow/contract/agent declarations |
| Parser error | 6 | Invalid timer unit, malformed spawn, missing fields |
| Checker valid | 20 | Pure function purity, recall dispatch, states reachability, warden resolution |
| Checker error | 14 | Pure violations (reason/classify/learn/recall/spawn/find/emit), invalid transitions |
| Runtime | 33 | Pure function results, task confidence paths, flow stage execution, agent traces |
| Error format | 3 | Error messages contain expected identifiers |

### Layer 2: Rust Integration Tests

| File | Tests | Focus |
|------|-------|-------|
| `tests/sensei_parser_tests.rs` | 8 | AST structure, handler counts, warden policies |
| `tests/sensei_checker_tests.rs` | 10 | Zero errors, purity exhaustive, recall taint |
| `tests/sensei_runtime_tests.rs` | 25 | Agent lifecycle, knowledge persistence, spawning, events, timers, state transitions, warden |
| `tests/sensei_knowledge_tests.rs` | 8 | Entry limits, retention, category recall, confidence propagation |

### Layer 3: E2E Shell Tests

`scripts/sensei-e2e-test.sh` runs 60+ tests across 9 categories against the built binary:

1. Basic functionality (help, status, ingest, fact learning)
2. Intelligence quality (syntax knowledge, code review, hallucination detection)
3. Learning & evolution (fact recall, session learning, knowledge growth)
4. Knowledge persistence (JSON validity, marker persistence)
5. Specialist spawning (deep-dive, repeated spawning)
6. Error handling (missing files, empty queries, special characters)
7. Trace & debug (FORGE_TRACE, FORGE_LOG_LEVEL)
8. Performance (query <30s, ingest <5s, status <5s)
9. Full pipeline (smoke test, cache stats)

## Running forge-sensei

### Build

```bash
bash scripts/build-sensei.sh
# Uses: forge build workflows/forge-sensei/ -o bin/forge-sensei
```

### Pre-train

```bash
# Breadth: whole documents
bash scripts/pretrain-sensei.sh

# Depth: categorized code patterns
bash scripts/pretrain-toolkit.sh

# Force re-ingestion
bash scripts/pretrain-sensei.sh --force

# Dry run (list files without ingesting)
bash scripts/pretrain-toolkit.sh --dry-run
```

### Query

```bash
bin/forge-sensei query "how do flows work in FORGE?"
bin/forge-sensei query "what is the difference between task and pure?"
```

### Review

```bash
bin/forge-sensei review "pure bad_fn\n  do\n    reason 'hello'"
# Flags: purity violation — reason is not allowed in pure functions
```

### Status

```bash
bin/forge-sensei status
# forge-sensei | Level: apprentice | Mastery: 45% | Interactions: 127
```

### Assess

```bash
bash ~/.claude/skills/forge-sensei/assess.sh
# Runs conformance tests, reports per-category scores, tracks trends
```

### Deep Dive

```bash
bin/forge-sensei deep-dive "SYNTAX"
# Spawns specialist_SYNTAX with filtered knowledge
```

### Serve

```bash
forge serve workflows/forge-sensei/ --watch
# Web UI at http://localhost:3030
```

## How Toolkit Agents Consume Knowledge

The knowledge transfer path from sensei to toolkit agents:

```
Pre-training (seed)
    │
    ▼
forge-sensei knowledge store (50K entries, 12 categories)
    │
    ├─ recall "TASKS ..." ─── TaskGenerator (#169)
    ├─ recall "FLOWS ..." ─── FlowGenerator (#170)
    ├─ recall "AGENTS ..." ── AgentGenerator (#171)
    ├─ recall "SYSTEMS ..." ─ SystemAssembler (#172)
    ├─ recall "ERRORS ..." ── RepairAgent (#173)
    └─ recall "TESTING ..." ─ TestGenerator (#174)
    │
    ▼
SpecAnalyzer (#175) — orchestrates all generators
```

Each toolkit agent queries sensei's knowledge store with category-prefixed recalls. The TF-IDF index ensures category-specific terms rank highest, returning the most relevant code patterns for generation.

When a toolkit agent generates code that compiles successfully, it emits a `LearnedInsight` back to sensei, reinforcing the pattern. Failed generations emit to the ERRORS category, creating error→fix mappings that prevent the same mistakes. This creates a self-improving loop where the curriculum grows with use.

---

*forge-sensei is part of FORGE Phase 2 — Milestone 11: Knowledge School. See [roadmap.md](../roadmap.md) for the full development plan.*
