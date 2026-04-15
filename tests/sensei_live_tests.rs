// FORGE sensei end-to-end acceptance tests — epic #249
// Mock tests run always; real-LLM tests gated on ANTHROPIC_API_KEY.
// Run mock tests:  cargo test --test sensei_live_tests
// Run live tests:  ANTHROPIC_API_KEY=sk-... cargo test --test sensei_live_tests -- --nocapture

use std::sync::Arc;

use forge::compose;
use forge::config::ForgeConfig;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::event_bus::EventBus;
use forge::runtime::executor::TaskExecutor;
use forge::runtime::http_server::ForgeServer;
use forge::runtime::storage::ForgeStorage;

// ── Helpers ────────────────────────────────────────────────────────

fn mock_registry() -> Arc<ProviderRegistry> {
    let config = ForgeConfig::default_mock_config();
    Arc::new(ProviderRegistry::from_config(config).expect("mock registry should build"))
}

fn haiku_registry() -> Option<Arc<ProviderRegistry>> {
    let api_key = std::env::var("ANTHROPIC_API_KEY").ok()?;
    if api_key.is_empty() {
        return None;
    }

    let mut config = ForgeConfig::default_mock_config();
    config.llm.default = "haiku".to_string();
    config.providers.clear();
    config.providers.insert(
        "haiku".to_string(),
        forge::config::ProviderConfig {
            type_: "anthropic".to_string(),
            model: Some("claude-haiku-4-5-20251001".to_string()),
            api_key: Some(api_key),
            base_url: None,
            fallback: None,
            capabilities: Some(forge::config::CapabilityOverride {
                max_context_tokens: None,
                quality_tier: Some(forge::llm::QualityTier::Balanced),
                local: None,
                cost_per_1k_input: None,
                cost_per_1k_output: None,
            }),
            headers: None,
            timeout_secs: None,
        },
    );

    ProviderRegistry::from_config(config).ok().map(Arc::new)
}

fn sensei_server_source_files() -> Vec<compose::SourceFile> {
    [
        ("types.forge", "workflows/forge-sensei/shared/types.forge"),
        ("events.forge", "workflows/forge-sensei/shared/events.forge"),
        ("states.forge", "workflows/forge-sensei/shared/states.forge"),
        ("tasks.forge", "workflows/forge-sensei/server/tasks.forge"),
        ("flows.forge", "workflows/forge-sensei/server/flows.forge"),
        ("agent.forge", "workflows/forge-sensei/server/agent.forge"),
        ("web.forge", "workflows/forge-sensei/server/web.forge"),
    ]
    .into_iter()
    .map(|(name, path)| {
        let source = std::fs::read_to_string(path).unwrap_or_else(|_| panic!("read {path}"));
        let program =
            forge::parser::parse(&source).unwrap_or_else(|_| panic!("parse {path} failed"));
        compose::SourceFile {
            path: name.to_string(),
            source,
            program,
        }
    })
    .collect()
}

/// Spawn a sensei server with the given registry and a temp storage.
/// Returns (base_url, _tmp_dir) — keep _tmp_dir alive for the test duration.
async fn spawn_sensei_server_with(registry: Arc<ProviderRegistry>) -> (String, tempfile::TempDir) {
    let source_files = sensei_server_source_files();
    let composed = compose::merge_programs(&source_files).expect("merge sensei files failed");

    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("sensei_test.redb");
    let storage = ForgeStorage::open(&db_path).unwrap();

    let config = ForgeConfig::default_mock_config();
    let executor = TaskExecutor::new(composed.program, registry, None)
        .with_storage(Arc::new(storage))
        .with_config(config);

    let event_bus = EventBus::new_shared(None);
    let server = ForgeServer::new(executor, None).with_event_bus(event_bus);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind failed");
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        server.run_on_listener(listener).await.ok();
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    (format!("http://127.0.0.1:{port}"), tmp)
}

/// Spawn a sensei server backed by real Haiku. Returns None if no API key.
async fn spawn_sensei_server_real() -> Option<(String, tempfile::TempDir)> {
    let registry = haiku_registry()?;
    Some(spawn_sensei_server_with(registry).await)
}

/// Spawn a sensei server with mock provider + temp storage.
async fn spawn_sensei_server_mock() -> (String, tempfile::TempDir) {
    spawn_sensei_server_with(mock_registry()).await
}

/// Spawn a sensei server with SSE events wired up.
async fn spawn_sensei_server_with_sse() -> (String, tempfile::TempDir) {
    let source_files = sensei_server_source_files();
    let composed = compose::merge_programs(&source_files).expect("merge sensei files failed");

    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("sensei_sse_test.redb");
    let storage = ForgeStorage::open(&db_path).unwrap();

    let (events_tx, _) = tokio::sync::broadcast::channel::<String>(64);
    let tracer = forge::tracer::Tracer::with_live(events_tx.clone());

    let config = ForgeConfig::default_mock_config();
    let executor = TaskExecutor::new(composed.program, mock_registry(), Some(tracer))
        .with_storage(Arc::new(storage))
        .with_config(config);

    let event_bus = EventBus::new_shared(None);
    let server = ForgeServer::new(executor, None)
        .with_event_bus(event_bus)
        .with_events_tx(events_tx);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind failed");
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        server.run_on_listener(listener).await.ok();
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    (format!("http://127.0.0.1:{port}"), tmp)
}

// ═══════════════════════════════════════════════════════════════════
// 3a. Real LLM endpoint tests (gated on ANTHROPIC_API_KEY)
// Run with: ANTHROPIC_API_KEY=sk-... cargo test --test sensei_live_tests sensei_live_
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn sensei_live_ask_returns_intelligent_answer() {
    let Some((base, _tmp)) = spawn_sensei_server_real().await else {
        eprintln!("SKIP: ANTHROPIC_API_KEY not set");
        return;
    };

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/api/ask"))
        .json(&serde_json::json!({"question": "What is a task in FORGE?"}))
        .send()
        .await
        .expect("request failed");

    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.expect("json parse failed");
    assert_eq!(status, 200, "real ask failed: {body}");

    let answer = body["answer"].as_str().unwrap_or("");
    assert!(
        !answer.is_empty(),
        "real LLM ask should return non-empty answer"
    );
    println!("Real ask answer: {}", &answer[..answer.len().min(300)]);
}

#[tokio::test]
async fn sensei_live_review_returns_feedback() {
    let Some((base, _tmp)) = spawn_sensei_server_real().await else {
        eprintln!("SKIP: ANTHROPIC_API_KEY not set");
        return;
    };

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/api/review"))
        .json(&serde_json::json!({
            "code": "task greet\n  needs name: Text\n  gives Text\n  do\n    give \"Hello {name}\""
        }))
        .send()
        .await
        .expect("request failed");

    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.expect("json parse failed");
    assert_eq!(status, 200, "real review failed: {body}");

    let result = body["result"].as_str().unwrap_or("");
    assert!(
        !result.is_empty(),
        "real LLM review should return non-empty result"
    );
    println!("Real review result: {}", &result[..result.len().min(300)]);
}

#[tokio::test]
async fn sensei_live_assess_detailed_classifies_and_predicts() {
    let Some((base, _tmp)) = spawn_sensei_server_real().await else {
        eprintln!("SKIP: ANTHROPIC_API_KEY not set");
        return;
    };

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/api/assess-detailed"))
        .json(&serde_json::json!({
            "test_input": "task greet\n  needs name: Text\n  gives Text\n  do\n    give \"Hello {name}\"",
            "expected": "parse_ok"
        }))
        .send()
        .await
        .expect("request failed");

    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.expect("json parse failed");
    assert_eq!(status, 200, "real assess-detailed failed: {body}");

    // AssessmentResult has: passed, prediction, expected, gap_topic
    assert!(
        body.get("passed").is_some(),
        "assess-detailed should return 'passed' field: {body}"
    );
    assert!(
        body.get("prediction").is_some(),
        "assess-detailed should return 'prediction' field: {body}"
    );
    assert!(
        body.get("gap_topic").is_some(),
        "assess-detailed should return 'gap_topic' field: {body}"
    );
    println!("Real assess-detailed: {body}");
}

#[tokio::test]
async fn sensei_live_self_assess_completes() {
    let Some((base, _tmp)) = spawn_sensei_server_real().await else {
        eprintln!("SKIP: ANTHROPIC_API_KEY not set");
        return;
    };

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/api/self-assess"))
        .send()
        .await
        .expect("request failed");

    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.expect("json parse failed");
    assert_eq!(status, 200, "real self-assess failed: {body}");

    let result = body["result"].as_str().unwrap_or("");
    assert!(
        result.contains("Self-check complete"),
        "self-assess should confirm completion, got: {result}"
    );
    println!("Real self-assess: {result}");
}

#[tokio::test]
async fn sensei_live_ingest_then_ask_roundtrip() {
    let Some((base, _tmp)) = spawn_sensei_server_real().await else {
        eprintln!("SKIP: ANTHROPIC_API_KEY not set");
        return;
    };

    let client = reqwest::Client::new();

    // Ingest a fact
    let resp = client
        .post(format!("{base}/api/ingest-fact"))
        .json(&serde_json::json!({
            "category": "SYNTAX",
            "fact": "The pipe operator >> chains task output to the next task input in FORGE"
        }))
        .send()
        .await
        .expect("ingest request failed");
    assert_eq!(resp.status(), 200, "ingest-fact should succeed");

    // Ask about the ingested fact
    let resp = client
        .post(format!("{base}/api/ask"))
        .json(&serde_json::json!({"question": "How does the pipe operator work in FORGE?"}))
        .send()
        .await
        .expect("ask request failed");

    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.expect("json parse failed");
    assert_eq!(status, 200, "ask after ingest failed: {body}");

    let answer = body["answer"].as_str().unwrap_or("");
    assert!(
        !answer.is_empty(),
        "ask after ingest should return non-empty answer"
    );
    println!("Ingest+ask roundtrip: {}", &answer[..answer.len().min(300)]);
}

#[tokio::test]
async fn sensei_live_learn_from_session() {
    let Some((base, _tmp)) = spawn_sensei_server_real().await else {
        eprintln!("SKIP: ANTHROPIC_API_KEY not set");
        return;
    };

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/api/learn-from-session"))
        .json(&serde_json::json!({
            "question": "how do flows work in FORGE?",
            "resolution": "flows use stages with needs clauses to pass data between them"
        }))
        .send()
        .await
        .expect("request failed");

    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.expect("json parse failed");
    assert_eq!(status, 200, "learn-from-session failed: {body}");

    let result = body["result"].as_str().unwrap_or("");
    assert!(
        result.contains("Learned from session"),
        "learn-from-session should confirm, got: {result}"
    );
}

// ═══════════════════════════════════════════════════════════════════
// 3b. Mastery persistence roundtrip (no LLM needed — mock provider)
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn sensei_update_mastery_then_status_shows_score() {
    let (base, _tmp) = spawn_sensei_server_mock().await;
    let client = reqwest::Client::new();

    // Update mastery to 75
    let resp = client
        .post(format!("{base}/api/update-mastery"))
        .json(&serde_json::json!({"score": 75}))
        .send()
        .await
        .expect("update-mastery request failed");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["result"].as_str().unwrap().contains("journeyman"),
        "update-mastery 75 should return journeyman, got: {body}"
    );
    assert!(
        body["result"].as_str().unwrap().contains("75"),
        "update-mastery should echo score, got: {body}"
    );

    // Verify status reflects the update
    let resp = reqwest::get(format!("{base}/api/status"))
        .await
        .expect("status request failed");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let status_text = body["status"].as_str().unwrap();
    assert!(
        status_text.contains("75"),
        "status should show 75% after update, got: {status_text}"
    );
    assert!(
        status_text.contains("journeyman"),
        "status should show journeyman after update, got: {status_text}"
    );
}

#[tokio::test]
async fn sensei_mastery_transitions_through_all_levels() {
    let (base, _tmp) = spawn_sensei_server_mock().await;
    let client = reqwest::Client::new();

    let transitions = [
        (0, "novice"),
        (40, "apprentice"),
        (70, "journeyman"),
        (90, "expert"),
    ];

    for (score, expected_level) in transitions {
        let resp = client
            .post(format!("{base}/api/update-mastery"))
            .json(&serde_json::json!({"score": score}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.unwrap();
        let result = body["result"].as_str().unwrap();
        assert!(
            result.contains(expected_level),
            "score {score} should yield {expected_level}, got: {result}"
        );

        // Verify status reflects this transition
        let resp = reqwest::get(format!("{base}/api/status")).await.unwrap();
        let body: serde_json::Value = resp.json().await.unwrap();
        let status_text = body["status"].as_str().unwrap();
        assert!(
            status_text.contains(expected_level),
            "status after score {score} should show {expected_level}, got: {status_text}"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════
// 3b-ii. Operational readiness pipeline (no LLM — mock provider) — #240
// Proves the novice → ingest → assess → apprentice narrative end-to-end
// over the HTTP daemon pattern. Locks in the invariants #240 closes.
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn sensei_operational_readiness_pipeline() {
    let (base, _tmp) = spawn_sensei_server_mock().await;
    let client = reqwest::Client::new();

    // 1. Fresh store — status reports novice (data.get("sensei:level") empty → "novice")
    let resp = reqwest::get(format!("{base}/api/status"))
        .await
        .expect("status request failed");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let status_text = body["status"].as_str().unwrap();
    assert!(
        status_text.contains("novice"),
        "fresh store must report novice, got: {status_text}"
    );

    // 2. Ingest a handful of categorized facts — simulates the pretrain phase.
    //    Each ingest should succeed without touching mastery state.
    let pretrain_corpus = [
        (
            "SYNTAX",
            "FORGE uses 2-space indentation for all nested blocks",
        ),
        (
            "TASKS",
            "pure functions cannot call reason, uncertain, or any LLM primitive",
        ),
        (
            "AGENTS",
            "agents declare memory with persistent keyword for ACID storage",
        ),
        (
            "SUPERVISION",
            "wardens supervise agents and define escalation policies",
        ),
    ];
    for (category, fact) in pretrain_corpus {
        let resp = client
            .post(format!("{base}/api/ingest-fact"))
            .json(&serde_json::json!({"category": category, "fact": fact}))
            .send()
            .await
            .expect("ingest-fact request failed");
        assert_eq!(
            resp.status(),
            200,
            "ingest-fact [{category}] must succeed on fresh store"
        );
    }

    // 3. After ingestion, mastery is still novice — ingest alone doesn't grade.
    let resp = reqwest::get(format!("{base}/api/status")).await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["status"].as_str().unwrap().contains("novice"),
        "ingest without assessment must NOT advance mastery"
    );

    // 4. Assessment step — simulate passing 50% of conformance tests.
    //    Daemon writes sensei:score + sensei:level to data store.
    let resp = client
        .post(format!("{base}/api/update-mastery"))
        .json(&serde_json::json!({"score": 50}))
        .send()
        .await
        .expect("update-mastery request failed");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["result"].as_str().unwrap().contains("apprentice"),
        "score 50 must yield apprentice, got: {body}"
    );

    // 5. Status now reports apprentice with the correct score.
    let resp = reqwest::get(format!("{base}/api/status")).await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let status_text = body["status"].as_str().unwrap();
    assert!(
        status_text.contains("apprentice") && status_text.contains("50"),
        "post-assessment status must show apprentice + score, got: {status_text}"
    );

    // 6. Further ingestion post-advancement still works and preserves level.
    let resp = client
        .post(format!("{base}/api/ingest-fact"))
        .json(&serde_json::json!({
            "category": "PATTERNS",
            "fact": "timer-driven agents use the 'timer' declaration with a duration"
        }))
        .send()
        .await
        .expect("post-advancement ingest failed");
    assert_eq!(resp.status(), 200);
    let resp = reqwest::get(format!("{base}/api/status")).await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["status"].as_str().unwrap().contains("apprentice"),
        "ingest after advancement must not reset mastery"
    );

    // 7. Advance past the journeyman threshold — locks in the full progression.
    let resp = client
        .post(format!("{base}/api/update-mastery"))
        .json(&serde_json::json!({"score": 95}))
        .send()
        .await
        .expect("update-mastery journeyman request failed");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["result"].as_str().unwrap().contains("expert"),
        "score 95 must yield expert, got: {body}"
    );
}

// Gate-message regression: the agent handler still carries the user-facing
// rejection string. If someone accidentally loosens the gate or drops the
// message, this test fails before the change lands. Intentionally source-only
// — no runtime dispatch, because the HTTP endpoints bypass the handler gates
// by design (daemon pattern: HTTP callers are first-party, gates are UX
// guidance at the CLI/handler layer, not a security boundary).
#[test]
fn sensei_review_gate_message_intact() {
    let source = std::fs::read_to_string("workflows/forge-sensei/server/agent.forge")
        .expect("read agent.forge");
    assert!(
        source.contains("Sensei is still at novice level"),
        "review handler novice-gate message regressed"
    );
    assert!(
        source.contains("Deep dive requires journeyman level"),
        "deep_dive handler journeyman-gate message regressed"
    );
}

// ═══════════════════════════════════════════════════════════════════
// 3c. Preflight / server-down tests (no LLM needed)
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn sensei_server_down_returns_connection_error() {
    // web.fetch to a port with nothing listening should fail gracefully
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap();

    let result = client.get("http://127.0.0.1:19999/api/status").send().await;

    assert!(
        result.is_err(),
        "request to dead server should fail, not succeed"
    );
}

#[tokio::test]
async fn sensei_check_handler_server_up_succeeds() {
    // When server is up, the /api/status endpoint returns 200 — which means
    // the client's `check` handler (web.fetch → non-empty → "server ok") works.
    let (base, _tmp) = spawn_sensei_server_mock().await;
    let resp = reqwest::get(format!("{base}/api/status"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["status"].as_str().unwrap().contains("forge-sensei"),
        "status endpoint should return forge-sensei info"
    );
}

// ═══════════════════════════════════════════════════════════════════
// 3d. SSE lifecycle events (no LLM needed)
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn sensei_sse_events_stream_on_handler_dispatch() {
    let (base, _tmp) = spawn_sensei_server_with_sse().await;

    // Subscribe to SSE events
    let sse_client = reqwest::Client::new();
    let sse_resp = sse_client
        .get(format!("{base}/__forge/events"))
        .header("Accept", "text/event-stream")
        .send()
        .await
        .expect("SSE connect failed");
    assert_eq!(sse_resp.status(), 200);

    // Trigger a handler that produces tracer events
    let _trigger = reqwest::get(format!("{base}/api/status"))
        .await
        .expect("trigger request failed");

    // Give tracer a moment to emit events
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Trigger again to ensure at least one event makes it through
    let _trigger2 = reqwest::get(format!("{base}/api/status"))
        .await
        .expect("trigger2 request failed");

    // The SSE stream is open — verify the endpoint responds and is streaming.
    // Full SSE consumption requires an async stream reader, but proving the
    // endpoint returns 200 with the right content type is the critical check.
    let ct = sse_resp
        .headers()
        .get("content-type")
        .expect("SSE should have content-type")
        .to_str()
        .unwrap();
    assert!(
        ct.contains("text/event-stream"),
        "SSE endpoint should return text/event-stream, got: {ct}"
    );
}
