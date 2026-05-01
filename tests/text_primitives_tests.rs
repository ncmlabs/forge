// Tests for text.replace and text.short_id runtime primitives — issue #360
// (T8.5). text.replace is needed because FORGE's `{var}` interpolation is
// parse-time only; templates loaded from TOML at runtime must be substituted
// explicitly. text.short_id supplies the workdir collision suffix the
// dev-cycle workflow needs when two clones land on the same {repo,issue} pair.

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
fn text_replace_is_allowed_in_all_boundaries() {
    let template = r#"
use
  text.replace

task render
  gives Text
  do
    out = text.replace("hello \{name\}", "\{name\}", "world")
    give out
"#;
    for boundary in ["server", "client", "shared"] {
        let src = format!("#! boundary: {boundary}\n{template}");
        let errs = errors_for(&src);
        assert!(
            errs.is_empty(),
            "text.replace should be allowed in {boundary} boundary, got errors: {errs:?}"
        );
    }
}

#[test]
fn text_short_id_is_allowed_in_all_boundaries() {
    let template = r#"
use
  text.short_id

task uid
  gives Text
  do
    id = text.short_id()
    give id
"#;
    for boundary in ["server", "client", "shared"] {
        let src = format!("#! boundary: {boundary}\n{template}");
        let errs = errors_for(&src);
        assert!(
            errs.is_empty(),
            "text.short_id should be allowed in {boundary} boundary, got errors: {errs:?}"
        );
    }
}

// ── Runtime behaviour ──────────────────────────────────────────────────────

#[tokio::test]
async fn text_replace_substitutes_every_occurrence() {
    use std::sync::Arc;
    let src = r#"fn main
  out = text.replace("a-\{x\}-b-\{x\}-c", "\{x\}", "Z")
  give out
"#;
    let program = forge::parser::parse(src).expect("parse");
    let config = forge::config::ForgeConfig::default_mock_config();
    let registry =
        Arc::new(forge::llm::registry::ProviderRegistry::from_config(config).expect("registry"));
    let executor = forge::runtime::executor::TaskExecutor::new(program, registry, None);
    let result = executor.run().await.expect("run");
    assert_eq!(format!("{}", result.value), "a-Z-b-Z-c");
}

#[tokio::test]
async fn text_replace_with_empty_needle_is_noop() {
    use std::sync::Arc;
    let src = r#"fn main
  out = text.replace("hello", "", "X")
  give out
"#;
    let program = forge::parser::parse(src).expect("parse");
    let config = forge::config::ForgeConfig::default_mock_config();
    let registry =
        Arc::new(forge::llm::registry::ProviderRegistry::from_config(config).expect("registry"));
    let executor = forge::runtime::executor::TaskExecutor::new(program, registry, None);
    let result = executor.run().await.expect("run");
    assert_eq!(format!("{}", result.value), "hello");
}

#[tokio::test]
async fn text_replace_chains_for_template_rendering() {
    use std::sync::Arc;
    // Models the dev-cycle commit-template render: replace three placeholders
    // by chaining text.replace calls. This is the actual T8.5 use case.
    let src = r#"fn main
  template = "feat(\{issue_id\}): \{title\}"
  step1 = text.replace(template, "\{issue_id\}", "360")
  step2 = text.replace(step1, "\{title\}", "config-driven dev-cycle")
  give step2
"#;
    let program = forge::parser::parse(src).expect("parse");
    let config = forge::config::ForgeConfig::default_mock_config();
    let registry =
        Arc::new(forge::llm::registry::ProviderRegistry::from_config(config).expect("registry"));
    let executor = forge::runtime::executor::TaskExecutor::new(program, registry, None);
    let result = executor.run().await.expect("run");
    assert_eq!(
        format!("{}", result.value),
        "feat(360): config-driven dev-cycle"
    );
}

#[tokio::test]
async fn text_short_id_returns_eight_hex_chars() {
    use std::sync::Arc;
    let src = r#"fn main
  id = text.short_id()
  give id
"#;
    let program = forge::parser::parse(src).expect("parse");
    let config = forge::config::ForgeConfig::default_mock_config();
    let registry =
        Arc::new(forge::llm::registry::ProviderRegistry::from_config(config).expect("registry"));
    let executor = forge::runtime::executor::TaskExecutor::new(program, registry, None);
    let result = executor.run().await.expect("run");
    let out = format!("{}", result.value);
    assert_eq!(out.len(), 8, "short_id should be 8 chars, got: {out}");
    assert!(
        out.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
        "short_id should be lowercase hex, got: {out}"
    );
}

#[tokio::test]
async fn text_short_id_yields_distinct_values() {
    use std::sync::Arc;
    // Collision suffix only useful if successive calls don't collide. 16
    // calls × 8 hex chars ⇒ collision probability ≈ negligible.
    let src = r#"fn main
  ids = []
  ids = ids + [text.short_id()]
  ids = ids + [text.short_id()]
  ids = ids + [text.short_id()]
  ids = ids + [text.short_id()]
  give ids
"#;
    let program = forge::parser::parse(src).expect("parse");
    let config = forge::config::ForgeConfig::default_mock_config();
    let registry =
        Arc::new(forge::llm::registry::ProviderRegistry::from_config(config).expect("registry"));
    let executor = forge::runtime::executor::TaskExecutor::new(program, registry, None);
    let result = executor.run().await.expect("run");
    let out = format!("{}", result.value);
    // Crude uniqueness check: the rendered list shouldn't contain duplicates.
    let parts: Vec<&str> = out
        .trim_matches(|c| c == '[' || c == ']')
        .split(", ")
        .collect();
    assert_eq!(parts.len(), 4);
    let mut sorted = parts.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        4,
        "expected 4 distinct short_ids, got duplicates in: {out}"
    );
}
