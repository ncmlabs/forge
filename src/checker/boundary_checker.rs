// FORGE boundary checker
// See issue #21 for full specification

use std::collections::HashSet;

use crate::ast::{
    BoundaryKind, Expr, Program, Spanned, Stmt, TaskBody, TemplatePart, TopLevel, TypeName,
};
use crate::diagnostic::Diagnostic;

// ── Public API ─────────────────────────────────────────────

pub fn check(programs: &[(&Program, &str)]) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // Phase 1: per-file validation
    for (program, file) in programs {
        let boundary = effective_boundary(program);
        check_endpoint_placement(program, boundary, file, &mut diagnostics);
    }

    // Phase 2: cross-file symbol table + reference validation
    let registry = BoundaryRegistry::build(programs);
    for (program, file) in programs {
        let boundary = effective_boundary(program);
        check_cross_boundary_refs(program, boundary, &registry, file, &mut diagnostics);
    }

    diagnostics
}

// ── Helpers ────────────────────────────────────────────────

/// Determine the effective boundary for a program.
/// Files without a directive default to Shared.
fn effective_boundary(program: &Program) -> BoundaryKind {
    program
        .boundary
        .as_ref()
        .map(|b| b.node.kind.node)
        .unwrap_or(BoundaryKind::Shared)
}

// ── Phase 1: Per-file checks ──────────────────────────────

fn check_endpoint_placement(
    program: &Program,
    boundary: BoundaryKind,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if boundary == BoundaryKind::Server {
        return; // endpoints are legal in server boundary
    }

    for item in &program.items {
        if let TopLevel::Endpoint(ep) = &item.node {
            let boundary_name = match boundary {
                BoundaryKind::Client => "client",
                BoundaryKind::Shared => "shared",
                BoundaryKind::Server => unreachable!(),
            };
            diagnostics.push(
                Diagnostic::error(
                    file,
                    format!(
                        "endpoint `{}` is not allowed in {} boundary",
                        ep.name.node, boundary_name
                    ),
                    ep.name.span.start..ep.name.span.end,
                    "endpoints can only be declared in `server` boundary files",
                )
                .with_help("move this endpoint to a file with `#! boundary: server`"),
            );
        }
    }
}

// ── Phase 2: Cross-file symbol table ──────────────────────

struct BoundaryRegistry {
    server_symbols: HashSet<String>,
    client_symbols: HashSet<String>,
}

impl BoundaryRegistry {
    fn build(programs: &[(&Program, &str)]) -> Self {
        let mut server_symbols = HashSet::new();
        let mut client_symbols = HashSet::new();

        for (program, _file) in programs {
            let boundary = effective_boundary(program);
            for item in &program.items {
                let name = top_level_name(&item.node);
                if let Some(name) = name {
                    match boundary {
                        BoundaryKind::Server => {
                            server_symbols.insert(name);
                        }
                        BoundaryKind::Client => {
                            client_symbols.insert(name);
                        }
                        BoundaryKind::Shared => {
                            // shared symbols are accessible from all boundaries
                        }
                    }
                }
            }
        }

        BoundaryRegistry { server_symbols, client_symbols }
    }
}

/// Extract the name of a top-level declaration, if it has one.
/// Returns None for Use and FnMain (which are skipped per the spec).
fn top_level_name(item: &TopLevel) -> Option<String> {
    match item {
        TopLevel::Task(d) => Some(d.name.node.clone()),
        TopLevel::Pure(d) => Some(d.name.node.clone()),
        TopLevel::Flow(d) => Some(d.name.node.clone()),
        TopLevel::Agent(d) => Some(d.name.node.clone()),
        TopLevel::Pool(d) => Some(d.name.node.clone()),
        TopLevel::Event(d) => Some(d.name.node.clone()),
        TopLevel::States(d) => Some(d.name.node.clone()),
        TopLevel::TypeDef(d) => Some(d.name.node.clone()),
        TopLevel::Endpoint(d) => Some(d.name.node.clone()),
        TopLevel::Contract(d) => Some(d.name.node.clone()),
        TopLevel::System(d) => Some(d.name.node.clone()),
        TopLevel::Use(_) | TopLevel::FnMain(_) => None,
    }
}

// ── Phase 2: Reference walker ──────────────────────────────

fn check_cross_boundary_refs(
    program: &Program,
    boundary: BoundaryKind,
    registry: &BoundaryRegistry,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for item in &program.items {
        match &item.node {
            TopLevel::Task(d) => match &d.body.node {
                TaskBody::Do(stmts) => {
                    check_refs_in_stmts(stmts, boundary, registry, file, diagnostics);
                }
                TaskBody::Is(expr) => {
                    check_refs_in_expr(expr, boundary, registry, file, diagnostics);
                }
            },
            TopLevel::Pure(d) => {
                check_refs_in_stmts(&d.body, boundary, registry, file, diagnostics);
            }
            TopLevel::Flow(d) => {
                for stage in &d.stages {
                    check_refs_in_stmts(&stage.node.body, boundary, registry, file, diagnostics);
                }
            }
            TopLevel::Agent(d) => {
                for handler in &d.handlers {
                    check_refs_in_stmts(
                        &handler.node.body,
                        boundary,
                        registry,
                        file,
                        diagnostics,
                    );
                }
            }
            TopLevel::Endpoint(d) => {
                check_refs_in_stmts(&d.body, boundary, registry, file, diagnostics);
            }
            TopLevel::FnMain(d) => {
                check_refs_in_stmts(&d.body, boundary, registry, file, diagnostics);
            }
            // No walkable bodies for these declarations
            TopLevel::Use(_)
            | TopLevel::Pool(_)
            | TopLevel::Event(_)
            | TopLevel::States(_)
            | TopLevel::TypeDef(_)
            | TopLevel::Contract(_)
            | TopLevel::System(_) => {}
        }
    }
}

fn check_refs_in_stmts(
    stmts: &[Spanned<Stmt>],
    boundary: BoundaryKind,
    registry: &BoundaryRegistry,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for stmt in stmts {
        check_refs_in_stmt(stmt, boundary, registry, file, diagnostics);
    }
}

fn check_refs_in_stmt(
    stmt: &Spanned<Stmt>,
    boundary: BoundaryKind,
    registry: &BoundaryRegistry,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match &stmt.node {
        Stmt::Bind(_, expr) | Stmt::Say(expr) | Stmt::ExprStmt(expr) => {
            check_refs_in_expr(expr, boundary, registry, file, diagnostics);
        }
        Stmt::Give(expr, with_expr) => {
            check_refs_in_expr(expr, boundary, registry, file, diagnostics);
            if let Some(w) = with_expr {
                check_refs_in_expr(w, boundary, registry, file, diagnostics);
            }
        }
        Stmt::Emit(name, args) => {
            check_name_ref(
                &name.node,
                name.span.start,
                name.span.end,
                boundary,
                registry,
                file,
                diagnostics,
            );
            for arg in args {
                check_refs_in_expr(&arg.node.value, boundary, registry, file, diagnostics);
            }
        }
        Stmt::Escalate(name) => {
            check_name_ref(
                &name.node,
                name.span.start,
                name.span.end,
                boundary,
                registry,
                file,
                diagnostics,
            );
        }
        Stmt::Forward(a, b) => {
            check_refs_in_expr(a, boundary, registry, file, diagnostics);
            check_refs_in_expr(b, boundary, registry, file, diagnostics);
        }
        Stmt::IfElse(ie) => {
            check_refs_in_expr(&ie.condition, boundary, registry, file, diagnostics);
            check_refs_in_stmts(&ie.then_body, boundary, registry, file, diagnostics);
            for (cond, body) in &ie.else_ifs {
                check_refs_in_expr(cond, boundary, registry, file, diagnostics);
                check_refs_in_stmts(body, boundary, registry, file, diagnostics);
            }
            if let Some(body) = &ie.else_body {
                check_refs_in_stmts(body, boundary, registry, file, diagnostics);
            }
        }
        Stmt::When(when) => {
            for clause in &when.clauses {
                check_refs_in_stmt(&clause.node.body, boundary, registry, file, diagnostics);
            }
            if let Some(else_clause) = &when.else_body {
                check_refs_in_stmt(&else_clause.node.body, boundary, registry, file, diagnostics);
            }
        }
        Stmt::Match(m) => {
            check_refs_in_expr(&m.subject, boundary, registry, file, diagnostics);
            for arm in &m.arms {
                check_refs_in_stmt(&arm.node.body, boundary, registry, file, diagnostics);
            }
        }
        Stmt::For(f) => {
            check_refs_in_expr(&f.iterable, boundary, registry, file, diagnostics);
            check_refs_in_stmts(&f.body, boundary, registry, file, diagnostics);
        }
        Stmt::MemoryUpdate(_, idx, expr) => {
            if let Some(i) = idx {
                check_refs_in_expr(i, boundary, registry, file, diagnostics);
            }
            check_refs_in_expr(expr, boundary, registry, file, diagnostics);
        }
        // No expressions to walk
        Stmt::TransitionTo(_)
        | Stmt::StartTimer { .. }
        | Stmt::CancelTimer { .. }
        | Stmt::ResetTimer(_) => {}
    }
}

fn check_refs_in_expr(
    expr: &Spanned<Expr>,
    boundary: BoundaryKind,
    registry: &BoundaryRegistry,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match &expr.node {
        Expr::Ident(name) => {
            check_name_ref(
                name,
                expr.span.start,
                expr.span.end,
                boundary,
                registry,
                file,
                diagnostics,
            );
        }
        Expr::Call(c) => {
            check_name_ref(
                &c.name.node,
                c.name.span.start,
                c.name.span.end,
                boundary,
                registry,
                file,
                diagnostics,
            );
            for arg in &c.args {
                check_refs_in_expr(&arg.node.value, boundary, registry, file, diagnostics);
            }
        }
        Expr::Constructor(c) => {
            if let TypeName::Custom(name) = &c.type_name.node {
                check_name_ref(
                    name,
                    c.type_name.span.start,
                    c.type_name.span.end,
                    boundary,
                    registry,
                    file,
                    diagnostics,
                );
            }
            for arg in &c.args {
                check_refs_in_expr(&arg.node.value, boundary, registry, file, diagnostics);
            }
        }
        Expr::Reason(inner) | Expr::Search(inner) => {
            check_refs_in_expr(inner, boundary, registry, file, diagnostics);
        }
        Expr::Classify(c) => {
            check_refs_in_expr(&c.input, boundary, registry, file, diagnostics);
        }
        Expr::TryOr(a, b) => {
            check_refs_in_expr(a, boundary, registry, file, diagnostics);
            check_refs_in_expr(b, boundary, registry, file, diagnostics);
        }
        Expr::Compose(parts) | Expr::FanOut(parts) => {
            for p in parts {
                check_refs_in_expr(p, boundary, registry, file, diagnostics);
            }
        }
        Expr::BinOp(a, _, b) | Expr::Index(a, b) => {
            check_refs_in_expr(a, boundary, registry, file, diagnostics);
            check_refs_in_expr(b, boundary, registry, file, diagnostics);
        }
        Expr::UnaryOp(_, a) | Expr::Paren(a) | Expr::FieldAccess(a, _) | Expr::GlobAccess(a) => {
            check_refs_in_expr(a, boundary, registry, file, diagnostics);
        }
        Expr::MethodCall(inner, _, args) => {
            check_refs_in_expr(inner, boundary, registry, file, diagnostics);
            for arg in args {
                check_refs_in_expr(&arg.node.value, boundary, registry, file, diagnostics);
            }
        }
        Expr::ArrayLit(elems) => {
            for e in elems {
                check_refs_in_expr(e, boundary, registry, file, diagnostics);
            }
        }
        Expr::Template(parts) => {
            for part in parts {
                if let TemplatePart::Interp(inner) = &part.node {
                    check_refs_in_expr(inner, boundary, registry, file, diagnostics);
                }
            }
        }
        // Leaves: literals, type access — no symbol references
        Expr::NumberLit(_) | Expr::BoolLit(_) | Expr::TypeAccess(_, _) => {}
    }
}

fn check_name_ref(
    name: &str,
    span_start: usize,
    span_end: usize,
    file_boundary: BoundaryKind,
    registry: &BoundaryRegistry,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match file_boundary {
        BoundaryKind::Client => {
            if registry.server_symbols.contains(name) {
                diagnostics.push(Diagnostic::error(
                    file,
                    format!("client code references server-only symbol `{}`", name),
                    span_start..span_end,
                    "this symbol is declared in a server boundary and cannot be used in client code",
                ));
            }
        }
        BoundaryKind::Server => {
            if registry.client_symbols.contains(name) {
                diagnostics.push(Diagnostic::error(
                    file,
                    format!("server code references client-only symbol `{}`", name),
                    span_start..span_end,
                    "this symbol is declared in a client boundary and cannot be used in server code",
                ));
            }
        }
        BoundaryKind::Shared => {
            if registry.server_symbols.contains(name) {
                diagnostics.push(Diagnostic::error(
                    file,
                    format!("shared code references server-only symbol `{}`", name),
                    span_start..span_end,
                    "this symbol is declared in a server boundary and cannot be used in shared code",
                ));
            } else if registry.client_symbols.contains(name) {
                diagnostics.push(Diagnostic::error(
                    file,
                    format!("shared code references client-only symbol `{}`", name),
                    span_start..span_end,
                    "this symbol is declared in a client boundary and cannot be used in shared code",
                ));
            }
        }
    }
}
