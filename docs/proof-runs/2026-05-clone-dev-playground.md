# T11.3 Proof Run — clone-dev vs. ncmlabs/forge-playground

Companion runbook for issue [#372](https://github.com/ncmlabs/forge/issues/372).
The proof point for the clone-developer track (#292): boot the clone-dev
swarm against `clone-dev.toml` pointing at `ncmlabs/forge-playground`, drive
all 10 seeded issues end-to-end (plan → approve → impl → PR → review →
merge), and capture per-issue metrics demonstrating that the agents are
learning — `approval_asks` for issue #10 strictly less than issue #1.

> **Read this end-to-end before booting anything.** Stop conditions are in
> §7 and need to be internalised before the first ask lands on Slack.

## 1. What this proves

| Field | Meaning |
| --- | --- |
| `approval_asks` | Stage-2 gate asks (gate_two + gate_three) per task. Gate-1 (Stage-1 issue triage) is excluded — it predates `task_id` allocation. |
| `ci_passed_first_try` | Did the tester pass on the implementer's first commit? |
| `review_rounds` | How many review cycles the reviewer needed. |
| `time_to_merge_mins` | Wall-clock from `IssueAssigned` to `TaskCompleted`. |
| `mastery_level_before` / `mastery_level_after` | Per-(specialist, project) FSM level at task start vs. settle. |
| `reverted_within_7d` | Structurally `false` for a same-day proof. Document in retrospective. |

Proof criterion (DoD bullet): `approval_asks[issue #10] < approval_asks[issue #1]`.

## 2. Pre-flight

### 2.1 Required secrets (env vars at launch shell)

| Var | Purpose |
| --- | --- |
| `ANTHROPIC_API_KEY` | LLM provider (`reason`/`classify`/`plan`/`review` routes). |
| `SLACK_BOT_TOKEN` | `xoxb-…`; `chat:write` + `channels:history` + `app_mentions:read`. |
| `SLACK_SIGNING_SECRET` | HMAC verification for `/webhook/approval` payloads. |
| `GH_TOKEN` | Personal access token with `repo` scope on `ncmlabs/forge-playground`. |
| `FORGE_CLONEDEV_CONFIG` | Absolute path to your local `clone-dev.toml`. |
| `FORGE_PROOF_RUN_ID` | Run identifier, e.g. `2026-05-13-run1`. Activates `proof_run_sink`. |

### 2.2 `clone-dev.toml`

Copy from `clone-dev.toml.example` at repo root, then:

- `[github] default_repo = "ncmlabs/forge-playground"`
- Confirm `[repos."ncmlabs/forge-playground"]` is present with `test_cmd = "cargo test"` (or `"echo ok"` for smoke).
- `[gates] start_implementation = true` and `merge_pr = true` so all three gates are live.

### 2.3 Verify the 10 seeded issues exist

```bash
gh issue list --repo ncmlabs/forge-playground --label clone-dev:impl --json number,title,labels --limit 20
```

Expect exactly 10 issues labelled with one of the `clone-dev:*` routing labels (T11.2 / #371).

## 3. Tunnel

```bash
scripts/ngrok-bg.sh start 3300
NGROK_URL=$(scripts/ngrok-bg.sh url)
echo "$NGROK_URL"        # printed by ngrok-bg.sh; format: https://<tunnel-id>.<domain>
```

Keep the inspector handy at <http://127.0.0.1:4040>.

## 4. Wire webhooks

### 4.1 Slack app

In your Slack app (api.slack.com/apps):

- **Interactivity & Shortcuts** → Request URL = `${NGROK_URL}/webhook/approval`
- **Event Subscriptions** → Request URL = `${NGROK_URL}/wake/mastermind/slack_message`; subscribe to `message.channels` + `app_mention`.

### 4.2 GitHub webhook on forge-playground

`Settings → Webhooks → Add webhook` on `ncmlabs/forge-playground`:

- Payload URL = `${NGROK_URL}/wake/mastermind/github_issue_opened`
- Content type = `application/json`, secret = (use `forge wake rotate` output, see below)
- Events = **Issues** + **Pull requests**

Add a second webhook (or check both events on the first) routing to:

- `${NGROK_URL}/wake/mastermind/github_pr_merged` — events: **Pull requests** only.

### 4.3 Register HMAC secrets

```bash
forge wake rotate --agent mastermind --trigger github_issue_opened
forge wake rotate --agent mastermind --trigger github_pr_merged
forge wake rotate --agent mastermind --trigger slack_message
```

Paste each generated secret into the corresponding GitHub / Slack webhook config.

## 5. Boot

**Terminal 1 — clone-dev**:

```bash
export FORGE_CLONEDEV_CONFIG="$PWD/clone-dev.toml"
export FORGE_PROOF_RUN_ID="2026-05-13-run1"
mkdir -p "metrics/proof-runs/$FORGE_PROOF_RUN_ID"

cargo run -- serve \
  --manifest workflows/clone-dev/forge.project.toml \
  -s workflows/clone-dev/shared/events.forge \
  -s workflows/clone-dev/shared/types.forge \
  -s workflows/clone-dev/shared/mastermind.forge \
  -s workflows/clone-dev/stage1/mastermind_intake.forge \
  -s workflows/clone-dev/stage1/slack_devops_monitor.forge \
  -s workflows/clone-dev/stage1/investigators/code_investigator.forge \
  -s workflows/clone-dev/stage1/investigators/ops_investigator.forge \
  -s workflows/clone-dev/stage1/investigators/security_investigator.forge \
  -s workflows/clone-dev/stage1/solution_proposer.forge \
  -s workflows/clone-dev/stage1/gate_one.forge \
  -s workflows/clone-dev/stage1/issue_creator.forge \
  -s workflows/clone-dev/stage2/label_router.forge \
  -s workflows/clone-dev/stage2/triage_specialist.forge \
  workflows/clone-dev/main.forge \
  --port 3300 2>&1 \
  | tee "metrics/proof-runs/$FORGE_PROOF_RUN_ID/clone-dev.log"
```

You should see `[proof_run_sink] active — run_id=… dir=metrics/proof-runs/…`.

**Terminal 2 — Observer**:

```bash
cargo run -- serve examples/observer/server.forge \
  -s examples/observer/shared.forge
```

Open <http://localhost:3002/static/index.html?server=http://localhost:3300>.

### 5.1 Smoke

```bash
curl -sf localhost:3300/health
curl -sf localhost:3300/__forge/inspect/mastery | jq '.specialists,.projects'
```

Observer should render the mastery tile (empty initially).

## 6. Trigger

For each of the 10 seeded issues, either:

- **(a)** Add/re-add the `clone-dev:impl` label via the GitHub UI to fire the webhook.
- **(b)** Open the issue fresh if it was closed during prior runs.

The mastermind's `label_router` will route the issue to the right specialist; the dev-cycle planner will pick it up and emit `PlanReady`.

### 6.1 Per-gate human protocol

To make `approval_asks` mean something, do **not** rubber-stamp:

| Gate | Approve when… | Reject when… |
| --- | --- | --- |
| **Gate-2** (start-impl) | Plan reads as a real plan: numbered steps, addresses the acceptance criteria, no obviously missing pieces. | Plan is vague, doesn't reference the acceptance criteria, or proposes the wrong shape. |
| **Gate-3** (merge-PR) | Diff matches plan + tests pass. | Diff is wrong shape, tests missing, or characterisation says "novel pattern, prior matches none." |

Write your rubric down in the retrospective.

## 7. Stop conditions

- **Per-issue stall** (45 min from `IssueAssigned` to `TaskCompleted`): leave the issue in its current state, open a follow-up issue describing the stall, continue to the next.
- **Three consecutive stalls**: halt the run; capture partial metrics into the retrospective; open a follow-up to debug.
- **Warden escalation with `urgency = high`** that's unrelated to the current issue: halt and triage. The whole point of supervised swarms is supervision — honour real escalations.
- **Proof criterion failure** is **not** a stop condition. If `approval_asks[#10] >= approval_asks[#1]`, the retrospective reports the honest result and proposes follow-ups. T11.3 is the run, not a forced outcome.

## 8. Capture

After issue 10 merges (or run halts):

```bash
ls "metrics/proof-runs/$FORGE_PROOF_RUN_ID/"
# Expect: issue-<N>.json × N, summary.json, clone-dev.log
```

Screenshot the Observer mastery tile (D3 chart + table) and save as
`metrics/proof-runs/$FORGE_PROOF_RUN_ID/mastery-tile.png`. Reproducibly via
the `playwright-cli` skill:

```bash
# Take a screenshot of the mastery tile section
# (replace the URL with your local observer)
playwright screenshot --url 'http://localhost:3002/static/index.html?server=http://localhost:3300' \
  --output "metrics/proof-runs/$FORGE_PROOF_RUN_ID/mastery-tile.png" \
  --full-page
```

Sanity-check the proof criterion:

```bash
RUN_DIR="metrics/proof-runs/$FORGE_PROOF_RUN_ID"
jq '.approval_asks' "$RUN_DIR/issue-1.json"   # adjust to your task_id naming
jq '.approval_asks' "$RUN_DIR/issue-10.json"
```

## 9. Retrospective (skeleton)

Write `metrics/proof-runs/$FORGE_PROOF_RUN_ID/retrospective.md` covering:

```markdown
# T11.3 Proof Run — Retrospective

**Run ID:** <FORGE_PROOF_RUN_ID>
**Date range:** <start> to <end>
**Operator:** <name>

## Headline numbers

| Metric | Issue #1 | Issue #10 | Mean | Trend |
| --- | --- | --- | --- | --- |
| `approval_asks` | … | … | … | … |
| `ci_passed_first_try` | … | … | … | … |
| `review_rounds` | … | … | … | … |
| `time_to_merge_mins` | … | … | … | … |

## Proof verdict

- [ ] Criterion met: `approval_asks[#10] < approval_asks[#1]`
- Analysis: …

## Mastery progression

- planner: <before> → <after>
- implementer: <before> → <after>
- tester: <before> → <after>
- reviewer: <before> → <after>
- release_manager: <before> → <after>

## What worked

…

## What didn't

…

## Follow-up issues filed during the run

- #… — <summary>
- #… — <summary>

## Approval rubric used

(How rigorous were the gate-2 / gate-3 decisions? What would you tighten next time?)
```

## 10. After the run

1. Update `CHANGELOG.md` (`Added` under `[Unreleased]`): "Clone-dev proof-run instrumentation + first end-to-end run (#372)".
2. Mark T11.3 shipped in `docs/roadmap.md` (or the phase-2 roadmap).
3. Commit `docs/proof-runs/2026-05-clone-dev-playground.md` + `metrics/proof-runs/<run-id>/` + the changelog/roadmap updates on the `feat/clone-dev-t11-3-proof-run` branch.
4. Open the PR targeting `development`; paste the headline numbers, proof verdict, and follow-up issue links into the body.
