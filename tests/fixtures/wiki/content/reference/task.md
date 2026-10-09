# task

The fundamental unit of LLM-powered computation in FORGE.

## Syntax

```forge
task <name>
  needs <param>: <Type>, ...
  gives <Type>
  do
    <body>
```

## Description

A `task` encapsulates an oracle query. Every task invocation returns `uncertain<T>` — a value tagged with a confidence score and source metadata. You must handle uncertainty with `when` blocks before the value can be used in deterministic context.

## Example

```forge
task summarize
  needs document: Text
  gives Text
  do
    result = reason "Summarize this document concisely: {document}"
    when result.sure -> give result
    when result.unsure -> give "Low-confidence summary: {result}"
    else -> give "Could not summarize."
```

## Failure Handling

Tasks can declare failure behavior with `if_fails`:

```forge
task risky_analysis
  needs data: Text
  gives Text
  if_fails: give "Analysis unavailable"
  do
    give reason "Analyze: {data}"
```

## Key Properties

- Returns `uncertain<T>` — confidence tracking is automatic
- Can call `reason`, `classify`, `search` capabilities
- Can call other tasks and pure functions
- Cannot be called from `pure` functions (enforced at compile time)
- Budget-tracked: token usage is estimated before execution

## See Also

- [pure](/docs?slug=pure) — deterministic functions
- [flow](/docs?slug=flow) — multi-stage pipelines
- [when](/docs?slug=when) — confidence-aware branching
