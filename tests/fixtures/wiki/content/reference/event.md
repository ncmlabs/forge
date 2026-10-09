# event

Typed messages for inter-agent communication.

## Syntax

```forge
event <Name>
  <field>: <Type>
  ...
```

## Description

Events are typed records that agents use to communicate. An agent `emit`s events, and other agents `subscribe` to receive them. Events enable loose coupling between agents — the emitter doesn't need to know who listens.

## Example

```forge
event OrderPlaced
  order_id: Text
  customer: Text
  total: Number

agent order_processor
  subscribe OrderPlaced

  on OrderPlaced
    say "Processing order {OrderPlaced.order_id}"
```

## Emitting Events

Inside an agent handler:

```forge
on create_order(customer: Text, total: Number)
  emit OrderPlaced(order_id: "ORD-001", customer: customer, total: total)
```

## Subscribing

Agents declare subscriptions at the top level:

```forge
agent listener
  subscribe OrderPlaced
  subscribe OrderCancelled

  on OrderPlaced
    say "New order received."
```

## Key Properties

- Events are typed — the compiler verifies field names and types
- Events propagate through the shared event bus within a `system`
- Multiple agents can subscribe to the same event
- Events are fire-and-forget (no return value)
- Events trigger agent handlers asynchronously

## See Also

- [agent](/docs?slug=agent) — event producers and consumers
- [system](/docs?slug=system) — shared event bus
- [states](/docs?slug=states) — events can trigger state transitions
