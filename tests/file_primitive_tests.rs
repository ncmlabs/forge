// Tests for file.read / toml.parse / json.parse runtime primitives — issue #380.
//
// file.read(path) reads a UTF-8 file; failures surface as zero-confidence
// Text so `when sure / else` routes them (matching the skill.* pattern
// from #375). file.read is server-boundary only; toml.parse/json.parse are
// boundary-agnostic pure functions over Text.

use std::sync::Arc;

use forge::checker::boundary_checker;
use forge::diagnostic::DiagnosticKind;
use forge::runtime::confidence::Value;

fn errors_for(src: &str) -> Vec<String> {
    let program = forge::parser::parse(src).expect("parse should succeed");
    let diags = boundary_checker::check(&[(&program, "test.forge")]);
    diags
        .into_iter()
        .filter(|d| matches!(d.kind, DiagnosticKind::Error))
        .map(|d| d.message)
        .collect()
}

async fn run(src: &str) -> forge::runtime::confidence::ConfidentValue {
    let program = forge::parser::parse(src).expect("parse should succeed");
    let config = forge::config::ForgeConfig::default_mock_config();
    let registry =
        Arc::new(forge::llm::registry::ProviderRegistry::from_config(config).expect("registry"));
    let executor = forge::runtime::executor::TaskExecutor::new(program, registry, None);
    executor.run().await.expect("run should succeed")
}

/// Render a filesystem path so it can be embedded in a FORGE string
/// literal. Windows paths contain `\`, which FORGE parses as the start
/// of a `\n`/`\r`/`\t`/`\"`/`\\` template escape — anything else is a
/// syntax error. Forward slashes are portable: Windows `std::fs::*`
/// accepts them, so the path still resolves at runtime.
fn forge_path(p: &std::path::Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

// ── Boundary acceptance ────────────────────────────────────────────────────

#[test]
fn file_read_is_allowed_in_server_boundary() {
    let src = r#"#! boundary: server

use
  file.read

task load
  gives Text
  do
    contents = file.read("/tmp/forge-test-existing")
    give contents
"#;
    let errs = errors_for(src);
    assert!(
        errs.is_empty(),
        "file.read should be allowed in server boundary, got: {:?}",
        errs
    );
}

#[test]
fn file_read_is_rejected_in_client_boundary() {
    let src = r#"#! boundary: client

use
  file.read

task load
  gives Text
  do
    contents = file.read("/etc/passwd")
    give contents
"#;
    let errs = errors_for(src);
    assert!(
        errs.iter()
            .any(|e| e.contains("file.read") && e.contains("client")),
        "expected file.read to be rejected in client boundary, got: {:?}",
        errs
    );
}

#[test]
fn file_read_is_rejected_in_shared_boundary() {
    let src = r#"#! boundary: shared

use
  file.read

task load
  gives Text
  do
    contents = file.read("/etc/passwd")
    give contents
"#;
    let errs = errors_for(src);
    assert!(
        errs.iter()
            .any(|e| e.contains("file.read") && e.contains("shared")),
        "expected file.read to be rejected in shared boundary, got: {:?}",
        errs
    );
}

// ── file.read runtime behaviour ────────────────────────────────────────────

#[tokio::test]
async fn file_read_returns_contents_for_existing_file() {
    let dir = std::env::temp_dir().join(format!("forge-file-read-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("hello.txt");
    std::fs::write(&path, "hi from disk").unwrap();

    let src = format!(
        r#"fn main
  contents = file.read("{}")
  give contents
"#,
        forge_path(&path)
    );
    let result = run(&src).await;
    assert_eq!(format!("{}", result.value), "hi from disk");
    assert!(result.confidence >= 0.99);

    std::fs::remove_file(&path).ok();
    std::fs::remove_dir(&dir).ok();
}

#[tokio::test]
async fn file_read_missing_file_returns_zero_confidence() {
    let src = r#"fn main
  contents = file.read("/definitely/not/a/real/path/for/forge-test")
  give contents
"#;
    let result = run(src).await;
    assert_eq!(result.confidence, 0.0);
    let msg = format!("{}", result.value);
    assert!(
        msg.contains("file.read"),
        "message should cite file.read, got: {msg}"
    );
    assert!(
        msg.contains("/definitely/not/a/real/path"),
        "message should include the path, got: {msg}"
    );
}

// ── toml.parse runtime behaviour ───────────────────────────────────────────

#[tokio::test]
async fn toml_parse_happy_path() {
    let dir = std::env::temp_dir().join(format!("forge-toml-happy-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("cfg.toml");
    std::fs::write(
        &path,
        r#"
name = "svc"
port = 8080
"#,
    )
    .unwrap();

    let src = format!(
        r#"type Host
  name: Text
  port: Number

fn main
  contents = file.read("{}")
  host = toml.parse(contents, "Host")
  give host
"#,
        forge_path(&path)
    );
    let result = run(&src).await;
    if let Value::Record(fields) = &result.value {
        assert!(matches!(&fields["name"].value, Value::Text(s) if s == "svc"));
        assert!(matches!(&fields["port"].value, Value::Number(n) if *n == 8080.0));
    } else {
        panic!("expected Record, got {:?}", result.value);
    }

    std::fs::remove_file(&path).ok();
    std::fs::remove_dir(&dir).ok();
}

#[tokio::test]
async fn toml_parse_missing_field_uses_type_default() {
    let src = r#"type Cfg
  host: Text
  port: Number

fn main
  cfg = toml.parse("host = \"prod\"", "Cfg")
  give cfg
"#;
    let result = run(src).await;
    if let Value::Record(fields) = &result.value {
        assert!(matches!(&fields["host"].value, Value::Text(s) if s == "prod"));
        assert!(matches!(&fields["port"].value, Value::Number(n) if *n == 0.0));
    } else {
        panic!();
    }
}

#[tokio::test]
async fn toml_parse_wrong_shape_returns_zero_confidence() {
    let src = r#"type Cfg
  port: Number

fn main
  cfg = toml.parse("port = \"not-a-number\"", "Cfg")
  give cfg
"#;
    let result = run(src).await;
    assert_eq!(result.confidence, 0.0);
    let msg = format!("{}", result.value);
    assert!(
        msg.contains("port"),
        "error should cite the field, got: {msg}"
    );
}

#[tokio::test]
async fn toml_parse_unknown_type_returns_zero_confidence() {
    let src = r#"fn main
  cfg = toml.parse("x = 1", "NeverDeclared")
  give cfg
"#;
    let result = run(src).await;
    assert_eq!(result.confidence, 0.0);
    let msg = format!("{}", result.value);
    assert!(
        msg.contains("NeverDeclared"),
        "error should cite the type, got: {msg}"
    );
}

#[tokio::test]
async fn toml_parse_invalid_syntax_returns_zero_confidence() {
    let src = r#"type Cfg
  x: Text

fn main
  cfg = toml.parse("!! not toml ::", "Cfg")
  give cfg
"#;
    let result = run(src).await;
    assert_eq!(result.confidence, 0.0);
    let msg = format!("{}", result.value);
    assert!(msg.contains("toml parse error"), "got: {msg}");
}

#[tokio::test]
async fn toml_parse_nested_record() {
    let src = r#"type Outer
  label: Text
  inner: Inner

type Inner
  count: Number

fn main
  cfg = toml.parse("label = \"top\"\n[inner]\ncount = 5\n", "Outer")
  give cfg
"#;
    let result = run(src).await;
    if let Value::Record(fields) = &result.value {
        if let Value::Record(inner) = &fields["inner"].value {
            assert!(matches!(&inner["count"].value, Value::Number(n) if *n == 5.0));
        } else {
            panic!("inner should be Record");
        }
    } else {
        panic!();
    }
}

// ── json.parse runtime behaviour ───────────────────────────────────────────

// JSON literals embed braces, which FORGE string templates reserve for
// interpolation — no `\{` escape exists. The tests below stage the JSON
// in a tempfile and read it through `file.read` the same way real code
// would use these primitives together.

#[tokio::test]
async fn json_parse_happy_path() {
    let dir = std::env::temp_dir().join(format!("forge-json-happy-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("host.json");
    std::fs::write(&path, r#"{"name":"svc","port":8080}"#).unwrap();

    let src = format!(
        r#"type Host
  name: Text
  port: Number

fn main
  contents = file.read("{}")
  host = json.parse(contents, "Host")
  give host
"#,
        forge_path(&path)
    );
    let result = run(&src).await;
    if let Value::Record(fields) = &result.value {
        assert!(matches!(&fields["name"].value, Value::Text(s) if s == "svc"));
        assert!(matches!(&fields["port"].value, Value::Number(n) if *n == 8080.0));
    } else {
        panic!("expected Record, got {:?}", result.value);
    }

    std::fs::remove_file(&path).ok();
    std::fs::remove_dir(&dir).ok();
}

#[tokio::test]
async fn json_parse_invalid_returns_zero_confidence() {
    let dir = std::env::temp_dir().join(format!("forge-json-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bad.json");
    std::fs::write(&path, "this is not json").unwrap();

    let src = format!(
        r#"type Cfg
  x: Text

fn main
  contents = file.read("{}")
  cfg = json.parse(contents, "Cfg")
  give cfg
"#,
        forge_path(&path)
    );
    let result = run(&src).await;
    assert_eq!(result.confidence, 0.0);
    let msg = format!("{}", result.value);
    assert!(msg.contains("json parse error"), "got: {msg}");

    std::fs::remove_file(&path).ok();
    std::fs::remove_dir(&dir).ok();
}
