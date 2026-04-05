# pool

Worker fleet management with configurable resolution strategies.

## Syntax

```forge
pool <name>
  workers: <TaskName> * <count>
  strategy: <fastest|majority|round-robin>
  timeout: <duration>
```

## Description

Pools distribute work across multiple workers and resolve results according to a strategy. This enables ensemble reasoning — multiple LLM calls that vote or race to reduce hallucination and improve reliability.

## Example

```forge
task fact_checker
  needs claim: Text
  gives Text
  do
    result = reason "Is this claim accurate? YES or NO: {claim}"
    when result.sure -> give result
    when result.unsure -> give "uncertain"
    else -> give "could not verify"

pool fact_checkers
  workers: fact_checker * 3
  strategy: majority
  timeout: 15s
```

Usage:

```forge
verdict = fact_checkers.send("check", "The speed of light is 299,792 km/s")
```

## Strategies

| Strategy | Behavior |
|----------|----------|
| `fastest` | Return the first result that completes |
| `majority` | Wait for all workers, return the consensus answer |
| `round-robin` | Distribute work evenly across workers |

## Key Properties

- Workers run concurrently
- Timeout prevents indefinite waiting
- Budget-aware: token costs are tracked per worker
- Composable with `>>` operator

## See Also

- [task](/docs?slug=task) — worker task definitions
- [flow](/docs?slug=flow) — sequential pipelines
- [warden](/docs?slug=warden) — supervision for worker failures
