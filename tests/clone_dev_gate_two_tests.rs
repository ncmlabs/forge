// Integration tests for T10.2 (#368) — gate_two agent under
// workflows/dev-cycle/gate_two.forge.
//
// Verifies the planner → Gate-2 → implementer hand-off slice:
//
//   PlanReady forks immediately:
//     gates_start_implementation == false → ImplementationApproved
//       emitted with decision_by = "auto (policy)"; no PostApproval.
//     gates_start_implementation == true  → PostApproval(request_id =
//       "plan-{issue_id}") to slack_adapter, transition to
//       waiting_approval, await ApprovalResponse.
//   ApprovalResponse(approved=true)  → ImplementationApproved
//     (decision_by = comment) + :white_check_mark: PostMessage.
//   ApprovalResponse(approved=false) → ImplementationRejected
//     (decision_by = comment) + :x: PostMessage. The planner then
//     consumes ImplementationRejected and re-emits PlanReady, bounded
//     by [defaults] max_plan_revisions.
//
// Cases:
//   1. plan_ready approval happy path — PlanReady → exactly one
//      PostApproval(request_id == "plan-{issue_id}") with title
//      "Start implementation: {issue_id}"; ApprovalResponse(true) →
//      ImplementationApproved(decision_by = comment) + :white_check_mark:.
//   2. plan_ready rejection           — same setup, approved=false →
//      ImplementationRejected(decision_by = comment, comment = comment)
//      and a :x: PostMessage. No ImplementationApproved.
//   3. auto-approve bypass            — fixture sets
//      gates_start_implementation=false → ImplementationApproved with
//      decision_by = "auto (policy)" emitted immediately, no PostApproval.
//   4. revision feedback loop         — planner is in compose, PlanReady
//      → reject → planner emits a fresh PlanReady (revision_count=1).
//      The bound is exercised via 4 sequential rejections — beyond the
//      configured max_plan_revisions=2, the planner emits no further
//      PlanReady (it escalates instead).
//   5. stale approval response        — ApprovalResponse with the wrong
//      request_id is ignored (no Implementation* events emitted).
//   6. approval channel routing       — when slack_approval_channel is
//      set, PostApproval and the confirmation PostMessage go to that
//      channel rather than the per-issue PlanReady.channel.
//
// The 5-minute approval_timeout escalation path is exercised in the
// real-LLM smoke run (clone_dev_gate_two_live_smoke.rs) rather than
// here — the test harness has no deterministic way to fast-forward
// FORGE timer ticks.
//
// Env-lock pattern mirrors clone_dev_gate_one_tests.rs verbatim:
// each test writes a fixture clone-dev.toml at a unique path under
// /tmp and sets FORGE_CLONEDEV_CONFIG. The static ENV_LOCK serializes
// tests so they don't stomp on the global env var; unique paths
// sidestep the process-wide config cache.
// ──────────────────────────────────────────────────────────────────────

#![allow(clippy::await_holding_lock)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use forge::compose;
use forge::runtime::confidence::{ConfidentValue, Value};
use forge::runtime::event_bus::{EventBus, EventPayload};
use forge::runtime::executor::TaskExecutor;
use forge::runtime::instance_registry::InstanceRegistry;
use forge::runtime::skill_executor::SkillExecutor;
use forge::runtime::skill_registry::SkillRegistry;

const GATE_TWO_PATH: &str = "workflows/dev-cycle/gate_two.forge";
const TYPES_PATH: &str = "workflows/clone-dev/shared/types.forge";
const DEV_CYCLE_AGENTS_PATH: &str = "workflows/dev-cycle/agents.forge";

// Window for `on start` (config load) + handler dispatch + emit
// propagation. Generous because config.load_clone_dev parses TOML
// from disk on first hit, and case #4 needs an extra LLM call
// resolution for each revision.
const HANDLER_WINDOW: Duration = Duration::from_millis(2500);

// Process-wide guard so parallel cargo test invocations don't race on
// FORGE_CLONEDEV_CONFIG. Tests serialize on this; the unique fixture
// path per test still avoids the config cache colliding (#357).
static ENV_LOCK: Mutex<()> = Mutex::new(());

// ── Test harness compose ──────────────────────────────────────
//
// gate_two reads PlanReady / ImplementationApproved / etc. — those
// events are declared in dev-cycle/agents.forge, which we source.
// We add a gate_probe agent that re-emits observed events as `say`
// frames and a test_system that ties everything together.
//
// We do NOT include the dev-cycle planner in the default harness —
// it carries an `on TaskCompleted` handler with a `reason` call that
// stalls under the mock LLM, and most cases drive PlanReady directly.
// Case #4 (revision loop) builds a separate program that includes
// the planner.

const TEST_HARNESS_BASE: &str = r#"#! boundary: server

warden test_ward
  manages [gate_two, gate_probe]
  on stuck: escalate, self
  on crash: restart, self
  on timeout: restart, self

agent gate_probe
  memory
    last_issue: Text
  subscribe ImplementationApproved
  subscribe ImplementationRejected
  subscribe PostApproval
  subscribe PostMessage
  subscribe WardenEscalation

  on ImplementationApproved(issue_id: Text, repo: Text, plan: Text, criteria: Text, branch: Text, channel: Text, callback_url: Text, test_cmd: Text, decision_by: Text)
    memory.last_issue = issue_id
    say "[probe] approved issue={issue_id} repo={repo} branch={branch} decision_by={decision_by}"

  on ImplementationRejected(issue_id: Text, comment: Text, decision_by: Text)
    say "[probe] rejected issue={issue_id} by={decision_by} comment={comment}"

  on PostApproval(channel: Text, title: Text, context: Text, callback_url: Text, request_id: Text, thread_ts: Text)
    say "[probe] post_approval channel={channel} request_id={request_id} title={title}"

  on PostMessage(channel: Text, text: Text, thread_ts: Text)
    say "[probe] post_message channel={channel} text={text}"

  on WardenEscalation(agent_id: Text, cause: Text, detail: Text, channel: Text)
    say "[probe] escalation agent={agent_id} cause={cause} detail={detail}"

system test_system
  use
    gate2: gate_two
    probe: gate_probe
"#;

const TEST_HARNESS_WITH_PLANNER: &str = r#"#! boundary: server

warden test_ward
  manages [planner, gate_two, gate_probe]
  on stuck: escalate, self
  on crash: restart, self
  on timeout: restart, self

agent gate_probe
  memory
    last_issue: Text
    plan_ready_count: Number
  subscribe PlanReady
  subscribe ImplementationApproved
  subscribe ImplementationRejected
  subscribe PostApproval
  subscribe PostMessage
  subscribe WardenEscalation

  on start
    memory.plan_ready_count = 0

  on PlanReady(issue_id: Text, repo: Text, plan: Text, criteria: Text, branch: Text, channel: Text, callback_url: Text, test_cmd: Text)
    memory.last_issue = issue_id
    memory.plan_ready_count = memory.plan_ready_count + 1
    say "[probe] plan_ready issue={issue_id} count={memory.plan_ready_count}"

  on ImplementationApproved(issue_id: Text, repo: Text, plan: Text, criteria: Text, branch: Text, channel: Text, callback_url: Text, test_cmd: Text, decision_by: Text)
    memory.last_issue = issue_id
    say "[probe] approved issue={issue_id} repo={repo} branch={branch} decision_by={decision_by}"

  on ImplementationRejected(issue_id: Text, comment: Text, decision_by: Text)
    say "[probe] rejected issue={issue_id} by={decision_by} comment={comment}"

  on PostApproval(channel: Text, title: Text, context: Text, callback_url: Text, request_id: Text, thread_ts: Text)
    say "[probe] post_approval channel={channel} request_id={request_id} title={title}"

  on PostMessage(channel: Text, text: Text, thread_ts: Text)
    say "[probe] post_message channel={channel} text={text}"

  on WardenEscalation(agent_id: Text, cause: Text, detail: Text, channel: Text)
    say "[probe] escalation agent={agent_id} cause={cause} detail={detail}"

system test_system
  use
    plan: planner
    gate2: gate_two
    probe: gate_probe
"#;

fn read_to_program(path: &str) -> (String, forge::ast::Program) {
    let src =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("could not read {path}: {e}"));
    let prog = forge::parser::parse(&src).unwrap_or_else(|e| panic!("parse {path}: {e:?}"));
    (src, prog)
}

fn build_program(harness_src: &'static str, with_planner: bool) -> forge::ast::Program {
    let (gate_src, gate_prog) = read_to_program(GATE_TWO_PATH);
    let (types_src, types_prog) = read_to_program(TYPES_PATH);
    let (agents_src, agents_prog) = read_to_program(DEV_CYCLE_AGENTS_PATH);
    let harness_prog = forge::parser::parse(harness_src).expect("parse harness");

    let mut files = vec![
        compose::SourceFile {
            path: TYPES_PATH.to_string(),
            source: types_src,
            program: types_prog,
        },
        compose::SourceFile {
            path: DEV_CYCLE_AGENTS_PATH.to_string(),
            source: agents_src,
            program: agents_prog,
        },
        compose::SourceFile {
            path: GATE_TWO_PATH.to_string(),
            source: gate_src,
            program: gate_prog,
        },
    ];
    let _ = with_planner; // both shapes need agents.forge for events; the
                          // harness controls which agents the system uses.
    files.push(compose::SourceFile {
        path: "test_harness.forge".to_string(),
        source: harness_src.to_string(),
        program: harness_prog,
    });
    let composed = compose::merge_programs(&files).expect("merge");
    composed.program
}

fn mock_registry() -> Arc<forge::llm::registry::ProviderRegistry> {
    let config = forge::config::ForgeConfig::default_mock_config();
    Arc::new(
        forge::llm::registry::ProviderRegistry::from_config(config)
            .expect("mock registry should build"),
    )
}

/// Write a clone-dev.toml fixture with the given gate-2 settings and
/// point FORGE_CLONEDEV_CONFIG at it. Returns the path so the caller
/// can keep it alive (the config loader canonicalizes and caches).
fn write_fixture_config(
    test_name: &str,
    start_implementation: bool,
    timeout_mins: u32,
    approval_channel: &str,
    max_plan_revisions: u32,
) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "forge-gate-two-{}-{}",
        std::process::id(),
        test_name
    ));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let path = dir.join("clone-dev.toml");
    let toml = format!(
        r#"
[org]
name = "test-org"

[slack]
default_channel  = "C-default"
approval_channel = "{approval_channel}"

[github]
default_repo = "test-org/test-repo"

[gates]
create_issue                      = true
create_issue_timeout_mins         = 30
start_implementation              = {start_implementation}
start_implementation_timeout_mins = {timeout_mins}

[defaults]
max_plan_revisions = {max_plan_revisions}

[llm.routing]
classify = "mock"
plan     = "mock"
"#
    );
    std::fs::write(&path, toml).expect("write fixture toml");
    std::env::set_var("FORGE_CLONEDEV_CONFIG", &path);
    path
}

struct Harness {
    event_bus: forge::runtime::event_bus::SharedEventBus,
    rx: tokio::sync::broadcast::Receiver<String>,
    _tracer: forge::tracer::Tracer,
}

async fn boot(harness_src: &'static str, with_planner: bool) -> Harness {
    let program = build_program(harness_src, with_planner);

    let registry = SkillRegistry::new();
    let shared_registry = Arc::new(Mutex::new(registry));

    let (events_tx, events_rx) = tokio::sync::broadcast::channel::<String>(1024);
    let tracer = forge::tracer::Tracer::with_live(events_tx.clone());

    let providers = mock_registry();
    let skill_executor = Arc::new(
        SkillExecutor::new(providers.clone(), shared_registry)
            .with_tracer(Arc::new(tracer.clone())),
    );

    let executor = TaskExecutor::new(program, providers, Some(tracer.clone()))
        .with_config(forge::config::ForgeConfig::default_mock_config())
        .with_skill_executor(skill_executor);

    let event_bus = EventBus::new_shared(executor.tracer().cloned());
    let instance_registry = Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));

    let system_runtime = executor
        .build_system_runtime()
        .expect("build system runtime")
        .expect("test_system should produce a runtime")
        .with_shared_infrastructure(event_bus.clone(), instance_registry.clone());

    tokio::spawn(async move {
        let _ = system_runtime.start().await;
    });

    // `on start` runs config.load_clone_dev — give it more headroom
    // than the issue_creator harness because we hit the disk.
    tokio::time::sleep(Duration::from_millis(600)).await;

    Harness {
        event_bus,
        rx: events_rx,
        _tracer: tracer,
    }
}

fn cv_text(s: &str) -> ConfidentValue {
    ConfidentValue::deterministic(Value::Text(s.into()))
}

fn cv_bool(b: bool) -> ConfidentValue {
    ConfidentValue::deterministic(Value::Bool(b))
}

async fn fire_plan_ready(h: &Harness, issue_id: &str, repo: &str, branch: &str, channel: &str) {
    let mut fields = HashMap::new();
    fields.insert("issue_id".to_string(), cv_text(issue_id));
    fields.insert("repo".to_string(), cv_text(repo));
    fields.insert(
        "plan".to_string(),
        cv_text("1. Do thing.\n2. Test thing.\n3. Ship thing."),
    );
    fields.insert(
        "criteria".to_string(),
        cv_text("- thing must work\n- tests must pass"),
    );
    fields.insert("branch".to_string(), cv_text(branch));
    fields.insert("channel".to_string(), cv_text(channel));
    fields.insert(
        "callback_url".to_string(),
        cv_text("http://localhost:3300/webhook/approval"),
    );
    fields.insert("test_cmd".to_string(), cv_text("cargo test --quiet"));

    let payload = EventPayload {
        event_name: "PlanReady".to_string(),
        args: vec![],
        source_agent: "test_driver".to_string(),
        fields,
    };
    let bus = h.event_bus.read().await;
    bus.publish(&payload);
}

async fn fire_issue_assigned(h: &Harness, issue_id: &str, repo: &str, title: &str, channel: &str) {
    let mut fields = HashMap::new();
    fields.insert("issue_id".to_string(), cv_text(issue_id));
    fields.insert("repo".to_string(), cv_text(repo));
    fields.insert("title".to_string(), cv_text(title));
    fields.insert(
        "body".to_string(),
        cv_text("Acceptance: it works and has tests."),
    );
    fields.insert("channel".to_string(), cv_text(channel));
    fields.insert(
        "callback_url".to_string(),
        cv_text("http://localhost:3300/webhook/approval"),
    );
    fields.insert("test_cmd".to_string(), cv_text("cargo test --quiet"));

    let payload = EventPayload {
        event_name: "IssueAssigned".to_string(),
        args: vec![],
        source_agent: "test_driver".to_string(),
        fields,
    };
    let bus = h.event_bus.read().await;
    bus.publish(&payload);
}

async fn fire_approval_response(h: &Harness, request_id: &str, approved: bool, comment: &str) {
    let mut fields = HashMap::new();
    fields.insert("request_id".to_string(), cv_text(request_id));
    fields.insert("approved".to_string(), cv_bool(approved));
    fields.insert("comment".to_string(), cv_text(comment));

    let payload = EventPayload {
        event_name: "ApprovalResponse".to_string(),
        args: vec![],
        source_agent: "test_driver".to_string(),
        fields,
    };
    let bus = h.event_bus.read().await;
    bus.publish(&payload);
}

async fn drain_frames(
    rx: &mut tokio::sync::broadcast::Receiver<String>,
    window: Duration,
) -> Vec<serde_json::Value> {
    let mut frames = Vec::<String>::new();
    let deadline = std::time::Instant::now() + window;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Ok(frame)) => frames.push(frame),
            Ok(Err(_)) | Err(_) => break,
        }
    }
    frames
        .iter()
        .filter_map(|f| serde_json::from_str(f).ok())
        .collect()
}

fn say_lines(frames: &[serde_json::Value]) -> Vec<String> {
    frames
        .iter()
        .filter(|v| v["event"] == "say")
        .filter_map(|v| v["text"].as_str())
        .map(String::from)
        .collect()
}

fn count_emits(frames: &[serde_json::Value], event: &str, source: &str) -> usize {
    frames
        .iter()
        .filter(|v| v["event"] == event && v["source_agent"] == source)
        .count()
}

// ── Tests ─────────────────────────────────────────────────────────────

// DoD: "between PlanReady and implementer activation, slack_adapter
// sends approval ... Approve → emits ImplementationApproved
// (subscribed by implementer)". Happy path.
#[tokio::test]
async fn plan_ready_approval_emits_post_approval_then_implementation_approved() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _fixture = write_fixture_config("happy", true, 30, "", 3);

    let mut h = boot(TEST_HARNESS_BASE, false).await;
    fire_plan_ready(
        &h,
        "ISSUE-123",
        "test-org/test-repo",
        "clone-dev/123",
        "C-issue",
    )
    .await;

    let frames_before = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let says_before = say_lines(&frames_before);

    let post_approvals = count_emits(&frames_before, "PostApproval", "gate_two");
    assert_eq!(
        post_approvals, 1,
        "exactly one PostApproval expected; says={says_before:?}"
    );
    let probe_card = says_before
        .iter()
        .find(|s| s.starts_with("[probe] post_approval"))
        .unwrap_or_else(|| panic!("probe should observe PostApproval; says={says_before:?}"));
    assert!(
        probe_card.contains("request_id=plan-ISSUE-123"),
        "request_id must be 'plan-{{issue_id}}'; got: {probe_card}"
    );
    assert!(
        probe_card.contains("title=Start implementation: ISSUE-123"),
        "PostApproval title must lead with 'Start implementation:'; got: {probe_card}"
    );

    assert_eq!(
        count_emits(&frames_before, "ImplementationApproved", "gate_two"),
        0,
        "no ImplementationApproved before approval response; says={says_before:?}"
    );

    // Simulate the human clicking Approve.
    fire_approval_response(&h, "plan-ISSUE-123", true, "alice").await;

    let frames_after = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let says_after = say_lines(&frames_after);

    let approved = count_emits(&frames_after, "ImplementationApproved", "gate_two");
    assert_eq!(
        approved, 1,
        "exactly one ImplementationApproved after approval; says={says_after:?}"
    );

    let probe_approved = says_after
        .iter()
        .find(|s| s.starts_with("[probe] approved"))
        .unwrap_or_else(|| panic!("probe should observe approved; says={says_after:?}"));
    assert!(
        probe_approved.contains("issue=ISSUE-123"),
        "ImplementationApproved must carry issue_id; got: {probe_approved}"
    );
    assert!(
        probe_approved.contains("repo=test-org/test-repo"),
        "ImplementationApproved must carry repo; got: {probe_approved}"
    );
    assert!(
        probe_approved.contains("decision_by=alice"),
        "decision_by must name approver; got: {probe_approved}"
    );

    let confirm = says_after
        .iter()
        .find(|s| s.starts_with("[probe] post_message") && s.contains(":white_check_mark:"))
        .unwrap_or_else(|| panic!("expected :white_check_mark: confirmation; says={says_after:?}"));
    assert!(
        confirm.contains("Implementation approved"),
        "confirmation must mention approval; got: {confirm}"
    );
}

// DoD: "ApprovalResponse ... → emits ... ImplementationRejected (back
// to planner for revision)". Verifies the rejection event shape; the
// planner-feedback loop is exercised in the revision case below.
#[tokio::test]
async fn plan_ready_rejection_emits_implementation_rejected() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _fixture = write_fixture_config("rejection", true, 30, "", 3);

    let mut h = boot(TEST_HARNESS_BASE, false).await;
    fire_plan_ready(
        &h,
        "ISSUE-456",
        "test-org/test-repo",
        "clone-dev/456",
        "C-issue",
    )
    .await;

    let _ = drain_frames(&mut h.rx, HANDLER_WINDOW).await;

    fire_approval_response(&h, "plan-ISSUE-456", false, "bob").await;

    let frames = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let says = say_lines(&frames);

    assert_eq!(
        count_emits(&frames, "ImplementationRejected", "gate_two"),
        1,
        "exactly one ImplementationRejected expected; says={says:?}"
    );
    assert_eq!(
        count_emits(&frames, "ImplementationApproved", "gate_two"),
        0,
        "no ImplementationApproved on rejection; says={says:?}"
    );

    let probe_reject = says
        .iter()
        .find(|s| s.starts_with("[probe] rejected"))
        .unwrap_or_else(|| panic!("probe should observe rejected; says={says:?}"));
    assert!(
        probe_reject.contains("issue=ISSUE-456"),
        "rejection must carry issue_id; got: {probe_reject}"
    );
    assert!(
        probe_reject.contains("by=bob"),
        "decision_by must name rejecter; got: {probe_reject}"
    );

    let rejection_msg = says
        .iter()
        .find(|s| s.contains(":x:"))
        .unwrap_or_else(|| panic!("expected :x: rejection notice; says={says:?}"));
    assert!(
        rejection_msg.contains("Plan rejected"),
        "rejection notice must mention 'Plan rejected'; got: {rejection_msg}"
    );
}

// DoD: "[gates] start_implementation = false bypass: planner
// auto-emits ImplementationApproved with decision_by: 'auto (policy)'".
#[tokio::test]
async fn auto_approve_bypass_skips_post_approval() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _fixture = write_fixture_config("autoapprove", false, 30, "", 3);

    let mut h = boot(TEST_HARNESS_BASE, false).await;
    fire_plan_ready(
        &h,
        "ISSUE-789",
        "test-org/test-repo",
        "clone-dev/789",
        "C-issue",
    )
    .await;

    let frames = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let says = say_lines(&frames);

    assert_eq!(
        count_emits(&frames, "PostApproval", "gate_two"),
        0,
        "auto-approve must not emit PostApproval; says={says:?}"
    );
    assert_eq!(
        count_emits(&frames, "ImplementationApproved", "gate_two"),
        1,
        "auto-approve should emit ImplementationApproved immediately; says={says:?}"
    );

    let probe_approved = says
        .iter()
        .find(|s| s.starts_with("[probe] approved"))
        .unwrap_or_else(|| panic!("probe should observe approved; says={says:?}"));
    assert!(
        probe_approved.contains("decision_by=auto (policy)"),
        "auto-approve must set decision_by to 'auto (policy)'; got: {probe_approved}"
    );

    assert!(
        says.iter().any(|s| s.contains("auto-approving")),
        "auto-approve should log the policy decision; says={says:?}"
    );
}

// DoD: "Revision path: on reject with comment → planner subscribes
// ImplementationRejected, re-plans with the rejection comment as
// additional context, emits new PlanReady (bounded by [defaults]
// max_plan_revisions)."
//
// Drives the full planner → gate_two → reject → planner re-emit
// loop. The mock LLM returns a deterministic string, so each revision
// produces a fresh PlanReady with the same plan body — what we
// actually test is the loop-and-bound behaviour, not the LLM quality
// (the real-LLM quality assertion lives in the live test).
#[tokio::test]
async fn revision_path_planner_re_emits_plan_ready_until_bound() {
    let _guard = ENV_LOCK.lock().unwrap();
    // max_plan_revisions = 2 → 3rd rejection should NOT produce a
    // fresh PlanReady; the planner escalates instead.
    let _fixture = write_fixture_config("revision", true, 30, "", 2);

    let mut h = boot(TEST_HARNESS_WITH_PLANNER, true).await;

    // Kick off the planner.
    fire_issue_assigned(
        &h,
        "ISSUE-rev",
        "test-org/test-repo",
        "Revision feedback test",
        "C-issue",
    )
    .await;

    // First PlanReady from the planner; gate_two posts an approval card.
    let _ = drain_frames(&mut h.rx, HANDLER_WINDOW).await;

    // ── Revision 1 ────────────────────────────────────────────
    fire_approval_response(&h, "plan-ISSUE-rev", false, "carol").await;
    let frames_r1 = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let says_r1 = say_lines(&frames_r1);
    let plan_ready_r1 = says_r1
        .iter()
        .filter(|s| s.starts_with("[probe] plan_ready"))
        .count();
    assert!(
        plan_ready_r1 >= 1,
        "planner must re-emit PlanReady after first rejection; says={says_r1:?}"
    );

    // ── Revision 2 ────────────────────────────────────────────
    fire_approval_response(&h, "plan-ISSUE-rev", false, "carol").await;
    let frames_r2 = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let says_r2 = say_lines(&frames_r2);
    let plan_ready_r2 = says_r2
        .iter()
        .filter(|s| s.starts_with("[probe] plan_ready"))
        .count();
    assert!(
        plan_ready_r2 >= 1,
        "planner must re-emit PlanReady after second rejection (still within bound); says={says_r2:?}"
    );

    // ── Revision 3 (over the bound) ───────────────────────────
    fire_approval_response(&h, "plan-ISSUE-rev", false, "carol").await;
    let frames_r3 = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let says_r3 = say_lines(&frames_r3);
    let plan_ready_r3 = says_r3
        .iter()
        .filter(|s| s.starts_with("[probe] plan_ready"))
        .count();
    assert_eq!(
        plan_ready_r3, 0,
        "planner must NOT emit PlanReady after exceeding max_plan_revisions=2; says={says_r3:?}"
    );

    // Audit `say` should mention the escalation.
    assert!(
        says_r3
            .iter()
            .any(|s| s.contains("Max plan revisions") || s.contains("Escalating")),
        "planner should log escalation when bound is exceeded; says={says_r3:?}"
    );
}

// Stale ApprovalResponse with the wrong request_id must be ignored —
// the gate's `requires request_id == \"plan-{{memory.issue_id}}\"`
// guard is what protects against cross-issue collisions.
#[tokio::test]
async fn stale_approval_response_ignored() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _fixture = write_fixture_config("stale", true, 30, "", 3);

    let mut h = boot(TEST_HARNESS_BASE, false).await;
    fire_plan_ready(
        &h,
        "ISSUE-real",
        "test-org/test-repo",
        "clone-dev/real",
        "C-issue",
    )
    .await;
    let _ = drain_frames(&mut h.rx, HANDLER_WINDOW).await;

    // Wrong request_id — must be ignored.
    fire_approval_response(&h, "plan-ISSUE-other", true, "mallory").await;

    let frames = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let says = say_lines(&frames);

    assert_eq!(
        count_emits(&frames, "ImplementationApproved", "gate_two"),
        0,
        "stale approval must not produce ImplementationApproved; says={says:?}"
    );
    assert_eq!(
        count_emits(&frames, "ImplementationRejected", "gate_two"),
        0,
        "stale approval must not produce ImplementationRejected; says={says:?}"
    );
}

// DoD: "send approval to config.slack.approval_channel". When the
// config has a non-empty approval_channel, the PostApproval and the
// confirmation PostMessage both route there.
#[tokio::test]
async fn approval_channel_overrides_per_issue_channel() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _fixture = write_fixture_config("channel", true, 30, "C-reviewers", 3);

    let mut h = boot(TEST_HARNESS_BASE, false).await;
    fire_plan_ready(
        &h,
        "ISSUE-ch",
        "test-org/test-repo",
        "clone-dev/ch",
        "C-issue",
    )
    .await;

    let frames_before = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let says_before = say_lines(&frames_before);
    let probe_card = says_before
        .iter()
        .find(|s| s.starts_with("[probe] post_approval"))
        .unwrap_or_else(|| panic!("probe should observe PostApproval; says={says_before:?}"));
    assert!(
        probe_card.contains("channel=C-reviewers"),
        "PostApproval must route to slack_approval_channel; got: {probe_card}"
    );

    fire_approval_response(&h, "plan-ISSUE-ch", true, "dave").await;
    let frames_after = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let says_after = say_lines(&frames_after);

    let confirm = says_after
        .iter()
        .find(|s| s.starts_with("[probe] post_message") && s.contains(":white_check_mark:"))
        .unwrap_or_else(|| panic!("expected :white_check_mark: confirmation; says={says_after:?}"));
    assert!(
        confirm.contains("channel=C-reviewers"),
        "confirmation must also route to slack_approval_channel; got: {confirm}"
    );
}
