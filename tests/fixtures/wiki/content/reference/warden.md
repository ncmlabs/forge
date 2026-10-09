# warden

Supervision trees that manage agent failures with typed policies.

## Syntax

```forge
warden <name>
  manages [<agent1>, <agent2>, ...]

  on <failure_type>: <response>, <scope>
    after <N>: <escalated_response>

  max_retries <N> per <duration> then escalate
```

## Description

A `warden` supervises a group of agents, detecting failures and responding according to typed policies. When an agent crashes, gets stuck, hallucinates, exceeds its budget, or times out, the warden applies the appropriate response — from gentle nudging to full escalation.

## Failure Types

| Type | Trigger |
|------|---------|
| `crash` | Agent handler throws a runtime error |
| `stuck` | Agent repeats the same response pattern |
| `hallucination` | Repeated low-confidence responses |
| `timeout` | Handler exceeds time limit |
| `budget` | Token budget exhausted |

## Response Actions

Ordered by severity: `nudge < downgrade < restart < replace < escalate`

| Response | Effect |
|----------|--------|
| `nudge` | Prompt the agent to try differently |
| `downgrade` | Reduce agent capability |
| `restart` | Kill and restart the agent |
| `replace` | Swap in a fresh agent instance |
| `escalate` | Remove agent, degrade gracefully |

## Scope

| Scope | Effect |
|-------|--------|
| `self` | Only the failing agent is affected |
| `downstream` | Failing agent + agents after it in the pipeline |
| `all` | All managed agents are restarted |

## Example

```forge
warden wiki_supervisor
  manages [search_agent, content_manager, qa_agent]

  on crash: restart, self
    after 3: escalate

  on stuck: nudge, self
    after 5: restart

  on hallucination: restart, self
    after 3: escalate

  on timeout: restart, self

  on budget: downgrade, self
    after 2: escalate

  max_retries 10 per 1h then escalate
```

## Escalation Ladder

The `after N:` clause defines escalation thresholds. The response severity must increase:

```forge
on crash: nudge, self      # First crash: nudge
  after 3: restart          # 3rd crash: restart
  after 5: escalate         # 5th crash: give up
```

The compiler enforces that escalation responses are strictly more severe.

## Circuit Breaker

`max_retries N per <duration> then escalate` limits total failures across all agents in the group. If exceeded, the warden escalates globally — preventing cascade failures.

## Key Properties

- Wardens are compile-time checked: managed agent names must exist
- Escalation severity ordering is enforced by the compiler
- Graceful degradation: escalated agents are removed but the system continues
- Agents can override warden policies with `warden_override` blocks
- All warden actions are visible in trace output (Principle VIII — Accountability)

## See Also

- [agent](/docs?slug=agent) — the processes being supervised
- [system](/docs?slug=system) — wiring agents into a supervised group
- [states](/docs?slug=states) — agent lifecycle transitions
