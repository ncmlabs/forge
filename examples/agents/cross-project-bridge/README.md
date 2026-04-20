# Cross-project bridge — closes #300 (T4.2)

This example ships the return path of clone-dev's cross-project handoff: a
FORGE agent in repo A that wakes when a PR merges in repo B. The inbound
primitive is the `webhook TRIGGER` block added in #335 — a declarative
peer to `schedule` (#332) and `correlate` (#334).

## What's here

| File                                    | Purpose                                                        |
| --------------------------------------- | -------------------------------------------------------------- |
| `main.forge`                            | `mastermind` agent declaring `webhook pr_merged`               |
| `webhooks/github-pr-merged.yml`         | GitHub Actions workflow that POSTs the signed webhook          |
| `tests/e2e.sh`                          | End-to-end acceptance: boots the server, fires a signed POST   |

## Running locally

```bash
# 1. Rotate a fresh 32-byte HMAC secret for this (agent, trigger) pair.
#    Stdout gets the hex secret once; stderr prints the warning.
forge wake rotate --agent mastermind --trigger pr_merged

# 2. Boot the server.
forge serve examples/agents/cross-project-bridge/main.forge

# 3. In another terminal, simulate repo B's PR-merged webhook.
secret="<paste the hex from step 1>"
body='{"repo":"repo-b","pr_number":42,"merged_by":"octocat"}'
sig=$(printf '%s' "$body" | openssl dgst -sha256 -hmac "$secret" | sed 's/^.* //')
curl -X POST http://127.0.0.1:3000/wake/mastermind/pr_merged \
  -H "Content-Type: application/json" \
  -H "X-Hub-Signature-256: sha256=$sig" \
  -d "$body"
```

Watch the Observer timeline at `http://127.0.0.1:3000/__forge/events` —
`webhook_received` should appear, followed by the `PrMerged` handler's
`say` output and a `TaskCompleted` emission.

## Wiring into GitHub

1. Copy `webhooks/github-pr-merged.yml` to `.github/workflows/forge-wake.yml`
   in the *downstream* repo (the one whose PR merges trigger the wake).
2. Add an Actions secret named `FORGE_WAKE_SECRET` with the value printed
   by `forge wake rotate`.
3. Add an Actions variable named `FORGE_WAKE_URL` pointing at your FORGE
   server (e.g. `https://forge.example.com`).

## Security notes

- Secrets live in the `FORGE_WAKE_SECRETS` redb table. They are never
  logged and never returned by `forge wake list`.
- HMAC verification is constant-time (`subtle::ConstantTimeEq`) — timing
  attacks on the signature comparison do not leak the secret.
- The handler reads the raw request body *before* attempting JSON parse, so
  HMAC verification is over the exact bytes the sender signed.
- The HTTP handler rate-limits per `(agent, trigger)` (10 rps, burst 20) to
  absorb retry storms without letting a rogue source wake specialists in a
  tight loop.

## Full round-trip (#354 + #335)

With the outgoing half shipped (#354), the complete cross-project loop
runs end-to-end without polling:

1. A specialist in repo A emits `CrossProjectRequested(source_task_id,
   target_repo, description, labels)` — see
   `examples/agents/clone-dev-skeleton/main.forge` for the handler side.
2. Repo A's `mastermind` calls
   `skill.github.create_labeled_issue(target_repo, description, body,
   composite_labels)` — a deterministic `gh issue create --label …`
   executor declared in `skills/github/SKILL.md` — and records
   `TaskBlocked(source_task_id, [issue_url])` in its task graph.
3. Repo B's `dev_cycle` picks up the labeled issue via T1.1 inbound
   label routing and works it. When its PR merges…
4. Repo B's GitHub Action (`webhooks/github-pr-merged.yml`) POSTs the
   HMAC-signed wake to repo A's `/wake/mastermind/pr_merged` — the
   return path this example ships (#335).
5. `webhook pr_merged` validates the signature, rehydrates mastermind,
   and publishes `PrMerged`. The `on PrMerged` handler emits
   `TaskCompleted` for the upstream task.
6. The existing `on TaskCompleted` handler (#299 T4.1) scans the task
   graph, drops the cleared blocker, and fires `UnblockTask` for any
   dependent task that is now free.

The two examples — this bridge (return path) and the clone-dev skeleton
(outgoing path) — compose into one loop with no scheduler, no polling,
and no custom broker. `examples/agents/cross-project-bridge/tests/round_trip.sh`
exercises both halves against a stubbed `gh` and asserts the Observer
timeline contains every step of the sequence.
