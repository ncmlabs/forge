// Integration test for T9.4 (#365) — issue_creator agent under
// workflows/clone-dev/stage1/issue_creator.forge.
//
// Verifies the Stage-1 → Stage-2 hand-off slice:
//   ProposalApproved(kind: "propose_issue", repo, title, body, ...) →
//     issue_creator (subscribe ProposalApproved where kind == ...) →
//       skill.github.create_labeled_issue (deterministic stub) →
//         emit IssueCreated(thread_ts, repo, issue_url, labels, "clone-dev")
//         emit PostMessage(channel, "Issue created: {url}", thread_ts)
//
// Skill mocking follows tests/skill_failure_live_test.rs: load the
// real skills/github/SKILL.md, then mutate the `create_labeled_issue`
// capability's argv to a deterministic command (printf URL → success,
// false → failure). This exercises the same SkillExecutor::execute_deterministic
// path as production, with no LLM and no network.
//
// Cases:
//   1. Happy path             — kind=propose_issue, skill returns URL → IssueCreated + PostMessage.
//   2. Wrong kind ignored     — kind=answer → no IssueCreated, no skill call.
//   3. Both attempts fail     — skill argv pinned to `false` → WardenEscalation, no IssueCreated.
//
// The retry-then-success path (first call fails, second succeeds) is
// not covered here because the deterministic argv stub is stateless;
// the same agent code that handles attempt #1 handles attempt #2 with
// identical control flow, so case 3 covers the retry call site and
// case 1 covers the success call site. Stateful flaky-skill simulation
// is left for an end-to-end test against a live `gh` retry harness.
// ──────────────────────────────────────────────────────────────────────

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use forge::compose;
use forge::runtime::confidence::{ConfidentValue, Value};
use forge::runtime::event_bus::{EventBus, EventPayload};
use forge::runtime::executor::TaskExecutor;
use forge::runtime::instance_registry::InstanceRegistry;
use forge::runtime::skill_executor::SkillExecutor;
use forge::runtime::skill_loader::SkillLoader;
use forge::runtime::skill_registry::SkillRegistry;

const ISSUE_CREATOR_PATH: &str = "workflows/clone-dev/stage1/issue_creator.forge";

const STUB_ISSUE_URL: &str = "https://github.com/test-org/test-repo/issues/42";

// Window for the agent to receive ProposalApproved, run the skill (a
// `printf` child process), and emit IssueCreated + PostMessage.
const HANDLER_WINDOW: Duration = Duration::from_millis(2500);

// Test harness: redeclares the four event surfaces issue_creator
// touches (ProposalApproved input; IssueCreated, PostMessage,
// WardenEscalation outputs) plus a minimal investigators_ward and a
// `creator_probe` agent that re-emits payloads as `say` lines so
// tests can inspect fields the SSE event_emit frame strips.
const TEST_HARNESS_SRC: &str = r#"#! boundary: server

event ProposalApproved
  thread_ts: Text
  channel: Text
  kind: Text
  repo: Text
  title: Text
  body: Text
  suggested_labels: Text[]
  evidence_refs: Text[]

event IssueCreated
  thread_ts: Text
  repo: Text
  issue_url: Text
  labels: Text[]
  creator: Text

event PostMessage
  channel: Text
  text: Text
  thread_ts: Text

event WardenEscalation
  agent_id: Text
  cause: Text
  detail: Text
  channel: Text

event IssueCreatorSucceeded
  thread_ts: Text
  channel: Text
  repo: Text
  issue_url: Text
  labels: Text[]

event IssueCreatorRetry
  thread_ts: Text
  channel: Text
  repo: Text
  title: Text
  body: Text
  composite_labels_csv: Text
  full_labels: Text[]

event IssueCreatorExhausted
  thread_ts: Text
  channel: Text
  repo: Text
  title: Text

warden investigators_ward
  manages [issue_creator, creator_probe]
  on stuck: escalate, self
  on crash: restart, self
  on timeout: restart, self

agent creator_probe
  memory
    last_thread: Text
  subscribe IssueCreated
  subscribe PostMessage
  subscribe WardenEscalation

  on IssueCreated(thread_ts: Text, repo: Text, issue_url: Text, labels: Text[], creator: Text)
    memory.last_thread = thread_ts
    say "[probe] issue_created thread={thread_ts} repo={repo} url={issue_url} labels_count={labels.length} creator={creator}"
    for lbl in labels
      say "[probe] thread={thread_ts} label={lbl}"

  on PostMessage(channel: Text, text: Text, thread_ts: Text)
    say "[probe] post_message channel={channel} thread={thread_ts} text={text}"

  on WardenEscalation(agent_id: Text, cause: Text, detail: Text, channel: Text)
    say "[probe] escalation agent={agent_id} cause={cause} detail={detail} channel={channel}"

system test_system
  use
    creator: issue_creator
    probe: creator_probe
"#;

fn read_to_program(path: &str) -> (String, forge::ast::Program) {
    let src =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("could not read {path}: {e}"));
    let prog = forge::parser::parse(&src).unwrap_or_else(|e| panic!("parse {path}: {e:?}"));
    (src, prog)
}

fn build_program() -> forge::ast::Program {
    let (creator_src, creator_prog) = read_to_program(ISSUE_CREATOR_PATH);
    let harness_prog = forge::parser::parse(TEST_HARNESS_SRC).expect("parse harness");

    let files = vec![
        compose::SourceFile {
            path: ISSUE_CREATOR_PATH.to_string(),
            source: creator_src,
            program: creator_prog,
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

/// Load `skills/github/SKILL.md` and replace the `create_labeled_issue`
/// capability's argv with a deterministic test command. `succeeding == true`
/// pins it to `printf "%s" <STUB_ISSUE_URL>` (exit 0, stdout = URL);
/// `succeeding == false` pins it to `false` (exit 1, no stdout) so both
/// attempts fail and the agent's escalation path fires.
fn load_github_skill(succeeding: bool) -> forge::runtime::skill::LoadedSkill {
    let mut skill = SkillLoader::parse_skill_md(Path::new("skills/github/SKILL.md"))
        .expect("skills/github/SKILL.md must parse — repo invariant");

    let mut replaced = false;
    for cap in &mut skill.manifest.capabilities {
        if cap.name == "create_labeled_issue" {
            let exec = cap
                .executor
                .as_mut()
                .expect("create_labeled_issue must ship with a deterministic executor");
            exec.argv = if succeeding {
                vec![
                    "printf".to_string(),
                    "%s".to_string(),
                    STUB_ISSUE_URL.to_string(),
                ]
            } else {
                vec!["false".to_string()]
            };
            replaced = true;
            break;
        }
    }
    assert!(
        replaced,
        "skills/github/SKILL.md must expose create_labeled_issue"
    );
    skill
}

struct Harness {
    event_bus: forge::runtime::event_bus::SharedEventBus,
    rx: tokio::sync::broadcast::Receiver<String>,
    // Keep these alive for the duration of the test.
    _tracer: forge::tracer::Tracer,
}

/// Boots the runtime with a github skill whose `create_labeled_issue`
/// is pinned to either succeed (returning STUB_ISSUE_URL) or fail (`false`).
async fn boot(skill_succeeds: bool) -> Harness {
    let program = build_program();

    let github_skill = load_github_skill(skill_succeeds);
    let mut registry = SkillRegistry::new();
    registry.register(github_skill);
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

    // `on start` and subscription registration.
    tokio::time::sleep(Duration::from_millis(400)).await;

    Harness {
        event_bus,
        rx: events_rx,
        _tracer: tracer,
    }
}

fn cv_text(s: &str) -> ConfidentValue {
    ConfidentValue::deterministic(Value::Text(s.into()))
}

fn cv_text_array(items: &[&str]) -> ConfidentValue {
    ConfidentValue::deterministic(Value::Array(
        items
            .iter()
            .map(|s| ConfidentValue::deterministic(Value::Text((*s).into())))
            .collect(),
    ))
}

#[allow(clippy::too_many_arguments)]
async fn fire_proposal_approved(
    h: &Harness,
    thread_ts: &str,
    channel: &str,
    kind: &str,
    repo: &str,
    title: &str,
    body: &str,
    suggested_labels: &[&str],
) {
    let mut fields = HashMap::new();
    fields.insert("thread_ts".to_string(), cv_text(thread_ts));
    fields.insert("channel".to_string(), cv_text(channel));
    fields.insert("kind".to_string(), cv_text(kind));
    fields.insert("repo".to_string(), cv_text(repo));
    fields.insert("title".to_string(), cv_text(title));
    fields.insert("body".to_string(), cv_text(body));
    fields.insert(
        "suggested_labels".to_string(),
        cv_text_array(suggested_labels),
    );
    fields.insert("evidence_refs".to_string(), cv_text_array(&[]));

    let payload = EventPayload {
        event_name: "ProposalApproved".to_string(),
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

// DoD: "an approved proposal creates an issue on a test repo; issue has
// the expected labels; IssueCreated event observed".
#[tokio::test]
async fn approved_propose_issue_emits_issue_created_and_posts_to_thread() {
    let mut h = boot(true).await;
    fire_proposal_approved(
        &h,
        "T-happy",
        "C-devops",
        "propose_issue",
        "test-org/test-repo",
        "Investigate api-gateway latency",
        "Latency spike correlates with retry budget regression.",
        &["clone-dev:plan"],
    )
    .await;

    let frames = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let says = say_lines(&frames);

    let issue_created = count_emits(&frames, "IssueCreated", "issue_creator");
    assert_eq!(
        issue_created, 1,
        "exactly one IssueCreated expected; says={says:?}"
    );

    let post_message = count_emits(&frames, "PostMessage", "issue_creator");
    assert_eq!(
        post_message, 1,
        "exactly one PostMessage expected; says={says:?}"
    );

    // Probe's IssueCreated re-emission carries the URL the skill returned.
    let probe_lines: Vec<&String> = says
        .iter()
        .filter(|s| s.starts_with("[probe] issue_created"))
        .collect();
    assert_eq!(
        probe_lines.len(),
        1,
        "probe should observe one IssueCreated; says={says:?}"
    );
    let probe_line = probe_lines[0];
    assert!(
        probe_line.contains(&format!("url={STUB_ISSUE_URL}")),
        "probe line missing stub URL: {probe_line}"
    );
    assert!(
        probe_line.contains("creator=clone-dev"),
        "probe line missing creator tag: {probe_line}"
    );
    // composite labels = ["clone-dev"] + suggested_labels = 2 entries.
    assert!(
        probe_line.contains("labels_count=2"),
        "expected labels_count=2 (clone-dev + 1 suggested); got: {probe_line}"
    );

    // Confirmation post lands in the originating thread.
    let post_line = says
        .iter()
        .find(|s| s.starts_with("[probe] post_message"))
        .unwrap_or_else(|| panic!("no probe post_message line; says={says:?}"));
    assert!(
        post_line.contains("thread=T-happy"),
        "post should target T-happy thread: {post_line}"
    );
    assert!(
        post_line.contains(&format!("Issue created: {STUB_ISSUE_URL}")),
        "post text should reference stub URL: {post_line}"
    );

    // No escalation on the happy path.
    let escalations = count_emits(&frames, "WardenEscalation", "issue_creator");
    assert_eq!(escalations, 0, "no escalation on happy path; says={says:?}");
}

// DoD-adjacent: subscribe filter `where kind == "propose_issue"` keeps
// other proposal kinds out of the issue-creation surface.
#[tokio::test]
async fn answer_kind_does_not_create_issue() {
    let mut h = boot(true).await;
    fire_proposal_approved(
        &h,
        "T-skip",
        "C-devops",
        "answer",
        "test-org/test-repo",
        "Should not appear",
        "Body",
        &[],
    )
    .await;

    let frames = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let says = say_lines(&frames);

    assert_eq!(
        count_emits(&frames, "IssueCreated", "issue_creator"),
        0,
        "answer kind must not produce IssueCreated; says={says:?}"
    );
    assert_eq!(
        count_emits(&frames, "PostMessage", "issue_creator"),
        0,
        "answer kind must not produce PostMessage; says={says:?}"
    );
    assert_eq!(
        count_emits(&frames, "WardenEscalation", "issue_creator"),
        0,
        "answer kind must not produce WardenEscalation; says={says:?}"
    );
}

// DoD: "Handles skill.github.create_issue failures: retry once, then
// escalate via warden". With argv pinned to `false`, both attempts fail
// → WardenEscalation, no IssueCreated.
#[tokio::test]
async fn double_skill_failure_escalates_via_warden() {
    let mut h = boot(false).await;
    fire_proposal_approved(
        &h,
        "T-fail",
        "C-devops",
        "propose_issue",
        "test-org/test-repo",
        "Will fail twice",
        "Body",
        &["clone-dev:plan"],
    )
    .await;

    let frames = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let says = say_lines(&frames);

    assert_eq!(
        count_emits(&frames, "IssueCreated", "issue_creator"),
        0,
        "no IssueCreated when skill fails; says={says:?}"
    );
    assert_eq!(
        count_emits(&frames, "PostMessage", "issue_creator"),
        0,
        "no PostMessage when skill fails; says={says:?}"
    );

    let escalations = count_emits(&frames, "WardenEscalation", "issue_creator");
    assert_eq!(
        escalations, 1,
        "exactly one WardenEscalation after retry exhaustion; says={says:?}"
    );

    let probe_esc = says
        .iter()
        .find(|s| s.starts_with("[probe] escalation"))
        .unwrap_or_else(|| panic!("probe should observe escalation; says={says:?}"));
    assert!(
        probe_esc.contains("agent=issue_creator"),
        "escalation must name issue_creator: {probe_esc}"
    );
    assert!(
        probe_esc.contains("cause=github_create_failed"),
        "escalation cause should be github_create_failed: {probe_esc}"
    );
}
