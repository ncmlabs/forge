// Tests for FORGE states checker (issue #17)

use forge::diagnostic::{Diagnostic, DiagnosticKind};
use forge::parser::parse;

fn check(source: &str) -> Vec<Diagnostic> {
    let program = parse(source).unwrap();
    forge::checker::states_checker::check(&program, "test.forge")
}

fn errors(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
    diags.iter().filter(|d| matches!(d.kind, DiagnosticKind::Error)).collect()
}

fn warnings(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
    diags.iter().filter(|d| matches!(d.kind, DiagnosticKind::Warning)).collect()
}

// ── State existence checks ────────────────────────────────────

#[test]
fn unknown_lifecycle_is_error() {
    let source = "\
agent broken
  lifecycle: NonExistent
  on ping(msg: Text)
    say msg
";
    let diags = check(source);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("NonExistent"));
}

#[test]
fn unknown_state_in_transition_is_error() {
    let source = "\
states GamePhase
  waiting -> playing

agent room
  lifecycle: GamePhase
  on start(msg: Text)
    requires lifecycle == waiting
    transition to nonexistent
";
    let diags = check(source);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("nonexistent"));
}

#[test]
fn unknown_state_in_guard_is_error() {
    let source = "\
states GamePhase
  waiting -> playing

agent room
  lifecycle: GamePhase
  on start(msg: Text)
    requires lifecycle == bogus
    say msg
";
    let diags = check(source);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("bogus"));
}

#[test]
fn valid_lifecycle_no_errors() {
    let source = "\
states GamePhase
  waiting -> playing

agent room
  lifecycle: GamePhase
  on start(msg: Text)
    requires lifecycle == waiting
    transition to playing
";
    let diags = check(source);
    let errs = errors(&diags);
    assert!(errs.is_empty());
}

#[test]
fn agent_without_lifecycle_is_skipped() {
    let source = "\
agent simple
  on ping(msg: Text)
    say msg
";
    let diags = check(source);
    assert!(diags.is_empty());
}

// ── Transition legality ───────────────────────────────────────

#[test]
fn illegal_transition_is_error() {
    let source = "\
states GamePhase
  waiting -> playing
  playing -> done

agent room
  lifecycle: GamePhase
  on cheat(msg: Text)
    requires lifecycle == waiting
    transition to done
";
    let diags = check(source);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("waiting"));
    assert!(errs[0].message.contains("done"));
}

#[test]
fn legal_transition_passes() {
    let source = "\
states GamePhase
  waiting -> playing
  playing -> done

agent room
  lifecycle: GamePhase
  on start(msg: Text)
    requires lifecycle == waiting
    transition to playing
  on finish(msg: Text)
    requires lifecycle == playing
    transition to done
";
    let diags = check(source);
    let errs = errors(&diags);
    assert!(errs.is_empty());
}

// ── Unguarded transition ──────────────────────────────────────

#[test]
fn unguarded_transition_is_error() {
    let source = "\
states GamePhase
  waiting -> playing

agent room
  lifecycle: GamePhase
  on start(msg: Text)
    transition to playing
";
    let diags = check(source);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("unguarded"));
}

#[test]
fn non_lifecycle_requires_does_not_count_as_guard() {
    let source = "\
states GamePhase
  waiting -> playing

agent room
  lifecycle: GamePhase
  on start(msg: Text)
    requires player_count < 2
    transition to playing
";
    let diags = check(source);
    let errs = errors(&diags);
    // Should be an unguarded transition error since requires is not a lifecycle guard
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("unguarded"));
}

// ── Structural warnings ───────────────────────────────────────

#[test]
fn terminal_state_is_warning() {
    let source = "\
states GamePhase
  waiting -> playing

agent room
  lifecycle: GamePhase
  on start(msg: Text)
    requires lifecycle == waiting
    transition to playing
";
    let diags = check(source);
    let warns = warnings(&diags);
    // `playing` has no outgoing edges — terminal warning
    assert!(!warns.is_empty());
    assert!(warns.iter().any(|w| w.message.contains("playing")));
}

#[test]
fn states_with_full_cycle_have_no_structural_warnings() {
    let source = "\
states TrafficLight
  red -> green
  green -> yellow
  yellow -> red

agent light
  lifecycle: TrafficLight
  on tick(msg: Text)
    requires lifecycle == red
    transition to green
";
    let diags = check(source);
    let warns = warnings(&diags);
    // All states have both incoming and outgoing edges — no structural warnings
    assert!(warns.is_empty(), "unexpected warnings: {:?}", warns);
}
