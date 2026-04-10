use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use forge::llm::providers::mock::MockProvider;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::event_bus::EventBus;
use forge::runtime::executor::TaskExecutor;
use forge::runtime::session_manager::{
    default_session_dir, NoopSessionController, SessionConfig, SessionDriver, SessionDriverEvent,
    SessionEvent, SessionListener, SessionManager, SessionRuntimeHandle, SessionState,
};
use forge::runtime::{confidence::ConfidentValue, confidence::Value};
use forge::tracer::Tracer;

fn mock_registry() -> Arc<ProviderRegistry> {
    let mock = MockProvider::new("mock").with_default("mock");
    let mut reg = ProviderRegistry::new("mock");
    reg.register("mock", Arc::new(mock));
    Arc::new(reg)
}

fn parse_source(src: &str) -> forge::ast::Program {
    forge::parser::parse(src).unwrap_or_else(|e| panic!("parse failed: {:?}", e))
}

fn text_value(s: &str) -> ConfidentValue {
    ConfidentValue::from_skill(Value::Text(s.to_string()), 0.84)
}

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
            external_session_id: Some("external-1".to_string()),
            process_id: Some(42),
            events: rx,
            controller: Arc::new(NoopSessionController),
        })
    }

    async fn resume(&self, _state: &SessionState) -> Result<Option<SessionRuntimeHandle>, String> {
        Ok(None)
    }
}

#[tokio::test]
async fn session_expression_emits_progress_and_completion_hooks() {
    let src = r#"
task review
  gives Text
  do
    result = session "code-review" agent "fake" prompt "check this" on progress -> emit ReviewUpdate(it) on complete -> emit ReviewDone(it)
    give result

fn main
  say review()
"#;
    let program = parse_source(src);
    let tracer = Tracer::with_capture();
    let bus = EventBus::new_shared(None);
    let mut review_update_rx = bus
        .write()
        .await
        .subscribe("ReviewUpdate", "listener-a", None);
    let mut review_done_rx = bus
        .write()
        .await
        .subscribe("ReviewDone", "listener-b", None);

    let temp = tempfile::tempdir().unwrap();
    let session_mgr = Arc::new(
        SessionManager::new(default_session_dir(temp.path())).with_tracer(Some(tracer.clone())),
    );
    session_mgr.register_driver(
        "fake",
        Arc::new(ScriptedDriver {
            events: vec![
                SessionDriverEvent::Progress {
                    payload: text_value("working"),
                    cost_delta_usd: 0.4,
                },
                SessionDriverEvent::Completed {
                    result: text_value("final answer"),
                },
            ],
        }),
    );

    let executor = TaskExecutor::new(program, mock_registry(), Some(tracer.clone()))
        .with_event_bus(bus.clone())
        .with_session_manager(session_mgr);
    executor.run().await.unwrap();

    let outputs = executor.outputs();
    assert!(
        outputs.iter().any(|line| line.contains("final answer")),
        "expected final session text in output, got: {:?}",
        outputs
    );

    let progress = tokio::time::timeout(std::time::Duration::from_secs(2), review_update_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(progress.args[0].value.to_string(), "working");

    let done = tokio::time::timeout(std::time::Duration::from_secs(2), review_done_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.args[0].value.to_string(), "final answer");

    let events = tracer.captured_events();
    assert!(events.contains(&"session_spawned".to_string()));
    assert!(events.contains(&"session_progress".to_string()));
    assert!(events.contains(&"session_completed".to_string()));
}

#[tokio::test]
async fn session_expression_coerces_to_agent_result() {
    let src = r#"
task review
  gives AgentResult
  do
    result = session "code-review" agent "fake" prompt "check this" gives AgentResult
    give result

fn main
  result = review()
  say result.plan
  say "{result.cost_usd}"
"#;
    let program = parse_source(src);
    let temp = tempfile::tempdir().unwrap();
    let session_mgr = Arc::new(SessionManager::new(default_session_dir(temp.path())));
    session_mgr.register_driver(
        "fake",
        Arc::new(ScriptedDriver {
            events: vec![SessionDriverEvent::Completed {
                result: ConfidentValue::from_agent_result(
                    [
                        (
                            "plan".to_string(),
                            ConfidentValue::from_skill(
                                Value::Text("review completed".to_string()),
                                0.9,
                            ),
                        ),
                        (
                            "confidence".to_string(),
                            ConfidentValue::deterministic(Value::Number(0.9)),
                        ),
                        (
                            "cost_usd".to_string(),
                            ConfidentValue::deterministic(Value::Number(0.0)),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                ),
            }],
        }),
    );

    let executor =
        TaskExecutor::new(program, mock_registry(), None).with_session_manager(session_mgr);
    executor.run().await.unwrap();

    let outputs = executor.outputs();
    assert!(
        outputs.iter().any(|line| line.contains("review completed")),
        "expected AgentResult plan in output, got: {:?}",
        outputs
    );
    assert!(
        outputs.iter().any(|line| line.contains("0")),
        "expected AgentResult cost output, got: {:?}",
        outputs
    );
}

// ── Issue #192: enriched events ────────────────────────────────────────────

#[tokio::test]
async fn progress_event_carries_timestamp_and_message() {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<SessionEvent>();
    let listener: SessionListener = Arc::new(move |event| {
        let _ = tx.send(event);
    });

    let temp = tempfile::tempdir().unwrap();
    let session_mgr = Arc::new(SessionManager::new(default_session_dir(temp.path())));
    session_mgr.register_driver(
        "fake",
        Arc::new(ScriptedDriver {
            events: vec![
                SessionDriverEvent::Progress {
                    payload: text_value("step 1 complete"),
                    cost_delta_usd: 0.1,
                },
                SessionDriverEvent::Completed {
                    result: text_value("done"),
                },
            ],
        }),
    );

    let _lid = session_mgr.add_listener(listener);
    let config = SessionConfig::new("test", "fake");
    let _ = session_mgr.run_to_completion(config, vec![]).await;

    let mut rx = rx;
    let mut found_progress = false;
    while let Ok(event) = rx.try_recv() {
        if let SessionEvent::Progress {
            timestamp, message, ..
        } = event
        {
            // timestamp should be recent (within last 10 seconds)
            let age = chrono::Utc::now() - timestamp;
            assert!(age.num_seconds() < 10, "timestamp too old: {:?}", timestamp);
            assert_eq!(message, Some("step 1 complete".to_string()));
            found_progress = true;
        }
    }
    assert!(found_progress, "expected at least one Progress event");
}

#[tokio::test]
async fn completion_event_carries_duration_and_final_status() {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<SessionEvent>();
    let listener: SessionListener = Arc::new(move |event| {
        let _ = tx.send(event);
    });

    let temp = tempfile::tempdir().unwrap();
    let session_mgr = Arc::new(SessionManager::new(default_session_dir(temp.path())));
    session_mgr.register_driver(
        "fake",
        Arc::new(ScriptedDriver {
            events: vec![SessionDriverEvent::Completed {
                result: text_value("all done"),
            }],
        }),
    );

    let _lid = session_mgr.add_listener(listener);
    let config = SessionConfig::new("test", "fake");
    let _ = session_mgr.run_to_completion(config, vec![]).await;

    let mut rx = rx;
    let mut found_completed = false;
    while let Ok(event) = rx.try_recv() {
        if let SessionEvent::Completed {
            duration_secs,
            final_status,
            ..
        } = event
        {
            assert!(
                duration_secs >= 0.0,
                "expected non-negative duration, got {}",
                duration_secs
            );
            assert_eq!(
                final_status,
                forge::runtime::session_manager::SessionStatus::Done
            );
            found_completed = true;
        }
    }
    assert!(found_completed, "expected a Completed event");
}

// ── Issue #192: EventBus integration ───────────────────────────────────────

#[tokio::test]
async fn session_manager_publishes_progress_to_event_bus() {
    let bus = EventBus::new_shared(None);
    let mut progress_rx = bus
        .write()
        .await
        .subscribe("session.progress", "test-listener", None);
    let mut complete_rx = bus
        .write()
        .await
        .subscribe("session.complete", "test-listener", None);

    let temp = tempfile::tempdir().unwrap();
    let session_mgr =
        Arc::new(SessionManager::new(default_session_dir(temp.path())).with_event_bus(bus.clone()));
    session_mgr.register_driver(
        "fake",
        Arc::new(ScriptedDriver {
            events: vec![
                SessionDriverEvent::Progress {
                    payload: text_value("working on it"),
                    cost_delta_usd: 0.2,
                },
                SessionDriverEvent::Completed {
                    result: text_value("finished"),
                },
            ],
        }),
    );

    let config = SessionConfig::new("bus-test", "fake");
    let _ = session_mgr.run_to_completion(config, vec![]).await;

    // Verify progress event arrived on EventBus
    let progress = tokio::time::timeout(Duration::from_secs(2), progress_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(progress.event_name, "session.progress");
    assert_eq!(progress.source_agent, "session_manager");
    assert!(progress.fields.contains_key("session_id"));
    assert!(progress.fields.contains_key("timestamp"));

    // Verify completion event arrived on EventBus
    let complete = tokio::time::timeout(Duration::from_secs(2), complete_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(complete.event_name, "session.complete");
    assert!(complete.fields.contains_key("duration_secs"));
    assert!(complete.fields.contains_key("final_status"));
}

// ── Issue #192: polling ────────────────────────────────────────────────────

/// A driver that delays completion so we can observe poll events.
struct DelayedDriver {
    delay_ms: u64,
}

#[async_trait]
impl SessionDriver for DelayedDriver {
    async fn start(
        &self,
        _session_id: &str,
        _config: &SessionConfig,
    ) -> Result<SessionRuntimeHandle, String> {
        let delay_ms = self.delay_ms;
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            let _ = tx.send(SessionDriverEvent::Completed {
                result: ConfidentValue::deterministic(Value::Text("delayed done".to_string())),
            });
        });
        Ok(SessionRuntimeHandle {
            external_session_id: None,
            process_id: None,
            events: rx,
            controller: Arc::new(NoopSessionController),
        })
    }

    async fn resume(&self, _state: &SessionState) -> Result<Option<SessionRuntimeHandle>, String> {
        Ok(None)
    }
}

#[tokio::test]
async fn polling_emits_session_poll_events() {
    let bus = EventBus::new_shared(None);
    let mut poll_rx = bus
        .write()
        .await
        .subscribe("session.poll", "poll-listener", None);

    let temp = tempfile::tempdir().unwrap();
    let session_mgr =
        Arc::new(SessionManager::new(default_session_dir(temp.path())).with_event_bus(bus.clone()));
    session_mgr.register_driver("slow", Arc::new(DelayedDriver { delay_ms: 500 }));

    // Start polling at 50ms interval
    let _poll_handle = session_mgr.start_polling(Duration::from_millis(50));

    // Spawn a session that takes 500ms
    let config = SessionConfig::new("poll-test", "slow");
    let mgr_clone = session_mgr.clone();
    let session_handle =
        tokio::spawn(async move { mgr_clone.run_to_completion(config, vec![]).await });

    // Wait for at least one poll event
    let poll_event = tokio::time::timeout(Duration::from_secs(2), poll_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(poll_event.event_name, "session.poll");
    assert!(poll_event.fields.contains_key("session_id"));
    assert!(poll_event.fields.contains_key("status"));
    assert!(poll_event.fields.contains_key("cost_usd"));

    // Stop polling and wait for session to finish
    session_mgr.stop_polling();
    let _ = session_handle.await.unwrap();
}

// ── Issue #192: session.status() ───────────────────────────────────────────

#[tokio::test]
async fn session_status_returns_record_with_fields() {
    let src = r#"
task check
  gives Text
  do
    result = session "status-test" agent "fake" prompt "go"
    give result

fn main
  say check()
"#;
    let program = parse_source(src);
    let temp = tempfile::tempdir().unwrap();
    let session_mgr = Arc::new(SessionManager::new(default_session_dir(temp.path())));
    session_mgr.register_driver(
        "fake",
        Arc::new(ScriptedDriver {
            events: vec![SessionDriverEvent::Completed {
                result: text_value("done"),
            }],
        }),
    );

    let executor =
        TaskExecutor::new(program, mock_registry(), None).with_session_manager(session_mgr.clone());
    executor.run().await.unwrap();

    // After session completes, query status directly on the manager
    let sessions: Vec<_> = {
        // Find the session ID from persisted state
        let session_dir = default_session_dir(temp.path());
        std::fs::read_dir(&session_dir)
            .unwrap()
            .filter_map(|e| {
                let e = e.ok()?;
                let name = e.file_name().to_string_lossy().to_string();
                if name.ends_with(".json") {
                    Some(name.trim_end_matches(".json").to_string())
                } else {
                    None
                }
            })
            .collect()
    };
    assert!(
        !sessions.is_empty(),
        "expected at least one persisted session"
    );

    let state = session_mgr.session_state(&sessions[0]).unwrap();
    assert_eq!(state.status.as_str(), "done");
    assert!(state.cost_usd >= 0.0);
}

#[tokio::test]
async fn session_status_expression_in_forge_code() {
    // Test that session.status("nonexistent") returns a fallback text
    let src = r#"
fn main
  status = session.status("no-such-session")
  say status
"#;
    let program = parse_source(src);
    let temp = tempfile::tempdir().unwrap();
    let session_mgr = Arc::new(SessionManager::new(default_session_dir(temp.path())));

    let executor =
        TaskExecutor::new(program, mock_registry(), None).with_session_manager(session_mgr);
    executor.run().await.unwrap();

    let outputs = executor.outputs();
    assert!(
        outputs.iter().any(|line| line.contains("unknown session")),
        "expected 'unknown session' fallback, got: {:?}",
        outputs
    );
}
