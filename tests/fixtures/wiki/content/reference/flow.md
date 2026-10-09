# flow

Multi-stage pipelines with automatic parallelism.

## Syntax

```forge
flow <name>
  needs <param>: <Type>
  gives <Type>

  stage <name>
    <body>

  stage <name>
    needs <prev_stage>.<var>
    <body>
```

## Description

Flows define multi-stage pipelines where the compiler infers which stages can run in parallel. Stages without data dependencies execute concurrently. Stages that need outputs from previous stages wait automatically.

## Example

```forge
flow review_code
  needs code: Text
  gives Text

  stage detect
    lang = detect_language(code)

  stage quality
    needs detect.lang
    report = analyze_quality(code, detect.lang)

  stage security
    needs detect.lang
    report = analyze_security(code, detect.lang)

  stage verdict
    needs quality.report, security.report
    give format_report(quality.report, security.report)
```

In this example, `quality` and `security` stages run in parallel after `detect` completes.

## Key Properties

- Stages without dependencies run concurrently
- DAG inference: the compiler builds a dependency graph automatically
- Final stage determines the flow's return value
- Can compose with `>>` operator

## See Also

- [task](/docs?slug=task) — individual computation units
- [pool](/docs?slug=pool) — worker fleets
