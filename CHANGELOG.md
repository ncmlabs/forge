# Changelog

All notable changes to FORGE will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- `forge-sensei`: FORGE language learning agent written in FORGE, with knowledge store, mastery progression (novice→expert), conformance-based assessment, and Claude Code integration via hook + skill
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

