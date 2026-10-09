# system

Orchestrate multiple agents into a coordinated unit with shared event bus and composition.

## Syntax

```forge
system <name>
  use
    <alias>: <agent_name>
    ...

  <alias> >> <alias> >> ...
```

## Description

A `system` declaration groups agents together, assigns aliases, and defines data flow between them using the `>>` composition operator. All agents in a system share an event bus, enabling inter-agent communication via `emit` and `subscribe`.

## Example

```forge
system customer_support
  use
    router: intent_router
    handler: ticket_handler
    escalation: escalation_agent

  router >> handler >> escalation
```

This wires three agents: the router classifies incoming requests, the handler processes tickets, and the escalation agent handles failures.

## Key Properties

- Agents share a common event bus within the system
- The `>>` operator defines data flow (output of one feeds input of next)
- Aliases provide readable names for agents within the system
- Systems can be supervised by a `warden`
- Each agent retains its own memory, lifecycle, and handler independence

## See Also

- [agent](/docs?slug=agent) — the building blocks of a system
- [event](/docs?slug=event) — inter-agent communication
- [warden](/docs?slug=warden) — supervision trees
