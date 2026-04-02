# FORGE Warden Supervision System — Design Spec

**Issue:** [#24](https://github.com/ncmlabs/forge/issues/24)
**Date:** 2026-04-02
**Status:** Draft

## Context

FORGE needs a supervision system for managing agent lifecycles, failure recovery, and escalation. Unlike Erlang/OTP (which supervises processes that crash), FORGE supervises agents that can be uncertain, hallucinate, get stuck, blow budgets, or lose confidence. The failure modes are fundamentally different, and the supervision strategies must match.

This spec replaces the Erlang-style `one_for_one / one_for_all / rest_for_one` strategies from issue #24 with a FORGE-native design built around four dimensions: failure types, response levels, impact scope, and escalation ladders.

## Core Construct: `warden`

The `warden` is a new top-level FORGE construct — peer to `agent`, `flow`, `task`, `pool`. It owns a group of agents (or other wardens) and enforces failure policies.

**Naming rationale:** "Warden" was chosen over "supervisor" (Erlang baggage), "steward" (too passive), and other candidates. A warden is a medieval/fantasy authority figure who manages and protects a territory — active, authoritative, and FORGE-native.

## The Three-Dimensional Policy Model

Every warden policy is a one-liner with three components:

```
on <failure_type>: <response>, <scope>
```

### Dimension 1: Failure Types

Five failure types specific to oracle-augmented agents:

| Failure Type | Meaning | Detection Mechanism |
|---|---|---|
| `stuck` | Agent looping without progress | Jaccard similarity > 0.8 on responses, unchanged memory, low confidence streak (existing `agent.rs` stuck detector) |
| `crash` | Hard runtime error | Agent process returns `RuntimeError` (provider down, division by zero, etc.) |
| `hallucination` | Output below usable confidence | Confidence < configurable threshold (default: `unreliable`, < 0.5) |
| `budget` | Token allocation exhausted | Token counter exceeds agent's budget limit |
| `timeout` | No response within time limit | Wall-clock timer expires |

### Dimension 2: Response Types (Escalation Ladder)

Four responses, ordered from lightest to most drastic:

| Response | What It Does | Use For |
|---|---|---|
| `nudge` | Help the agent in place — inject a system hint, reset recent memory entries. Agent keeps its identity and memory roots. | `stuck` agents, mild confidence dips |
| `restart` | Kill and respawn with same config. Fresh memory, state machine back to `initial`. | `crash`, `timeout`, corrupted state |
| `replace` | Spawn a new agent, potentially with different config/model. Optionally specify: `replace with: agent_v2`. | `hallucination` (try stronger model), persistent `stuck` |
| `escalate` | Give up on automatic recovery. Emit structured `WardFailure` event to parent warden or human. | Final resort (Principle VII: Human Ceiling) |

### Dimension 3: Impact Scope

Who else is affected when a failure triggers a response:

| Scope | Meaning | Mechanism |
|---|---|---|
| `self` | Only the failing agent. All others continue uninterrupted. | No propagation. |
| `downstream` | Failing agent + all agents that depend on its output. | Warden traces the dependency graph (from `>>` composition and `stage.*` references). Downstream agents pause until recovery completes. |
| `all` | Every agent in this warden's group. | Red button. All agents pause/restart together. |

`downstream` replaces Erlang's `rest_for_one` but follows **data flow** instead of start order.

## Syntax

### Basic Warden Declaration

```
warden intake_line
  manages [classifier, router, validator]

  on stuck: nudge, self
    after 3: restart
  on crash: restart, all
    after 3: escalate
  on hallucination: replace, downstream
  on budget: escalate, self
  on timeout: restart, self
    after 2: replace

  max_retries 5 per 60s then escalate
```

### Per-Agent Overrides (Hybrid Model)

The warden sets group-level defaults. Individual agents can override specific policies:

```
agent classifier
  states
    idle -> classifying -> done

  warden_override
    on stuck: replace, self
    on timeout: nudge, self
```

Agent overrides take precedence over warden defaults for that specific agent and failure type.

### Nested Wardens (Supervision Tree)

Wardens can manage other wardens, forming a tree:

```
warden factory
  manages [intake_line, processing_line, output_line]

  on crash: restart, all
    after 3: escalate

  max_retries 3 per 120s then escalate

warden intake_line
  manages [classifier, router]

  on stuck: nudge, self
  on crash: restart, downstream

warden processing_line
  manages [analyst, enricher, scorer]

  on crash: restart, all
  on budget: escalate, self
```

Escalation chain: agent failure → child warden → parent warden → human.

### Automatic Escalation Ladder

The `after N` syntax enables automatic response promotion:

```
on stuck: nudge, self
  after 3: restart        # 3 total failures → promote to restart
  after 6: escalate       # 6 total failures → give up
```

The `after N` count is **cumulative total failures** for that agent+failure_type combination, not per-response-level. So `after 6` means "6 total stuck failures for this agent," not "3 more after the restart promotion."

The warden climbs the ladder automatically: `nudge → restart → replace → escalate`.

### Group-Level Circuit Breaker

```
max_retries 5 per 60s then escalate
```

Counts total failures across all managed agents within the time window. If the group is too unstable (5 failures in 60 seconds), the warden itself escalates — regardless of individual agent retry counts.

## Runtime Behavior

### Lifecycle

1. **Spawn phase:** Warden starts managed agents in declaration order. Each agent registers with the warden.
2. **Monitoring:** Warden receives failure signals from agents via existing detection mechanisms.
3. **Policy lookup:** On failure, check agent-level `warden_override` first, then warden defaults.
4. **Response dispatch:** Execute the response (nudge/restart/replace/escalate).
5. **Scope enforcement:** Apply scope (self/downstream/all) by pausing/restarting affected agents.
6. **Retry tracking:** Increment counters. Check `after N` thresholds and `max_retries`.

### Scope Enforcement Detail

**`self`:** No propagation. Only the failing agent is affected.

**`downstream`:** The warden maintains a dependency graph derived from `>>` composition between agents. On failure:
1. Trace all agents reachable from the failing agent in the dependency graph.
2. Send `pause` signal to downstream agents.
3. Handle the failing agent's recovery.
4. Once recovered, send `resume` to paused agents.
5. Downstream agents replay from the recovered agent's new output if needed.

**`all`:** Pause all managed agents, handle the failure, resume all from clean state.

### Nudge Mechanics

When a warden nudges an agent:
- Inject a system message into the agent's LLM context (e.g., "try a different approach")
- Optionally reset the last N memory entries to break repetitive loops
- The agent continues with its existing identity, state, and memory roots
- Stuck detection counters reset after a successful nudge

### Replace Mechanics

When a warden replaces an agent:
- Kill the current agent process
- If `replace with: agent_v2` is specified, spawn the replacement agent
- If no replacement specified, spawn same config (equivalent to restart)
- The replacement receives the original agent's pending work/input

## Integration with Existing FORGE Constructs

### `system` Declaration

The top-level warden is wired into the `system` block as the root of the supervision tree:

```
system customer_support
  warden: factory
  providers
    anthropic "claude-sonnet-4-5-20250514"
```

### Event Bus

The warden uses the existing `SharedEventBus` for:
- Receiving failure signals from managed agents
- Emitting `WardFailure` escalation events to parent wardens
- Broadcasting `WardAction` trace events

### Tracing (Principle VIII)

Every warden decision produces a structured trace event:

```
WardAction {
  warden: "intake_line",
  agent: "classifier",
  failure_type: stuck,
  response: nudge,
  scope: self,
  retry_count: 2,
  timestamp: ...,
}
```

### States Machine

When a warden restarts or replaces an agent, the agent's state machine resets to its `initial` state.

### Confidence System

The `hallucination` failure type integrates with FORGE's existing `ConfidentValue` and confidence predicates (`sure`, `unsure`, `unreliable`). The warden monitors agent output confidence and triggers the policy when output falls below the threshold.

## Principles Alignment

| Principle | How Warden Satisfies It |
|---|---|
| **V. Supervision** | "Write the happy path. Declare failure policy." The warden IS the declared failure policy. |
| **I. Honesty** | Explicit scope on every policy line — no hidden defaults. You read the policy, you know what happens. |
| **VII. Human Ceiling** | `escalate` is a first-class response. The ladder always ends at human. |
| **VIII. Accountability** | Every warden decision is traced. Full audit trail of failures and responses. |
| **IV. Composition** | Wardens compose into trees. Wardens manage agents, flows, and other wardens uniformly. |
| **VI. Self-Reference** | Three concepts (failure type, response, scope), one pattern (`on X: Y, Z`). Learnable from a single example. |

## What This Replaces

The original issue #24 proposed Erlang-style strategies:

| Erlang Strategy | FORGE Equivalent |
|---|---|
| `one_for_one` | `on <failure>: <response>, self` |
| `one_for_all` | `on <failure>: <response>, all` |
| `rest_for_one` | `on <failure>: <response>, downstream` (follows data flow, not start order) |
| Max restart intensity | `max_retries N per Ts then escalate` |

FORGE goes further with: multiple failure types (not just crash), graduated response ladder (not just restart), and automatic escalation.

## Verification Plan

1. **Grammar:** Add `warden` as a top-level construct in `grammar/forge.pest`
2. **Parser:** Parse warden declarations into AST nodes
3. **Checker:** Validate that managed agents exist, failure types are valid, response/scope enums are correct
4. **Runtime:** Implement `Warden` struct in `src/runtime/warden.rs` with monitoring, policy dispatch, scope enforcement, and retry tracking
5. **Tests:**
   - Unit: warden policy lookup with overrides
   - Unit: retry counting and ladder escalation
   - Unit: scope enforcement (self, downstream, all)
   - Integration: mock agents that crash/get stuck on demand
   - Integration: nested warden tree with escalation chain
   - Acceptance: full system with warden managing agents through multiple failure/recovery cycles
