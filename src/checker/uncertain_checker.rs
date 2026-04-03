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

    for stmt in stmts {
        check_stmt(stmt, &mut tainted, file, diagnostics);
    }
}

fn check_stmt(
    stmt: &Spanned<Stmt>,
    tainted: &mut HashSet<String>,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match &stmt.node {
        Stmt::Bind(name, expr) => {
            if expr_is_oracle(expr) {
                tainted.insert(name.node.clone());
            } else {
                // Reassignment to non-oracle clears taint
                tainted.remove(&name.node);
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
            }
            // Recurse into when/else bodies (they inherit the cleared taint)
            for clause in &when.clauses {
                check_stmt(&clause.node.body, tainted, file, diagnostics);
            }
            if let Some(else_clause) = &when.else_body {
                check_stmt(&else_clause.node.body, tainted, file, diagnostics);
            }
        }
        Stmt::Match(m) => {
            // Match on a tainted subject dispatches the uncertainty
            if let Expr::Ident(name) = &m.subject.node {
                tainted.remove(name);
            }
            for arm in &m.arms {
                check_stmt(&arm.node.body, tainted, file, diagnostics);
            }
        }
        Stmt::IfElse(ie) => {
            for s in &ie.then_body {
                check_stmt(s, tainted, file, diagnostics);
            }
            for (_cond, body) in &ie.else_ifs {
                for s in body {
                    check_stmt(s, tainted, file, diagnostics);
                }
            }
            if let Some(body) = &ie.else_body {
                for s in body {
                    check_stmt(s, tainted, file, diagnostics);
                }
            }
        }
        Stmt::For(f) => {
            for s in &f.body {
                check_stmt(s, tainted, file, diagnostics);
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
        Expr::Reason(_) | Expr::Classify(_) | Expr::Search(_)
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
            file,
            "unhandled uncertain: oracle result given without confidence dispatch".to_string(),
            expr.span.start..expr.span.end,
            "this oracle call returns uncertain<T> which cannot be given directly",
        )
        .with_help("bind the result to a variable, then use `when` or `match` before giving"),
    );
}
