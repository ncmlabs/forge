# requires

Precondition guards that reject invalid inputs before execution.

## Syntax

```forge
on <handler>(param: Type)
  requires <condition> on fail: give <expr>
  <body>
```

## Description

A `requires` clause validates preconditions at the start of an agent handler. If the condition is false, the `on fail` expression is returned immediately without executing the handler body. This prevents invalid operations from reaching LLM calls or state transitions.

## Example

```forge
agent content_manager
  lifecycle: PageLifecycle

  on create_page(slug: Text, title: Text, content: Text)
    requires slug.length > 0 on fail: give "slug required"
    requires title.length > 0 on fail: give "title required"
    data.store("page:{slug}", content)
    give "Created: {slug}"

  on publish_page(slug: Text)
    requires lifecycle == review on fail: give "must be in review to publish"
    transition to published
    give "Published: {slug}"
```

## Guard Patterns

**Input validation:**
```forge
requires slug.length > 0 on fail: give "slug required"
```

**Lifecycle preconditions:**
```forge
requires lifecycle == draft on fail: give "must be in draft"
```

**Memory-based guards:**
```forge
requires memory.player_count < 2 on fail: give "game full"
```

## Key Properties

- Guards execute before any handler body statements
- Multiple `requires` clauses are checked in order
- The `on fail` expression is the return value when the guard fails
- Guards can reference parameters, `lifecycle`, and `memory`
- Guards are compile-time checked for type correctness

## See Also

- [agent](/docs?slug=agent) — agents use requires in handlers
- [states](/docs?slug=states) — lifecycle preconditions
- [when](/docs?slug=when) — runtime confidence branching (different from requires)
