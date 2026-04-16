// FORGE spawn checker — issue #83
// Validates spawn statements: warns if the template agent has no failure policy
// (Principle VII — Accountability).

use std::collections::HashMap;

use crate::ast::*;
use crate::diagnostic::Diagnostic;

/// Run spawn checks on a single program.
pub fn check(program: &Program, file: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // Collect agent declarations by name
    let agents: HashMap<&str, &AgentDecl> = program
        .items
        .iter()
        .filter_map(|item| match &item.node {
            TopLevel::Agent(a) => Some((a.name.node.as_str(), a.as_ref())),
            _ => None,
        })
        .collect();

    // Walk all statement-bearing bodies in the program
    for item in &program.items {
        match &item.node {
            TopLevel::Task(t) => {
                if let TaskBody::Do(stmts) = &t.body.node {
                    check_stmts(stmts, &agents, file, &mut diagnostics);
                }
                if let Some(ref fail_stmts) = t.if_fails {
                    check_stmts(fail_stmts, &agents, file, &mut diagnostics);
                }
            }
            TopLevel::Agent(a) => {
                for handler in &a.handlers {
                    check_stmts(&handler.node.body, &agents, file, &mut diagnostics);
                }
            }
            TopLevel::Flow(f) => {
                for stage in &f.stages {
                    check_stmts(&stage.node.body, &agents, file, &mut diagnostics);
                }
            }
            TopLevel::FnMain(main) => {
                check_stmts(&main.body, &agents, file, &mut diagnostics);
            }
            _ => {}
        }
    }

    diagnostics
}

fn check_stmts(
    stmts: &[Spanned<Stmt>],
    agents: &HashMap<&str, &AgentDecl>,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for stmt in stmts {
        check_stmt(stmt, agents, file, diagnostics);
    }
}

fn check_stmt(
    stmt: &Spanned<Stmt>,
    agents: &HashMap<&str, &AgentDecl>,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match &stmt.node {
        Stmt::Retire(_) => {
            // Retire with knowledge export: the runtime will produce a proper
            // error if no knowledge store — nothing to check statically.
        }
        Stmt::Spawn(s) => {
            let template_name = &s.template.node;
            if let Some(agent) = agents.get(template_name.as_str()) {
                if agent.stuck_policy.is_none() {
                    diagnostics.push(
                        Diagnostic::warning(
                            file,
                            format!(
                                "spawning agent `{}` which has no failure policy",
                                template_name
                            ),
                            s.template.span.start..s.template.span.end,
                            "this agent has no stuck_policy or if_stuck handler",
                        )
                        .with_help(
                            "add a failure policy to the agent declaration for Principle VII (Accountability)",
                        ),
                    );
                }
            }
        }
        // Check find expressions in bind statements
        Stmt::Bind(_, expr) | Stmt::ExprStmt(expr) => {
            check_find_in_expr(expr, agents, file, diagnostics);
        }
        // Recurse into nested statement bodies
        Stmt::IfElse(ie) => {
            check_stmts(&ie.then_body, agents, file, diagnostics);
            for (_, body) in &ie.else_ifs {
                check_stmts(body, agents, file, diagnostics);
            }
            if let Some(ref body) = ie.else_body {
                check_stmts(body, agents, file, diagnostics);
            }
        }
        Stmt::For(f) => {
            check_stmts(&f.body, agents, file, diagnostics);
        }
        _ => {}
    }
}

/// Warn when `find all X` references an unknown agent template.
fn check_find_in_expr(
    expr: &Spanned<Expr>,
    agents: &HashMap<&str, &AgentDecl>,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Expr::Find(f) = &expr.node {
        let template = match &f.kind {
            FindKind::AllByTemplate(t) | FindKind::AllByTemplateFiltered(t, _) => Some(t),
            FindKind::ByAlias(_) => None,
        };
        if let Some(t) = template {
            if !agents.contains_key(t.node.as_str()) {
                diagnostics.push(
                    Diagnostic::warning(
                        file,
                        format!("find references unknown agent template `{}`", t.node),
                        t.span.start..t.span.end,
                        "no agent declaration with this name exists in the program",
                    )
                    .with_help("check the agent name or ensure it is declared"),
                );
            }
        }
    }
}
