# FORGE

**The language for trustworthy AI agent systems.**

FORGE treats LLM calls as oracle queries — not function calls. Every response carries confidence metadata. Every boundary between deterministic and non-deterministic code is enforced at compile time.

## What Makes FORGE Different

Most AI frameworks bolt agents onto existing languages. FORGE was designed from first principles for a world where code collaborates with oracles that can be wrong.

- **Honesty is mandatory.** Every oracle call returns `uncertain<T>`. You handle confidence or the compiler stops you.
- **Determinism has a boundary.** `pure` functions cannot call LLMs. This is enforced, not requested.
- **Agents have lifecycles.** Birth, learning, specialization, retirement — with knowledge preservation.
- **Supervision is built in.** `warden` blocks declare what happens when agents crash, hallucinate, or get stuck.
- **Everything composes.** Flows, agents, pools — all pipeline with `>>`.

## Quick Example

```forge
task summarize
  needs document: Text
  gives Text
  do
    result = reason "Summarize this document: {document}"
    when result.sure -> give result
    when result.unsure -> give "Low-confidence summary: {result}"
    else -> give "Could not summarize with confidence."
```

## Get Started

Read the [Getting Started](/docs?slug=getting-started) guide or explore the [Reference](/docs?slug=task) documentation.
