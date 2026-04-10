// FORGE Layer 1 acceptance tests — issue #26
// Proves the substrate is complete: checker errors caught, runtime works with mock provider.

use std::collections::HashMap;
use std::sync::Arc;

use forge::ast::{Program, TopLevel};
use forge::checker;
use forge::checker::boundary_checker;
use forge::diagnostic::{Diagnostic, DiagnosticKind};
use forge::llm::providers::mock::MockProvider;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::agent::AgentProcess;
use forge::runtime::confidence::{ConfidentValue, Value};
use forge::runtime::executor::TaskExecutor;

// ── Helpers ──────────────────────────────────────────────────────

fn parse_file(path: &str) -> Program {
    let source =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("could not read {}: {}", path, e));
    forge::parser::parse(&source).unwrap_or_else(|e| panic!("parse failed for {}: {:?}", path, e))
}

fn check_file(path: &str) -> Vec<Diagnostic> {
    let program = parse_file(path);
    let filename = std::path::Path::new(path)
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    let mut diags = checker::check_all(&program, filename);
    // Also run boundary checker for single-file (per-file checks only)
    diags.extend(boundary_checker::check(&[(&program, filename)]));
    diags
}

fn check_files(paths: &[&str]) -> Vec<Diagnostic> {
    let programs: Vec<(Program, String)> = paths
        .iter()
        .map(|p| {
            let program = parse_file(p);
            let filename = std::path::Path::new(p)
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            (program, filename)
        })
        .collect();

    let mut diags = Vec::new();
    for (program, filename) in &programs {
        diags.extend(checker::check_all(program, filename));
    }

    let refs: Vec<(&Program, &str)> = programs.iter().map(|(p, f)| (p, f.as_str())).collect();
    diags.extend(boundary_checker::check(&refs));

    diags
}

fn mock_registry(mock: MockProvider) -> Arc<ProviderRegistry> {
    let mut reg = ProviderRegistry::new("mock");
    reg.register("mock", Arc::new(mock));
    Arc::new(reg)
}

fn errors(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
    diags
        .iter()
        .filter(|d| matches!(d.kind, DiagnosticKind::Error))
        .collect()
}

fn text_param(key: &str, val: &str) -> (String, ConfidentValue) {
    (
        key.to_string(),
        ConfidentValue::deterministic(Value::Text(val.to_string())),
    )
}

fn number_param(key: &str, val: f64) -> (String, ConfidentValue) {
    (
        key.to_string(),
        ConfidentValue::deterministic(Value::Number(val)),
    )
}

// ── Checker error acceptance tests ───────────────────────────────

#[test]
fn accept_uncertain_error() {
    let diags = check_file("examples/errors/uncertain_error.forge");
    let errs = errors(&diags);
    assert!(!errs.is_empty(), "should detect unhandled uncertain");
    assert!(
        errs.iter()
            .any(|d| d.message.contains("unhandled uncertain")),
        "error message should contain 'unhandled uncertain', got: {:?}",
        errs.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn accept_session_uncertain_error() {
    let source = "task review\n  gives Text\n  do\n    result = session \"code-review\" prompt \"check\"\n    give result\n";
    let program = forge::parser::parse(source).unwrap();
    let mut diags = checker::check_all(&program, "session_uncertain.forge");
    diags.extend(boundary_checker::check(&[(
        &program,
        "session_uncertain.forge",
    )]));
    let errs = errors(&diags);
    assert!(
        errs.iter()
            .any(|d| d.message.contains("unhandled uncertain")),
        "session should be treated as uncertain/oracle output: {:?}",
        errs.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn accept_pure_error() {
    let diags = check_file("examples/errors/pure_error.forge");
    let errs = errors(&diags);
    assert!(!errs.is_empty(), "should detect LLM op in pure function");
    assert!(
        errs.iter().any(|d| d.message.contains("cannot use")),
        "error message should contain 'cannot use', got: {:?}",
        errs.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn accept_states_error() {
    let diags = check_file("examples/errors/states_error.forge");
    let errs = errors(&diags);
    assert!(!errs.is_empty(), "should detect illegal transition");
    assert!(
        errs.iter()
            .any(|d| d.message.contains("illegal transition")),
        "error message should contain 'illegal transition', got: {:?}",
        errs.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn accept_boundary_error() {
    let diags = check_files(&[
        "examples/errors/boundary_error_server.forge",
        "examples/errors/boundary_error_client.forge",
    ]);
    let errs = errors(&diags);
    assert!(!errs.is_empty(), "should detect cross-boundary reference");
    assert!(
        errs.iter()
            .any(|d| d.message.contains("server-only symbol")),
        "error message should contain 'server-only symbol', got: {:?}",
        errs.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

// ── Runtime acceptance tests ─────────────────────────────────────

#[tokio::test]
async fn accept_hello_run() {
    let program = parse_file("examples/basics/hello.forge");
    let mock = MockProvider::new("mock").with_default("mock response");
    let executor = TaskExecutor::new(program, mock_registry(mock), None);
    let result = executor.run().await;
    assert!(
        result.is_ok(),
        "hello.forge should run without error: {:?}",
        result.err()
    );
    let outputs = executor.outputs();
    assert_eq!(outputs, vec!["Hello, World!"]);
}

#[tokio::test]
async fn accept_research_run_without_search_provider() {
    // Without a configured search provider, search produces a FlowError
    // (it tries to connect to localhost:8080 SearXNG by default).
    let program = parse_file("examples/llm/research.forge");
    let mock = MockProvider::new("mock")
        .with_response("search", "Search results about artificial intelligence")
        .with_response("synthesize", "A synthesized report on AI advances")
        .with_response("factually consistent", "Yes, this is factually consistent")
        .with_default("mock research response");
    let executor = TaskExecutor::new(program, mock_registry(mock), None);
    let result = executor.run().await;
    assert!(
        result.is_err(),
        "research.forge should fail without a search provider"
    );
}

#[tokio::test]
async fn accept_tictactoe_game() {
    // Load room_agent (the agent under test) and platform (pure functions + states)
    let room_source = std::fs::read_to_string("examples/tictactoe/room_agent.forge")
        .expect("could not read room_agent.forge");
    let platform_source = std::fs::read_to_string("examples/tictactoe/platform.forge")
        .expect("could not read platform.forge");

    let room_program = forge::parser::parse(&room_source).expect("parse room_agent failed");
    let platform_program = forge::parser::parse(&platform_source).expect("parse platform failed");

    // Extract the agent declaration from room_agent.forge
    let agent_decl = room_program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Agent(a) => Some(a.as_ref().clone()),
            _ => None,
        })
        .expect("no agent in room_agent.forge");

    // Extract states declaration (GamePhase) — could be in either file
    let states_decl = room_program
        .items
        .iter()
        .chain(platform_program.items.iter())
        .find_map(|item| match &item.node {
            TopLevel::States(s) => Some(s.clone()),
            _ => None,
        });

    // Build a combined program with pure functions from platform + agent items
    let mut combined_items = platform_program.items.clone();
    combined_items.extend(room_program.items.clone());
    let combined_program = Program {
        boundary: None,
        items: combined_items,
    };

    // Mock provider — room_agent doesn't use LLM calls directly,
    // but we need a registry for AgentProcess
    let mock = MockProvider::new("mock").with_default("mock response");

    let agent = AgentProcess::new(
        agent_decl,
        states_decl.as_ref(),
        mock_registry(mock),
        None,
        combined_program,
        None,
        None,
    );

    // Initialize board with "_" markers (memory default is empty strings)
    {
        let mut ctx = agent.context().lock().unwrap();
        let blank_board: Vec<ConfidentValue> = (0..9)
            .map(|_| ConfidentValue::deterministic(Value::Text("_".to_string())))
            .collect();
        ctx.memory.set(
            "board",
            ConfidentValue::deterministic(Value::Array(blank_board)),
        );
        ctx.memory.set(
            "current_turn",
            ConfidentValue::deterministic(Value::Text("X".to_string())),
        );
    }

    // ── Phase 1: Join mechanics and state transitions ────────────

    // 1. Player X joins
    println!("=== Player X joins ===");
    let params = HashMap::from([text_param("player", "X")]);
    let _r = agent.dispatch("join", params).await.unwrap();

    {
        let ctx = agent.context().lock().unwrap();
        let count = ctx.memory.get("player_count").unwrap();
        assert!(
            matches!(&count.value, Value::Number(n) if *n == 1.0),
            "player_count should be 1 after first join"
        );
        let has_join_event = ctx
            .event_sink
            .emitted
            .iter()
            .any(|e| e.name == "PlayerJoined");
        assert!(has_join_event, "should emit PlayerJoined event");
    }

    // 2. Player O joins — triggers transition to "playing"
    println!("=== Player O joins ===");
    let params = HashMap::from([text_param("player", "O")]);
    let _r = agent.dispatch("join", params).await.unwrap();

    {
        let ctx = agent.context().lock().unwrap();
        let count = ctx.memory.get("player_count").unwrap();
        assert!(
            matches!(&count.value, Value::Number(n) if *n == 2.0),
            "player_count should be 2 after second join"
        );
        let sm = ctx
            .state_machine
            .as_ref()
            .expect("should have state machine");
        assert_eq!(
            sm.current, "playing",
            "lifecycle should be 'playing' after 2 joins"
        );
    }

    // 3. Requires guard rejects third join
    println!("=== Third join attempt (should be rejected) ===");
    let params = HashMap::from([text_param("player", "Z")]);
    let result = agent.dispatch("join", params).await.unwrap();
    assert!(
        matches!(result, Some(ref v) if matches!(&v.value, Value::Text(s) if s.contains("game already started"))),
        "third join should be rejected with 'game already started', got: {:?}",
        result
    );

    // ── Phase 2: Full scripted game — X wins with top row ────────
    //    Board layout: 0|1|2
    //                   3|4|5
    //                   6|7|8

    let moves = [
        ("X", 0.0), // X takes top-left
        ("O", 3.0), // O takes middle-left
        ("X", 1.0), // X takes top-center
        ("O", 4.0), // O takes center
        ("X", 2.0), // X takes top-right → X wins!
    ];

    let mut last_result = None;
    for (player, cell) in &moves {
        println!("=== Move: {} -> cell {} ===", player, cell);
        let params = HashMap::from([text_param("player", player), number_param("cell", *cell)]);
        last_result = agent.dispatch("move", params).await.unwrap();
    }

    // The final move should return a GameResult with winner X
    let result = last_result.expect("last move should return a GameResult");
    println!("Game result: {:?}", result.value);

    match &result.value {
        Value::Record(fields) => {
            // Result is a tagged record: { _type: "GameResult", _value: { winner, detail } }
            let inner = fields
                .get("_value")
                .and_then(|v| match &v.value {
                    Value::Record(inner) => Some(inner),
                    _ => None,
                })
                .or_else(|| {
                    // Or a flat record with winner directly
                    if fields.contains_key("winner") {
                        Some(fields)
                    } else {
                        None
                    }
                })
                .expect("GameResult should have winner field (flat or tagged)");
            let winner = inner
                .get("winner")
                .expect("inner record should have winner");
            assert!(
                matches!(&winner.value, Value::Text(s) if s == "X"),
                "winner should be X, got: {:?}",
                winner.value
            );
        }
        other => panic!("expected GameResult record, got: {:?}", other),
    }

    // Verify board state
    {
        let ctx = agent.context().lock().unwrap();
        let board = ctx.memory.get("board").expect("board should exist");
        println!("Final board: {:?}", board.value);
    }

    println!("=== Tic-tac-toe full game complete — X wins! ===");
}

// ── Fact-check pool acceptance test (issue #63) ─────────────────

#[test]
fn accept_fact_check_pool_parse() {
    let diags = check_file("examples/agents/fact_check_pool.forge");
    let errs = errors(&diags);
    assert!(
        errs.is_empty(),
        "fact_check_pool.forge should have no checker errors, got: {:?}",
        errs.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn accept_fact_check_pool_run() {
    let program = parse_file("examples/agents/fact_check_pool.forge");
    // Mock provider returns 3 identical responses for the 3 pool workers
    let mock = MockProvider::new("mock").with_default("YES this claim is factually accurate");
    let executor = TaskExecutor::new(program, mock_registry(mock), None);
    let result = executor.run().await;
    assert!(
        result.is_ok(),
        "fact_check_pool.forge should run without error: {:?}",
        result.err()
    );
    let outputs = executor.outputs();
    assert!(
        outputs
            .iter()
            .any(|o| o.contains("YES") || o.contains("Verdict")),
        "should output a verdict, got: {:?}",
        outputs
    );
}

// ── CLI smoke tests ──────────────────────────────────────────────

#[test]
fn cli_check_valid_exits_zero() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_forge"))
        .args(["check", "examples/basics/hello.forge"])
        .output()
        .expect("failed to execute forge binary");
    assert!(
        output.status.success(),
        "forge check on valid file should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn cli_check_error_exits_nonzero() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_forge"))
        .args(["check", "examples/errors/states_error.forge"])
        .output()
        .expect("failed to execute forge binary");
    assert!(
        !output.status.success(),
        "forge check on error file should exit non-zero"
    );
}
