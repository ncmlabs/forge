# Changelog

All notable changes to FORGE will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
- Anthropic provider now sends tool definitions and parses `tool_use` response blocks, enabling skill executor agentic loop (#198)
- OpenAI-compatible provider now sends tool definitions and parses `tool_calls` response fields (#198)

### Added
- Session polling + event integration: `SessionManager` publishes typed events (`session.progress`, `session.complete`, `session.failed`, `session.cancelled`) directly to `EventBus`, enriched with timestamp, message, duration, and final status fields (#192)
- Timer-based session polling with configurable interval (default 5s) — publishes `session.poll` events for active sessions to EventBus (#192)
- `session.status(id)` imperative expression — query session state from FORGE code, returns Record with `status`, `cost_usd`, `started_at`, `updated_at`, `error` fields (#192)
- Declarative adapter system: `ADAPTER.toml` format for CLI agent integration with field mapping, permission modes, and progress detection (#191)
- Built-in adapters for Claude Code (`adapters/claude/`) and Codex (`adapters/codex/`) with verified CLI patterns (#191)
- `ConfigDrivenDriver` — generic `SessionDriver` that reads any ADAPTER.toml and handles process spawn, stdin piping, stdout parsing, and AgentResult extraction (#191)
- Adapter resolution chain: project local → installed → built-in, with `[adapters]` section in `forge.project.toml` (#191)
- Generic fallback adapter for arbitrary CLI agents (command = agent name, text output, 0.5 confidence) (#191)
- `SessionManager` runtime for long-running external agent sessions with UUID-backed session IDs, persisted JSON state under `.forge-data/sessions/`, startup `resume_all()`, progress listeners, budget enforcement, and graceful cancel escalation (#190)
- `session` runtime execution in `TaskExecutor` with `on progress` / `on complete` hook emission, `Text`/`AgentResult` output coercion, and lifecycle tracing for Principle VIII (#190)
- `session` expression front-end: grammar, AST, parser, checker, resolver, cost estimation, and runtime placeholder support with `agent`, `prompt`, `tools`, `timeout`, `budget`, `gives AgentResult`, and `on progress`/`on complete` emit hooks (#189)
- `AgentResult` built-in type — standard typed result contract for agent/session runs with 9 fields: `plan`, `patch_summary`, `files_changed`, `tests_run`, `tests_passed`, `cost_usd`, `confidence`, `approval_needed`, `metadata` (#193)
- `AgentResult` constructor with default fields: `AgentResult()` or `AgentResult(plan: "fix bug", confidence: 0.9)` — unspecified fields get zero/empty defaults (#193)
- `AgentResult` confidence sync: the `confidence` field value propagates to the `ConfidentValue` wrapper, enabling `when result.sure ->` branching (#193)
- Agent-to-CLI delegation research doc for Claude Code and Codex, including session adapter and AgentResult mapping tables plus live-tested invocation patterns (#165)
- Reference typed SKILL.md wrappers for Claude Code and Codex under `skills/` (#165)
- Rich SKILL.md capability signatures with multi-capability registration and compile-time validation of `skill.namespace.method(...)` calls (#164)
- `forge.project.toml` gains `[skills]` section for project-level skill declarations with `path` and `source` options (#163)
- Skill resolution chain: project local (`./skills/`) → installed (`.agents/skills/`) → global (`~/.forge/skills/`) with clear error on missing (#163)
- `skills-lock.json` integrity verification — warns when installed skill content diverges from lock hash (#163)
- Compile-time validation of `use skill.X` against project-declared skills via `CheckContext::with_skills()` (#163)
- Pure checker rejects `skill.*` calls inside `pure` functions (Principle II — determinism boundary) (#163)
- `command` expression primitive with optional `in`, `timeout`, and `background` modifiers (#160)
- Structured argv form for `command`: `command ["git", "commit", "-m", msg]` — safe, no injection (#160)
- `env` modifier for `command`: `command "build" env { RUST_LOG: "debug" }` (#160)
- `command` runtime execution — synchronous process spawning with structured record return (`stdout`, `stderr`, `exit_code`, `success`) (#161)
- `command` background mode — UUID handle-based lifecycle with `command.status()`, `command.output()`, `command.cancel()` (#162)
- `CommandManager` process manager with incremental output buffering, timeout auto-cancel, and graceful shutdown (#162)

## [0.1.0] - 2026-04-09

First release of FORGE — the agent-native programming language where LLM calls are oracle queries, not function calls. All 14 language primitives, 7 semantic checkers, full async runtime, and 3 showcase applications. 883 tests passing across unit, conformance, integration, and E2E.

### Added

#### Core Language
- Core language with 14 primitives: task, flow, agent, pool, system, warden, event, states, pure, endpoint, type, contract, use, uncertain
- PEG parser (609 rules) with comprehensive error diagnostics and span tracking
- Semantic checkers: purity, boundary, states, requires, uncertain, spawn, warden supervision
- `when` confidence branching (`when result.sure -> ... else ->`) for handling LLM uncertainty
- `try ... or` fallback expressions for graceful error handling
- Pattern matching via `match` expressions
- `for` loops over iterables
- Template strings with `{var}` interpolation and `{!expr}` raw interpolation for Html context
- `.split(delimiter)` string method, `.join(delimiter)` array method, `.length` field access on Text
- Variable reassignment in nested scopes (`for`/`if`/`match` blocks)
- Array concatenation via `+` operator
- Mixed `Text + Html` and `Html + Text` concatenation
- `downgrade` warden response — new severity level between Nudge and Restart
- Safe record field access — missing fields return `Unit` instead of crashing

#### Runtime & Execution
- Async executor (tokio-based) with automatic DAG parallelism in flows
- Agent lifecycle with event loops, memory (persistent + ephemeral), timers, and subscriptions
- Event bus with typed pub/sub for inter-agent communication
- Pool management with round-robin and fastest-wins strategies
- Warden supervision with escalation ladders (nudge → downgrade → restart → replace → escalate)
- State machine validation and lifecycle enforcement
- System runtime orchestration — shared event bus, instance registry, warden snapshots
- `forge serve` boots system runtime (agents + wardens) alongside HTTP server when a `system` declaration is present

#### LLM Integration
- LLM provider support: Anthropic, OpenAI-compatible, Ollama, Groq, Mock
- Token cost tracking and estimation per call and per agent
- Confidence tracking with sources: Deterministic, LLMDirect, ConsensusAgreement, Derived, KnowledgeRecall, ExecResult, SkillInvocation
- Multi-turn tool-use in mock provider for deterministic testing
- Unified tool-use in `CompletionRequest`/`CompletionResponse`

#### Agent Ecosystem
- `memory persistent` keyword — typed memory fields survive process restarts via write-through to ACID storage (redb)
- Progressive learning: `knowledge`, `recall`, and `learn` primitives for persistent, searchable knowledge stores
- Knowledge categories with `category:` suffix on `learn` statements
- Agent instance registry for runtime discovery — `spawn`, `find`, `retire` lifecycle primitives
- Dynamic warden `adopt()`/`release()` for runtime supervision management
- Agent portability: `exportable` modifier, `.forgepkg.json` package format, `forge export`/`import`/`inspect` CLI

#### Web Runtime
- `Html` builtin type with automatic `text/html` Content-Type and auto-escaping for XSS prevention
- `html.layout(title, body)` and `html.escape(text)` built-in capabilities
- `markdown.render(content)` — Markdown to HTML with tables, footnotes, task lists, FORGE syntax highlighting
- HTTP client: `web.fetch(url)` and `web.post(url, body)` with catchable failures
- Web search: `search "query"` with configurable provider (SearXNG)
- Static file serving with `[server.static]` config and `asset()` URL helper
- Hot-reload development mode via `forge serve --watch`
- Webhook support with JSON parsing and optional HMAC-SHA256 signature verification
- `emit` in endpoint handlers for publishing events from webhooks
- Multi-file `forge serve` with `-s` flag for dependencies
- Boundary enforcement: `web.fetch()`, `web.post()`, and `search` restricted to `boundary: server`
- Root URL redirect (`GET /` → `/home`)
- Automatic storage provisioning (`.forge-data/server.redb`)

#### Data & Embeddings
- `data.store(key, value)`, `data.get(key)`, `data.list(prefix)`, `data.delete(key)` — persistent KV storage backed by redb
- Vector embeddings and semantic search — `data.embed(content)` generates vectors via OpenAI-compatible providers; `data.search(query, top_k)` performs cosine similarity search with confidence from similarity scores; `EmbeddingProvider` trait with OpenAI-compatible and Mock implementations; `VectorIndex` with JSON persistence; `[embeddings]` config section; costs tracked in Observer (#50)

#### Skill System
- `exec` primitive — first-class CLI execution returning `uncertain<Text>` with confidence from exit code; enforced by pure checker, uncertain checker, and tracer (#40)
- Host skill bridge — LLM-mediated SKILL.md execution via `skill.namespace.method()` syntax; agentic loop with tool-use; results capped at 0.99 confidence (#40)
- `SkillExecutor` wiring at all `TaskExecutor::new()` call sites; respects `[skills]` config

#### Build System
- `forge build` — package FORGE programs as standalone CLI binaries with embedded sources and runtime
- Multi-file composition via `forge.project.toml` manifest with cross-file symbol dedup
- `forge run --manifest` for multi-file execution without building
- Generated agent binaries with clap subcommands per handler, interactive REPL, and `--help`
- Config embedding: `--embed-config` bakes config into binaries with runtime override support
- `--dry-run` mode for build validation

#### Observer & Introspection
- Live trace stream — `/__forge/events` SSE endpoint streaming all trace events as JSON; `say` statements emit trace events; broadcast channel survives hot-reloads (#138)
- Runtime introspection API — `/__forge/inspect/*` endpoints for agent state, system topology, warden health, storage keys as JSON (#139)
- Agent topology visualization — live D3 force-directed graph (#141)
- Cost and confidence dashboard — live token counts, USD cost, per-agent breakdown, confidence histogram (#142)
- Failure injection API — `POST /__forge/inject/:type` for testing warden policies (stuck, crash, timeout, hallucination, budget) (#143)
- Standalone Observer app — SPA connecting to any FORGE server with tabbed dashboard, D3 topology, cost panel, swim-lane trace timeline (#144)

#### Showcase Applications
- **FORGE Wiki** — dogfooding web app demonstrating all 14 primitives; content agent (CRUD + lifecycle), search agent (confidence gating), doc generation flow (5-stage DAG), fact-checking pool (3-worker majority vote), warden supervision (5 failure policies), Tailwind + DaisyUI UI (#59-#66)
- **Sentinel** — AI-powered repo health dashboard; `exec` for git data, `exec >> reason` composition, skill-based deep analysis, parallel scan flow, 5 HTTP endpoints (#132)
- **Observer** — standalone SPA for live agent tracing, topology, cost tracking (#144)
- **forge-sensei** — self-referential learning agent written in FORGE; knowledge store, mastery progression (novice→expert), specialist apprentices, conformance-based assessment, Claude Code integration

#### Testing & CI
- 883 tests: unit (200+), conformance (85), grammar (104), parser (78), AST (53), checkers (104+), runtime (70+), integration (50+), wiki acceptance (52)
- End-to-end wiki acceptance tests covering all 7 subsystems
- Playwright E2E tests for wiki, observer, and embeddings
- Conformance test suite for language correctness validation
- CI/CD on ubuntu, macos, windows with MSRV 1.85
- FORGE syntax highlighting (Prism.js language definition)

#### Documentation & Tooling
- Wiki documentation: README (quick start, config, endpoints, deployment) and ARCHITECTURE (system diagrams, feature map) (#66)
- `CLAUDE.md` with project development rules
- Development lifecycle workflow spec (`workflows/dev-cycle.forge`)
- `forge-workflow` Claude Code skill for guided development sessions
- forge-sensei scripts: build, pretrain, consult, assess, smoke-test, cache management

### Changed
- forge-sensei rewritten as flagship FORGE application with 8 new constructs and objective assessment
- forge-sensei runs as standalone binary (`bin/forge-sensei`) with multi-hop recall for improved retrieval accuracy

### Fixed
- Wiki Auto Reference & Fact-Check Report UX — fixed Record serialization, improved extraction prompts (#124)
- Wiki Q&A and search grounded in actual doc content, eliminating hallucinated syntax (#122)
- Sentinel dashboard overflow — long text contained with scroll (#140)
- Sentinel analyst missing insight storage (#140)
