# pure

Deterministic functions guaranteed to never call an LLM.

## Syntax

```forge
pure <name>
  needs <param>: <Type>, ...
  gives <Type>
  do
    <body>
```

## Description

A `pure` function is the determinism boundary in FORGE. The compiler statically enforces that no `reason`, `classify`, or other LLM operations appear inside a pure function body — not even transitively through task calls. This gives you a hard guarantee: pure functions always produce the same output for the same input.

## Example

```forge
pure format_price
  needs amount: Number, currency: Text
  gives Text
  do
    if currency == "USD"
      give "${amount}"
    if currency == "EUR"
      give "€{amount}"
    give "{amount} {currency}"
```

## What You Can Do in Pure

- String operations and templates
- Arithmetic and comparisons
- `if`/`else` branching
- Call other `pure` functions
- Access function parameters

## What You Cannot Do in Pure

- `reason` (LLM call)
- `classify` (LLM call)
- Call `task` functions (they may use LLM)
- `emit` events
- Access `memory` or `data.store`

The compiler rejects these at build time, not runtime.

## Key Properties

- Pure functions return `deterministic<T>` — always confidence 1.0
- Used for HTML rendering, formatting, validation, and data transformation
- The compiler traces all call paths to enforce the boundary
- Pure functions cannot be `exportable`

## See Also

- [task](/docs?slug=task) — LLM-powered computation (the opposite of pure)
- [boundary](/docs?slug=boundary) — file-level separation of server/client/shared
- [when](/docs?slug=when) — branching on confidence (never needed in pure)
