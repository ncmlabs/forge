// FORGE uncertain checker — Principle I (Honesty) enforcement
// Detects oracle results (reason/classify/search) used without
// confidence dispatch (when/match). See issue #26.

use std::collections::HashSet;

use crate::ast::{Expr, Program, Spanned, Stmt, TaskBody, TopLevel};
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
            TopLevel::FnMain(d) => {
                check_stmts(&d.body, file, &mut diagnostics);
            }
            // Pure functions can't use oracle ops (caught by pure_checker).
            // Other declarations have no statement bodies to check.
            _ => {}
        }
    }

    diagnostics
}

// ── Taint tracking ──────────────────────────────────────────────

fn check_stmts(stmts: &[Spanned<Stmt>], file: &str, diagnostics: &mut Vec<Diagnostic>) {
    let mut tainted: HashSet<String> = HashSet::new();
    // #484: names last bound from a `command`/`exec` result. Their
    // `.success` / `.exit_code` field is a deterministic verdict, so branching
    // on it satisfies the confidence gate too (see the IfElse arm).
    let mut command_bound: HashSet<String> = HashSet::new();

    for stmt in stmts {
        check_stmt(stmt, &mut tainted, &mut command_bound, file, diagnostics);
    }
}

fn check_stmt(
    stmt: &Spanned<Stmt>,
    tainted: &mut HashSet<String>,
    command_bound: &mut HashSet<String>,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match &stmt.node {
        Stmt::Bind(name, expr) => {
            if expr_is_oracle(expr) {
                tainted.insert(name.node.clone());
                // Only a foreground `command` record exposes `.success` /
                // `.exit_code`, so only it can be dispatched by reading them.
                if super::command_gate_checker::is_command_record(expr) {
                    command_bound.insert(name.node.clone());
                } else {
                    command_bound.remove(&name.node);
                }
            } else {
                // Reassignment to non-oracle clears taint
                tainted.remove(&name.node);
                command_bound.remove(&name.node);
            }
        }
        Stmt::Give(expr, _metas) => {
            // Inline oracle in give: always an error
            if expr_is_oracle(expr) {
                emit_inline_oracle_error(expr, file, diagnostics);
            } else if let Some(name) = expr_tainted_name(expr, tainted) {
                emit_unhandled_error(&name, expr, file, diagnostics);
            }
        }
        Stmt::Say(_) | Stmt::ExprStmt(_) => {
            // say/expr of tainted value is fine — it's give that
            // promotes uncertain<T> to T.
        }
        Stmt::When(when) => {
            // When dispatches on a confidence predicate — clear taint
            // for the subject variable.
            for clause in &when.clauses {
                let subject = &clause.node.predicate.node.subject.node;
                tainted.remove(subject);
                command_bound.remove(subject);
            }
            // Recurse into when/else bodies (they inherit the cleared taint)
            for clause in &when.clauses {
                check_stmt(&clause.node.body, tainted, command_bound, file, diagnostics);
            }
            if let Some(else_clause) = &when.else_body {
                check_stmt(
                    &else_clause.node.body,
                    tainted,
                    command_bound,
                    file,
                    diagnostics,
                );
            }
        }
        Stmt::Match(m) => {
            // Match on a tainted subject dispatches the uncertainty
            if let Expr::Ident(name) = &m.subject.node {
                tainted.remove(name);
                command_bound.remove(name);
            }
            for arm in &m.arms {
                check_stmt(&arm.node.body, tainted, command_bound, file, diagnostics);
            }
        }
        Stmt::IfElse(ie) => {
            // #484: `if x.success` / `if x.exit_code != 0` on a command result
            // is the deterministic verdict, so it also dispatches the
            // confidence uncertainty — no extra `when x.sure` gate is needed.
            let mut reads = HashSet::new();
            super::command_gate_checker::collect_gate_reads(
                &ie.condition,
                command_bound,
                &mut reads,
            );
            for (cond, _) in &ie.else_ifs {
                super::command_gate_checker::collect_gate_reads(cond, command_bound, &mut reads);
            }
            for name in reads {
                tainted.remove(&name);
            }
            for s in &ie.then_body {
                check_stmt(s, tainted, command_bound, file, diagnostics);
            }
            for (_cond, body) in &ie.else_ifs {
                for s in body {
                    check_stmt(s, tainted, command_bound, file, diagnostics);
                }
            }
            if let Some(body) = &ie.else_body {
                for s in body {
                    check_stmt(s, tainted, command_bound, file, diagnostics);
                }
            }
        }
        Stmt::For(f) => {
            for s in &f.body {
                check_stmt(s, tainted, command_bound, file, diagnostics);
            }
        }
        // Other statements don't interact with taint tracking
        _ => {}
    }
}

// ── Oracle detection ────────────────────────────────────────────

/// Returns true if the expression is directly an oracle call
/// (reason, classify, search).
fn expr_is_oracle(expr: &Spanned<Expr>) -> bool {
    matches!(
        &expr.node,
        Expr::Reason(_)
            | Expr::Classify(_)
            | Expr::Search(_)
            | Expr::Recall(_)
            | Expr::Exec(_)
            | Expr::Command(_)
            | Expr::Session(_)
            | Expr::CommandMethod(_, _)
            | Expr::SessionMethod(_, _)
    )
}

/// If the expression references a tainted variable (directly or via
/// field access), return that variable's name.
fn expr_tainted_name(expr: &Spanned<Expr>, tainted: &HashSet<String>) -> Option<String> {
    match &expr.node {
        Expr::Ident(name) => {
            if tainted.contains(name) {
                Some(name.clone())
            } else {
                None
            }
        }
        Expr::FieldAccess(inner, _) | Expr::GlobAccess(inner) => expr_tainted_name(inner, tainted),
        _ => None,
    }
}

// ── Diagnostics ─────────────────────────────────────────────────

fn emit_unhandled_error(
    name: &str,
    expr: &Spanned<Expr>,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    diagnostics.push(
        Diagnostic::error(
            "E020",
            file,
            format!(
                "unhandled uncertain: `{}` may be uncertain and must be dispatched with when/match",
                name
            ),
            expr.span.start..expr.span.end,
            "this value came from an oracle call",
        )
        .with_help("use `when result.sure -> ...` or `match result` to handle uncertainty first"),
    );
}

fn emit_inline_oracle_error(expr: &Spanned<Expr>, file: &str, diagnostics: &mut Vec<Diagnostic>) {
    diagnostics.push(
        Diagnostic::error(
            "E021",
            file,
            "unhandled uncertain: oracle result given without confidence dispatch".to_string(),
            expr.span.start..expr.span.end,
            "this oracle call returns uncertain<T> which cannot be given directly",
        )
        .with_help("bind the result to a variable, then use `when` or `match` before giving"),
    );
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn uncertain_errors(src: &str) -> Vec<String> {
        let program = parse(src).expect("parse failed");
        check(&program, "test.forge")
            .into_iter()
            .filter(|d| d.code == "E020")
            .map(|d| d.message)
            .collect()
    }

    #[test]
    fn exit_status_branch_dispatches_command_uncertainty() {
        let msgs = uncertain_errors(
            r#"
task t
  gives Text
  do
    result = command ["cargo", "test"] timeout 10m
    if result.success
      give result.stdout
    else
      give result.stderr
"#,
        );
        assert!(msgs.is_empty(), "unexpected E020: {msgs:?}");
    }

    #[test]
    fn exec_exit_status_branch_is_not_a_gate() {
        // `exec` returns Text: `result.exit_code` cannot dispatch it (#507).
        // The E150 twin of this case lives in `command_gate_checker`.
        let msgs = uncertain_errors(
            r#"
task t
  gives Text
  do
    result = exec "cargo test"
    if result.exit_code != 0
      give "failed"
    else
      give result
"#,
        );
        assert_eq!(msgs.len(), 1, "exec has no exit_code field: {msgs:?}");
    }

    #[test]
    fn when_dispatch_clears_exec_uncertainty() {
        let msgs = uncertain_errors(
            r#"
task t
  gives Text
  do
    result = exec "cargo test"
    when result.sure -> give result
    else -> give "FAILED"
"#,
        );
        assert!(msgs.is_empty(), "unexpected E020: {msgs:?}");
    }

    #[test]
    fn oracle_result_still_needs_its_own_gate() {
        let msgs = uncertain_errors(
            r#"
task t
  gives Text
  do
    x = command "cargo test"
    if x.success
      v = reason "explain"
      give v
"#,
        );
        assert_eq!(msgs.len(), 1, "reason still needs when/match: {msgs:?}");
    }
}
