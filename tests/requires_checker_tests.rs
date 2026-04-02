// Tests for FORGE requires checker (issue #18)

use forge::diagnostic::{Diagnostic, DiagnosticKind};
use forge::parser::parse;

fn check(source: &str) -> Vec<Diagnostic> {
    let program = parse(source).unwrap();
    forge::checker::requires_checker::check(&program, "test.forge")
}

fn warnings(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
    diags
        .iter()
        .filter(|d| matches!(d.kind, DiagnosticKind::Warning))
        .collect()
}

// ── Task call detection ──────────────────────────────────────

#[test]
fn task_in_requires_is_warning() {
    let source = "\
task is_valid
  needs board: Text, cell: Number
  gives Bool
  do
    give reason \"Is cell {cell} valid on {board}?\"

agent room
  on move(player: Text, cell: Number)
    requires is_valid(memory.board, cell)
    say \"ok\"
";
    let diags = check(source);
    let warns = warnings(&diags);
    assert_eq!(warns.len(), 1);
    assert!(warns[0].message.contains("is_valid"));
    assert!(warns[0].message.contains("stochastic"));
}

#[test]
fn pure_in_requires_is_clean() {
    let source = "\
pure valid_move
  needs board: Text, cell: Number
  gives Bool
  do
    give cell > 0

agent room
  on move(player: Text, cell: Number)
    requires valid_move(board, cell)
    say \"ok\"
";
    let diags = check(source);
    assert!(
        diags.is_empty(),
        "pure calls should not produce warnings: {:?}",
        diags
    );
}

#[test]
fn no_requires_is_clean() {
    let source = "\
agent room
  on ping(msg: Text)
    say msg
";
    let diags = check(source);
    assert!(diags.is_empty());
}

#[test]
fn lifecycle_guard_no_false_positive() {
    let source = "\
states GamePhase
  waiting -> playing

agent room
  lifecycle: GamePhase
  on start(msg: Text)
    requires lifecycle == waiting
    say msg
";
    let diags = check(source);
    assert!(
        diags.is_empty(),
        "lifecycle guards should not trigger requires_checker: {:?}",
        diags
    );
}

// ── Multiple requires ────────────────────────────────────────

#[test]
fn multiple_requires_warns_each_task() {
    let source = "\
task check_turn
  needs player: Text
  gives Bool
  do
    give reason \"Is it {player}'s turn?\"

task check_cell
  needs cell: Number
  gives Bool
  do
    give reason \"Is cell {cell} open?\"

agent room
  on move(player: Text, cell: Number)
    requires check_turn(player)
    requires check_cell(cell)
    say \"ok\"
";
    let diags = check(source);
    let warns = warnings(&diags);
    assert_eq!(warns.len(), 2);
    assert!(warns[0].message.contains("check_turn"));
    assert!(warns[1].message.contains("check_cell"));
}

// ── Nested task call in expression ───────────────────────────

#[test]
fn nested_task_call_in_requires() {
    let source = "\
task check_move
  needs cell: Number
  gives Bool
  do
    give reason \"Valid?\"

agent room
  on move(player: Text, cell: Number)
    requires cell > 0 and check_move(cell)
    say \"ok\"
";
    let diags = check(source);
    let warns = warnings(&diags);
    assert_eq!(warns.len(), 1);
    assert!(warns[0].message.contains("check_move"));
}

// ── on fail: give with task call ─────────────────────────────

#[test]
fn task_in_on_fail_give_is_warning() {
    let source = "\
task make_error
  needs msg: Text
  gives Text
  do
    give reason \"Format error: {msg}\"

agent room
  on move(player: Text, cell: Number)
    requires cell > 0  on fail: give make_error(\"bad cell\")
    say \"ok\"
";
    let diags = check(source);
    let warns = warnings(&diags);
    assert_eq!(warns.len(), 1);
    assert!(warns[0].message.contains("make_error"));
}

// ── LLM operations in requires ───────────────────────────────

#[test]
fn reason_in_requires_is_warning() {
    let source = "\
use
  llm.reason

agent room
  on move(player: Text, cell: Number)
    requires reason \"Is this valid?\"
    say \"ok\"
";
    let diags = check(source);
    let warns = warnings(&diags);
    assert_eq!(warns.len(), 1);
    assert!(warns[0].message.contains("reason"));
    assert!(warns[0].message.contains("LLM"));
}
