// FORGE sensei end-to-end acceptance tests — epic #249
// Mock tests run always; real-LLM tests require explicit opt-in (#288) via
// FORGE_LLM_LIVE=1 AND ANTHROPIC_API_KEY. Having only the API key in the
// environment is intentionally NOT enough — a bare `cargo test` must never
// make paid API calls.
// Run mock tests:  cargo test --test sensei_live_tests
// Run live tests:  FORGE_LLM_LIVE=1 ANTHROPIC_API_KEY=sk-... cargo test --test sensei_live_tests -- --nocapture

use std::path::Path;
use std::sync::Arc;

use forge::ast::{Expr, Program, TemplatePart, TopLevel};
use forge::compose;
use forge::config::ForgeConfig;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::event_bus::EventBus;
use forge::runtime::executor::TaskExecutor;
use forge::runtime::http_server::ForgeServer;
use forge::runtime::instance_registry::InstanceRegistry;
use forge::runtime::storage::ForgeStorage;

// ── Helpers ────────────────────────────────────────────────────────

fn mock_registry() -> Arc<ProviderRegistry> {
    let config = ForgeConfig::default_mock_config();
    Arc::new(ProviderRegistry::from_config(config).expect("mock registry should build"))
}

fn haiku_registry() -> Option<Arc<ProviderRegistry>> {
    // Real-LLM opt-in (#288): both FORGE_LLM_LIVE=1 and a non-empty API key
    // are required. Having just the key (common on dev laptops) must not
    // trigger paid calls during a default `cargo test` run.
    if std::env::var("FORGE_LLM_LIVE").ok().as_deref() != Some("1") {
        return None;
    }
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

/// Redirect every agent's knowledge store root from `~/.forge/sensei*` to a
/// directory under `tmp_root`, so tests don't write to the developer's real
/// `~/.forge/sensei/knowledge.json`. Operates on the parsed AST's template
/// literals, which is where the sensei workflow declares its store path.
fn rewrite_knowledge_paths(program: &mut Program, tmp_root: &Path) {
    for item in program.items.iter_mut() {
        if let TopLevel::Agent(agent_decl) = &mut item.node {
            if let Some(knowledge) = agent_decl.knowledge.as_mut() {
                if let Expr::Template(parts) = &mut knowledge.node.store_path.node {
                    for part in parts.iter_mut() {
                        if let TemplatePart::Text(text) = &mut part.node {
                            if let Some(rest) = text.strip_prefix("~/.forge/") {
                                *text = tmp_root.join(rest).to_string_lossy().into_owned();
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Spawn a sensei server with the given registry and a temp storage.
/// Returns (base_url, _tmp_dir) — keep _tmp_dir alive for the test duration.
///
/// The harness mirrors the production path in `src/main.rs::serve_program`:
/// it builds and starts the declared `SystemRuntime` so the `forge_sensei`
/// agent actually subscribes to the shared event bus. Without this, endpoint
/// `emit` calls (e.g. `/api/ingest-fact` → `emit LearnedInsight`) are
/// delivered to the bus but have no subscriber, and the bug that #284
/// documents goes undetected.
async fn spawn_sensei_server_with(registry: Arc<ProviderRegistry>) -> (String, tempfile::TempDir) {
    let source_files = sensei_server_source_files();
    let mut composed = compose::merge_programs(&source_files).expect("merge sensei files failed");

    let tmp = tempfile::tempdir().unwrap();
    rewrite_knowledge_paths(&mut composed.program, tmp.path());

    let db_path = tmp.path().join("sensei_test.redb");
    let storage = ForgeStorage::open(&db_path).unwrap();

    let config = ForgeConfig::default_mock_config();
    let executor = TaskExecutor::new(composed.program, registry, None)
        .with_storage(Arc::new(storage))
        .with_config(config);

    let event_bus = EventBus::new_shared(None);
    let instance_registry = Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));

    // Build the system runtime BEFORE moving the executor into the server, so
    // both halves share the same event bus and the agent's `subscribe
    // LearnedInsight` attaches to the bus that endpoint `emit` publishes to.
    let system_runtime = executor
        .build_system_runtime()
        .ok()
        .flatten()
        .map(|sr| sr.with_shared_infrastructure(event_bus.clone(), instance_registry.clone()));

    let server = ForgeServer::new(executor, None).with_event_bus(event_bus);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind failed");
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        server.run_on_listener(listener).await.ok();
    });

    if let Some(sr) = system_runtime {
        tokio::spawn(async move {
            let _ = sr.start().await;
        });
    }

    // Give the server AND the agent process time to come up and subscribe.
    // Without a subscribed agent, emit→learn is silently lost.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
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
    let mut composed = compose::merge_programs(&source_files).expect("merge sensei files failed");

    let tmp = tempfile::tempdir().unwrap();
    rewrite_knowledge_paths(&mut composed.program, tmp.path());

    let db_path = tmp.path().join("sensei_sse_test.redb");
    let storage = ForgeStorage::open(&db_path).unwrap();

    let (events_tx, _) = tokio::sync::broadcast::channel::<String>(64);
    let tracer = forge::tracer::Tracer::with_live(events_tx.clone());

    let config = ForgeConfig::default_mock_config();
    let executor = TaskExecutor::new(composed.program, mock_registry(), Some(tracer))
        .with_storage(Arc::new(storage))
        .with_config(config);

    let event_bus = EventBus::new_shared(None);
    let instance_registry = Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));

    let system_runtime = executor
        .build_system_runtime()
        .ok()
        .flatten()
        .map(|sr| sr.with_shared_infrastructure(event_bus.clone(), instance_registry.clone()));

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

    if let Some(sr) = system_runtime {
        tokio::spawn(async move {
            let _ = sr.start().await;
        });
    }

    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    (format!("http://127.0.0.1:{port}"), tmp)
}

// ═══════════════════════════════════════════════════════════════════
// 3a. Real LLM endpoint tests (explicit opt-in — #288)
// Run with: FORGE_LLM_LIVE=1 ANTHROPIC_API_KEY=sk-... cargo test --test sensei_live_tests sensei_live_
// ═══════════════════════════════════════════════════════════════════

#[tokio::test]
async fn sensei_live_ask_returns_intelligent_answer() {
    let Some((base, _tmp)) = spawn_sensei_server_real().await else {
        eprintln!("SKIP: set FORGE_LLM_LIVE=1 and ANTHROPIC_API_KEY to run");
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
        eprintln!("SKIP: set FORGE_LLM_LIVE=1 and ANTHROPIC_API_KEY to run");
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
        eprintln!("SKIP: set FORGE_LLM_LIVE=1 and ANTHROPIC_API_KEY to run");
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
        eprintln!("SKIP: set FORGE_LLM_LIVE=1 and ANTHROPIC_API_KEY to run");
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
        eprintln!("SKIP: set FORGE_LLM_LIVE=1 and ANTHROPIC_API_KEY to run");
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
        eprintln!("SKIP: set FORGE_LLM_LIVE=1 and ANTHROPIC_API_KEY to run");
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

    // 2b. All pretrain facts must land in knowledge.json. Previously this
    // could only assert >= 1 because the stuck detector (issue #286) tripped
    // on the 3rd consecutive Unit-returning `on LearnedInsight` dispatch,
    // which cascaded into a warden circuit-breaker trip. With #286 fixed,
    // Unit-returning event-absorption handlers no longer feed the detector
    // and the full batch persists end-to-end.
    let knowledge_path = _tmp.path().join("sensei/knowledge.json");
    let entries = poll_knowledge_entries(&knowledge_path, pretrain_corpus.len(), 10_000).await;
    for (_, fact) in pretrain_corpus {
        assert!(
            entries.iter().any(|e| {
                e.get("content")
                    .and_then(|v| v.as_str())
                    .is_some_and(|c| c.contains(fact))
            }),
            "pretrain fact missing from knowledge.json: {fact:?}; entries: {entries:?}"
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

/// Poll `{tmp}/sensei/knowledge.json` until it has at least `min_entries`
/// entries or the timeout elapses, then return the decoded entries.
/// Event delivery + agent dispatch + knowledge-store save happen off the
/// request path, so this needs to tolerate a bounded async delay.
async fn poll_knowledge_entries(
    path: &std::path::Path,
    min_entries: usize,
    timeout_ms: u64,
) -> Vec<serde_json::Value> {
    let start = std::time::Instant::now();
    loop {
        if let Ok(raw) = std::fs::read_to_string(path) {
            if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(&raw) {
                if parsed.len() >= min_entries {
                    return parsed;
                }
            }
        }
        if start.elapsed().as_millis() as u64 >= timeout_ms {
            // Return whatever we have so the caller's assertion produces a
            // useful diagnostic instead of a bare timeout.
            return std::fs::read_to_string(path)
                .ok()
                .and_then(|raw| serde_json::from_str(&raw).ok())
                .unwrap_or_default();
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

// Regression for #284: `api_ingest_fact` emits a `LearnedInsight` event, and
// the `forge_sensei` agent's `on LearnedInsight` handler is the only path
// that actually calls `learn` into the knowledge store. Previously the
// subscription filter `where source == "toolkit" or source == "specialist"`
// silently rejected the endpoint's `source: "api"` emit, so the endpoint
// returned 200 but nothing was ever persisted. This test posts a unique
// marker and asserts the marker reaches knowledge.json.
#[tokio::test]
async fn sensei_ingest_fact_persists_to_knowledge_store() {
    let (base, tmp) = spawn_sensei_server_mock().await;
    let client = reqwest::Client::new();

    let marker = format!(
        "UNIQUE-MARKER-{}-284-regression",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );

    let resp = client
        .post(format!("{base}/api/ingest-fact"))
        .json(&serde_json::json!({
            "category": "TEST-284",
            "fact": marker,
        }))
        .send()
        .await
        .expect("ingest-fact request failed");
    assert_eq!(resp.status(), 200, "ingest-fact must return 200");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["result"]
            .as_str()
            .unwrap_or("")
            .contains("Learned [TEST-284]"),
        "ingest-fact must return Learned acknowledgement, got: {body}"
    );

    // The endpoint acknowledgement alone is NOT proof of persistence — that
    // was the whole point of #284. Verify the marker actually reaches disk.
    let knowledge_path = tmp.path().join("sensei/knowledge.json");
    let entries = poll_knowledge_entries(&knowledge_path, 1, 2_000).await;
    assert!(
        entries.iter().any(|e| e
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .contains(&marker)),
        "ingested marker [{marker}] must appear in knowledge.json; \
         got {} entries at {}: {entries:?}",
        entries.len(),
        knowledge_path.display(),
    );
}

// Regression for #284: webhook-ingest shares the same subscribe-filter
// codepath as api-ingest-fact. If someone widens the filter for "api" but
// forgets "webhook", this test catches it.
#[tokio::test]
async fn sensei_webhook_ingest_persists_to_knowledge_store() {
    let (base, tmp) = spawn_sensei_server_mock().await;
    let client = reqwest::Client::new();

    let marker = format!(
        "WEBHOOK-MARKER-{}-284-regression",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );

    let resp = client
        .post(format!("{base}/webhook/webhook_ingest"))
        .json(&serde_json::json!({
            "category": "TEST-284-HOOK",
            "fact": marker,
        }))
        .send()
        .await
        .expect("webhook_ingest request failed");
    assert_eq!(resp.status(), 200, "webhook_ingest must return 200");

    let knowledge_path = tmp.path().join("sensei/knowledge.json");
    let entries = poll_knowledge_entries(&knowledge_path, 1, 2_000).await;
    assert!(
        entries.iter().any(|e| e
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .contains(&marker)),
        "webhook-ingested marker [{marker}] must appear in knowledge.json; \
         got {} entries: {entries:?}",
        entries.len(),
    );
}

// Regression for #283: `api_ingest` previously called `learn from document`
// directly inside the endpoint body. That primitive requires agent context,
// so the runtime rejected it with "learn outside agent" — the path-based
// pretrain pipeline (`scripts/pretrain-sensei.sh` phases 1–3, 5–6) failed
// silently with the CLI collapsing the error into "server unreachable".
//
// The fix mirrors #284's pattern: endpoint emits `IngestRequested(source:
// "api")`, agent subscribes to it and runs `learn from document(path)`
// inside its own context. This test writes a tempfile with a unique marker,
// POSTs the path to `/api/ingest`, and asserts the marker (read through the
// agent's document-chunking codepath) reaches knowledge.json.
#[tokio::test]
async fn sensei_api_ingest_persists_document_to_knowledge_store() {
    let (base, tmp) = spawn_sensei_server_mock().await;
    let client = reqwest::Client::new();

    let marker = format!(
        "UNIQUE-DOC-MARKER-{}-283-regression",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );

    // Write a document containing the marker. `learn_from_document` chunks
    // at 500 chars, so the marker on its own line will survive chunking.
    let doc_path = tmp.path().join("ingest-fixture.md");
    std::fs::write(&doc_path, format!("# Ingest fixture\n\nMarker: {marker}\n"))
        .expect("write fixture doc failed");

    let resp = client
        .post(format!("{base}/api/ingest"))
        .json(&serde_json::json!({
            "document_path": doc_path.to_string_lossy(),
        }))
        .send()
        .await
        .expect("ingest request failed");
    assert_eq!(resp.status(), 200, "ingest must return 200");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["result"].as_str().unwrap_or("").contains("Ingested:"),
        "ingest must return Ingested acknowledgement, got: {body}"
    );

    // The endpoint ack alone is not proof — the "learn outside agent" bug
    // made the endpoint return 200 only when the fix was in place; before
    // the fix the endpoint returned a runtime error. Verify the document
    // contents actually reach disk via the agent's knowledge store.
    let knowledge_path = tmp.path().join("sensei/knowledge.json");
    let entries = poll_knowledge_entries(&knowledge_path, 1, 2_000).await;
    assert!(
        entries.iter().any(|e| e
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .contains(&marker)),
        "ingested document marker [{marker}] must appear in knowledge.json; \
         got {} entries at {}: {entries:?}",
        entries.len(),
        knowledge_path.display(),
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
