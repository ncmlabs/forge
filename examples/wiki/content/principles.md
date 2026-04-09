# The Nine First Principles

FORGE is built on nine non-negotiable principles. These are not aspirations — they are enforced by the compiler and runtime.

## I. Honesty

Every oracle call returns `uncertain<T>`. There is no way to silently treat an LLM response as deterministic truth.

```forge
result = reason "What is 2+2?"
# result is uncertain — you MUST handle it
when result.sure -> say "Confident: {result}"
when result.unsure -> say "Not sure: {result}"
else -> say "Cannot determine"
```

## II. Determinism Boundary

`pure` functions cannot call LLMs. The compiler enforces this at compile time.

```forge
pure format_report
  needs data: Text
  gives Text
  do
    # This is guaranteed deterministic — no oracle calls possible
    give "=== Report ===\n{data}"
```

## III. Token Economy

Budget limits are language-level. The compiler tracks cost before any LLM is called. `forge cost` estimates token usage statically.

## IV. Composition Completeness

Everything works with the `>>` operator. Flows, agents, pools — all composable, all parallelizable.

```forge
content >> search    # Content changes trigger search re-indexing
content >> docs      # Content changes trigger doc regeneration
```

## V. Supervision

Declare failure policy, not recovery code. `warden` blocks handle crashes, hallucinations, stuck states, and budget overruns.

```forge
warden supervisor
  manages [agent_a, agent_b]
  on crash: restart, self
    after 3: escalate
  on hallucination: restart, self
  on stuck: nudge, self
```

## VI. Self-Reference

The language is writable by agents. FORGE programs can generate FORGE programs — the bridge to Layer 2 (toolkit agents).

## VII. Human Ceiling

`escalate to human` is the safe default. When agents cannot resolve a situation, they must have a path to human oversight.

## VIII. Accountability

All decisions are traced automatically. The runtime records every oracle call, every confidence score, every state transition.

## IX. Boundary

Server/client code separation is enforced at compile time. LLM calls and data persistence are restricted to `boundary: server` files.
