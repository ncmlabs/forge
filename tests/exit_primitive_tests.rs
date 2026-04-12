// Tests for proc.exit runtime primitive — issue #258 (part of epic #249).
//
// proc.exit(code) raises RuntimeError::Exit(code), which the generated CLI
// dispatch (src/build.rs) translates to std::process::exit(code). In non-CLI
// contexts, it surfaces as an uncaught exit signal.

use forge::checker::boundary_checker;
use forge::diagnostic::DiagnosticKind;
use forge::runtime::executor::RuntimeError;

fn errors_for(src: &str) -> Vec<String> {
    let program = forge::parser::parse(src).expect("parse should succeed");
    let diags = boundary_checker::check(&[(&program, "test.forge")]);
    diags
        .into_iter()
        .filter(|d| matches!(d.kind, DiagnosticKind::Error))
        .map(|d| d.message)
        .collect()
}

// ── Boundary acceptance ────────────────────────────────────────────────────

#[test]
fn proc_exit_is_allowed_in_client_boundary() {
    let src = r#"#! boundary: client

use
  web.fetch
  env.get
  proc.exit

agent tiny_client
  on check
    base = env.get("SERVER", "http://127.0.0.1:3000")
    response = try web.fetch("{base}/api/status") or ""
    if response == ""
      say "unreachable"
      proc.exit(1)
    say "ok"
"#;
    let errs = errors_for(src);
    assert!(
        errs.is_empty(),
        "proc.exit should be allowed in client boundary, got errors: {:?}",
        errs
    );
}

// ── Runtime behaviour ──────────────────────────────────────────────────────

#[tokio::test]
async fn proc_exit_raises_exit_error_with_code() {
    use std::sync::Arc;

    let src = r#"fn main
  proc.exit(42)
"#;
    let program = forge::parser::parse(src).expect("parse should succeed");
    let config = forge::config::ForgeConfig::default_mock_config();
    let registry =
        Arc::new(forge::llm::registry::ProviderRegistry::from_config(config).expect("registry"));
    let executor = forge::runtime::executor::TaskExecutor::new(program, registry, None);
    let result = executor.run().await;
    match result {
        Err(RuntimeError::Exit(code)) => assert_eq!(code, 42),
        other => panic!("expected RuntimeError::Exit(42), got {:?}", other),
    }
}

#[tokio::test]
async fn proc_exit_zero_raises_exit_error() {
    use std::sync::Arc;

    let src = r#"fn main
  proc.exit(0)
"#;
    let program = forge::parser::parse(src).expect("parse should succeed");
    let config = forge::config::ForgeConfig::default_mock_config();
    let registry =
        Arc::new(forge::llm::registry::ProviderRegistry::from_config(config).expect("registry"));
    let executor = forge::runtime::executor::TaskExecutor::new(program, registry, None);
    let result = executor.run().await;
    match result {
        Err(RuntimeError::Exit(code)) => assert_eq!(code, 0),
        other => panic!("expected RuntimeError::Exit(0), got {:?}", other),
    }
}

#[tokio::test]
async fn proc_exit_clamps_out_of_range_codes() {
    use std::sync::Arc;

    let src = r#"fn main
  proc.exit(999)
"#;
    let program = forge::parser::parse(src).expect("parse should succeed");
    let config = forge::config::ForgeConfig::default_mock_config();
    let registry =
        Arc::new(forge::llm::registry::ProviderRegistry::from_config(config).expect("registry"));
    let executor = forge::runtime::executor::TaskExecutor::new(program, registry, None);
    let result = executor.run().await;
    match result {
        Err(RuntimeError::Exit(code)) => assert_eq!(code, 255),
        other => panic!(
            "expected RuntimeError::Exit(255) (clamped), got {:?}",
            other
        ),
    }
}
