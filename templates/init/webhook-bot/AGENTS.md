# {{name}}

A FORGE project scaffolded by `forge init --template webhook-bot`.

## What this is

A `#! boundary: server` program. `triage_payload` classifies an inbound body;
two HTTP entry points publish `WebhookReceived` to `agent triage_bot`, whose
handler stores the label in `memory`:

- `POST /webhook/inbound` — the declared `endpoint`, no auth, for local tests.
- `POST /wake/triage_bot/inbound` — the agent's `webhook` block, HMAC-verified.
  Register its secret first: `forge wake rotate --agent triage_bot --trigger inbound`.

`forge test` opens no socket: `fn main` calls the classify task directly with a
sample payload. `forge serve main.forge` is the real HTTP path.

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
