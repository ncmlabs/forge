// FORGE command background mode acceptance tests — issue #162
// Proves background execution works: spawn returns handle, status polling,
// output buffering, cancel, timeout, and cleanup.

use std::sync::{Arc, Mutex};

use forge::llm::providers::mock::MockProvider;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::command_manager::CommandManager;
use forge::runtime::executor::TaskExecutor;
use forge::tracer::Tracer;

// ── Helpers ──────────────────────────────────────────────────────

fn parse_file(path: &str) -> forge::ast::Program {
    let source =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("could not read {}: {}", path, e));
    forge::parser::parse(&source).unwrap_or_else(|e| panic!("parse failed for {}: {:?}", path, e))
}

fn parse_source(src: &str) -> forge::ast::Program {
    forge::parser::parse(src).unwrap_or_else(|e| panic!("parse failed: {:?}", e))
}

fn mock_registry(mock: MockProvider) -> Arc<ProviderRegistry> {
    let mut reg = ProviderRegistry::new("mock");
    reg.register("mock", Arc::new(mock));
    Arc::new(reg)
}

fn executor_with_bg(program: forge::ast::Program) -> TaskExecutor {
    let mock = MockProvider::new("mock").with_default("mock");
    let mgr = Arc::new(Mutex::new(CommandManager::new()));
    TaskExecutor::new(program, mock_registry(mock), None).with_command_manager(mgr)
}

fn executor_with_bg_and_tracer(program: forge::ast::Program, tracer: Tracer) -> TaskExecutor {
    let mock = MockProvider::new("mock").with_default("mock");
    let mgr = Arc::new(Mutex::new(CommandManager::new()));
    TaskExecutor::new(program, mock_registry(mock), Some(tracer)).with_command_manager(mgr)
}

// ── Background: spawn returns handle ────────────────────────────

#[tokio::test]
#[cfg(not(target_os = "windows"))]
async fn background_spawn_returns_handle() {
    let program = parse_source(
        r#"
task run
  gives Text
  do
    handle = command "echo hello" background true timeout 10s
    give handle

fn main
  say run()
"#,
    );
    let executor = executor_with_bg(program);
    let result = executor.run().await;
    assert!(
        result.is_ok(),
        "background spawn should succeed: {:?}",
        result.err()
    );
    let outputs = executor.outputs();
    // Handle should be a UUID (36 chars with dashes)
    assert!(
        outputs.iter().any(|o| o.len() == 36 && o.contains('-')),
        "should return a UUID handle, got: {:?}",
        outputs
    );
}

// ── Background: status shows completed after process exits ──────

#[tokio::test]
#[cfg(not(target_os = "windows"))]
async fn background_status_completed() {
    let program = parse_file("examples/command/command_background.forge");
    let executor = executor_with_bg(program);
    let result = executor.run().await;
    assert!(
        result.is_ok(),
        "command_background.forge should run without error: {:?}",
        result.err()
    );
    let outputs = executor.outputs();
    assert!(
        outputs.iter().any(|o| o.contains("completed")),
        "status should show 'completed', got: {:?}",
        outputs
    );
    assert!(
        outputs.iter().any(|o| o.contains("bg_hello")),
        "output should contain 'bg_hello', got: {:?}",
        outputs
    );
}

// ── Background: cancel terminates process ───────────────────────

#[tokio::test]
#[cfg(not(target_os = "windows"))]
async fn background_cancel() {
    let program = parse_file("examples/command/command_background_cancel.forge");
    let executor = executor_with_bg(program);
    let result = executor.run().await;
    assert!(
        result.is_ok(),
        "command_background_cancel.forge should run without error: {:?}",
        result.err()
    );
    let outputs = executor.outputs();
    assert!(
        outputs.iter().any(|o| o.contains("cancelled")),
        "status should show 'cancelled', got: {:?}",
        outputs
    );
}

// ── Background: output buffering ────────────────────────────────

#[tokio::test]
#[cfg(not(target_os = "windows"))]
async fn background_output_buffered() {
    let program = parse_source(
        r#"
task run
  gives Text
  do
    handle = command ["sh", "-c", "echo line1; echo line2; echo line3"] background true timeout 10s
    wait = command "sleep 0.3"
    output = command.output(handle)
    when output.sure -> give output.stdout
    else -> give "no_output"

fn main
  say run()
"#,
    );
    let executor = executor_with_bg(program);
    let result = executor.run().await;
    assert!(
        result.is_ok(),
        "output buffering should succeed: {:?}",
        result.err()
    );
    let outputs = executor.outputs();
    assert!(
        outputs
            .iter()
            .any(|o| o.contains("line1") && o.contains("line2") && o.contains("line3")),
        "should capture all output lines, got: {:?}",
        outputs
    );
}

// ── Background: timeout auto-cancels ────────────────────────────

#[tokio::test]
#[cfg(not(target_os = "windows"))]
async fn background_timeout() {
    let program = parse_source(
        r#"
task run
  gives Text
  do
    handle = command "sleep 60" background true timeout 1s
    wait = command "sleep 2"
    status = command.status(handle)
    when status.sure -> give status.status
    else -> give "unknown"

fn main
  say run()
"#,
    );
    let executor = executor_with_bg(program);
    let result = executor.run().await;
    assert!(
        result.is_ok(),
        "timeout test should succeed: {:?}",
        result.err()
    );
    let outputs = executor.outputs();
    assert!(
        outputs.iter().any(|o| o.contains("timed_out")),
        "status should show 'timed_out', got: {:?}",
        outputs
    );
}

// ── Background: tracing events ──────────────────────────────────

#[tokio::test]
#[cfg(not(target_os = "windows"))]
async fn background_produces_trace_events() {
    let program = parse_source(
        r#"
task run
  gives Text
  do
    handle = command "echo traced" background true timeout 10s
    give handle

fn main
  say run()
"#,
    );
    let tracer = Tracer::with_capture();
    let executor = executor_with_bg_and_tracer(program, tracer.clone());
    let _result = executor.run().await;
    let events = tracer.captured_events();
    assert!(
        events.contains(&"command_call".to_string()),
        "should emit command_call trace event, got: {:?}",
        events
    );
    assert!(
        events.contains(&"command_bg_spawn".to_string()),
        "should emit command_bg_spawn trace event, got: {:?}",
        events
    );
}

// ── Background: shutdown_all kills running processes ────────────

#[tokio::test]
#[cfg(not(target_os = "windows"))]
async fn background_shutdown_all() {
    let mgr = Arc::new(Mutex::new(CommandManager::new()));

    // Spawn a long-running process directly via the manager
    let mut child_cmd = tokio::process::Command::new("sleep");
    child_cmd.arg("60");
    child_cmd.stdout(std::process::Stdio::piped());
    child_cmd.stderr(std::process::Stdio::piped());
    let child = child_cmd.spawn().unwrap();

    let handle = mgr
        .lock()
        .unwrap()
        .spawn_background(child, "sleep 60".to_string(), None, None)
        .unwrap();

    // Verify it's running
    {
        let status = mgr.lock().unwrap().status(&handle).unwrap();
        let status_text = format!("{}", status.value);
        assert!(
            status_text.contains("running"),
            "should be running initially, got: {}",
            status_text
        );
    }

    // Shutdown all
    mgr.lock().unwrap().shutdown_all();

    // Give time for the kill to propagate
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Verify it's no longer running
    {
        let status = mgr.lock().unwrap().status(&handle).unwrap();
        let status_text = format!("{}", status.value);
        assert!(
            !status_text.contains("running"),
            "should not be running after shutdown, got: {}",
            status_text
        );
    }
}
