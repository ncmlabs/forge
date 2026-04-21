# FORGE Observer

Standalone runtime inspector that connects to any running FORGE server. Like Erlang's `:observer`, but purpose-built for FORGE concepts: wardens, confidence, token economy, stuck detection, and knowledge stores.

## Quick Start

```bash
# Start a FORGE server to observe (e.g., Sentinel)
forge serve examples/sentinel/server.forge -s examples/sentinel/shared.forge

# In another terminal, start the Observer
forge serve examples/observer/server.forge -s examples/observer/shared.forge

# Open in browser
open http://localhost:3002/static/index.html?server=http://localhost:3001
```

The `?server=` parameter auto-connects to the target server. You can also type the URL manually in the connection bar.

## Views

### Tree
Supervision hierarchy: system root, wardens, managed agents. Click any node to inspect memory fields, timers, stuck/hallucination flags, event counts, and lifecycle state. Live SSE event stream shows runtime activity in real time.

### Topology
D3 force-directed graph showing agent composition wiring and warden supervision relationships. Nodes pulse on events; edges animate during data flow. Click any node for detailed inspection.

### Cockpit
Orchestration view for a running clone-dev / dev-cycle swarm (#308). Pipeline Kanban shows active issues by stage (planning → implementing → testing → reviewing → pr_ready → merged), derived from each specialist's `memory.issue_id` and lifecycle state. Decision Queue surfaces agents in any `awaiting_*` state with their PR and Slack links. Agent Activity strip renders live per-agent pills annotated with warden health.

### Costs
Token economy dashboard: total cost (USD), LLM call counts, token throughput, per-operation/agent/provider breakdowns, and confidence distribution histogram. Updates live via SSE.

### Timeline
Swim-lane visualization of all trace events over time. Six lanes: LLM, Exec, Flow, Events, Warden, HTTP. Brush-zoom to focus on a time range, filter by category, click any tick for full event details. Double-click to reset zoom.

## Configuration

The observer runs on port 3002 by default (see `forge.config.toml`). It makes no LLM calls and needs no API keys.

## How It Works

The Observer is a pure static SPA served by a minimal FORGE app. All data comes from the target server's introspection endpoints:

- `GET /__forge/inspect/topology` -- system graph
- `GET /__forge/inspect/agents` -- running instances
- `GET /__forge/inspect/agents/:id` -- deep agent state
- `GET /__forge/inspect/wardens` -- supervision health
- `GET /__forge/inspect/costs` -- token economy
- `GET /__forge/events` (SSE) -- live trace stream
- `POST /__forge/inject/:type` -- failure injection (testing)
