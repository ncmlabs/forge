# {{name}}

A FORGE project scaffolded by `forge init --template agent`.

## What this is

`main.forge` declares `agent greeter`: lifecycle `states`, `memory`, and an
`on start` handler guarded by `requires lifecycle == idle`. `fn main` spawns
the agent and sends it its first message through `with memory topic: "..."`;
a spawned agent's `on start` fires as soon as it comes up.

## The loop

```bash
forge check --json main.forge                # parse + resolve + check, zero diagnostics
forge test main.forge --expect expected.txt  # replay the recorded fixture, assert stdout ($0)
forge run main.forge                         # live run on the provider in forge.config.toml
```

Round trip: `forge check` first and fix every diagnostic, then `forge test`
(replay, no provider call), then `forge run`.

`forge.config.toml` defaults to the mock provider — no API key, no tokens, $0.
Under mock, every oracle call returns the same canned text, so `expected.txt`
records that text; switch to a real provider and run
`forge run main.forge --record` to re-record both the fixture and real output.

## Where to go next

- The FORGE card (one page, the syntax that trips agents):
  https://github.com/ncmlabs/forge/blob/main/docs/forge-card.md
- `llms.txt`, the machine entry point:
  https://github.com/ncmlabs/forge/blob/main/llms.txt
