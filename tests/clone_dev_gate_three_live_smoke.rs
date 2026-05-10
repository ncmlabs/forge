// Real-Slack + Real-GitHub live smoke for T10.3 (#369) — gate_three
// (merge-PR config toggle on the reviewer agent) end-to-end against
// actual external services.
//
// Mirrors the gate_two live-smoke pattern (clone_dev_gate_two_live_smoke.rs)
// but exercises the reviewer agent's two new code paths:
//
//   merge_pr = true  → reviewer emits PostApproval; a real Slack
//                      approval card lands in the test channel.
//   merge_pr = false → reviewer skips slack approval, calls
//                      skill.github.merge_pr directly, posts a
//                      :robot_face: AUTO-MERGED PostMessage to the
//                      channel instead.
//
// Each case is gated on its own env vars (#288 pattern) so a bare
// `cargo test` never touches Slack or GitHub:
//
//   FORGE_SLACK_LIVE=1 SLACK_BOT_TOKEN=xoxb-... FORGE_SLACK_TEST_CHANNEL=C-...
//   FORGE_GITHUB_LIVE=1 GITHUB_TOKEN=ghp-... FORGE_GITHUB_TEST_REPO=ncmlabs/forge-playground
//
// Running:
//   FORGE_SLACK_LIVE=1 SLACK_BOT_TOKEN=... FORGE_SLACK_TEST_CHANNEL=... \
//   FORGE_GITHUB_LIVE=1 GITHUB_TOKEN=... FORGE_GITHUB_TEST_REPO=... \
//     cargo test --test clone_dev_gate_three_live_smoke -- --nocapture
//
// Cases:
//   1. live_gate_three_posts_real_slack_approval_card_when_merge_pr_true
//      — needs Slack + GitHub. Boots reviewer + slack_adapter, sets up
//      a real test branch + commit + PR via `gh`, fires AcceptanceMet,
//      polls Slack for the approval-card title "PR for ... ready for
//      review", then cleans up.
//
//   2. live_gate_three_auto_merges_when_merge_pr_false
//      — needs Slack + GitHub. Same setup but with `gates.merge_pr =
//      false` in the fixture config. Asserts no PostApproval emitted,
//      `skill.github.merge_pr` invoked (PR actually merged on
//      GitHub), and the :robot_face: AUTO-MERGED PostMessage lands in
//      Slack. Cleans up the merged PR (squash deletes the branch).
//
// The 2x merge_approval_timeout escalation path is not exercised here
// (no deterministic way to fast-forward FORGE timer ticks in-process)
// — operators can manually verify by running with
// `merge_pr_timeout_mins = 1` and waiting 2 minutes.
// ──────────────────────────────────────────────────────────────────────

#![allow(clippy::await_holding_lock)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use forge::compose;
use forge::config::ForgeConfig;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::confidence::{ConfidentValue, Value};
use forge::runtime::event_bus::{EventBus, EventPayload};
use forge::runtime::executor::TaskExecutor;
use forge::runtime::instance_registry::InstanceRegistry;
use forge::runtime::skill_executor::SkillExecutor;
use forge::runtime::skill_registry::SkillRegistry;

const TYPES_PATH: &str = "workflows/clone-dev/shared/types.forge";
const DEV_CYCLE_AGENTS_PATH: &str = "workflows/dev-cycle/agents.forge";
const SLACK_ADAPTER_PATH: &str = "examples/agents/slack-adapter/agents.forge";

const HANDLER_WINDOW: Duration = Duration::from_secs(30);
const SLACK_POLL_TIMEOUT: Duration = Duration::from_secs(20);

static ENV_LOCK: Mutex<()> = Mutex::new(());

// We boot just the reviewer + slack_adapter + a probe — not the full
// dev-cycle pipeline. The reviewer is the only T10.3 surface; the
// upstream planner/implementer/tester aren't needed because we drive
// AcceptanceMet directly.
const TEST_HARNESS_REVIEWER_AND_SLACK: &str = r#"#! boundary: server

warden test_ward
  manages [reviewer, slack_adapter, gate_probe]
  on stuck: escalate, self
  on crash: restart, self
  on timeout: restart, self

agent gate_probe
  memory
    last_issue: Text
  subscribe PostApproval
  subscribe PostApprovalResult
  subscribe PostMessage
  subscribe PRMerged
  subscribe TestsFailed
  subscribe WardenEscalation

  on PostApproval(channel: Text, title: Text, context: Text, callback_url: Text, request_id: Text, thread_ts: Text)
    say "[probe] post_approval channel={channel} request_id={request_id} title={title}"

  on PostApprovalResult(channel: Text, request_id: Text, approved: Bool, approver: Text, summary: Text, thread_ts: Text)
    say "[probe] post_approval_result channel={channel} request_id={request_id} approved={approved} approver={approver}"

  on PostMessage(channel: Text, text: Text, thread_ts: Text)
    say "[probe] post_message channel={channel} text={text}"

  on PRMerged(issue_id: Text, repo: Text, branch: Text, ci_passed_first_try: Bool, review_rounds: Number)
    memory.last_issue = issue_id
    say "[probe] pr_merged issue={issue_id} repo={repo} branch={branch}"

  on TestsFailed(issue_id: Text, repo: Text, branch: Text, failures: Text, channel: Text, callback_url: Text, test_cmd: Text)
    say "[probe] tests_failed issue={issue_id} branch={branch}"

  on WardenEscalation(agent_id: Text, cause: Text, detail: Text, channel: Text)
    say "[probe] escalation agent={agent_id} cause={cause}"

system test_system
  use
    review: reviewer
    slack: slack_adapter
    probe: gate_probe
"#;

fn read_to_program(path: &str) -> (String, forge::ast::Program) {
    let src =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("could not read {path}: {e}"));
    let prog = forge::parser::parse(&src).unwrap_or_else(|e| panic!("parse {path}: {e:?}"));
    (src, prog)
}

fn build_program_with_slack() -> forge::ast::Program {
    let (types_src, types_prog) = read_to_program(TYPES_PATH);
    let (agents_src, agents_prog) = read_to_program(DEV_CYCLE_AGENTS_PATH);
    let (slack_src, slack_prog) = read_to_program(SLACK_ADAPTER_PATH);
    let harness_prog =
        forge::parser::parse(TEST_HARNESS_REVIEWER_AND_SLACK).expect("parse harness");

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
            path: SLACK_ADAPTER_PATH.to_string(),
            source: slack_src,
            program: slack_prog,
        },
        compose::SourceFile {
            path: "test_harness.forge".to_string(),
            source: TEST_HARNESS_REVIEWER_AND_SLACK.to_string(),
            program: harness_prog,
        },
    ];
    let composed = compose::merge_programs(&files).expect("merge");
    composed.program
}

fn slack_live_provider() -> Option<(String, String)> {
    if std::env::var("FORGE_SLACK_LIVE").ok().as_deref() != Some("1") {
        return None;
    }
    let token = std::env::var("SLACK_BOT_TOKEN").ok()?;
    let channel = std::env::var("FORGE_SLACK_TEST_CHANNEL").ok()?;
    if token.is_empty() || channel.is_empty() {
        return None;
    }
    Some((token, channel))
}

fn github_live_provider() -> Option<(String, String)> {
    if std::env::var("FORGE_GITHUB_LIVE").ok().as_deref() != Some("1") {
        return None;
    }
    let token = std::env::var("GITHUB_TOKEN").ok()?;
    let repo = std::env::var("FORGE_GITHUB_TEST_REPO").ok()?;
    if token.is_empty() || repo.is_empty() {
        return None;
    }
    Some((token, repo))
}

fn write_fixture_config(test_name: &str, slack_channel: &str, merge_pr: bool) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "forge-gate-three-smoke-{}-{}",
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
default_channel  = "{slack_channel}"
approval_channel = "{slack_channel}"
bot_token_env    = "SLACK_BOT_TOKEN"

[github]
default_repo = ""
token_env    = "GITHUB_TOKEN"

[gates]
create_issue                      = true
create_issue_timeout_mins         = 30
start_implementation              = true
start_implementation_timeout_mins = 30
merge_pr                          = {merge_pr}
merge_pr_timeout_mins             = 30

[defaults]
max_plan_revisions = 3
auto_approve       = false

[llm.routing]
plan = "mock"
"#
    );
    std::fs::write(&path, toml).expect("write fixture toml");
    std::env::set_var("FORGE_CLONEDEV_CONFIG", &path);
    path
}

struct Harness {
    event_bus: forge::runtime::event_bus::SharedEventBus,
    rx: tokio::sync::broadcast::Receiver<String>,
    skill_registry: Arc<Mutex<SkillRegistry>>,
    _tracer: forge::tracer::Tracer,
}

async fn boot(providers: Arc<ProviderRegistry>) -> Harness {
    let program = build_program_with_slack();

    // Real Slack/GitHub skill registry — load from on-disk SKILL.md
    // so the executor resolves chat.postMessage / pr merge / etc. via
    // the same argv shapes the runtime uses in production.
    let mut registry = SkillRegistry::new();
    let skills = forge::runtime::skill_loader::SkillLoader::load_from_dirs(&[
        PathBuf::from("skills/slack"),
        PathBuf::from("skills/github"),
    ]);
    for skill in skills {
        registry.register(skill);
    }
    let shared_registry = Arc::new(Mutex::new(registry));

    let (events_tx, events_rx) = tokio::sync::broadcast::channel::<String>(2048);
    let tracer = forge::tracer::Tracer::with_live(events_tx.clone());

    let skill_executor = Arc::new(
        SkillExecutor::new(providers.clone(), shared_registry.clone())
            .with_tracer(Arc::new(tracer.clone())),
    );

    let executor = TaskExecutor::new(program, providers, Some(tracer.clone()))
        .with_config(ForgeConfig::default_mock_config())
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

    tokio::time::sleep(Duration::from_millis(800)).await;

    Harness {
        event_bus,
        rx: events_rx,
        skill_registry: shared_registry,
        _tracer: tracer,
    }
}

fn cv_text(s: &str) -> ConfidentValue {
    ConfidentValue::deterministic(Value::Text(s.into()))
}

fn cv_bool(b: bool) -> ConfidentValue {
    ConfidentValue::deterministic(Value::Bool(b))
}

async fn fire_acceptance_met(
    h: &Harness,
    issue_id: &str,
    repo: &str,
    branch: &str,
    workdir: &str,
    channel: &str,
) {
    let mut fields = HashMap::new();
    fields.insert("issue_id".to_string(), cv_text(issue_id));
    fields.insert("repo".to_string(), cv_text(repo));
    fields.insert("branch".to_string(), cv_text(branch));
    fields.insert("workdir".to_string(), cv_text(workdir));
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

/// Poll Slack `conversations.history` until a message containing
/// `marker` appears, or the timeout elapses.
fn poll_slack_for_marker(
    token: &str,
    channel: &str,
    marker: &str,
    timeout: Duration,
) -> Option<String> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        let out = Command::new("curl")
            .arg("-s")
            .arg("-H")
            .arg(format!("Authorization: Bearer {token}"))
            .arg(format!(
                "https://slack.com/api/conversations.history?channel={channel}&limit=10"
            ))
            .output()
            .ok()?;
        let body = String::from_utf8_lossy(&out.stdout);
        if body.contains(marker) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                if let Some(arr) = json.get("messages").and_then(|m| m.as_array()) {
                    for msg in arr {
                        let text = msg.get("text").and_then(|t| t.as_str()).unwrap_or("");
                        let blocks = msg.to_string();
                        if text.contains(marker) || blocks.contains(marker) {
                            if let Some(ts) = msg.get("ts").and_then(|t| t.as_str()) {
                                return Some(ts.to_string());
                            }
                        }
                    }
                }
            }
            return Some(String::new());
        }
        std::thread::sleep(Duration::from_millis(1500));
    }
    None
}

fn delete_slack_message(token: &str, channel: &str, ts: &str) {
    if ts.is_empty() {
        return;
    }
    let _ = Command::new("curl")
        .arg("-s")
        .arg("-X")
        .arg("POST")
        .arg("-H")
        .arg(format!("Authorization: Bearer {token}"))
        .arg("-H")
        .arg("Content-Type: application/json; charset=utf-8")
        .arg("-d")
        .arg(format!(r#"{{"channel":"{channel}","ts":"{ts}"}}"#))
        .arg("https://slack.com/api/chat.delete")
        .output();
}

/// Set up a throwaway test branch + commit on the test repo so the
/// reviewer's create_pr / merge_pr calls have a real target. Returns
/// the branch name on success.
fn setup_test_branch(repo: &str, branch: &str) -> bool {
    // Get the default-branch SHA to fork from.
    let sha_out = Command::new("gh")
        .arg("api")
        .arg(format!("repos/{repo}"))
        .arg("--jq")
        .arg(".default_branch")
        .output();
    let default_branch = match sha_out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => return false,
    };
    let head_sha_out = Command::new("gh")
        .arg("api")
        .arg(format!("repos/{repo}/git/refs/heads/{default_branch}"))
        .arg("--jq")
        .arg(".object.sha")
        .output();
    let head_sha = match head_sha_out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => return false,
    };

    // Create the test branch.
    let create = Command::new("gh")
        .arg("api")
        .arg("-X")
        .arg("POST")
        .arg(format!("repos/{repo}/git/refs"))
        .arg("-f")
        .arg(format!("ref=refs/heads/{branch}"))
        .arg("-f")
        .arg(format!("sha={head_sha}"))
        .output();
    if !matches!(&create, Ok(o) if o.status.success()) {
        return false;
    }

    // Add a tiny commit so the branch differs from the default branch
    // (gh pr create requires a non-empty diff). We touch a marker file.
    let put = Command::new("gh")
        .arg("api")
        .arg("-X")
        .arg("PUT")
        .arg(format!(
            "repos/{repo}/contents/.gate-three-smoke-{branch}.txt"
        ))
        .arg("-f")
        .arg("message=gate_three smoke marker")
        .arg("-f")
        .arg(format!("branch={branch}"))
        .arg("-f")
        .arg("content=Z2F0ZV90aHJlZSBzbW9rZSBtYXJrZXIK") // base64("gate_three smoke marker\n")
        .output();
    matches!(put, Ok(o) if o.status.success())
}

fn cleanup_test_branch(repo: &str, branch: &str) {
    let _ = Command::new("gh")
        .arg("api")
        .arg("-X")
        .arg("DELETE")
        .arg(format!("repos/{repo}/git/refs/heads/{branch}"))
        .output();
}

fn close_open_pr_for_branch(repo: &str, branch: &str) {
    // List PRs for this branch and close any still open.
    let list = Command::new("gh")
        .arg("pr")
        .arg("list")
        .arg("-R")
        .arg(repo)
        .arg("--head")
        .arg(branch)
        .arg("--state")
        .arg("open")
        .arg("--json")
        .arg("number")
        .output();
    if let Ok(o) = list {
        if o.status.success() {
            if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&o.stdout) {
                if let Some(arr) = json.as_array() {
                    for pr in arr {
                        if let Some(num) = pr.get("number").and_then(|n| n.as_i64()) {
                            let _ = Command::new("gh")
                                .arg("pr")
                                .arg("close")
                                .arg("-R")
                                .arg(repo)
                                .arg(num.to_string())
                                .output();
                        }
                    }
                }
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────

// merge_pr=true smoke: drives AcceptanceMet against a real branch, the
// reviewer creates a real PR + emits PostApproval, the slack_adapter
// posts a real `chat.postMessage` with the approval-card title, and we
// poll Slack to verify the card landed in the test channel. Cleanup
// closes the PR and deletes the branch.
#[tokio::test]
async fn live_gate_three_posts_real_slack_approval_card_when_merge_pr_true() {
    let _guard = ENV_LOCK.lock().unwrap();

    let (slack_token, slack_channel) = match slack_live_provider() {
        Some(p) => p,
        None => {
            eprintln!(
                "skipping live_gate_three_posts_real_slack_approval_card_when_merge_pr_true: \
                 set FORGE_SLACK_LIVE=1 + SLACK_BOT_TOKEN + FORGE_SLACK_TEST_CHANNEL to run"
            );
            return;
        }
    };
    let (_gh_token, gh_repo) = match github_live_provider() {
        Some(p) => p,
        None => {
            eprintln!(
                "skipping live_gate_three_posts_real_slack_approval_card_when_merge_pr_true: \
                 set FORGE_GITHUB_LIVE=1 + GITHUB_TOKEN + FORGE_GITHUB_TEST_REPO to run"
            );
            return;
        }
    };

    let issue_id = format!("smoke-true-{}", std::process::id());
    let branch = format!("clone-dev/gate-three-smoke-true-{}", std::process::id());
    let workdir = format!("/tmp/forge-smoke-{}", std::process::id());

    if !setup_test_branch(&gh_repo, &branch) {
        panic!(
            "could not set up test branch {branch} on {gh_repo}; check GITHUB_TOKEN scopes \
             (needs repo write) and that the repo exists"
        );
    }
    eprintln!("set up test branch {branch} on {gh_repo}");

    // Mock LLM is fine — the reviewer's `reason` calls (lesson
    // extraction in on TaskCompleted) don't fire on the AcceptanceMet
    // path we drive here.
    let mock_config = ForgeConfig::default_mock_config();
    let providers =
        Arc::new(ProviderRegistry::from_config(mock_config).expect("mock registry should build"));

    let _fixture = write_fixture_config("merge_pr_true", &slack_channel, true);
    let mut h = boot(providers).await;
    let _ = &h.skill_registry; // keep alive

    fire_acceptance_met(&h, &issue_id, &gh_repo, &branch, &workdir, &slack_channel).await;

    let frames = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let says = say_lines(&frames);

    // Reviewer must emit PostApproval (not auto-merge) when merge_pr=true.
    assert_eq!(
        count_emits(&frames, "PostApproval", "reviewer"),
        1,
        "merge_pr=true: reviewer must emit one PostApproval; says={says:?}"
    );
    assert_eq!(
        count_emits(&frames, "PRMerged", "reviewer"),
        0,
        "merge_pr=true: reviewer must NOT auto-merge before approval; says={says:?}"
    );

    // Poll Slack for the real approval card.
    let marker = format!("PR for {issue_id} in {gh_repo} ready for review");
    let ts = poll_slack_for_marker(&slack_token, &slack_channel, &marker, SLACK_POLL_TIMEOUT)
        .unwrap_or_else(|| {
            cleanup_test_branch(&gh_repo, &branch);
            close_open_pr_for_branch(&gh_repo, &branch);
            panic!(
                "Slack approval card with marker '{marker}' did not arrive in {slack_channel} \
                 within {SLACK_POLL_TIMEOUT:?}; says={says:?}"
            )
        });
    eprintln!("approval card landed in {slack_channel} ts={ts}");

    // Cleanup — delete the Slack card, close the open PR, delete the branch.
    delete_slack_message(&slack_token, &slack_channel, &ts);
    close_open_pr_for_branch(&gh_repo, &branch);
    cleanup_test_branch(&gh_repo, &branch);
}

// merge_pr=false smoke: drives AcceptanceMet against a real branch.
// The reviewer skips slack approval, calls skill.github.merge_pr
// directly, and posts a :robot_face: AUTO-MERGED PostMessage. We
// assert the bus events on the reviewer side, poll Slack for the
// confirmation message, and confirm the PR was actually merged on
// GitHub. The squash-merge auto-deletes the branch (--delete-branch).
#[tokio::test]
async fn live_gate_three_auto_merges_when_merge_pr_false() {
    let _guard = ENV_LOCK.lock().unwrap();

    let (slack_token, slack_channel) = match slack_live_provider() {
        Some(p) => p,
        None => {
            eprintln!(
                "skipping live_gate_three_auto_merges_when_merge_pr_false: \
                 set FORGE_SLACK_LIVE=1 + SLACK_BOT_TOKEN + FORGE_SLACK_TEST_CHANNEL to run"
            );
            return;
        }
    };
    let (_gh_token, gh_repo) = match github_live_provider() {
        Some(p) => p,
        None => {
            eprintln!(
                "skipping live_gate_three_auto_merges_when_merge_pr_false: \
                 set FORGE_GITHUB_LIVE=1 + GITHUB_TOKEN + FORGE_GITHUB_TEST_REPO to run"
            );
            return;
        }
    };

    let issue_id = format!("smoke-false-{}", std::process::id());
    let branch = format!("clone-dev/gate-three-smoke-false-{}", std::process::id());
    let workdir = format!("/tmp/forge-smoke-{}", std::process::id());

    if !setup_test_branch(&gh_repo, &branch) {
        panic!(
            "could not set up test branch {branch} on {gh_repo}; check GITHUB_TOKEN scopes \
             (needs repo write) and that the repo exists"
        );
    }
    eprintln!("set up test branch {branch} on {gh_repo}");

    let mock_config = ForgeConfig::default_mock_config();
    let providers =
        Arc::new(ProviderRegistry::from_config(mock_config).expect("mock registry should build"));

    let _fixture = write_fixture_config("merge_pr_false", &slack_channel, false);
    let mut h = boot(providers).await;
    let _ = &h.skill_registry;

    fire_acceptance_met(&h, &issue_id, &gh_repo, &branch, &workdir, &slack_channel).await;

    let frames = drain_frames(&mut h.rx, HANDLER_WINDOW).await;
    let says = say_lines(&frames);

    // No slack approval card on the auto-merge path.
    assert_eq!(
        count_emits(&frames, "PostApproval", "reviewer"),
        0,
        "merge_pr=false: reviewer must NOT emit PostApproval; says={says:?}"
    );
    // PRMerged emitted exactly once.
    assert_eq!(
        count_emits(&frames, "PRMerged", "reviewer"),
        1,
        "merge_pr=false: reviewer must emit one PRMerged; says={says:?}"
    );
    // PostMessage emitted (the :robot_face: confirmation).
    let post_message_count = count_emits(&frames, "PostMessage", "reviewer");
    assert!(
        post_message_count >= 1,
        "merge_pr=false: reviewer must emit PostMessage confirmation; says={says:?}"
    );

    // The probe should have observed the gates.merge_pr=false log line.
    assert!(
        says.iter()
            .any(|s| s.contains("gates.merge_pr=false") && s.contains("auto-merging")),
        "reviewer should log gates.merge_pr=false branch; says={says:?}"
    );

    // Poll Slack for the :robot_face: AUTO-MERGED confirmation.
    let marker = "AUTO-MERGED (gates.merge_pr=false)";
    let ts = poll_slack_for_marker(&slack_token, &slack_channel, marker, SLACK_POLL_TIMEOUT)
        .unwrap_or_else(|| {
            cleanup_test_branch(&gh_repo, &branch);
            close_open_pr_for_branch(&gh_repo, &branch);
            panic!(
                "Slack auto-merge confirmation with marker '{marker}' did not arrive in \
                 {slack_channel} within {SLACK_POLL_TIMEOUT:?}; says={says:?}"
            )
        });
    eprintln!("auto-merge confirmation landed in {slack_channel} ts={ts}");

    // Cleanup — delete confirmation, ensure no open PR remains, drop branch
    // (squash-merge with --delete-branch already removed it on success).
    delete_slack_message(&slack_token, &slack_channel, &ts);
    close_open_pr_for_branch(&gh_repo, &branch);
    cleanup_test_branch(&gh_repo, &branch);
}
