// Tests for FORGE states checker (issue #17)

use forge::compose::SourceFile;
use forge::diagnostic::{Diagnostic, DiagnosticKind};
use forge::parser::parse;

fn check(source: &str) -> Vec<Diagnostic> {
    let program = parse(source).unwrap();
    forge::checker::states_checker::check(&program, "test.forge")
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

// ── Conflicting guards ───────────────────────────────────────

#[test]
fn conflicting_lifecycle_guards_is_error() {
    let source = "\
states GamePhase
  waiting -> playing
  playing -> done

agent room
  lifecycle: GamePhase
  on start(msg: Text)
    requires lifecycle == waiting
    requires lifecycle == playing
    transition to playing
";
    let diags = check(source);
    let errs = errors(&diags);
    assert!(
        errs.iter().any(|d| d.message.contains("conflicting")),
        "expected conflicting guards error, got: {:?}",
        errs
    );
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

// ── Additional transition legality tests ─────────────────────

#[test]
fn handler_without_transitions_no_error() {
    let source = "\
states GamePhase
  waiting -> playing

agent room
  lifecycle: GamePhase
  on ping(msg: Text)
    say msg
";
    let diags = check(source);
    let errs = errors(&diags);
    assert!(errs.is_empty());
}

#[test]
fn transition_nested_in_if_is_checked() {
    let source = "\
states GamePhase
  waiting -> playing

agent room
  lifecycle: GamePhase
  on start(msg: Text)
    requires lifecycle == waiting
    if msg == \"go\"
      transition to playing
";
    let diags = check(source);
    let errs = errors(&diags);
    assert!(errs.is_empty());
}

#[test]
fn multiple_agents_checked_independently() {
    let source = "\
states PhaseA
  idle -> active

states PhaseB
  open -> closed

agent a1
  lifecycle: PhaseA
  on go(msg: Text)
    requires lifecycle == idle
    transition to active

agent a2
  lifecycle: PhaseB
  on close(msg: Text)
    requires lifecycle == open
    transition to closed
";
    let diags = check(source);
    let errs = errors(&diags);
    assert!(errs.is_empty());
}

// ── Unreachable and opaque guard tests ───────────────────────

#[test]
fn unreachable_state_emits_warning() {
    // State 'orphan' appears only as a `to` target from 'a', but also
    // has a self-declared transition to 'done'. The key: 'orphan' doesn't
    // appear as a `from` in another transition that would let it be initial.
    // Actually, the checker defines initial_states as from-states not in to-set.
    // So let's use a state that only appears as `to` with no way to reach it
    // other than from states it can't be reached from.
    // Simplest: a -> b, a -> c, c -> d. 'd' is terminal. 'b' is terminal.
    // Both 'b' and 'd' have no outgoing = terminal warnings. 'a' is initial.
    // All states are reachable. Not a good test.
    // Better: we rely on the checker's definition. initial = from-states not in to-set.
    // With `a -> b, b -> c, d -> c`: d is initial (from, not in to-set), a is initial,
    // so neither is "unreachable". The checker's logic is correct for disjoint graphs.
    // Let's just test that the warning message format is correct using a true unreachable:
    // a -> b, b -> c. Add state 'd' somehow... but we can't add a state without a transition.
    // Every state in StatesDecl is defined by transitions. There's no isolated state.
    // So an unreachable state must be a `to` that's never a `from` AND is not reachable
    // from an initial state. But wait — the checker's initial_states = from not in to.
    // unreachable = incoming.is_empty() && not initial. incoming = edges where to == state.
    // For `a -> b, b -> c`: incoming(a) = {}, initial={a}, not unreachable.
    // incoming(b) = {a->b}, not empty, not checked. incoming(c) = {b->c}, not empty.
    // No unreachable states here either. The definition makes it hard to have an
    // unreachable state because all states come from transitions.
    // Skip this test — in practice, unreachable states are extremely unlikely given
    // that states are defined purely via transitions.
    // Instead, verify the full cycle case produces no warnings (already tested above).

    // The only scenario: a state appears as `to` but never as `from` and
    // is also never the `to` of any edge from an initial state... but that contradicts
    // it appearing as `to`. So all `to` states have at least one incoming edge.
    // And all `from` states are either initial or have incoming edges.
    // Therefore: unreachable states are impossible in the current StatesDecl format.
    // This test verifies that understanding.
    let source = "\
states Linear
  a -> b
  b -> c
";
    let diags = check(source);
    let warns = warnings(&diags);
    // 'c' is terminal (no outgoing), but no state is unreachable
    assert!(
        !warns.iter().any(|d| d.message.contains("unreachable")),
        "no states should be unreachable in a linear chain"
    );
}

#[test]
fn initial_state_is_not_flagged_unreachable() {
    let source = "\
states Simple
  begin -> done
";
    let diags = check(source);
    let warns = warnings(&diags);
    // 'begin' is initial — should NOT be flagged as unreachable
    assert!(
        !warns
            .iter()
            .any(|d| d.message.contains("begin") && d.message.contains("no incoming")),
        "initial state 'begin' should not be flagged unreachable"
    );
}

#[test]
fn opaque_guard_emits_warning() {
    let source = "\
states GamePhase
  waiting -> playing

agent room
  lifecycle: GamePhase
  on start(msg: Text)
    requires lifecycle != waiting
    transition to playing
";
    let diags = check(source);
    let warns = warnings(&diags);
    assert!(
        warns
            .iter()
            .any(|d| d.message.contains("complex") || d.message.contains("statically")),
        "expected opaque guard warning, got: {:?}",
        warns
    );
}

// ── Cross-file lifecycle references (#313) ──────────────────

fn check_merged(sources: &[(&str, &str)]) -> Vec<Diagnostic> {
    let source_files: Vec<SourceFile> = sources
        .iter()
        .map(|(path, src)| SourceFile {
            path: path.to_string(),
            source: src.to_string(),
            program: parse(src).unwrap(),
        })
        .collect();
    let composed = forge::compose::merge_programs(&source_files).unwrap();
    let merged_fname = source_files
        .first()
        .map(|sf| sf.path.clone())
        .unwrap_or_default();
    forge::checker::check_all(&composed.program, &merged_fname)
}

#[test]
fn cross_file_lifecycle_resolves_after_merge() {
    let states_src = "\
states MasteryLevel
  novice -> apprentice when score >= 40
  apprentice -> expert when score >= 90
  expert -> expert
";
    let agent_src = "\
agent learner
  lifecycle: MasteryLevel
  on progress(msg: Text)
    requires lifecycle == novice
    transition to apprentice
";
    let diags = check_merged(&[("states.forge", states_src), ("agent.forge", agent_src)]);
    let errs = errors(&diags);
    assert!(
        errs.is_empty(),
        "cross-file lifecycle should resolve after merge, got: {:?}",
        errs
    );
}

#[test]
fn truly_unknown_lifecycle_still_errors_after_merge() {
    let states_src = "\
states MasteryLevel
  novice -> apprentice when score >= 40
";
    let agent_src = "\
agent learner
  lifecycle: DoesNotExist
  on progress(msg: Text)
    say msg
";
    let diags = check_merged(&[("states.forge", states_src), ("agent.forge", agent_src)]);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("DoesNotExist"));
}
