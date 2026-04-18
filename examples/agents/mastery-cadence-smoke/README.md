# mastery-cadence-smoke

T5 scheduler MVP live smoke test for issue #332.

## What it proves

`schedule mastery_review` declared with `when: every 30s` and `mode: spawn` fires
on cadence via the `WakeService` subsystem. Each fire lands on the event bus
as a normal `mastery_review` event; the agent's `on mastery_review(prompt: Text)`
handler runs identically to any other bus handler. No agent-side changes are
needed to consume a scheduled event — that's the load-bearing design contract
from #332.

State persists in redb (under `.forge-data/`), so a mid-run restart enters the
catchup sweep (policy `once`): exactly one fire catches up, then the regular
cadence resumes.

## Acceptance checks

- [ ] 5–6 `schedule_fired` tracer events across a 3-minute run.
- [ ] `memory.tick_count` monotonically advances.
- [ ] Kill & restart the process mid-run: no duplicate fires, no missed ticks.

## Run

```bash
cargo run --bin forge -- serve \
  --manifest examples/agents/mastery-cadence-smoke/forge.project.toml \
  examples/agents/mastery-cadence-smoke/main.forge --port 3232

# In another terminal, tail the trace:
curl -sN http://localhost:3232/__forge/events | grep schedule_fired
```
