# mastermind-pattern — Reusable classify-and-dispatch topology

**Issue:** [ncmlabs/forge#295][i295] · **Extracted from:** [#293][i293] (T1.1 walking skeleton) · **Track:** [#292][i292] (Layer 3 — Clone Developer)

[i295]: https://github.com/ncmlabs/forge/issues/295
[i293]: https://github.com/ncmlabs/forge/issues/293
[i292]: https://github.com/ncmlabs/forge/issues/292

A named, reusable template for agent clusters where **one classifier dispatches to N supervised specialists with correlation ids**. Built from the T1.1 walking skeleton after the shape proved out end-to-end; intentionally thinned so the four invariants are visible on a single screen. Zero new runtime primitives per [#292][i292] — pure composition of `subscribe...where`, `emit`, and `warden manages`.

```
POST /ticket {kind, channel, message}
      │
      │  emit TicketInbound
      ▼
  mastermind         ← classify via `reason`, assign task_id = T{n},
      │                 append TaskNode to task_graph
      │  emit TaskRouted(task_id, target_agent, kind, ...)
      ▼
 ┌─────────────────┬─────────────────┐
 │ support_        │ billing_        │   subscribe TaskRouted
 │  specialist     │  specialist     │     where target_agent ==
 │  where=="supp…" │  where=="bill…" │       "<self>"
 │  → say [done]   │  → say [done]   │
 └─────────────────┴─────────────────┘
```

`org_warden` supervises all three. Observer stitches the 3-hop trace automatically via the `event_emit` / `event_delivered` SSE stream.

## When to use / when NOT to use

| Use this pattern when… | Use something else when… |
|---|---|
| You have **≥ 2 specialists** selected by a **classification** step. | A single agent can classify and act in one hop — see [`inbound-triager/main.forge`][triager]. |
| Specialists run **asynchronously** after a routing decision. | Stages run **sequentially** — use `>>` composition / task_id correlation across stages. See [`workflows/dev-cycle/main.forge`][devcycle]. |
| You need a **correlation id** (`task_id`) threaded through every downstream event for Observer stitching and lifecycle tracking. | You're a **fan-in adapter** (many publishers → one translator → external capability) — see [`slack-adapter/main.forge`][adapter]. |
| You want a **single supervised boundary** covering classifier + all specialists. | You're a **human-gated single flow** — use the approval-gate shape. See [`approval-gate/main.forge`][approval]. |

[triager]: ../inbound-triager/main.forge
[devcycle]: ../../../workflows/dev-cycle/main.forge
[adapter]: ../slack-adapter/main.forge
[approval]: ../approval-gate/main.forge

## Pattern ingredients — the four invariants

1. **Classification step.** `reason "..."` (or `classify ... into [...]` once that idiom settles — see [T1.1 design notes][i293]). The mastermind commits to a target via explicit branching (`if kind == "..."`), never by feeding raw LLM output into routing. *reason suggests, code decides.*
2. **Typed routing event with `target_agent: Text`.** Specialists filter with `subscribe <RoutingEvent> where target_agent == "<self>"`. The filter is what makes this a dispatch pattern rather than a broadcast.
3. **Persistent `task_id` + `task_graph`.** `task_id = "T{counter}"` assigned from a persistent `task_counter`; append a `TaskNode` to `task_graph: TaskNode[]` on every dispatch; thread `task_id` through every downstream event. **Never reuse an id.** Observer uses `task_id` to stitch hops across agents.
4. **Single warden** covering mastermind + every specialist. Policies mirror `clone-dev-skeleton/main.forge:452–464` (nudge-then-escalate on stuck/hallucination/contradiction/budget; restart on timeout/crash; rate-limit cap).

## Recipe — add a new specialist

1. **Extend the classification vocabulary.** Add a branch to the mastermind's `on TicketInbound` for the new `kind`:
   ```forge
   if kind == "refund"
     target = "refund"
   ```
2. **Define the specialist.** Subscribe with the target filter:
   ```forge
   agent refund_specialist
     subscribe TaskRouted where target_agent == "refund"
     ...
   ```
3. **Register with the warden.**
   ```forge
   warden org_warden
     manages [mastermind, support_specialist, billing_specialist, refund_specialist]
     ...
   ```
4. **(Optional) Track ownership in the graph.** The `TaskNode.specialist` field is already there — no schema change needed.

### Correlation-id conventions

- **Format:** `T{counter}` from persistent `task_counter`. Monotonic, never reset, never reused.
- **Threading:** every downstream event carries `task_id` as its first field. If a specialist emits a follow-up event (e.g., `TaskCompleted`), it reuses the same `task_id`.
- **Stitching:** the Observer matches `event_emit` / `event_delivered` SSE pairs by event id; `task_id` is what humans use to follow a flow across hops.

### Warden integration

- **MVP**: one flat warden per cluster (this example). Simple to reason about; escalation ladder is a single path.
- **Future** (when specialists start spawning children): keep `org_warden` as the top layer and add a sub-warden per specialist for its child agents. The top warden's `max_retries N per 1h then escalate` still caps the whole cluster's blast radius.

## Reference — agents inheriting this shape

| Agent | Path | Status |
|---|---|---|
| **clone-dev-skeleton** (T1.1) | `examples/agents/clone-dev-skeleton/main.forge` | **Exemplar** — full mastermind with T4.1 extensions (blocked_on, cycle detection, UnblockTask fanout, SeedTask cross-project entry). Reference implementation when you need more than the base. |
| **inbound-triager** | `examples/agents/inbound-triager/main.forge` | **Inspired this pattern** — single-hop classify-and-route precursor. Historically upstream; does not (and should not) adopt the dispatcher shape. |
| **workflows/dev-cycle** | `workflows/dev-cycle/main.forge` | **Not this shape** — sequential 5-stage pipeline correlated by `task_id`, no dispatcher routing. Listed so readers don't conflate. |
| **T6.1 meeting extractor** *(future)* | `examples/agents/meeting-ingest/` | Slated to adopt by reference — see [#305][i305]. |

[i305]: https://github.com/ncmlabs/forge/issues/305

## Run

```bash
export ANTHROPIC_API_KEY=...

cargo run -- serve \
  --manifest examples/agents/mastermind-pattern/forge.project.toml \
  examples/agents/mastermind-pattern/main.forge \
  --port 3311
```

## Acceptance

### 1. Route to `support_specialist`

```bash
curl -X POST http://localhost:3311/ticket \
  -d 'kind=support' \
  -d 'channel=C0123456789' \
  -d 'message=my widget broke'
```

Expected (stdout + Observer SSE `/__forge/events`):
- `TicketInbound` emit — 1 subscriber (`mastermind`)
- `mastermind` → `llm_request(reason)` → `llm_response` → `[mastermind] T1 kind=support -> support (reason-said support)`
- `support_specialist.HandlerStarted(TaskRouted)` — the `where target_agent == "support"` filter matched
- `[support_specialist] T1: handling support ticket in C0123456789 — my widget broke`

### 2. Route to `billing_specialist`

```bash
curl -X POST http://localhost:3311/ticket \
  -d 'kind=billing' \
  -d 'channel=C0123456789' \
  -d 'message=double-charged in March'
```

Expected: mastermind assigns `T2`, routes to `billing_specialist`; the support specialist sees nothing (filter excludes it).

### 3. Warden escalation

With the server running, fire four stuck injections on `support_specialist`:

```bash
for i in 1 2 3 4; do
  curl -X POST http://localhost:3311/__forge/inject/stuck \
       -H 'Content-Type: application/json' \
       -d '{"agent":"support_specialist"}'
  sleep 1
done
```

Expected: `ward_action` SSE events from `org_warden`; once `after 3: escalate` on `stuck` fires, the agent lands in `degraded_agents`. The runtime stays up — the mastermind and `billing_specialist` keep serving.

> The configured `max_retries 3 per 1h then escalate` counts across all supervised agents within the 1-hour window. Restart `forge serve` for a clean counter.

## Files

- `main.forge` — types, events, three agents, warden, system, endpoint. ~140 lines.
- `forge.project.toml` — manifest (no skills — the pattern demo is self-contained).
- `forge.config.toml` — local LLM config (Anthropic Haiku with `quality_tier=balanced` override).

## See also

- [`clone-dev-skeleton/`](../clone-dev-skeleton/) — realistic, skill-integrated variant; extends this base with T4.1 task-graph ops.
- [#292][i292] — the clone-developer track umbrella.
