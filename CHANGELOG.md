# Changelog

All notable changes to FORGE will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
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
