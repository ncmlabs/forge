# agent

Stateful processes with lifecycle, memory, event handling, and knowledge stores.

## Syntax

```forge
agent <name>
  lifecycle: <StatesName>
  memory
    <field>: <Type>
  timer <name>: <duration>
  subscribe <EventName>

  on start
    <body>

  on <handler>(param: Type)
    <body>

  on <EventName>
    <body>

  on timeout
    <body>
```

## Description

Agents are the core building block for stateful AI processes. They maintain memory across interactions, follow lifecycle state machines, subscribe to events, and can learn from interactions via knowledge stores.

## Example

```forge
agent assistant
  lifecycle: AssistantPhase
  memory
    message_count: Number
    last_topic: Text
  timer idle_timeout: 10m
  subscribe UserMessage

  on start
    say "Assistant ready."
    memory.message_count = 0

  on message(text: Text)
    memory.message_count = memory.message_count + 1
    response = reason "Reply to: {text}"
    when response.sure -> say response
    else -> say "I'm not confident in my response."

  on UserMessage
    say "Received event."

  on timeout
    say "Session timed out."
```

## Key Features

- **Memory**: Typed fields that persist across handler invocations
- **Persistent memory**: `memory persistent` fields survive process restarts via ACID storage
- **Lifecycle**: State machines declared with `states`, transitions enforced at compile time
- **Knowledge**: `learn`/`recall` for accumulating expertise over time
- **Events**: Subscribe to typed events from other agents
- **Timers**: Scheduled events with start/cancel/reset
- **Stuck detection**: Automatic detection when agent makes no progress
- **Exportable**: `exportable agent` can be packaged and imported

## See Also

- [states](/docs?slug=states) — lifecycle state machines
- [event](/docs?slug=event) — typed inter-agent messages
- [warden](/docs?slug=warden) — agent supervision
