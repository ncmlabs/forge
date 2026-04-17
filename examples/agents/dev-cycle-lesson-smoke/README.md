# dev-cycle-lesson-smoke — T5.1 (#302) live acceptance test

Proves the outcome-driven learning loop end-to-end without needing
GitHub/Slack auth or a running dev-cycle pipeline: one endpoint hit
seeds the five dev-cycle specialists, emits a synthetic
`TaskCompleted`, and verifies each specialist writes a
perspective-specific lesson into the shared knowledge store.

Mirrors the production wiring in `workflows/dev-cycle/main.forge`:
same event signatures, same category scheme (`lesson-{agent}-{slug}`),
same shared knowledge store primitive. The difference is this file
strips out GitHub/Slack/git plumbing so the T5.1 pattern is the
only variable under test.

## Run

```bash
ANTHROPIC_API_KEY=... cargo run -- serve \
  --manifest examples/agents/dev-cycle-lesson-smoke/forge.project.toml \
  examples/agents/dev-cycle-lesson-smoke/main.forge --port 3213

curl -X POST 'http://localhost:3213/smoke' \
  -H 'Content-Type: application/x-www-form-urlencoded' \
  -d 'task_id=SMOKE-1&repo=ncmlabs/forge-playground'
```

## Expected output

`.forge-knowledge/dev-cycle-smoke/knowledge.json` gains **5 new
entries**, one per category:

- `lesson-planner-ncmlabs/forge-playground`
- `lesson-implementer-ncmlabs/forge-playground`
- `lesson-tester-ncmlabs/forge-playground`
- `lesson-reviewer-ncmlabs/forge-playground`
- `lesson-release-ncmlabs/forge-playground`

Event trace (`/__forge/events` SSE) shows `TaskCompleted` fan out to
**5 subscribers**, each `on TaskCompleted` handler running a real
`reason` call and landing a `LessonExtracted` event.

## Verify

```bash
jq 'length' .forge-knowledge/dev-cycle-smoke/knowledge.json  # 5
jq -r '.[].category' .forge-knowledge/dev-cycle-smoke/knowledge.json | sort -u
```
