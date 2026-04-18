// Integration tests for issue #332 — WakeService MVP.
// These tests run the full WakeService::run loop under Tokio's paused time,
// so virtual scheduling-cadence advances deterministically. For the wall-
// clock oracle we use `MockClock` (and one pass with `RecordedClock` to prove
// replay determinism).

use std::sync::Arc;
use std::time::Duration as StdDuration;

use forge::ast::{
    Duration as AstDuration, DurationUnit, ScheduleField, ScheduleMode, Span, Spanned, WhenExpr,
};
use forge::runtime::clock::{MockClock, RecordedClock, SharedClock};
use forge::runtime::event_bus::{EventBus, EventPayload, SharedEventBus};
use forge::runtime::storage::{ForgeStorage, ScheduleStatus, SharedStorage};
use forge::runtime::wake_service::{
    BudgetQuery, FireOutcome, ScheduleRegistration, WakeService, WakeServiceConfig,
};
use forge::tracer::Tracer;

fn sp<T>(node: T) -> Spanned<T> {
    Spanned {
        node,
        span: Span { start: 0, end: 0 },
    }
}

fn every_schedule(name: &str, secs: u64) -> ScheduleField {
    ScheduleField {
        name: sp(name.to_string()),
        when: Some(sp(WhenExpr::Every(AstDuration {
            value: secs,
            unit: DurationUnit::Seconds,
        }))),
        mode: Some(sp(ScheduleMode::Spawn)),
        prompt: None,
        emit: None,
        precision: None,
        duplicates: Vec::new(),
    }
}

fn noop_budget() -> BudgetQuery {
    Arc::new(|_| None)
}

fn temp_storage() -> (tempfile::TempDir, SharedStorage) {
    let dir = tempfile::tempdir().unwrap();
    let storage = ForgeStorage::open(&dir.path().join("schedules.redb")).unwrap();
    (dir, Arc::new(storage))
}

async fn subscribe(
    bus: &SharedEventBus,
    event: &str,
    agent: &str,
) -> tokio::sync::mpsc::Receiver<EventPayload> {
    bus.write().await.subscribe(event, agent, None)
}

fn fast_config() -> WakeServiceConfig {
    WakeServiceConfig {
        instance_id: "integration-instance".into(),
        tick_interval: StdDuration::from_millis(20),
        claim_ttl: StdDuration::from_secs(60),
        catchup_stagger: StdDuration::from_millis(1),
        max_consecutive_errors: 3,
    }
}

// ── `every 30s` under MockClock fires at expected intervals ─────────────────

#[tokio::test]
async fn every_schedule_fires_at_expected_intervals_under_mock_clock() {
    let clock = MockClock::new(0);
    let regs = vec![ScheduleRegistration {
        agent: "probe".into(),
        schedule: every_schedule("tick", 30),
    }];
    let tracer = Tracer::with_capture();
    let bus = EventBus::new_shared(Some(tracer.clone()));
    let (_dir, storage) = temp_storage();
    let mut rx = subscribe(&bus, "tick", "probe").await;

    let service = WakeService::new(
        regs.clone(),
        bus.clone(),
        storage.clone(),
        Arc::new(clock.clone()) as SharedClock,
        tracer.clone(),
        noop_budget(),
        fast_config(),
    );
    service.reconcile().unwrap();

    // Simulate three consecutive 30-second ticks by advancing the mock clock
    // and sweeping. We drive sweeps directly rather than via run() so the
    // test is synchronous about when "now" advances.
    for i in 1..=3u64 {
        clock.set(i * 30_000);
        let outcome = service.fire_once(&regs[0]).await.unwrap();
        assert!(
            matches!(outcome, FireOutcome::Delivered { .. }),
            "tick {i}: unexpected outcome: {outcome:?}"
        );
    }

    let mut delivered = Vec::new();
    while let Ok(p) = rx.try_recv() {
        delivered.push(p.event_name);
    }
    assert_eq!(delivered.len(), 3);

    let fired_events = tracer
        .captured_events()
        .into_iter()
        .filter(|e| e == "schedule_fired")
        .count();
    assert_eq!(fired_events, 3);

    let state = storage
        .get_schedule_state("probe", "tick")
        .unwrap()
        .unwrap();
    assert_eq!(state.last_status, ScheduleStatus::Success);
    assert_eq!(state.last_run_at_ms, Some(90_000));
}

// ── Crash/restart → catchup fires exactly once (policy `once`) ──────────────

#[tokio::test]
async fn restart_fires_catchup_exactly_once() {
    let (dir, _) = temp_storage();
    let db_path = dir.path().join("persist.redb");

    // First "process": reconcile, then imagine it crashes at t=0 without ever
    // firing. The schedule is due every hour; we jump to t=5h later — the
    // declared policy is `once` so only one catchup fire should land.
    let storage1 = Arc::new(ForgeStorage::open(&db_path).unwrap());
    let tracer1 = Tracer::with_capture();
    let bus1 = EventBus::new_shared(Some(tracer1.clone()));
    let regs = vec![ScheduleRegistration {
        agent: "probe".into(),
        schedule: every_schedule("hourly", 3600),
    }];
    let clock1 = MockClock::new(0);
    let service1 = WakeService::new(
        regs.clone(),
        bus1.clone(),
        storage1.clone(),
        Arc::new(clock1) as SharedClock,
        tracer1,
        noop_budget(),
        fast_config(),
    );
    service1.reconcile().unwrap();
    drop(service1);
    drop(storage1);
    // (simulate crash — process exits, redb file remains)

    // Second "process": reopens the same file, several hours late. Catchup
    // fires one event, no matter how many cycles were missed.
    let storage2 = Arc::new(ForgeStorage::open(&db_path).unwrap());
    let tracer2 = Tracer::with_capture();
    let bus2 = EventBus::new_shared(Some(tracer2.clone()));
    let mut rx = subscribe(&bus2, "hourly", "probe").await;
    let clock2 = MockClock::new(5 * 3600 * 1000); // t = 5h
    let service2 = WakeService::new(
        regs.clone(),
        bus2.clone(),
        storage2.clone(),
        Arc::new(clock2) as SharedClock,
        tracer2.clone(),
        noop_budget(),
        fast_config(),
    );
    service2.reconcile().unwrap();
    service2.catchup().await.unwrap();

    let mut fires = 0;
    while rx.try_recv().is_ok() {
        fires += 1;
    }
    assert_eq!(fires, 1, "catchup policy `once`: exactly one fire");

    let fired = tracer2
        .captured_events()
        .into_iter()
        .filter(|e| e == "schedule_fired")
        .count();
    assert_eq!(fired, 1);
}

// ── RecordedClock replay: identical firing timestamps + bus ordering ────────
//
// Writer pass: run with MockClock, capture the wall-time samples + bus order.
// Replay pass: run with RecordedClock over the captured samples, assert the
// tracer produced the same fire-event sequence at the same scheduled times.
// This is the OpenClaw-impossible guarantee: replay is deterministic because
// wall-clock is an oracle on the determinism boundary (Principle II).

/// Wraps a `Clock` and records every `now_ms()` call, so a test can replay
/// the exact oracle sequence the writer consumed.
struct CapturingClock {
    inner: Arc<dyn forge::runtime::clock::Clock>,
    log: std::sync::Mutex<Vec<u64>>,
}

impl CapturingClock {
    fn new(inner: Arc<dyn forge::runtime::clock::Clock>) -> Self {
        Self {
            inner,
            log: std::sync::Mutex::new(Vec::new()),
        }
    }
    fn snapshot(&self) -> Vec<u64> {
        self.log.lock().unwrap().clone()
    }
}

impl forge::runtime::clock::Clock for CapturingClock {
    fn now_ms(&self) -> u64 {
        let v = self.inner.now_ms();
        self.log.lock().unwrap().push(v);
        v
    }
}

#[tokio::test]
async fn recorded_clock_replay_is_deterministic() {
    let regs = vec![ScheduleRegistration {
        agent: "probe".into(),
        schedule: every_schedule("tick", 60),
    }];

    // ── Writer pass ─────────────────────────────────────────
    let mock = MockClock::new(0);
    let capturing = Arc::new(CapturingClock::new(
        Arc::new(mock.clone()) as Arc<dyn forge::runtime::clock::Clock>
    ));
    let tracer_w = Tracer::with_capture();
    let bus_w = EventBus::new_shared(Some(tracer_w.clone()));
    let (_dw, storage_w) = temp_storage();
    let _rx_w = subscribe(&bus_w, "tick", "probe").await;
    let service_w = WakeService::new(
        regs.clone(),
        bus_w.clone(),
        storage_w.clone(),
        capturing.clone() as SharedClock,
        tracer_w.clone(),
        noop_budget(),
        fast_config(),
    );
    service_w.reconcile().unwrap();

    for i in 1..=4u64 {
        mock.set(i * 60_000);
        service_w.fire_once(&regs[0]).await.unwrap();
    }
    let writer_events = tracer_w
        .captured_log()
        .into_iter()
        .filter(|(name, _)| name == "schedule_fired")
        .map(|(_, payload)| payload["scheduled_at_ms"].as_u64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(writer_events.len(), 4);

    // Capture the exact clock-read sequence for replay.
    let samples = capturing.snapshot();
    assert!(
        !samples.is_empty(),
        "writer must have read the clock at least once"
    );

    // ── Replay pass ─────────────────────────────────────────
    let clock_r: SharedClock = Arc::new(RecordedClock::new(samples));
    let tracer_r = Tracer::with_capture();
    let bus_r = EventBus::new_shared(Some(tracer_r.clone()));
    let (_dr, storage_r) = temp_storage();
    let _rx_r = subscribe(&bus_r, "tick", "probe").await;
    let service_r = WakeService::new(
        regs.clone(),
        bus_r.clone(),
        storage_r.clone(),
        clock_r,
        tracer_r.clone(),
        noop_budget(),
        fast_config(),
    );
    service_r.reconcile().unwrap();
    for _ in 0..4 {
        service_r.fire_once(&regs[0]).await.unwrap();
    }
    let replay_events = tracer_r
        .captured_log()
        .into_iter()
        .filter(|(name, _)| name == "schedule_fired")
        .map(|(_, payload)| payload["scheduled_at_ms"].as_u64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        writer_events, replay_events,
        "replay produced different scheduled_at_ms sequence than writer"
    );
}

// ── Within-process multi-dispatcher: exactly one fires per tick ─────────────
//
// Two concurrent `WakeService` instances sharing the same redb file (i.e. the
// in-process analogue of the "two forge serve" acceptance criterion — which
// is physically impossible at the redb layer, see storage.rs notes). This
// test proves the transactional-claim mechanism serializes concurrent
// dispatchers so only one of them wins per scheduled tick.

#[tokio::test]
async fn concurrent_dispatchers_fire_exactly_once_per_tick() {
    let (_dir, storage) = temp_storage();
    let regs = vec![ScheduleRegistration {
        agent: "probe".into(),
        schedule: every_schedule("tick", 30),
    }];
    let clock = MockClock::new(0);

    // Two dispatchers, two instance_ids, same storage/bus/clock.
    let tracer = Tracer::with_capture();
    let bus = EventBus::new_shared(Some(tracer.clone()));
    let mut rx = subscribe(&bus, "tick", "probe").await;

    let make_service = |id: &str| {
        WakeService::new(
            regs.clone(),
            bus.clone(),
            storage.clone(),
            Arc::new(clock.clone()) as SharedClock,
            tracer.clone(),
            noop_budget(),
            WakeServiceConfig {
                instance_id: id.into(),
                ..fast_config()
            },
        )
    };
    let svc_a = make_service("A");
    let svc_b = make_service("B");
    svc_a.reconcile().unwrap();

    clock.set(60_000);
    // Race: both dispatchers sweep concurrently. Both see the schedule as due,
    // both call fire_once → both enter try_claim_schedule. redb's write-txn
    // serialization gives us exactly one Claimed outcome for that tick;
    // the loser's sweep completes with a ClaimLost tracer event and no
    // bus delivery. After A's fire advances next_run_at to 90_000s,
    // B's sweep sees the schedule as not due and simply returns.
    let (ra, rb) = tokio::join!(svc_a.sweep(), svc_b.sweep());
    ra.unwrap();
    rb.unwrap();

    // Exactly one bus delivery for the 60_000 ms scheduled moment.
    let mut count = 0;
    while rx.try_recv().is_ok() {
        count += 1;
    }
    assert_eq!(count, 1, "exactly one delivery for scheduled moment 60_000");

    // And exactly one schedule_fired tracer event.
    let fires = tracer
        .captured_events()
        .into_iter()
        .filter(|e| e == "schedule_fired")
        .count();
    assert_eq!(fires, 1);
}

// ── Service run loop: spawn + shutdown is clean ─────────────────────────────

#[tokio::test(start_paused = true)]
async fn wake_service_run_loop_drains_on_shutdown() {
    let (_dir, storage) = temp_storage();
    let clock = MockClock::new(0);
    let regs = vec![ScheduleRegistration {
        agent: "probe".into(),
        schedule: every_schedule("tick", 30),
    }];
    let tracer = Tracer::with_capture();
    let bus = EventBus::new_shared(Some(tracer.clone()));
    let _rx = subscribe(&bus, "tick", "probe").await;

    let service = WakeService::new(
        regs,
        bus,
        storage,
        Arc::new(clock.clone()) as SharedClock,
        tracer,
        noop_budget(),
        fast_config(),
    );
    let handle = service.spawn();

    // Let the tick loop run briefly — tokio's virtual time advances
    // fast enough under start_paused.
    tokio::time::sleep(StdDuration::from_millis(100)).await;

    handle.shutdown();
    let res = tokio::time::timeout(StdDuration::from_secs(2), handle.join).await;
    assert!(res.is_ok(), "WakeService should drain after shutdown()");
}

// ── `mode: wake` dispatch (#333) ────────────────────────────────────────────
//
// These tests exercise the rehydrate-before-publish contract: WakeService
// must drive `AgentLifecycle::rehydrate_or_spawn` to restore `memory
// persistent` and re-subscribe the agent to the bus BEFORE it publishes the
// wake event. The tracer order is `schedule_fired → schedule_rehydrated →
// event_delivered` — load-bearing for replay (Principle II).

use forge::ast::{AgentDecl, FieldDef, Program, TypeName};
use forge::llm::registry::ProviderRegistry;
use forge::runtime::agent_lifecycle::AgentLifecycle;
use forge::runtime::instance_registry::InstanceRegistry;
use forge::runtime::warded::AgentBlueprint;

fn wake_schedule(name: &str, secs: u64, emit: Option<&str>) -> ScheduleField {
    ScheduleField {
        name: sp(name.to_string()),
        when: Some(sp(WhenExpr::Every(AstDuration {
            value: secs,
            unit: DurationUnit::Seconds,
        }))),
        mode: Some(sp(ScheduleMode::Wake)),
        prompt: None,
        emit: emit.map(|e| sp(e.to_string())),
        precision: None,
        duplicates: Vec::new(),
    }
}

fn decl_with_schedule(name: &str, schedule: ScheduleField) -> AgentDecl {
    AgentDecl {
        exportable: false,
        name: sp(name.to_string()),
        lifecycle: None,
        memory: vec![
            sp(FieldDef {
                name: "last_level".to_string(),
                type_name: sp(TypeName::Text),
            }),
            sp(FieldDef {
                name: "review_count".to_string(),
                type_name: sp(TypeName::Number),
            }),
        ],
        memory_persistent: true,
        knowledge: None,
        timers: Vec::new(),
        schedules: vec![sp(schedule)],
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

// Persist a memory JSON blob the way the executor's persistent-write path
// does it, so `AgentProcess::new` will restore it via `restore_from_json`.
fn persist_memory(storage: &SharedStorage, agent_name: &str, review_count: f64, last_level: &str) {
    let blob = serde_json::json!({
        "review_count": {
            "value": { "Number": review_count },
            "confidence": 1.0,
            "provenance": [],
        },
        "last_level": {
            "value": { "Text": last_level },
            "confidence": 1.0,
            "provenance": [],
        },
    });
    storage
        .store(&format!("agent:{}:memory", agent_name), &blob.to_string())
        .unwrap();
}

#[tokio::test]
async fn wake_fires_rehydrated_before_published_with_memory_keys_in_tracer() {
    let clock = MockClock::new(0);
    let tracer = Tracer::with_capture();
    let bus = EventBus::new_shared(Some(tracer.clone()));
    let (_dir, storage) = temp_storage();

    // Persist memory from a previous "run" so rehydration observably restores it.
    persist_memory(&storage, "drift_watcher", 5.0, "journeyman");

    // Wire the lifecycle with a blueprint keyed by alias. WakeService's
    // `agent` field is the alias, so we match that here.
    let mut bps = std::collections::HashMap::new();
    bps.insert(
        "drift_watcher".to_string(),
        blueprint_for(decl_with_schedule(
            "drift_watcher",
            wake_schedule("drift_check", 6 * 3600, Some("DriftCheckDue")),
        )),
    );
    let registry = Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));
    let lifecycle = Arc::new(AgentLifecycle::new(
        Arc::new(bps),
        registry.clone(),
        bus.clone(),
        Some(storage.clone()),
        None,
    ));

    // Subscribe a listener to the emit event so we see it arrive AFTER
    // rehydration. If publish happened first, the subscriber would receive
    // it before we see `schedule_rehydrated` in the tracer log.
    let mut rx = subscribe(&bus, "DriftCheckDue", "listener").await;

    let regs = vec![ScheduleRegistration {
        agent: "drift_watcher".into(),
        schedule: wake_schedule("drift_check", 6 * 3600, Some("DriftCheckDue")),
    }];
    let service = WakeService::new(
        regs.clone(),
        bus.clone(),
        storage.clone(),
        Arc::new(clock.clone()) as SharedClock,
        tracer.clone(),
        noop_budget(),
        fast_config(),
    )
    .with_lifecycle(lifecycle.clone());

    service.reconcile().unwrap();
    clock.set(6 * 3600 * 1000);
    let outcome = service.fire_once(&regs[0]).await.unwrap();
    assert!(matches!(outcome, FireOutcome::Delivered { .. }));

    // Wake event delivered.
    let payload = rx.try_recv().expect("DriftCheckDue must be delivered");
    assert_eq!(payload.event_name, "DriftCheckDue");

    // Tracer ordering: fired → rehydrated → event_delivered/event_emit.
    let log = tracer.captured_log();
    let names: Vec<&str> = log.iter().map(|(n, _)| n.as_str()).collect();
    let fired_idx = names
        .iter()
        .position(|n| *n == "schedule_fired")
        .expect("schedule_fired must be emitted");
    let rehydrated_idx = names
        .iter()
        .position(|n| *n == "schedule_rehydrated")
        .expect("schedule_rehydrated must be emitted");
    let delivered_idx = names
        .iter()
        .position(|n| *n == "event_delivered" || *n == "event_emit")
        .expect("bus must trace delivery");
    assert!(
        fired_idx < rehydrated_idx,
        "expected schedule_fired before schedule_rehydrated, got names={:?}",
        names
    );
    assert!(
        rehydrated_idx < delivered_idx,
        "expected schedule_rehydrated before event delivery, got names={:?}",
        names
    );

    // `memory_keys_restored` payload reports the declared fields that were
    // present in redb (alphabetical order per the helper).
    let rehydrated_payload = &log[rehydrated_idx].1;
    let restored = rehydrated_payload["memory_keys_restored"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(restored, vec!["last_level", "review_count"]);

    // The rehydrated agent is now live in the registry under its decl name.
    let live = registry.read().await.find_by_name("drift_watcher");
    assert_eq!(live.len(), 1);
}

#[tokio::test]
async fn wake_without_emit_falls_back_to_schedule_tick_event() {
    let clock = MockClock::new(0);
    let tracer = Tracer::with_capture();
    let bus = EventBus::new_shared(Some(tracer.clone()));
    let (_dir, storage) = temp_storage();

    let mut bps = std::collections::HashMap::new();
    bps.insert(
        "beat_watcher".to_string(),
        blueprint_for(decl_with_schedule(
            "beat_watcher",
            wake_schedule("heartbeat", 30, None),
        )),
    );
    let registry = Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));
    let lifecycle = Arc::new(AgentLifecycle::new(
        Arc::new(bps),
        registry,
        bus.clone(),
        Some(storage.clone()),
        None,
    ));

    let mut rx = subscribe(&bus, "heartbeat.tick", "listener").await;

    let regs = vec![ScheduleRegistration {
        agent: "beat_watcher".into(),
        schedule: wake_schedule("heartbeat", 30, None),
    }];
    let service = WakeService::new(
        regs.clone(),
        bus.clone(),
        storage.clone(),
        Arc::new(clock.clone()) as SharedClock,
        tracer.clone(),
        noop_budget(),
        fast_config(),
    )
    .with_lifecycle(lifecycle);

    service.reconcile().unwrap();
    clock.set(60_000);
    let outcome = service.fire_once(&regs[0]).await.unwrap();
    assert!(matches!(outcome, FireOutcome::Delivered { .. }));

    let payload = rx.try_recv().expect("heartbeat.tick must be delivered");
    assert_eq!(payload.event_name, "heartbeat.tick");
}

#[tokio::test]
async fn wake_without_lifecycle_traces_rehydrate_failure_and_counts_as_error() {
    let clock = MockClock::new(0);
    let tracer = Tracer::with_capture();
    let bus = EventBus::new_shared(Some(tracer.clone()));
    let (_dir, storage) = temp_storage();

    let regs = vec![ScheduleRegistration {
        agent: "orphan".into(),
        schedule: wake_schedule("check", 30, Some("CheckDue")),
    }];
    // Intentionally NO `with_lifecycle`.
    let service = WakeService::new(
        regs.clone(),
        bus.clone(),
        storage.clone(),
        Arc::new(clock.clone()) as SharedClock,
        tracer.clone(),
        noop_budget(),
        fast_config(),
    );

    service.reconcile().unwrap();
    clock.set(60_000);
    let outcome = service.fire_once(&regs[0]).await.unwrap();
    assert_eq!(outcome, FireOutcome::NoSubscribers);

    let names: Vec<String> = tracer.captured_log().into_iter().map(|(n, _)| n).collect();
    assert!(
        names.iter().any(|n| n == "session_rehydrate_failed"),
        "expected session_rehydrate_failed, got {:?}",
        names
    );
    // `schedule_fired` is still emitted (the schedule fired; delivery just
    // failed because there's no lifecycle to rehydrate through).
    assert!(names.iter().any(|n| n == "schedule_fired"));
}

#[tokio::test]
async fn wake_fires_twice_reuses_same_rehydrated_instance() {
    let clock = MockClock::new(0);
    let tracer = Tracer::with_capture();
    let bus = EventBus::new_shared(Some(tracer.clone()));
    let (_dir, storage) = temp_storage();

    let mut bps = std::collections::HashMap::new();
    bps.insert(
        "probe".to_string(),
        blueprint_for(decl_with_schedule(
            "probe",
            wake_schedule("tick", 30, Some("Tick")),
        )),
    );
    let registry = Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));
    let lifecycle = Arc::new(AgentLifecycle::new(
        Arc::new(bps),
        registry.clone(),
        bus.clone(),
        Some(storage.clone()),
        None,
    ));

    let _rx = subscribe(&bus, "Tick", "listener").await;

    let regs = vec![ScheduleRegistration {
        agent: "probe".into(),
        schedule: wake_schedule("tick", 30, Some("Tick")),
    }];
    let service = WakeService::new(
        regs.clone(),
        bus.clone(),
        storage.clone(),
        Arc::new(clock.clone()) as SharedClock,
        tracer.clone(),
        noop_budget(),
        fast_config(),
    )
    .with_lifecycle(lifecycle);

    service.reconcile().unwrap();
    for tick in 1..=2u64 {
        clock.set(tick * 60_000);
        let _ = service.fire_once(&regs[0]).await.unwrap();
    }

    // Only ONE live instance after two fires — the second fire reuses the
    // first rehydrated agent (Principle I: no duplicate sessions).
    assert_eq!(registry.read().await.find_by_name("probe").len(), 1);

    // `schedule_rehydrated` fires on each wake dispatch. The second should
    // carry an empty `memory_keys_restored` because the agent was already
    // live (no new restore happened).
    let rehydrated_payloads: Vec<_> = tracer
        .captured_log()
        .into_iter()
        .filter(|(n, _)| n == "schedule_rehydrated")
        .map(|(_, p)| p)
        .collect();
    assert_eq!(rehydrated_payloads.len(), 2);
    let second_keys = rehydrated_payloads[1]["memory_keys_restored"]
        .as_array()
        .unwrap();
    assert!(
        second_keys.is_empty(),
        "second rehydrate should report no keys since agent was already live"
    );
}

#[tokio::test]
async fn wake_replay_with_recorded_clock_is_deterministic() {
    use forge::runtime::clock::Clock as ClockTrait;

    // Writer pass — MockClock + CapturingClock.
    let mock = MockClock::new(0);
    let capturing = Arc::new(CapturingClock::new(
        Arc::new(mock.clone()) as Arc<dyn ClockTrait>
    ));
    let tracer_w = Tracer::with_capture();
    let bus_w = EventBus::new_shared(Some(tracer_w.clone()));
    let (_dw, storage_w) = temp_storage();
    persist_memory(&storage_w, "probe", 7.0, "expert");
    let mut bps_w = std::collections::HashMap::new();
    bps_w.insert(
        "probe".to_string(),
        blueprint_for(decl_with_schedule(
            "probe",
            wake_schedule("tick", 60, Some("Tick")),
        )),
    );
    let registry_w = Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));
    let lifecycle_w = Arc::new(AgentLifecycle::new(
        Arc::new(bps_w),
        registry_w,
        bus_w.clone(),
        Some(storage_w.clone()),
        None,
    ));
    let _rx_w = subscribe(&bus_w, "Tick", "listener").await;
    let regs = vec![ScheduleRegistration {
        agent: "probe".into(),
        schedule: wake_schedule("tick", 60, Some("Tick")),
    }];
    let service_w = WakeService::new(
        regs.clone(),
        bus_w.clone(),
        storage_w.clone(),
        capturing.clone() as SharedClock,
        tracer_w.clone(),
        noop_budget(),
        fast_config(),
    )
    .with_lifecycle(lifecycle_w);
    service_w.reconcile().unwrap();
    for i in 1..=3u64 {
        mock.set(i * 60_000);
        service_w.fire_once(&regs[0]).await.unwrap();
    }

    let writer_sequence: Vec<String> = tracer_w
        .captured_log()
        .into_iter()
        .map(|(n, _)| n)
        .filter(|n| n == "schedule_fired" || n == "schedule_rehydrated" || n == "event_emit")
        .collect();
    let samples = capturing.snapshot();
    assert!(!samples.is_empty());

    // Replay pass — RecordedClock.
    let clock_r: SharedClock = Arc::new(RecordedClock::new(samples));
    let tracer_r = Tracer::with_capture();
    let bus_r = EventBus::new_shared(Some(tracer_r.clone()));
    let (_dr, storage_r) = temp_storage();
    persist_memory(&storage_r, "probe", 7.0, "expert");
    let mut bps_r = std::collections::HashMap::new();
    bps_r.insert(
        "probe".to_string(),
        blueprint_for(decl_with_schedule(
            "probe",
            wake_schedule("tick", 60, Some("Tick")),
        )),
    );
    let registry_r = Arc::new(tokio::sync::RwLock::new(InstanceRegistry::new()));
    let lifecycle_r = Arc::new(AgentLifecycle::new(
        Arc::new(bps_r),
        registry_r,
        bus_r.clone(),
        Some(storage_r.clone()),
        None,
    ));
    let _rx_r = subscribe(&bus_r, "Tick", "listener").await;
    let service_r = WakeService::new(
        regs.clone(),
        bus_r.clone(),
        storage_r.clone(),
        clock_r,
        tracer_r.clone(),
        noop_budget(),
        fast_config(),
    )
    .with_lifecycle(lifecycle_r);
    service_r.reconcile().unwrap();
    for _ in 0..3 {
        service_r.fire_once(&regs[0]).await.unwrap();
    }

    let replay_sequence: Vec<String> = tracer_r
        .captured_log()
        .into_iter()
        .map(|(n, _)| n)
        .filter(|n| n == "schedule_fired" || n == "schedule_rehydrated" || n == "event_emit")
        .collect();

    assert_eq!(
        writer_sequence, replay_sequence,
        "wake-mode replay must produce identical tracer sequence"
    );
}
