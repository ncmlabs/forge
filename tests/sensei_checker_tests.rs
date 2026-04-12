// FORGE forge-sensei checker tests
// Validates that the checker produces zero errors on the split sensei program
// and catches purity violations as expected.

use forge::checker;
use forge::checker::boundary_checker;
use forge::diagnostic::{Diagnostic, DiagnosticKind};

fn sensei_server_sources() -> Vec<forge::compose::SourceFile> {
    [
        "workflows/forge-sensei/shared/types.forge",
        "workflows/forge-sensei/shared/events.forge",
        "workflows/forge-sensei/shared/states.forge",
        "workflows/forge-sensei/server/tasks.forge",
        "workflows/forge-sensei/server/flows.forge",
        "workflows/forge-sensei/server/agent.forge",
        "workflows/forge-sensei/server/web.forge",
    ]
    .into_iter()
    .map(|path| {
        let source = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("could not read {}: {}", path, e));
        let program = forge::parser::parse(&source)
            .unwrap_or_else(|e| panic!("parse failed for {}: {:?}", path, e));
        forge::compose::SourceFile {
            path: path.to_string(),
            source,
            program,
        }
    })
    .collect()
}

fn check_sensei_server() -> Vec<Diagnostic> {
    let source_files = sensei_server_sources();
    let refs = source_files
        .iter()
        .map(|sf| (&sf.program, sf.path.as_str()))
        .collect::<Vec<_>>();
    let composed =
        forge::compose::merge_programs(&source_files).expect("sensei server files should merge");
    let mut diags =
        checker::check_all(&composed.program, "workflows/forge-sensei/server/web.forge");
    diags.extend(boundary_checker::check(&refs));
    diags
}

fn errors(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
    diags
        .iter()
        .filter(|d| matches!(d.kind, DiagnosticKind::Error))
        .collect()
}

fn warnings(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
    diags
        .iter()
        .filter(|d| matches!(d.kind, DiagnosticKind::Warning))
        .collect()
}

// ── Test 1: zero checker errors on split forge-sensei server ─────

#[test]
fn check_sensei_zero_errors() {
    let diags = check_sensei_server();
    let errs = errors(&diags);
    if !errs.is_empty() {
        for e in &errs {
            eprintln!("ERROR: {} — {}", e.message, e.label);
        }
    }
    assert!(
        errs.is_empty(),
        "split forge-sensei server should produce zero checker errors, found {}",
        errs.len()
    );
}

// ── Test 2: warnings inventory ──────────────────────────────────

#[test]
fn check_sensei_warnings_inventory() {
    let diags = check_sensei_server();
    let warns = warnings(&diags);

    // Print warnings for documentation
    if !warns.is_empty() {
        eprintln!("--- split forge-sensei warnings ({}) ---", warns.len());
        for w in &warns {
            eprintln!("  WARN: {} — {}", w.message, w.label);
        }
        eprintln!("--- end warnings ---");
    }

    assert!(
        warns.len() <= 10,
        "expected at most 10 warnings, found {}",
        warns.len()
    );
}

// ── Test 3: purity exhaustive — reason in pure must error ───────

#[test]
fn check_sensei_purity_exhaustive() {
    let src = r#"pure bad_pure
  needs x: Text
  gives Text
  do
    give reason "think about {x}"
"#;
    let program =
        forge::parser::parse(src).expect("parse of bad pure should succeed syntactically");
    let diags = checker::check_all(&program, "purity_test.forge");
    let errs = errors(&diags);

    assert!(
        !errs.is_empty(),
        "a pure function using `reason` should produce at least one checker error"
    );
    assert!(
        errs.iter().any(|d| {
            let msg = d.message.to_lowercase();
            msg.contains("pure") || msg.contains("cannot use") || msg.contains("reason")
        }),
        "error should mention purity violation; got: {:?}",
        errs.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}
