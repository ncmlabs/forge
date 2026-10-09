# The FORGE Card

> One page of FORGE for agents: the syntax that trips you up, one idiom per primitive, and the errors you will actually hit. `docs/forge-reference.md` is the full authority; `grammar/forge.pest` wins any disagreement.

## 1. What FORGE is

FORGE is a language for oracle-augmented computation: LLM calls are primitives, uncertainty is a compile-time type, and independent work runs in parallel. Deterministic logic lives in `pure`, stochastic logic in `task`, and agents are first-class with memory, lifecycle states, events, and supervision.

Authoring loop, in this order: `forge check app.forge` (parse + resolve + check — fix every error first) → `FORGE_MOCK=1 forge run app.forge` (the whole program on the mock provider: no API key, no tokens, no cost) → `forge run app.forge` (only once the mock run is clean).

## 2. Syntax rules that trip agents

- **Exact 2-space indentation.** Levels are 2 / 4 / 6 / 8 spaces (i1–i4) and nesting stops at i4; 3 spaces or a tab is a parse error.
- **No braces, no semicolons, no parentheses around conditions.** Newlines separate statements.
- **No blank lines inside a body.** A blank line ends the block, and the indented line after it fails to parse.
- **`needs` / `gives` / `do`.** `needs name: Type` (the type is required), `gives Type`, body inside `do` at i1 → i2. `fn main` has none of them: its body sits at i1.
- **Template strings.** `"{x}"` interpolates at parse time; `\{` and `\}` are literal braces; `\n`, `\t`, `\"`, `\\` are the other escapes.
- **`when` takes an arrow, not a block.** `when x.sure -> give x` is one line, with `else -> ...` as a sibling; only `if`, `for`, and `match` take an indented body.

## 3. The determinism boundary

`task` may call oracles and perform effects; `pure` may not — it is deterministic and always returns confidence 1.0. Forbidden in `pure`: `reason`, `classify`, `search`, `recall`, `exec`, `command`, `session`, `file.read`, `skill.*`, `escalate`, `try ... or`, and any call to a `task`.

Wrong — `reason` in a pure function:
```forge-error
pure summarize
  needs text: Text
  gives Text
  do
    give reason "summarize {text}"
```
Right — deterministic work only:
```forge
pure summarize
  needs text: Text
  gives Text
  do
    give "summary: {text}"
```

## 4. Uncertainty: bind, then dispatch

`reason`, `classify`, `search`, `recall`, `exec`, `command`, `session`, `file.read`, `toml.parse`, `json.parse`, and every `skill.*` call return an uncertain value; taint survives assignment and field access. Bind the result to a name, then dispatch it with `when <name>.sure -> ...` / `.unsure` / `else`, or with `match`. Never `give` the raw result, and never inline an oracle in `give`: `give reason "..."` fails with "unhandled uncertain: oracle result given without confidence dispatch". Levels: `sure` (≥ 0.8), `unsure` (0.5–0.8), `unreliable` (< 0.5), `conflicted`; `when x.sure(above: 0.9)` overrides the threshold.

Wrong — `give` of a raw oracle result:
```forge-error
task analyze
  needs topic: Text
  gives Text
  do
    result = reason "analyze {topic}"
    give result
```
Right — every path acknowledges the confidence:
```forge
task analyze
  needs topic: Text
  gives Text
  do
    result = reason "analyze {topic}"
    when result.sure -> give result
    when result.unsure -> give "uncertain: {result}"
    else -> give "unknown"
```
