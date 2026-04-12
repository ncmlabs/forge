// Tests for FORGE boundary checker (issue #21)

use forge::diagnostic::{Diagnostic, DiagnosticKind};
use forge::parser::parse;

/// Parse multiple (source, filename) pairs and run boundary checker.
fn check_boundary(sources: &[(&str, &str)]) -> Vec<Diagnostic> {
    let parsed: Vec<_> = sources
        .iter()
        .map(|(src, name)| {
            let program = parse(src).unwrap_or_else(|_| panic!("parse failed for {}", name));
            (program, name.to_string())
        })
        .collect();
    let refs: Vec<_> = parsed.iter().map(|(p, n)| (p, n.as_str())).collect();
    forge::checker::boundary_checker::check(&refs)
}

fn errors(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
    diags
        .iter()
        .filter(|d| matches!(d.kind, DiagnosticKind::Error))
        .collect()
}

#[allow(dead_code)]
fn warnings(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
    diags
        .iter()
        .filter(|d| matches!(d.kind, DiagnosticKind::Warning))
        .collect()
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

// ── Shared type serializability ─────────────────────────────

#[test]
fn shared_type_with_agent_field_is_error() {
    let shared = "\
#! boundary: shared

type Session
  user: Text
  handler: MyAgent
";
    let server = "\
#! boundary: server

agent MyAgent
  on ping(msg: Text)
    say msg
";
    let diags = check_boundary(&[(shared, "shared.forge"), (server, "server.forge")]);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("Session"));
    assert!(errs[0].message.contains("handler"));
}

#[test]
fn session_expression_recurses_into_nested_references() {
    let server = "\
#! boundary: server

pure server_only
  needs input: Text
  gives Text
  do
    give input
";
    let client = "\
#! boundary: client

event ReviewUpdate
  payload: Text

task review
  needs input: Text
  gives Text
  do
    prompt_text = \"review {input}\"
    result = session \"code-review\" prompt prompt_text tools [server_only(input)] budget 2.0 on progress -> emit ReviewUpdate(server_only(input))
    give result
";
    let diags = check_boundary(&[(server, "server.forge"), (client, "client.forge")]);
    let errs = errors(&diags);
    assert!(
        errs.iter().any(|d| d.message.contains("server_only")),
        "expected boundary checker to recurse into session subexpressions: {:?}",
        errs.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

#[test]
fn shared_type_with_pool_field_is_error() {
    let shared = "\
#! boundary: shared

type Config
  name: Text
  workers: MyPool
";
    let server = "\
#! boundary: server

task worker
  needs x: Text
  gives Text
  do
    give x

pool MyPool
  workers: worker * 3
  strategy: fastest
";
    let diags = check_boundary(&[(shared, "shared.forge"), (server, "server.forge")]);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("Config"));
    assert!(errs[0].message.contains("workers"));
}

#[test]
fn shared_type_with_only_primitive_fields_is_ok() {
    let shared = "\
#! boundary: shared

type Message
  content: Text
  count: Number
  valid: Bool
";
    let diags = check_boundary(&[(shared, "shared.forge")]);
    assert!(diags.is_empty());
}

#[test]
fn shared_event_with_agent_field_is_error() {
    let shared = "\
#! boundary: shared

event Signal
  payload: MyAgent
";
    let server = "\
#! boundary: server

agent MyAgent
  on ping(msg: Text)
    say msg
";
    let diags = check_boundary(&[(shared, "shared.forge"), (server, "server.forge")]);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("Signal"));
    assert!(errs[0].message.contains("payload"));
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

// ── Acceptance criteria ─────────────────────────────────────

#[test]
fn server_agent_invisible_to_client() {
    let server = "\
#! boundary: server

agent SecretAgent
  on process(data: Text)
    say data
";
    let client = "\
#! boundary: client

task show
  needs input: Text
  gives Text
  do
    result = SecretAgent(input)
    give result
";
    let diags = check_boundary(&[(server, "server.forge"), (client, "client.forge")]);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("SecretAgent"));
    assert!(errs[0].message.contains("server"));
}

#[test]
fn file_without_boundary_defaults_to_shared() {
    // No boundary = shared. Shared cannot reference server symbols.
    let server = "\
#! boundary: server

task secret
  needs x: Text
  gives Text
  do
    give x
";
    let no_boundary = "\
task caller
  needs x: Text
  gives Text
  do
    result = secret(x)
    give result
";
    let diags = check_boundary(&[(server, "server.forge"), (no_boundary, "utils.forge")]);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("secret"));
    assert_eq!(errs[0].file, "utils.forge");
}

#[test]
fn all_shared_files_no_boundary_violations() {
    let file1 = "\
#! boundary: shared

pure helper
  needs x: Text
  gives Text
  do
    give x
";
    let file2 = "\
#! boundary: shared

task process
  needs x: Text
  gives Text
  do
    result = helper(x)
    give result
";
    let diags = check_boundary(&[(file1, "helpers.forge"), (file2, "main.forge")]);
    assert!(errors(&diags).is_empty());
}

#[test]
fn client_only_code_no_errors() {
    let client = "\
#! boundary: client

task render
  needs data: Text
  gives Text
  do
    give data

pure format
  needs x: Text
  gives Text
  do
    give x
";
    let diags = check_boundary(&[(client, "client.forge")]);
    assert!(diags.is_empty());
}

#[test]
fn minimal_file_no_errors() {
    // A minimal file with no boundary directive and no cross-boundary refs
    let source = "\
task noop
  needs x: Text
  gives Text
  do
    give x
";
    let diags = check_boundary(&[(source, "minimal.forge")]);
    assert!(diags.is_empty());
}

// ── HTTP client boundary enforcement (issue #51) ────────────

#[test]
fn web_fetch_in_client_boundary_is_ok() {
    // Updated for #250: web.fetch is a legitimate client capability for pure-FORGE
    // HTTP clients talking to a server. Only shared boundary still rejects it.
    let source = "\
#! boundary: client

fn main
  page = web.fetch(\"https://example.com\")
  give page
";
    let diags = check_boundary(&[(source, "client.forge")]);
    let errs = errors(&diags);
    assert!(
        errs.is_empty(),
        "web.fetch should be allowed in client boundary (#250), got: {:?}",
        errs
    );
}

#[test]
fn web_post_in_client_boundary_is_ok() {
    // Updated for #250: see above.
    let source = "\
#! boundary: client

fn main
  resp = web.post(\"https://example.com\", \"body\")
  give resp
";
    let diags = check_boundary(&[(source, "client.forge")]);
    let errs = errors(&diags);
    assert!(
        errs.is_empty(),
        "web.post should be allowed in client boundary (#250), got: {:?}",
        errs
    );
}

#[test]
fn web_fetch_in_shared_boundary_is_error() {
    // Added for #250: shared is the one boundary where web.* is not allowed
    // (shared is for types + pure code that both sides import).
    let source = "\
#! boundary: shared

fn main
  page = web.fetch(\"https://example.com\")
  give page
";
    let diags = check_boundary(&[(source, "shared.forge")]);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("web.fetch()"));
    assert!(errs[0].message.contains("shared"));
}

#[test]
fn web_fetch_in_server_boundary_is_ok() {
    let source = "\
#! boundary: server

fn main
  page = web.fetch(\"https://example.com\")
  give page
";
    let diags = check_boundary(&[(source, "server.forge")]);
    assert!(diags.is_empty());
}

#[test]
fn web_post_in_server_boundary_is_ok() {
    let source = "\
#! boundary: server

fn main
  resp = web.post(\"https://example.com\", \"body\")
  give resp
";
    let diags = check_boundary(&[(source, "server.forge")]);
    assert!(diags.is_empty());
}

#[test]
fn search_in_client_boundary_is_error() {
    let source = "\
#! boundary: client

fn main
  results = search \"rust programming\"
  give results
";
    let diags = check_boundary(&[(source, "client.forge")]);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("search"));
    assert!(errs[0].message.contains("client"));
}

#[test]
fn search_in_server_boundary_is_ok() {
    let source = "\
#! boundary: server

fn main
  results = search \"rust programming\"
  give results
";
    let diags = check_boundary(&[(source, "server.forge")]);
    assert!(diags.is_empty());
}
