// FORGE correlate checker tests — issue #334
// Covers each CheckError variant plus the canonical green path.

use forge::checker::correlate_checker;
use forge::diagnostic::{Diagnostic, DiagnosticKind};
use forge::parser::parse;

fn check(source: &str) -> Vec<Diagnostic> {
    let program = parse(source).expect("parse must succeed");
    correlate_checker::check(&program, "test.forge")
}

fn errors(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
    diags
        .iter()
        .filter(|d| matches!(d.kind, DiagnosticKind::Error))
        .collect()
}

// ── Green path ────────────────────────────────────────────────────────────────

#[test]
fn canonical_slack_specialist_is_clean() {
    let source = "\
event SlackMention
  thread_ts: Text
  message: Text
event SlackReply
  thread_ts: Text
agent slack_specialist
  memory persistent
    thread_ts: Text
    task_id: Text
  correlate on SlackMention.thread_ts
    mode: wake
    emit: SlackReply
  on SlackReply
    say \"handled\"
";
    let diags = check(source);
    assert!(
        errors(&diags).is_empty(),
        "expected no errors, got: {:#?}",
        diags
    );
}

#[test]
fn canonical_slack_specialist_with_inline_handler_is_clean() {
    // `mode: wake` can drop `emit:` when the agent already handles the event directly.
    let source = "\
event SlackMention
  thread_ts: Text
agent slack_specialist
  memory persistent
    thread_ts: Text
  correlate on SlackMention.thread_ts
    mode: wake
  on SlackMention
    say \"handled\"
";
    let diags = check(source);
    assert!(errors(&diags).is_empty(), "got: {:#?}", diags);
}

// ── Unknown event / field ──────────────────────────────────────────────────────

#[test]
fn correlate_on_unknown_event_is_flagged() {
    let source = "\
agent a
  memory persistent
    thread_ts: Text
  correlate on NotDeclared.thread_ts
    mode: wake
    emit: Out
  on start
    say \"ok\"
";
    let diags = check(source);
    let errs = errors(&diags);
    assert!(
        errs.iter().any(|d| d.message.contains("unknown event")),
        "expected unknown-event error, got: {:#?}",
        diags
    );
}

#[test]
fn correlate_on_unknown_field_is_flagged() {
    let source = "\
event E
  other: Text
agent a
  memory persistent
    thread_ts: Text
  correlate on E.thread_ts
    mode: wake
    emit: Out
  on start
    say \"ok\"
";
    let diags = check(source);
    let errs = errors(&diags);
    assert!(
        errs.iter().any(|d| d.message.contains("no field")),
        "expected missing-field error, got: {:#?}",
        diags
    );
}

#[test]
fn correlate_on_non_text_field_is_flagged() {
    let source = "\
event E
  n: Number
agent a
  memory persistent
    n: Text
  correlate on E.n
    mode: wake
    emit: Out
  on start
    say \"ok\"
";
    let diags = check(source);
    let errs = errors(&diags);
    assert!(
        errs.iter().any(|d| d.message.contains("must be `Text`")),
        "expected field-not-Text error, got: {:#?}",
        diags
    );
}

// ── Memory-field requirement ───────────────────────────────────────────────────

#[test]
fn correlate_without_matching_memory_field_is_flagged() {
    let source = "\
event E
  k: Text
agent a
  memory persistent
    other: Text
  correlate on E.k
    mode: wake
    emit: Out
  on start
    say \"ok\"
";
    let diags = check(source);
    let errs = errors(&diags);
    assert!(
        errs.iter().any(|d| d.message.contains("memory persistent")),
        "expected missing-memory-field error, got: {:#?}",
        diags
    );
}

#[test]
fn correlate_without_memory_persistent_is_flagged() {
    let source = "\
event E
  k: Text
agent a
  memory
    k: Text
  correlate on E.k
    mode: wake
    emit: Out
  on start
    say \"ok\"
";
    let diags = check(source);
    let errs = errors(&diags);
    assert!(
        errs.iter().any(|d| d.message.contains("memory persistent")),
        "expected missing-memory-field error when memory is non-persistent, got: {:#?}",
        diags
    );
}

// ── Mode / emit coherence ──────────────────────────────────────────────────────

#[test]
fn correlate_missing_mode_is_flagged() {
    let source = "\
event E
  k: Text
agent a
  memory persistent
    k: Text
  correlate on E.k
    emit: Out
  on start
    say \"ok\"
";
    let diags = check(source);
    let errs = errors(&diags);
    assert!(
        errs.iter().any(|d| d.message.contains("missing a `mode:`")),
        "expected missing-mode error, got: {:#?}",
        diags
    );
}

#[test]
fn correlate_wake_without_emit_or_handler_is_flagged() {
    let source = "\
event E
  k: Text
agent a
  memory persistent
    k: Text
  correlate on E.k
    mode: wake
  on start
    say \"ok\"
";
    let diags = check(source);
    let errs = errors(&diags);
    assert!(
        errs.iter().any(|d| d.message.contains("no `emit:`")),
        "expected wake-missing-pair error, got: {:#?}",
        diags
    );
}

// ── Duplicates ────────────────────────────────────────────────────────────────

#[test]
fn duplicate_correlate_pair_is_flagged() {
    let source = "\
event E
  k: Text
agent a
  memory persistent
    k: Text
  correlate on E.k
    mode: wake
    emit: Out
  correlate on E.k
    mode: wake
    emit: Out
  on start
    say \"ok\"
";
    let diags = check(source);
    let errs = errors(&diags);
    assert!(
        errs.iter()
            .any(|d| d.message.contains("duplicate correlate")),
        "expected duplicate-correlate error, got: {:#?}",
        diags
    );
}

#[test]
fn duplicate_option_is_flagged() {
    let source = "\
event E
  k: Text
agent a
  memory persistent
    k: Text
  correlate on E.k
    mode: wake
    mode: spawn
    emit: Out
  on start
    say \"ok\"
";
    let diags = check(source);
    let errs = errors(&diags);
    assert!(
        errs.iter().any(|d| d.message.contains("duplicate `mode`")),
        "expected duplicate-option error, got: {:#?}",
        diags
    );
}
