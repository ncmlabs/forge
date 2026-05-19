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
# Point at the clone-dev TOML the mastermind loads at startup (T8.2 #357).
# Copy and edit clone-dev.toml.example at the repo root.
export FORGE_CLONEDEV_CONFIG=$PWD/clone-dev.toml

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

## Config — `clone-dev.toml` (T8.2 #357)

The mastermind loads `$FORGE_CLONEDEV_CONFIG` on startup and logs
`Loaded config for org=<name>`. The loader lives in Rust
(`src/runtime/clone_dev_config.rs`) and is exposed to FORGE as the
intrinsic `config.load_clone_dev(path) -> CloneDevConfig`.

### Sections

| Section | Purpose |
|---|---|
| `[org]`           | Friendly org name (logged at boot) |
| `[slack]`         | `bot_token_env`, `signing_secret_env`, `default_channel` |
| `[github]`        | `token_env` for `skill.github.*` |
| `[labels]`        | Org-wide `triage` + `blocked` label lists |
| `[llm.routing]`   | Named provider profiles (`fast`, `balanced`, `high`) |
| `[warden]`        | `max_retries`, `escalate_after_seconds` |
| `[budget]`        | `per_task_usd`, `per_hour_usd` |
| `[gates]`         | Approval-required task kinds + auto-approve labels |
| `[defaults]`      | Per-repo-style knobs applied org-wide |
| `[repos."<slug>"]` | Per-repo overrides keyed on `<owner>/<name>` |

### Merge rules

- **Scalar fields** (Text, Number): per-repo wins over `[defaults]`.
- **Array fields** (`labels_extra`, …): concatenated — `[defaults]` first, then `[repos."…"]`.
- **Absent per-repo numeric field**: inherits `[defaults]` value.
- **Absent `[defaults]` numeric field**: surfaces as `-1.0` to FORGE
  (the `repo_config_for` pure helper treats `-1.0` as "inherit org-wide default").

### Secrets — env-var indirection

Any TOML key suffixed `_env` is treated as an environment-variable
NAME, not the secret itself. The loader resolves it via
`std::env::var` at parse time:

```toml
[slack]
bot_token_env      = "SLACK_BOT_TOKEN"       # NOT the literal token
signing_secret_env = "SLACK_SIGNING_SECRET"
```

The resolved value appears as `config.slack_bot_token` on the FORGE
side. Missing env vars resolve to the empty string — there is no
error on unset, so agents can check `config.slack_bot_token == ""`
and degrade gracefully.

### Minimal example

See `clone-dev.toml.example` at the repo root for a fully commented
example covering all ten sections and both merge patterns.

### Scope note

T8.2 wires config into the mastermind only (the agent that actually
reads it). Specialist-level threading — implementer/tester/reviewer
timeouts, per-repo model overrides, and slack-adapter's escalation
channel — lands with T8.5 (budgets) and T8.6 (approval gates),
which are the tickets that introduce per-specialist behavior the
config drives. The shared type + loader + merge logic are ready for
those tracks to consume.

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

## Playground Proof-Run Preflight

The T11.3 proof run targets `ncmlabs/forge-playground` as a TypeScript
proof-run tracker on its `main` branch. Issue `ncmlabs/forge#409` resets the
older Rust/Go proof surface and reseeds the queue around the app.

Before starting a run against `ncmlabs/forge-playground`, verify the repository
default branch and TypeScript surface:

```bash
gh repo view ncmlabs/forge-playground --json defaultBranchRef --jq '.defaultBranchRef.name'
tmp="$(mktemp -d)"
gh repo clone ncmlabs/forge-playground "$tmp/forge-playground"
cd "$tmp/forge-playground"
test "$(git branch --show-current)" = "main"
test -f package.json
test -f package-lock.json
test -f src/server/app.ts
test -f src/client/App.tsx
npm ci
npm run typecheck
npm test
npm run build
```

See [2026-05 Clone-Dev Playground Proof Run](../../docs/proof-runs/2026-05-clone-dev-playground.md)
for the full preflight.

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
