// FORGE timer engine tests — issue #20
// Tests for async countdown timers on agents.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use tokio::sync::mpsc;
use tokio::time;

use forge::ast::*;
use forge::llm::providers::mock::MockProvider;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::agent::*;
use forge::runtime::confidence::{ConfidentValue, Value};
use forge::runtime::timer_engine::{TimerEngine, TimerFired};

// ── Helpers ─────────────────────────────────────────────────────────────────

fn spanned<T>(node: T) -> Spanned<T> {
    Spanned::new(node, Span { start: 0, end: 0 })
}

fn empty_program() -> Program {
    Program {
        boundary: None,
        items: vec![],
    }
}

fn mock_registry() -> Arc<ProviderRegistry> {
    let mock = MockProvider::new("mock").with_default("mock response");
    let mut reg = ProviderRegistry::new("mock");
    reg.register("mock", Arc::new(mock));
    Arc::new(reg)
}

fn timer_field(name: &str, secs: u64) -> Spanned<TimerField> {
    spanned(TimerField {
        name: spanned(name.to_string()),
        duration: spanned(Duration {
            value: secs,
            unit: DurationUnit::Seconds,
        }),
    })
}

fn simple_agent_with_timers(
    timers: Vec<Spanned<TimerField>>,
    handlers: Vec<Spanned<OnHandler>>,
) -> AgentDecl {
    AgentDecl {
        exportable: false,
        name: spanned("test_agent".into()),
        lifecycle: None,
        memory: vec![],
        memory_persistent: false,
        knowledge: None,
        timers,
        schedules: vec![],
        correlates: vec![],
        webhooks: vec![],
        subscriptions: vec![],
        handlers,
        warden_override: Vec::new(),
        stuck_policy: None,
    }
}

/// Receive a TimerFired event, expecting it to arrive after sleep auto-advance.
async fn recv_fired(rx: &mut mpsc::Receiver<TimerFired>) -> TimerFired {
    rx.try_recv().expect("expected TimerFired event")
}

/// Assert no TimerFired event is pending.
async fn assert_no_fire(rx: &mut mpsc::Receiver<TimerFired>) {
    // Yield to let any pending tasks complete
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    assert!(rx.try_recv().is_err(), "expected no timer fire");
}

// ── Unit tests: TimerEngine directly ────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn timer_fires_after_duration() {
    let fields = vec![timer_field("timeout", 5)];
    let (fire_tx, mut fire_rx) = mpsc::channel::<TimerFired>(64);
    let mut engine = TimerEngine::new("agent1", &fields, fire_tx, None);

    engine.start("timeout", None).unwrap();
    time::sleep(StdDuration::from_secs(6)).await;

    let fired = recv_fired(&mut fire_rx).await;
    assert_eq!(fired.timer_name, "timeout");
    assert_eq!(fired.agent_id, "agent1");
    assert!(fired.context.is_none());
}

#[tokio::test(start_paused = true)]
async fn timer_cancel_prevents_firing() {
    let fields = vec![timer_field("timeout", 5)];
    let (fire_tx, mut fire_rx) = mpsc::channel::<TimerFired>(64);
    let mut engine = TimerEngine::new("agent1", &fields, fire_tx, None);

    engine.start("timeout", None).unwrap();
    time::sleep(StdDuration::from_secs(2)).await;
    engine.cancel("timeout", &None).unwrap();
    time::sleep(StdDuration::from_secs(10)).await;

    assert_no_fire(&mut fire_rx).await;
}

#[tokio::test(start_paused = true)]
async fn timer_reset_restarts_countdown() {
    let fields = vec![timer_field("timeout", 5)];
    let (fire_tx, mut fire_rx) = mpsc::channel::<TimerFired>(64);
    let mut engine = TimerEngine::new("agent1", &fields, fire_tx, None);

    engine.start("timeout", None).unwrap();
    // Advance 3s (not enough to fire the 5s timer)
    time::sleep(StdDuration::from_secs(3)).await;
    assert_no_fire(&mut fire_rx).await;

    // Reset — cancels old, starts new 5s countdown
    engine.reset("timeout", None).unwrap();

    // Advance 4s — still not enough since reset restarted the countdown
    time::sleep(StdDuration::from_secs(4)).await;
    assert_no_fire(&mut fire_rx).await;

    // Advance 2 more seconds — total 6s since reset, should fire
    time::sleep(StdDuration::from_secs(2)).await;

    let fired = recv_fired(&mut fire_rx).await;
    assert_eq!(fired.timer_name, "timeout");
}

#[tokio::test(start_paused = true)]
async fn timer_with_context_delivers_context() {
    let fields = vec![timer_field("reconnect", 3)];
    let (fire_tx, mut fire_rx) = mpsc::channel::<TimerFired>(64);
    let mut engine = TimerEngine::new("agent1", &fields, fire_tx, None);

    let ctx = ConfidentValue::deterministic(Value::Text("player1".into()));
    engine.start("reconnect", Some(ctx)).unwrap();
    time::sleep(StdDuration::from_secs(4)).await;

    let fired = recv_fired(&mut fire_rx).await;
    assert_eq!(fired.timer_name, "reconnect");
    match fired.context {
        Some(cv) => assert!(matches!(cv.value, Value::Text(ref s) if s == "player1")),
        None => panic!("expected context value"),
    }
}

#[tokio::test(start_paused = true)]
async fn multiple_concurrent_timers_independent() {
    let fields = vec![timer_field("short", 3), timer_field("long", 10)];
    let (fire_tx, mut fire_rx) = mpsc::channel::<TimerFired>(64);
    let mut engine = TimerEngine::new("agent1", &fields, fire_tx, None);

    engine.start("short", None).unwrap();
    engine.start("long", None).unwrap();

    // Advance past short timer
    time::sleep(StdDuration::from_secs(4)).await;

    let fired1 = recv_fired(&mut fire_rx).await;
    assert_eq!(fired1.timer_name, "short");

    // Long timer should not have fired yet
    assert_no_fire(&mut fire_rx).await;

    // Advance past long timer
    time::sleep(StdDuration::from_secs(7)).await;

    let fired2 = recv_fired(&mut fire_rx).await;
    assert_eq!(fired2.timer_name, "long");
}

#[tokio::test(start_paused = true)]
async fn cancel_one_context_leaves_other() {
    let fields = vec![timer_field("reconnect", 5)];
    let (fire_tx, mut fire_rx) = mpsc::channel::<TimerFired>(64);
    let mut engine = TimerEngine::new("agent1", &fields, fire_tx, None);

    let ctx1 = ConfidentValue::deterministic(Value::Text("player1".into()));
    let ctx2 = ConfidentValue::deterministic(Value::Text("player2".into()));
    engine.start("reconnect", Some(ctx1.clone())).unwrap();
    engine.start("reconnect", Some(ctx2)).unwrap();

    // Cancel only player1
    engine.cancel("reconnect", &Some(ctx1)).unwrap();

    time::sleep(StdDuration::from_secs(6)).await;

    // Only player2 should fire
    let fired = recv_fired(&mut fire_rx).await;
    assert_eq!(fired.timer_name, "reconnect");
    match fired.context {
        Some(cv) => assert!(matches!(cv.value, Value::Text(ref s) if s == "player2")),
        None => panic!("expected player2 context"),
    }

    // No more fires
    assert_no_fire(&mut fire_rx).await;
}

#[tokio::test(start_paused = true)]
async fn unknown_timer_errors() {
    let fields = vec![timer_field("timeout", 5)];
    let (fire_tx, _fire_rx) = mpsc::channel::<TimerFired>(64);
    let mut engine = TimerEngine::new("agent1", &fields, fire_tx, None);

    assert!(engine.start("bogus", None).is_err());
    assert!(engine.cancel("bogus", &None).is_err());
    assert!(engine.reset("bogus", None).is_err());
}

#[tokio::test(start_paused = true)]
async fn cancel_all_stops_everything() {
    let fields = vec![timer_field("a", 5), timer_field("b", 5)];
    let (fire_tx, mut fire_rx) = mpsc::channel::<TimerFired>(64);
    let mut engine = TimerEngine::new("agent1", &fields, fire_tx, None);

    engine.start("a", None).unwrap();
    engine.start("b", None).unwrap();
    engine.cancel_all();

    time::sleep(StdDuration::from_secs(10)).await;

    assert_no_fire(&mut fire_rx).await;
}

// ── Integration tests: through AgentProcess ─────────────────────────────────

#[tokio::test(start_paused = true)]
async fn timer_expired_dispatches_handler() {
    let expired_handler = spanned(OnHandler {
        event: spanned("timeout.expired".into()),
        params: vec![],
        payload_type: None,
        requires: vec![],
        body: vec![spanned(Stmt::Emit(spanned("TimedOut".into()), vec![]))],
    });

    let start_handler = spanned(OnHandler {
        event: spanned("begin".into()),
        params: vec![],
        payload_type: None,
        requires: vec![],
        body: vec![spanned(Stmt::StartTimer {
            name: spanned("timeout".into()),
            context: None,
        })],
    });

    let timers = vec![timer_field("timeout", 2)];
    let decl = simple_agent_with_timers(timers, vec![start_handler, expired_handler]);
    let mut agent = AgentProcess::new(
        decl,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    );

    // Start the timer via handler dispatch
    agent.dispatch("begin", HashMap::new()).await.unwrap();

    // Verify timer is Running
    {
        let ctx = agent.context().lock().unwrap();
        assert_eq!(
            ctx.timer_manager.state("timeout"),
            Some(&TimerState::Running)
        );
    }

    // Advance time past the timer duration
    time::sleep(StdDuration::from_secs(3)).await;

    // Receive the timer event and dispatch it (simulating run() loop)
    let fired = recv_agent_fired(&mut agent).await;
    assert_eq!(fired.timer_name, "timeout");
    agent.handle_timer_fired(fired).await.unwrap();

    // Verify timer state is Expired and handler ran (emitted event)
    let ctx = agent.context().lock().unwrap();
    assert_eq!(
        ctx.timer_manager.state("timeout"),
        Some(&TimerState::Expired)
    );
    assert_eq!(ctx.event_sink.emitted.len(), 1);
    assert_eq!(ctx.event_sink.emitted[0].name, "TimedOut");
}

/// Helper to receive from agent's timer_rx
async fn recv_agent_fired(agent: &mut AgentProcess) -> TimerFired {
    agent
        .timer_rx
        .try_recv()
        .expect("expected TimerFired event from agent")
}

#[tokio::test(start_paused = true)]
async fn timer_expired_with_context_param() {
    let expired_handler = spanned(OnHandler {
        event: spanned("reconnect.expired".into()),
        params: vec![spanned(Param {
            name: "player".to_string(),
            type_name: spanned(TypeName::Text),
        })],
        payload_type: None,
        requires: vec![],
        body: vec![spanned(Stmt::Emit(
            spanned("PlayerForfeited".into()),
            vec![],
        ))],
    });

    let start_handler = spanned(OnHandler {
        event: spanned("disconnect".into()),
        params: vec![],
        payload_type: None,
        requires: vec![],
        body: vec![spanned(Stmt::StartTimer {
            name: spanned("reconnect".into()),
            context: Some(spanned(Expr::Template(vec![spanned(TemplatePart::Text(
                "player1".into(),
            ))]))),
        })],
    });

    let timers = vec![timer_field("reconnect", 30)];
    let decl = simple_agent_with_timers(timers, vec![start_handler, expired_handler]);
    let mut agent = AgentProcess::new(
        decl,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    );

    // Start timer with context
    agent.dispatch("disconnect", HashMap::new()).await.unwrap();

    // Advance past duration
    time::sleep(StdDuration::from_secs(31)).await;

    // Receive and dispatch
    let fired = recv_agent_fired(&mut agent).await;
    assert_eq!(fired.timer_name, "reconnect");
    assert!(fired.context.is_some());
    agent.handle_timer_fired(fired).await.unwrap();

    // Verify handler ran
    let ctx = agent.context().lock().unwrap();
    assert_eq!(ctx.event_sink.emitted.len(), 1);
    assert_eq!(ctx.event_sink.emitted[0].name, "PlayerForfeited");
}

#[tokio::test(start_paused = true)]
async fn timer_cancel_in_handler_prevents_expiry() {
    let start_handler = spanned(OnHandler {
        event: spanned("begin".into()),
        params: vec![],
        payload_type: None,
        requires: vec![],
        body: vec![spanned(Stmt::StartTimer {
            name: spanned("timeout".into()),
            context: None,
        })],
    });

    let cancel_handler = spanned(OnHandler {
        event: spanned("stop".into()),
        params: vec![],
        payload_type: None,
        requires: vec![],
        body: vec![spanned(Stmt::CancelTimer {
            name: spanned("timeout".into()),
            context: None,
        })],
    });

    let timers = vec![timer_field("timeout", 5)];
    let decl = simple_agent_with_timers(timers, vec![start_handler, cancel_handler]);
    let mut agent = AgentProcess::new(
        decl,
        None,
        mock_registry(),
        None,
        empty_program(),
        None,
        None,
        None,
    );

    // Start then cancel
    agent.dispatch("begin", HashMap::new()).await.unwrap();
    agent.dispatch("stop", HashMap::new()).await.unwrap();

    // Advance past duration
    time::sleep(StdDuration::from_secs(10)).await;
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;

    // No timer event should arrive
    assert!(agent.timer_rx.try_recv().is_err());

    // Sync state should be Idle (from cancel handler)
    let ctx = agent.context().lock().unwrap();
    assert_eq!(ctx.timer_manager.state("timeout"), Some(&TimerState::Idle));
}

#[tokio::test(start_paused = true)]
async fn duration_unit_conversion() {
    let fields = vec![spanned(TimerField {
        name: spanned("check".to_string()),
        duration: spanned(Duration {
            value: 2,
            unit: DurationUnit::Minutes,
        }),
    })];
    let (fire_tx, mut fire_rx) = mpsc::channel::<TimerFired>(64);
    let mut engine = TimerEngine::new("agent1", &fields, fire_tx, None);

    engine.start("check", None).unwrap();

    // 119s — should not fire yet
    time::sleep(StdDuration::from_secs(119)).await;
    assert_no_fire(&mut fire_rx).await;

    // 2 more seconds — total 121s > 120s, should fire
    time::sleep(StdDuration::from_secs(2)).await;

    let fired = recv_fired(&mut fire_rx).await;
    assert_eq!(fired.timer_name, "check");
}
