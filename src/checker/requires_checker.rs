// FORGE requires checker
// See issue #18 for full specification

use std::collections::HashSet;

use crate::ast::{Expr, FailPolicy, Program, Spanned, TemplatePart, TopLevel};
use crate::diagnostic::Diagnostic;

// ── Public API ──────────────────────────────────────────────

pub fn check(program: &Program, file: &str) -> Vec<Diagnostic> {
    let mut task_names = HashSet::new();
    for item in &program.items {
        if let TopLevel::Task(task) = &item.node {
            task_names.insert(task.name.node.clone());
        }
    }

    let mut diagnostics = Vec::new();

    for item in &program.items {
        if let TopLevel::Agent(agent) = &item.node {
            for handler in &agent.handlers {
                for req in &handler.node.requires {
                    check_requires_expr(&req.node.condition, &task_names, file, &mut diagnostics);
                    if let Some(Spanned {
                        node: FailPolicy::Give(expr),
                        ..
                    }) = &req.node.on_fail
                    {
                        check_requires_expr(expr, &task_names, file, &mut diagnostics);
                    }
                }
            }
        }
    }

    diagnostics
}

// ── Expression walker ───────────────────────────────────────

fn check_requires_expr(
    expr: &Spanned<Expr>,
    task_names: &HashSet<String>,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match &expr.node {
        Expr::Reason(_) => {
            diagnostics.push(
                Diagnostic::warning(
                    file,
                    "requires clause uses LLM operation `reason`",
                    expr.span.start..expr.span.end,
                    "LLM operations are stochastic — preconditions should be deterministic",
                )
                .with_help("use a `pure` function instead"),
            );
        }
        Expr::Classify(_) => {
            diagnostics.push(
                Diagnostic::warning(
                    file,
                    "requires clause uses LLM operation `classify`",
                    expr.span.start..expr.span.end,
                    "LLM operations are stochastic — preconditions should be deterministic",
                )
                .with_help("use a `pure` function instead"),
            );
        }
        Expr::Search(_) => {
            diagnostics.push(
                Diagnostic::warning(
                    file,
                    "requires clause uses LLM operation `search`",
                    expr.span.start..expr.span.end,
                    "LLM operations are stochastic — preconditions should be deterministic",
                )
                .with_help("use a `pure` function instead"),
            );
        }
        Expr::Recall(_) => {
            diagnostics.push(
                Diagnostic::warning(
                    file,
                    "requires clause uses knowledge operation `recall`",
                    expr.span.start..expr.span.end,
                    "knowledge retrieval is non-deterministic — preconditions should be deterministic",
                )
                .with_help("use a `pure` function instead"),
            );
        }
        Expr::TryOr(a, b) => {
            diagnostics.push(
                Diagnostic::warning(
                    file,
                    "requires clause uses `try...or` which wraps stochastic operations",
                    expr.span.start..expr.span.end,
                    "preconditions should be deterministic",
                )
                .with_help("use a `pure` function instead"),
            );
            check_requires_expr(a, task_names, file, diagnostics);
            check_requires_expr(b, task_names, file, diagnostics);
        }
        Expr::Call(c) => {
            if task_names.contains(&c.name.node) {
                diagnostics.push(
                    Diagnostic::warning(
                        file,
                        format!(
                            "requires clause calls task `{}` which may be stochastic",
                            c.name.node
                        ),
                        c.name.span.start..c.name.span.end,
                        "preconditions should be deterministic",
                    )
                    .with_help("use a `pure` function instead"),
                );
            }
            for arg in &c.args {
                check_requires_expr(&arg.node.value, task_names, file, diagnostics);
            }
        }
        Expr::Compose(parts) | Expr::FanOut(parts) => {
            for p in parts {
                check_requires_expr(p, task_names, file, diagnostics);
            }
        }
        Expr::BinOp(a, _, b) | Expr::Index(a, b) => {
            check_requires_expr(a, task_names, file, diagnostics);
            check_requires_expr(b, task_names, file, diagnostics);
        }
        Expr::UnaryOp(_, a) | Expr::Paren(a) | Expr::FieldAccess(a, _) | Expr::GlobAccess(a) => {
            check_requires_expr(a, task_names, file, diagnostics);
        }
        Expr::MethodCall(inner, _, args) => {
            check_requires_expr(inner, task_names, file, diagnostics);
            for arg in args {
                check_requires_expr(&arg.node.value, task_names, file, diagnostics);
            }
        }
        Expr::Constructor(c) => {
            for arg in &c.args {
                check_requires_expr(&arg.node.value, task_names, file, diagnostics);
            }
        }
        Expr::ArrayLit(elems) => {
            for e in elems {
                check_requires_expr(e, task_names, file, diagnostics);
            }
        }
        Expr::Template(parts) => {
            for part in parts {
                match &part.node {
                    TemplatePart::Interp(inner) | TemplatePart::RawInterp(inner) => {
                        check_requires_expr(inner, task_names, file, diagnostics);
                    }
                    _ => {}
                }
            }
        }
        Expr::Find(_) => {
            diagnostics.push(
                Diagnostic::warning(
                    file,
                    "requires clause uses `find` which queries runtime state",
                    expr.span.start..expr.span.end,
                    "runtime state is non-deterministic — preconditions should be deterministic",
                )
                .with_help("use a `pure` function instead"),
            );
        }
        Expr::Exec(_) => {
            diagnostics.push(
                Diagnostic::warning(
                    file,
                    "requires clause uses `exec` which runs an external process",
                    expr.span.start..expr.span.end,
                    "external processes are non-deterministic — preconditions should be deterministic",
                )
                .with_help("use a `pure` function instead"),
            );
        }
        Expr::Command(_) | Expr::CommandMethod(_, _) => {
            diagnostics.push(
                Diagnostic::warning(
                    file,
                    "requires clause uses `command` which runs an external process",
                    expr.span.start..expr.span.end,
                    "external processes are non-deterministic — preconditions should be deterministic",
                )
                .with_help("use a `pure` function instead"),
            );
        }
        Expr::Session(_) => {
            diagnostics.push(
                Diagnostic::warning(
                    file,
                    "requires clause uses `session` which delegates to an external agent",
                    expr.span.start..expr.span.end,
                    "external agent sessions are non-deterministic — preconditions should be deterministic",
                )
                .with_help("use a `pure` function instead"),
            );
        }
        // Leaves: literals, idents, type access — always deterministic
        Expr::NumberLit(_) | Expr::BoolLit(_) | Expr::Ident(_) | Expr::TypeAccess(_, _) => {}
    }
}
