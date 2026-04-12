// Tests for env.get runtime primitive — issue #251 (part of epic #249).
//
// env.get(name, default) reads an environment variable at runtime, falling back
// to `default` if unset. Usable in all boundaries (server, client, shared).

use forge::checker::boundary_checker;
use forge::diagnostic::DiagnosticKind;

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
fn env_get_is_allowed_in_client_boundary() {
    let src = r#"#! boundary: client

use
  env.get

task resolve_server
  gives Text
  do
    url = env.get("FORGE_SENSEI_SERVER", "http://127.0.0.1:3000")
    give url
"#;
    let errs = errors_for(src);
    assert!(
        errs.is_empty(),
        "env.get should be allowed in client boundary, got errors: {:?}",
        errs
    );
}

#[test]
fn env_get_is_allowed_in_server_boundary() {
    let src = r#"#! boundary: server

use
  env.get

task resolve_home
  gives Text
  do
    home = env.get("HOME", "/tmp")
    give home
"#;
    let errs = errors_for(src);
    assert!(
        errs.is_empty(),
        "env.get should be allowed in server boundary, got errors: {:?}",
        errs
    );
}

#[test]
fn env_get_is_allowed_in_shared_boundary() {
    let src = r#"#! boundary: shared

use
  env.get

task resolve_log_level
  gives Text
  do
    level = env.get("FORGE_LOG_LEVEL", "info")
    give level
"#;
    let errs = errors_for(src);
    assert!(
        errs.is_empty(),
        "env.get should be allowed in shared boundary, got errors: {:?}",
        errs
    );
}

// ── Runtime behaviour ──────────────────────────────────────────────────────

#[tokio::test]
async fn env_get_returns_set_value() {
    use std::sync::Arc;
    std::env::set_var("FORGE_TEST_ENV_VALUE", "hello-from-env");

    let src = r#"fn main
  value = env.get("FORGE_TEST_ENV_VALUE", "default")
  give value
"#;
    let program = forge::parser::parse(src).expect("parse should succeed");
    let config = forge::config::ForgeConfig::default_mock_config();
    let registry =
        Arc::new(forge::llm::registry::ProviderRegistry::from_config(config).expect("registry"));
    let executor = forge::runtime::executor::TaskExecutor::new(program, registry, None);
    let result = executor.run().await.expect("run should succeed");
    assert_eq!(format!("{}", result.value), "hello-from-env");

    std::env::remove_var("FORGE_TEST_ENV_VALUE");
}

#[tokio::test]
async fn env_get_returns_default_when_unset() {
    use std::sync::Arc;
    std::env::remove_var("FORGE_TEST_UNSET_VAR");

    let src = r#"fn main
  value = env.get("FORGE_TEST_UNSET_VAR", "fallback-value")
  give value
"#;
    let program = forge::parser::parse(src).expect("parse should succeed");
    let config = forge::config::ForgeConfig::default_mock_config();
    let registry =
        Arc::new(forge::llm::registry::ProviderRegistry::from_config(config).expect("registry"));
    let executor = forge::runtime::executor::TaskExecutor::new(program, registry, None);
    let result = executor.run().await.expect("run should succeed");
    assert_eq!(format!("{}", result.value), "fallback-value");
}
