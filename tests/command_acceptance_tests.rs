// FORGE command primitive acceptance tests — issue #161
// Proves command works end-to-end: string mode, argv mode, structured record
// return, confidence routing, working directory, env vars, timeout, and tracing.

use std::sync::Arc;

use forge::llm::providers::mock::MockProvider;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::executor::TaskExecutor;
use forge::tracer::Tracer;

// ── Helpers ──────────────────────────────────────────────────────

fn parse_file(path: &str) -> forge::ast::Program {
    let source =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("could not read {}: {}", path, e));
    forge::parser::parse(&source).unwrap_or_else(|e| panic!("parse failed for {}: {:?}", path, e))
}

fn mock_registry(mock: MockProvider) -> Arc<ProviderRegistry> {
    let mut reg = ProviderRegistry::new("mock");
    reg.register("mock", Arc::new(mock));
    Arc::new(reg)
}

// ── Runtime: string mode success ────────────────────────────────

#[tokio::test]
async fn command_success_returns_stdout() {
    let program = parse_file("examples/command_success.forge");
    let mock = MockProvider::new("mock").with_default("mock");
    let executor = TaskExecutor::new(program, mock_registry(mock), None);
    let result = executor.run().await;
    assert!(
        result.is_ok(),
        "command_success.forge should run without error: {:?}",
        result.err()
    );
    let outputs = executor.outputs();
    assert!(
        outputs.iter().any(|o| o.contains("hello_from_command")),
        "should contain stdout 'hello_from_command', got: {:?}",
        outputs
    );
}

// ── Runtime: argv mode ──────────────────────────────────────────

#[tokio::test]
async fn command_argv_mode() {
    let program = parse_file("examples/command_argv.forge");
    let mock = MockProvider::new("mock").with_default("mock");
    let executor = TaskExecutor::new(program, mock_registry(mock), None);
    let result = executor.run().await;
    assert!(
        result.is_ok(),
        "command_argv.forge should run without error: {:?}",
        result.err()
    );
    let outputs = executor.outputs();
    assert!(
        outputs.iter().any(|o| o.contains("hello from argv")),
        "should contain 'hello from argv', got: {:?}",
        outputs
    );
}

// ── Runtime: failing command routes to else ──────────────────────

#[tokio::test]
async fn command_fail_routes_to_else() {
    let program = parse_file("examples/command_fail.forge");
    let mock = MockProvider::new("mock").with_default("mock");
    let executor = TaskExecutor::new(program, mock_registry(mock), None);
    let result = executor.run().await;
    assert!(
        result.is_ok(),
        "command_fail.forge should run without error: {:?}",
        result.err()
    );
    let outputs = executor.outputs();
    assert!(
        outputs.iter().any(|o| o.contains("UNRELIABLE")),
        "failing command should route to else (UNRELIABLE), got: {:?}",
        outputs
    );
    assert!(
        !outputs.iter().any(|o| o.contains("SHOULD_NOT_REACH")),
        "failing command must not reach sure branch, got: {:?}",
        outputs
    );
}

// ── Runtime: working directory ──────────────────────────────────

#[tokio::test]
async fn command_workdir() {
    let program = parse_file("examples/command_workdir.forge");
    let mock = MockProvider::new("mock").with_default("mock");
    let executor = TaskExecutor::new(program, mock_registry(mock), None);
    let result = executor.run().await;
    assert!(
        result.is_ok(),
        "command_workdir.forge should run without error: {:?}",
        result.err()
    );
    let outputs = executor.outputs();
    // macOS resolves /tmp to /private/tmp
    assert!(
        outputs.iter().any(|o| o.contains("/tmp")),
        "working directory should be /tmp, got: {:?}",
        outputs
    );
}

// ── Runtime: environment variables ──────────────────────────────

#[tokio::test]
async fn command_env_vars() {
    let program = parse_file("examples/command_env.forge");
    let mock = MockProvider::new("mock").with_default("mock");
    let executor = TaskExecutor::new(program, mock_registry(mock), None);
    let result = executor.run().await;
    assert!(
        result.is_ok(),
        "command_env.forge should run without error: {:?}",
        result.err()
    );
    let outputs = executor.outputs();
    assert!(
        outputs.iter().any(|o| o.contains("bar")),
        "env var FORGE_TEST_VAR should be 'bar', got: {:?}",
        outputs
    );
}

// ── Runtime: timeout ────────────────────────────────────────────

#[tokio::test]
async fn command_timeout_errors() {
    let program = parse_file("examples/command_timeout.forge");
    let mock = MockProvider::new("mock").with_default("mock");
    let executor = TaskExecutor::new(program, mock_registry(mock), None);
    let result = executor.run().await;
    assert!(
        result.is_err(),
        "command_timeout.forge should return an error on timeout"
    );
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("timed out"),
        "error should mention 'timed out', got: {}",
        err
    );
}

// ── Runtime: stderr capture ─────────────────────────────────────

#[tokio::test]
async fn command_stderr_captured() {
    let program = parse_file("examples/command_stderr.forge");
    let mock = MockProvider::new("mock").with_default("mock");
    let executor = TaskExecutor::new(program, mock_registry(mock), None);
    let result = executor.run().await;
    assert!(
        result.is_ok(),
        "command_stderr.forge should run without error: {:?}",
        result.err()
    );
    let outputs = executor.outputs();
    assert!(
        outputs.iter().any(|o| o.contains("err_output")),
        "should capture stderr 'err_output', got: {:?}",
        outputs
    );
}

// ── Tracing: command calls produce trace events ─────────────────

#[tokio::test]
async fn command_produces_trace_events() {
    let program = parse_file("examples/command_success.forge");
    let mock = MockProvider::new("mock").with_default("mock");
    let tracer = Tracer::with_capture();
    let executor = TaskExecutor::new(program, mock_registry(mock), Some(tracer.clone()));
    let _result = executor.run().await;
    let events = tracer.captured_events();
    assert!(
        events.contains(&"command_call".to_string()),
        "should emit command_call trace event, got: {:?}",
        events
    );
    assert!(
        events.contains(&"command_return".to_string()),
        "should emit command_return trace event, got: {:?}",
        events
    );
}
