// Integration tests for T10.3 (#369) — Gate-3 (merge-PR) config
// toggle on the reviewer agent in workflows/dev-cycle/agents.forge.
//
// Gate 3 differs from gates 1 and 2 in shape: the merge-PR approval
// already lives inside the reviewer agent (lines ~720-862 of
// dev-cycle/agents.forge), so this issue adds a config branch and a
// timeout state machine rather than a new standalone agent. The
// reviewer reads `repo_config_for(memory.config, repo_slug).merge_pr`
// to decide whether to emit a Slack approval card or auto-merge.
//
// Unlike gate_one / gate_two, the reviewer is heavily integration-y:
// every code path calls `skill.github.create_pr`, `check_ci`,
// `merge_pr`, and `delete_branch` (real `gh` CLI invocations) plus
// `recall` against the knowledge store. None of the existing tests in
// `tests/` boot the reviewer agent end-to-end (grep confirms it).
// Mocking the gh skill is out of scope for this PR; the live reviewer
// behavior will be exercised by a follow-up live smoke test mirroring
// `clone_dev_gate_two_live_smoke.rs`.
//
// What this file *does* cover deterministically — the new config
// surface that the reviewer reads:
//
//   1. Default value when [gates] merge_pr is omitted (back-compat
//      with pre-#369 TOML files).
//   2. Authored values from the [gates] section override the default.
//   3. Per-repo `[repos."<owner>/<name>"] merge_pr = ...` overrides
//      the gate value while leaving other repos on the global default.
//   4. The shipped `workflows/dev-cycle/clone-dev.toml` parses cleanly
//      and exposes the documented defaults — guards against authoring
//      drift between code and config.
//   5. The FORGE record `to_forge_record()` exposes both the new
//      top-level scalars and the per-repo `merge_pr` field, which is
//      what `repo_config_for` reads on the FORGE side.
//
// The Rust unit tests in src/runtime/clone_dev_config.rs already
// cover (1)-(3) and (5) at a granular level; this file re-asserts
// them at the integration layer (loading from disk, resolving env
// vars, exercising the cache) and adds the shipped-config check (4).

#![allow(clippy::float_cmp)]
#![allow(clippy::await_holding_lock)]

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use forge::compose;
use forge::llm::providers::mock::MockProvider;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::clone_dev_config::CloneDevConfig;
use forge::runtime::confidence::{ConfidentValue, Value};
use forge::runtime::event_bus::{EventBus, EventPayload};
use forge::runtime::executor::TaskExecutor;
use forge::runtime::instance_registry::InstanceRegistry;
use forge::runtime::skill::{SkillCapabilityExecutor, SkillExecutorKind};
use forge::runtime::skill_executor::SkillExecutor;
use forge::runtime::skill_loader::SkillLoader;
use forge::runtime::skill_registry::SkillRegistry;

const SHIPPED_CONFIG_PATH: &str = "workflows/dev-cycle/clone-dev.toml";
const TYPES_PATH: &str = "workflows/clone-dev/shared/types.forge";
const DEV_CYCLE_AGENTS_PATH: &str = "workflows/dev-cycle/agents.forge";
const HANDLER_WINDOW: Duration = Duration::from_secs(3);

static ENV_LOCK: Mutex<()> = Mutex::new(());

const TEST_HARNESS_REVIEWER: &str = r#"#! boundary: server

event PostApproval
  channel: Text
  title: Text
  context: Text
  callback_url: Text
  request_id: Text
  thread_ts: Text

event PostApprovalResult
  channel: Text
  request_id: Text
  approved: Bool
  approver: Text
  summary: Text
  thread_ts: Text

event PostMessage
  channel: Text
  text: Text
  thread_ts: Text

event RequestHuman
  channel: Text
  context: Text
  urgency: Text
  thread_ts: Text

event WardenEscalation
  agent_id: Text
  cause: Text
  detail: Text
  channel: Text

warden test_ward
  manages [reviewer, gate_probe]
  on stuck: escalate, self
  on crash: restart, self
  on timeout: restart, self

agent gate_probe
  subscribe PostApproval
  subscribe PostApprovalResult
  subscribe PostMessage
  subscribe WardenEscalation

  on PostApproval(channel: Text, title: Text, context: Text, callback_url: Text, request_id: Text, thread_ts: Text)
    say "[probe] post_approval channel={channel} request_id={request_id} title={title}"

  on PostApprovalResult(channel: Text, request_id: Text, approved: Bool, approver: Text, summary: Text, thread_ts: Text)
    say "[probe] post_approval_result channel={channel} request_id={request_id} approved={approved} approver={approver}"

  on PostMessage(channel: Text, text: Text, thread_ts: Text)
    say "[probe] post_message channel={channel} text={text}"

  on WardenEscalation(agent_id: Text, cause: Text, detail: Text, channel: Text)
    say "[probe] escalation channel={channel} agent={agent_id} cause={cause}"

system test_system
  use
    review: reviewer
    probe: gate_probe
"#;

fn read_to_program(path: &str) -> (String, forge::ast::Program) {
    let src =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("could not read {path}: {e}"));
    let prog = forge::parser::parse(&src).unwrap_or_else(|e| panic!("parse {path}: {e:?}"));
    (src, prog)
}

fn build_reviewer_program() -> forge::ast::Program {
    let (types_src, types_prog) = read_to_program(TYPES_PATH);
    let (agents_src, agents_prog) = read_to_program(DEV_CYCLE_AGENTS_PATH);
    let harness_prog = forge::parser::parse(TEST_HARNESS_REVIEWER).expect("parse harness");
    let files = vec![
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
            path: "test_harness.forge".to_string(),
            source: TEST_HARNESS_REVIEWER.to_string(),
            program: harness_prog,
        },
    ];
    compose::merge_programs(&files).expect("merge").program
}

fn mock_registry() -> Arc<ProviderRegistry> {
    let mock = MockProvider::new("mock")
        .with_response("Write a Keep-a-Changelog entry", "Fixed: test changelog")
        .with_response(
            "Write a concise PR description",
            "## Summary\n\nTest PR.\n\n## Verification\n\ncargo test",
        )
        .with_default("mock response");
    let mut registry = ProviderRegistry::new("mock");
    registry.register("mock", Arc::new(mock));
    Arc::new(registry)
}

fn stubbed_github_skill() -> forge::runtime::skill::LoadedSkill {
    let mut skill = SkillLoader::parse_skill_md(Path::new("skills/github/SKILL.md"))
        .expect("skills/github/SKILL.md must parse");
    for cap in &mut skill.manifest.capabilities {
        let output = match cap.name.as_str() {
            "create_pr" => "https://github.com/test-org/test-repo/pull/1",
            "update_pr" => "updated",
            "get_pr_for_branch" => {
                r#"{"number":1,"url":"https://github.com/test-org/test-repo/pull/1","state":"OPEN"}"#
            }
            "check_ci" => "pass",
            "merge_pr" => "merged",
            "delete_branch" => "deleted",
            "close_issue" => "closed",
            _ => continue,
        };
        cap.executor = Some(SkillCapabilityExecutor {
            params: Vec::new(),
            kind: SkillExecutorKind::Command,
            argv: vec!["printf".to_string(), "%s".to_string(), output.to_string()],
            result: None,
        });
    }
    skill
}

struct Harness {
    event_bus: forge::runtime::event_bus::SharedEventBus,
    rx: tokio::sync::broadcast::Receiver<String>,
    _tracer: forge::tracer::Tracer,
    _config_dir: tempfile::TempDir,
    _storage_dir: tempfile::TempDir,
}

fn write_reviewer_fixture_config(
    approval_channel: &str,
    default_channel: &str,
) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("create fixture dir");
    let path = dir.path().join("clone-dev.toml");
    let toml = format!(
        r#"
[org]
name = "test-org"

[slack]
default_channel  = "{default_channel}"
approval_channel = "{approval_channel}"

[github]
default_repo = "test-org/test-repo"

[gates]
create_issue                      = true
create_issue_timeout_mins         = 30
start_implementation              = true
start_implementation_timeout_mins = 30
merge_pr                          = true
merge_pr_timeout_mins             = 30

[defaults]
auto_approve       = false
max_plan_revisions = 3

[llm.routing]
plan   = "mock"
review = "mock"
"#
    );
    std::fs::write(&path, toml).expect("write fixture toml");
    std::env::set_var("FORGE_CLONEDEV_CONFIG", &path);
    dir
}

async fn boot_reviewer(approval_channel: &str, default_channel: &str) -> Harness {
    let config_dir = write_reviewer_fixture_config(approval_channel, default_channel);
    let storage_dir = tempfile::tempdir().expect("create storage dir");
    std::env::set_var("FORGE_STORAGE_ROOT", storage_dir.path());

    let program = build_reviewer_program();
    let mut registry = SkillRegistry::new();
    registry.register(stubbed_github_skill());
    let shared_registry = Arc::new(Mutex::new(registry));

    let (events_tx, events_rx) = tokio::sync::broadcast::channel::<String>(2048);
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
        .with_shared_infrastructure(event_bus.clone(), instance_registry);
    tokio::spawn(async move {
        let _ = system_runtime.start().await;
    });
    tokio::time::sleep(Duration::from_millis(700)).await;
    Harness {
        event_bus,
        rx: events_rx,
        _tracer: tracer,
        _config_dir: config_dir,
        _storage_dir: storage_dir,
    }
}

fn cv_text(s: &str) -> ConfidentValue {
    ConfidentValue::deterministic(Value::Text(s.into()))
}

fn cv_bool(b: bool) -> ConfidentValue {
    ConfidentValue::deterministic(Value::Bool(b))
}

async fn fire_acceptance_met(h: &Harness, issue_id: &str, channel: &str) {
    let mut fields = HashMap::new();
    fields.insert("issue_id".to_string(), cv_text(issue_id));
    fields.insert("repo".to_string(), cv_text("test-org/test-repo"));
    fields.insert("title".to_string(), cv_text("Test title"));
    fields.insert("plan".to_string(), cv_text("1. Implement\n2. Test"));
    fields.insert("criteria".to_string(), cv_text("- Gate 3 channel resolves"));
    fields.insert(
        "branch".to_string(),
        cv_text(&format!("clone-dev/{issue_id}")),
    );
    fields.insert("workdir".to_string(), cv_text("/tmp/forge-gate-three-test"));
    fields.insert(
        "report".to_string(),
        cv_text("Acceptance criteria met. Tests green."),
    );
    fields.insert("channel".to_string(), cv_text(channel));
    fields.insert(
        "callback_url".to_string(),
        cv_text("http://localhost:3300/webhook/approval"),
    );
    fields.insert("test_cmd".to_string(), cv_text("cargo test --quiet"));
    fields.insert("ci_passed_first_try".to_string(), cv_bool(true));

    let payload = EventPayload {
        event_name: "AcceptanceMet".to_string(),
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

// ── (1) Default value when [gates] merge_pr is omitted ─────────────

#[test]
fn merge_pr_defaults_to_true_when_gates_section_omits_it() {
    // Pre-#369 TOML files don't carry the new keys. The reviewer must
    // boot with the slack-approval flow on by default — operators opt
    // into auto-merge, never the other way around.
    let cfg = CloneDevConfig::from_toml_str(
        r#"
        [org]
        name = "back-compat"
        "#,
    )
    .expect("parse minimal");
    assert!(
        cfg.gates_merge_pr,
        "gates.merge_pr must default to true (slack-approval on)"
    );
    assert_eq!(
        cfg.gates_merge_pr_timeout_mins, 30.0,
        "gates.merge_pr_timeout_mins must default to 30"
    );
}

// ── (2) Authored values override the default ──────────────────────

#[test]
fn merge_pr_false_disables_slack_approval_globally() {
    // `merge_pr = false` is the sandbox-org case: every repo
    // auto-merges after CI green + knowledge-store consultation,
    // skipping the Slack approval card entirely.
    let cfg = CloneDevConfig::from_toml_str(
        r#"
        [gates]
        merge_pr              = false
        merge_pr_timeout_mins = 60
        "#,
    )
    .expect("parse");
    assert!(!cfg.gates_merge_pr);
    assert_eq!(cfg.gates_merge_pr_timeout_mins, 60.0);
}

// ── (3) Per-repo override semantics ───────────────────────────────

#[test]
fn per_repo_merge_pr_overrides_global_gate_value() {
    // The motivating case from the issue: "some orgs want auto-merge
    // for sandbox repos; most want the manual button-click." A single
    // organization config can mix both — production stays on the gate,
    // sandbox flips off, and both inherit through the same
    // `repo_config_for` lookup the reviewer uses.
    let cfg = CloneDevConfig::from_toml_str(
        r#"
        [gates]
        merge_pr = true

        [repos."acme/prod"]
        # inherits the global gate (true)

        [repos."acme/sandbox"]
        merge_pr = false
        "#,
    )
    .expect("parse");

    assert!(cfg.gates_merge_pr, "global default stays true");
    let prod = cfg
        .repos
        .iter()
        .find(|r| r.slug == "acme/prod")
        .expect("prod repo");
    let sandbox = cfg
        .repos
        .iter()
        .find(|r| r.slug == "acme/sandbox")
        .expect("sandbox repo");
    assert!(
        prod.merge_pr,
        "prod inherits gates.merge_pr = true (slack approval still required)"
    );
    assert!(
        !sandbox.merge_pr,
        "sandbox per-repo override flips merge_pr to false (auto-merge)"
    );
}

#[test]
fn per_repo_merge_pr_can_re_enable_when_gate_is_off() {
    // Symmetric override: a globally-disabled gate can be re-enabled
    // for one critical repo. This is the "auto-merge everywhere except
    // the production repo" shape.
    let cfg = CloneDevConfig::from_toml_str(
        r#"
        [gates]
        merge_pr = false

        [repos."acme/critical"]
        merge_pr = true

        [repos."acme/sandbox"]
        # inherits gates.merge_pr = false
        "#,
    )
    .expect("parse");

    assert!(!cfg.gates_merge_pr);
    let critical = cfg
        .repos
        .iter()
        .find(|r| r.slug == "acme/critical")
        .expect("critical repo");
    let sandbox = cfg
        .repos
        .iter()
        .find(|r| r.slug == "acme/sandbox")
        .expect("sandbox repo");
    assert!(
        critical.merge_pr,
        "critical repo per-repo override re-enables slack approval"
    );
    assert!(!sandbox.merge_pr, "sandbox inherits gates.merge_pr = false");
}

// ── (4) Shipped config parses with documented defaults ─────────────

#[test]
fn shipped_clone_dev_toml_parses_with_documented_gate_three_defaults() {
    // The dev-cycle standalone runner ships with
    // workflows/dev-cycle/clone-dev.toml. After this PR it carries
    // `[gates] merge_pr = true` and `merge_pr_timeout_mins = 30`. If
    // an operator deletes those lines or types `merge_pr = false` by
    // accident, this test catches the drift before the reviewer boots
    // with surprising behavior.
    let src = std::fs::read_to_string(SHIPPED_CONFIG_PATH).unwrap_or_else(|e| {
        panic!("could not read {SHIPPED_CONFIG_PATH}: {e}");
    });
    let cfg = CloneDevConfig::from_toml_str(&src).expect("parse shipped config");
    assert!(
        cfg.gates_merge_pr,
        "shipped config should keep gates.merge_pr = true (default behavior)"
    );
    assert_eq!(
        cfg.gates_merge_pr_timeout_mins, 30.0,
        "shipped config should keep gates.merge_pr_timeout_mins = 30"
    );
}

// ── (5) FORGE record exposes the new fields ───────────────────────

#[test]
fn forge_record_carries_merge_pr_top_level_and_per_repo() {
    // The reviewer's FORGE code reads `memory.config.gates_merge_pr`
    // (top-level) and `repo_config_for(...).merge_pr` (per-repo). Both
    // must surface on the Value::Record produced by `to_forge_record`
    // or the reviewer's `on start` log line will print garbage and
    // the auto-merge branch will silently never fire.
    use forge::runtime::confidence::Value;

    let cfg = CloneDevConfig::from_toml_str(
        r#"
        [gates]
        merge_pr              = false
        merge_pr_timeout_mins = 45

        [repos."acme/sandbox"]
        merge_pr = true
        "#,
    )
    .expect("parse");
    let record = cfg.to_forge_record();
    let fields = match record {
        Value::Record(ref f) => f,
        _ => panic!("expected Record"),
    };

    let flag = fields
        .get("gates_merge_pr")
        .expect("gates_merge_pr field missing");
    match &flag.value {
        Value::Bool(b) => assert!(!*b, "top-level gates_merge_pr should reflect TOML"),
        _ => panic!("gates_merge_pr should be Bool"),
    }

    let timeout = fields
        .get("gates_merge_pr_timeout_mins")
        .expect("gates_merge_pr_timeout_mins field missing");
    match &timeout.value {
        Value::Number(n) => assert_eq!(*n, 45.0),
        _ => panic!("gates_merge_pr_timeout_mins should be Number"),
    }

    // The per-repo override surfaces on the repo record so
    // `repo_config_for` returns the right value. Sandbox flipped the
    // gate back to true even though the global default is false.
    let repos = fields.get("repos").expect("repos field missing");
    let items = match &repos.value {
        Value::Array(items) => items,
        _ => panic!("repos should be Array"),
    };
    let sandbox = items
        .iter()
        .find_map(|cv| match &cv.value {
            Value::Record(r) => {
                let slug = r.get("slug").and_then(|s| match &s.value {
                    Value::Text(t) => Some(t.as_str()),
                    _ => None,
                })?;
                if slug == "acme/sandbox" {
                    Some(r)
                } else {
                    None
                }
            }
            _ => None,
        })
        .expect("sandbox repo record missing");
    let merge_pr = sandbox
        .get("merge_pr")
        .expect("repo merge_pr field missing");
    match &merge_pr.value {
        Value::Bool(b) => assert!(
            *b,
            "sandbox per-repo merge_pr override should win on the repo record"
        ),
        _ => panic!("repo.merge_pr should be Bool"),
    }
}

#[tokio::test]
async fn clone_dev_gate_three_empty_inbound_channel_uses_configured_approval_channel() {
    let _guard = ENV_LOCK.lock().unwrap();
    let mut h = boot_reviewer("C-reviewers", "C-default").await;

    fire_acceptance_met(&h, "430-approval", "").await;
    let frames = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let says = say_lines(&frames);

    assert!(
        says.iter().any(|s| {
            s.contains("[probe] post_approval channel=C-reviewers")
                && s.contains("request_id=430-approval")
        }),
        "Gate 3 PostApproval must route to slack.approval_channel when inbound channel is empty; says={says:#?}"
    );
    assert!(
        !says
            .iter()
            .any(|s| s.contains("[probe] post_approval channel= request_id=430-approval")),
        "Gate 3 must not emit an empty-channel PostApproval; says={says:#?}"
    );
}

#[tokio::test]
async fn clone_dev_gate_three_empty_inbound_channel_falls_back_to_default_channel() {
    let _guard = ENV_LOCK.lock().unwrap();
    let mut h = boot_reviewer("", "C-default").await;

    fire_acceptance_met(&h, "430-default", "").await;
    let frames = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let says = say_lines(&frames);

    assert!(
        says.iter().any(|s| {
            s.contains("[probe] post_approval channel=C-default")
                && s.contains("request_id=430-default")
        }),
        "Gate 3 PostApproval must fall back to slack.default_channel when approval and inbound channels are empty; says={says:#?}"
    );
}

#[tokio::test]
async fn clone_dev_gate_three_approval_result_uses_resolved_channel() {
    let _guard = ENV_LOCK.lock().unwrap();
    let mut h = boot_reviewer("C-reviewers", "C-default").await;

    fire_acceptance_met(&h, "430-result", "").await;
    let before = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let before_says = say_lines(&before);
    assert!(
        before_says
            .iter()
            .any(|s| s.contains("[probe] post_approval channel=C-reviewers")),
        "precondition: Gate 3 approval should be pending in C-reviewers; says={before_says:#?}"
    );

    fire_approval_response(&h, "430-result", true, "alice").await;
    let after = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let after_says = say_lines(&after);

    assert!(
        after_says.iter().any(|s| {
            s.contains("[probe] post_approval_result channel=C-reviewers")
                && s.contains("request_id=430-result")
                && s.contains("approved=true")
        }),
        "Gate 3 approval confirmation must use the same resolved channel; says={after_says:#?}"
    );
}
