# Changelog

All notable changes to FORGE will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Vector embeddings and semantic search** — `data.embed(content)` generates vector embeddings via OpenAI-compatible providers and stores them in a persistent vector index; `data.search(query, top_k)` performs cosine similarity search returning ranked results with confidence derived from similarity scores; new `EmbeddingProvider` trait with `OpenAICompatEmbeddingProvider` (works with Ollama + nomic-embed-text, OpenAI text-embedding-3-small) and `MockEmbeddingProvider` for testing; `VectorIndex` with brute-force cosine similarity, JSON persistence, and pre-normalized unit vectors; `[embeddings]` config section referencing existing `[providers.*]` entries; `data.search` capability registered in resolver; embedding costs tracked via existing cost aggregator and visible in Observer; graceful fallback with clear error when embeddings not configured (#50)
- **Standalone Observer app** — new `examples/observer/` SPA that connects to any running FORGE server via `/__forge/*` endpoints; connection management with URL input, auto-reconnect, and localStorage persistence; tabbed dashboard with supervision tree, D3 force-directed topology graph, token economy cost panel, and swim-lane trace timeline with brush zoom; shared detail panel for agent/warden/system inspection; SSE event stream with stale detection; CORS fix (`allow_methods` + `allow_headers`) enables cross-origin observer connections; Playwright E2E tests with mock data (#144)
- **Failure injection API** — `POST /__forge/inject/:type` endpoints for testing warden policies; supports stuck, crash, timeout, hallucination, and budget failure types; signals flow through the existing warden pipeline producing `ward_action` SSE events and updating inspect API retry counts; `SharedSignalSenders` collected from system runtime before agent spawning; returns 400/404/503 with helpful error messages for invalid requests; 10 integration tests (#143)
- **Cost and confidence dashboard** — new `/costs` page in Sentinel showing live token counts, accumulated cost (USD), per-agent and per-operation breakdown, confidence distribution histogram, and provider/model cost attribution; `/__forge/inspect/costs` API endpoint returns aggregated metrics as JSON; `CostAggregator` subscribes to the trace event broadcast channel for real-time accumulation; agent name now included in `llm_response` trace events for per-agent cost tracking; D3 confidence histogram with color-coded buckets (sure/unsure/unreliable); all metrics update live via SSE (#142)
- **Sentinel Observer: live agent tree visualization** — new `/observer` endpoint with CSS-based agent hierarchy tree, real-time SSE event stream, agent detail panel with memory/timer/flag inspection, warden health panel, and slow-LLM UX compensation (skeleton screens, thinking indicators, elapsed timers, stale detection); Playwright E2E tests for observer and dashboard; consumes `/__forge/inspect/*` and `/__forge/events` APIs client-side (#140)
- **System runtime in serve mode** — `forge serve` now boots the system runtime (agents + wardens) as a background task alongside the HTTP server when a `system` declaration is present; shared `event_bus`, `instance_registry`, and `warden_snapshots` allow the observer to show live agent state, memory, and warden health; `TaskExecutor::build_system_runtime()` + `SystemRuntime::with_shared_infrastructure()` enable infrastructure sharing between HTTP server and agent runtime (#140)

### Fixed
- **Sentinel dashboard overflow** — long LLM analysis text now contained with `max-height` and scroll; prevents text from breaking out of cards and overlapping metric grid (#140)
- **Sentinel analyst missing insight storage** — analyst agent now calls `data.store("insight:latest", insight)` so the dashboard insight card shows real data (#140)
- **Runtime introspection API** — read-only `/__forge/inspect/*` endpoints exposing agent state, system topology, warden health, and storage keys as JSON; `InstanceRegistry` extended with `AgentContext` refs for deep inspection (memory, timers, stuck/hallucination flags); `WardenSnapshot` and `TopologySnapshot` types shared via `AppState`; `EventBus::subscription_info()` and `ForgeStorage::list_with_sizes()` for topology and storage introspection; 12 integration tests (#139)
- **Live trace stream SSE endpoint** — `/__forge/events` streams all trace events as JSON over Server-Sent Events in serve mode; `Tracer::with_live(tx)` broadcasts to both stderr and an SSE channel; `say` statements now emit trace events (`{"event":"say","text":"..."}`); broadcast channel survives hot-reloads in watch mode; enables real-time feedback for long-running operations like Sentinel scans (#138)
- **Sentinel: AI-powered repo health dashboard** — killer app example at `examples/sentinel/` demonstrating all 17 FORGE primitives (exec, skill, >>, when/sure/unsure, task, pure, flow, pool, agent, warden, system, event, states, endpoint, data.store/get, classify, reason); self-monitoring dashboard that uses `exec` for git data gathering, `exec >> reason` composition for analysis, `skill.git_analysis.weekly_report()` for deep agentic analysis, 3-worker majority-vote health scoring pool, two supervised agents (git_inspector + analyst), parallel scan flow, and 5 HTTP endpoints; configured for Ollama on local GPU server (#132)
- **Mock provider multi-turn tool-use** — `MockProvider::with_tool_call_sequence()` enables deterministic simulation of multi-turn tool-use conversations; drains (not cycles) so the agentic loop terminates naturally; skill E2E test validates the full pipeline: SKILL.md loading → registry → executor agentic loop → bash_exec tool execution → confidence capping (#131)
- **SkillExecutor wiring** — `build_skill_executor()` helper loads skills from config directories and wires the executor at all four `TaskExecutor::new()` call sites, enabling `skill.namespace.method()` calls at runtime; respects `[skills]` config for dirs, timeout, and max turns; no-op when config section is absent (#130)
- **`exec` primitive** — first-class CLI execution expression (`exec "command"`) that runs shell commands and returns `uncertain<Text>` with confidence derived from exit code (0 → 0.9, non-zero → 0.3); enforced by pure checker (exec in pure = compile error), uncertain checker (must handle with `when`), and accountability tracer (`exec_call`/`exec_return` events); composes via `>>` with LLM operations (#40)
- **Host skill bridge** — LLM-mediated execution of SKILL.md ecosystem skills via `skill.namespace.method()` syntax; skills loaded from directories, parsed from YAML frontmatter, executed through an agentic loop where the LLM reads instructions and uses tools (bash, HTTP); results wrapped as `uncertain<T>` with confidence capped at 0.99; pure checker rejects skill calls; full trace events (`skill_call`/`skill_return`); config via `[skills]` TOML section (#40)
- **LLM tool-use extension** — `ToolDefinition` and `ToolCallRequest` types for LLM tool-use; `MockProvider::with_tool_call_response()` for tool-call simulation in tests (#40)
- **`ConfidenceSource::ExecResult`** — confidence source for CLI execution results, capped at 0.95 (#40)
- **`ConfidenceSource::SkillInvocation`** — confidence source for external skill results, capped at 0.99 (#40)
- **Wiki documentation** — `examples/wiki/README.md` (quick start, configuration, endpoints, deployment guide) and `examples/wiki/ARCHITECTURE.md` (system diagrams, data/event flows, supervision tree, complete 14-primitive feature map); updated top-level README with wiki showcase section (#66)

### Fixed
- **Wiki Auto Reference & Fact-Check Report UX** — fixed Record serialization leak (`{reference:` / `{report:` prefix) by changing `Value::Record` Display to output values instead of `{key: value}` debug format; moved `data.store` into generating stages; improved extraction prompts to produce structured markdown; restructured fact-check output with summary table and collapsible per-claim verdicts; added intro context to both pages (#124)
- **Wiki Q&A, search, and auto-reference grounded in actual docs** — `answer_question`, `search_docs`, and `generate_docs` flow now inject real page content into the LLM context via `gather_docs`, eliminating hallucinated FORGE syntax (#122)

### Changed
- **Unified tool-use into CompletionRequest/Response** — `tools` field on `CompletionRequest` and `tool_calls` field on `CompletionResponse` replace the separate `complete_with_tools()` trait method, `CompletionWithToolsResponse` type, and `resolve_and_complete_with_tools()` registry method; tool-use requests now get the same fallback chain as standard completions; eliminates duplicate `estimate_confidence()` code (#133)
- **Wiki UX polish** — doc links show dotted underline with highlight on hover; search results styled as card-like items; confidence badges color-coded (green/yellow/red); theme toggle uses sun/moon icons; activity log gets subtle border (#120)
- **Wiki search and Q&A link to doc pages** — LLM responses now include markdown links to relevant documentation pages (e.g. `[agent](/docs?slug=agent)`) instead of plain text mentions (#120)

### Added
- **End-to-end wiki acceptance tests** — 44 Rust integration tests covering all 7 wiki subsystems: checker/parse validation, page lifecycle (CRUD + state transitions), HTTP endpoints (including root redirect, webhooks), search agent (with re-indexing on events), Q&A agent (with confidence tier branching for low/medium/high), doc generation flow + fact-check pool (majority vote), and warden supervision (crash/stuck/escalation/circuit breaker); all tests run with mock provider in under 2 seconds (#65)
- **Array concatenation** — `Array + Array` now works via the `+` operator, enabling `memory.pages = memory.pages + [slug]` patterns in agents (#65)
- **Wiki warden supervision** — `wiki_supervisor` manages `search_agent`, `content_manager`, and new `qa_agent` with typed failure policies for crash, hallucination, stuck, timeout, and budget; graceful degradation on escalation (agents removed but wiki continues serving); `downgrade` response type added to the language (`nudge < downgrade < restart < replace < escalate`); timeout detection via `tokio::time::timeout` on handler execution; hallucination detection via repeated low-confidence responses; supervision tree visible in trace output; circuit breaker with graceful shutdown (#64)
- **`downgrade` warden response** — new `WardResponse::Downgrade` variant between Nudge and Restart in the escalation severity ordering, parsed from `on budget: downgrade, self` syntax (#64)
- **`qa_agent` wiki agent** — wraps `answer_question` task with persistent memory tracking question count and last question, routed from `/ask` endpoint (#64)
- **Playwright E2E tests for warden** — mock-mode tests verifying wiki functions under supervision, plus real-API tests (gated by `ANTHROPIC_API_KEY`) covering all endpoints with actual LLM calls (#64)
- **Wiki fact-checking pool** — `verify_document` task extracts claims via LLM and verifies each through `fact_check_panel` (3 workers, majority strategy, 30s timeout, fallback on timeout); individual verdicts via `fact_check_detail` (all strategy); claims classified as PASS/NEEDS_REVIEW/FAIL based on consensus confidence; integrated as `fact_check` stage in `generate_docs` flow with dedicated `admin_fact_check` endpoint (#63)
- **`.split(delimiter)` string method** — splits Text into Array of Text values for per-element iteration with `for` loops (#63)
- **`.join(delimiter)` array method** — joins Array/List elements into Text with delimiter (#63)
- **`.length` field access on Text** — `text.length` now works as a property (previously only available as `.len`) (#63)
- **Variable reassignment in nested scopes** — `bind` now updates outer-scope variables when reassigning inside `for`/`if`/`match` blocks (#63)
- **Mixed Text/Html concatenation** — `Text + Html` and `Html + Text` now work via the `+` operator, producing Text (#63)
- **Wiki doc generation flow** — `generate_docs` flow with 5-stage DAG (scan, parallel extraction of tasks/agents/flows, reference generation, publish), `data.store` + `emit PageUpdated`, triggered via `/admin_generate_docs` endpoint with HTML result page (#62)
- **Wiki search agent** — LLM-powered search and Q&A with confidence gating (`when .sure/.unsure/else`), persistent memory tracking index version and query count, event-driven re-indexing via `subscribe`, and confidence badge on answers (#61)
- **Wiki content agent** — full CRUD operations with lifecycle states, persistent memory, requires guards, event emission, and KV persistence for the wiki content manager (#60)
- **FORGE Wiki project** — dogfooding showcase web application in `examples/wiki/` demonstrating all 14 language primitives with Tailwind CSS + DaisyUI UI, content seeding, stub agents/flows/pools, warden supervision, and system wiring (#59)
- **`data.get(key)` capability** — retrieve values from persistent KV storage, returns `Unit` for missing keys (#59)
- **`data.list(prefix)` capability** — list all keys matching a prefix from persistent KV storage, returns `Array` (#59)
- **`data.delete(key)` capability** — remove entries from persistent KV storage (#59)
- **`data.store` runtime dispatch** — `data.store(key, value)` now executes against redb-backed storage in `forge serve` context (#59)
- **Root URL redirect** — `GET /` now redirects to `/home` for wiki-style applications (#59)
- **Automatic storage provisioning** — `forge serve` creates a `.forge-data/server.redb` database alongside the served file for `data.*` capabilities (#59)
- **FORGE syntax highlighting** — Prism.js language definition for FORGE at `examples/wiki/static/js/forge-highlight.js` (#59)
- Wiki content: getting started guide, first principles reference, roadmap, and reference docs for task, agent, flow, pool (#59)
- **Webhook support** — `POST /webhook/{endpoint}` routes dispatch inbound HTTP requests to endpoint handlers with JSON body parsing, Content-Type validation (must be `application/json`), and optional HMAC-SHA256 signature verification via `X-Hub-Signature-256` header (#52)
- **`emit` in endpoint handlers** — `emit` statements now work outside agent context when an event bus is attached, enabling webhook endpoints to publish events that agents subscribe to (#52)
- **`[server.webhook_secrets]` config** — per-endpoint HMAC secrets for webhook signature verification with constant-time comparison (#52)
- **Multi-file `forge serve`** — `forge serve entry.forge -s dep1.forge -s dep2.forge` merges source files before serving, enabling multi-file projects with endpoint declarations
- **forge-sensei web interface** — multi-file project (`workflows/forge-sensei/`) with status dashboard, query form, code review, and webhook endpoints using DaisyUI-styled HTML rendering

### Changed
- Rewrite forge-sensei as flagship FORGE application — fix 7 correctness bugs (recall dispatch, state transitions, degenerate confidence, dead events, timer no-op, fallback lie, specialist lifecycle), add 8 FORGE constructs (type records, flows, contract, match, requires, pipe, try/or, for loops), objective assessment via predict_outcome + check_prediction
- Harden sensei scripts: build-sensei.sh (skip-if-unchanged, smoke test), pretrain-sensei.sh (error capture, jq, idempotency, --force/--dry-run), consult-sensei.sh (jq, content caching, thresholds), assess.sh (per-category scoring, trend tracking, --json)

### Added
- **HTTP client capabilities** — `web.fetch(url)` for GET requests and `web.post(url, body)` for POST requests, returning response body as Text; errors produce catchable failures for use with `try ... or` (#51)
- **Web search implementation** — `search "query"` now dispatches to a configurable search provider (SearXNG) instead of returning an empty list; results include title, url, and snippet fields (#51)
- **`[web]` config section** — configurable timeout, max redirects, search provider, API key, and search URL with env var expansion (#51)
- **Boundary enforcement for HTTP** — `web.fetch()`, `web.post()`, and `search` are restricted to `boundary: server` files (#51)
- **`markdown.render(content)` built-in capability** — converts Markdown to HTML using pulldown-cmark with tables, footnotes, strikethrough, and task lists; `forge` code blocks get `class="language-forge"` for Prism.js syntax highlighting (#49)
- **Hot-reload development mode** via `forge serve --watch` — watches `.forge` files for changes and hot-swaps the endpoint handler without dropping connections; parse/check errors display in terminal while server keeps running with previous version; config file changes trigger full server restart (#47)
- **Static file serving** with configurable root directory and URL prefix via `[server.static]` config, powered by tower-http `ServeDir` (#46)
- **`asset()` built-in function** returns prefixed URL path for static assets, e.g. `asset("css/style.css")` → `/static/css/style.css` (#46)
- **Default static directory scaffold** with Tailwind CSS CDN, DaisyUI components, Prism.js FORGE syntax highlighting, and demo page (#46)
- `Html` builtin type with automatic `text/html` Content-Type inference from endpoint return type annotations (#44)
- Safe record field access — missing fields return `Unit` instead of crashing, enabling graceful handling of optional request query/header parameters (#44)
- Auto-escaping template interpolation in Html context for XSS prevention (#45)
- `{!expr}` raw interpolation syntax for trusted HTML insertion — bypasses auto-escaping in Html context (#45)
- `html.layout(title, body)` and `html.escape(text)` built-in capabilities for HTML document composition (#45)
- Exhaustive forge-sensei test suite: parser, checker, and runtime conformance tests + Rust integration tests
- `scripts/sensei-smoke-test.sh` for end-to-end integration testing
- `scripts/sensei-cache.sh` for knowledge store and cache management (clean/reset/stats)

### Added
- `deep_dive(topic)` handler on forge-sensei — spawns specialist apprentice agents with filtered knowledge by category, find-before-spawn dedup, `LearnedInsight` subscription for ongoing learning, confidence cap on derived knowledge (#88)
- `system` declaration runtime semantics — system blocks now serve as the orchestration root, creating shared event bus and instance registry, spawning initial agents from the use-block, wiring event routing from compose expressions (`a >> b`), integrating with wardens for supervised agents, and enforcing resource limits via `[system]` config section (#87)
- `retire` statement for graceful agent lifecycle termination — `retire` (self), `retire "alias"` (by alias), with optional `with knowledge export: "path.json"` to preserve knowledge as ForgePackage before exit; unregisters from instance registry (Principle VIII — Accountability) (#86)
- Dynamic warden `adopt()`/`release()` methods for runtime supervision management of spawned agents — `adopt` adds an agent to the manages list, `release` removes and clears retry state (#86)
- `find` expression for runtime agent instance discovery — `find "alias"` returns a single Record, `find all template` returns an Array, `find all template where lifecycle == state` filters by lifecycle state; queries `InstanceRegistry`, forbidden in `pure` functions (#84)
- `spawn` statement for creating agent instances at runtime — `child = spawn agent as "alias"` with `with knowledge where category == "X"`, `with confidence_cap: 0.8`, `with memory field: value` options; registers in instance registry, transfers filtered knowledge with confidence decay (Principle I), warns on missing failure policy (Principle VII) (#83)
- `category:` suffix on `learn` statements for categorized knowledge storage — `learn "fact" category: "boundary"`, `learn from interaction(...) category: "troubleshooting"`, `learn from document(...) category: "docs"` (#85)
- Agent instance registry: `InstanceRegistry` tracks living agent instances at runtime for discovery and composition — register/unregister/find_by_name/find_all, shared via `Arc<RwLock>`, integrated into `WardedRuntime` and `TaskExecutor` (#82)
- Knowledge categories: `category` field on `KnowledgeEntry`, `learn_direct_categorized()`, `export_by_category()`, `export_above_confidence()`, and `export_filtered()` for domain-specific knowledge transfer (#81)
- `memory persistent` keyword for agents — typed memory fields survive process restarts via write-through to ACID storage (#57)
- `ForgeStorage`: redb-backed key-value store with ACID transactions — persistence foundation for agents and future `data.store`/`data.get` capabilities (#48)
- `forge build` command: package FORGE programs as standalone CLI binaries with embedded sources and runtime (#74)
- Multi-file composition: `forge.project.toml` manifest, `merge_programs()` engine, cross-file symbol dedup, single fn main/system enforcement
- `forge run --manifest` for multi-file execution without building a binary
- Generated agent binaries with clap subcommands per handler, interactive REPL, and `--help`
- Config embedding: `--embed-config` bakes a `forge.config.toml` into built binaries with runtime override support
- `--dry-run` mode for build validation without compilation
- `forge-sensei`: FORGE language learning agent written in FORGE, with knowledge store, mastery progression (novice→expert), conformance-based assessment, and Claude Code integration via hook + skill
- `scripts/build-sensei.sh` convenience script to build the forge-sensei binary

### Changed
- forge-sensei now runs as a standalone binary (`bin/forge-sensei`) instead of `cargo run -- send` — faster hook, pre-training, and assessment (#77)
- forge-sensei uses multi-hop recall: `categorize_for_recall` task identifies relevant knowledge categories before `recall`, improving retrieval accuracy from 97% to 100% on conformance tests (#79)
- `forge send` CLI command for non-interactive agent message dispatch
- Agent portability: `exportable` modifier, `import ... from ... as` declaration, `.forgepkg.json` package format with SHA-256 integrity, `forge export`/`import`/`inspect` CLI commands (#72)
- Progressive learning agents: `knowledge`, `recall`, and `learn` language primitives for persistent, searchable knowledge stores that enable agents to accumulate expertise through use
- Development lifecycle workflow spec (`workflows/dev-cycle.forge`) — dogfoods FORGE to model the issue-to-merge cycle with 5 expert agents, 2 state machines, and warden supervision
- `CLAUDE.md` with project development rules (branching, changelog, release conventions)
- Pre-commit workflow gate hook (`scripts/check-workflow.sh`) — enforces branch ancestry, changelog staged, and tests passing
- `forge-workflow` Claude Code skill for guided development sessions
- Core language with 14 primitives: task, flow, agent, pool, system, uncertain, pure, event, states, timer, boundary, requires, warded, when
- PEG parser with comprehensive error diagnostics
- Semantic checkers: purity, boundary, states, requires, warden supervision
- Runtime executor with async support
- LLM provider support: Anthropic, OpenAI-compatible, Ollama, Groq
- Token cost tracking and estimation
- Conformance test suite
- End-to-end acceptance tests
- Fleet code generation
- Interactive agent REPL

