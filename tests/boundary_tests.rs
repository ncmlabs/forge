// Tests for FORGE boundary checker (issue #21)

use forge::diagnostic::{Diagnostic, DiagnosticKind};
use forge::parser::parse;

/// Parse multiple (source, filename) pairs and run boundary checker.
fn check_boundary(sources: &[(&str, &str)]) -> Vec<Diagnostic> {
    let parsed: Vec<_> = sources
        .iter()
        .map(|(src, name)| {
            let program = parse(src).expect(&format!("parse failed for {}", name));
            (program, name.to_string())
        })
        .collect();
    let refs: Vec<_> = parsed.iter().map(|(p, n)| (p, n.as_str())).collect();
    forge::checker::boundary_checker::check(&refs)
}

fn errors(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
    diags.iter().filter(|d| matches!(d.kind, DiagnosticKind::Error)).collect()
}

fn warnings(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
    diags.iter().filter(|d| matches!(d.kind, DiagnosticKind::Warning)).collect()
}

// ── Endpoint placement ──────────────────────────────────────

#[test]
fn endpoint_in_client_boundary_is_error() {
    let source = "\
#! boundary: client

endpoint login(user: Text, pass: Text)
  give \"ok\"
";
    let diags = check_boundary(&[(source, "client.forge")]);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("login"));
    assert!(errs[0].message.contains("client"));
}

#[test]
fn endpoint_in_shared_boundary_is_error() {
    let source = "\
#! boundary: shared

endpoint health()
  give \"ok\"
";
    let diags = check_boundary(&[(source, "shared.forge")]);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("health"));
    assert!(errs[0].message.contains("shared"));
}

#[test]
fn endpoint_in_server_boundary_is_ok() {
    let source = "\
#! boundary: server

endpoint health()
  give \"ok\"
";
    let diags = check_boundary(&[(source, "server.forge")]);
    assert!(diags.is_empty());
}

#[test]
fn endpoint_in_file_without_boundary_is_error() {
    // No boundary directive = defaults to shared
    let source = "\
endpoint health()
  give \"ok\"
";
    let diags = check_boundary(&[(source, "no_boundary.forge")]);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("shared"));
}

// ── Cross-boundary reference checks ─────────────────────────

#[test]
fn client_referencing_server_task_is_error() {
    let server = "\
#! boundary: server

task process_secret
  needs data: Text
  gives Text
  do
    give data
";
    let client = "\
#! boundary: client

task show_ui
  needs input: Text
  gives Text
  do
    result = process_secret(input)
    give result
";
    let diags = check_boundary(&[(server, "server.forge"), (client, "client.forge")]);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("process_secret"));
    assert!(errs[0].message.contains("server"));
    assert_eq!(errs[0].file, "client.forge");
}

#[test]
fn server_referencing_client_declaration_is_error() {
    let client = "\
#! boundary: client

pure render_ui
  needs data: Text
  gives Text
  do
    give data
";
    let server = "\
#! boundary: server

task process
  needs input: Text
  gives Text
  do
    result = render_ui(input)
    give result
";
    let diags = check_boundary(&[(client, "client.forge"), (server, "server.forge")]);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("render_ui"));
    assert!(errs[0].message.contains("client"));
    assert_eq!(errs[0].file, "server.forge");
}

#[test]
fn shared_type_accessible_from_server_and_client() {
    let shared = "\
#! boundary: shared

type Message
  content: Text
  sender: Text
";
    let server = "\
#! boundary: server

task process
  needs msg: Text
  gives Text
  do
    m = Message(content: msg, sender: \"system\")
    give msg
";
    let client = "\
#! boundary: client

task display
  needs msg: Text
  gives Text
  do
    m = Message(content: msg, sender: \"user\")
    give msg
";
    let diags = check_boundary(&[
        (shared, "shared.forge"),
        (server, "server.forge"),
        (client, "client.forge"),
    ]);
    assert!(errors(&diags).is_empty());
}

#[test]
fn same_boundary_references_are_ok() {
    let server1 = "\
#! boundary: server

pure validate
  needs x: Text
  gives Bool
  do
    give true
";
    let server2 = "\
#! boundary: server

task process
  needs input: Text
  gives Text
  do
    ok = validate(input)
    give input
";
    let diags = check_boundary(&[(server1, "server1.forge"), (server2, "server2.forge")]);
    assert!(errors(&diags).is_empty());
}
