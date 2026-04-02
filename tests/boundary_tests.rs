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
