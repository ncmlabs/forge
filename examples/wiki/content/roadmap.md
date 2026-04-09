# Roadmap

FORGE is built in four layers. Each layer enables the next.

## Layer 1: The Substrate (90% complete)

The core language, compiler, and runtime. 14 primitives, 7 semantic checkers, async runtime, HTTP server, persistent storage, knowledge management.

**What you can build today:**
- Standalone agent CLIs from a single `.forge` file
- Multi-agent systems with shared event buses
- Knowledge-driven agents that learn from interactions
- HTTP servers with endpoint routing and webhooks
- Provider-independent code (swap LLM via config)

## Layer 2: Toolkit Agents

Agents that write FORGE code from specifications. The conformance test suite ensures generated code is valid.

**Prerequisites:** Layer 1 complete + conformance suite.

## Layer 3: Automation Factory

Specification to running system — fully automated. Give it a description, get a deployed multi-agent system.

**Prerequisites:** Layer 2 complete.

## Layer 4: Self-Improvement

The factory optimizes itself. Agents analyze their own performance, identify bottlenecks, and generate improvements.

**Prerequisites:** Layer 3 complete.

## Current Focus

Track C: Building the FORGE Wiki as a dogfooding showcase — a real web application that demonstrates every language primitive.
