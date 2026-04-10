use std::sync::Arc;

use async_trait::async_trait;
use forge::llm::providers::mock::MockProvider;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::event_bus::EventBus;
use forge::runtime::executor::TaskExecutor;
use forge::runtime::session_manager::{
    default_session_dir, NoopSessionController, SessionConfig, SessionDriver, SessionDriverEvent,
    SessionManager, SessionRuntimeHandle, SessionState,
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
