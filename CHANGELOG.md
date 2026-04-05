# Changelog

All notable changes to FORGE will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

