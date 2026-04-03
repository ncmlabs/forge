# Changelog

All notable changes to FORGE will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
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

