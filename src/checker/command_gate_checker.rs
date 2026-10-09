// FORGE command gate checker — issue #484
// A `command`/`exec` result is an unchecked claim about the outside world:
// until the body dispatches it, no oracle may decide in its place (#431).
// Dataflow tracking mirrors `uncertain_checker`, with a different taint.

use std::collections::HashSet;

use crate::ast::{Expr, Program, Spanned, Stmt, TaskBody, TemplatePart, TopLevel};
use crate::diagnostic::Diagnostic;

// ── Public API ──────────────────────────────────────────────────

pub fn check(program: &Program, file: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for item in &program.items {
        match &item.node {
            TopLevel::Task(d) => {
                if let TaskBody::Do(stmts) = &d.body.node {
                    check_stmts(stmts, file, &mut diagnostics);
                }
            }
            TopLevel::Flow(d) => {
                for stage in &d.stages {
                    check_stmts(&stage.node.body, file, &mut diagnostics);
                }
            }
            TopLevel::Agent(d) => {
                for handler in &d.handlers {
                    check_stmts(&handler.node.body, file, &mut diagnostics);
                }
            }
            TopLevel::FnMain(d) => check_stmts(&d.body, file, &mut diagnostics),
            _ => {}
        }
    }

    diagnostics
}

// ── Unchecked tracking ──────────────────────────────────────────

fn check_stmts(stmts: &[Spanned<Stmt>], file: &str, diagnostics: &mut Vec<Diagnostic>) {
    let mut unchecked: HashSet<String> = HashSet::new();

    for stmt in stmts {
        check_stmt(stmt, &mut unchecked, file, diagnostics);
    }
}

fn check_stmt(
    stmt: &Spanned<Stmt>,
    unchecked: &mut HashSet<String>,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match &stmt.node {
        Stmt::Bind(name, expr) => {
            match command_binding(expr) {
                // Background handles are checked through `command.status`.
                Some(true) => {
                    unchecked.remove(&name.node);
                }
                Some(false) => {
                    unchecked.insert(name.node.clone());
                }
                None => {
                    // Reassignment to anything else clears the gate.
                    unchecked.remove(&name.node);
                    // An oracle verdict must not be built on a raw command
                    // result: that is the #431 shape.
                    if matches!(expr.node, Expr::Reason(_) | Expr::Classify(_)) {
                        gate_expr(expr, unchecked, file, diagnostics);
                    }
                }
            }
        }
        Stmt::Give(expr, _metas) => gate_expr(expr, unchecked, file, diagnostics),
        Stmt::Emit(_name, args) => {
            for arg in args {
                gate_expr(&arg.node.value, unchecked, file, diagnostics);
            }
        }
        Stmt::When(when) => {
            // Confidence dispatch on the command result itself: the command's
            // confidence is its exit status, so `.sure` is a success check.
            for clause in &when.clauses {
                unchecked.remove(&clause.node.predicate.node.subject.node);
            }
            for clause in &when.clauses {
                check_stmt(&clause.node.body, unchecked, file, diagnostics);
            }
            if let Some(else_clause) = &when.else_body {
                check_stmt(&else_clause.node.body, unchecked, file, diagnostics);
            }
        }
        Stmt::Match(m) => {
            // A branch on `.success` / `.exit_code` is the deterministic gate.
            let mut reads = HashSet::new();
            collect_gate_reads(&m.subject, unchecked, &mut reads);
            for name in reads {
                unchecked.remove(&name);
            }
            for arm in &m.arms {
                check_stmt(&arm.node.body, unchecked, file, diagnostics);
            }
        }
        Stmt::IfElse(ie) => {
            // The gate counts for both branches and for every statement
            // after the `if`, so clear before walking any body.
            let mut reads = HashSet::new();
            collect_gate_reads(&ie.condition, unchecked, &mut reads);
            for (cond, _) in &ie.else_ifs {
                collect_gate_reads(cond, unchecked, &mut reads);
            }
            for name in reads {
                unchecked.remove(&name);
            }
            for s in &ie.then_body {
                check_stmt(s, unchecked, file, diagnostics);
            }
            for (_cond, body) in &ie.else_ifs {
                for s in body {
                    check_stmt(s, unchecked, file, diagnostics);
                }
            }
            if let Some(body) = &ie.else_body {
                for s in body {
                    check_stmt(s, unchecked, file, diagnostics);
                }
            }
        }
        Stmt::For(f) => {
            for s in &f.body {
                check_stmt(s, unchecked, file, diagnostics);
            }
        }
        // `say` is not a decision, and `transition` takes no value.
        _ => {}
    }
}

/// `Some(true)` for a background command handle (exempt), `Some(false)` for a
/// foreground `command`/`exec` result.
fn command_binding(expr: &Spanned<Expr>) -> Option<bool> {
    match &expr.node {
        Expr::Command(c) => Some(c.background.as_ref().is_some_and(|b| b.node)),
        Expr::Exec(_) => Some(false),
        _ => None,
    }
}

// ── Expression walking ──────────────────────────────────────────

/// Base variable name of an access chain: `x.stdout` and `x` both give `x`.
fn base_name(expr: &Spanned<Expr>) -> Option<&str> {
    match &expr.node {
        Expr::Ident(n) => Some(n),
        Expr::FieldAccess(inner, _) | Expr::GlobAccess(inner) => base_name(inner),
        Expr::Paren(inner) => base_name(inner),
        _ => None,
    }
}

/// Report every unchecked command result referenced by `expr`.
fn gate_expr(
    expr: &Spanned<Expr>,
    unchecked: &HashSet<String>,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut names = Vec::new();
    collect_unchecked_refs(expr, unchecked, &mut names);
    for name in names {
        emit_gate_error(&name, expr, file, diagnostics);
    }
}

fn collect_unchecked_refs(
    expr: &Spanned<Expr>,
    unchecked: &HashSet<String>,
    out: &mut Vec<String>,
) {
    match base_name(expr) {
        Some(name) => {
            if unchecked.contains(name) && !out.iter().any(|n| n == name) {
                out.push(name.to_string());
            }
        }
        None => walk_children(expr, &mut |child| {
            collect_unchecked_refs(child, unchecked, out)
        }),
    }
}

/// Collect names of unchecked results whose `.success` / `.exit_code` is read.
fn collect_gate_reads(
    expr: &Spanned<Expr>,
    unchecked: &HashSet<String>,
    out: &mut HashSet<String>,
) {
    if let Expr::FieldAccess(inner, field) = &expr.node {
        if field.node == "success" || field.node == "exit_code" {
            if let Some(name) = base_name(inner) {
                if unchecked.contains(name) {
                    out.insert(name.to_string());
                }
            }
        }
    }
    walk_children(expr, &mut |child| collect_gate_reads(child, unchecked, out));
}

fn walk_children(expr: &Spanned<Expr>, f: &mut impl FnMut(&Spanned<Expr>)) {
    match &expr.node {
        Expr::NumberLit(_) | Expr::BoolLit(_) | Expr::Ident(_) | Expr::TypeAccess(_, _) => {}
        Expr::Template(parts) => {
            for p in parts {
                if let TemplatePart::Interp(e) | TemplatePart::RawInterp(e) = &p.node {
                    f(e);
                }
            }
        }
        Expr::Call(c) => {
            for a in &c.args {
                f(&a.node.value);
            }
        }
        Expr::Constructor(c) => {
            for a in &c.args {
                f(&a.node.value);
            }
        }
        Expr::Reason(r) => f(&r.prompt),
        Expr::Classify(c) => f(&c.input),
        Expr::Search(e)
        | Expr::Recall(e)
        | Expr::Exec(e)
        | Expr::Paren(e)
        | Expr::GlobAccess(e) => f(e),
        Expr::Command(c) => {
            f(&c.cmd);
            if let Some(wd) = &c.working_dir {
                f(wd);
            }
        }
        Expr::CommandMethod(_, args) | Expr::SessionMethod(_, args) => {
            for a in args {
                f(&a.node.value);
            }
        }
        Expr::Session(_) | Expr::Find(_) => {}
        Expr::TryOr(a, b) => {
            f(a);
            f(b);
        }
        Expr::Compose(items) | Expr::FanOut(items) | Expr::ArrayLit(items) => {
            for it in items {
                f(it);
            }
        }
        Expr::FieldAccess(inner, _) => f(inner),
        Expr::Index(a, b) => {
            f(a);
            f(b);
        }
        Expr::MethodCall(recv, _, args) => {
            f(recv);
            for a in args {
                f(&a.node.value);
            }
        }
        Expr::BinOp(a, _, b) => {
            f(a);
            f(b);
        }
        Expr::UnaryOp(_, e) => f(e),
    }
}

// ── Diagnostics ─────────────────────────────────────────────────

fn emit_gate_error(
    name: &str,
    expr: &Spanned<Expr>,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    diagnostics.push(
        Diagnostic::error(
            "E150",
            file,
            format!("command result `{name}` used before checking `{name}.success`"),
            expr.span.start..expr.span.end,
            "this decision may overrule a failed command",
        )
        .with_help(
            "branch on x.success (or x.exit_code) first; inside the failure branch an oracle may explain the failure, not decide it",
        ),
    );
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn gate_errors(src: &str) -> Vec<String> {
        let program = parse(src).expect("parse failed");
        check(&program, "test.forge")
            .into_iter()
            .filter(|d| d.code == "E150")
            .map(|d| d.message)
            .collect()
    }

    fn assert_flagged(src: &str) {
        let msgs = gate_errors(src);
        assert_eq!(msgs.len(), 1, "expected one E150, got: {msgs:?}");
        assert!(
            msgs[0].contains("used before checking"),
            "unexpected message: {}",
            msgs[0]
        );
    }

    fn assert_clean(src: &str) {
        let msgs = gate_errors(src);
        assert!(msgs.is_empty(), "unexpected E150: {msgs:?}");
    }

    #[test]
    fn give_of_unchecked_result_is_flagged() {
        assert_flagged(
            r#"
task t
  gives Text
  do
    x = command ["cargo", "test"]
    give x.stdout
"#,
        );
    }

    #[test]
    fn emit_of_unchecked_result_is_flagged() {
        assert_flagged(
            r#"
agent a
  on start
    x = command "cargo test"
    emit Done(out: x.stdout)
"#,
        );
    }

    #[test]
    fn oracle_prompt_of_unchecked_result_is_flagged() {
        assert_flagged(
            r#"
task t
  gives Text
  do
    x = command "cargo test"
    v = reason "judge {x.stdout}"
    when v.sure -> give v
    else -> give "unknown"
"#,
        );
    }

    #[test]
    fn exec_result_is_flagged_too() {
        assert_flagged(
            r#"
task t
  gives Text
  do
    x = exec "cargo test"
    give x
"#,
        );
    }

    #[test]
    fn success_branch_clears_the_gate() {
        assert_clean(
            r#"
task t
  gives Text
  do
    x = command "cargo test"
    if x.success
      give x.stdout
    else
      give "tests failed"
"#,
        );
    }

    #[test]
    fn exit_code_branch_clears_the_gate() {
        assert_clean(
            r#"
task t
  gives Text
  do
    x = command "cargo test"
    if x.exit_code != 0
      give "tests failed"
    else
      give x.stdout
"#,
        );
    }

    #[test]
    fn gate_counts_after_the_if() {
        assert_clean(
            r#"
task t
  gives Text
  do
    x = command "cargo test"
    if x.success
      say "green"
    give x.stdout
"#,
        );
    }

    #[test]
    fn oracle_in_failure_branch_is_allowed() {
        assert_clean(
            r#"
task t
  gives Text
  do
    x = command "cargo test"
    if x.success
      give x.stdout
    else
      v = reason "explain this failure: {x.stderr}"
      when v.sure -> give v
      else -> give "unknown"
"#,
        );
    }

    #[test]
    fn background_command_is_exempt() {
        assert_clean(
            r#"
task t
  gives Text
  do
    h = command "cargo watch" background true
    say h.status
    give "started"
"#,
        );
    }

    #[test]
    fn say_of_unchecked_result_is_allowed() {
        assert_clean(
            r#"
task t
  gives Text
  do
    x = command "cargo test"
    say x.stdout
    give "done"
"#,
        );
    }

    #[test]
    fn confidence_dispatch_clears_the_gate() {
        assert_clean(
            r#"
task t
  gives Text
  do
    x = command "cargo test"
    when x.sure -> give x.stdout
    else -> give "FAILED"
"#,
        );
    }

    #[test]
    fn match_on_exit_code_clears_the_gate() {
        assert_clean(
            r#"
task t
  gives Text
  do
    x = command "cargo test"
    match x.exit_code
      ok -> give x.stdout
      _ -> give "tests failed"
"#,
        );
    }
}
