// Tests for FORGE pure checker (issue #16)

use forge::checker::pure_checker::{check, CheckError};
use forge::parser::parse;

// ── Rejection tests ─────────────────────────────────────────

#[test]
fn pure_rejects_reason() {
    let source =
        "pure bad\n  needs x: Text\n  gives Text\n  do\n    result = reason x\n    give result\n";
    let program = parse(source).unwrap();
    let errors = check(&program);
    assert_eq!(errors.len(), 1);
    assert!(
        matches!(&errors[0], CheckError::PureUsesLlm { name, op, .. } if name == "bad" && *op == "reason")
    );
}

#[test]
fn pure_rejects_classify() {
    let source = "pure bad\n  needs msg: Text\n  gives Text\n  do\n    result = classify msg into [\"a\", \"b\"]\n    give result\n";
    let program = parse(source).unwrap();
    let errors = check(&program);
    assert_eq!(errors.len(), 1);
    assert!(
        matches!(&errors[0], CheckError::PureUsesLlm { name, op, .. } if name == "bad" && *op == "classify")
    );
}

#[test]
fn pure_rejects_search() {
    let source =
        "pure bad\n  needs q: Text\n  gives Text\n  do\n    result = search q\n    give result\n";
    let program = parse(source).unwrap();
    let errors = check(&program);
    assert_eq!(errors.len(), 1);
    assert!(
        matches!(&errors[0], CheckError::PureUsesLlm { name, op, .. } if name == "bad" && *op == "search")
    );
}

#[test]
fn pure_rejects_try_or() {
    let source = "pure bad\n  needs x: Text\n  gives Text\n  do\n    result = try x or \"fallback\"\n    give result\n";
    let program = parse(source).unwrap();
    let errors = check(&program);
    assert!(errors
        .iter()
        .any(|e| matches!(e, CheckError::PureUsesTryOr { name, .. } if name == "bad")));
}

#[test]
fn pure_rejects_escalate() {
    let source =
        "pure bad\n  needs x: Text\n  gives Text\n  do\n    escalate to human\n    give x\n";
    let program = parse(source).unwrap();
    let errors = check(&program);
    assert_eq!(errors.len(), 1);
    assert!(matches!(&errors[0], CheckError::PureEscalates { name, .. } if name == "bad"));
}

#[test]
fn pure_rejects_call_to_task() {
    let source = "\
task stochastic_thing\n  needs x: Text\n  gives Text\n  do\n    give reason x\n\
\npure bad\n  needs x: Text\n  gives Text\n  do\n    result = stochastic_thing(x)\n    give result\n";
    let program = parse(source).unwrap();
    let errors = check(&program);
    assert_eq!(errors.len(), 1);
    assert!(
        matches!(&errors[0], CheckError::PureCallsTask { name, callee, .. } if name == "bad" && callee == "stochastic_thing")
    );
}

// ── Acceptance tests ────────────────────────────────────────

#[test]
fn pure_allows_arithmetic() {
    let source = "pure add\n  needs a: Number, b: Number\n  gives Number\n  do\n    give a + b\n";
    let program = parse(source).unwrap();
    let errors = check(&program);
    assert!(errors.is_empty());
}

#[test]
fn pure_allows_call_to_pure() {
    let source = "\
pure helper\n  needs x: Number\n  gives Number\n  do\n    give x + 1\n\
\npure caller\n  needs x: Number\n  gives Number\n  do\n    result = helper(x)\n    give result\n";
    let program = parse(source).unwrap();
    let errors = check(&program);
    assert!(errors.is_empty());
}

// ── Nesting and multiple-error tests ────────────────────────

#[test]
fn pure_rejects_reason_nested_in_if() {
    let source = "pure bad\n  needs x: Text\n  gives Text\n  do\n    if x == \"go\"\n      result = reason x\n      give result\n    else\n      give x\n";
    let program = parse(source).unwrap();
    let errors = check(&program);
    assert_eq!(errors.len(), 1);
    assert!(
        matches!(&errors[0], CheckError::PureUsesLlm { name, op, .. } if name == "bad" && *op == "reason")
    );
}

#[test]
fn pure_rejects_reason_nested_in_for() {
    let source = "pure bad\n  needs items: Text\n  gives Text\n  do\n    for item in items\n      say reason item\n    give items\n";
    let program = parse(source).unwrap();
    let errors = check(&program);
    assert_eq!(errors.len(), 1);
    assert!(
        matches!(&errors[0], CheckError::PureUsesLlm { name, op, .. } if name == "bad" && *op == "reason")
    );
}

#[test]
fn pure_reports_all_violations() {
    let source = "pure bad\n  needs x: Text\n  gives Text\n  do\n    a = reason x\n    b = search x\n    escalate to human\n    give a\n";
    let program = parse(source).unwrap();
    let errors = check(&program);
    assert_eq!(errors.len(), 3);
}

#[test]
fn multiple_pure_fns_all_checked() {
    let source = "\
pure bad1\n  needs x: Text\n  gives Text\n  do\n    give reason x\n\
\npure bad2\n  needs x: Text\n  gives Text\n  do\n    give search x\n";
    let program = parse(source).unwrap();
    let errors = check(&program);
    assert_eq!(errors.len(), 2);
    let names: Vec<&str> = errors
        .iter()
        .map(|e| match e {
            CheckError::PureUsesLlm { name, .. } => name.as_str(),
            _ => "",
        })
        .collect();
    assert!(names.contains(&"bad1"));
    assert!(names.contains(&"bad2"));
}
