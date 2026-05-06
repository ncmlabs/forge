// Integration test for T9.3 (#364) — solution_proposer agent under
// workflows/clone-dev/stage1/solution_proposer.forge.
//
// Verifies the aggregate-and-debounce slice of Stage 1:
//   Finding(thread_ts, ...) [×N, same thread] →
//     solution_proposer (correlate on Finding.thread_ts; 2s quiet_window
//                        timer reset on every arrival) →
//       reason for plan (mocked) →
//         emit ProposalReady(thread_ts, kind, content, suggested_labels,
//                            evidence_refs, confidence)
//
// Mirrors tests/clone_dev_investigator_tests.rs for harness wiring
// (compose source files, MockProvider per phase, EventBus +
// InstanceRegistry + system runtime, fire-and-drain SSE frames).
//
// Skips:
//   - HTTP layer (covered by webhook/e2e tests).
//   - mastermind_intake's parallel per-Finding stitch-back (covered by
//     clone_dev_intake_tests.rs).
// ──────────────────────────────────────────────────────────────────────

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use forge::compose;
use forge::llm::providers::mock::MockProvider;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::confidence::{ConfidentValue, Value};
use forge::runtime::event_bus::EventBus;
use forge::runtime::executor::TaskExecutor;
use forge::runtime::instance_registry::InstanceRegistry;

const PROPOSER_PATH: &str = "workflows/clone-dev/stage1/solution_proposer.forge";

// Quiet-window length is 2s in the agent declaration. Tests sleep
// QUIET_WINDOW_PAD past the last fire to ensure the timer has expired
// and the ProposalReady has been emitted before draining SSE frames.
const QUIET_WINDOW_PAD: Duration = Duration::from_millis(2800);

// Test harness: redeclares Finding, ProposalReady, and WardenEscalation
// (the agent's three event surfaces), declares a minimal investigators_ward,
// and adds a `proposal_probe` agent that subscribes to ProposalReady and
// re-emits its fields as `say` lines so the tests can inspect them
// (the runtime's SSE event_emit frame carries `event`, `source_agent`,
// and `subscribers` only — not payload fields). The probe stays
// inside the harness so the proposer's own surface is unchanged in
// production.
const TEST_HARNESS_SRC: &str = r#"#! boundary: server

event Finding
  thread_ts: Text
  domain: Text
  summary: Text
  evidence: Text[]
  confidence: Number
  suggested_action: Text

event ProposalReady
  thread_ts: Text
  kind: Text
  content: Text
  suggested_labels: Text[]
  evidence_refs: Text[]
  confidence: Number

event WardenEscalation
  agent_id: Text
  cause: Text
  detail: Text
  channel: Text

warden investigators_ward
  manages [solution_proposer, proposal_probe]
  on stuck: escalate, self
  on crash: restart, self
  on timeout: restart, self

agent proposal_probe
  memory
    last_thread: Text
  subscribe ProposalReady

  on ProposalReady(thread_ts: Text, kind: Text, content: Text, suggested_labels: Text[], evidence_refs: Text[], confidence: Number)
    memory.last_thread = thread_ts
    say "[probe] thread={thread_ts} kind={kind} labels_count={suggested_labels.length} evidence_count={evidence_refs.length}"
    for lbl in suggested_labels
      say "[probe] thread={thread_ts} label={lbl}"
    for ev in evidence_refs
      say "[probe] thread={thread_ts} evidence={ev}"

system test_system
  use
    proposer: solution_proposer
    probe: proposal_probe

endpoint fire_finding(thread_ts: Text, domain: Text, summary: Text, confidence: Number, suggested_action: Text) -> Text
  emit Finding(thread_ts: thread_ts, domain: domain, summary: summary, evidence: ["ev1"], confidence: confidence, suggested_action: suggested_action)
  give "queued"
"#;

/// Mock provider for the `plan` phase chain. Two canned outputs cover
/// the propose_issue path (labels nonempty) and the answer path (labels
/// empty). Substring matches drive selection — both keys appear in the
/// proposer's draft prompt verbatim.
fn build_proposer_registry(propose_issue: bool) -> Arc<ProviderRegistry> {
    // The proposer's draft prompt starts "Draft a Gate-1 proposal from
    // these Findings:\n[<domain>] action=...". The mock matches on the
    // suggested_action token we baked into each fired Finding so the
    // 'answer' vs 'propose_issue' tests can each get the right canned
    // line back without sharing a registry.
    let mock = if propose_issue {
        MockProvider::new("mock-plan").with_response(
            "Draft a Gate-1 proposal",
            "propose_issue|||The api-gateway latency points at the recent retry budget regression — file an issue.|||clone-dev:plan,clone-dev:impl",
        )
    } else {
        MockProvider::new("mock-plan").with_response(
            "Draft a Gate-1 proposal",
            "answer|||No code change needed; explain the retry budget interaction in-thread.|||",
        )
    }
    .with_default("answer|||fallback|||");

    let mut registry = ProviderRegistry::new("mock-plan");
    registry.register("mock-plan", Arc::new(mock));
    registry.set_phase_chain("plan", vec!["mock-plan".into()]);
    Arc::new(registry)
}

fn read_to_program(path: &str) -> (String, forge::ast::Program) {
    let src =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("could not read {path}: {e}"));
    let prog = forge::parser::parse(&src).unwrap_or_else(|e| panic!("parse {path}: {e:?}"));
    (src, prog)
}

fn build_program() -> forge::ast::Program {
    let (proposer_src, proposer_prog) = read_to_program(PROPOSER_PATH);
    let harness_prog = forge::parser::parse(TEST_HARNESS_SRC).expect("parse harness");

    let files = vec![
        compose::SourceFile {
            path: PROPOSER_PATH.to_string(),
            source: proposer_src,
            program: proposer_prog,
        },
        compose::SourceFile {
            path: "test_harness.forge".to_string(),
            source: TEST_HARNESS_SRC.to_string(),
            program: harness_prog,
        },
    ];
    let composed = compose::merge_programs(&files).expect("merge");
    let diagnostics = forge::checker::check_all(&composed.program, "test_harness.forge");
    let errors: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.kind == forge::diagnostic::DiagnosticKind::Error)
        .collect();
    assert!(errors.is_empty(), "checker errors: {errors:#?}");
    composed.program
}

struct Harness {
    executor: TaskExecutor,
    rx: tokio::sync::broadcast::Receiver<String>,
}

async fn boot(propose_issue: bool) -> Harness {
    let program = build_program();
    let (events_tx, events_rx) = tokio::sync::broadcast::channel::<String>(1024);
    let tracer = forge::tracer::Tracer::with_live(events_tx.clone());

    let executor = TaskExecutor::new(
        program,
        build_proposer_registry(propose_issue),
        Some(tracer.clone()),
    )
    .with_config(forge::config::ForgeConfig::default_mock_config());

    let event_bus = EventBus::new_shared(executor.tracer().cloned());
    let instance_registry = Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));

    let system_runtime = executor
        .build_system_runtime()
        .expect("build system runtime")
        .expect("test_system should produce a runtime")
        .with_shared_infrastructure(event_bus.clone(), instance_registry.clone());

    let executor = executor.with_event_bus(event_bus.clone());

    tokio::spawn(async move {
        let _ = system_runtime.start().await;
    });

    // Let `on start` run and the timer arm.
    tokio::time::sleep(Duration::from_millis(300)).await;

    Harness {
        executor,
        rx: events_rx,
    }
}

async fn fire(
    h: &Harness,
    thread_ts: &str,
    domain: &str,
    summary: &str,
    confidence: f64,
    suggested_action: &str,
) {
    let mut args = HashMap::new();
    args.insert(
        "thread_ts".to_string(),
        ConfidentValue::deterministic(Value::Text(thread_ts.into())),
    );
    args.insert(
        "domain".to_string(),
        ConfidentValue::deterministic(Value::Text(domain.into())),
    );
    args.insert(
        "summary".to_string(),
        ConfidentValue::deterministic(Value::Text(summary.into())),
    );
    args.insert(
        "confidence".to_string(),
        ConfidentValue::deterministic(Value::Number(confidence)),
    );
    args.insert(
        "suggested_action".to_string(),
        ConfidentValue::deterministic(Value::Text(suggested_action.into())),
    );

    h.executor
        .exec_endpoint("fire_finding", args, None)
        .await
        .expect("endpoint dispatch");
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

// ── Frame helpers ─────────────────────────────────────────────────────

fn proposal_emits(frames: &[serde_json::Value], source: &str) -> usize {
    frames
        .iter()
        .filter(|v| v["event"] == "ProposalReady" && v["source_agent"] == source)
        .count()
}

fn say_lines(frames: &[serde_json::Value]) -> Vec<String> {
    frames
        .iter()
        .filter(|v| v["event"] == "say")
        .filter_map(|v| v["text"].as_str())
        .map(String::from)
        .collect()
}

/// Parse a probe summary line of the shape:
///   "[probe] thread=T-xx kind=propose_issue labels_count=2 evidence_count=2"
/// into (thread, kind, labels_count, evidence_count).
fn parse_probe_summary(s: &str) -> Option<(String, String, usize, usize)> {
    if !s.starts_with("[probe] thread=") {
        return None;
    }
    let after = s.trim_start_matches("[probe] ");
    let mut thread = None;
    let mut kind = None;
    let mut labels_count = None;
    let mut evidence_count = None;
    for part in after.split(' ') {
        let (k, v) = part.split_once('=')?;
        match k {
            "thread" => thread = Some(v.to_string()),
            "kind" => kind = Some(v.to_string()),
            "labels_count" => labels_count = v.parse().ok(),
            "evidence_count" => evidence_count = v.parse().ok(),
            _ => {}
        }
    }
    Some((thread?, kind?, labels_count?, evidence_count?))
}

fn probe_summaries_for(frames: &[serde_json::Value], thread: &str) -> Vec<(String, String, usize, usize)> {
    say_lines(frames)
        .iter()
        .filter_map(|s| parse_probe_summary(s))
        .filter(|(t, _, _, _)| t == thread)
        .collect()
}

fn probe_label_lines(frames: &[serde_json::Value], thread: &str) -> Vec<String> {
    let prefix = format!("[probe] thread={thread} label=");
    say_lines(frames)
        .iter()
        .filter_map(|s| s.strip_prefix(&prefix).map(String::from))
        .collect()
}

fn probe_evidence_lines(frames: &[serde_json::Value], thread: &str) -> Vec<String> {
    let prefix = format!("[probe] thread={thread} evidence=");
    say_lines(frames)
        .iter()
        .filter_map(|s| s.strip_prefix(&prefix).map(String::from))
        .collect()
}

// ── Tests ─────────────────────────────────────────────────────────────

// DoD: "single finding → one ProposalReady emitted".
#[tokio::test]
async fn single_finding_emits_one_proposal() {
    let mut h = boot(true).await;
    fire(&h, "T-single", "ops", "latency spike on api-gateway", 0.5, "answer").await;

    let frames = drain_frames(&mut h.rx, QUIET_WINDOW_PAD).await;
    let count = proposal_emits(&frames, "solution_proposer");
    assert_eq!(
        count, 1,
        "single finding should produce exactly one ProposalReady; got {count} (say={:?})",
        say_lines(&frames)
    );
    let summaries = probe_summaries_for(&frames, "T-single");
    assert_eq!(
        summaries.len(),
        1,
        "probe should observe one ProposalReady on T-single; saw {summaries:?}"
    );
}

// DoD: "two findings with same correlation ID → one ProposalReady emitted".
// Two domains on T-aggr, 200ms apart; the 2s quiet_window must batch both
// into a single proposal whose evidence_refs covers both domains.
#[tokio::test]
async fn two_findings_same_thread_aggregate_to_one_proposal() {
    let mut h = boot(true).await;
    fire(&h, "T-aggr", "ops", "latency spike on api-gateway", 0.5, "answer").await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    fire(&h, "T-aggr", "code", "retry budget exhausted in handler", 0.7, "review").await;

    let frames = drain_frames(&mut h.rx, QUIET_WINDOW_PAD).await;
    let count = proposal_emits(&frames, "solution_proposer");
    assert_eq!(
        count, 1,
        "two findings on same thread should debounce to one ProposalReady; got {count} (say={:?})",
        say_lines(&frames)
    );
    let summaries = probe_summaries_for(&frames, "T-aggr");
    assert_eq!(summaries.len(), 1, "expected one probe summary; saw {summaries:?}");
    let (_, _, _labels_count, evidence_count) = &summaries[0];
    assert_eq!(*evidence_count, 2, "evidence_refs should carry both findings");

    let evidence = probe_evidence_lines(&frames, "T-aggr");
    let joined = evidence.join("|");
    assert!(joined.contains("ops:"), "evidence missing ops domain: {evidence:?}");
    assert!(joined.contains("code:"), "evidence missing code domain: {evidence:?}");
}

// DoD: "for `propose_issue` kind, fills `suggested_labels` with the right
// `clone-dev:<specialist>` label(s)". Mock returns a propose_issue line
// with two labels; assert they survive parsing.
#[tokio::test]
async fn propose_issue_kind_carries_suggested_labels() {
    let mut h = boot(true).await;
    fire(&h, "T-prop", "code", "issue-shaped finding", 0.7, "review").await;

    let frames = drain_frames(&mut h.rx, QUIET_WINDOW_PAD).await;
    let summaries = probe_summaries_for(&frames, "T-prop");
    assert_eq!(summaries.len(), 1, "expected 1 proposal; say={:?}", say_lines(&frames));
    let (_, kind, labels_count, _) = &summaries[0];
    assert_eq!(kind, "propose_issue");
    assert_eq!(*labels_count, 2, "two labels expected from the mock CSV");

    let mut labels = probe_label_lines(&frames, "T-prop");
    labels.sort();
    assert_eq!(
        labels,
        vec!["clone-dev:impl".to_string(), "clone-dev:plan".to_string()],
        "labels should mirror the mock's CSV output"
    );
}

// Inverse: kind=answer → labels stay empty (the mock returns trailing
// empty CSV; parse_labels_csv must drop empties).
#[tokio::test]
async fn answer_kind_has_empty_labels() {
    let mut h = boot(false).await;
    fire(&h, "T-ans", "ops", "answerable finding", 0.5, "answer").await;

    let frames = drain_frames(&mut h.rx, QUIET_WINDOW_PAD).await;
    let summaries = probe_summaries_for(&frames, "T-ans");
    assert_eq!(summaries.len(), 1, "expected 1 proposal; say={:?}", say_lines(&frames));
    let (_, kind, labels_count, _) = &summaries[0];
    assert_eq!(kind, "answer");
    assert_eq!(*labels_count, 0, "answer kind should leave suggested_labels empty");

    let labels = probe_label_lines(&frames, "T-ans");
    assert!(
        labels.is_empty(),
        "no probe label lines should be emitted for answer kind; got {labels:?}"
    );
}
