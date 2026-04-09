// FORGE pure checker
// See issue #16 for full specification

use std::collections::HashSet;

use crate::ast::{Expr, Program, Spanned, Stmt, TemplatePart, TopLevel};
use crate::diagnostic::Diagnostic;

// ── Errors ───────────────────────────────────────────────────

#[derive(Debug)]
pub enum CheckError {
    /// reason/classify/search inside a pure function
    PureUsesLlm {
        name: String,
        op: &'static str,
        span_start: usize,
        span_end: usize,
    },
    /// try...or expression inside a pure function
    PureUsesTryOr {
        name: String,
        span_start: usize,
        span_end: usize,
    },
    /// escalate statement inside a pure function
    PureEscalates {
        name: String,
        span_start: usize,
        span_end: usize,
    },
    /// call to a task from a pure function
    PureCallsTask {
        name: String,
        callee: String,
        span_start: usize,
        span_end: usize,
    },
}

impl CheckError {
    pub fn to_diagnostic(&self, file: &str) -> Diagnostic {
        match self {
            CheckError::PureUsesLlm {
                name,
                op,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!("pure function `{}` cannot use `{}`", name, op),
                *span_start..*span_end,
                format!(
                    "`{}` is an LLM operation, not allowed in pure functions",
                    op
                ),
            )
            .with_help("move this operation to a `task` instead"),
            CheckError::PureUsesTryOr {
                name,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!("pure function `{}` cannot use `try...or`", name),
                *span_start..*span_end,
                "`try...or` wraps stochastic operations, not allowed in pure functions",
            )
            .with_help("pure functions are deterministic — remove the try/or fallback"),
            CheckError::PureEscalates {
                name,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!("pure function `{}` cannot use `escalate`", name),
                *span_start..*span_end,
                "`escalate` is a side effect, not allowed in pure functions",
            )
            .with_help("move escalation logic to a `task` instead"),
            CheckError::PureCallsTask {
                name,
                callee,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!("pure function `{}` cannot call task `{}`", name, callee),
                *span_start..*span_end,
                format!(
                    "`{}` is a task (possibly stochastic), not callable from pure",
                    callee
                ),
            )
            .with_help("pure functions can only call other pure functions"),
        }
    }
}

// ── Checker ──────────────────────────────────────────────────

pub fn check(program: &Program) -> Vec<CheckError> {
    // Build name registries from top-level declarations
    let mut task_names = HashSet::new();
    for item in &program.items {
        if let TopLevel::Task(task) = &item.node {
            task_names.insert(task.name.node.clone());
        }
    }

    let mut errors = Vec::new();

    for item in &program.items {
        if let TopLevel::Pure(pure) = &item.node {
            check_pure_body(&pure.name.node, &pure.body, &task_names, &mut errors);
        }
    }

    errors
}

fn check_pure_body(
    fn_name: &str,
    stmts: &[Spanned<Stmt>],
    task_names: &HashSet<String>,
    errors: &mut Vec<CheckError>,
) {
    for stmt in stmts {
        check_pure_stmt(fn_name, stmt, task_names, errors);
    }
}

fn check_pure_stmt(
    fn_name: &str,
    stmt: &Spanned<Stmt>,
    task_names: &HashSet<String>,
    errors: &mut Vec<CheckError>,
) {
    match &stmt.node {
        Stmt::Bind(_, expr) | Stmt::Say(expr) | Stmt::ExprStmt(expr) => {
            check_pure_expr(fn_name, expr, task_names, errors);
        }
        Stmt::Give(expr, metas) => {
            check_pure_expr(fn_name, expr, task_names, errors);
            for meta in metas {
                check_pure_expr(fn_name, &meta.node.value, task_names, errors);
            }
        }
        Stmt::Escalate(_) => {
            errors.push(CheckError::PureEscalates {
                name: fn_name.to_string(),
                span_start: stmt.span.start,
                span_end: stmt.span.end,
            });
        }
        Stmt::When(when) => {
            for clause in &when.clauses {
                check_pure_stmt(fn_name, &clause.node.body, task_names, errors);
            }
            if let Some(else_clause) = &when.else_body {
                check_pure_stmt(fn_name, &else_clause.node.body, task_names, errors);
            }
        }
        Stmt::Match(m) => {
            check_pure_expr(fn_name, &m.subject, task_names, errors);
            for arm in &m.arms {
                check_pure_stmt(fn_name, &arm.node.body, task_names, errors);
            }
        }
        Stmt::IfElse(ie) => {
            check_pure_expr(fn_name, &ie.condition, task_names, errors);
            for s in &ie.then_body {
                check_pure_stmt(fn_name, s, task_names, errors);
            }
            for (cond, body) in &ie.else_ifs {
                check_pure_expr(fn_name, cond, task_names, errors);
                for s in body {
                    check_pure_stmt(fn_name, s, task_names, errors);
                }
            }
            if let Some(body) = &ie.else_body {
                for s in body {
                    check_pure_stmt(fn_name, s, task_names, errors);
                }
            }
        }
        Stmt::For(f) => {
            check_pure_expr(fn_name, &f.iterable, task_names, errors);
            for s in &f.body {
                check_pure_stmt(fn_name, s, task_names, errors);
            }
        }
        Stmt::Forward(a, b) => {
            check_pure_expr(fn_name, a, task_names, errors);
            check_pure_expr(fn_name, b, task_names, errors);
        }
        Stmt::Emit(_, args) => {
            for arg in args {
                check_pure_expr(fn_name, &arg.node.value, task_names, errors);
            }
        }
        Stmt::Learn(..) => {
            errors.push(CheckError::PureUsesLlm {
                name: fn_name.to_string(),
                op: "learn",
                span_start: stmt.span.start,
                span_end: stmt.span.end,
            });
        }
        Stmt::Spawn(..) => {
            errors.push(CheckError::PureUsesLlm {
                name: fn_name.to_string(),
                op: "spawn",
                span_start: stmt.span.start,
                span_end: stmt.span.end,
            });
        }
        _ => {}
    }
}

fn check_pure_expr(
    fn_name: &str,
    expr: &Spanned<Expr>,
    task_names: &HashSet<String>,
    errors: &mut Vec<CheckError>,
) {
    match &expr.node {
        Expr::Reason(_) => {
            errors.push(CheckError::PureUsesLlm {
                name: fn_name.to_string(),
                op: "reason",
                span_start: expr.span.start,
                span_end: expr.span.end,
            });
        }
        Expr::Classify(_) => {
            errors.push(CheckError::PureUsesLlm {
                name: fn_name.to_string(),
                op: "classify",
                span_start: expr.span.start,
                span_end: expr.span.end,
            });
        }
        Expr::Search(_) => {
            errors.push(CheckError::PureUsesLlm {
                name: fn_name.to_string(),
                op: "search",
                span_start: expr.span.start,
                span_end: expr.span.end,
            });
        }
        Expr::Recall(_) => {
            errors.push(CheckError::PureUsesLlm {
                name: fn_name.to_string(),
                op: "recall",
                span_start: expr.span.start,
                span_end: expr.span.end,
            });
        }
        Expr::TryOr(a, b) => {
            errors.push(CheckError::PureUsesTryOr {
                name: fn_name.to_string(),
                span_start: expr.span.start,
                span_end: expr.span.end,
            });
            check_pure_expr(fn_name, a, task_names, errors);
            check_pure_expr(fn_name, b, task_names, errors);
        }
        Expr::Call(c) => {
            if task_names.contains(&c.name.node) {
                errors.push(CheckError::PureCallsTask {
                    name: fn_name.to_string(),
                    callee: c.name.node.clone(),
                    span_start: c.name.span.start,
                    span_end: c.name.span.end,
                });
            }
            for arg in &c.args {
                check_pure_expr(fn_name, &arg.node.value, task_names, errors);
            }
        }
        Expr::Compose(parts) | Expr::FanOut(parts) => {
            for p in parts {
                check_pure_expr(fn_name, p, task_names, errors);
            }
        }
        Expr::BinOp(a, _, b) | Expr::Index(a, b) => {
            check_pure_expr(fn_name, a, task_names, errors);
            check_pure_expr(fn_name, b, task_names, errors);
        }
        Expr::UnaryOp(_, a) | Expr::Paren(a) | Expr::FieldAccess(a, _) | Expr::GlobAccess(a) => {
            check_pure_expr(fn_name, a, task_names, errors);
        }
        Expr::MethodCall(inner, _, args) => {
            check_pure_expr(fn_name, inner, task_names, errors);
            for arg in args {
                check_pure_expr(fn_name, &arg.node.value, task_names, errors);
            }
        }
        Expr::Constructor(c) => {
            for arg in &c.args {
                check_pure_expr(fn_name, &arg.node.value, task_names, errors);
            }
        }
        Expr::ArrayLit(elems) => {
            for e in elems {
                check_pure_expr(fn_name, e, task_names, errors);
            }
        }
        Expr::Template(parts) => {
            for part in parts {
                match &part.node {
                    TemplatePart::Interp(inner) | TemplatePart::RawInterp(inner) => {
                        check_pure_expr(fn_name, inner, task_names, errors);
                    }
                    _ => {}
                }
            }
        }
        Expr::Find(_) => {
            errors.push(CheckError::PureUsesLlm {
                name: fn_name.to_string(),
                op: "find",
                span_start: expr.span.start,
                span_end: expr.span.end,
            });
        }
        Expr::Exec(_) => {
            errors.push(CheckError::PureUsesLlm {
                name: fn_name.to_string(),
                op: "exec",
                span_start: expr.span.start,
                span_end: expr.span.end,
            });
        }
        Expr::Command(_) | Expr::CommandMethod(_, _) => {
            errors.push(CheckError::PureUsesLlm {
                name: fn_name.to_string(),
                op: "command",
                span_start: expr.span.start,
                span_end: expr.span.end,
            });
        }
        // Leaves: literals, idents, type access — always pure
        Expr::NumberLit(_) | Expr::BoolLit(_) | Expr::Ident(_) | Expr::TypeAccess(_, _) => {}
    }
}
