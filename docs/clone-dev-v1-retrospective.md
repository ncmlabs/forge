# Clone-Dev v1 Retrospective (T11.4, closes epic #292)

**Date:** 2026-09-03 · **Author:** Claude (Hermes agent, on behalf of Marci) · **Scope:** the clone-developer track, epic #292, proof run #372 (T11.3), retro #373 (this document).

## Summary

The clone-developer track set out to prove Layer 3 of FORGE — *spec in → running
system out, no human writing FORGE* — with a supervised agent swarm (mastermind +
stateful specialists, fronted by Slack, driven by an outcome-driven mastery loop)
that takes a GitHub issue through plan → approve → implement → PR → review → merge.

**What was proven:** the full pipeline works end-to-end on a real external
repository. `ncmlabs/forge-playground#26` went from signed GitHub wake to a
merged PR (#36) in **3.48 minutes**, with **2 approval asks**, **CI green on the
first try**, **1 review round**, and the implementer/planner/reviewer/release
mastery FSMs advancing **novice → expert** on real outcomes. Total LLM cost of
the reference run: **$0.23** (25 LLM calls, ~49k tokens in / ~6.8k out).

**What was not proven:** the original T11.3 goal of processing **all 10 seeded
issues** in one autonomous sweep. The run was repeatedly halted by real defects
— which is itself the headline finding: the swarm found its own trust and
integration bugs faster than it could process the queue, and every one of them
was real, filed, and fixed.

**Verdict:** v1 is complete as a proof-of-architecture with a 1-issue deep
verification loop, not as a 10-issue batch demonstration. The epic closes with
that scope stated plainly.

## Architecture

- Design spec: `docs/proof-runs/2026-05-clone-dev-playground.md` and the
  milestone issues T1.1–T11.4 (see `roadmap.md`, Layer 3 table).
- Topology: `workflows/clone-dev/` — mastermind intake (conversational
  classifier, T9.1), investigators with skill allow-lists (T9.2), solution
  proposer (T9.3), issue creator (T9.4), label router (T8.3), and the dev-cycle
  five-agent core — planner → implementer → tester → reviewer → release manager
  (T2), with Slack approval gates one/two/three (T10.1–T10.3), cross-project
  webhook driver (T4.2, #335), per-repo knowledge scoping (T8.4, #359), config
  driven workdir/branch/commit templates (T8.5, #360), per-phase multi-provider
  LLM routing (T8.6, #361), outcome-driven learning + swarm mastery FSM
  (T5.1/T5.2, #302/#303).

## What shipped (the epic's 18+ sub-issues, all merged)

T1.1 walking skeleton (#293) · T2 dev-cycle topology (#310) · T5.4 PR history
miner (#294) · T2.2 reviewer knowledge consultation (#297) · T2.1 implementer
iteration loop (#296) · T3.1 Slack adapter (#298) · T4.1 task-graph +
cycle detection (#299) · T4.2 webhook driver (#335) + outgoing half (#354) ·
T5.1 outcome-driven learning (#302) · T5.2 mastery FSM (#303) · T8.1–T8.6
assembly/loader/router/scoping/templates/routing (#356, #357, #358, #359,
#360, #361) · T9.1–T9.5 intake/investigators/proposer/creator/monitor (#362,
#363, #364, #365, #366) · T10.1–T10.3 approval gates (#367, #368, #369) ·
T11.1–T11.2 target scaffolding + seeding (#370, #371) · T11.3 proof run (#372).

## Proof-run metrics (T11.3 evidence)

Reference run: `metrics/proof-runs/2026-05-19-run4/` (issue #26 → PR #36, merged
2026-05-19T19:20:50Z, merge commit `237e98a`).

| Metric | Value |
|---|---|
| Approval asks | 2 (plan, merge) |
| CI passed first try | yes |
| Review rounds | 1 |
| Time to merge | 3.48 min |
| Mastery (planner/implementer/reviewer/release_manager) | novice → expert, score 100.0, 0 regressions |
| LLM cost | $0.2284 total (25 calls; classify $0.0065, reason $0.0921, skill tools $0.1300) |
| Tokens | 42,251 in / 6,779 out |

Per-issue metrics for the remaining seeded queue were not captured: the run
never reached batch mode (see Defects). The T11.3 acceptance items around a
full-queue JSON and mastery-tile screenshot therefore close as *not met —
superseded by the defect-first reality*, with the single-issue metrics above as
the verified reference.

## What worked

- **Signed wake → full dev-cycle, hands off the keyboard:** GitHub webhook →
  HMAC-verified wake → mastermind routing → plan → Slack Gate-2 approval →
  implementation → PR → CI. The human touched exactly two approval cards.
- **Slack gates as the trust surface:** approval cards with interactive blocks
  worked live (T10.1–T10.3), including the merge approval that let the swarm
  merge its own PR after human sign-off.
- **Mastery FSM on real outcomes:** specialist levels advanced from actual
  CI/review results, not self-report — the mechanism the whole track banks on.
- **Cost profile:** a full supervised dev-cycle for $0.23 makes the "factory"
  economics plausible.

## What didn't work (and became the defect harvest)

Each halt produced filed, fixed defects — the track's real output:

| Defect class | Issues | Lesson |
|---|---|---|
| Slack `send_approval` JSON injection broke cards | #401 | Approval surfaces need schema-validated templating, not string substitution |
| Playground default branch / stale scaffolds contaminated runs | #404, #409 | Proof targets must be reset deliberately; preflight must check default branch |
| GitHub issue wake couldn't drive the TS proof queue (missing `npm ci`, signed-wake storage) | #412, #415 | Fresh-clone preflights belong in the implementer prompt contract |
| System arrow chains forwarded status events into the implementer | #413, #414-class | Explicit subscriptions beat implicit arrows in swarm topologies |
| Implementer executed fenced Markdown instead of shell | #414, #424 | Shell execution must be argv-based with fence stripping |
| Reviewer passed verbose `create_pr` output into `merge_pr`; PR closeout gaps | #421 | Inter-agent interfaces need typed selectors, not log-echoed IDs |
| Reviewer Gate-3 resume/fast-path and channel routing | #426/#427, #430/#433 | Gate 3 must resolve its channel exactly like Gate 2 (config → event → default) |
| Split Slack approval blocks | #428/#429 | Card size limits are real; block splitting is mandatory |
| **Tester let a failed `test_cmd` become `AcceptanceMet` via LLM acceptance classification** | #431 → fixed by #451 | **The single most important finding:** deterministic command failure must hard-block; an LLM must never be able to overrule a red test |
| Gate-3 `channel_not_found` on the GitHub-wake path (empty event channel) | #430 → fixed by #433 | Trust path bugs cluster at approval boundaries |
| Provider `model` field didn't env-expand (found in the 05-25 BERCASTLE rerun) | → fixed by #458 | Config secrets/models must be deployable without editing committed files |

The 2026-05-25 BERCASTLE rerun also validated FORGE against a non-Anthropic,
OpenAI-compatible provider through classify and plan phases — the same provider
pattern later adopted for the Daily Frame pipeline and the surface-audit rework
(#449).

## What we'd change for v2

1. **Batch-first verification.** The 10-issue sweep should be the *default*
   proof shape, with per-issue defect halts triaged into follow-ups rather than
   restarting the run. v1 burned its calendar on five restarts of a single issue.
2. **Deterministic gates before LLM judgment** (now shipped via #451) —
   v1's worst near-miss would have shipped unverified code.
3. **Run identity and storage isolation from day one** (`FORGE_STORAGE_ROOT` +
   `FORGE_PROOF_RUN_ID` came mid-track; they should have been T1 flags).
4. **Provider-agnostic runs as a first-class test matrix** (Anthropic,
   OpenAI-compat, local vLLM) — the BERCASTLE rerun surfaced the env-expansion
   gap only by accident.
5. **Metrics capture as a harness, not a manual step** — the T11.3 JSON/chart
   deliverables depend on it, and manual capture is why they slipped.

## Open follow-ups

- Closed as fixed during/after the run: #401, #403, #404, #409, #412, #413,
  #414, #415, #421, #424, #426, #428, #430.
- The v2 items above are deliberately **not** filed as new issues yet — epic
  close (#292) should be followed by the Layer-3 dogfooding decision
  (clone-dev on forge's own `development`), at which point v2 scope gets filed
  against real needs.

## Epic close

- `roadmap.md` Layer 3 table already records T11.3 ✅; with this retrospective,
  T11.4 ✅ and epic **#292 closes as: v1 proven (1-issue deep loop, $0.23,
  novice→expert mastery, zero silent failures tolerated), batch sweep deferred
  to v2.**
- Layer 3 status moves from *Kickoff* to *v1 complete — factory proven on one
  repo; dogfooding on forge itself is the next initiative.*
