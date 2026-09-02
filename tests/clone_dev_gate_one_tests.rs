// Integration tests for T10.1 (#367) — gate_one agent under
// workflows/clone-dev/stage1/gate_one.forge.
//
// Verifies the Stage-1 → Gate-1 → T9.4 hand-off slice:
//
//   DevOpsRequest seeds memory.channel via correlate-on-thread_ts.
//   ProposalReady forks by kind:
//     answer        → PostMessage(content) into the originating thread.
//     small_action  → PostMessage stub + SmallActionApproved placeholder.
//     propose_issue → PostApproval to slack_adapter, transition to
//                     waiting_approval, await ApprovalResponse.
//                     If gates_create_issue == false, auto-approve
//                     immediately without going through Slack.
//   ApprovalResponse(approved=true)  → ProposalApproved + :white_check_mark:.
//   ApprovalResponse(approved=false) → ProposalRejected + :x:.
//
// Cases:
//   1. propose_issue happy path  — DevOpsRequest + ProposalReady →
//      PostApproval emitted; injecting ApprovalResponse(approved=true)
//      yields ProposalApproved with the derived title/body and a
//      :white_check_mark: PostMessage.
//   2. propose_issue rejection   — same as #1 but approved=false →
//      ProposalRejected (rejection_reason="human") + :x: PostMessage,
//      no ProposalApproved.
//   3. auto-approve bypass       — fixture sets gates_create_issue=false
//      → ProposalApproved emitted immediately, no PostApproval.
//   4. answer kind               — ProposalReady(kind=answer) →
//      PostMessage(content) only; no PostApproval, no ProposalApproved.
//   5. small_action kind         — ProposalReady(kind=small_action) →
//      PostMessage stub + SmallActionApproved; no PostApproval.
//
// The 5-minute approval_timeout escalation path is exercised in the
// real-LLM smoke run rather than here — the test harness has no
// deterministic way to fast-forward FORGE timer ticks.
//
// Env-lock pattern: each test writes a fixture clone-dev.toml at a
// unique path under /tmp and sets FORGE_CLONEDEV_CONFIG. The static
// ENV_LOCK serializes tests so they don't stomp on the global env
// var; the unique paths sidestep the process-wide config cache that
// keys by canonicalized path (src/runtime/clone_dev_config.rs:672-693).
// ──────────────────────────────────────────────────────────────────────

// std::sync::Mutex is the right tool here even in async context: we
// only need it to serialize FORGE_CLONEDEV_CONFIG mutations across
// parallel cargo test threads. The lock is intentionally held across
// awaits so a second test can't stomp the env var while the runtime
// still owns it (the loader canonicalizes the path on first read).
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

const GATE_ONE_PATH: &str = "workflows/clone-dev/stage1/gate_one.forge";

// Window for `on start` (config load) + handler dispatch + emit
// propagation. Generous because config.load_clone_dev parses TOML
// from disk on first hit.
const HANDLER_WINDOW: Duration = Duration::from_millis(2500);

// Process-wide guard so parallel cargo test invocations don't race on
// FORGE_CLONEDEV_CONFIG. Tests serialize on this; the unique fixture
// path per test still avoids the config cache colliding (#357).
static ENV_LOCK: Mutex<()> = Mutex::new(());

const TEST_HARNESS_SRC: &str = r#"#! boundary: server

event DevOpsRequest
  channel: Text
  user: Text
  text: Text
  thread_ts: Text
  message_ts: Text

event ProposalReady
  thread_ts: Text
  kind: Text
  content: Text
  suggested_labels: Text[]
  evidence_refs: Text[]
  confidence: Number

event ProposalApproved
  thread_ts: Text
  channel: Text
  kind: Text
  repo: Text
  title: Text
  body: Text
  suggested_labels: Text[]
  evidence_refs: Text[]

event ProposalRejected
  thread_ts: Text
  channel: Text
  kind: Text
  rejection_reason: Text
  decision_by: Text
  comment: Text

event SmallActionApproved
  thread_ts: Text
  channel: Text
  content: Text
  evidence_refs: Text[]

event ApprovalResponse
  request_id: Text
  approved: Bool
  comment: Text

event PostApproval
  channel: Text
  title: Text
  context: Text
  callback_url: Text
  request_id: Text
  thread_ts: Text

event PostMessage
  channel: Text
  text: Text
  thread_ts: Text

event WardenEscalation
  agent_id: Text
  cause: Text
  detail: Text
  channel: Text

warden investigators_ward
  manages [gate_one, gate_probe]
  on stuck: escalate, self
  on crash: restart, self
  on timeout: restart, self

agent gate_probe
  memory
    last_thread: Text
  subscribe ProposalApproved
  subscribe ProposalRejected
  subscribe SmallActionApproved
  subscribe PostApproval
  subscribe PostMessage
  subscribe WardenEscalation

  on ProposalApproved(thread_ts: Text, channel: Text, kind: Text, repo: Text, title: Text, body: Text, suggested_labels: Text[], evidence_refs: Text[])
    memory.last_thread = thread_ts
    say "[probe] approved thread={thread_ts} kind={kind} repo={repo} title={title} labels_count={suggested_labels.length}"

  on ProposalRejected(thread_ts: Text, channel: Text, kind: Text, rejection_reason: Text, decision_by: Text, comment: Text)
    say "[probe] rejected thread={thread_ts} kind={kind} reason={rejection_reason} by={decision_by}"

  on SmallActionApproved(thread_ts: Text, channel: Text, content: Text, evidence_refs: Text[])
    say "[probe] small_action thread={thread_ts} content={content}"

  on PostApproval(channel: Text, title: Text, context: Text, callback_url: Text, request_id: Text, thread_ts: Text)
    say "[probe] post_approval channel={channel} thread={thread_ts} request_id={request_id} title={title}"

  on PostMessage(channel: Text, text: Text, thread_ts: Text)
    say "[probe] post_message channel={channel} thread={thread_ts} text={text}"

  on WardenEscalation(agent_id: Text, cause: Text, detail: Text, channel: Text)
    say "[probe] escalation agent={agent_id} cause={cause} detail={detail}"

system test_system
  use
    gate: gate_one
    probe: gate_probe
"#;

fn read_to_program(path: &str) -> (String, forge::ast::Program) {
    let src =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("could not read {path}: {e}"));
    let prog = forge::parser::parse(&src).unwrap_or_else(|e| panic!("parse {path}: {e:?}"));
    (src, prog)
}

fn build_program() -> forge::ast::Program {
    let (gate_src, gate_prog) = read_to_program(GATE_ONE_PATH);
    let (types_src, types_prog) = read_to_program("workflows/clone-dev/shared/types.forge");
    let harness_prog = forge::parser::parse(TEST_HARNESS_SRC).expect("parse harness");

    let files = vec![
        compose::SourceFile {
            path: "workflows/clone-dev/shared/types.forge".to_string(),
            source: types_src,
            program: types_prog,
        },
        compose::SourceFile {
            path: GATE_ONE_PATH.to_string(),
            source: gate_src,
            program: gate_prog,
        },
        compose::SourceFile {
            path: "test_harness.forge".to_string(),
            source: TEST_HARNESS_SRC.to_string(),
            program: harness_prog,
        },
    ];
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

/// Write a clone-dev.toml fixture with the given gate-1 settings and
/// point FORGE_CLONEDEV_CONFIG at it. Returns the path so the caller
/// can keep it alive (the config loader canonicalizes and caches).
fn write_fixture_config(test_name: &str, create_issue: bool, timeout_mins: u32) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "forge-gate-one-{}-{}",
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
default_channel = "C-default"

[github]
default_repo = "test-org/test-repo"

[gates]
create_issue              = {create_issue}
create_issue_timeout_mins = {timeout_mins}

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

async fn boot() -> Harness {
    let program = build_program();

    // Empty registry — gate_one calls no skills directly. PostApproval
    // / PostMessage / WardenEscalation are bus events the harness
    // gate_probe re-emits; the real slack_adapter doesn't need to be
    // present for these tests.
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

fn cv_number(n: f64) -> ConfidentValue {
    ConfidentValue::deterministic(Value::Number(n))
}

fn cv_text_array(items: &[&str]) -> ConfidentValue {
    ConfidentValue::deterministic(Value::Array(
        items
            .iter()
            .map(|s| ConfidentValue::deterministic(Value::Text((*s).into())))
            .collect(),
    ))
}

async fn fire_devops_request(h: &Harness, channel: &str, thread_ts: &str) {
    let mut fields = HashMap::new();
    fields.insert("channel".to_string(), cv_text(channel));
    fields.insert("user".to_string(), cv_text("U-test"));
    fields.insert("text".to_string(), cv_text("seed channel"));
    fields.insert("thread_ts".to_string(), cv_text(thread_ts));
    fields.insert("message_ts".to_string(), cv_text(thread_ts));

    let payload = EventPayload {
        event_name: "DevOpsRequest".to_string(),
        args: vec![],
        source_agent: "test_driver".to_string(),
        fields,
    };
    let bus = h.event_bus.read().await;
    bus.publish(&payload);
}

async fn fire_proposal_ready(
    h: &Harness,
    thread_ts: &str,
    kind: &str,
    content: &str,
    suggested_labels: &[&str],
    evidence_refs: &[&str],
) {
    let mut fields = HashMap::new();
    fields.insert("thread_ts".to_string(), cv_text(thread_ts));
    fields.insert("kind".to_string(), cv_text(kind));
    fields.insert("content".to_string(), cv_text(content));
    fields.insert(
        "suggested_labels".to_string(),
        cv_text_array(suggested_labels),
    );
    fields.insert("evidence_refs".to_string(), cv_text_array(evidence_refs));
    fields.insert("confidence".to_string(), cv_number(0.9));

    let payload = EventPayload {
        event_name: "ProposalReady".to_string(),
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

// DoD: "On approval: fork by kind — propose_issue triggers T9.4". The
// happy path drives DevOpsRequest, ProposalReady(propose_issue),
// ApprovalResponse(approved=true) and asserts PostApproval (gate sent
// the card), then ProposalApproved (issue_creator hand-off ready) +
// :white_check_mark: confirmation.
#[tokio::test]
async fn propose_issue_approval_emits_post_approval_then_proposal_approved() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _fixture = write_fixture_config("happy", true, 30);

    let mut h = boot().await;
    fire_devops_request(&h, "C-devops", "T-happy").await;
    fire_proposal_ready(
        &h,
        "T-happy",
        "propose_issue",
        "API gateway latency spikes correlate with retry budget regression. Stage-2 should investigate.",
        &["clone-dev:plan"],
        &["ops: latency p95 over budget"],
    )
    .await;

    let frames_before = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let says_before = say_lines(&frames_before);

    // Gate emitted PostApproval before any approval response was sent.
    let post_approvals = count_emits(&frames_before, "PostApproval", "gate_one");
    assert_eq!(
        post_approvals, 1,
        "exactly one PostApproval expected; says={says_before:?}"
    );
    let probe_card = says_before
        .iter()
        .find(|s| s.starts_with("[probe] post_approval"))
        .unwrap_or_else(|| panic!("probe should observe PostApproval; says={says_before:?}"));
    assert!(
        probe_card.contains("request_id=T-happy"),
        "request_id must equal thread_ts; got: {probe_card}"
    );
    assert!(
        probe_card.contains("title=Create issue:"),
        "PostApproval title must lead with 'Create issue:'; got: {probe_card}"
    );

    // Gate has not yet emitted ProposalApproved (waiting on human).
    assert_eq!(
        count_emits(&frames_before, "ProposalApproved", "gate_one"),
        0,
        "no ProposalApproved before approval response; says={says_before:?}"
    );

    // Now simulate the human clicking Approve.
    fire_approval_response(&h, "T-happy", true, "alice").await;

    let frames_after = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let says_after = say_lines(&frames_after);

    let approved = count_emits(&frames_after, "ProposalApproved", "gate_one");
    assert_eq!(
        approved, 1,
        "exactly one ProposalApproved after approval; says={says_after:?}"
    );

    let probe_approved = says_after
        .iter()
        .find(|s| s.starts_with("[probe] approved"))
        .unwrap_or_else(|| panic!("probe should observe approved; says={says_after:?}"));
    assert!(
        probe_approved.contains("repo=test-org/test-repo"),
        "ProposalApproved must carry fixture repo; got: {probe_approved}"
    );
    assert!(
        probe_approved.contains("kind=propose_issue"),
        "kind must be propose_issue; got: {probe_approved}"
    );
    assert!(
        probe_approved.contains("labels_count=1"),
        "labels_count should be 1 (clone-dev:plan); got: {probe_approved}"
    );

    // Confirmation reply lands in the same Slack thread.
    let post_msgs: Vec<&String> = says_after
        .iter()
        .filter(|s| s.starts_with("[probe] post_message"))
        .collect();
    let confirm = post_msgs
        .iter()
        .find(|s| s.contains(":white_check_mark:"))
        .unwrap_or_else(|| panic!("expected :white_check_mark: confirmation; says={says_after:?}"));
    assert!(
        confirm.contains("thread=T-happy"),
        "confirmation must target T-happy thread; got: {confirm}"
    );
    assert!(
        confirm.contains("Approved by alice"),
        "confirmation must name approver; got: {confirm}"
    );
}

// DoD: "On approval ... ProposalRejected on reject path".
#[tokio::test]
async fn propose_issue_rejection_emits_proposal_rejected() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _fixture = write_fixture_config("rejection", true, 30);

    let mut h = boot().await;
    fire_devops_request(&h, "C-devops", "T-reject").await;
    fire_proposal_ready(
        &h,
        "T-reject",
        "propose_issue",
        "Should be rejected.",
        &["clone-dev:ops"],
        &["ops: synthetic"],
    )
    .await;

    let _ = drain_frames(&mut h.rx, HANDLER_WINDOW).await;

    fire_approval_response(&h, "T-reject", false, "bob").await;

    let frames = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let says = say_lines(&frames);

    assert_eq!(
        count_emits(&frames, "ProposalRejected", "gate_one"),
        1,
        "exactly one ProposalRejected expected; says={says:?}"
    );
    assert_eq!(
        count_emits(&frames, "ProposalApproved", "gate_one"),
        0,
        "no ProposalApproved on rejection; says={says:?}"
    );

    let probe_reject = says
        .iter()
        .find(|s| s.starts_with("[probe] rejected"))
        .unwrap_or_else(|| panic!("probe should observe rejected; says={says:?}"));
    assert!(
        probe_reject.contains("reason=human"),
        "rejection_reason must be 'human'; got: {probe_reject}"
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
        rejection_msg.contains("thread=T-reject"),
        "rejection notice must target the originating thread; got: {rejection_msg}"
    );
}

// DoD: "[gates] create_issue = false bypass: auto-approves with
// decision_by: 'auto (policy)' for configured orgs".
#[tokio::test]
async fn auto_approve_bypass_skips_post_approval() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _fixture = write_fixture_config("autoapprove", false, 30);

    let mut h = boot().await;
    fire_devops_request(&h, "C-devops", "T-auto").await;
    fire_proposal_ready(
        &h,
        "T-auto",
        "propose_issue",
        "Auto-approved policy bypass.",
        &["clone-dev:plan"],
        &["ops: synthetic"],
    )
    .await;

    let frames = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let says = say_lines(&frames);

    assert_eq!(
        count_emits(&frames, "PostApproval", "gate_one"),
        0,
        "auto-approve must not emit PostApproval; says={says:?}"
    );
    assert_eq!(
        count_emits(&frames, "ProposalApproved", "gate_one"),
        1,
        "auto-approve should emit ProposalApproved immediately; says={says:?}"
    );

    let probe_approved = says
        .iter()
        .find(|s| s.starts_with("[probe] approved"))
        .unwrap_or_else(|| panic!("probe should observe approved; says={says:?}"));
    assert!(
        probe_approved.contains("repo=test-org/test-repo"),
        "auto-approve must still derive repo from config; got: {probe_approved}"
    );

    // Audit `say` should mention the auto-policy path.
    assert!(
        says.iter().any(|s| s.contains("auto-approving")),
        "auto-approve should log the policy decision; says={says:?}"
    );
}

// DoD: "On approval: fork by kind — answer posts the reply to the
// originating thread". Per the issue's design, `answer` is an
// information-only proposal kind that bypasses the approval card.
#[tokio::test]
async fn answer_kind_posts_directly_no_approval() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _fixture = write_fixture_config("answer", true, 30);

    let mut h = boot().await;
    fire_devops_request(&h, "C-devops", "T-answer").await;
    fire_proposal_ready(
        &h,
        "T-answer",
        "answer",
        "The pipeline timeout is configured at 10 minutes.",
        &[],
        &["ops: pipeline.yaml line 47"],
    )
    .await;

    let frames = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let says = say_lines(&frames);

    assert_eq!(
        count_emits(&frames, "PostApproval", "gate_one"),
        0,
        "answer kind must not emit PostApproval; says={says:?}"
    );
    assert_eq!(
        count_emits(&frames, "ProposalApproved", "gate_one"),
        0,
        "answer kind must not emit ProposalApproved; says={says:?}"
    );

    let direct = says
        .iter()
        .find(|s| {
            s.starts_with("[probe] post_message")
                && s.contains("text=The pipeline timeout is configured at 10 minutes.")
        })
        .unwrap_or_else(|| panic!("expected direct PostMessage with content; says={says:?}"));
    assert!(
        direct.contains("thread=T-answer"),
        "answer reply must target the originating thread; got: {direct}"
    );
}

// DoD-adjacent: small_action stub posts a confirmation and emits a
// SmallActionApproved placeholder for the future action runner.
#[tokio::test]
async fn small_action_kind_posts_stub_and_emits_placeholder() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _fixture = write_fixture_config("smallaction", true, 30);

    let mut h = boot().await;
    fire_devops_request(&h, "C-devops", "T-action").await;
    fire_proposal_ready(
        &h,
        "T-action",
        "small_action",
        "Flip the canary flag back to false.",
        &[],
        &["ops: deploy.yaml"],
    )
    .await;

    let frames = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let says = say_lines(&frames);

    assert_eq!(
        count_emits(&frames, "PostApproval", "gate_one"),
        0,
        "small_action must not emit PostApproval; says={says:?}"
    );
    assert_eq!(
        count_emits(&frames, "ProposalApproved", "gate_one"),
        0,
        "small_action must not emit ProposalApproved; says={says:?}"
    );
    assert_eq!(
        count_emits(&frames, "SmallActionApproved", "gate_one"),
        1,
        "small_action must emit SmallActionApproved placeholder; says={says:?}"
    );

    let stub = says
        .iter()
        .find(|s| s.starts_with("[probe] post_message") && s.contains("[stub] small_action:"))
        .unwrap_or_else(|| panic!("expected stub PostMessage; says={says:?}"));
    assert!(
        stub.contains("thread=T-action"),
        "stub must target the originating thread; got: {stub}"
    );
}
