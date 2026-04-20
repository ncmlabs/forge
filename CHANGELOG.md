# Changelog

All notable changes to FORGE will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- `WakeService` `mode: wake` + session rehydration (T4 prerequisite): new `src/runtime/agent_lifecycle.rs` exposes `AgentLifecycle::rehydrate_or_spawn(alias)`, which reuses the live `AgentProcess` if one is registered and otherwise spawns a fresh one — restoring `memory persistent` from redb (`agent:{name}:memory`) and attaching a bus subscription BEFORE returning. `WakeService::with_lifecycle` wires the helper into the dispatcher so `schedule ... mode: wake` now routes through `dispatch_wake`: it emits `schedule_rehydrated { memory_keys_restored }` before publishing the `emit:` event (falling back to `{schedule_name}.tick` when no `emit:` is declared). Tracer ordering is load-bearing for replay — `schedule_fired → schedule_rehydrated → event_delivered` is asserted in a new `RecordedClock` test mirroring the #332 writer/replay pattern. When no lifecycle is wired, wake fires degrade cleanly: `session_rehydrate_failed` is traced and the fire is counted as `NoSubscribers`. Five new integration tests cover memory-keys-in-tracer, `.tick` fallback, lifecycle-missing failure, instance reuse across fires, and wake-mode replay determinism. New `schedule` Observer timeline category (`examples/observer/static/js/timeline.js`) covers `schedule_fired`, `schedule_rehydrated`, `schedule_skipped_*`, `schedule_errored`, `schedule_claim_lost`, and `session_rehydrate_failed` — the #332 events were previously unmapped, so this catches both issues. New `examples/agents/wake-rehydration-smoke/` reference example registered in `examples/validation.toml`. Unblocks #334 (CorrelationDriver / T4.4 Slack-thread continuity) and #335 (WebhookDriver / T4.2 cross-project handoff) — both reuse `AgentLifecycle::rehydrate_or_spawn` (#333)
- Derived-surface drift automation (issue #229, continuous): two-layer mechanism to keep `docs/forge-reference.md`, `skills/claude-code/SKILL.md` FORGE preamble, and checked-in `.forge` examples aligned with compiler/runtime source truth as language work lands. **Layer 1** (`.github/workflows/forge-surface-hint.yml`) posts/updates a single checklist comment on any PR that touches `grammar/**`, `src/ast.rs`, `src/parser.rs`, `src/resolver.rs`, `src/checker/**`, `src/runtime/**`, `src/main.rs`, or `src/build.rs` — zero LLM cost, just an author-time nudge. **Layer 2** (`.github/workflows/forge-surface-audit.yml`, Mondays 07:00 UTC + manual dispatch) invokes `claude -p` non-interactively with `scripts/audit-forge-surface.prompt.md`, enforces a path-guard that rejects any edits outside `docs/`, `skills/`, `examples/`, `workflows/`, and `CHANGELOG.md`, runs `cargo fmt --check` / `clippy -D warnings` / `cargo test` / `scripts/check-forge-examples.sh`, and if the diff is non-empty opens a draft PR `chore: FORGE surface audit (YYYY-Www)` labeled `surface-audit`. Requires repo secret `ANTHROPIC_API_KEY`. Does not replace `tests/example_validation_tests.rs`; complements it (#229)
- `WakeService` + `CronDriver` + `mode: spawn` (T5 scheduler MVP): the runtime half of the schedule subsystem introduced by #331. New `src/runtime/wake_service.rs` owns a tokio task spawned by `SystemRuntime::start` after agents subscribe; it reads declared schedules from `AgentDecl.schedules`, persists per-(agent, schedule) rows in a new redb `FORGE_SCHEDULES` table, and — on each tick — publishes a bus event named after the schedule, carrying the `prompt:` text in `fields["prompt"]`. Agents consume scheduled events through the normal `on <schedule_name>` handler path — the agent event loop at `src/runtime/agent.rs:772` is unchanged (the issue's load-bearing contract). Four differentiators over OpenClaw's reference scheduler ship as specified: (I) compile-time correctness via #331; (II) wall-clock is a `Clock`-trait oracle on the determinism boundary, with `SystemClock` / `MockClock` / `RecordedClock` impls and a replay test proving identical firing sequences across writer and replay passes; (III) a caller-injected budget gate (v1: session-level `CostTracker`, per-agent upgrade path wired in the hook signature) emits `schedule_skipped_budget` and defers; (VIII) a transactional claim via `ForgeStorage::try_claim_schedule` (redb write-txn serialization) gives exactly-once semantics for concurrent in-process dispatchers; cross-process isolation is enforced one level up by redb itself (a second `Database::create` on the same `.forge-data/` file fails with "Database already open" — a stronger guarantee than a cooperative lock file). `CronDriver::next_fire` covers all three `WhenExpr` forms (`daily at`, `every`, raw 5-field cron) using the existing `croner = "3"` dependency — no new cron crate. Five new tracer events: `schedule_fired`, `schedule_skipped_concurrent`, `schedule_skipped_budget`, `schedule_errored`, `schedule_claim_lost`. `catchup_policy: once` is hardcoded for v1 (AST extension deferred); `precision: high` bumps the tick from 60s to 1s; declared-but-removed schedules are GC'd at registration time. Wires into real forge-sensei (`workflows/forge-sensei/server/agent.forge` now declares `schedule mastery_review: daily at "09:00" mode: spawn`, replacing the manual `/update_mastery` CLI trigger) and a self-contained smoke fixture (`examples/agents/mastery-cadence-smoke/`, registered in `examples/validation.toml`). 37 new tests (7 clock, 11 storage, 15 wake_service unit, 5 integration including the RecordedClock replay determinism test and a 16-way concurrent-claim race). Unblocks #333 (`mode: wake` dispatcher), #334 (CorrelationDriver), #335 (WebhookDriver), #336 (Observer schedule lane), and T5 mastery cadence for forge-sensei (#332)
- `schedule` block grammar + AST + parser + compile-time checker (foundation for the WakeService subsystem): new top-level block inside `agent_decl`, parallel to `timer_field` but durable and cross-session. Three `when:` forms (`daily at "HH:MM"`, `every <duration>`, `cron "m h dom mon dow"` — strict 5-field Unix via the new `croner` dependency) and two `mode:`s (`spawn` requires `prompt:`, `wake` requires `emit:` or a paired `on <name>.tick` handler). New `schedule_checker` pass emits ten span-annotated diagnostics (missing `when:`/`mode:`, spawn-without-prompt, wake-without-pair, duplicate name, duplicate option, invalid cron, invalid time literal, zero duration, name collision with timer or handler event, plus warnings for extraneous `emit:` under spawn or `prompt:` under wake). Runtime dispatch still unimplemented — a well-formed `schedule` block parses and checks green but does not yet fire. Blocks #332 (CronDriver MVP), #333 (wake-mode rehydration), #334 (CorrelationDriver / T4.4), #335 (WebhookDriver / T4.2), and #336 (Observer schedule lane). 27 new tests (11 parser golden ASTs + 16 checker variants). Grammar reference docs updated with a new "Schedules" section (#331)
- Mastermind agent pattern (T1.2): new `examples/agents/mastermind-pattern/` packages the classify-and-dispatch topology first proven by T1.1's `clone-dev-skeleton` into a named, documented, self-contained reference example. `main.forge` (~140 lines) carries the minimal shape — `TicketInbound` → mastermind classifies via `reason` with defensive `if kind ==` branching → `TaskRouted(task_id, target_agent, ...)` → two `say`-only specialists filter with `subscribe TaskRouted where target_agent == "<self>"` → one `org_warden manages [mastermind, support_specialist, billing_specialist]`. `README.md` documents the four invariants (classification / target_agent routing / persistent `task_id`+`task_graph` / single warden), a when-to-use-vs-not table naming `inbound-triager` (single-hop), `workflows/dev-cycle` (sequential pipeline), `slack-adapter` (fan-in adapter), and `approval-gate` (human-gated single flow) as alternative shapes, a recipe for adding a new specialist, correlation-id conventions, warden integration guidance, and a reference table of adopters. Zero new runtime primitives; self-contained (`[skills]` empty, Haiku + `quality_tier=balanced` pinned in `forge.config.toml` to prevent warden-budget escalation on first curl). Downstream issue #305 (T6.1 meeting ingest) flagged to adopt by reference (#295)
- Mastery-over-time Observer tile (T5.3): new `/__forge/inspect/mastery` endpoint returns per-(specialist, project) level transitions (parsed from `mastery-{specialist}-{project}` knowledge-store entries written by T5.2) and per-task `review_rounds` history from a new `TaskHistoryAggregator` that subscribes to `TaskCompleted` on the event bus. New Observer tile (`examples/observer/static/js/mastery.js` + Mastery tab) renders three panels: mastery score over time (D3 multi-line chart per specialist with novice/apprentice/journeyman/expert bands), approval-asks-per-task trend (D3 bars with regression line — the #292 proof-point visualization), and a summary table. Frontend refetches on SSE `TaskCompleted`/`MasteryUpdated` emissions (no SSE payload enrichment needed). First brick of T7 cockpit (#308); aggregator is reusable (#304)
- Task graph in mastermind memory (T4.1): `examples/agents/clone-dev-skeleton/main.forge` replaces the flat `task_graph: Text[]` of pipe-delimited strings with a typed `TaskNode[]` graph (`{task_id, status, blocked_on, specialist, project}`), adds four events (`TaskBlocked`, `TaskCompleted`, `UnblockTask`, `CycleDetected`), and extends the mastermind with `on TaskBlocked` (1-hop cycle refusal → emits `CycleDetected` and escalates via warden) and `on TaskCompleted` (scans dependents, emits `UnblockTask` for any that become fully free; completed node flips to `status: "done"`, remaining blockers drop the completed id from `blocked_on`). Graph persists via `memory persistent` (leverages #57). Two smoke endpoints (`POST /task_blocked`, `POST /task_completed`) drive the flows end-to-end pending T4.2 cross-project producers. Cycle detection is intentionally shallow (1-hop, per DoD's "simple cycle detection"); deeper wedges fall through to the warden's escalate-after-3 policy (#299)
- Swarm mastery FSM (T5.2): new `SwarmMastery` states (novice → apprentice → journeyman → expert with regression paths), `MasterySignal` + `MasteryUpdated` events, and per-(specialist, project) mastery tracking in `workflows/dev-cycle/main.forge`. On `TaskCompleted`, a `swarm_mastery_coordinator` agent attributes signals per specialist (planner/release_manager own merged, implementer/tester own `ci_passed_first_try`, reviewer owns `review_rounds`) and fans out `MasterySignal` events to `swarm_mastery_tuple` children — one spawned per (specialist, project) tuple. Each tuple accumulates clean/regress counts, computes mastery score via `compute_swarm_score`, transitions its `SwarmMastery` lifecycle, and persists level changes into the knowledge store under `mastery-{specialist}-{project}`. Runs side-by-side with (does not replace) the sensei conformance-based mastery FSM (#303)
- Outcome-driven learning loop (T5.1): `workflows/dev-cycle/main.forge` now emits `TaskCompleted(task_id, outcome, ci_passed_first_try, review_rounds, time_to_merge, reverted_within_7d)` from the release_manager on `PRMerged`, and each of the five specialists (planner, implementer, tester, reviewer, release_manager) subscribes to its own task's completion to emit `LessonExtracted(task_id, agent_id, category, content, confidence)` and `learn` a perspective-specific lesson ("this plan shape → outcome", "this error pattern → fix", "this coverage gap → miss", "this objection → response", "this PR shape → merge path") into the shared knowledge store under agent-scoped category prefixes (`lesson-{agent}-{repo_slug}`). `ci_passed_first_try` and `review_rounds` signals are threaded from tester/reviewer through `AcceptanceMet` and `PRMerged` (#302)
- Slack adapter agent (`examples/agents/slack-adapter/`): one agent wraps all outbound Slack. Specialists stop calling `skill.slack.*` directly and instead emit seven typed events — `PostApproval`, `PostApprovalResult`, `PostMessage`, `PostThreadReply`, `AddReaction`, `RequestHuman`, `WardenEscalation` — which the adapter handles through a shared template library (five plain-text templates for T3.1; rich Block Kit deferred). Pool of 1 worker with `strategy: fastest` provides serialized rate-limit backpressure. Migrates `pr-review-bot` and `approval-gate` to emit events instead of calling Slack skills directly; both drop `skill.slack` from their project manifests. A `smoke.forge` sibling exposes one HTTP endpoint per event for live verification. Live-verified against a real Slack workspace: all seven event types deliver end-to-end (incl. Approve/Reject buttons with correlation). Canonical "adapter" pattern for the clone-developer track (#298)
- Slack skill `send_approval` capability is now fully wired: previously declared without an executor (silent-success stub), it now posts `chat.postMessage` with interactive Block Kit buttons whose `value` fields encode `approved:{request_id}` / `rejected:{request_id}` for webhook correlation (#298)
- Implementer iteration loop: on test failure the implementer now diagnoses failures via a dedicated `diagnose_failures` task before generating fixes, tracks per-iteration diagnosis history in `memory.iteration_log`, and escalates with structured context (issue ID, iteration count, diagnosis history, last failure output) when the configurable `memory.max_iterations` cap is reached. Warden `dev_lead` serves as backstop. Both `dev-cycle/main.forge` and dogfooding `dev-cycle.forge` updated (#296)
- PR history miner agent (`examples/agents/pr-history-miner/`): mines merged PRs from a GitHub repo, characterizes each diff with `reason`, summarizes reviewer feedback, and stores structured decision-history entries in the knowledge store via `learn` with `category: "pr-decisions-{project}"`. Idempotent via persistent watermark, resumable across restarts, rate-limit aware. Seeds the knowledge store for T2.2 (reviewer consults prior decisions) (#294)
- GitHub skill: `list_prs(repo, state, limit)`, `get_pr_reviews(repo, pr_number)`, `get_pr_diff(repo, pr_number)` — three new deterministic executor capabilities for PR history mining and review analysis

### Fixed
- Session `AgentResult.cost_usd` now reflects the adapter's reported total instead of always being 0 in `--print`/`stream-json` modes. `SessionManager::inject_cost` previously overwrote the `cost_usd` field (already populated by `parse_final` from `total_cost_usd`) with the session accumulator, which only grows via `SessionDriverEvent::Progress { cost_delta_usd }` — Claude CLI's streaming `assistant` events carry no per-turn cost, so the accumulator stayed at 0 and clobbered the final total. `inject_cost` now reconciles the two sources (max), updates `SessionState.cost_usd` in place, and emits `BudgetUpdated` so tracer/status queries see the authoritative total. `examples/session/session_claude_e2e.forge` now prints a non-zero `cost_usd` (#272)
- Agent-originated `emit` now reaches the SSE `/__forge/events` trace stream. `forge serve` previously built the shared `EventBus` with a `None` tracer, so `EventBus::publish` — the path exercised by the agent handler drain (`agent.rs:drain_event_sink`) — silently skipped `event_emit` / `event_delivered` tracing. Endpoint emits appeared only because the executor traced them a second time on the side. The fix injects `executor.tracer().cloned()` into `EventBus::new_shared` at both serve entry points (watch + non-watch) and removes the now-redundant explicit `event_emit` call on the endpoint path, leaving the bus as the single source of emit/delivery tracing. Observer tiles (#299, #300) that render agent fan-out in real time no longer need `HandlerCompleted`-based workarounds (#325)
- Knowledge store dual-instance bug: `ingest-fact` and `learn-from-session` data is now immediately queryable via `recall`/`query` without a pretrain cycle. The agent and endpoint executor now share a single `SharedKnowledgeStore` (`Arc<Mutex<KnowledgeStore>>`) instead of creating separate instances. In interpreter mode (`forge serve`), endpoints previously had no knowledge store at all (#309)
- Knowledge store O(N²) corpus build: `add_entry` now uses incremental `index_entry` (O(1) per add) instead of full `rebuild_index` (O(N) per add). Content deduplication prevents the same fact from being stored multiple times (#280 quick-fix)
- Hot-reload now preserves the knowledge store across code changes in `forge serve --watch`, matching the existing storage and session manager preservation pattern

### Changed
- FORGE surface audit (2026-W17): `docs/forge-reference.md` warden Responses table adds missing `downgrade` response (present in `grammar/forge.pest` `ward_response` rule, `src/ast.rs` `WardResponse::Downgrade`, and `src/checker/warden_checker.rs` severity-ordering help text); severity numbers adjusted: `restart`→3, `replace`→4, `escalate`→5.
- FORGE surface audit (2026-W16): `docs/forge-reference.md` reserved-keyword list now matches `grammar/forge.pest` exactly (added 16 missing hard-reserved tokens: `downgrade`, `to`, `recall`, `learn`, `spawn`, `find`, `retire`, `exec`, `exportable`, `import`, `from`, `as`, `command`, `background`, `session`, `schedule`); warden failure-types table gains `contradiction` and "five failure types" corrected to "six" throughout; schedule status note updated to reflect `WakeService`/`CronDriver` shipping in #332/#333; `skills/claude-code/SKILL.md` preamble adds `schedule` (`mode: spawn`, `mode: wake`) as a fully-dispatched FORGE surface.
- Surface audit now understands contextually-reserved keywords. `grammar/forge.pest` gains a structured `// ---- Contextual keywords ----` annotation block above the `keyword` rule listing every token-to-enforcing-rule mapping (search → search_expr, can → can_signature, on → on_handler, etc.). `scripts/audit-forge-surface.prompt.md` teaches the auditor that `docs/forge-reference.md` distinguishes two classes: "Reserved keywords" (must match the `keyword` rule exactly) and "Additional contextual keywords" (a curated subset of the annotation — never auto-remove unless absent from both sources; never auto-add). Relocates `search` from the hard-reserved doc list to the contextual list where it belongs — this is the drift PR #341 tried to address via deletion. Future weekly audits now produce zero diff on this front (#345)

### Added
- Clone-dev walking skeleton (`examples/agents/clone-dev-skeleton/`): proves the 3-hop topology `HTTP → mastermind → specialist → Slack` end-to-end using only existing runtime primitives. The mastermind classifies inbound `/clone_dev` tasks via `reason`, assigns a `task_id`, and emits `TaskRouted(task_id, target_agent, kind, ...)`; `pr_reviewer` (forked from `pr-review-bot` with the body intact) and `echo_specialist` subscribe with `where target_agent == "<self>"` filters. `org_warden` supervises all three agents. First concrete move on the Clone-developer track (#292, Layer 3 kickoff) and the gate that unfreezes #184 dev-cycle-executor rescope (#293)
- Session + Knowledge integration (`examples/session/session_claude_knowledge_live.forge`): demonstrates the full learning loop — agent delegates a question to Claude Code via `session`, persists the answer with `learn from interaction`, and proves persistence with `recall`. Composes three primitives in one `forge run`-able program. Closes Phase 2 M11 (Knowledge School) (#273)
- Auto-dispatched `start` lifecycle handler: `AgentProcess::run()` now fires `on start` automatically when an agent comes up, so agents can self-initialize (memory setup, banners, kicking off work) without an external event-bus trigger. Closes a Composition gap that left `on start` handlers dormant under `forge run` (#273)
- `spawn` from `fn main` is now synchronous: when called outside an agent context, the runtime awaits the spawned agent's event loop instead of fire-and-forget. Lets `fn main` act as a supervisor that waits for child agents to retire (#273)
- Approval gate pattern (`examples/agents/approval-gate/`): standard ApprovalRequest/ApprovalResponse events, dedicated `POST /webhook/approval` endpoint handling Slack interactive payloads (form-encoded) and direct JSON, agent lifecycle with `waiting_approval` state, timer-based timeout with re-notification (up to 3 retries then escalate), filtered event subscription for request correlation. Updated Slack skill `send_approval` with `request_id` parameter for button-value correlation. Completes P2.M9 Orchestration (#182)
- Inbound Triager agent (`examples/agents/inbound-triager/`): subscribes to SlackMention, IssueCreated, and PRReviewNeeded events, classifies each via LLM into route/escalate decisions, emits TriageRouted events for downstream ProjectAgents, and escalates to humans via Slack. Includes persistent memory, knowledge store for learning routing patterns, warden supervision, and live Slack integration (#181)
- Session worker smoke test (`examples/session/session_claude_e2e.forge`): FORGE delegates to Claude Code to write a code-analyzer FORGE program, then validates it with `forge check`, executes it with `forge run`, and verifies the full cycle — proving language bootstrapping end-to-end (#241)
- Cross-platform install scripts: `install-sensei.sh` now works on macOS, Linux, and WSL. Replaced macOS-only `date -j -f` with portable `python3` fallback, added `--uninstall` flag for service teardown, and WSL/systemd preflight warnings. CI job verifies install/uninstall round-trip on Ubuntu (#257)
- Cross-platform `StartupManager` trait (`src/runtime/startup/`) with per-OS backends for launchctl (macOS), systemd user units (Linux/WSL), and schtasks (Windows). Generated FORGE server binaries now expose `install-service` / `service {start|stop|status|uninstall}` subcommands that emit JSON, replacing the macOS-only shell glue in `install-sensei-server.sh` (which is now a thin wrapper) (#254)
- Unified persistent storage root: new `[storage] root` key in `forge.config.toml` (with `FORGE_STORAGE_ROOT` env override and legacy `knowledge.store_path` fallback) routes CLI and server redb databases to the same file regardless of working directory. `ForgeStorage::open_from_config` / `resolve_root` replace the ad-hoc `./.forge-data` joins in main.rs and build.rs (#253)
- Agent lifecycle events on the tracer broadcast: `AgentStarted`, `HandlerStarted`, `HandlerCompleted` (status: `success` | `timeout` | `error` | `blocked_by_requires`, with `duration_ms` and `confidence`), and `AgentShutdown` (reason: `retire` | `channel_closed` | `error`). Automatically surfaces on `/__forge/events` SSE and the cost-aggregator channel so Observer and internal agents can see the runtime run itself (#255)
- `proc.exit(code)` runtime primitive: one-shot CLI handlers can signal a non-zero exit, translated by the generated dispatch into `std::process::exit(code)` without the error-formatting path (#258)
- forge-sensei client `check` command: reports `server ok` and exit 0 when `/api/status` is reachable, prints a clear unreachable message with start instructions and exits 1 otherwise (#258)
- forge-sensei client handlers now exit 1 when the server is unreachable (query/review/status/ingest/etc.), so shell scripts and CI can rely on the exit code (#258)
- forge-sensei native boundary split: shared/server/client FORGE projects now build separate server and pure HTTP client binaries, with client-safe API endpoints for sensei commands (#256)
- forge-sensei `/api/self-assess` endpoint: triggers built-in curriculum evaluation and persists mastery internally via `data.store` — no external assess.sh/cron needed (#247)
- forge-sensei status endpoints (`/api/status`, `/status` HTML) now reflect persisted mastery state instead of hardcoded 0 (#247)
- Half-open circuit breaker recovery: warden no longer terminates on trip — enters cooldown → probe → resume cycle, making servers self-healing when providers recover (#247)
- Generated server binaries now support `--check` (validate config + provider health), `--config <path>` (explicit config), and `--reset` (clear persisted state) CLI flags (#247)
- Startup health check and banner: servers print config source and provider reachability on boot, continue in degraded mode if providers are unreachable (#247)
- Server resilience integration tests: HTTP binding under provider failure, health check API, circuit breaker reset verification (#247)
- forge-sensei server/client deployment path: `forge build --entry/--source` can build the composed server binary, generated sensei CLIs can route commands through `--server`, JSON `/api/*` endpoints cover CLI workflows, and install scripts now install CLI/server wrappers plus a macOS LaunchAgent helper (#240)
- Toolkit agent knowledge transfer infrastructure: forge-sensei subscribes to `LearnedInsight` events from toolkit and specialist agents, closing the feedback loop so spawned agents' learnings compound in sensei's knowledge store (#167)
- `--version` flag for all FORGE agent binaries built with `forge build`, sourced from `forge.project.toml` version field (#167)
- Knowledge transfer integration tests: export-by-category with confidence cap, AgentTransfer source tagging, full seed→export→cap→merge→recall cycle (#167)
- Conformance tests for `subscribe` with compound `or` filter and spawn-with-knowledge runtime execution (#167)
- Example toolkit agent (`examples/agents/toolkit_demo.forge`) demonstrating spawn→recall→emit→absorb pattern (#167)

### Fixed
- forge-sensei `update-mastery` CLI command now actually advances mastery against a live daemon. Form-encoded POST bodies (e.g. `score=75`, sent by `web.post` with `Content-Type: text/plain`) previously flattened every value to `Text`, so `api_update_mastery(score: Number)` received `Text("75")`, failed the type check, and the client's `try web.post … or ""` collapsed the empty response into "server unreachable at http://127.0.0.1:3000". Extracted `coerce_to_param_type` in `src/runtime/http_server.rs` and wired endpoint parameter types through both the form-encoded POST branch and the GET query-string branch — now symmetric with the raw-body single-param path that already coerced. Also removed the `|| true` guard in `scripts/sensei-assess.sh` so a future regression surfaces instead of silently completing green (#282)
- forge-sensei `/api/ingest` (path-based document ingestion) now actually persists. The endpoint called `learn from document(path)` directly, but that primitive requires agent context — HTTP endpoints run outside any agent, so the runtime rejected it with "learn outside agent" and the CLI (`try web.post … or ""`) collapsed the error into "server unreachable". Rewrote the endpoint to emit `IngestRequested(source: "api")`, added a subscription + handler in `forge_sensei` that runs `learn from document(path)` inside agent context — the same pattern #284 established for `/api/ingest-fact`. Unblocks `scripts/pretrain-sensei.sh` phases 1–3, 5–6 (which had been silently failing every file against a live daemon) (#283)
- Agent stuck detector (`src/runtime/agent.rs::StuckDetector`) no longer trips on Unit-returning event-subscription handlers. `dispatch()` now skips `record_turn` when a handler completes without hitting a `give` — event-absorption handlers (`on LearnedInsight` and the like) do bounded side-effect work rather than produce a response, so feeding them into the Jaccard "all similar" check was both semantically wrong and, in practice, tripped stuck after 3 consecutive unit turns (empty `response_text` compares as Jaccard=1.0). Previously tanked `pretrain-sensei.sh` and the operational-readiness pipeline by tripping the warden circuit breaker after ~2 facts (#286)
- Subscribe filters can now reference agent memory. `AgentProcess::should_handle` binds `memory` into the filter eval env the same way `dispatch` does, so documented patterns like `subscribe LearnedInsight where category == memory.topic` (README, forge-reference, forge-sensei specialist) no longer error with `UndefinedVariable { name: "memory" }`, crash the agent task, and cascade into a warden restart loop. Latent bug uncovered while verifying the #286 regression test end-to-end (#286)
- forge-sensei `/api/ingest-fact` (and `POST /webhook/webhook_ingest`) now actually persist facts to the knowledge store. The subscription filter in `forge_sensei` was `where source == "toolkit" or source == "specialist"`, which silently rejected events emitted by HTTP endpoints (`source: "api"`) and webhooks (`source: "webhook"`) — the endpoint returned `Learned [CATEGORY]` while the event was dropped at the agent's event loop. Widened the filter to include `"api"` and `"webhook"` so external ingest actually reaches `learn` (#284)
- `tests/sensei_live_tests.rs` harness now builds and starts the declared `SystemRuntime`, matching production (`src/main.rs::serve_program`). Previously the harness created a server with an event bus but never spawned the `forge_sensei` agent, so every endpoint-`emit` → agent-`subscribe` path ran with no subscriber and regressions like #284 went undetected. Tests also route knowledge-store writes to a per-test tempdir instead of `~/.forge/sensei` (#284)
- forge-sensei client handlers (query/review/ingest/deep-dive/update-mastery) now form-encode their POST bodies (e.g. `question=...`) so the server can bind the parameters instead of failing with `undefined variable` (#258)
- `install-sensei.sh` default config now points to vLLM on `192.168.10.195:8000` instead of stale Ollama localhost defaults; server wrapper uses `--config` flag for unambiguous config resolution (#247)
- Multi-file project builds (`forge build`) no longer fail on cross-file lifecycle references; checker now validates the merged program (#167)
- Quiz tutor example now runs cleanly with `FORGE_MOCK=1 forge run examples/agents/quiz_tutor.forge`, and mock-runnable examples reject checker warnings in the validation gate (#231)
- FORGE reference, training workflow doc, and local examples now cover implemented command/session/AgentResult/verification/knowledge/skill surfaces (#229)
- Anthropic provider now sends tool definitions and parses `tool_use` response blocks, enabling skill executor agentic loop (#198)
- OpenAI-compatible provider now sends tool definitions and parses `tool_calls` response fields (#198)

### Changed
- Skill capabilities can now opt into deterministic command execution metadata, letting simple `skill.*` operations bypass the LLM-mediated SKILL.md loop while keeping agentic fallback behavior (#237)
- Bumped `redb` from v2 to v4 and MSRV from 1.85 to 1.89 (#235)

### Added
- Mock-only FORGE example validation gate: every checked-in `.forge` example is classified in `examples/validation.toml`, expected-error examples assert diagnostics, live/external examples are counted as skipped, and `scripts/check-forge-examples.sh` runs the fast `FORGE_MOCK=1` validation path (#231)
- Slack skill — bidirectional Web API via `curl` with 13 typed capabilities: send-message, send-rich-message (Block Kit), reply-thread, add-reaction, list-channels, read-history (incremental polling), read-thread, detect-mentions, send-approval (interactive buttons + webhook callback), edit-message, delete-message, pin-message, member-info (#177)
- GitHub skill — `gh` CLI wrapper with 8 typed capabilities: create-issue, list-issues, create-branch, create-pr, check-ci, merge-pr, delete-branch, close-issue (#176)
- Contradiction events and warden integration: `Contradiction` failure type in grammar/AST/parser/checker, `AgentSignal::Contradiction` variant, `SessionEvent::ContradictionDetected` with `session.contradiction` EventBus payload, `ContradictionSummary` persisted in session state for resume, verification gate in executor blocking high-risk actions on contradicted results (#205)
- Default contradiction escalation policy: nudge → restart (after 2) → escalate (after 4) — built-in fallback when no explicit `on contradiction:` warden policy exists (#205)
- Warden signal channel in SessionManager: contradictions detected during session verification are automatically reported to the warden for policy resolution (#205)
- Verification engine: 5-stage validator pipeline (schema, reference, environment, execution, policy) that resolves pending `VerificationResult` from claims to `Verified`/`Insufficient`/`Contradicted`/`Error` by checking real filesystem and test state (#204)
- `VerificationEngine` orchestrator with pluggable `Validator` trait, integrated into `SessionManager.mark_completed()` for automatic verification on session completion (#204)
- Risk classification helper `classify_risk()` — derives `RiskClass` from AgentResult fields and metadata side-effect markers (#204)
- Verification contract types: `Claim`, `Evidence`, `VerificationResult`, `Contradiction`, `RiskClass` — runtime claim-evidence-verification model for trust progression, accessible via `AgentResult.metadata.verification` (#203)
- Implicit claim extraction from AgentResult fields: `files_changed`, `tests_run`/`tests_passed`, and `plan` automatically seed verification claims during session result parsing (#203)
- `VerificationResult` predicates: `is_verified()`, `is_contradicted()`, `is_actionable(max_risk)` for downstream approval gates (#203)
- Sandbox isolation: `isolate worktree "branch"` modifier on `spawn` and `session` creates a git worktree at `.forge-data/worktrees/{branch-slug}/` for filesystem isolation, with automatic cleanup on agent retire or session completion (#194)
- Session polling + event integration: `SessionManager` publishes typed events (`session.progress`, `session.complete`, `session.failed`, `session.cancelled`) directly to `EventBus`, enriched with timestamp, message, duration, and final status fields (#192)
- Timer-based session polling with configurable interval (default 5s) — publishes `session.poll` events for active sessions to EventBus (#192)
- `session.status(id)` imperative expression — query session state from FORGE code, returns Record with `status`, `cost_usd`, `started_at`, `updated_at`, `error` fields (#192)
- Declarative adapter system: `ADAPTER.toml` format for CLI agent integration with field mapping, permission modes, and progress detection (#191)
- Built-in adapters for Claude Code (`adapters/claude/`) and Codex (`adapters/codex/`) with verified CLI patterns (#191)
- `ConfigDrivenDriver` — generic `SessionDriver` that reads any ADAPTER.toml and handles process spawn, stdin piping, stdout parsing, and AgentResult extraction (#191)
- Adapter resolution chain: project local → installed → built-in, with `[adapters]` section in `forge.project.toml` (#191)
- Generic fallback adapter for arbitrary CLI agents (command = agent name, text output, 0.5 confidence) (#191)
- `SessionManager` runtime for long-running external agent sessions with UUID-backed session IDs, persisted JSON state under `.forge-data/sessions/`, startup `resume_all()`, progress listeners, budget enforcement, and graceful cancel escalation (#190)
- `session` runtime execution in `TaskExecutor` with `on progress` / `on complete` hook emission, `Text`/`AgentResult` output coercion, and lifecycle tracing for Principle VIII (#190)
- `session` expression front-end: grammar, AST, parser, checker, resolver, cost estimation, and runtime placeholder support with `agent`, `prompt`, `tools`, `timeout`, `budget`, `gives AgentResult`, and `on progress`/`on complete` emit hooks (#189)
- `AgentResult` built-in type — standard typed result contract for agent/session runs with 9 fields: `plan`, `patch_summary`, `files_changed`, `tests_run`, `tests_passed`, `cost_usd`, `confidence`, `approval_needed`, `metadata` (#193)
- `AgentResult` constructor with default fields: `AgentResult()` or `AgentResult(plan: "fix bug", confidence: 0.9)` — unspecified fields get zero/empty defaults (#193)
- `AgentResult` confidence sync: the `confidence` field value propagates to the `ConfidentValue` wrapper, enabling `when result.sure ->` branching (#193)
- Agent-to-CLI delegation research doc for Claude Code and Codex, including session adapter and AgentResult mapping tables plus live-tested invocation patterns (#165)
- Reference typed SKILL.md wrappers for Claude Code and Codex under `skills/` (#165)
- Rich SKILL.md capability signatures with multi-capability registration and compile-time validation of `skill.namespace.method(...)` calls (#164)
- `forge.project.toml` gains `[skills]` section for project-level skill declarations with `path` and `source` options (#163)
- Skill resolution chain: project local (`./skills/`) → installed (`.agents/skills/`) → global (`~/.forge/skills/`) with clear error on missing (#163)
- `skills-lock.json` integrity verification — warns when installed skill content diverges from lock hash (#163)
- Compile-time validation of `use skill.X` against project-declared skills via `CheckContext::with_skills()` (#163)
- Pure checker rejects `skill.*` calls inside `pure` functions (Principle II — determinism boundary) (#163)
- `command` expression primitive with optional `in`, `timeout`, and `background` modifiers (#160)
- Structured argv form for `command`: `command ["git", "commit", "-m", msg]` — safe, no injection (#160)
- `env` modifier for `command`: `command "build" env { RUST_LOG: "debug" }` (#160)
- `command` runtime execution — synchronous process spawning with structured record return (`stdout`, `stderr`, `exit_code`, `success`) (#161)
- `command` background mode — UUID handle-based lifecycle with `command.status()`, `command.output()`, `command.cancel()` (#162)
- `CommandManager` process manager with incremental output buffering, timeout auto-cancel, and graceful shutdown (#162)

## [0.1.0] - 2026-04-09

First release of FORGE — the agent-native programming language where LLM calls are oracle queries, not function calls. All 14 language primitives, 7 semantic checkers, full async runtime, and 3 showcase applications. 883 tests passing across unit, conformance, integration, and E2E.

### Added

#### Core Language
- Core language with 14 primitives: task, flow, agent, pool, system, warden, event, states, pure, endpoint, type, contract, use, uncertain
- PEG parser (609 rules) with comprehensive error diagnostics and span tracking
- Semantic checkers: purity, boundary, states, requires, uncertain, spawn, warden supervision
- `when` confidence branching (`when result.sure -> ... else ->`) for handling LLM uncertainty
- `try ... or` fallback expressions for graceful error handling
- Pattern matching via `match` expressions
- `for` loops over iterables
- Template strings with `{var}` interpolation and `{!expr}` raw interpolation for Html context
- `.split(delimiter)` string method, `.join(delimiter)` array method, `.length` field access on Text
- Variable reassignment in nested scopes (`for`/`if`/`match` blocks)
- Array concatenation via `+` operator
- Mixed `Text + Html` and `Html + Text` concatenation
- `downgrade` warden response — new severity level between Nudge and Restart
- Safe record field access — missing fields return `Unit` instead of crashing

#### Runtime & Execution
- Async executor (tokio-based) with automatic DAG parallelism in flows
- Agent lifecycle with event loops, memory (persistent + ephemeral), timers, and subscriptions
- Event bus with typed pub/sub for inter-agent communication
- Pool management with round-robin and fastest-wins strategies
- Warden supervision with escalation ladders (nudge → downgrade → restart → replace → escalate)
- State machine validation and lifecycle enforcement
- System runtime orchestration — shared event bus, instance registry, warden snapshots
- `forge serve` boots system runtime (agents + wardens) alongside HTTP server when a `system` declaration is present

#### LLM Integration
- LLM provider support: Anthropic, OpenAI-compatible, Ollama, Groq, Mock
- Token cost tracking and estimation per call and per agent
- Confidence tracking with sources: Deterministic, LLMDirect, ConsensusAgreement, Derived, KnowledgeRecall, ExecResult, SkillInvocation
- Multi-turn tool-use in mock provider for deterministic testing
- Unified tool-use in `CompletionRequest`/`CompletionResponse`

#### Agent Ecosystem
- `memory persistent` keyword — typed memory fields survive process restarts via write-through to ACID storage (redb)
- Progressive learning: `knowledge`, `recall`, and `learn` primitives for persistent, searchable knowledge stores
- Knowledge categories with `category:` suffix on `learn` statements
- Agent instance registry for runtime discovery — `spawn`, `find`, `retire` lifecycle primitives
- Dynamic warden `adopt()`/`release()` for runtime supervision management
- Agent portability: `exportable` modifier, `.forgepkg.json` package format, `forge export`/`import`/`inspect` CLI

#### Web Runtime
- `Html` builtin type with automatic `text/html` Content-Type and auto-escaping for XSS prevention
- `html.layout(title, body)` and `html.escape(text)` built-in capabilities
- `markdown.render(content)` — Markdown to HTML with tables, footnotes, task lists, FORGE syntax highlighting
- HTTP client: `web.fetch(url)` and `web.post(url, body)` with catchable failures
- Web search: `search "query"` with configurable provider (SearXNG)
- Static file serving with `[server.static]` config and `asset()` URL helper
- Hot-reload development mode via `forge serve --watch`
- Webhook support with JSON parsing and optional HMAC-SHA256 signature verification
- `emit` in endpoint handlers for publishing events from webhooks
- Multi-file `forge serve` with `-s` flag for dependencies
- Boundary enforcement: `web.fetch()`, `web.post()`, and `search` restricted to `boundary: server`
- Root URL redirect (`GET /` → `/home`)
- Automatic storage provisioning (`.forge-data/server.redb`)

#### Data & Embeddings
- `data.store(key, value)`, `data.get(key)`, `data.list(prefix)`, `data.delete(key)` — persistent KV storage backed by redb
- Vector embeddings and semantic search — `data.embed(content)` generates vectors via OpenAI-compatible providers; `data.search(query, top_k)` performs cosine similarity search with confidence from similarity scores; `EmbeddingProvider` trait with OpenAI-compatible and Mock implementations; `VectorIndex` with JSON persistence; `[embeddings]` config section; costs tracked in Observer (#50)

#### Skill System
- `exec` primitive — first-class CLI execution returning `uncertain<Text>` with confidence from exit code; enforced by pure checker, uncertain checker, and tracer (#40)
- Host skill bridge — LLM-mediated SKILL.md execution via `skill.namespace.method()` syntax; agentic loop with tool-use; results capped at 0.99 confidence (#40)
- `SkillExecutor` wiring at all `TaskExecutor::new()` call sites; respects `[skills]` config

#### Build System
- `forge build` — package FORGE programs as standalone CLI binaries with embedded sources and runtime
- Multi-file composition via `forge.project.toml` manifest with cross-file symbol dedup
- `forge run --manifest` for multi-file execution without building
- Generated agent binaries with clap subcommands per handler, interactive REPL, and `--help`
- Config embedding: `--embed-config` bakes config into binaries with runtime override support
- `--dry-run` mode for build validation

#### Observer & Introspection
- Live trace stream — `/__forge/events` SSE endpoint streaming all trace events as JSON; `say` statements emit trace events; broadcast channel survives hot-reloads (#138)
- Runtime introspection API — `/__forge/inspect/*` endpoints for agent state, system topology, warden health, storage keys as JSON (#139)
- Agent topology visualization — live D3 force-directed graph (#141)
- Cost and confidence dashboard — live token counts, USD cost, per-agent breakdown, confidence histogram (#142)
- Failure injection API — `POST /__forge/inject/:type` for testing warden policies (stuck, crash, timeout, hallucination, budget) (#143)
- Standalone Observer app — SPA connecting to any FORGE server with tabbed dashboard, D3 topology, cost panel, swim-lane trace timeline (#144)

#### Showcase Applications
- **FORGE Wiki** — dogfooding web app demonstrating all 14 primitives; content agent (CRUD + lifecycle), search agent (confidence gating), doc generation flow (5-stage DAG), fact-checking pool (3-worker majority vote), warden supervision (5 failure policies), Tailwind + DaisyUI UI (#59-#66)
- **Sentinel** — AI-powered repo health dashboard; `exec` for git data, `exec >> reason` composition, skill-based deep analysis, parallel scan flow, 5 HTTP endpoints (#132)
- **Observer** — standalone SPA for live agent tracing, topology, cost tracking (#144)
- **forge-sensei** — self-referential learning agent written in FORGE; knowledge store, mastery progression (novice→expert), specialist apprentices, conformance-based assessment, Claude Code integration

#### Testing & CI
- 883 tests: unit (200+), conformance (85), grammar (104), parser (78), AST (53), checkers (104+), runtime (70+), integration (50+), wiki acceptance (52)
- End-to-end wiki acceptance tests covering all 7 subsystems
- Playwright E2E tests for wiki, observer, and embeddings
- Conformance test suite for language correctness validation
- CI/CD on ubuntu, macos, windows with MSRV 1.89
- FORGE syntax highlighting (Prism.js language definition)

#### Documentation & Tooling
- Wiki documentation: README (quick start, config, endpoints, deployment) and ARCHITECTURE (system diagrams, feature map) (#66)
- `CLAUDE.md` with project development rules
- Development lifecycle workflow spec (`workflows/dev-cycle.forge`)
- `forge-workflow` Claude Code skill for guided development sessions
- forge-sensei scripts: build, pretrain, consult, assess, smoke-test, cache management

### Changed
- forge-sensei rewritten as flagship FORGE application with 8 new constructs and objective assessment
- forge-sensei runs as standalone binary (`bin/forge-sensei`) with multi-hop recall for improved retrieval accuracy

### Fixed
- Wiki Auto Reference & Fact-Check Report UX — fixed Record serialization, improved extraction prompts (#124)
- Wiki Q&A and search grounded in actual doc content, eliminating hallucinated syntax (#122)
- Sentinel dashboard overflow — long text contained with scroll (#140)
- Sentinel analyst missing insight storage (#140)
