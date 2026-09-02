/// Tests for file.read inside agent event handlers — issue #435
use forge::checker::boundary_checker;
use forge::parser::parse;

fn errors_for(src: &str) -> Vec<String> {
    let program = parse(src).expect("Failed to parse");
    let diagnostics = boundary_checker::check(&[(&program, "test.forge")]);
    diagnostics.into_iter().map(|d| d.message).collect()
}

#[test]
fn file_read_is_allowed_in_agent_handler_server_boundary() {
    let src = r#"#! boundary: server

use
  file.read

exportable agent dbg
  on start
    r = try file.read("/etc/hostname") or "FALLBACK"
    say "R={r}"

fn main
  a = spawn dbg as "dbg"
"#;
    let errs = errors_for(src);
    assert!(
        errs.is_empty(),
        "file.read should be allowed in agent handler with server boundary, got: {:?}",
        errs
    );
}

#[test]
fn file_read_without_try_in_agent_handler_server_boundary() {
    let src = r#"#! boundary: server

use
  file.read

exportable agent dbg
  on start
    r = file.read("/etc/hostname")
    say "R={r}"

fn main
  a = spawn dbg as "dbg"
"#;
    let errs = errors_for(src);
    assert!(
        errs.is_empty(),
        "file.read without try should be allowed in agent handler with server boundary, got: {:?}",
        errs
    );
}
