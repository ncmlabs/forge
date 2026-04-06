# when

Confidence-aware branching for handling uncertain LLM results.

## Syntax

```forge
when <expr>.sure -> <body>
when <expr>.unsure -> <body>
when <expr>.unreliable -> <body>
else -> <body>
```

## Description

Every LLM call in FORGE returns an `uncertain<T>` value tagged with a confidence score. The `when` block branches on that confidence, forcing explicit handling of uncertainty. This is FORGE's core honesty primitive — you cannot silently ignore low-confidence results.

## Confidence Tiers

| Tier | Confidence | Meaning |
|------|-----------|---------|
| `.sure` | >= 0.8 | High confidence — safe to use directly |
| `.unsure` | 0.5 - 0.8 | Medium confidence — use with caveats |
| `.unreliable` | < 0.5 | Low confidence — don't trust |
| `.conflicted` | split consensus | Pool workers disagree |

## Example

```forge
task answer
  needs question: Text
  gives Text
  do
    result = reason "Answer: {question}"
    when result.sure -> give result
    when result.unsure -> give "I'm not fully confident: {result}"
    else -> give "I don't have enough information."
```

## Custom Thresholds

```forge
when result.sure(0.95) -> give result
```

The `.sure(threshold)` form lets you set a custom minimum confidence.

## Key Properties

- `when` blocks are exhaustive — the compiler warns if you don't handle all tiers
- The `else` branch catches everything not matched above
- Confidence is estimated heuristically from the LLM response content
- `when` blocks can appear in tasks, agent handlers, and flow stages
- Only values from LLM calls (reason, classify) carry confidence — deterministic values are always `.sure`

## See Also

- [task](/docs?slug=task) — tasks return uncertain values
- [pool](/docs?slug=pool) — consensus-based confidence from multiple workers
- [pure](/docs?slug=pure) — pure functions always return deterministic (sure) values
