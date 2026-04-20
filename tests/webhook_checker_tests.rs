// FORGE webhook checker tests — issue #335
// Covers each CheckError variant plus the canonical green path.

use forge::checker::webhook_checker;
use forge::diagnostic::{Diagnostic, DiagnosticKind};
use forge::parser::parse;

fn check(source: &str) -> Vec<Diagnostic> {
    let program = parse(source).expect("parse must succeed");
    webhook_checker::check(&program, "test.forge")
}

fn errors(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
    diags
        .iter()
        .filter(|d| matches!(d.kind, DiagnosticKind::Error))
        .collect()
}

fn has_error_containing(diags: &[Diagnostic], needle: &str) -> bool {
    errors(diags).iter().any(|d| d.message.contains(needle))
}

// ── Green path ────────────────────────────────────────────────────────────────

#[test]
fn canonical_mastermind_bridge_is_clean() {
    let source = "\
event PrMerged
  repo: Text
  pr_number: Number
agent mastermind
  webhook pr_merged
    mode: wake
    emit: PrMerged
  on PrMerged
    say \"cross-project PR merged\"
";
    let diags = check(source);
    assert!(
        errors(&diags).is_empty(),
        "expected no errors, got: {:#?}",
        diags
    );
}

#[test]
fn spawn_mode_without_handler_is_clean() {
    // `mode: spawn` creates a fresh instance, so no existing handler is required.
    let source = "\
event Poke
  tag: Text
agent a
  webhook kicker
    mode: spawn
    emit: Poke
  on start
    say \"ok\"
";
    let diags = check(source);
    assert!(errors(&diags).is_empty(), "got: {:#?}", diags);
}

// ── Missing mode / emit ────────────────────────────────────────────────────────

#[test]
fn missing_mode_is_flagged() {
    let source = "\
event E
  k: Text
agent a
  webhook t
    emit: E
  on E
    say \"hi\"
";
    let diags = check(source);
    assert!(
        has_error_containing(&diags, "missing a `mode:` clause"),
        "expected missing-mode error, got: {:#?}",
        diags
    );
}

#[test]
fn missing_emit_is_flagged() {
    let source = "\
event E
  k: Text
agent a
  webhook t
    mode: wake
  on E
    say \"hi\"
";
    let diags = check(source);
    assert!(
        has_error_containing(&diags, "missing an `emit:` clause"),
        "expected missing-emit error, got: {:#?}",
        diags
    );
}

// ── Unknown event ──────────────────────────────────────────────────────────────

#[test]
fn emit_of_unknown_event_is_flagged() {
    let source = "\
agent a
  webhook t
    mode: wake
    emit: NotDeclared
  on start
    say \"ok\"
";
    let diags = check(source);
    assert!(
        has_error_containing(&diags, "unknown event"),
        "expected unknown-event error, got: {:#?}",
        diags
    );
}

// ── mode: wake needs a handler ────────────────────────────────────────────────

#[test]
fn wake_without_handler_is_flagged() {
    let source = "\
event E
  k: Text
agent a
  webhook t
    mode: wake
    emit: E
  on start
    say \"ok\"
";
    let diags = check(source);
    assert!(
        has_error_containing(&diags, "has no `on E` handler"),
        "expected wake-without-handler error, got: {:#?}",
        diags
    );
}

// ── Duplicates ────────────────────────────────────────────────────────────────

#[test]
fn duplicate_trigger_name_is_flagged() {
    let source = "\
event E
  k: Text
agent a
  webhook t
    mode: wake
    emit: E
  webhook t
    mode: wake
    emit: E
  on E
    say \"hi\"
";
    let diags = check(source);
    assert!(
        has_error_containing(&diags, "duplicate webhook block"),
        "expected duplicate-trigger error, got: {:#?}",
        diags
    );
}

#[test]
fn duplicate_option_is_flagged() {
    let source = "\
event E
  k: Text
agent a
  webhook t
    mode: wake
    mode: spawn
    emit: E
  on E
    say \"hi\"
";
    let diags = check(source);
    assert!(
        has_error_containing(&diags, "duplicate `mode` option"),
        "expected duplicate-option error, got: {:#?}",
        diags
    );
}
