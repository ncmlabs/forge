// FORGE capability resolver
// See issue #6 for full implementation

use std::collections::HashMap;

use crate::ast::{Expr, Program, Spanned, Stmt, TopLevel};
use crate::diagnostic::Diagnostic;
use crate::types::{is_compatible, CapabilitySignature, ForgeType};

// ── Errors ───────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("unknown capability `{name}`")]
    UnknownCapability {
        name: String,
        span_start: usize,
        span_end: usize,
    },

    #[error("composition type mismatch: `{left}` is not compatible with `{right}`")]
    CompositionMismatch {
        left: String,
        right: String,
        span_start: usize,
        span_end: usize,
    },

    #[error("pure function `{name}` cannot use LLM operations (reason/classify/search)")]
    PureUsesLlm {
        name: String,
        span_start: usize,
        span_end: usize,
    },
}

impl ResolveError {
    pub fn to_diagnostic(&self, file: &str, registry: &CapabilityRegistry) -> Diagnostic {
        match self {
            ResolveError::UnknownCapability { name, span_start, span_end } => {
                let namespace = name.split('.').next().unwrap_or("");
                let available: Vec<&str> = registry.capabilities.keys()
                    .filter(|k| k.starts_with(namespace))
                    .map(|k| k.as_str())
                    .collect();
                let help = if available.is_empty() {
                    None
                } else {
                    Some(format!("available {} capabilities: {}", namespace, available.join(", ")))
                };
                let mut diag = Diagnostic::error(
                    file,
                    format!("unknown capability '{}'", name),
                    *span_start..*span_end,
                    "not found in capability registry",
                );
                if let Some(h) = help {
                    diag = diag.with_help(h);
                }
                diag
            }
            ResolveError::CompositionMismatch { left, right, span_start, span_end } => {
                Diagnostic::error(
                    file,
                    format!("composition type mismatch: `{}` → `{}`", left, right),
                    *span_start..*span_end,
                    format!("`{}` is not compatible with `{}`", left, right),
                )
            }
            ResolveError::PureUsesLlm { name, span_start, span_end } => {
                Diagnostic::error(
                    file,
                    format!("pure function `{}` cannot use LLM operations", name),
                    *span_start..*span_end,
                    "reason/classify/search not allowed in pure functions",
                ).with_help("move this operation to a `task` instead")
            }
        }
    }
}

// ── Capability registry ──────────────────────────────────────

pub struct CapabilityRegistry {
    capabilities: HashMap<String, CapabilitySignature>,
}

impl CapabilityRegistry {
    /// Create a registry with all built-in capabilities.
    pub fn builtin() -> Self {
        let mut caps = HashMap::new();

        caps.insert("llm.reason".into(), CapabilitySignature {
            inputs: vec![ForgeType::Text],
            output: ForgeType::Text,
        });
        caps.insert("llm.classify".into(), CapabilitySignature {
            inputs: vec![ForgeType::Text],
            output: ForgeType::Classification,
        });
        caps.insert("web.search".into(), CapabilitySignature {
            inputs: vec![ForgeType::Text],
            output: ForgeType::Results,
        });
        caps.insert("data.store".into(), CapabilitySignature {
            inputs: vec![ForgeType::Text, ForgeType::Text],
            output: ForgeType::Unit,
        });
        caps.insert("data.embed".into(), CapabilitySignature {
            inputs: vec![ForgeType::Text],
            output: ForgeType::Embedding,
        });

        Self { capabilities: caps }
    }

    pub fn resolve(&self, name: &str) -> Option<&CapabilitySignature> {
        self.capabilities.get(name)
    }
}

// ── Check context ────────────────────────────────────────────

pub struct CheckContext {
    pub registry: CapabilityRegistry,
    errors: Vec<ResolveError>,
}

impl CheckContext {
    pub fn new(_file: &str) -> Self {
        Self {
            registry: CapabilityRegistry::builtin(),
            errors: Vec::new(),
        }
    }

    /// Run all checks on the program. Returns Ok(()) if no errors, or Err with all collected errors.
    pub fn check(mut self, program: &Program) -> Result<(), Vec<ResolveError>> {
        // Pass 1: resolve use declarations
        for item in &program.items {
            if let TopLevel::Use(use_decl) = &item.node {
                for cap in &use_decl.capabilities {
                    if self.registry.resolve(&cap.node).is_none() {
                        self.errors.push(ResolveError::UnknownCapability {
                            name: cap.node.clone(),
                            span_start: cap.span.start,
                            span_end: cap.span.end,
                        });
                    }
                }
            }
        }

        // Pass 2: check declarations
        for item in &program.items {
            match &item.node {
                TopLevel::Task(task) => {
                    self.check_stmts_composition(&task_body_stmts(task));
                }
                TopLevel::Pure(pure) => {
                    // Check purity: no LLM operations allowed
                    self.check_pure_body(&pure.name.node, &pure.body);
                    self.check_stmts_composition(&pure.body);
                }
                TopLevel::Flow(flow) => {
                    for stage in &flow.stages {
                        self.check_stmts_composition(&stage.node.body);
                    }
                }
                TopLevel::Agent(agent) => {
                    for handler in &agent.handlers {
                        self.check_stmts_composition(&handler.node.body);
                    }
                }
                TopLevel::Endpoint(ep) => {
                    self.check_stmts_composition(&ep.body);
                }
                TopLevel::FnMain(main) => {
                    self.check_stmts_composition(&main.body);
                }
                _ => {}
            }
        }

        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(self.errors)
        }
    }

    /// Check that a pure function body contains no LLM operations.
    fn check_pure_body(&mut self, fn_name: &str, stmts: &[Spanned<Stmt>]) {
        for stmt in stmts {
            self.check_pure_stmt(fn_name, stmt);
        }
    }

    fn check_pure_stmt(&mut self, fn_name: &str, stmt: &Spanned<Stmt>) {
        match &stmt.node {
            Stmt::Bind(_, expr) | Stmt::Give(expr, _) | Stmt::Say(expr) | Stmt::ExprStmt(expr) => {
                self.check_pure_expr(fn_name, expr);
            }
            Stmt::When(when) => {
                for clause in &when.clauses {
                    self.check_pure_stmt(fn_name, &clause.node.body);
                }
                if let Some(else_clause) = &when.else_body {
                    self.check_pure_stmt(fn_name, &else_clause.node.body);
                }
            }
            Stmt::Match(m) => {
                self.check_pure_expr(fn_name, &m.subject);
                for arm in &m.arms {
                    self.check_pure_stmt(fn_name, &arm.node.body);
                }
            }
            Stmt::IfElse(ie) => {
                self.check_pure_expr(fn_name, &ie.condition);
                for s in &ie.then_body {
                    self.check_pure_stmt(fn_name, s);
                }
                for (cond, body) in &ie.else_ifs {
                    self.check_pure_expr(fn_name, cond);
                    for s in body {
                        self.check_pure_stmt(fn_name, s);
                    }
                }
                if let Some(body) = &ie.else_body {
                    for s in body {
                        self.check_pure_stmt(fn_name, s);
                    }
                }
            }
            Stmt::For(f) => {
                self.check_pure_expr(fn_name, &f.iterable);
                for s in &f.body {
                    self.check_pure_stmt(fn_name, s);
                }
            }
            Stmt::Forward(a, b) => {
                self.check_pure_expr(fn_name, a);
                self.check_pure_expr(fn_name, b);
            }
            Stmt::Emit(_, args) => {
                for arg in args {
                    self.check_pure_expr(fn_name, &arg.node.value);
                }
            }
            _ => {}
        }
    }

    fn check_pure_expr(&mut self, fn_name: &str, expr: &Spanned<Expr>) {
        match &expr.node {
            Expr::Reason(_) | Expr::Classify(_) | Expr::Search(_) => {
                self.errors.push(ResolveError::PureUsesLlm {
                    name: fn_name.to_string(),
                    span_start: expr.span.start,
                    span_end: expr.span.end,
                });
            }
            Expr::TryOr(a, b) => {
                self.check_pure_expr(fn_name, a);
                self.check_pure_expr(fn_name, b);
            }
            Expr::Compose(parts) => {
                for p in parts {
                    self.check_pure_expr(fn_name, p);
                }
            }
            Expr::FanOut(parts) => {
                for p in parts {
                    self.check_pure_expr(fn_name, p);
                }
            }
            Expr::BinOp(a, _, b) => {
                self.check_pure_expr(fn_name, a);
                self.check_pure_expr(fn_name, b);
            }
            Expr::UnaryOp(_, a) => {
                self.check_pure_expr(fn_name, a);
            }
            Expr::Call(c) => {
                for arg in &c.args {
                    self.check_pure_expr(fn_name, &arg.node.value);
                }
            }
            Expr::Constructor(c) => {
                for arg in &c.args {
                    self.check_pure_expr(fn_name, &arg.node.value);
                }
            }
            Expr::FieldAccess(inner, _) | Expr::GlobAccess(inner) => {
                self.check_pure_expr(fn_name, inner);
            }
            Expr::Index(a, b) => {
                self.check_pure_expr(fn_name, a);
                self.check_pure_expr(fn_name, b);
            }
            Expr::MethodCall(inner, _, args) => {
                self.check_pure_expr(fn_name, inner);
                for arg in args {
                    self.check_pure_expr(fn_name, &arg.node.value);
                }
            }
            Expr::Paren(inner) => {
                self.check_pure_expr(fn_name, inner);
            }
            Expr::ArrayLit(elems) => {
                for e in elems {
                    self.check_pure_expr(fn_name, e);
                }
            }
            Expr::Template(parts) => {
                for part in parts {
                    if let crate::ast::TemplatePart::Interp(inner) = &part.node {
                        self.check_pure_expr(fn_name, inner);
                    }
                }
            }
            // Leaves: literals, idents, type access — always pure
            Expr::NumberLit(_)
            | Expr::BoolLit(_)
            | Expr::Ident(_)
            | Expr::TypeAccess(_, _) => {}
        }
    }

    /// Walk statements looking for Compose expressions and check type compatibility.
    fn check_stmts_composition(&mut self, stmts: &[Spanned<Stmt>]) {
        for stmt in stmts {
            self.check_stmt_composition(stmt);
        }
    }

    fn check_stmt_composition(&mut self, stmt: &Spanned<Stmt>) {
        match &stmt.node {
            Stmt::Bind(_, expr) | Stmt::Give(expr, _) | Stmt::Say(expr) | Stmt::ExprStmt(expr) => {
                self.check_expr_composition(expr);
            }
            Stmt::When(when) => {
                for clause in &when.clauses {
                    self.check_stmt_composition(&clause.node.body);
                }
                if let Some(else_clause) = &when.else_body {
                    self.check_stmt_composition(&else_clause.node.body);
                }
            }
            Stmt::Match(m) => {
                self.check_expr_composition(&m.subject);
                for arm in &m.arms {
                    self.check_stmt_composition(&arm.node.body);
                }
            }
            Stmt::IfElse(ie) => {
                self.check_expr_composition(&ie.condition);
                self.check_stmts_composition(&ie.then_body);
                for (cond, body) in &ie.else_ifs {
                    self.check_expr_composition(cond);
                    self.check_stmts_composition(body);
                }
                if let Some(body) = &ie.else_body {
                    self.check_stmts_composition(body);
                }
            }
            Stmt::For(f) => {
                self.check_expr_composition(&f.iterable);
                self.check_stmts_composition(&f.body);
            }
            _ => {}
        }
    }

    fn check_expr_composition(&mut self, expr: &Spanned<Expr>) {
        match &expr.node {
            Expr::Compose(parts) => {
                // Check adjacent pairs for type compatibility
                for window in parts.windows(2) {
                    let left = &window[0];
                    let right = &window[1];
                    if let (Some(lt), Some(rt)) = (infer_type(left), infer_input_type(right)) {
                        if !is_compatible(&lt, &rt) {
                            self.errors.push(ResolveError::CompositionMismatch {
                                left: lt.to_string(),
                                right: rt.to_string(),
                                span_start: right.span.start,
                                span_end: right.span.end,
                            });
                        }
                    }
                }
                // Recurse into sub-expressions
                for p in parts {
                    self.check_expr_composition(p);
                }
            }
            Expr::TryOr(a, b) => {
                self.check_expr_composition(a);
                self.check_expr_composition(b);
            }
            Expr::FanOut(parts) => {
                for p in parts {
                    self.check_expr_composition(p);
                }
            }
            Expr::BinOp(a, _, b) => {
                self.check_expr_composition(a);
                self.check_expr_composition(b);
            }
            Expr::UnaryOp(_, a) => {
                self.check_expr_composition(a);
            }
            Expr::Call(c) => {
                for arg in &c.args {
                    self.check_expr_composition(&arg.node.value);
                }
            }
            Expr::Paren(inner) | Expr::FieldAccess(inner, _) | Expr::GlobAccess(inner) => {
                self.check_expr_composition(inner);
            }
            Expr::Index(a, b) => {
                self.check_expr_composition(a);
                self.check_expr_composition(b);
            }
            Expr::MethodCall(inner, _, args) => {
                self.check_expr_composition(inner);
                for arg in args {
                    self.check_expr_composition(&arg.node.value);
                }
            }
            _ => {}
        }
    }

}

// ── Type inference (partial, POC) ────────────────────────────

/// Infer the output type of an expression (partial — returns None for unknowns).
fn infer_type(expr: &Spanned<Expr>) -> Option<ForgeType> {
    match &expr.node {
        Expr::NumberLit(_) => Some(ForgeType::Number),
        Expr::BoolLit(_) => Some(ForgeType::Bool),
        Expr::Template(_) => Some(ForgeType::Text),
        Expr::Reason(_) => Some(ForgeType::Text),
        Expr::Classify(_) => Some(ForgeType::Classification),
        Expr::Search(_) => Some(ForgeType::Results),
        Expr::Compose(parts) => parts.last().and_then(infer_type),
        Expr::ArrayLit(_) => None, // would need element type inference
        _ => None,
    }
}

/// Infer the expected input type for an expression when used as the RHS of `>>`.
fn infer_input_type(expr: &Spanned<Expr>) -> Option<ForgeType> {
    match &expr.node {
        // Most callables accept Text in the POC
        Expr::Call(_) => Some(ForgeType::Text),
        Expr::Reason(_) => Some(ForgeType::Text),
        Expr::Classify(_) => Some(ForgeType::Text),
        Expr::Search(_) => Some(ForgeType::Text),
        _ => None,
    }
}

/// Extract statements from a task body.
fn task_body_stmts(task: &crate::ast::TaskDecl) -> Vec<Spanned<Stmt>> {
    match &task.body.node {
        crate::ast::TaskBody::Do(stmts) => stmts.clone(),
        crate::ast::TaskBody::Is(expr) => {
            vec![Spanned::new(Stmt::ExprStmt((**expr).clone()), task.body.span)]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    #[test]
    fn resolve_valid_capabilities() {
        let source = "use\n  llm.reason\n  llm.classify\n\ntask foo\n  needs x: Text\n  gives Text\n  do\n    give x\n";
        let program = parse(source).unwrap();
        let ctx = CheckContext::new("<test>");
        assert!(ctx.check(&program).is_ok());
    }

    #[test]
    fn reject_unknown_capability() {
        let source = "use\n  llm.reason\n  magic.wand\n\ntask foo\n  needs x: Text\n  gives Text\n  do\n    give x\n";
        let program = parse(source).unwrap();
        let ctx = CheckContext::new("<test>");
        let errs = ctx.check(&program).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(&errs[0], ResolveError::UnknownCapability { name, .. } if name == "magic.wand"));
    }

    #[test]
    fn no_use_block_is_ok() {
        let source = "task greet\n  needs name: Text\n  gives Text\n  do\n    say \"Hello, {name}!\"\n";
        let program = parse(source).unwrap();
        let ctx = CheckContext::new("<test>");
        assert!(ctx.check(&program).is_ok());
    }

    #[test]
    fn pure_rejects_reason() {
        let source = "pure bad\n  needs x: Text\n  gives Text\n  do\n    result = reason x\n    give result\n";
        let program = parse(source).unwrap();
        let ctx = CheckContext::new("<test>");
        let errs = ctx.check(&program).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(&errs[0], ResolveError::PureUsesLlm { name, .. } if name == "bad"));
    }

    #[test]
    fn pure_allows_non_llm() {
        let source = "pure add\n  needs a: Number, b: Number\n  gives Number\n  do\n    give a + b\n";
        let program = parse(source).unwrap();
        let ctx = CheckContext::new("<test>");
        assert!(ctx.check(&program).is_ok());
    }

    #[test]
    fn check_all_builtin_capabilities() {
        let registry = CapabilityRegistry::builtin();
        assert!(registry.resolve("llm.reason").is_some());
        assert!(registry.resolve("llm.classify").is_some());
        assert!(registry.resolve("web.search").is_some());
        assert!(registry.resolve("data.store").is_some());
        assert!(registry.resolve("data.embed").is_some());
        assert!(registry.resolve("magic.wand").is_none());
    }
}
