// Live end-to-end regression for #375.
//
// Earlier unit test (`skill_integration_tests.rs::skill_error_routes_through_else_...`)
// proves the behaviour at the TaskExecutor::run() level using a hand-built
// skill. This file goes further: it exercises the fix through the same
// runtime topology that `forge serve` wires up — real `skills/github/SKILL.md`
// loaded from disk, live Tracer broadcast to a mock SSE channel, EventBus,
// SystemRuntime, InstanceRegistry, and the reviewer-flavoured `on AcceptanceMet`
// handler pattern. The `check_ci` capability's `argv` is pinned to `false`
// so the real `SkillExecutor::execute_deterministic` path fails exactly the
// way it does in production when `gh pr checks` returns non-zero.
//
// The assertion that makes this an honest #375 proof is on the
// `HandlerCompleted` frame: the reviewer's handler must complete with
// `status: "success"` and NOT `status: "error"`. Pre-fix, the SkillError
// from `check_ci` was mapped to a handler-level `FlowError` and the frame
// would carry `status: "error"` — the exact symptom reported in the issue.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use forge::runtime::event_bus::{EventBus, EventPayload};
use forge::runtime::executor::TaskExecutor;
use forge::runtime::instance_registry::InstanceRegistry;
use forge::runtime::skill_executor::SkillExecutor;
use forge::runtime::skill_loader::SkillLoader;
use forge::runtime::skill_registry::SkillRegistry;

const REVIEWER_LITE_SRC: &str = r#"#! boundary: server

event AcceptanceMet
  repo: Text
  branch: Text

agent reviewer
  subscribe AcceptanceMet
  on start
    say "[reviewer] ready"
  on AcceptanceMet(repo: Text, branch: Text)
    say "[reviewer] handling"
    ci_result = skill.github.check_ci(repo, branch)
    when ci_result.sure -> say "[reviewer] CI_SURE"
    else -> say "[reviewer] CI_ELSE"
    say "[reviewer] HANDLER_DONE"

warden w
  manages [reviewer]
  on stuck: nudge, self
  on timeout: restart, self
  on crash: restart, self
  on hallucination: nudge, self
  on contradiction: nudge, self
  on budget: nudge, self

system rev_system
  use
    r: reviewer
"#;

fn mock_registry() -> Arc<forge::llm::registry::ProviderRegistry> {
    let config = forge::config::ForgeConfig::default_mock_config();
    Arc::new(
        forge::llm::registry::ProviderRegistry::from_config(config)
            .expect("mock registry should build"),
    )
}

/// Load the real `skills/github/SKILL.md` but pin `check_ci`'s argv to `false`
/// so the deterministic command executor always exits non-zero — the exact
/// failure mode the #375 trace captured in production.
fn load_github_skill_with_failing_check_ci() -> forge::runtime::skill::LoadedSkill {
    let mut skill = SkillLoader::parse_skill_md(Path::new("skills/github/SKILL.md"))
        .expect("skills/github/SKILL.md must parse — repo invariant");

    let mut replaced = false;
    for cap in &mut skill.manifest.capabilities {
        if cap.name == "check_ci" {
            let exec = cap
                .executor
                .as_mut()
                .expect("check_ci must ship with a deterministic executor");
            exec.argv = vec!["false".to_string()];
            replaced = true;
            break;
        }
    }
    assert!(replaced, "real skills/github/SKILL.md must expose check_ci");
    skill
}

#[tokio::test]
async fn reviewer_survives_real_check_ci_failure_live_smoke() {
    // 1. Parse the test program (no checker — runtime dispatch doesn't need it).
    let program = forge::parser::parse(REVIEWER_LITE_SRC).expect("parse reviewer-lite");

    // 2. Build the real skill registry with a pinned-to-fail check_ci.
    let github_skill = load_github_skill_with_failing_check_ci();
    let mut registry = SkillRegistry::new();
    registry.register(github_skill);
    let shared_registry = Arc::new(Mutex::new(registry));

    // 3. Live tracer routed through a broadcast channel — mirrors how
    //    `/__forge/events` feeds SSE subscribers (same shape as #325's live test).
    let (events_tx, mut events_rx) = tokio::sync::broadcast::channel::<String>(1024);
    let tracer = forge::tracer::Tracer::with_live(events_tx.clone());

    let providers = mock_registry();
    let skill_executor = Arc::new(
        SkillExecutor::new(providers.clone(), shared_registry)
            .with_tracer(Arc::new(tracer.clone())),
    );

    // 4. Wire TaskExecutor + EventBus + SystemRuntime + InstanceRegistry exactly
    //    as `forge serve` does (sse_agent_emit_live_test.rs is the canonical
    //    template we follow here).
    let executor = TaskExecutor::new(program, providers, Some(tracer.clone()))
        .with_config(forge::config::ForgeConfig::default_mock_config())
        .with_skill_executor(skill_executor);

    let event_bus = EventBus::new_shared(executor.tracer().cloned());
    let instance_registry = Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));

    let system_runtime = executor
        .build_system_runtime()
        .expect("build system runtime")
        .expect("rev_system should produce a runtime")
        .with_shared_infrastructure(event_bus.clone(), instance_registry.clone());

    tokio::spawn(async move {
        let _ = system_runtime.start().await;
    });

    // Give `on start` time to run + subscriptions to register.
    tokio::time::sleep(Duration::from_millis(400)).await;

    // 5. Publish AcceptanceMet through the bus — same path endpoint emits take.
    let mut fields = std::collections::HashMap::new();
    fields.insert(
        "repo".to_string(),
        forge::runtime::confidence::ConfidentValue::deterministic(
            forge::runtime::confidence::Value::Text("ncmlabs/forge".to_string()),
        ),
    );
    fields.insert(
        "branch".to_string(),
        forge::runtime::confidence::ConfidentValue::deterministic(
            forge::runtime::confidence::Value::Text("feature/test".to_string()),
        ),
    );
    let payload = EventPayload {
        event_name: "AcceptanceMet".to_string(),
        args: vec![],
        source_agent: "test_driver".to_string(),
        fields,
    };
    {
        let bus = event_bus.read().await;
        let delivered = bus.publish(&payload);
        assert!(
            delivered >= 1,
            "reviewer should have subscribed to AcceptanceMet"
        );
    }

    // 6. Drain tracer frames for a generous window — the handler runs async,
    //    and `false` spawns a real child process.
    let mut frames = Vec::<String>::new();
    let deadline = std::time::Instant::now() + Duration::from_millis(3000);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, events_rx.recv()).await {
            Ok(Ok(f)) => frames.push(f),
            Ok(Err(_)) | Err(_) => break,
        }
    }

    let parsed: Vec<serde_json::Value> = frames
        .iter()
        .filter_map(|f| serde_json::from_str(f).ok())
        .collect();

    // Tracer frames are flat JSON objects: {"event": "...", ...other_fields}.

    // 7a. The real skill.github.check_ci call executed and reported failure.
    let check_ci_return = parsed
        .iter()
        .find(|v| v["event"] == "skill_return" && v["skill"] == "github.check_ci")
        .unwrap_or_else(|| {
            panic!(
                "expected a skill_return frame for github.check_ci — skill must actually have run; frames = {:#?}",
                frames
            )
        });
    assert_eq!(
        check_ci_return["success"], false,
        "check_ci must have returned success=false via the `false` exit — frame = {:?}",
        check_ci_return
    );

    // 7b. The else branch of `when ci_result.sure / else` matched — this is
    //     the contract the fix restores. Pre-fix the handler crashed before
    //     the when_dispatch frames were emitted.
    let else_match = parsed
        .iter()
        .find(|v| v["event"] == "when_dispatch" && v["level"] == "else" && v["matched"] == true);
    assert!(
        else_match.is_some(),
        "#375: the else branch must run when skill returns low confidence — frames = {:#?}",
        frames
    );

    // 7c. The reviewer's AcceptanceMet handler completed with status=success.
    //     This is the load-bearing #375 contract: a failing skill must not
    //     bubble up as a handler-level error. Pre-fix, this frame would carry
    //     status="error" (the exact symptom reported in the issue).
    let handler_completed = parsed
        .iter()
        .find(|v| {
            v["event"] == "HandlerCompleted"
                && v["agent"] == "reviewer"
                && v["handler"] == "AcceptanceMet"
        })
        .unwrap_or_else(|| {
            panic!(
                "expected a HandlerCompleted frame for reviewer.AcceptanceMet; frames = {:#?}",
                frames
            )
        });
    assert_eq!(
        handler_completed["status"], "success",
        "#375 fix is load-bearing here: failing skill must not mark handler as error — frame = {:?}",
        handler_completed
    );

    // 7d. Belt-and-braces: no error-status HandlerCompleted frames for reviewer
    //     at all across the run.
    let any_error = parsed.iter().any(|v| {
        v["event"] == "HandlerCompleted" && v["agent"] == "reviewer" && v["status"] == "error"
    });
    assert!(
        !any_error,
        "no reviewer handler should have reported status=error after #375 — frames = {:#?}",
        frames
    );
}
