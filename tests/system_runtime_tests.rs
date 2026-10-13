// FORGE system runtime tests — issue #87
// Tests for SystemRuntime: binding resolution, wiring parse, spawning,
// event routing, resource limits, and warden integration.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use forge::ast::{Program, SystemDecl, TopLevel};
use forge::config::SystemConfig;
use forge::runtime::system::SystemRuntime;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn mock_providers() -> Arc<forge::llm::registry::ProviderRegistry> {
    let config = forge::config::ForgeConfig::default_mock_config();
    Arc::new(forge::llm::registry::ProviderRegistry::from_config(config).unwrap())
}

/// Parse a complete FORGE source string and extract the system decl + full program.
fn parse_system(src: &str) -> (SystemDecl, Program) {
    let program = forge::parser::parse(src).unwrap();
    let system_decl = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::System(s) => Some(s.clone()),
            _ => None,
        })
        .expect("source must contain a system declaration");
    (system_decl, program)
}

/// Minimal FORGE source with two agents and a system declaration.
const TWO_AGENT_SYSTEM: &str = "\
use
  llm.reason

agent my_service
  on greet(name: Text)
    result = reason \"hello {name}\"
    say result

agent my_db
  on query(q: Text)
    result = reason \"query: {q}\"
    say result

system app
  use
    svc: my_service
    db: my_db
  svc >> db
";

/// FORGE source with two agents, no wiring.
const SIMPLE_SYSTEM: &str = "\
use
  llm.reason

agent alpha
  on start()
    say \"alpha\"

agent beta
  on start()
    say \"beta\"

system pair
  use
    a: alpha
    b: beta
";

/// FORGE source with a warden managing one system agent.
const WARDEN_SYSTEM: &str = "\
use
  llm.reason

agent worker
  on start()
    say \"working\"

agent helper
  on start()
    say \"helping\"

warden supervisor
  manages [worker]
  on crash: restart, self
  on stuck: nudge, self
  on hallucination: nudge, self
  on budget: nudge, all
  on timeout: restart, self

system guarded
  use
    w: worker
    h: helper
";

// ── Binding Resolution Tests ─────────────────────────────────────────────────

#[test]
fn system_resolves_bindings_to_agents() {
    let (system_decl, program) = parse_system(TWO_AGENT_SYSTEM);
    let providers = mock_providers();
    let runtime = SystemRuntime::new(&system_decl, &program, providers, None, None);
    assert!(runtime.is_ok());
}

#[test]
fn system_errors_on_unknown_agent() {
    let src = "\
use
  llm.reason

agent real_agent
  on start()
    say \"hello\"

system bad
  use
    x: nonexistent_agent
";
    let (system_decl, program) = parse_system(src);
    let providers = mock_providers();
    let runtime = SystemRuntime::new(&system_decl, &program, providers, None, None);
    assert!(runtime.is_err());
    let err = runtime.err().unwrap().to_string();
    assert!(err.contains("nonexistent_agent"), "error: {}", err);
}

// ── Wiring Parse Tests ───────────────────────────────────────────────────────

#[test]
fn system_parses_compose_wiring() {
    let (system_decl, program) = parse_system(TWO_AGENT_SYSTEM);
    let providers = mock_providers();
    let runtime = SystemRuntime::new(&system_decl, &program, providers, None, None);
    assert!(runtime.is_ok());
}

#[test]
fn system_no_wiring_is_valid() {
    let (system_decl, program) = parse_system(SIMPLE_SYSTEM);
    let providers = mock_providers();
    let runtime = SystemRuntime::new(&system_decl, &program, providers, None, None);
    assert!(runtime.is_ok());
}

#[test]
fn system_three_stage_wiring() {
    let src = "\
use
  llm.reason

agent ingest
  on start()
    say \"ingest\"

agent process
  on start()
    say \"process\"

agent output
  on start()
    say \"output\"

system pipeline
  use
    i: ingest
    p: process
    o: output
  i >> p >> o
";
    let (system_decl, program) = parse_system(src);
    let providers = mock_providers();
    let runtime = SystemRuntime::new(&system_decl, &program, providers, None, None);
    assert!(runtime.is_ok());
}

// ── Resource Limit Tests ─────────────────────────────────────────────────────

#[tokio::test]
async fn system_enforces_max_agents_limit() {
    let (system_decl, program) = parse_system(SIMPLE_SYSTEM);
    let providers = mock_providers();

    let config = SystemConfig {
        max_agents: Some(1),
        max_memory_mb: None,
    };

    let runtime =
        SystemRuntime::new(&system_decl, &program, providers, None, Some(&config)).unwrap();

    // Starting should fail because we have 2 agents but max is 1
    let result = runtime.start().await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("max_agents"), "error: {}", err);
}

// ── Spawning Tests ───────────────────────────────────────────────────────────

#[tokio::test]
async fn system_spawns_agents_and_exits_cleanly() {
    let (system_decl, program) = parse_system(SIMPLE_SYSTEM);
    let providers = mock_providers();

    let runtime = SystemRuntime::new(&system_decl, &program, providers, None, None).unwrap();

    let _registry = runtime.instance_registry().clone();

    // Agents will spawn and exit quickly (mock provider, no events to process)
    let result = runtime.start().await;
    assert!(result.is_ok(), "system start failed: {:?}", result);
}

// ── Start gate Tests (#545) ──────────────────────────────────────────────────

/// Build a system where `starter` (which emits `Kickoff` from `on start`) is
/// bound first and `worker` (the only subscriber) last, with `fillers` agents
/// in between.
fn start_gate_source(fillers: usize) -> String {
    let mut src = String::from(
        "event Kickoff\n  topic: Text\n\nagent starter\n  on start()\n    emit Kickoff(topic: \"go\")\n\n",
    );
    for i in 0..fillers {
        src.push_str(&format!(
            "agent filler_{i}\n  on start()\n    say \"filler {i}\"\n\n"
        ));
    }
    src.push_str("agent worker\n  subscribe Kickoff\n\n  on Kickoff(topic: Text)\n    say \"worker handled the kickoff\"\n\nsystem chain\n  use\n    starter: starter\n");
    for i in 0..fillers {
        src.push_str(&format!("    filler_{i}: filler_{i}\n"));
    }
    src.push_str("    worker: worker\n");
    src
}

/// A system subscribes every agent before any `on start` runs, so an earlier
/// agent's start emit reaches a later binding (#545). Multi-threaded on
/// purpose: the bug was a scheduling race, invisible on a single thread.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn start_gate_subscribes_every_agent_before_on_start() {
    const FILLERS: usize = 5;
    use forge::tracer::Tracer;

    // `worker`'s subscription keeps its bus channel open, so the system never
    // runs out of agents and `start()` cannot return on its own. Run it in the
    // background and watch the trace instead.
    for round in 0..20 {
        let src = start_gate_source(FILLERS);
        let (system_decl, program) = parse_system(&src);
        let providers = mock_providers();
        let tracer = Tracer::with_capture();

        let runtime = SystemRuntime::new(
            &system_decl,
            &program,
            providers,
            Some(tracer.clone()),
            None,
        )
        .unwrap();
        let system = tokio::spawn(async move { runtime.start().await });

        // Worker's handler is what the race swallowed. Wait for it, so the
        // assertions below read a settled trace.
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && !worker_handled(&tracer) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        system.abort();

        let log = tracer.captured_log();
        // The publish counts its deliveries: zero means `on start` outran the
        // subscriber's binding, which is the bug.
        let kickoff: Vec<u64> = log
            .iter()
            .filter(|(name, v)| name == "event_emit" && v["event"] == "Kickoff")
            .map(|(_, v)| v["subscribers"].as_u64().unwrap_or(0))
            .collect();
        assert!(
            !kickoff.is_empty() && kickoff.iter().all(|n| *n >= 1),
            "round {round}: starter's `on start` emit reached {kickoff:?} subscriber(s)"
        );
        assert!(
            worker_handled(&tracer),
            "round {round}: worker's Kickoff handler never ran"
        );
    }
}

fn worker_handled(tracer: &forge::tracer::Tracer) -> bool {
    tracer
        .captured_log()
        .iter()
        .any(|(name, v)| name == "say" && v["text"] == "worker handled the kickoff")
}

// ── Warden Integration Tests ─────────────────────────────────────────────────

#[test]
fn system_discovers_wardens_for_managed_agents() {
    let (system_decl, program) = parse_system(WARDEN_SYSTEM);
    let providers = mock_providers();
    let runtime = SystemRuntime::new(&system_decl, &program, providers, None, None);
    assert!(runtime.is_ok());
}

// ── Event Routing Tests ──────────────────────────────────────────────────────

#[tokio::test]
async fn event_bus_routes_events_between_agents() {
    use forge::runtime::event_bus::{EventBus, EventPayload};

    let mut bus = EventBus::new(None);

    // Subscribe agent_b to "Foo" events
    let mut rx = bus.subscribe("Foo", "agent_b", None);

    // Add route: agent_a → agent_b
    bus.add_route("agent_a", "agent_b");

    // Publish a "Foo" event from agent_a
    let payload = EventPayload {
        event_name: "Foo".to_string(),
        args: vec![],
        source_agent: "agent_a".to_string(),
        fields: HashMap::new(),
    };

    let delivered = bus.publish(&payload);
    assert!(
        delivered >= 1,
        "expected at least 1 delivery, got {}",
        delivered
    );

    // agent_b should have received the event
    let event = rx.recv().await.unwrap();
    assert_eq!(event.event_name, "Foo");
    assert_eq!(event.source_agent, "agent_a");
}

#[tokio::test]
async fn event_bus_routing_adds_delivery_to_target() {
    use forge::runtime::event_bus::{EventBus, EventPayload};

    let mut bus = EventBus::new(None);

    // agent_b subscribes to "Foo" — it's a route target from agent_a
    let mut rx_b = bus.subscribe("Foo", "agent_b", None);
    bus.add_route("agent_a", "agent_b");

    // Publish "Foo" from agent_a — agent_b is both a subscriber and route target
    let payload = EventPayload {
        event_name: "Foo".to_string(),
        args: vec![],
        source_agent: "agent_a".to_string(),
        fields: HashMap::new(),
    };

    let delivered = bus.publish(&payload);
    // agent_b receives via normal subscription (1) + route forward (1) = 2
    assert_eq!(delivered, 2);

    // Verify agent_b received both copies
    assert!(rx_b.recv().await.is_some());
    assert!(rx_b.recv().await.is_some());
}

#[tokio::test]
async fn event_bus_routing_does_not_forward_to_non_targets() {
    use forge::runtime::event_bus::{EventBus, EventPayload};

    let mut bus = EventBus::new(None);

    // Subscribe agent_b and agent_c to "Ping"
    let mut rx_b = bus.subscribe("Ping", "agent_b", None);
    let mut rx_c = bus.subscribe("Ping", "agent_c", None);

    // Route only a → b (NOT a → c)
    bus.add_route("agent_a", "agent_b");

    let payload = EventPayload {
        event_name: "Ping".to_string(),
        args: vec![],
        source_agent: "agent_a".to_string(),
        fields: HashMap::new(),
    };

    let delivered = bus.publish(&payload);

    // agent_b receives via both normal subscription match AND routing
    assert!(rx_b.recv().await.is_some());

    // agent_c receives via normal subscription match (both subscribed to "Ping"),
    // but NOT via routing. The bus delivers to all matching subscribers.
    assert!(rx_c.recv().await.is_some());

    // Total delivered should include both subscribers + 1 route forward
    // (but forward to agent_b only counts once since it goes through same channel)
    assert!(delivered >= 2);
}

#[tokio::test]
async fn planner_post_message_route_does_not_reach_implementer() {
    use forge::runtime::event_bus::{EventBus, EventPayload};

    let mut bus = EventBus::new(None);

    let mut rx_slack = bus.subscribe("PostMessage", "slack_adapter", None);
    let mut rx_impl = bus.subscribe("ImplementationApproved", "implementer", None);

    bus.add_route("planner", "implementer");

    let payload = EventPayload {
        event_name: "PostMessage".to_string(),
        args: vec![],
        source_agent: "planner".to_string(),
        fields: HashMap::new(),
    };

    let delivered = bus.publish(&payload);

    assert_eq!(delivered, 1);
    assert_eq!(rx_slack.recv().await.unwrap().event_name, "PostMessage");
    assert!(rx_impl.try_recv().is_err());
}

// ── Config Parsing Tests ─────────────────────────────────────────────────────

#[test]
fn parse_system_config_from_toml() {
    let toml_str = r#"
[llm]
default = "mock"

[providers.mock]
type = "mock"

[system]
max_agents = 50
max_memory_mb = 512
"#;
    let config: forge::config::ForgeConfig = toml::from_str(toml_str).unwrap();
    let system = config.system.unwrap();
    assert_eq!(system.max_agents, Some(50));
    assert_eq!(system.max_memory_mb, Some(512));
}

#[test]
fn parse_system_config_optional() {
    let toml_str = r#"
[llm]
default = "mock"

[providers.mock]
type = "mock"
"#;
    let config: forge::config::ForgeConfig = toml::from_str(toml_str).unwrap();
    assert!(config.system.is_none());
}

// ── Full Parse-to-Runtime Test ───────────────────────────────────────────────

#[test]
fn parse_system_declaration_and_build_runtime() {
    let (system_decl, program) = parse_system(TWO_AGENT_SYSTEM);
    let providers = mock_providers();
    let runtime = SystemRuntime::new(&system_decl, &program, providers, None, None);
    assert!(runtime.is_ok());
}
