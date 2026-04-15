# clone-dev-skeleton — Walking skeleton for the clone-developer track

**Issue:** [ncmlabs/forge#293][i293] · **Track:** [#292][i292] (Layer 3 kickoff — Clone Developer)

[i293]: https://github.com/ncmlabs/forge/issues/293
[i292]: https://github.com/ncmlabs/forge/issues/292

Proves the **3-hop topology** `HTTP → mastermind → specialist → Slack` end-to-end
using only primitives that already exist in the FORGE runtime. No new grammar,
no new capabilities — just composition.

```
POST /clone_dev {kind, ...}
      │
      │  emit ClonedevTaskInbound
      ▼
  mastermind         ← classify via `reason`, assign task_id,
      │                 append to task_graph
      │  emit TaskRouted(task_id, target_agent, kind, ...)
      ▼
 ┌─────────────────┬─────────────────┐
 │  pr_reviewer    │  echo_specialist │   subscribe TaskRouted
 │   where=="pr_…" │   where=="echo"  │     where target_agent ==
 │   →github+reason│   →slack.send    │       "<self>"
 │   →slack.approve│                  │
 └─────────────────┴─────────────────┘
```

`org_warden` supervises all three agents (per [#293 DoD][i293]).

## Fork relationship

Forked from `examples/agents/pr-review-bot/server.forge`. The `pr_reviewer`
logic (github.get_pr / check_ci / reason summary / slack.send_approval / merge)
is preserved — only the subscription is rewired (`TaskRouted where
target_agent == "pr_reviewer"` instead of `ReviewPRRequested`), and an incoming
`task_id` is stored so Observer traces stitch cleanly across hops.

## Run

```bash
export SLACK_BOT_TOKEN=xoxb-…
export ANTHROPIC_API_KEY=…

cargo run -- serve \
  --manifest examples/agents/clone-dev-skeleton/forge.project.toml \
  examples/agents/clone-dev-skeleton/main.forge \
  --port 3211
```

## Acceptance

### 1. Route to pr_reviewer

```bash
curl -X POST http://localhost:3211/clone_dev \
  -d 'kind=pr_review' \
  -d 'repo=ncmlabs/forge-playground' \
  -d 'pr_number=1' \
  -d 'channel=C0123456789' \
  -d 'callback_url=http://localhost:3211/webhook/approval'
```

Expected trace (visible in stdout + Observer SSE `/__forge/events`):

- `ClonedevTaskInbound` emit — 1 subscriber (mastermind)
- `mastermind.HandlerStarted(ClonedevTaskInbound)` → `llm_request(reason)` → `llm_response` → `[mastermind] kind=pr_review to pr_reviewer for task T1`
- `pr_reviewer.HandlerStarted(TaskRouted)` — the `where target_agent == "pr_reviewer"` filter matched
- `skill_call github.get_pr` → `reason` summary → `skill_call slack.send_approval` (Approve/Reject buttons in Slack)

### 2. Route to echo_specialist

```bash
curl -X POST http://localhost:3211/clone_dev \
  -d 'kind=echo' \
  -d 'message=hello world' \
  -d 'channel=C0123456789'
```

Expected: mastermind routes to `echo_specialist`, which posts
`[clone-dev/T2] hello world` to the channel.

### 3. Warden escalation

With the server running, fire four stuck injections on `echo_specialist`:

```bash
for i in 1 2 3 4; do
  curl -X POST http://localhost:3211/__forge/inject/stuck \
       -H 'Content-Type: application/json' \
       -d '{"agent":"echo_specialist"}'
  sleep 1
done
```

Expected: `ward_action` SSE events from `org_warden`; once the `after 3: escalate`
threshold for `stuck` is exceeded, the warden escalates and the agent lands in
`degraded_agents`. The runtime stays up (graceful degradation — the other agents
keep serving).

> The configured `max_retries 3 per 1h then escalate` counts failures across all
> supervised agents within the 1-hour window, so in a long session prior crashes
> may bring the threshold forward. That's by design — it's how the warden's
> rate-limit works. Restart `forge serve` for a clean counter.

## Design notes for follow-up issues

The skeleton intentionally leaves the following for downstream tracks:

- **task_graph close-out.** `task_graph: Text[]` appends records as
  `"{task_id}|{target}|{kind}"` but does not remove on completion. T4.1 will
  extend this into a richer graph with `blocked_on` relations.
- **Classification via `classify`.** Per issue spec we use `reason`. Once the
  pattern settles, `classify ... into [...]` is a cleaner idiom (see
  `examples/agents/inbound-triager/main.forge:85`). Follow-up.
- **Mastermind pattern extraction.** Lifting the mastermind into a named,
  reusable pattern is T1.2 — tracked separately.
- **Slack adapter.** Specialists here call `skill.slack.*` directly (mirroring
  `pr-review-bot`). T3.1 centralizes that into a `slack_adapter` agent.

## Files

- `main.forge` — events, states, three agents, warden, system, endpoint.
- `forge.project.toml` — manifest (skills: slack, github).
- `forge.config.toml` — local LLM config (Anthropic haiku).
