# dev-cycle-mastery-smoke — T5.2 (#303) live acceptance test

Proves the `SwarmMastery` FSM wiring end-to-end without needing
GitHub/Slack/git credentials: one endpoint hit fires a `TaskCompleted`
event, the coordinator attributes signals per specialist, spawns one
tuple agent per (specialist, project), and each tuple's lifecycle
transitions up or down through `novice → apprentice → journeyman →
expert` based on rolling clean/regress counts.

Mirrors the production agents in `workflows/dev-cycle/main.forge`:
same states, events, pure scoring functions, and attribution rules.
Strips the dev-cycle event producers so `TaskCompleted` can be fired
directly.

## Run

```bash
ANTHROPIC_API_KEY=... cargo run -- serve \
  --manifest examples/agents/dev-cycle-mastery-smoke/forge.project.toml \
  examples/agents/dev-cycle-mastery-smoke/main.forge --port 3214
```

Fire 3 clean merges:

```bash
for i in 1 2 3; do
  curl -s -X POST 'http://localhost:3214/complete' \
    -H 'Content-Type: application/x-www-form-urlencoded' \
    -d "task_id=T-$i&repo=ncmlabs/forge-playground&outcome=merged&ci_passed_first_try=true&review_rounds=1&reverted_within_7d=false"
done
```

## Expected output

`.forge-knowledge/mastery-smoke/knowledge.json` after one clean merge:
**5 entries**, one per specialist, all at `level:expert` (because
`(clean-regress)/total * 50 + 50 = 100` on a single clean signal):

```
$ jq -r '.[].category' .forge-knowledge/mastery-smoke/knowledge.json | sort -u
mastery-implementer-ncmlabs/forge-playground
mastery-planner-ncmlabs/forge-playground
mastery-release_manager-ncmlabs/forge-playground
mastery-reviewer-ncmlabs/forge-playground
mastery-tester-ncmlabs/forge-playground
```

Fire regression tasks (outcome=rejected, review_rounds=3, reverted) to
watch tuples regress back down:

```bash
curl -X POST 'http://localhost:3214/complete' \
  -d 'task_id=T-4&repo=ncmlabs/forge-playground&outcome=rejected&ci_passed_first_try=false&review_rounds=3&reverted_within_7d=true'
```

Each regress signal lowers the score. After enough regressions, levels
transition back down (expert → journeyman → apprentice → novice).

## Event trace

`/__forge/events` SSE shows:

1. `TaskCompleted` fires, subscribers: 1 (coordinator)
2. Coordinator emits 5 `MasterySignal` events (one per specialist)
3. Each `swarm_mastery_tuple` child matches its own signal via the
   `where specialist == memory.specialist and project == memory.project`
   filter
4. On level change, tuple emits `MasteryUpdated` and persists a
   snapshot into the shared knowledge store under
   `mastery-{specialist}-{project}`

## Runtime note (spawned agents share the knowledge store)

Spawned child agents in FORGE historically received a **fresh** (empty)
knowledge store — each child writing to the same declared path would
overwrite the others. The runtime was fixed as part of T5.2 to inherit
the parent's shared knowledge store when the child declares `knowledge`
and no `with knowledge where category` filter-transfer is requested
(the toolkit-demo pattern). See `src/runtime/executor.rs` spawn path.

For this inheritance to work, the coordinator (or any ancestor agent)
must declare a `knowledge store: ...` block so the program's shared
store is wired into its context.
