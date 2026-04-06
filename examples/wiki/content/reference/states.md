# states

Lifecycle state machines with compiler-enforced transitions.

## Syntax

```forge
states <Name>
  <from> -> <to> when <guard>
  ...
```

## Description

A `states` declaration defines a finite state machine. Agents bind to a lifecycle via `lifecycle: <StatesName>`, and the compiler enforces that only declared transitions are allowed. Illegal transitions are compile-time errors, not runtime surprises.

## Example

```forge
states PageLifecycle
  draft -> review when content_complete
  review -> published when approved
  review -> draft when needs_revision
  published -> archived when archive_requested
  archived -> draft when restore_requested
```

## Transitioning

Inside an agent handler, use `transition to`:

```forge
agent content_manager
  lifecycle: PageLifecycle

  on publish_page(slug: Text)
    requires lifecycle == review on fail: give "must be in review"
    transition to published
    give "Published: {slug}"
```

## Key Properties

- The initial state is the first `from` state in the first transition
- The compiler rejects transitions not declared in the `states` block
- `requires lifecycle == <state>` guards ensure correct preconditions
- State machines are stored per-agent instance
- Persistent agents restore their lifecycle state from storage on restart

## See Also

- [agent](/docs?slug=agent) — agents that use lifecycle state machines
- [requires](/docs?slug=requires) — precondition guards for transitions
- [warden](/docs?slug=warden) — supervision reacts to agent state
