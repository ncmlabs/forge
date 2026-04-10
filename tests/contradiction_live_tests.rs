// Live integration tests for contradiction events (issue #205).
//
// These tests exercise the full pipeline: a session driver returns
// an AgentResult claiming files that don't exist → verification engine
// detects contradictions → SessionEvent::ContradictionDetected is emitted
// → session.contradiction arrives on EventBus → ContradictionSummary
// is persisted in session state.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use forge::runtime::agent::AgentSignal;
use forge::runtime::confidence::{ConfidentValue, Value};
use forge::runtime::event_bus::EventBus;
use forge::runtime::session_manager::{
    default_session_dir, NoopSessionController, SessionConfig, SessionDriver, SessionDriverEvent,
    SessionEvent, SessionListener, SessionManager, SessionRuntimeHandle, SessionState,
    SessionStatus,
};
use forge::runtime::verification::{extract_implicit_claims, inject_pending_verification};
use forge::runtime::verification_engine::VerificationEngine;

// ── Test Driver ───────────────────────────────────────────────

struct ScriptedDriver {
    events: Vec<SessionDriverEvent>,
}

#[async_trait]
impl SessionDriver for ScriptedDriver {
    async fn start(
        &self,
        _session_id: &str,
        _config: &SessionConfig,
    ) -> Result<SessionRuntimeHandle, String> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        for event in self.events.clone() {
            tx.send(event).unwrap();
        }
        drop(tx);
        Ok(SessionRuntimeHandle {
            external_session_id: Some("ext-1".to_string()),
            process_id: Some(1234),
            events: rx,
            controller: Arc::new(NoopSessionController),
        })
    }

    async fn resume(&self, _state: &SessionState) -> Result<Option<SessionRuntimeHandle>, String> {
        Ok(None)
    }
}

// ── Helpers ───────────────────────────────────────────────────

fn text_cv(s: &str) -> ConfidentValue {
    ConfidentValue::from_skill(Value::Text(s.to_string()), 0.85)
}

/// Build an AgentResult that claims files_changed with nonexistent files.
/// The verification engine's ReferenceValidator will contradict this.
/// Includes pending verification with extracted claims (simulating what
/// parse_final does in the real adapter path).
fn contradicted_agent_result() -> ConfidentValue {
    let mut fields = ConfidentValue::default_agent_result_fields();
    fields.insert("plan".to_string(), text_cv("implement login fix"));
    fields.insert(
        "confidence".to_string(),
        ConfidentValue::deterministic(Value::Number(0.92)),
    );
    fields.insert(
        "files_changed".to_string(),
        ConfidentValue::deterministic(Value::Array(vec![
            ConfidentValue::deterministic(Value::Text("nonexistent_file.rs".into())),
            ConfidentValue::deterministic(Value::Text("also_missing.rs".into())),
        ])),
    );
    fields.insert(
        "cost_usd".to_string(),
        ConfidentValue::deterministic(Value::Number(0.0)),
    );

    // Inject pending verification with implicit claims — this is what
    // session_adapter::parse_final does for real adapters.
    let claims = extract_implicit_claims(&fields);
    let mut meta = std::collections::HashMap::new();
    inject_pending_verification(&mut meta, claims);
    fields.insert(
        "metadata".to_string(),
        ConfidentValue::deterministic(Value::Record(meta)),
    );

    ConfidentValue::from_agent_result(fields)
}

/// Build an AgentResult where all claimed files actually exist.
fn verified_agent_result(dir: &std::path::Path) -> ConfidentValue {
    // Create the file on disk so verification passes
    std::fs::write(dir.join("real_file.rs"), "fn main() {}").unwrap();

    let mut fields = ConfidentValue::default_agent_result_fields();
    fields.insert("plan".to_string(), text_cv("refactor real_file"));
    fields.insert(
        "confidence".to_string(),
        ConfidentValue::deterministic(Value::Number(0.9)),
    );
    fields.insert(
        "files_changed".to_string(),
        ConfidentValue::deterministic(Value::Array(vec![ConfidentValue::deterministic(
            Value::Text("real_file.rs".into()),
        )])),
    );
    fields.insert(
        "cost_usd".to_string(),
        ConfidentValue::deterministic(Value::Number(0.0)),
    );

    // Inject pending verification with implicit claims
    let claims = extract_implicit_claims(&fields);
    let mut meta = std::collections::HashMap::new();
    inject_pending_verification(&mut meta, claims);
    fields.insert(
        "metadata".to_string(),
        ConfidentValue::deterministic(Value::Record(meta)),
    );

    ConfidentValue::from_agent_result(fields)
}

// ── Live Test: Contradiction detected → event emitted ─────────

#[tokio::test]
async fn contradiction_emits_session_event_on_completion() {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<SessionEvent>();
    let listener: SessionListener = Arc::new(move |event| {
        let _ = tx.send(event);
    });

    let temp = tempfile::tempdir().unwrap();
    // Create .git so EnvironmentValidator doesn't go fatal
    std::fs::create_dir(temp.path().join(".git")).unwrap();

    let mut config = SessionConfig::new("contradiction-test", "fake");
    config.working_dir = Some(temp.path().to_string_lossy().to_string());

    let session_mgr = Arc::new(
        SessionManager::new(default_session_dir(temp.path()))
            .with_verification_engine(VerificationEngine::coding_session()),
    );
    session_mgr.register_driver(
        "fake",
        Arc::new(ScriptedDriver {
            events: vec![SessionDriverEvent::Completed {
                result: contradicted_agent_result(),
            }],
        }),
    );

    let _lid = session_mgr.add_listener(listener);
    let state = session_mgr.run_to_completion(config, vec![]).await.unwrap();

    // Session should complete (not crash)
    assert_eq!(state.status, SessionStatus::Done);

    // ContradictionSummary should be persisted
    let summary = state
        .contradiction_summary
        .as_ref()
        .expect("contradiction_summary should be set");
    assert_eq!(summary.count, 2, "two missing files = two contradictions");
    assert!(summary.high_severity_count > 0);
    assert_eq!(summary.max_severity, "high");
    assert_eq!(summary.verification_status, "contradicted");

    // Check that ContradictionDetected event was emitted
    let mut rx = rx;
    let mut found_contradiction_event = false;
    while let Ok(event) = rx.try_recv() {
        if let SessionEvent::ContradictionDetected {
            contradiction_count,
            high_severity_count,
            max_severity,
            verification_status,
            ..
        } = event
        {
            assert_eq!(contradiction_count, 2);
            assert!(high_severity_count > 0);
            assert_eq!(max_severity, "high");
            assert_eq!(verification_status, "contradicted");
            found_contradiction_event = true;
        }
    }
    assert!(
        found_contradiction_event,
        "expected a ContradictionDetected session event"
    );
}

// ── Live Test: Verified result → no contradiction event ────────

#[tokio::test]
async fn verified_result_emits_no_contradiction_event() {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<SessionEvent>();
    let listener: SessionListener = Arc::new(move |event| {
        let _ = tx.send(event);
    });

    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join(".git")).unwrap();

    let mut config = SessionConfig::new("verified-test", "fake");
    config.working_dir = Some(temp.path().to_string_lossy().to_string());

    let session_mgr = Arc::new(
        SessionManager::new(default_session_dir(temp.path()))
            .with_verification_engine(VerificationEngine::coding_session()),
    );
    session_mgr.register_driver(
        "fake",
        Arc::new(ScriptedDriver {
            events: vec![SessionDriverEvent::Completed {
                result: verified_agent_result(temp.path()),
            }],
        }),
    );

    let _lid = session_mgr.add_listener(listener);
    let state = session_mgr.run_to_completion(config, vec![]).await.unwrap();

    assert_eq!(state.status, SessionStatus::Done);
    assert!(
        state.contradiction_summary.is_none(),
        "verified result should have no contradiction_summary"
    );

    // No ContradictionDetected event
    let mut rx = rx;
    while let Ok(event) = rx.try_recv() {
        if matches!(event, SessionEvent::ContradictionDetected { .. }) {
            panic!("should not emit ContradictionDetected for verified result");
        }
    }
}

// ── Live Test: EventBus receives session.contradiction ─────────

#[tokio::test]
async fn contradiction_publishes_to_event_bus() {
    let bus = EventBus::new_shared(None);
    let mut contradiction_rx =
        bus.write()
            .await
            .subscribe("session.contradiction", "test-listener", None);

    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join(".git")).unwrap();

    let mut config = SessionConfig::new("bus-contradiction", "fake");
    config.working_dir = Some(temp.path().to_string_lossy().to_string());

    let session_mgr = Arc::new(
        SessionManager::new(default_session_dir(temp.path()))
            .with_event_bus(bus.clone())
            .with_verification_engine(VerificationEngine::coding_session()),
    );
    session_mgr.register_driver(
        "fake",
        Arc::new(ScriptedDriver {
            events: vec![SessionDriverEvent::Completed {
                result: contradicted_agent_result(),
            }],
        }),
    );

    let _ = session_mgr.run_to_completion(config, vec![]).await;

    // Verify session.contradiction payload arrived on EventBus
    let payload = tokio::time::timeout(Duration::from_secs(2), contradiction_rx.recv())
        .await
        .expect("timed out waiting for session.contradiction")
        .expect("channel closed");

    assert_eq!(payload.event_name, "session.contradiction");
    assert_eq!(payload.source_agent, "session_manager");
    assert!(payload.fields.contains_key("session_id"));
    assert!(payload.fields.contains_key("contradiction_count"));
    assert!(payload.fields.contains_key("high_severity_count"));
    assert!(payload.fields.contains_key("max_severity"));
    assert!(payload.fields.contains_key("verification_status"));
    assert!(payload.fields.contains_key("risk_class"));

    // Check field values
    if let Value::Number(n) = &payload.fields["contradiction_count"].value {
        assert_eq!(*n as usize, 2);
    } else {
        panic!("contradiction_count should be a number");
    }
}

// ── Live Test: Warden signal channel receives contradiction ────

#[tokio::test]
async fn contradiction_sends_warden_signal() {
    let (signal_tx, mut signal_rx) = tokio::sync::mpsc::channel::<AgentSignal>(64);

    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join(".git")).unwrap();

    let mut config = SessionConfig::new("warden-signal-test", "fake");
    config.working_dir = Some(temp.path().to_string_lossy().to_string());

    let session_mgr = Arc::new(
        SessionManager::new(default_session_dir(temp.path()))
            .with_verification_engine(VerificationEngine::coding_session()),
    );
    session_mgr.set_warden_signal(signal_tx);
    session_mgr.register_driver(
        "fake",
        Arc::new(ScriptedDriver {
            events: vec![SessionDriverEvent::Completed {
                result: contradicted_agent_result(),
            }],
        }),
    );

    let _ = session_mgr.run_to_completion(config, vec![]).await;

    // Warden signal should have been sent
    let signal = tokio::time::timeout(Duration::from_secs(2), signal_rx.recv())
        .await
        .expect("timed out waiting for warden signal")
        .expect("channel closed");

    match signal {
        AgentSignal::Contradiction {
            agent_name,
            detail,
            severity,
        } => {
            assert_eq!(agent_name, "fake");
            assert!(detail.contains("contradiction"), "detail: {}", detail);
            assert_eq!(severity, "high");
        }
        other => panic!("expected Contradiction signal, got {:?}", other),
    }
}

// ── Live Test: Persisted state survives reload ─────────────────

#[tokio::test]
async fn contradiction_summary_survives_session_persistence() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join(".git")).unwrap();

    let mut config = SessionConfig::new("persist-test", "fake");
    config.working_dir = Some(temp.path().to_string_lossy().to_string());

    let session_mgr = Arc::new(
        SessionManager::new(default_session_dir(temp.path()))
            .with_verification_engine(VerificationEngine::coding_session()),
    );
    session_mgr.register_driver(
        "fake",
        Arc::new(ScriptedDriver {
            events: vec![SessionDriverEvent::Completed {
                result: contradicted_agent_result(),
            }],
        }),
    );

    let state = session_mgr.run_to_completion(config, vec![]).await.unwrap();
    let session_id = state.id.clone();
    assert!(state.contradiction_summary.is_some());

    // Create a new SessionManager that reads from the same base_dir
    let reloaded_mgr = Arc::new(SessionManager::new(default_session_dir(temp.path())));
    reloaded_mgr.register_driver("fake", Arc::new(ScriptedDriver { events: vec![] }));
    let _ = reloaded_mgr.resume_all().await;

    // Look up the reloaded session state
    let reloaded_state = reloaded_mgr
        .session_state(&session_id)
        .expect("session should be found after reload");
    assert_eq!(reloaded_state.status, SessionStatus::Done);

    let summary = reloaded_state
        .contradiction_summary
        .as_ref()
        .expect("contradiction_summary should survive persistence");
    assert_eq!(summary.count, 2);
    assert_eq!(summary.max_severity, "high");
    assert_eq!(summary.verification_status, "contradicted");
}
