# clone-dev — top-level FORGE assembly (#356)

The existential clone-developer program. Boots the full supervised swarm on
**one HTTP port**, composing five imported pieces under a single `org_warden`:

- `workflows/dev-cycle/agents.forge` — 5 specialists (planner, implementer,
  tester, reviewer, release_manager) + 2 mastery agents (swarm_mastery_coordinator,
  swarm_mastery_tuple)
- `examples/agents/slack-adapter/agents.forge` — slack_adapter (the ONLY
  outbound Slack surface)
- `shared/mastermind.forge` — clone-dev mastermind (classify + route into
  dev-cycle; follows the [mastermind-pattern](../../examples/agents/mastermind-pattern/README.md)
  invariants; task-graph + 1-hop cycle detection from T4.1 #299)
- `shared/types.forge` — `TaskNode` + 12 pure cycle-detection helpers
- `shared/events.forge` — `CloneDevInbound`, `GithubIssueOpened`,
  `GithubPrMerged`, `SlackMessageReceived`, `TaskBlocked`, `UnblockTask`,
  `CycleDetected`, `SeedTask`, `CrossProjectRequested`

Track 8 of the clone-dev epic (#292). Blocks T8.2, T8.3, T8.5, T8.6, and all
of Track 9.

## Boot

```bash
# Set required env
export ANTHROPIC_API_KEY=sk-ant-...
export SLACK_BOT_TOKEN=xoxb-...
# Optional: path that T8.2 will load (T8.1 only reads the var)
export FORGE_CLONEDEV_CONFIG=$PWD/workflows/clone-dev/clone-dev.config.toml

cargo run -- serve \
  --manifest workflows/clone-dev/forge.project.toml \
  -s workflows/clone-dev/shared/types.forge \
  -s workflows/clone-dev/shared/events.forge \
  -s workflows/clone-dev/shared/mastermind.forge \
  -s workflows/dev-cycle/agents.forge \
  -s examples/agents/slack-adapter/agents.forge \
  workflows/clone-dev/main.forge \
  --port 3300
```

## Webhook secret registration

Before external traffic can wake the mastermind via `/wake/mastermind/*`,
register an HMAC secret per trigger (#335):

```bash
forge wake rotate --agent mastermind --trigger github_issue_opened
forge wake rotate --agent mastermind --trigger github_pr_merged
forge wake rotate --agent mastermind --trigger slack_message
```

Each prints the hex secret once on stdout. Paste it into the sender (GitHub
Actions `FORGE_WAKE_SECRET`, Slack signing secret shim, etc.).

## HTTP surface

| Method | Path | Purpose |
|---|---|---|
| GET  | `/health`                              | Liveness probe |
| POST | `/clone_dev`                           | Direct API inbound → `CloneDevInbound` |
| POST | `/task_blocked`                        | Smoke: record a blocker (1-hop cycle check) |
| POST | `/task_completed`                      | Smoke: mark a task done (fans out `UnblockTask`) |
| POST | `/seed_task`                           | Smoke: seed a node without LLM classification |
| POST | `/cross_project`                       | T4.2 outgoing handoff |
| POST | `/wake/mastermind/github_issue_opened` | HMAC webhook → `GithubIssueOpened` → mastermind |
| POST | `/wake/mastermind/github_pr_merged`    | HMAC webhook → `GithubPrMerged` → mastermind |
| POST | `/wake/mastermind/slack_message`       | HMAC webhook → `SlackMessageReceived` → mastermind |
| POST | `/webhook/approval`                    | Slack approval callback (runtime-built-in) |
| GET  | `/__forge/events`                      | Live SSE trace |
| GET  | `/__forge/inspect/{agents,topology,wardens,costs,mastery,schedules}` | Introspection |

## Acceptance (DoD #356)

With the server running on port 3300:

```bash
# 1. compile + boot cleanly
curl -s localhost:3300/health
# → ok

# 2. topology shows the full assembly
curl -s localhost:3300/__forge/inspect/topology | jq '.agents | length'
# → 9  (mastermind + 5 specialists + 2 mastery + slack_adapter)

# 3. one warden covers all agents
curl -s localhost:3300/__forge/inspect/wardens | jq 'length'
# → 1   (org_warden)

# 4. wake endpoints visible under declared triggers
curl -s localhost:3300/__forge/inspect/schedules | jq '.webhooks'

# 5. direct inbound → IssueAssigned → planner pickup
curl -X POST localhost:3300/clone_dev \
  -d 'kind=github_issue' \
  -d 'repo=ncmlabs/forge-playground' \
  -d 'issue_id=1' \
  -d 'title=demo' \
  -d 'body=demo body' \
  -d 'channel=C0123456789' \
  -d 'callback_url=http://localhost:3300/webhook/approval' \
  -d 'test_cmd=echo ok'

# 6. task-graph smoke endpoints
curl -X POST localhost:3300/seed_task -d 'task_id=T9&specialist=planner&project=playground'
curl -X POST localhost:3300/task_blocked -d 'task_id=T9&blocker_id=T0'
curl -X POST localhost:3300/task_completed -d 'task_id=T0&repo=ncmlabs/forge-playground&outcome=merged'

# 7. approval round-trip
curl -X POST localhost:3300/webhook/approval \
  -H 'Content-Type: application/json' \
  -d '{"request_id":"1","approved":true,"comment":"ok"}'

# 8. SSE trace
curl -N localhost:3300/__forge/events | head -5
```

## Regression gates

The refactor required `workflows/dev-cycle/` and `examples/agents/slack-adapter/`
to split into `agents.forge` (library) + `main.forge` (standalone wrapper).
Both standalone programs still boot:

```bash
# dev-cycle standalone (now sources slack-adapter's agents.forge)
cargo run -- serve \
  --manifest workflows/dev-cycle/forge.project.toml \
  -s workflows/dev-cycle/agents.forge \
  -s examples/agents/slack-adapter/agents.forge \
  workflows/dev-cycle/main.forge --port 3212

# slack-adapter standalone
cargo run -- serve \
  --manifest examples/agents/slack-adapter/forge.project.toml \
  -s examples/agents/slack-adapter/agents.forge \
  examples/agents/slack-adapter/main.forge --port 3213
```
