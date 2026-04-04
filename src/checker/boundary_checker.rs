// FORGE boundary checker
// See issue #21 for full specification

use std::collections::{HashMap, HashSet};

use crate::ast::{
    BoundaryKind, Expr, FieldDef, LearnSource, Program, Spanned, SpawnOption, Stmt, TaskBody,
    TemplatePart, TopLevel, TypeName,
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

    // Phase 2a: shared type serializability
    check_shared_serializability(programs, &registry, &mut diagnostics);

    // Phase 2b: cross-boundary reference validation
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

#[derive(Debug, Clone, Copy)]
enum SymbolKind {
    Task,
    Pure,
    Flow,
    Agent,
    Pool,
    Event,
    States,
    TypeDef,
    Endpoint,
    Contract,
    System,
}

struct BoundaryRegistry {
    server_symbols: HashSet<String>,
    client_symbols: HashSet<String>,
    /// Maps symbol name to its kind (needed for serializability check)
    symbol_kinds: HashMap<String, SymbolKind>,
}

impl BoundaryRegistry {
    fn build(programs: &[(&Program, &str)]) -> Self {
        let mut server_symbols = HashSet::new();
        let mut client_symbols = HashSet::new();
        let mut symbol_kinds = HashMap::new();

        for (program, _file) in programs {
            let boundary = effective_boundary(program);
            for item in &program.items {
                let name_and_kind = top_level_name_and_kind(&item.node);
                if let Some((name, kind)) = name_and_kind {
                    symbol_kinds.insert(name.clone(), kind);
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

        BoundaryRegistry {
            server_symbols,
            client_symbols,
            symbol_kinds,
        }
    }
}

/// Extract the name and kind of a top-level declaration, if it has one.
/// Returns None for Use and FnMain (which are skipped per the spec).
fn top_level_name_and_kind(item: &TopLevel) -> Option<(String, SymbolKind)> {
    match item {
        TopLevel::Task(d) => Some((d.name.node.clone(), SymbolKind::Task)),
        TopLevel::Pure(d) => Some((d.name.node.clone(), SymbolKind::Pure)),
        TopLevel::Flow(d) => Some((d.name.node.clone(), SymbolKind::Flow)),
        TopLevel::Agent(d) => Some((d.name.node.clone(), SymbolKind::Agent)),
        TopLevel::Pool(d) => Some((d.name.node.clone(), SymbolKind::Pool)),
        TopLevel::Event(d) => Some((d.name.node.clone(), SymbolKind::Event)),
        TopLevel::States(d) => Some((d.name.node.clone(), SymbolKind::States)),
        TopLevel::TypeDef(d) => Some((d.name.node.clone(), SymbolKind::TypeDef)),
        TopLevel::Endpoint(d) => Some((d.name.node.clone(), SymbolKind::Endpoint)),
        TopLevel::Contract(d) => Some((d.name.node.clone(), SymbolKind::Contract)),
        TopLevel::System(d) => Some((d.name.node.clone(), SymbolKind::System)),
        TopLevel::Warden(d) => Some((d.name.node.clone(), SymbolKind::System)),
        TopLevel::Use(_) | TopLevel::FnMain(_) | TopLevel::Import(_) => None,
    }
}

// ── Phase 2a: Shared type serializability ─────────────────

fn check_shared_serializability(
    programs: &[(&Program, &str)],
    registry: &BoundaryRegistry,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for (program, file) in programs {
        let boundary = effective_boundary(program);
        if boundary != BoundaryKind::Shared {
            continue;
        }

        for item in &program.items {
            let (decl_name, fields) = match &item.node {
                TopLevel::TypeDef(td) => (&td.name.node, &td.fields),
                TopLevel::Event(ev) => (&ev.name.node, &ev.fields),
                _ => continue,
            };
            check_fields_serializable(decl_name, fields, registry, file, diagnostics);
        }
    }
}

fn check_fields_serializable(
    decl_name: &str,
    fields: &[Spanned<FieldDef>],
    registry: &BoundaryRegistry,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for field in fields {
        if let TypeName::Custom(ref_name) = &field.node.type_name.node {
            if let Some(kind) = registry.symbol_kinds.get(ref_name.as_str()) {
                let non_serializable = matches!(
                    kind,
                    SymbolKind::Agent | SymbolKind::Pool | SymbolKind::Flow
                );
                if non_serializable {
                    let kind_name = match kind {
                        SymbolKind::Agent => "agent",
                        SymbolKind::Pool => "pool",
                        SymbolKind::Flow => "flow",
                        _ => unreachable!(),
                    };
                    diagnostics.push(
                        Diagnostic::error(
                            file,
                            format!(
                                "shared type `{}` contains non-serializable field `{}`",
                                decl_name, field.node.name
                            ),
                            field.node.type_name.span.start..field.node.type_name.span.end,
                            format!(
                                "`{}` is an {} reference, which cannot cross the wire",
                                ref_name, kind_name
                            ),
                        )
                        .with_help(
                            "use a shared type or primitive type for shared boundary fields",
                        ),
                    );
                }
            }
        }
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
                    check_refs_in_stmts(&handler.node.body, boundary, registry, file, diagnostics);
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
            | TopLevel::Warden(_)
            | TopLevel::Event(_)
            | TopLevel::States(_)
            | TopLevel::TypeDef(_)
            | TopLevel::Contract(_)
            | TopLevel::System(_)
            | TopLevel::Import(_) => {}
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
        Stmt::Give(expr, metas) => {
            check_refs_in_expr(expr, boundary, registry, file, diagnostics);
            for meta in metas {
                check_refs_in_expr(&meta.node.value, boundary, registry, file, diagnostics);
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
                check_refs_in_stmt(
                    &else_clause.node.body,
                    boundary,
                    registry,
                    file,
                    diagnostics,
                );
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
        Stmt::Learn(source, category) => {
            match &source.node {
                LearnSource::Direct(expr) | LearnSource::FromDocument(expr) => {
                    check_refs_in_expr(expr, boundary, registry, file, diagnostics);
                }
                LearnSource::FromInteraction(args) => {
                    for arg in args {
                        check_refs_in_expr(&arg.node.value, boundary, registry, file, diagnostics);
                    }
                }
            }
            if let Some(cat_expr) = category {
                check_refs_in_expr(cat_expr, boundary, registry, file, diagnostics);
            }
        }
        Stmt::Spawn(s) => {
            if let Some(ref alias) = s.alias {
                check_refs_in_expr(alias, boundary, registry, file, diagnostics);
            }
            for opt in &s.options {
                match &opt.node {
                    SpawnOption::ConfidenceCap(expr) | SpawnOption::MemoryInit(_, expr) => {
                        check_refs_in_expr(expr, boundary, registry, file, diagnostics);
                    }
                    SpawnOption::KnowledgeFilter(_) => {}
                }
            }
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
        Expr::Reason(inner) | Expr::Search(inner) | Expr::Recall(inner) => {
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
