// FORGE exec primitive acceptance tests — issue #40
// Proves exec works end-to-end: real CLI commands, confidence handling,
// when dispatch, composition, checker enforcement, and tracing.

use std::sync::Arc;

use forge::checker;
use forge::diagnostic::{Diagnostic, DiagnosticKind};
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

fn errors(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
    diags
        .iter()
        .filter(|d| matches!(d.kind, DiagnosticKind::Error))
        .collect()
}

// ── Runtime: exec runs real commands ─────────────────────────────

#[tokio::test]
async fn exec_success_echoes_output() {
    let program = parse_file("examples/command/exec_success.forge");
    let mock = MockProvider::new("mock").with_default("mock");
    let executor = TaskExecutor::new(program, mock_registry(mock), None);
    let result = executor.run().await;
    assert!(
        result.is_ok(),
        "exec_success.forge should run without error: {:?}",
        result.err()
    );
    let outputs = executor.outputs();
    assert!(
        outputs.iter().any(|o| o.contains("hello_from_exec")),
        "should contain exec output 'hello_from_exec', got: {:?}",
        outputs
    );
}

#[tokio::test]
async fn exec_fail_routes_to_else() {
    let program = parse_file("examples/command/exec_fail.forge");
    let mock = MockProvider::new("mock").with_default("mock");
    let executor = TaskExecutor::new(program, mock_registry(mock), None);
    let result = executor.run().await;
    assert!(
        result.is_ok(),
        "exec_fail.forge should run without error: {:?}",
        result.err()
    );
    let outputs = executor.outputs();
    // exit 1 → confidence 0.3 → unreliable → else branch → "UNRELIABLE"
    assert!(
        outputs.iter().any(|o| o.contains("UNRELIABLE")),
        "failing command should route to else (UNRELIABLE), got: {:?}",
        outputs
    );
    // Must NOT reach the sure branch
    assert!(
        !outputs.iter().any(|o| o.contains("SHOULD_NOT_REACH")),
        "failing command must not reach sure branch, got: {:?}",
        outputs
    );
}

#[tokio::test]
async fn exec_when_dispatch_routes_correctly() {
    let program = parse_file("examples/command/exec_when_dispatch.forge");
    let mock = MockProvider::new("mock").with_default("mock");
    let executor = TaskExecutor::new(program, mock_registry(mock), None);
    let result = executor.run().await;
    assert!(
        result.is_ok(),
        "exec_when_dispatch.forge should run: {:?}",
        result.err()
    );
    let outputs = executor.outputs();
    // First call: "echo works" succeeds → confidence 0.9 → sure → "SURE:works"
    assert!(
        outputs
            .iter()
            .any(|o| o.starts_with("SURE:") && o.contains("works")),
        "successful command should route to SURE, got: {:?}",
        outputs
    );
    // Second call: exit 1 → confidence 0.3 → unreliable → "LOW:..."
    assert!(
        outputs.iter().any(|o| o.starts_with("LOW:")),
        "failing command should route to LOW, got: {:?}",
        outputs
    );
}

#[tokio::test]
async fn exec_multi_captures_pwd() {
    let program = parse_file("examples/command/exec_multi.forge");
    let mock = MockProvider::new("mock").with_default("mock");
    let executor = TaskExecutor::new(program, mock_registry(mock), None);
    let result = executor.run().await;
    assert!(
        result.is_ok(),
        "exec_multi.forge should run: {:?}",
        result.err()
    );
    let outputs = executor.outputs();
    // pwd should return an actual directory path
    assert!(
        outputs.iter().any(|o| o.starts_with("/")),
        "pwd should return an absolute path, got: {:?}",
        outputs
    );
}

#[tokio::test]
async fn exec_compose_with_reason() {
    let program = parse_file("examples/command/exec_compose.forge");
    let mock = MockProvider::new("mock").with_default("5 files found");
    let executor = TaskExecutor::new(program, mock_registry(mock), None);
    let result = executor.run().await;
    assert!(
        result.is_ok(),
        "exec_compose.forge should run: {:?}",
        result.err()
    );
    let outputs = executor.outputs();
    // exec "ls -la" >> reason "..." → mock returns "5 files found"
    assert!(
        outputs.iter().any(|o| o.contains("5 files found")),
        "exec >> reason should produce LLM response, got: {:?}",
        outputs
    );
}

// ── Checker: exec in pure function = error ──────────────────────

#[test]
fn exec_pure_error_detected() {
    let program = parse_file("examples/errors/exec_pure_error.forge");
    let filename = "exec_pure_error.forge";
    let diags = checker::check_all(&program, filename);
    let errs = errors(&diags);
    assert!(
        !errs.is_empty(),
        "exec in pure function should produce checker error"
    );
    assert!(
        errs.iter().any(|d| d.message.contains("exec")),
        "error should mention 'exec', got: {:?}",
        errs.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

// ── Checker: exec result without when = uncertain error ─────────

#[test]
fn exec_uncertain_error_detected() {
    let program = parse_file("examples/errors/exec_uncertain_error.forge");
    let filename = "exec_uncertain_error.forge";
    let diags = checker::check_all(&program, filename);
    let errs = errors(&diags);
    assert!(
        !errs.is_empty(),
        "unhandled exec result should produce uncertain error"
    );
    assert!(
        errs.iter().any(|d| d.message.contains("uncertain")),
        "error should mention 'uncertain', got: {:?}",
        errs.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

// ── Tracing: exec calls produce trace events ────────────────────

#[tokio::test]
async fn exec_produces_trace_events() {
    let program = parse_file("examples/command/exec_success.forge");
    let mock = MockProvider::new("mock").with_default("mock");
    let tracer = Tracer::with_capture();
    let executor = TaskExecutor::new(program, mock_registry(mock), Some(tracer.clone()));
    let _result = executor.run().await;
    let events = tracer.captured_events();
    assert!(
        events.contains(&"exec_call".to_string()),
        "should emit exec_call trace event, got: {:?}",
        events
    );
    assert!(
        events.contains(&"exec_return".to_string()),
        "should emit exec_return trace event, got: {:?}",
        events
    );
}

// ── CLI: forge check validates exec fixtures ────────────────────

#[test]
fn cli_check_exec_success_exits_zero() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_forge"))
        .args(["check", "examples/command/exec_success.forge"])
        .output()
        .expect("failed to execute forge binary");
    assert!(
        output.status.success(),
        "forge check on exec_success.forge should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn cli_check_exec_pure_error_exits_nonzero() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_forge"))
        .args(["check", "examples/errors/exec_pure_error.forge"])
        .output()
        .expect("failed to execute forge binary");
    assert!(
        !output.status.success(),
        "forge check on exec_pure_error.forge should exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("exec"),
        "error output should mention 'exec', got: {}",
        stderr
    );
}

#[test]
fn cli_check_exec_uncertain_error_exits_nonzero() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_forge"))
        .args(["check", "examples/errors/exec_uncertain_error.forge"])
        .output()
        .expect("failed to execute forge binary");
    assert!(
        !output.status.success(),
        "forge check on exec_uncertain_error.forge should exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("uncertain"),
        "error output should mention 'uncertain', got: {}",
        stderr
    );
}
