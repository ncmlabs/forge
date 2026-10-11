# {{name}}

A FORGE project scaffolded by `forge init --template pipeline`.

## What this is

`main.forge` declares `flow brief`: `gather_notes` and `gather_refs` have no
`needs` clause, so they run in the same wave in parallel; `synthesize` waits
for both and dispatches the merged result with `when`.

## The loop

```bash
forge check --json main.forge                # parse + resolve + check, zero diagnostics
forge test main.forge --expect expected.txt  # replay the recorded fixture, assert stdout ($0)
forge run main.forge                         # live run on the provider in forge.config.toml
```

Round trip: `forge check` first and fix every diagnostic, then `forge test`
(replay, no provider call), then `forge run`.

`forge.config.toml` defaults to the mock provider — no API key, no tokens, $0.
`forge test` replays `main.forge.fixtures.json` instead of calling a provider.
Under mock, every oracle call returns the same canned text, so `expected.txt`
records that text; switch to a real provider and run
`forge run main.forge --record` to re-record both the fixture and real output.
The fixture is a lockfile: committed, so an output change shows up as a diff.

## Where to go next

- The FORGE card (one page, the syntax that trips agents):
  https://github.com/ncmlabs/forge/blob/main/docs/forge-card.md
- `llms.txt`, the machine entry point:
  https://github.com/ncmlabs/forge/blob/main/llms.txt
