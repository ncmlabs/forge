// Tests for boundary client cap set — issue #250 (part of epic #249).
//
// A boundary: client program must be able to do HTTP egress (web.fetch/web.post)
// and read env vars (env.get — see #251), but must be locked out of the
// server-lifecycle capabilities: spawn, emit, data.*, endpoint, search.

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

// ── web.fetch / web.post: allowed in client boundary (the core of #250) ────

#[test]
fn client_boundary_allows_web_post() {
    let src = r#"#! boundary: client

use
  web.post

task call_server
  needs question: Text
  gives Text
  do
    response = web.post("http://localhost:3000/api/ask", question)
    give response
"#;
    let errs = errors_for(src);
    assert!(
        errs.is_empty(),
        "client boundary should allow web.post, got errors: {:?}",
        errs
    );
}

#[test]
fn client_boundary_allows_web_fetch() {
    let src = r#"#! boundary: client

use
  web.fetch

task call_server
  gives Text
  do
    response = web.fetch("http://localhost:3000/api/status")
    give response
"#;
    let errs = errors_for(src);
    assert!(
        errs.is_empty(),
        "client boundary should allow web.fetch, got errors: {:?}",
        errs
    );
}

#[test]
fn server_boundary_still_allows_web_post() {
    let src = r#"#! boundary: server

use
  web.post

task call_other
  gives Text
  do
    response = web.post("http://other-service/api", "payload")
    give response
"#;
    let errs = errors_for(src);
    assert!(
        errs.is_empty(),
        "server boundary should still allow web.post, got errors: {:?}",
        errs
    );
}

#[test]
fn shared_boundary_rejects_web_post() {
    let src = r#"#! boundary: shared

use
  web.post

task call_something
  gives Text
  do
    response = web.post("http://x/y", "z")
    give response
"#;
    let errs = errors_for(src);
    assert!(
        errs.iter()
            .any(|e| e.contains("web.post") && e.contains("shared")),
        "shared boundary should reject web.post, got errors: {:?}",
        errs
    );
}

// ── data.store / data.get: rejected in client boundary ─────────────────────

#[test]
fn client_boundary_rejects_data_store() {
    let src = r#"#! boundary: client

use
  data.store

task persist
  needs key: Text, value: Text
  gives Text
  do
    data.store(key, value)
    give "stored"
"#;
    let errs = errors_for(src);
    assert!(
        errs.iter()
            .any(|e| e.contains("data.store") && e.contains("client")),
        "client boundary should reject data.store, got errors: {:?}",
        errs
    );
}

#[test]
fn client_boundary_rejects_data_get() {
    let src = r#"#! boundary: client

use
  data.get

task read
  needs key: Text
  gives Text
  do
    value = data.get(key)
    give value
"#;
    let errs = errors_for(src);
    assert!(
        errs.iter()
            .any(|e| e.contains("data.get") && e.contains("client")),
        "client boundary should reject data.get, got errors: {:?}",
        errs
    );
}

// ── endpoints: rejected in client (unchanged from prior behavior) ─────────

#[test]
fn client_boundary_rejects_endpoint() {
    let src = r#"#! boundary: client

endpoint health() -> Text
  give "ok"
"#;
    let errs = errors_for(src);
    assert!(
        errs.iter()
            .any(|e| e.contains("endpoint") && e.contains("client")),
        "client boundary should reject endpoint decls, got errors: {:?}",
        errs
    );
}

// ── spawn: rejected in client boundary ────────────────────────────────────

#[test]
fn client_boundary_rejects_spawn() {
    let src = r#"#! boundary: client

agent specialist
  on start
    say "ready"

agent client_agent
  on invoke
    child = spawn specialist as "child_{1}"
    say "spawned"
"#;
    let errs = errors_for(src);
    assert!(
        errs.iter()
            .any(|e| e.contains("spawn") && e.contains("client")),
        "client boundary should reject spawn, got errors: {:?}",
        errs
    );
}

// ── emit: rejected in client boundary ─────────────────────────────────────

#[test]
fn client_boundary_rejects_emit() {
    let src = r#"#! boundary: client

event Insight
  topic: Text

agent client_agent
  on invoke
    emit Insight(topic: "test")
    say "emitted"
"#;
    let errs = errors_for(src);
    assert!(
        errs.iter()
            .any(|e| e.contains("emit") && e.contains("client")),
        "client boundary should reject emit, got errors: {:?}",
        errs
    );
}

// ── Combined: a realistic pure-FORGE client agent should pass cleanly ─────

#[test]
fn realistic_client_agent_compiles_cleanly() {
    let src = r#"#! boundary: client

use
  web.post
  web.fetch

agent forge_sensei_client
  on status
    response = web.fetch("http://127.0.0.1:3000/api/status")
    say response

  on query(question: Text)
    response = web.post("http://127.0.0.1:3000/api/ask", question)
    say response
"#;
    let errs = errors_for(src);
    assert!(
        errs.is_empty(),
        "realistic client agent should have no errors, got: {:?}",
        errs
    );
}
