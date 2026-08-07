/// Tests for file.read inside agent event handlers without explicit boundary — issue #435

use forge::checker::boundary_checker;
use forge::parser::parse;

fn errors_for(src: &str) -> Vec<String> {
    let program = parse(src).expect("Failed to parse");
    let diagnostics = boundary_checker::check(&[( &program, "test.forge")]);
    diagnostics.into_iter().map(|d| d.message).collect()
}

#[test]
fn file_read_without_try_in_agent_handler_no_boundary() {
    // Default boundary is "shared" - file.read should be rejected
    let src = r#"
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
    // file.read should be rejected in shared boundary
    assert!(
        errs.iter().any(|e| e.contains("file.read") && e.contains("shared")),
        "file.read should be rejected in shared boundary, got: {:?}",
        errs
    );
}
