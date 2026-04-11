#!/usr/bin/env bash
# pretrain-toolkit.sh — Categorized code-pattern curriculum for forge-sensei
#
# Ingests ~65 curated FORGE code patterns across 6 categories for code generation.
# Each pattern is keyword-rich and optimized for TF-IDF recall by toolkit agents.
#
# Usage: bash scripts/pretrain-toolkit.sh [--force] [--dry-run] [--verify-only]
set -euo pipefail

FORGE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SENSEI_BIN="${SENSEI_BIN:-$FORGE_ROOT/bin/forge-sensei}"

# Auto-detect mock mode
if [ "${FORGE_MOCK:-}" = "1" ] && [ -z "${FORGE_CONFIG:-}" ]; then
  export FORGE_CONFIG="$FORGE_ROOT/config/mock.config.toml"
fi
MANIFEST_FILE="$FORGE_ROOT/.forge-knowledge/toolkit-manifest.sha256"
FORCE=false
DRY_RUN=false
VERIFY_ONLY=false

for arg in "$@"; do
  case "$arg" in
    --force) FORCE=true ;;
    --dry-run) DRY_RUN=true ;;
    --verify-only) VERIFY_ONLY=true ;;
  esac
done

if [ "$VERIFY_ONLY" = true ]; then
  # Skip straight to verification
  PHASE=7
fi

if [ ! -x "$SENSEI_BIN" ] && [ "$DRY_RUN" = false ] && [ "$VERIFY_ONLY" = false ]; then
  echo "Error: forge-sensei binary not found at $SENSEI_BIN"
  echo "Run: bash scripts/build-sensei.sh"
  exit 1
fi

# ── Counters ─────────────────────────────────────────────────
COUNT=0
TOTAL=63
FAILED=0
FAIL_LOG=()
PHASE_START=$SECONDS

# ── Helpers ──────────────────────────────────────────────────

ingest_fact() {
  local category="$1"
  local fact="$2"
  COUNT=$((COUNT + 1))
  if [ "$DRY_RUN" = true ]; then
    local preview="${fact:0:80}"
    printf "  [%d/%d] [%s] %s...\n" "$COUNT" "$TOTAL" "$category" "$preview"
    return
  fi
  local err
  if err=$("$SENSEI_BIN" ingest-fact "$category" "$fact" 2>&1); then
    printf "  [%d/%d] [%s] OK\n" "$COUNT" "$TOTAL" "$category"
  else
    FAILED=$((FAILED + 1))
    FAIL_LOG+=("FAIL: [$category] -- $err")
    printf "  [%d/%d] [%s] FAIL\n" "$COUNT" "$TOTAL" "$category"
  fi
}

phase_time() {
  local now=$SECONDS
  local elapsed=$((now - PHASE_START))
  PHASE_START=$now
  echo "  (${elapsed}s)"
}

# ── Idempotency check ────────────────────────────────────────
if [ "$FORCE" = false ] && [ "$DRY_RUN" = false ] && [ "$VERIFY_ONLY" = false ]; then
  CURRENT_MANIFEST=$(shasum -a 256 "$0" 2>/dev/null | cut -d' ' -f1)
  if [ -f "$MANIFEST_FILE" ]; then
    CACHED_MANIFEST=$(cat "$MANIFEST_FILE")
    if [ "$CURRENT_MANIFEST" = "$CACHED_MANIFEST" ]; then
      echo "Toolkit curriculum up to date (script unchanged since last pretrain)."
      echo "Use --force to re-ingest, --verify-only to check recall."
      exit 0
    fi
  fi
fi

if [ "$VERIFY_ONLY" = false ]; then
  echo "=== forge-sensei Toolkit Curriculum ==="
  echo "Categories: TASKS, FLOWS, AGENTS, SYSTEMS, ERRORS, TESTING"
  echo "Total patterns: ~$TOTAL"
  echo ""
fi

# ══════════════════════════════════════════════════════════════
# Phase 1: TASKS — Atomic compute units
# ══════════════════════════════════════════════════════════════

if [ "${VERIFY_ONLY:-false}" = false ]; then

echo "Phase 1: TASKS..."

ingest_fact "TASKS" "$(cat <<'FACT'
FORGE basic task declaration pattern. A task is the primary compute unit with typed inputs and outputs.
Syntax:
  task <name>
    needs <param>: <Type>
    gives <Type>
    do
      give <expression>
Example:
  task greet
    needs name: Text
    gives Text
    do
      give "Hello, {name}!"
Key rules:
  - needs declares typed parameters
  - gives declares return type (Text, Number, Bool, Html, or custom type)
  - do begins the body
  - give returns a value (like return)
  - Template strings use {variable} for interpolation
Related: see FLOWS for composing tasks into pipelines
FACT
)"

ingest_fact "TASKS" "$(cat <<'FACT'
FORGE task with reason LLM call pattern. Tasks can invoke LLMs via the reason primitive for natural language generation.
Syntax:
  task <name>
    needs <param>: Text
    gives Text
    do
      result = reason "<prompt template with {param}>"
      when result.sure -> give result
      when result.unsure -> give "Uncertain: {result}"
      else -> give "<fallback>"
Example:
  task analyze_quality
    needs code: Text, language: Text
    gives Text
    do
      result = reason "Analyze this {language} code for quality issues:\n\n{code}"
      when result.sure -> give result
      when result.unsure -> give result
      else -> give "Quality review inconclusive."
Key rules:
  - reason sends a prompt to the LLM and returns an uncertain value
  - The result MUST be dispatched with when/else (Principle I: Honesty)
  - when result.sure triggers on high confidence
  - when result.unsure triggers on medium confidence
  - else triggers on low confidence or failure
  - Requires: use llm.reason at the top of the file
Related: see ERRORS for what happens if you skip when/else dispatch
FACT
)"

ingest_fact "TASKS" "$(cat <<'FACT'
FORGE task with classify LLM call pattern. Classify categorizes input into one of several labels.
Syntax:
  result = classify <input> into [<labels>]
  when result.sure -> give result
  when result.unsure -> give "<fallback>"
  else -> give "<default>"
Example:
  task classify_intent
    needs message: Text
    gives Text
    do
      result = classify message into ["buy", "support", "cancel", "other"]
      when result.sure -> give result
      when result.unsure -> give "unclear"
      else -> give "unknown"
Key rules:
  - classify takes an input and array of label strings
  - Returns an uncertain value requiring when/else dispatch
  - Labels should be mutually exclusive and collectively exhaustive
  - Requires: use llm.classify at the top of the file
Related: see categorize_for_recall in forge-sensei for a 12-label classify example
FACT
)"

ingest_fact "TASKS" "$(cat <<'FACT'
FORGE confidence dispatch pattern. All LLM operations (reason, classify, search, recall, exec) return uncertain values that must be dispatched.
Syntax:
  result = reason "<prompt>"
  when result.sure -> <action>
  when result.unsure -> <action>
  else -> <action>
Example:
  result = reason "Analyze {topic}"
  when result.sure -> give result
  when result.unsure
    emit KnowledgeGapFound(category: "GENERAL", question: topic)
    give "Uncertain: {result}"
  else
    escalate to human
Key rules:
  - Three confidence tiers: sure (high), unsure (medium), else (low/failure)
  - Omitting when/else on an uncertain value is a compile error
  - Each branch can contain multiple statements (multi-line block)
  - The -> arrow syntax is for single-line branches
  - sure/unsure can have different behaviors (not required to be identical)
  - Pure functions never produce uncertain values (always sure)
Related: see ERRORS for unhandled uncertain compile errors
FACT
)"

ingest_fact "TASKS" "$(cat <<'FACT'
FORGE task with exec shell command pattern. Tasks can execute shell commands via the exec primitive.
Syntax:
  result = exec "<shell command>"
  when result.sure -> give result
  when result.unsure -> give "<fallback>"
  else -> give "<error message>"
Example:
  task get_recent_commits
    needs repo_path: Text
    gives Text
    do
      result = exec "git -C {repo_path} log --oneline -10"
      when result.sure -> give result
      when result.unsure -> give result
      else -> give "Failed to retrieve commits"
Key rules:
  - exec runs a shell command and returns an uncertain value
  - Template strings work inside exec for dynamic commands
  - Result must be dispatched with when/else
  - exec is a boundary operation (only in server context)
Related: see compose pattern for exec >> reason pipes
FACT
)"

ingest_fact "TASKS" "$(cat <<'FACT'
FORGE compose pipe operator pattern. The >> pipe passes output from one operation as input to the next.
Syntax:
  result = <operation1> >> <operation2>(_pipe)
Example:
  task analyze_velocity
    needs repo_path: Text
    gives Text
    do
      result = exec "git -C {repo_path} log --oneline -30" >> reason "Analyze commit velocity from:\n{_pipe}"
      when result.sure -> give result
      when result.unsure -> give result
      else -> give "Analysis failed."
Key rules:
  - >> chains operations left to right
  - _pipe is a special variable referring to the output of the left side
  - Common pattern: exec >> reason (shell output analyzed by LLM)
  - The final result carries the confidence of the last operation
Related: see FLOWS for multi-stage pipelines (flows are like multi-step pipes)
FACT
)"

ingest_fact "TASKS" "$(cat <<'FACT'
FORGE task with skill bridge pattern. Tasks can call external skills registered in the project.
Syntax:
  result = skill.<name>.<method>(<args>)
Example:
  task create_issue
    needs title: Text, body: Text
    gives Text
    do
      result = skill.github.create_issue(title, body)
      when result.sure -> give result
      else -> give "Failed to create issue."
Key rules:
  - Skills are declared in forge.project.toml or via use skill.<name>
  - Skill calls return uncertain values (external I/O)
  - Each skill method maps to a CLI command or API call
  - Skills are pluggable — not built into the language
Related: see SYSTEMS for how skills integrate into larger agent systems
FACT
)"

ingest_fact "TASKS" "$(cat <<'FACT'
FORGE pure function pattern. Pure functions are deterministic — no LLM calls, no side effects, no I/O. The compiler enforces purity.
Syntax:
  pure <name>
    needs <param>: <Type>
    gives <Type>
    do
      <deterministic logic>
      give <result>
Example:
  pure compute_mastery_score
    needs passed: Number, total: Number
    gives Number
    do
      if total == 0
        give 0
      give (passed / total) * 100
Key rules:
  - Cannot use: reason, classify, search, exec, recall, learn, spawn, find, emit, escalate, try/or
  - Results are always sure (confidence = 1.0)
  - Can call other pure functions
  - Can use: if/else, for loops, string operations, arithmetic
  - Ideal for formatting, validation, scoring, rendering
Related: see ERRORS for compile errors when violating purity
FACT
)"

ingest_fact "TASKS" "$(cat <<'FACT'
FORGE pure function with conditional chains pattern. Pure functions use cascading if/give for multi-branch logic.
Syntax:
  pure <name>
    needs <param>: <Type>
    gives <Type>
    do
      if <condition1>
        give <value1>
      if <condition2>
        give <value2>
      give <default>
Example:
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
Key rules:
  - Each if/give acts as an early return
  - Order matters: most specific conditions first
  - Final give is the default fallback
  - No else keyword needed — just chain if blocks
Related: see pure function basic pattern for simpler examples
FACT
)"

ingest_fact "TASKS" "$(cat <<'FACT'
FORGE pure function with string operations pattern. Pure functions can use built-in string methods for text processing.
Syntax:
  pure <name>
    needs <param>: Text
    gives <Type>
    do
      if param.contains(<substring>)
        give true
      give false
Example:
  pure check_prediction
    needs predicted: Text, expected: Text
    gives Bool
    do
      if predicted.contains(expected)
        give true
      give false
Key rules:
  - String methods: .contains(), .len(), .lower(), .upper(), .trim()
  - Array methods: .none(), .all_same, .first
  - Template strings: "text {variable} more text"
  - Bool values: true, false
Related: see pure function HTML pattern for rendering helpers
FACT
)"

ingest_fact "TASKS" "$(cat <<'FACT'
FORGE pure function HTML rendering pattern. Pure functions returning Html type are used for deterministic page rendering.
Syntax:
  pure <name>
    needs <param>: Text
    gives Html
    do
      give "<div>{param}</div>"
Example:
  pure render_page
    needs title: Text, body: Html
    gives Html
    do
      nav = render_nav()
      content = "<div class=\"container\">{!nav}{!body}</div>"
      give html.layout(title, content)
Key rules:
  - Html is a distinct type from Text
  - {!variable} is raw HTML interpolation (no escaping)
  - {variable} is escaped interpolation (safe for user content)
  - html.layout(title, body) wraps in full HTML document structure
  - Pure HTML functions compose: one calls another
Related: see SYSTEMS endpoint pattern for serving HTML via HTTP
FACT
)"

ingest_fact "TASKS" "$(cat <<'FACT'
FORGE task composition pattern. Tasks can call other tasks to compose complex behavior.
Syntax:
  task <outer>
    needs <params>
    gives <Type>
    do
      intermediate = <inner_task>(<args>)
      result = <another_task>(intermediate, <more_args>)
      give result
Example:
  task judge
    needs topic: Text, round: Number
    gives Text
    do
      arg_for = argue_for(topic)
      arg_against = argue_against(topic)
      score_for = score_argument(arg_for, topic)
      score_against = score_argument(arg_against, topic)
      verdict = format_verdict(score_for, score_against)
      give verdict
Key rules:
  - Tasks can call other tasks like regular functions
  - Each task call may involve LLM calls (reason, classify)
  - The calling task inherits uncertainty from its callees
  - Pure functions can also be called from tasks (always sure)
Related: see FLOWS for declarative multi-stage composition with parallelism
FACT
)"

ingest_fact "TASKS" "$(cat <<'FACT'
FORGE use declaration pattern. The use block imports language primitives at the top of a file.
Syntax:
  use
    llm.reason
    llm.classify
    web.search
Example:
  use
    llm.reason
    llm.classify

  task analyze
    needs input: Text
    gives Text
    do
      result = reason "Analyze: {input}"
      when result.sure -> give result
      else -> give "unknown"
Key rules:
  - use must appear at the top of the file before any declarations
  - Available primitives: llm.reason, llm.classify, web.search
  - Only import what you actually use
  - Pure functions don't need use declarations (no LLM primitives)
Related: see boundary directives in SYSTEMS for server/client/shared contexts
FACT
)"

ingest_fact "TASKS" "$(cat <<'FACT'
FORGE fn main entry point pattern. Programs use fn main as the entry point for CLI execution.
Syntax:
  fn main
    result = <task_call>(<args>)
    say result
Example:
  task greet
    needs name: Text
    gives Text
    do
      give "Hello, {name}!"

  fn main
    result = greet("World")
    say result
Key rules:
  - fn main is the program entry point
  - say prints output to stdout
  - fn main can call tasks, pure functions, and flows
  - Not needed for agent-based programs (agents use handlers)
  - Not needed for web endpoints (endpoints handle HTTP)
Related: see AGENTS for event-driven entry points instead of fn main
FACT
)"

ingest_fact "TASKS" "$(cat <<'FACT'
FORGE template string interpolation pattern. Template strings embed variables and expressions using curly braces.
Syntax:
  text = "Hello, {name}!"
  html = "<div>{!raw_html}</div>"
  multi = "Line 1: {var1}\nLine 2: {var2}"
Example:
  pure format_status
    needs level: Text, score: Number, interactions: Number
    gives Text
    do
      give "forge-sensei | Level: {level} | Mastery: {score}% | Interactions: {interactions}"
Key rules:
  - {variable} embeds a variable with HTML escaping (safe for user input)
  - {!variable} embeds raw HTML without escaping (for trusted content)
  - \n creates newlines in strings
  - Works in reason prompts, say output, give values, and learn content
  - Nested interpolation is not supported
Related: see pure function patterns for common formatting examples
FACT
)"

ingest_fact "TASKS" "$(cat <<'FACT'
FORGE task with web search pattern. Tasks can search the web using the search primitive.
Syntax:
  result = search <query>
Example:
  task gather_research
    needs topic: Text
    gives Text
    do
      web_results = search topic
      papers = search "{topic} academic paper"
      news = search "{topic} latest news"
      give "{web_results}\n{papers}\n{news}"
Key rules:
  - search sends a web query and returns uncertain results
  - Must dispatch with when/else
  - Requires: use web.search at the top of the file
  - Returns aggregated search results as Text
  - Commonly used in research flows with parallel gather stages
Related: see FLOWS research flow pattern for parallel search composition
FACT
)"

phase_time

# ══════════════════════════════════════════════════════════════
# Phase 2: FLOWS — Multi-stage DAG pipelines
# ══════════════════════════════════════════════════════════════

echo "Phase 2: FLOWS..."

ingest_fact "FLOWS" "$(cat <<'FACT'
FORGE flow with parallel stages and DAG dependencies pattern. Flows are multi-stage pipelines where stages run in parallel unless they declare dependencies.
Syntax:
  flow <name>
    needs <param>: <Type>
    gives <Type>

    stage <name1>
      <computation>

    stage <name2>
      <computation>

    stage <name3>
      needs <name1>.<field>, <name2>.<field>
      <computation using dependencies>
      give <result>
Example:
  flow review_code
    needs code: Text
    gives Text

    stage detect
      lang = detect_language(code)

    stage quality
      needs detect.lang
      report = analyze_quality(code, detect.lang)

    stage security
      needs detect.lang
      report = analyze_security(code, detect.lang)

    stage verdict
      needs detect.lang, quality.report, security.report
      give format_verdict(detect.lang, quality.report, security.report)
Key rules:
  - Stages without needs run in parallel
  - needs stage.field declares a dependency on a previous stage's output
  - The last stage's give is the flow's return value
  - quality and security run in parallel (both only need detect)
  - verdict waits for all three to complete
  - Flow parameters are available in all stages
Related: see TASKS for the individual task declarations used in flow stages
FACT
)"

ingest_fact "FLOWS" "$(cat <<'FACT'
FORGE research flow with parallel gather pattern. Multiple search stages run in parallel, then results are synthesized and verified.
Syntax:
  flow research
    needs topic: Text
    gives Text

    stage gather_web
      result = search topic

    stage gather_papers
      result = search "{topic} academic paper"

    stage gather_news
      result = search "{topic} latest news"

    stage synthesize
      needs gather_web.result, gather_papers.result, gather_news.result
      draft = reason "synthesize: {gather_web.result} {gather_papers.result} {gather_news.result}"

    stage verify
      needs synthesize.draft
      checked = reason "is this factually consistent? {synthesize.draft}"
      when checked.sure -> give checked
      else -> give "Could not verify report."
Key rules:
  - gather_web, gather_papers, gather_news run in parallel (no needs)
  - synthesize waits for all three gather stages
  - verify waits for synthesize
  - Pattern: fan-out (parallel gather) → funnel (synthesize) → verify
  - Requires: use web.search and use llm.reason
Related: see TASKS search pattern for the search primitive
FACT
)"

ingest_fact "FLOWS" "$(cat <<'FACT'
FORGE pipeline flow with sequential stages pattern. Flows can model strictly sequential pipelines where each stage depends on the previous.
Syntax:
  flow <name>
    needs <params>
    gives <Type>

    stage setup
      <initialization>

    stage plan
      needs setup.<field>
      <planning>

    stage implement
      needs plan.<field>
      <implementation>

    stage test
      needs implement.<field>
      <testing>
Example from dev-cycle.forge:
  flow implementation_pipeline
    needs issue: Issue
    gives PullRequest

    stage setup
      branch = branch_name(issue)

    stage plan
      needs setup.branch
      plan_text = draft_plan(issue)

    stage implement
      needs plan.plan_text
      code = implement_plan(plan_text)

    stage test
      needs implement.code
      report = run_tests(code)

    stage changelog
      needs implement.code
      entry = draft_changelog_entry(issue, code)

    stage pr_ready
      needs test.report, changelog.entry
      give draft_pr_description(issue, test.report, changelog.entry)
Key rules:
  - Each stage depends on the previous via needs
  - test and changelog can run in parallel (both only need implement)
  - pr_ready waits for both test and changelog
  - Pattern: linear pipeline with optional parallel forks
Related: see SYSTEMS for the system wiring that coordinates agents running these flows
FACT
)"

ingest_fact "FLOWS" "$(cat <<'FACT'
FORGE recall-driven flow pattern. Flows can integrate knowledge store recall for context-aware processing.
Syntax:
  flow answer_query
    needs question: Text
    gives QueryResult

    stage categorize
      categories = question >> categorize_for_recall(_pipe)

    stage retrieve
      needs categorize.categories
      prior = recall "{categorize.categories} {question}"

    stage respond
      needs retrieve.prior
      when retrieve.prior.sure -> give QueryResult(answer: response, confidence_tier: "sure", sources_used: 1)
      when retrieve.prior.unsure -> give QueryResult(answer: response, confidence_tier: "unsure", sources_used: 1)
      else -> give QueryResult(answer: response, confidence_tier: "none", sources_used: 0)
Key rules:
  - recall in a flow stage returns an uncertain value
  - Must dispatch with when/else in the responding stage
  - Pattern: categorize → retrieve → respond (knowledge-augmented generation)
  - The >> pipe chains categorize_for_recall into the recall query
  - Type constructors (QueryResult(...)) create structured return values
Related: see AGENTS knowledge agent pattern for recall in agent handlers
FACT
)"

ingest_fact "FLOWS" "$(cat <<'FACT'
FORGE document generation flow pattern. Complex flows can coordinate many parallel stages for content generation.
Example from docgen.forge:
  flow generate_docs
    needs source_code: Text
    gives Text

    stage analyze
      structure = reason "Analyze code structure: {source_code}"

    stage doc_functions
      needs analyze.structure
      docs = reason "Document each function: {analyze.structure}"

    stage doc_types
      needs analyze.structure
      docs = reason "Document types and interfaces: {analyze.structure}"

    stage doc_examples
      needs analyze.structure
      examples = reason "Generate usage examples: {analyze.structure}"

    stage assemble
      needs doc_functions.docs, doc_types.docs, doc_examples.docs
      give reason "Assemble documentation: {doc_functions.docs}\n{doc_types.docs}\n{doc_examples.docs}"
Key rules:
  - analyze runs first (no dependencies)
  - doc_functions, doc_types, doc_examples run in parallel (all need only analyze)
  - assemble waits for all three documentation stages
  - Pattern: analyze → parallel documentation → assemble
Related: see TASKS reason pattern for the LLM calls within each stage
FACT
)"

ingest_fact "FLOWS" "$(cat <<'FACT'
FORGE repo scan flow with data gathering pattern. Flows can combine exec and reason stages for analyzing external systems.
Example from sentinel:
  flow repo_scan
    needs repo_path: Text
    gives ScanResult

    stage gather_git
      commits = get_recent_commits(repo_path)
      velocity = get_commit_velocity(repo_path)
      churn = get_code_churn(repo_path)

    stage gather_metrics
      branches = get_branch_hygiene(repo_path)
      size = get_codebase_size(repo_path)
      build = get_build_status(repo_path)

    stage analyze
      needs gather_git.*, gather_metrics.*
      velocity_analysis = analyze_velocity(gather_git.velocity)
      churn_analysis = analyze_churn(gather_git.churn)
      branch_analysis = analyze_branches(gather_metrics.branches)

    stage score
      needs analyze.*
      health = assess_health(analyze.velocity_analysis, analyze.churn_analysis, analyze.branch_analysis)
Key rules:
  - gather_git and gather_metrics run in parallel
  - needs stage.* captures all outputs from a stage
  - exec-based tasks (git commands) run in gather stages
  - reason-based tasks analyze the gathered data
  - Pattern: parallel gather → analyze → score
Related: see TASKS exec pattern and compose pipe pattern
FACT
)"

ingest_fact "FLOWS" "$(cat <<'FACT'
FORGE flow with stage dependency declaration pattern. Stages use needs to declare which previous stage outputs they require.
Syntax:
  stage <name>
    needs <stage1>.<field>, <stage2>.<field>
    <computation using stage1.field and stage2.field>
Example:
  stage verdict
    needs detect.lang, quality.report, security.report
    give format_verdict(detect.lang, quality.report, security.report)
Key rules:
  - needs <stage>.<field> declares a dependency on a specific output
  - needs <stage>.* captures all outputs from a stage (wildcard)
  - Multiple dependencies: stage waits for all of them
  - No circular dependencies allowed
  - Stages without needs run immediately (in parallel with other independent stages)
  - The runtime automatically determines execution order from the dependency graph
Related: see FLOWS parallel stages pattern for the complete flow structure
FACT
)"

ingest_fact "FLOWS" "$(cat <<'FACT'
FORGE flow with pool verification pattern. Flows can invoke pools for consensus-based verification of results.
Example from wiki:
  flow verify_document
    needs document: Text
    gives Text

    stage check
      result = fact_check_panel(document)
      when result.sure -> give result
      else -> give "Verification inconclusive"

  pool fact_check_panel
    workers: fact_checker * 3
    strategy: majority
    timeout: 30s
    fallback: manual_review
Key rules:
  - pool declares a group of worker agents with a consensus strategy
  - strategy: majority requires >50% agreement
  - strategy: all requires unanimous agreement
  - strategy: fastest takes the first response
  - timeout defines max wait time
  - fallback task runs if pool times out
  - Flows call pools like regular tasks
Related: see SYSTEMS pool pattern for pool declaration details
FACT
)"

phase_time

# ══════════════════════════════════════════════════════════════
# Phase 3: AGENTS — Stateful actors with lifecycle
# ══════════════════════════════════════════════════════════════

echo "Phase 3: AGENTS..."

ingest_fact "AGENTS" "$(cat <<'FACT'
FORGE basic agent with lifecycle and memory pattern. Agents are stateful actors with typed memory, lifecycle state machines, and event-driven handlers.
Syntax:
  agent <name>
    lifecycle: <StatesName>
    memory
      <field>: <Type>

    on <event>(<params>)
      <handler body>
Example:
  agent support_bot
    lifecycle: SupportPhase
    memory
      topic: Text
      message_count: Number
      escalation_count: Number
    timer session_timeout: 10m

    on message(customer: Text, content: Text)
      memory.message_count = memory.message_count + 1
      intent = classify content into ["question", "complaint", "feedback", "urgent"]
      response = reason "Help this customer with their {intent}: {content}"
      when response.sure -> say response
      when response.unsure -> say "Let me look into that."
      else -> escalate to human

    on resolve(customer: Text)
      requires lifecycle == active on fail: give "session is not active"
      transition to resolved

    if stuck for 3 turns
      escalate to human
Key rules:
  - lifecycle binds to a states declaration
  - memory fields persist across handler invocations
  - on <event>(<params>) declares a handler
  - memory.<field> reads/writes persistent state
  - say outputs text to the user
  - escalate to human delegates to a human operator
  - transition to <state> advances the lifecycle
Related: see SYSTEMS states and warden patterns for lifecycle and supervision
FACT
)"

ingest_fact "AGENTS" "$(cat <<'FACT'
FORGE knowledge agent with recall and learn pattern. Agents can maintain a knowledge store for persistent learning and retrieval.
Syntax:
  agent <name>
    knowledge store: "<path>"
      max_entries: <number>
      retention: <duration>

    on query(question: Text)
      prior = recall "{question}"
      when prior.sure -> give prior
      else -> escalate to human

    on learn_fact(fact: Text)
      learn "{fact}"
Example:
  exportable agent research_assistant
    lifecycle: LearningPhase
    memory
      interaction_count: Number
    knowledge store: ".forge-knowledge/research"
      max_entries: 5000
      retention: 90d

    on query(question: Text)
      prior = recall "{question}"
      when prior.sure -> give prior
      else -> escalate to human

    on query_and_learn(question: Text)
      answer = reason "Answer: {question}"
      when answer.sure -> learn from interaction(question, answer, 0.9)
      when answer.unsure -> learn from interaction(question, answer, 0.5)
      else -> escalate to human

    on ingest(document_path: Text)
      learn from document(document_path)
Key rules:
  - knowledge store declares persistent storage path, capacity, and retention
  - recall "<query>" retrieves relevant entries (returns uncertain value)
  - learn "<content>" stores a new knowledge entry
  - learn from interaction(q, a, confidence) stores a Q&A pair
  - learn from document(path) ingests a file in 500-word chunks
  - recall must be dispatched with when/else
Related: see knowledge categories in forge-sensei for how to organize knowledge
FACT
)"

ingest_fact "AGENTS" "$(cat <<'FACT'
FORGE agent with timer and stuck detection pattern. Agents can declare timers that fire periodically and stuck detection for automatic escalation.
Syntax:
  agent <name>
    timer <name>: <duration>

    on <timer_name>.expired
      <handler body>
      reset <timer_name>

    if stuck for <N> turns
      <escalation action>
Example:
  agent quiz_tutor
    lifecycle: TutorPhase
    memory
      score: Number
      questions_asked: Number
    timer session_timeout: 30m

    on start
      start session_timeout
      say "Quiz session started!"

    on session_timeout.expired
      say "Session timed out after 30 minutes."
      reset session_timeout

    if stuck for 3 turns
      say "Tutor is stuck. Escalating."
      escalate to human
Key rules:
  - timer <name>: <duration> declares a timer (s, m, h, d units)
  - start <timer_name> activates the timer in on start handler
  - reset <timer_name> restarts the timer countdown
  - on <timer_name>.expired fires when the timer elapses
  - if stuck for N turns triggers after N consecutive unproductive turns
  - escalate to human delegates to a human operator
Related: see SYSTEMS warden pattern for supervision of stuck agents
FACT
)"

ingest_fact "AGENTS" "$(cat <<'FACT'
FORGE agent with event subscribe and emit pattern. Agents can emit events and subscribe to events from other agents for inter-agent communication.
Syntax:
  agent <name>
    subscribe <EventName> where <filter>

    on <EventName>(<fields>)
      <handler body>

    on some_handler()
      emit <EventName>(field1: value1, field2: value2)
Example:
  agent analyst
    subscribe ScanComplete where repo == memory.repo
    memory
      repo: Text
      last_analysis: Text

    on ScanComplete(repo: Text, result: Text)
      analysis = reason "Analyze scan results: {result}"
      memory.last_analysis = analysis
      emit AnalysisReady(repo: repo, analysis: analysis)
Key rules:
  - subscribe <Event> where <filter> receives events matching the filter
  - where clause filters by field values (typically matching agent memory)
  - emit <Event>(fields) broadcasts an event to all subscribers
  - Events must be declared as event types before use
  - emit fields must match the event declaration exactly (compile-time check)
  - Events are the primary inter-agent communication mechanism
Related: see SYSTEMS event declaration pattern for defining events
FACT
)"

ingest_fact "AGENTS" "$(cat <<'FACT'
FORGE exportable agent pattern. Exportable agents can be built into standalone CLI binaries.
Syntax:
  exportable agent <name>
    lifecycle: <StatesName>
    memory persistent
      <fields>
    knowledge store: "<path>"
      max_entries: <number>
      retention: <duration>
Example:
  exportable agent forge_sensei
    lifecycle: MasteryLevel
    memory persistent
      interaction_count: Number
      success_count: Number
      last_assessment_score: Number
      current_level: Text
    knowledge store: ".forge-knowledge/sensei"
      max_entries: 50000
      retention: 365d
    timer self_assess: 6h
Key rules:
  - exportable keyword makes the agent buildable as a standalone binary
  - memory persistent survives across process restarts (serialized to disk)
  - Build command: forge build <source> -o <binary_path>
  - Run command: <binary> <handler_name> <args>
  - Only one agent per program can be exportable
Related: see contract pattern in SYSTEMS for declaring the agent's public API
FACT
)"

ingest_fact "AGENTS" "$(cat <<'FACT'
FORGE agent with requires guards pattern. Handler preconditions gate execution based on lifecycle state.
Syntax:
  on <handler>(<params>)
    requires lifecycle == <state> on fail: <fallback>
    <handler body>
Example:
  on review(code: Text)
    requires lifecycle == apprentice or lifecycle == journeyman or lifecycle == expert
      on fail: give "Code review unlocks at apprentice level. Run assessment first."
    memory.interaction_count = memory.interaction_count + 1
    give review_code(code)

  on move(player: Text, cell: Number)
    requires lifecycle == playing on fail: give "Game not in progress."
    requires player == memory.current_player on fail: give "Not your turn."
    <move logic>
Key rules:
  - requires lifecycle == <state> checks current lifecycle state
  - Multiple requires can be chained (all must pass)
  - on fail: <action> specifies what happens if the guard fails
  - on fail: give returns a value without executing the handler
  - on fail: silent does nothing
  - on fail: escalate delegates to the warden
  - Guards prevent handlers from running in invalid states
Related: see SYSTEMS states pattern for defining lifecycle state machines
FACT
)"

ingest_fact "AGENTS" "$(cat <<'FACT'
FORGE agent with state transitions pattern. Agents advance their lifecycle by transitioning between states.
Syntax:
  transition to <state>
Example:
  on batch_assess(tests: Text[], expectations: Text[])
    passed = 0
    total = 0
    for test in tests
      total = total + 1
      result = assess_detailed(test, expectations[total - 1])
      if result.passed
        passed = passed + 1
    score = compute_mastery_score(passed, total)
    if score >= 90 and lifecycle != expert
      transition to expert
    if score >= 70 and lifecycle == apprentice
      transition to journeyman
    if score >= 40 and lifecycle == novice
      transition to apprentice
Key rules:
  - transition to <state> must follow a declared edge in the states definition
  - Illegal transitions are compile errors (e.g., novice -> expert directly)
  - lifecycle == <state> checks the current state
  - Transitions can only happen inside agent handlers
  - The runtime enforces the state machine — no skipping states
Related: see SYSTEMS states declaration pattern for defining valid transitions
FACT
)"

ingest_fact "AGENTS" "$(cat <<'FACT'
FORGE spawn with knowledge transfer pattern. Agents can spawn child agents with filtered knowledge and injected memory.
Syntax:
  child = spawn <agent_type> as "<alias>"
    with knowledge where category == <filter>
    with memory <field>: <value>
Example:
  on deep_dive(topic: Text)
    requires lifecycle == journeyman or lifecycle == expert
      on fail: give "Deep dive requires journeyman level."
    existing = find "specialist_{topic}"
    if existing
      give existing
    child = spawn specialist as "specialist_{topic}"
      with knowledge where category == topic
      with memory topic: topic
    emit LearnedInsight(category: topic, content: "Specialist spawned", source: "deep_dive", confidence: 0.9)
    give child
Key rules:
  - spawn <type> as "<alias>" creates a new agent instance
  - with knowledge where <filter> transfers only matching knowledge entries
  - with memory <field>: <value> injects initial memory values
  - find "<alias>" looks up an existing agent by alias (returns nil if not found)
  - The spawned agent type must be declared in the same program
  - Specialists subscribe to events filtered by their topic
Related: see specialist agent pattern for the spawned agent's implementation
FACT
)"

ingest_fact "AGENTS" "$(cat <<'FACT'
FORGE agent with persistent memory pattern. Persistent memory survives process restarts by serializing to disk.
Syntax:
  agent <name>
    memory persistent
      <field1>: <Type>
      <field2>: <Type>
Example:
  agent git_inspector
    memory persistent
      last_scan_time: Text
      total_scans: Number
      repo_health_score: Number

    on start
      say "Inspector ready. Last scan: {memory.last_scan_time}. Total: {memory.total_scans}"

    on scan(repo_path: Text)
      memory.total_scans = memory.total_scans + 1
      memory.last_scan_time = "now"
      <scan logic>
Key rules:
  - memory persistent keyword enables disk-backed state
  - Fields are restored on process restart
  - Serialized as JSON in .forge-data/ directory
  - Without persistent, memory is lost when the process exits
  - Numbers initialize to 0, Text to empty string
Related: see AGENTS exportable pattern for building persistent agents into binaries
FACT
)"

ingest_fact "AGENTS" "$(cat <<'FACT'
FORGE multi-agent cooperation pattern. Multiple agents in a system communicate via events and coordinate through a warden.
Example from dev-cycle.forge (5 cooperating agents):
  agent planner
    subscribe IssueAssigned
    on IssueAssigned(issue: Issue)
      plan = draft_plan(issue)
      emit PlanApproved(issue: issue, plan: plan)

  agent implementer
    subscribe PlanApproved
    on PlanApproved(issue: Issue, plan: Text)
      code = implement_plan(plan)
      emit ImplementationReady(issue: issue, code: code)

  agent tester
    subscribe ImplementationReady
    on ImplementationReady(issue: Issue, code: Text)
      report = run_tests(code)
      if report.passed
        emit AcceptanceMet(issue: issue)
      else
        emit TestsFailed(issue: issue, report: report)

  agent reviewer
    subscribe AcceptanceMet
    on AcceptanceMet(issue: Issue)
      pr = draft_pr_description(issue)
      emit PRReady(issue: issue, pr: pr)

  system forge_dev
    planner >> implementer >> tester >> reviewer
Key rules:
  - Each agent subscribes to the event emitted by the previous agent
  - Events carry typed data between agents
  - The system declaration wires agents together with >> composition
  - A warden supervises all agents for failure handling
  - Pattern: event-driven pipeline where each agent owns one responsibility
Related: see SYSTEMS system composition and warden patterns
FACT
)"

phase_time

# ══════════════════════════════════════════════════════════════
# Phase 4: SYSTEMS — Orchestration and infrastructure
# ══════════════════════════════════════════════════════════════

echo "Phase 4: SYSTEMS..."

ingest_fact "SYSTEMS" "$(cat <<'FACT'
FORGE system with composition pattern. Systems wire agents together using the >> composition operator.
Syntax:
  system <name>
    use
      <binding1>: <agent_type>
      <binding2>: <agent_type>
    <binding1> >> <binding2>
Example:
  system tictactoe
    use
      game: room_agent
      lobby: lobby_handler
    game >> lobby

  system repo_sentinel
    inspector >> analyst

  system forge_dev
    planner >> implementer >> tester >> reviewer
Key rules:
  - system declares a composition of agents
  - use block binds agent types to local names
  - >> wires agents in a communication chain
  - Events flow along the >> direction
  - A system can contain 2 or more agents
  - Each system should have a warden for supervision
Related: see AGENTS multi-agent pattern for the agents being composed
FACT
)"

ingest_fact "SYSTEMS" "$(cat <<'FACT'
FORGE pool declaration pattern. Pools create worker groups with consensus strategies for parallel evaluation.
Syntax:
  pool <name>
    workers: <agent_type> * <count>
    strategy: <majority|all|fastest>
    timeout: <duration>
    fallback: <task_name>
Example:
  pool fact_check_panel
    workers: fact_checker * 3
    strategy: majority
    timeout: 30s
    fallback: manual_review

  pool health_panel
    workers: health_assessor * 3
    strategy: majority
    timeout: 15s
    fallback: default_health_assessment
Key rules:
  - workers declares agent type and count
  - strategy: majority — >50% must agree
  - strategy: all — unanimous agreement required
  - strategy: fastest — first response wins
  - timeout defines max wait time for consensus
  - fallback task runs if the pool times out
  - Pools are called like tasks from flows or handlers
Related: see FLOWS pool verification pattern for using pools in flows
FACT
)"

ingest_fact "SYSTEMS" "$(cat <<'FACT'
FORGE warden with escalation policies pattern. Wardens supervise agents with failure-specific response ladders.
Syntax:
  warden <name>
    manages [<agent1>, <agent2>]
    on <failure_type>: <response>, <target>
      after <N>: <escalated_response>
    max_retries <N> per <duration> then <final_action>
Example:
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
Key rules:
  - manages lists the agents under supervision
  - Failure types: stuck, hallucination, contradiction, crash, timeout, budget
  - Responses: nudge (gentle prompt), restart (reset agent), escalate (to human)
  - after N escalates after N occurrences of the same failure
  - max_retries is a circuit breaker — prevents infinite retry loops
  - self means the warden handles it; escalate means human intervention
  - All managed agents must be declared in the same program
Related: see AGENTS stuck detection and requires guards for agent-level failure handling
FACT
)"

ingest_fact "SYSTEMS" "$(cat <<'FACT'
FORGE contract declaration pattern. Contracts define the public API of an exportable agent.
Syntax:
  contract <Name>
    can <method>(<params>) -> <ReturnType>
Example:
  contract ForgeTutor
    can query(question: Text) -> QueryResult
    can review(code: Text) -> Text
    can status() -> Text
    can assess_detailed(test_input: Text, expected: Text) -> AssessmentResult

  contract GameRoom
    can join(player: Text) -> Text
    can move(player: Text, cell: Number) -> GameResult
Key rules:
  - contract declares what an exportable agent can do
  - Each can declaration maps to an on handler in the agent
  - Return types must match the handler's give type
  - Contracts are documentation and type-checking, not runtime enforcement
  - One contract per exportable agent
Related: see AGENTS exportable pattern for the agent implementing the contract
FACT
)"

ingest_fact "SYSTEMS" "$(cat <<'FACT'
FORGE states declaration with conditional guards pattern. States define finite state machines with transition conditions.
Syntax:
  states <Name>
    <state1> -> <state2> when <condition>
    <state2> -> <state3> when <condition>
    <state3> -> <state3>
Example:
  states MasteryLevel
    novice -> apprentice when conformance_score >= 40
    apprentice -> journeyman when conformance_score >= 70
    journeyman -> expert when conformance_score >= 90
    expert -> expert

  states IssueLifecycle
    open -> planning
    planning -> in_progress
    in_progress -> testing
    testing -> review_ready
    review_ready -> merged

  states SpecialistPhase
    learning -> ready when absorbed_count >= 5
    ready -> ready
Key rules:
  - Each line declares a valid transition
  - when <condition> adds a guard (optional)
  - Self-transitions (expert -> expert) are allowed
  - Transitions not declared are illegal (compile error if attempted)
  - Agents bind to states via lifecycle: <StatesName>
  - transition to <state> in handlers must follow declared edges
Related: see AGENTS state transition pattern for using transitions in handlers
FACT
)"

ingest_fact "SYSTEMS" "$(cat <<'FACT'
FORGE event declaration pattern. Events are typed messages for inter-agent communication.
Syntax:
  event <Name>
    <field1>: <Type>
    <field2>: <Type>
Example:
  event LearnedInsight
    category: Text
    content: Text
    source: Text
    confidence: Number

  event IssueAssigned
    issue: Issue
    assignee: Text

  event ScanComplete
    repo: Text
    result: ScanResult
    timestamp: Text
Key rules:
  - Events have typed fields like types
  - emit <Event>(field: value) broadcasts to all subscribers
  - subscribe <Event> where <filter> receives matching events
  - emit fields must exactly match the event declaration
  - Events are the primary inter-agent communication mechanism
  - Agents can both emit and subscribe to the same event type
Related: see AGENTS event subscribe/emit pattern for using events in handlers
FACT
)"

ingest_fact "SYSTEMS" "$(cat <<'FACT'
FORGE type declaration pattern. Types are named records with typed fields for structured data.
Syntax:
  type <Name>
    <field1>: <Type>
    <field2>: <Type>
Example:
  type QueryResult
    answer: Text
    confidence_tier: Text
    sources_used: Number

  type Issue
    title: Text
    body: Text
    labels: Text[]
    assignee: Text

  type ScanResult
    health_score: Number
    issues_found: Number
    recommendations: Text
Key rules:
  - Built-in types: Text, Number, Bool, Html
  - Array types: Text[], Number[]
  - Custom types can reference other custom types
  - Type constructors: QueryResult(answer: "...", confidence_tier: "sure", sources_used: 1)
  - Field access: result.answer, result.confidence_tier
  - Types are used in task needs/gives, event fields, and agent memory
Related: see TASKS for using types in task signatures
FACT
)"

ingest_fact "SYSTEMS" "$(cat <<'FACT'
FORGE endpoint declaration pattern. Endpoints serve HTTP requests and return Html or Text responses.
Syntax:
  endpoint <name>(<params>) -> <Html|Text>
    <handler body>
    give <response>
Example:
  endpoint status() -> Html
    level = determine_level(memory.last_assessment_score)
    card = "<div class=\"card\">{level}</div>"
    give render_page("Status", card)

  endpoint ask(question: Text) -> Html
    result = answer_query(question)
    body = render_answer(result.answer, result.confidence_tier)
    give render_page("Answer", body)

  endpoint api_health() -> Text
    give "{\"status\": \"ok\", \"level\": \"{memory.current_level}\"}"
Key rules:
  - Endpoints handle HTTP GET (no params) or POST (with params)
  - Return type is Html (for web pages) or Text (for API responses)
  - Can call tasks, flows, pure functions
  - Served via: forge serve <source> --watch
  - Default port: 3030
  - Requires: #! boundary: server at the top of the file
Related: see TASKS pure HTML rendering pattern for building page content
FACT
)"

ingest_fact "SYSTEMS" "$(cat <<'FACT'
FORGE boundary directive pattern. Boundary directives control where code can execute in a client-server architecture.
Syntax:
  #! boundary: server
  #! boundary: client
  #! boundary: shared
Example:
  # server.forge
  #! boundary: server

  task fetch_data
    needs query: Text
    gives Text
    do
      result = exec "curl -s 'https://api.example.com?q={query}'"
      when result.sure -> give result
      else -> give "fetch failed"

  # shared.forge
  #! boundary: shared

  type SearchResult
    title: Text
    url: Text
    snippet: Text

  event SearchCompleted
    results: SearchResult[]
Key rules:
  - server: code runs on the server (exec, endpoints, data access)
  - client: code runs in the browser (future WASM compilation)
  - shared: types and events visible to both server and client
  - Cross-boundary calls are compile errors (client cannot call server tasks directly)
  - Multi-file projects split across shared/server/client files
Related: see TASKS exec pattern for server-side shell commands
FACT
)"

ingest_fact "SYSTEMS" "$(cat <<'FACT'
FORGE system with multiple agents pattern. Complete system declaration showing how all components wire together.
Example from dev-cycle.forge:
  type Issue
    title: Text
    body: Text

  event IssueAssigned
    issue: Issue

  event PlanApproved
    issue: Issue
    plan: Text

  states IssueLifecycle
    open -> planning -> in_progress -> testing -> review_ready -> merged

  agent planner
    lifecycle: IssueLifecycle
    subscribe IssueAssigned
    on IssueAssigned(issue: Issue)
      plan = draft_plan(issue)
      emit PlanApproved(issue: issue, plan: plan)

  warden dev_lead
    manages [planner, implementer, tester, reviewer]
    on stuck: nudge, self
      after 3: escalate
    on crash: restart, self
    max_retries 5 per 1h then escalate

  system forge_dev
    planner >> implementer >> tester >> reviewer
Key rules:
  - Complete system needs: types, events, states, agents, warden, system
  - Types define data structures passed between agents
  - Events define messages for inter-agent communication
  - States define lifecycle progressions
  - Agents subscribe to events and emit new ones
  - Warden supervises all agents
  - System wires agents in execution order
Related: all other SYSTEMS patterns for individual component details
FACT
)"

phase_time

# ══════════════════════════════════════════════════════════════
# Phase 5: ERRORS — Compiler error → fix mappings
# ══════════════════════════════════════════════════════════════

echo "Phase 5: ERRORS..."

# Pure function violations (8 types)
for primitive in reason classify escalate exec recall learn spawn find; do
  case "$primitive" in
    reason)
      code='pure bad\n  needs x: Text\n  gives Text\n  do\n    give reason "think about {x}"'
      fix="Change 'pure' to 'task' to allow LLM operations, or remove the reason call."
      ;;
    classify)
      code='pure bad\n  needs x: Text\n  gives Text\n  do\n    give classify x into ["a", "b"]'
      fix="Change 'pure' to 'task' to allow LLM operations, or remove the classify call."
      ;;
    escalate)
      code='pure bad\n  needs x: Text\n  gives Text\n  do\n    escalate to human'
      fix="Move escalate to an agent handler. Pure functions cannot escalate."
      ;;
    exec)
      code='pure bad\n  needs x: Text\n  gives Text\n  do\n    give exec "echo {x}"'
      fix="Change 'pure' to 'task' to allow shell execution, or remove the exec call."
      ;;
    recall)
      code='pure bad\n  needs x: Text\n  gives Text\n  do\n    give recall "{x}"'
      fix="Move recall to a task or agent handler. Pure functions cannot access knowledge store."
      ;;
    learn)
      code='pure bad\n  needs x: Text\n  gives Text\n  do\n    learn "{x}"\n    give "ok"'
      fix="Move learn to an agent handler. Pure functions cannot modify knowledge store."
      ;;
    spawn)
      code='pure bad\n  needs x: Text\n  gives Text\n  do\n    child = spawn worker as "w"\n    give "ok"'
      fix="Move spawn to an agent handler. Pure functions cannot create agents."
      ;;
    find)
      code='pure bad\n  needs x: Text\n  gives Text\n  do\n    agent = find "worker"\n    give "ok"'
      fix="Move find to an agent handler. Pure functions cannot discover agents."
      ;;
  esac

  ingest_fact "ERRORS" "$(cat <<FACT
FORGE compiler error: pure function using ${primitive}. Pure functions cannot use ${primitive} because it is a non-deterministic or side-effect operation.
Erroneous code:
  ${code}
Compiler error: "cannot use ${primitive} in pure function"
Fix: ${fix}
Key rules:
  - Pure functions must be deterministic — no LLM, no I/O, no side effects
  - Forbidden in pure: reason, classify, search, exec, recall, learn, spawn, find, emit, escalate, try/or
  - Pure results are always sure (confidence = 1.0)
Related: see TASKS pure function pattern for valid pure function examples
FACT
)"
done

# Boundary cross-reference error
ingest_fact "ERRORS" "$(cat <<'FACT'
FORGE compiler error: boundary cross-reference. Client code cannot directly call server-side tasks or access server resources.
Erroneous code:
  # server.forge
  #! boundary: server
  task fetch_data
    needs q: Text
    gives Text
    do
      give exec "curl {q}"

  # client.forge
  #! boundary: client
  fn main
    data = fetch_data("query")  # ERROR: cross-boundary call
Compiler error: "cannot reference server boundary item from client context"
Fix: Use shared types and events for cross-boundary communication. Client emits an event, server subscribes and processes it.
Key rules:
  - server tasks are invisible to client code
  - client tasks are invisible to server code
  - shared types and events are visible to both
  - The boundary system enforces clean separation of concerns
Related: see SYSTEMS boundary directive pattern for proper boundary usage
FACT
)"

# Unhandled uncertain result
ingest_fact "ERRORS" "$(cat <<'FACT'
FORGE compiler error: unhandled uncertain result. Values from reason, classify, search, recall, and exec are uncertain and MUST be dispatched with when/else.
Erroneous code:
  task bad
    needs x: Text
    gives Text
    do
      result = reason "analyze {x}"
      give result  # ERROR: result is uncertain, not dispatched
Compiler error: "uncertain value used without confidence dispatch"
Fix: Add when/else dispatch before using the result:
  task good
    needs x: Text
    gives Text
    do
      result = reason "analyze {x}"
      when result.sure -> give result
      when result.unsure -> give "uncertain: {result}"
      else -> give "failed"
Key rules:
  - Principle I (Honesty): uncertain values cannot be treated as certain
  - All LLM operations return uncertain values
  - recall returns uncertain values (knowledge may not match)
  - exec returns uncertain values (commands may fail)
  - The compiler catches this at compile time, not runtime
Related: see TASKS confidence dispatch pattern for proper when/else usage
FACT
)"

# States illegal transition
ingest_fact "ERRORS" "$(cat <<'FACT'
FORGE compiler error: illegal state transition. Transitions must follow edges declared in the states definition.
Erroneous code:
  states MasteryLevel
    novice -> apprentice when score >= 40
    apprentice -> journeyman when score >= 70
    journeyman -> expert when score >= 90

  agent bad
    lifecycle: MasteryLevel
    on skip_ahead
      transition to expert  # ERROR if lifecycle == novice (no novice -> expert edge)
Compiler error: "illegal transition: no edge from novice to expert"
Fix: Follow the declared transition path. Novice must go through apprentice and journeyman before reaching expert.
Key rules:
  - Only transitions declared in the states block are allowed
  - The compiler checks that all transition to statements reference valid edges
  - Self-transitions must be explicitly declared (e.g., expert -> expert)
  - Guard conditions (when) are checked at runtime, edges at compile time
Related: see SYSTEMS states declaration pattern for defining valid transitions
FACT
)"

phase_time

# ══════════════════════════════════════════════════════════════
# Phase 6: TESTING — Conformance test structure
# ══════════════════════════════════════════════════════════════

echo "Phase 6: TESTING..."

ingest_fact "TESTING" "$(cat <<'FACT'
FORGE conformance test structure: parser test (parse_ok). Tests that verify valid FORGE code parses successfully.
JSON structure:
  {
    "name": "valid_task_declaration",
    "category": "parser",
    "description": "Basic task declaration with needs/gives/do parses successfully",
    "input": "task greet\n  needs name: Text\n  gives Text\n  do\n    give \"hello\"\n",
    "expected": {
      "outcome": "parse_ok"
    }
  }
Key rules:
  - category is always "parser"
  - input contains the FORGE source code as a string (with \n for newlines)
  - outcome "parse_ok" means the parser accepts the input without errors
  - No mock_responses needed (parsing doesn't involve LLM)
  - Test files go in conformance/parser/ directory
  - File naming convention: <feature>_<description>.json
Related: see parser error test pattern for testing invalid syntax
FACT
)"

ingest_fact "TESTING" "$(cat <<'FACT'
FORGE conformance test structure: parser test (parse_error). Tests that verify invalid FORGE code is rejected by the parser.
JSON structure:
  {
    "name": "invalid_syntax",
    "category": "parser",
    "description": "Malformed task declaration fails to parse",
    "input": "task\n  gives\n  broken syntax!!!\n",
    "expected": {
      "outcome": "parse_error",
      "error_contains": ["expected"]
    }
  }
Key rules:
  - outcome "parse_error" means the parser rejects the input
  - error_contains is an array of strings that must appear in the error message
  - Useful for testing: missing keywords, invalid indentation, malformed declarations
  - Test files go in conformance/parser/ directory
Related: see parser parse_ok test pattern for testing valid syntax
FACT
)"

ingest_fact "TESTING" "$(cat <<'FACT'
FORGE conformance test structure: checker test (compile_ok). Tests that verify valid FORGE code passes the type checker and semantic analysis.
JSON structure:
  {
    "name": "compile_ok_clean_program",
    "category": "checker",
    "description": "A valid program with no violations compiles cleanly",
    "input": "task greet\n  needs name: Text\n  gives Text\n  do\n    give \"Hello, {name}!\"\n\nfn main\n  result = greet(\"World\")\n  say result\n",
    "expected": {
      "outcome": "compile_ok"
    }
  }
Key rules:
  - category is always "checker"
  - outcome "compile_ok" means the checker finds no errors
  - The checker validates: purity, uncertainty dispatch, state transitions, boundary rules, warden references
  - Test files go in conformance/checker/ directory
Related: see checker compile_error test pattern for testing semantic violations
FACT
)"

ingest_fact "TESTING" "$(cat <<'FACT'
FORGE conformance test structure: checker test (compile_error). Tests that verify the checker catches semantic violations.
JSON structure:
  {
    "name": "pure_no_reason",
    "category": "checker",
    "subcategory": "pure",
    "description": "Pure function cannot use reason (LLM operation)",
    "input": "pure bad\n  needs x: Text\n  gives Text\n  do\n    give reason \"think about {x}\"\n",
    "expected": {
      "outcome": "compile_error",
      "error_contains": ["cannot use", "reason"]
    }
  }
Key rules:
  - outcome "compile_error" means the checker rejects the code
  - error_contains verifies specific error message fragments
  - subcategory (optional) groups related tests (pure, boundary, states, etc.)
  - Test files go in conformance/checker/ directory
  - Common subcategories: pure (purity violations), boundary (cross-boundary calls), states (invalid transitions), uncertain (unhandled uncertain values)
Related: see ERRORS category for the full list of compiler error patterns
FACT
)"

ingest_fact "TESTING" "$(cat <<'FACT'
FORGE conformance test structure: runtime test with mock_responses. Tests that verify runtime behavior with simulated LLM responses.
JSON structure:
  {
    "name": "confidence_dispatch_sure",
    "category": "runtime",
    "description": "When branch dispatches on sure confidence from LLM response",
    "input": "use\n  llm.reason\n\ntask analyze\n  needs topic: Text\n  gives Text\n  do\n    result = reason \"analyze {topic}\"\n    when result.sure -> give result\n    when result.unsure -> give \"not sure\"\n\nfn main\n  result = analyze(\"test\")\n  say result\n",
    "mock_responses": [
      {"text": "This is a clear analysis", "confidence": 0.85}
    ],
    "expected": {
      "outcome": "run_ok",
      "trace_shape": ["task_call", "llm_request", "llm_response", "when_dispatch", "task_return"]
    }
  }
Key rules:
  - mock_responses simulates LLM responses in order (first call gets first mock)
  - Each mock has text (the LLM output) and confidence (0.0-1.0)
  - trace_shape verifies the execution trace (sequence of runtime events)
  - outcome "run_ok" means the program executes without runtime errors
  - Test files go in conformance/runtime/ directory
Related: see runtime trace test pattern for pure function tests without mocks
FACT
)"

ingest_fact "TESTING" "$(cat <<'FACT'
FORGE conformance test structure: runtime test with trace verification. Tests that verify execution traces for deterministic operations.
JSON structure:
  {
    "name": "hello_task_runs",
    "category": "runtime",
    "description": "A simple task with no LLM calls executes and produces output",
    "input": "task greet\n  needs name: Text\n  gives Text\n  do\n    give \"Hello, {name}!\"\n\nfn main\n  result = greet(\"World\")\n  say result\n",
    "mock_responses": [],
    "expected": {
      "outcome": "run_ok",
      "trace_shape": ["task_call", "task_return"]
    }
  }
Key rules:
  - mock_responses is empty for deterministic tests (pure functions, basic tasks)
  - trace_shape verifies the exact sequence of runtime events
  - Common trace events: task_call, task_return, llm_request, llm_response, when_dispatch, memory_set, emit, say, learn
  - Pure function calls appear as task_call/task_return without llm events
  - Test files go in conformance/runtime/ directory
Related: see runtime mock test pattern for LLM-involving tests
FACT
)"

ingest_fact "TESTING" "$(cat <<'FACT'
FORGE conformance test structure: error message format test. Tests that verify compiler error messages contain helpful identifiers.
JSON structure:
  {
    "name": "error_pure_oracle",
    "category": "errors",
    "description": "Error message for pure function using oracle contains function name",
    "input": "pure my_func\n  needs x: Text\n  gives Text\n  do\n    give reason \"think about {x}\"\n",
    "expected": {
      "outcome": "compile_error",
      "error_contains": ["my_func", "reason", "pure"]
    }
  }
Key rules:
  - Error message tests verify that error output is helpful and specific
  - error_contains should include: the identifier name, the violation type, and the context
  - Good error messages help developers fix issues quickly
  - Test files go in conformance/errors/ directory
Related: see ERRORS category for the full catalog of compiler error patterns
FACT
)"

ingest_fact "TESTING" "$(cat <<'FACT'
FORGE conformance test inventory summary. The test suite covers parsing, checking, and runtime across all language features.
Categories and approximate counts:
  - conformance/parser/ — ~30 tests (valid declarations, invalid syntax)
  - conformance/checker/ — ~25 tests (purity, uncertainty, states, boundary, warden)
  - conformance/runtime/ — ~20 tests (pure functions, LLM mocking, trace verification)
  - conformance/errors/ — ~5 tests (error message quality)
  - tests/sensei_parser_tests.rs — 8 Rust integration tests
  - tests/sensei_checker_tests.rs — 10 Rust integration tests
  - tests/sensei_runtime_tests.rs — 25 Rust integration tests
  - tests/sensei_knowledge_tests.rs — 8 Rust integration tests
Key rules:
  - Conformance tests are JSON files run by the test harness
  - Rust integration tests use the Rust test framework (cargo test)
  - E2E tests (sensei-e2e-test.sh) test the built binary end-to-end
  - Run all: cargo test && bash scripts/sensei-e2e-test.sh
  - New features should add conformance tests first, then Rust tests
Related: see all TESTING patterns for individual test structure templates
FACT
)"

phase_time

fi # end VERIFY_ONLY skip

# ══════════════════════════════════════════════════════════════
# Phase 7: Verification
# ══════════════════════════════════════════════════════════════

if [ "$DRY_RUN" = false ]; then
  echo ""
  echo "Phase 7: Verification..."
  VERIFY_PASS=0
  VERIFY_FAIL=0

  verify_recall() {
    local query="$1"
    local min_confidence="$2"
    local output
    # Use query handler which runs answer_query flow (categorize → recall → respond)
    output=$("$SENSEI_BIN" query "$query" 2>&1) || true
    if echo "$output" | grep -q "confidence_tier: sure\|confidence_tier: unsure"; then
      VERIFY_PASS=$((VERIFY_PASS + 1))
      local tier
      tier=$(echo "$output" | grep -o "confidence_tier: [a-z]*" | head -1 | cut -d' ' -f2)
      printf "  PASS: query \"%s\" → confidence_tier: %s\n" "$query" "$tier"
    elif echo "$output" | grep -q "confidence_tier: none"; then
      VERIFY_FAIL=$((VERIFY_FAIL + 1))
      printf "  FAIL: query \"%s\" → confidence_tier: none (no knowledge matched)\n" "$query"
    else
      VERIFY_FAIL=$((VERIFY_FAIL + 1))
      printf "  FAIL: query \"%s\" → unexpected output\n" "$query"
    fi
  }

  verify_recall "FORGE task declaration needs gives" 0.6
  verify_recall "FORGE flow parallel stages wave" 0.6
  verify_recall "FORGE agent lifecycle memory" 0.6
  verify_recall "FORGE system warden composition" 0.6
  verify_recall "FORGE pure function compiler error" 0.6
  verify_recall "FORGE conformance test structure" 0.6

  echo ""
  echo "Verification: $VERIFY_PASS/6 passed"
  if [ "$VERIFY_FAIL" -gt 0 ]; then
    echo "WARNING: $VERIFY_FAIL recall queries returned no results."
    echo "Consider running pretrain-sensei.sh first, then re-running this script."
  fi
  phase_time
fi

# ── Summary ───────────────────────────────────────────────────

if [ "$DRY_RUN" = true ]; then
  echo ""
  echo "=== Dry Run Complete ==="
  echo "Would ingest: $COUNT facts across 6 categories"
  exit 0
fi

if [ "$VERIFY_ONLY" = true ]; then
  exit 0
fi

SUCCEEDED=$((COUNT - FAILED))
PCTVAL=0
if [ "$COUNT" -gt 0 ]; then
  PCTVAL=$((SUCCEEDED * 100 / COUNT))
fi
echo ""
echo "=== Toolkit Curriculum Complete ==="
echo "Ingested: $SUCCEEDED/$COUNT ($PCTVAL%)"

if [ "$FAILED" -gt 0 ]; then
  echo ""
  echo "Failures ($FAILED):"
  for entry in "${FAIL_LOG[@]}"; do
    echo "  $entry"
  done
fi

# Save manifest for idempotency
mkdir -p "$(dirname "$MANIFEST_FILE")"
shasum -a 256 "$0" 2>/dev/null | cut -d' ' -f1 > "$MANIFEST_FILE"

echo ""
"$SENSEI_BIN" status 2>/dev/null || true
