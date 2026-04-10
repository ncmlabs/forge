// FORGE skill E2E test — issue #131
// Tests the full skill execution pipeline with a mock provider that
// simulates multi-turn tool-use conversations (no network access needed).

use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use forge::llm::providers::mock::MockProvider;
use forge::llm::registry::ProviderRegistry;
use forge::llm::ToolCallRequest;
use forge::runtime::skill_executor::SkillExecutor;
use forge::runtime::skill_loader::SkillLoader;
use forge::runtime::skill_registry::SkillRegistry;
use forge::tracer::Tracer;

// ── Helpers ──────────────────────────────────────────────────────

fn mock_registry(mock: MockProvider) -> Arc<ProviderRegistry> {
    let mut reg = ProviderRegistry::new("mock");
    reg.register("mock", Arc::new(mock));
    Arc::new(reg)
}

fn create_temp_skill(name: &str, description: &str, body: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join(name);
    std::fs::create_dir_all(&skill_dir).unwrap();
    let mut file = std::fs::File::create(skill_dir.join("SKILL.md")).unwrap();
    write!(
        file,
        "---\nname: {}\ndescription: {}\nallowed-tools: Bash\ntimeout: 30\n---\n\n{}",
        name, description, body
    )
    .unwrap();
    dir
}

// ── E2E: full skill execution with mock tool-use ────────────────

#[tokio::test]
async fn skill_e2e_mock_tool_use() {
    // 1. Create a temp SKILL.md
    let dir = create_temp_skill(
        "echo-test",
        "Echo test skill",
        "Run: echo \"hello from skill\"\nReturn the output.",
    );

    // 2. Load and register the skill
    let skills = SkillLoader::load_from_dirs(&[dir.path().to_path_buf()]);
    assert_eq!(skills.len(), 1, "should discover one skill");
    assert_eq!(skills[0].manifest.name, "echo-test");

    let mut registry = SkillRegistry::new();
    for skill in skills {
        registry.register(skill);
    }
    assert!(registry.get("echo-test").is_some());
    let shared_registry = Arc::new(Mutex::new(registry));

    // 3. Configure mock provider for multi-turn tool-use:
    //    Turn 1: return a bash_exec tool call
    //    Turn 2: sequence exhausted → no tool calls → loop exits
    let bash_tool_call = ToolCallRequest {
        id: "call_1".to_string(),
        name: "bash_exec".to_string(),
        arguments: serde_json::json!({"command": "echo hello from skill"}),
    };

    let mock = MockProvider::new("mock")
        .with_responses_sequence(vec![
            String::new(),                  // turn 1: content irrelevant (tool calls present)
            "hello from skill".to_string(), // turn 2: final answer
        ])
        .with_tool_call_sequence(vec![
            vec![bash_tool_call], // turn 1: request bash_exec
                                  // turn 2: exhausted → empty → agentic loop exits
        ]);

    // 4. Build SkillExecutor
    let providers = mock_registry(mock);
    let mut executor = SkillExecutor::new(providers, shared_registry);
    executor.max_turns = 5;
    executor.default_timeout = Duration::from_secs(10);

    // 5. Execute the skill
    let args = HashMap::new();
    let result = executor.execute("echo-test", "invoke", &args).await;

    // 6. Assertions
    assert!(
        result.is_ok(),
        "skill execution should succeed: {:?}",
        result.err()
    );
    let cv = result.unwrap();

    // The final response comes from the mock's text sequence ("hello from skill")
    let output = cv.value.to_string();
    assert!(
        output.contains("hello from skill"),
        "result should contain 'hello from skill', got: {}",
        output
    );

    // Confidence should be capped at skill default (0.99)
    assert!(
        cv.confidence <= 0.99,
        "confidence should be capped at 0.99, got: {}",
        cv.confidence
    );
}

// ── E2E: GitHub skill live integration (requires gh auth) ──────

#[tokio::test]
#[ignore] // Run with: cargo test github_e2e -- --ignored
async fn github_e2e_create_verify_close() {
    // Skip if gh is not authenticated
    let auth = std::process::Command::new("gh")
        .args(["auth", "status"])
        .output();
    match auth {
        Ok(o) if o.status.success() => {}
        _ => {
            eprintln!("Skipping: gh not authenticated");
            return;
        }
    }

    let repo = "ncmlabs/forge";
    let test_title = "[test] Integration test issue — safe to close";
    let test_body = "Automated integration test for GitHub skill (#176). This issue will be closed automatically.";

    // Step 1: Create issue
    let create = std::process::Command::new("gh")
        .args([
            "issue", "create", "-R", repo, "--title", test_title, "--body", test_body,
        ])
        .output()
        .expect("gh issue create failed");
    assert!(
        create.status.success(),
        "create issue failed: {}",
        String::from_utf8_lossy(&create.stderr)
    );
    let issue_url = String::from_utf8_lossy(&create.stdout).trim().to_string();
    assert!(
        issue_url.contains("/issues/"),
        "expected issue URL, got: {}",
        issue_url
    );

    // Extract issue number from URL
    let issue_number = issue_url
        .rsplit('/')
        .next()
        .expect("could not extract issue number");

    // Step 2: Verify issue exists via direct view (avoids search index lag)
    let view = std::process::Command::new("gh")
        .args([
            "issue",
            "view",
            issue_number,
            "-R",
            repo,
            "--json",
            "number,title,state",
        ])
        .output()
        .expect("gh issue view failed");
    assert!(
        view.status.success(),
        "issue {} not found: {}",
        issue_number,
        String::from_utf8_lossy(&view.stderr)
    );
    let view_output = String::from_utf8_lossy(&view.stdout);
    assert!(
        view_output.contains("OPEN"),
        "issue should be open, got: {}",
        view_output
    );

    // Step 3: Close the issue
    let close = std::process::Command::new("gh")
        .args(["issue", "close", issue_number, "-R", repo])
        .output()
        .expect("gh issue close failed");
    assert!(
        close.status.success(),
        "close issue failed: {}",
        String::from_utf8_lossy(&close.stderr)
    );

    // Step 4: Verify closed
    let view = std::process::Command::new("gh")
        .args([
            "issue",
            "view",
            issue_number,
            "-R",
            repo,
            "--json",
            "state",
            "-q",
            ".state",
        ])
        .output()
        .expect("gh issue view failed");
    assert!(view.status.success());
    let state = String::from_utf8_lossy(&view.stdout).trim().to_string();
    assert_eq!(state, "CLOSED", "issue should be closed, got: {}", state);
}

// ── Mock provider: multi-turn sequence drains correctly ─────────

#[tokio::test]
async fn mock_tool_call_sequence_drains() {
    let tool_call = ToolCallRequest {
        id: "call_1".to_string(),
        name: "test_tool".to_string(),
        arguments: serde_json::json!({}),
    };

    let mock = MockProvider::new("mock")
        .with_default("done")
        .with_tool_call_sequence(vec![
            vec![tool_call.clone()], // turn 1
            vec![tool_call.clone()], // turn 2
        ]);

    let providers = mock_registry(mock);
    let _tracer = Tracer::with_capture(); // Principle VIII: trace oracle calls

    // Turn 1: should have tool calls
    let req = forge::llm::CompletionRequest::simple("test");
    let resp = providers.resolve_and_complete(req, None).await.unwrap();
    assert_eq!(resp.tool_calls.len(), 1, "turn 1 should have 1 tool call");

    // Turn 2: should have tool calls
    let req = forge::llm::CompletionRequest::simple("test");
    let resp = providers.resolve_and_complete(req, None).await.unwrap();
    assert_eq!(resp.tool_calls.len(), 1, "turn 2 should have 1 tool call");

    // Turn 3: sequence exhausted, should be empty
    let req = forge::llm::CompletionRequest::simple("test");
    let resp = providers.resolve_and_complete(req, None).await.unwrap();
    assert!(
        resp.tool_calls.is_empty(),
        "turn 3 should have no tool calls (drained)"
    );
}
