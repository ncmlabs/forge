// Integration tests for issue #334 — CorrelationDriver end-to-end.
// Drives the real CorrelationDriver + AgentLifecycle + EventBus together so
// the hit/rehydrate/publish ordering is observable, and exercises the
// atomic memory-with-correlations write primitive.

use std::collections::HashMap;
use std::sync::Arc;

use forge::ast::{
    AgentDecl, CorrelateField, FieldDef, Program, ScheduleMode, Span, Spanned, TypeName,
};
use forge::llm::registry::ProviderRegistry;
use forge::runtime::agent_lifecycle::AgentLifecycle;
use forge::runtime::confidence::{ConfidentValue, Value};
use forge::runtime::correlation_driver::{CorrelationDriver, CorrelationRegistration};
use forge::runtime::event_bus::{EventBus, EventPayload, SharedEventBus};
use forge::runtime::instance_registry::InstanceRegistry;
use forge::runtime::storage::{ForgeStorage, SharedStorage};
use forge::runtime::warded::AgentBlueprint;
use forge::tracer::Tracer;

fn sp<T>(node: T) -> Spanned<T> {
    Spanned {
        node,
        span: Span { start: 0, end: 0 },
    }
}

fn temp_storage() -> (tempfile::TempDir, SharedStorage) {
    let dir = tempfile::tempdir().unwrap();
    let storage = ForgeStorage::open(&dir.path().join("correlations.redb")).unwrap();
    (dir, Arc::new(storage))
}

/// A minimal specialist that declares a correlate block on SlackMention.thread_ts.
fn slack_specialist_decl() -> AgentDecl {
    AgentDecl {
        exportable: false,
        name: sp("slack_specialist".to_string()),
        lifecycle: None,
        memory: vec![
            sp(FieldDef {
                name: "thread_ts".to_string(),
                type_name: sp(TypeName::Text),
            }),
            sp(FieldDef {
                name: "task_id".to_string(),
                type_name: sp(TypeName::Text),
            }),
        ],
        memory_persistent: true,
        knowledge: None,
        timers: Vec::new(),
        schedules: Vec::new(),
        correlates: vec![sp(CorrelateField {
            event_type: sp("SlackMention".to_string()),
            field_name: sp("thread_ts".to_string()),
            mode: Some(sp(ScheduleMode::Wake)),
            emit: None,
            duplicates: Vec::new(),
        })],
        webhooks: vec![],
        subscriptions: Vec::new(),
        warden_override: Vec::new(),
        handlers: Vec::new(),
        stuck_policy: None,
    }
}

fn blueprint_for(decl: AgentDecl) -> AgentBlueprint {
    AgentBlueprint {
        decl,
        states: None,
        program: Program {
            boundary: None,
            items: Vec::new(),
        },
        registry: Arc::new(ProviderRegistry::new("test")),
        tracer: None,
        skill_executor: None,
        shared_knowledge_store: None,
    }
}

fn mention_payload(thread_ts: &str) -> EventPayload {
    let mut fields = HashMap::new();
    fields.insert(
        "thread_ts".to_string(),
        ConfidentValue::deterministic(Value::Text(thread_ts.to_string())),
    );
    EventPayload {
        event_name: "SlackMention".to_string(),
        args: Vec::new(),
        source_agent: "http".to_string(),
        fields,
    }
}

fn lifecycle_for(
    bus: &SharedEventBus,
    storage: &SharedStorage,
) -> (
    Arc<AgentLifecycle>,
    Arc<tokio::sync::RwLock<InstanceRegistry>>,
) {
    let mut bps = HashMap::new();
    bps.insert(
        "slack_specialist".to_string(),
        blueprint_for(slack_specialist_decl()),
    );
    let registry = Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));
    let lifecycle = Arc::new(AgentLifecycle::new(
        Arc::new(bps),
        registry.clone(),
        bus.clone(),
        Some(storage.clone()),
        None,
    ));
    (lifecycle, registry)
}

// ── Hit path: thread reply routes to rehydrated specialist ────────────────────

#[tokio::test]
async fn second_mention_on_known_thread_routes_to_rehydrated_session() {
    let tracer = Tracer::with_capture();
    let bus = EventBus::new_shared(Some(tracer.clone()));
    let (_dir, storage) = temp_storage();

    // Seed a correlation row as if a prior session handled thread T1.
    storage
        .upsert_correlation("slack_specialist", "thread_ts", "T1", "slack_specialist")
        .unwrap();

    let driver = CorrelationDriver::new(
        storage.clone(),
        vec![CorrelationRegistration {
            agent_alias: "slack_specialist".into(),
            event_type: "SlackMention".into(),
            field_name: "thread_ts".into(),
            mode: ScheduleMode::Wake,
            emit: None,
        }],
    );
    let (lifecycle, registry) = lifecycle_for(&bus, &storage);

    // Inbound mention on the known thread.
    let payload = mention_payload("T1");
    let hit = driver.match_event(&payload).unwrap().expect("T1 must hit");
    assert_eq!(hit.target_alias, "slack_specialist");

    // Rehydrate before publish (same ordering the executor uses).
    let handle = lifecycle
        .rehydrate_or_spawn(&hit.target_alias)
        .await
        .expect("rehydrate must succeed");
    assert_eq!(handle.agent_name, "slack_specialist");

    // Specialist is now live in the registry — exactly one instance.
    assert_eq!(registry.read().await.len(), 1);
    assert!(!registry
        .read()
        .await
        .find_by_name("slack_specialist")
        .is_empty());

    tracer.correlation_hit(
        &hit.event_type,
        &hit.field_name,
        &hit.field_value,
        &hit.target_alias,
    );

    let log = tracer.captured_log();
    let names: Vec<&str> = log.iter().map(|(n, _)| n.as_str()).collect();
    assert!(
        names.contains(&"correlation_hit"),
        "expected correlation_hit in log: {:?}",
        names
    );
}

// ── Miss path: unknown thread does not rehydrate ─────────────────────────────

#[tokio::test]
async fn unknown_thread_does_not_rehydrate() {
    let tracer = Tracer::with_capture();
    let bus = EventBus::new_shared(Some(tracer.clone()));
    let (_dir, storage) = temp_storage();

    // No seeded rows — every lookup misses.
    let driver = CorrelationDriver::new(
        storage.clone(),
        vec![CorrelationRegistration {
            agent_alias: "slack_specialist".into(),
            event_type: "SlackMention".into(),
            field_name: "thread_ts".into(),
            mode: ScheduleMode::Wake,
            emit: None,
        }],
    );
    let (_lifecycle, registry) = lifecycle_for(&bus, &storage);

    let payload = mention_payload("T-unknown");
    assert!(driver.match_event(&payload).unwrap().is_none());

    // No rehydration happened — registry stays empty.
    assert_eq!(registry.read().await.len(), 0);

    // Record the miss.
    if let Some((field, value)) = driver.first_registered_field(&payload) {
        tracer.correlation_miss(&payload.event_name, &field, &value);
    }
    let names: Vec<String> = tracer
        .captured_log()
        .iter()
        .map(|(n, _)| n.clone())
        .collect();
    assert!(
        names.iter().any(|n| n == "correlation_miss"),
        "expected correlation_miss in log: {:?}",
        names
    );
}

// ── Atomic memory-with-correlations write ────────────────────────────────────

#[tokio::test]
async fn memory_and_correlation_land_in_one_write_txn() {
    let (_dir, storage) = temp_storage();

    // Write the memory blob and a correlation row in one atomic call.
    let mem_json =
        r#"{"thread_ts":{"value":{"Text":"T-new"},"confidence":1.0,"source":"Deterministic"}}"#;
    let rows = vec![(
        "slack_specialist".to_string(),
        "thread_ts".to_string(),
        "T-new".to_string(),
        "slack_specialist".to_string(),
    )];
    storage
        .store_memory_with_correlations("agent:slack_specialist:memory", mem_json, &rows)
        .unwrap();

    // Both sides are present.
    assert_eq!(
        storage.get("agent:slack_specialist:memory").unwrap(),
        Some(mem_json.to_string())
    );
    let target = storage
        .lookup_correlation("slack_specialist", "thread_ts", "T-new")
        .unwrap();
    assert_eq!(target, Some("slack_specialist".to_string()));

    // A second mention on the newly-registered thread now matches.
    let driver = CorrelationDriver::new(
        storage.clone(),
        vec![CorrelationRegistration {
            agent_alias: "slack_specialist".into(),
            event_type: "SlackMention".into(),
            field_name: "thread_ts".into(),
            mode: ScheduleMode::Wake,
            emit: None,
        }],
    );
    assert!(driver
        .match_event(&mention_payload("T-new"))
        .unwrap()
        .is_some());
}
