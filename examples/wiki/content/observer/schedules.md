# Observer: Schedules & Wakes

The FORGE runtime emits a rich family of tracer events for durable schedules,
external webhooks, event correlations, and session rehydration. The Observer
SPA renders this **wake surface** as glyphs on the trace timeline, live
countdowns on a dedicated **Schedules** tab, and cost attribution rows on the
**Costs** tab.

This page documents the three operator surfaces that expose the wake-family
tracer events: the introspection endpoint, the CLI, and the Observer SPA.

## Introspection endpoint

```
GET /__forge/inspect/schedules
```

Returns the current wake surface for the running program.

| Field | Description |
| --- | --- |
| `schedules[]` | Every `(agent, schedule)` pair, merging declared shape with live `ScheduleState` |
| `schedules[].declaration` | `when`, `mode`, `precision`, `emit` from the `.forge` source |
| `schedules[].next_run_at_ms` | Unix ms when the schedule next fires (`null` if never registered) |
| `schedules[].last_run_at_ms` | Unix ms of the most recent fire (`null` if never fired) |
| `schedules[].last_status` | `pending` / `success` / `error` / `skippedconcurrent` / `skippedbudget` / `halted` / `not_registered` |
| `schedules[].consecutive_errors` | Error streak — after `max_consecutive_errors`, schedules halt |
| `schedules[].claimed_by` | `instance_id` holding a live claim (or `null`) |
| `webhooks[]` | Every declared `endpoint` with a `signed` flag (does this endpoint require HMAC?) |
| `correlations_declared[]` | `correlate on Event.field` blocks from the AST |
| `correlations_live[]` | `(agent, field, value_count)` aggregated from the persisted correlation table |

**Secrets are never exposed.** The `signed` flag tells operators whether an
endpoint enforces HMAC, but the secret itself stays in `AppState.webhook_secrets`.

## CLI: `forge agent-inspect`

```
forge agent-inspect path/to/agent.forge
forge agent-inspect path/to/agent.forge --agent forge_sensei
```

Prints the **static declaration** view of an agent's wake surface — no runtime,
no I/O, just the AST:

```
FORGE Agent: forge_sensei
  memory (persistent): interaction_count, current_level, ...
  handlers: start, ingest, query, review, ...
  schedules:
    mastery_review  when: daily 09:00  mode: spawn
  correlations: (none)

Webhooks / endpoints: (none in this file)
```

Use `agent-inspect` when reviewing a `.forge` file without starting a server.
For the **live** view — including `next_run_at`, consecutive errors, and claim
holder — use the introspection endpoint above.

## Observer SPA

### Timeline glyphs

The trace timeline renders each wake-family tracer event with its own glyph
overlaid on the category lane:

| Glyph | Event | Meaning |
| --- | --- | --- |
| ⏰ | `schedule_fired` | Claim taken, handler dispatched (tooltip shows `scheduled_time → wall_time` delta) |
| ⏸ | `schedule_skipped_concurrent` | Another instance holds the claim |
| 💰 | `schedule_skipped_budget` | Budget gate blocked the fire — savings counted on the Costs tab |
| ❌ | `schedule_errored` | Handler failed; tooltip shows error + retry count |
| ⚠ | `schedule_claim_lost` | Our claim was overwritten by a competing writer |
| 💤 | `schedule_rehydrated` / `session_rehydrate_failed` | Wake-mode session restore |
| 🪝 | `webhook_received` | External HTTP POST (✓ signed / ✗ bad sig / unsigned) |
| 🎯 | `correlation_hit` | Inbound event routed to a pre-existing session |

Event filters in the timeline header toggle the **Schedule**, **Webhook**, and
**Correlate** lanes independently.

### Schedules tab

A dedicated tab lists every declared schedule with a **live countdown** to the
next fire. The countdown ticks every second and auto-refreshes whenever any
`schedule_*` / `webhook_received` / `correlation_*` tracer event arrives.

Columns:

- **Schedule** — `agent.schedule` identifier
- **When** — declared schedule spec (e.g. `daily 09:00`, `every 6 hours`, `cron "…"`)
- **Mode** — `spawn` or `wake`
- **Next** — live countdown from `next_run_at_ms - Date.now()`
- **Last run** — wall-clock time of the previous fire
- **Status** — colored badge reflecting `last_status`
- **Errors** — `consecutive_errors` count

The right-hand panel lists **webhooks** (with HMAC enforcement flag),
**declared correlations**, and the **live correlation key count** per agent/field.

### Costs tab: schedule attribution

Two tables were added to the Costs tab:

- **By Schedule** — cost attribution per `(agent, schedule)`, populated when
  LLM trace events carry a `schedule_name` field. Empty when no schedule
  context is attached to the current LLM activity.
- **Saved by budget gate** — count of `schedule_skipped_budget` events per
  `(agent, schedule)`. Accurate tally, driven entirely by tracer events.

## Key properties

- **No silent routing.** Every wake-family decision — fires, skips, claims,
  rehydrations — emits a discrete tracer event (Principle I: honesty).
- **Replay-safe.** The tracer events carry `scheduled_at_ms` and `wall_time_ms`
  so the replay harness can reconstruct cadence deterministically (Principle II).
- **Boundary separation.** An agent's declared wake shape is part of its
  protocol, surfaced by the CLI and the introspection endpoint alongside
  `memory` / `handlers` (Principle IX).
- **Secrets stay inside.** Webhook secrets never leave `AppState`; the
  endpoint exposes only a `signed: bool` flag.

## Related

- `WakeService` design — see issue #332.
- Session rehydration — see issue #333.
- Correlation driver — see issue #334.
- Wake surface Observer — see issue #336.
