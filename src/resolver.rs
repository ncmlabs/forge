// FORGE capability resolver
// See issue #6 for full implementation

use std::collections::HashMap;

use crate::ast::{Expr, Program, Spanned, Stmt, TopLevel};
use crate::diagnostic::Diagnostic;
use crate::types::{from_type_name, is_compatible, CapabilitySignature, ForgeType};

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

    #[error("capability `{name}` expects {expected} arguments but got {actual}")]
    ArgumentCountMismatch {
        name: String,
        expected: usize,
        actual: usize,
        span_start: usize,
        span_end: usize,
    },

    #[error("argument type mismatch for `{name}`: expected `{expected}`, got `{actual}`")]
    ArgumentTypeMismatch {
        name: String,
        expected: String,
        actual: String,
        span_start: usize,
        span_end: usize,
    },
}

impl ResolveError {
    pub fn to_diagnostic(&self, file: &str, registry: &CapabilityRegistry) -> Diagnostic {
        match self {
            ResolveError::UnknownCapability {
                name,
                span_start,
                span_end,
            } => {
                let namespace = name.split('.').next().unwrap_or("");
                let available: Vec<&str> = registry
                    .capabilities
                    .keys()
                    .filter(|k| k.starts_with(namespace))
                    .map(|k| k.as_str())
                    .collect();
                let help = if available.is_empty() {
                    None
                } else {
                    Some(format!(
                        "available {} capabilities: {}",
                        namespace,
                        available.join(", ")
                    ))
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
            ResolveError::CompositionMismatch {
                left,
                right,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!("composition type mismatch: `{}` → `{}`", left, right),
                *span_start..*span_end,
                format!("`{}` is not compatible with `{}`", left, right),
            ),
            ResolveError::ArgumentCountMismatch {
                name,
                expected,
                actual,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!(
                    "capability '{}' expects {} arguments but got {}",
                    name, expected, actual
                ),
                *span_start..*span_end,
                "adjust the skill call to match the declared signature",
            ),
            ResolveError::ArgumentTypeMismatch {
                name,
                expected,
                actual,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!(
                    "argument type mismatch for '{}': expected `{}`, got `{}`",
                    name, expected, actual
                ),
                *span_start..*span_end,
                "change the argument or update the skill capability signature",
            ),
        }
    }
}

// ── Capability registry ──────────────────────────────────────

pub struct CapabilityRegistry {
    capabilities: HashMap<String, CapabilitySignature>,
}

impl CapabilityRegistry {
    /// Create a registry with builtins + external skill capabilities.
    pub fn with_skills(skill_signatures: HashMap<String, CapabilitySignature>) -> Self {
        let mut registry = Self::builtin();
        for (name, sig) in skill_signatures {
            registry.capabilities.insert(name, sig);
        }
        registry
    }

    /// Create a registry with all built-in capabilities.
    pub fn builtin() -> Self {
        let mut caps = HashMap::new();

        caps.insert(
            "llm.reason".into(),
            CapabilitySignature {
                inputs: vec![ForgeType::Text],
                output: ForgeType::Text,
            },
        );
        caps.insert(
            "llm.classify".into(),
            CapabilitySignature {
                inputs: vec![ForgeType::Text],
                output: ForgeType::Classification,
            },
        );
        caps.insert(
            "web.search".into(),
            CapabilitySignature {
                inputs: vec![ForgeType::Text],
                output: ForgeType::Results,
            },
        );
        caps.insert(
            "web.fetch".into(),
            CapabilitySignature {
                inputs: vec![ForgeType::Text],
                output: ForgeType::Text,
            },
        );
        caps.insert(
            "web.post".into(),
            CapabilitySignature {
                inputs: vec![ForgeType::Text, ForgeType::Text],
                output: ForgeType::Text,
            },
        );
        // env.get(name, default) -> Text — read an environment variable at runtime,
        // with fallback. Allowed in all boundaries (server, client, shared).
        // See issue #251 (part of epic #249).
        caps.insert(
            "env.get".into(),
            CapabilitySignature {
                inputs: vec![ForgeType::Text, ForgeType::Text],
                output: ForgeType::Text,
            },
        );
        caps.insert(
            "data.store".into(),
            CapabilitySignature {
                inputs: vec![ForgeType::Text, ForgeType::Text],
                output: ForgeType::Unit,
            },
        );
        caps.insert(
            "data.get".into(),
            CapabilitySignature {
                inputs: vec![ForgeType::Text],
                output: ForgeType::Text,
            },
        );
        caps.insert(
            "data.list".into(),
            CapabilitySignature {
                inputs: vec![ForgeType::Text],
                output: ForgeType::Text,
            },
        );
        caps.insert(
            "data.delete".into(),
            CapabilitySignature {
                inputs: vec![ForgeType::Text],
                output: ForgeType::Unit,
            },
        );
        caps.insert(
            "data.embed".into(),
            CapabilitySignature {
                inputs: vec![ForgeType::Text],
                output: ForgeType::Embedding,
            },
        );
        caps.insert(
            "data.search".into(),
            CapabilitySignature {
                inputs: vec![ForgeType::Text],
                output: ForgeType::Results,
            },
        );

        caps.insert(
            "html.layout".into(),
            CapabilitySignature {
                inputs: vec![ForgeType::Text, ForgeType::Html],
                output: ForgeType::Html,
            },
        );
        caps.insert(
            "html.escape".into(),
            CapabilitySignature {
                inputs: vec![ForgeType::Text],
                output: ForgeType::Text,
            },
        );
        caps.insert(
            "markdown.render".into(),
            CapabilitySignature {
                inputs: vec![ForgeType::Text],
                output: ForgeType::Html,
            },
        );
        caps.insert(
            "asset".into(),
            CapabilitySignature {
                inputs: vec![ForgeType::Text],
                output: ForgeType::Text,
            },
        );

        // command.status / command.output / command.cancel — issue #162
        caps.insert(
            "command.status".into(),
            CapabilitySignature {
                inputs: vec![ForgeType::Text],
                output: ForgeType::Text,
            },
        );
        caps.insert(
            "command.output".into(),
            CapabilitySignature {
                inputs: vec![ForgeType::Text],
                output: ForgeType::Text,
            },
        );
        caps.insert(
            "command.cancel".into(),
            CapabilitySignature {
                inputs: vec![ForgeType::Text],
                output: ForgeType::Unit,
            },
        );

        Self { capabilities: caps }
    }

    pub fn resolve(&self, name: &str) -> Option<&CapabilitySignature> {
        self.capabilities.get(name)
    }

    /// Check if `name` is a namespace prefix for one or more capabilities.
    /// e.g. "skill.github" matches "skill.github.create_issue", "skill.github.list_issues", etc.
    pub fn has_namespace(&self, name: &str) -> bool {
        let prefix = format!("{}.", name);
        self.capabilities.keys().any(|k| k.starts_with(&prefix))
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

    /// Create a check context with external skill capabilities registered.
    pub fn with_skills(
        _file: &str,
        skill_signatures: HashMap<String, CapabilitySignature>,
    ) -> Self {
        Self {
            registry: CapabilityRegistry::with_skills(skill_signatures),
            errors: Vec::new(),
        }
    }

    /// Run all checks on the program. Returns Ok(()) if no errors, or Err with all collected errors.
    pub fn check(mut self, program: &Program) -> Result<(), Vec<ResolveError>> {
        // Pass 1: resolve use declarations
        for item in &program.items {
            if let TopLevel::Use(use_decl) = &item.node {
                for cap in &use_decl.capabilities {
                    if self.registry.resolve(&cap.node).is_none()
                        && !self.registry.has_namespace(&cap.node)
                    {
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
                    let mut env = params_env(&task.needs);
                    self.check_stmts_composition(&task_body_stmts(task), &mut env);
                }
                TopLevel::Pure(pure) => {
                    let mut env = params_env(&pure.needs);
                    self.check_stmts_composition(&pure.body, &mut env);
                }
                TopLevel::Flow(flow) => {
                    let base_env = params_env(&flow.needs);
                    for stage in &flow.stages {
                        let mut env = base_env.clone();
                        self.check_stmts_composition(&stage.node.body, &mut env);
                    }
                }
                TopLevel::Agent(agent) => {
                    for handler in &agent.handlers {
                        let mut env = HashMap::new();
                        self.check_stmts_composition(&handler.node.body, &mut env);
                    }
                }
                TopLevel::Endpoint(ep) => {
                    let mut env = params_env(&ep.params);
                    self.check_stmts_composition(&ep.body, &mut env);
                }
                TopLevel::FnMain(main) => {
                    let mut env = HashMap::new();
                    self.check_stmts_composition(&main.body, &mut env);
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

    /// Walk statements looking for Compose expressions and check type compatibility.
    fn check_stmts_composition(
        &mut self,
        stmts: &[Spanned<Stmt>],
        env: &mut HashMap<String, ForgeType>,
    ) {
        for stmt in stmts {
            self.check_stmt_composition(stmt, env);
        }
    }

    fn check_stmt_composition(
        &mut self,
        stmt: &Spanned<Stmt>,
        env: &mut HashMap<String, ForgeType>,
    ) {
        match &stmt.node {
            Stmt::Bind(name, expr) => {
                self.check_expr_composition(expr, env);
                if let Some(ty) = infer_type(expr, env, &self.registry) {
                    env.insert(name.node.clone(), ty);
                }
            }
            Stmt::Say(expr) | Stmt::ExprStmt(expr) => {
                self.check_expr_composition(expr, env);
            }
            Stmt::Give(expr, metas) => {
                self.check_expr_composition(expr, env);
                for meta in metas {
                    self.check_expr_composition(&meta.node.value, env);
                }
            }
            Stmt::When(when) => {
                for clause in &when.clauses {
                    let mut branch_env = env.clone();
                    self.check_stmt_composition(&clause.node.body, &mut branch_env);
                }
                if let Some(else_clause) = &when.else_body {
                    let mut branch_env = env.clone();
                    self.check_stmt_composition(&else_clause.node.body, &mut branch_env);
                }
            }
            Stmt::Match(m) => {
                self.check_expr_composition(&m.subject, env);
                for arm in &m.arms {
                    let mut branch_env = env.clone();
                    self.check_stmt_composition(&arm.node.body, &mut branch_env);
                }
            }
            Stmt::IfElse(ie) => {
                self.check_expr_composition(&ie.condition, env);
                let mut then_env = env.clone();
                self.check_stmts_composition(&ie.then_body, &mut then_env);
                for (cond, body) in &ie.else_ifs {
                    self.check_expr_composition(cond, env);
                    let mut branch_env = env.clone();
                    self.check_stmts_composition(body, &mut branch_env);
                }
                if let Some(body) = &ie.else_body {
                    let mut else_env = env.clone();
                    self.check_stmts_composition(body, &mut else_env);
                }
            }
            Stmt::For(f) => {
                self.check_expr_composition(&f.iterable, env);
                let mut loop_env = env.clone();
                self.check_stmts_composition(&f.body, &mut loop_env);
            }
            _ => {}
        }
    }

    fn check_expr_composition(&mut self, expr: &Spanned<Expr>, env: &HashMap<String, ForgeType>) {
        match &expr.node {
            Expr::Compose(parts) => {
                // Check adjacent pairs for type compatibility
                for window in parts.windows(2) {
                    let left = &window[0];
                    let right = &window[1];
                    if let (Some(lt), Some(rt)) = (
                        infer_type(left, env, &self.registry),
                        infer_input_type(right, env, &self.registry),
                    ) {
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
                    self.check_expr_composition(p, env);
                }
            }
            Expr::TryOr(a, b) => {
                self.check_expr_composition(a, env);
                self.check_expr_composition(b, env);
            }
            Expr::FanOut(parts) => {
                for p in parts {
                    self.check_expr_composition(p, env);
                }
            }
            Expr::BinOp(a, _, b) => {
                self.check_expr_composition(a, env);
                self.check_expr_composition(b, env);
            }
            Expr::UnaryOp(_, a) => {
                self.check_expr_composition(a, env);
            }
            Expr::Call(c) => {
                for arg in &c.args {
                    self.check_expr_composition(&arg.node.value, env);
                }
            }
            Expr::Paren(inner) | Expr::FieldAccess(inner, _) | Expr::GlobAccess(inner) => {
                self.check_expr_composition(inner, env);
            }
            Expr::Index(a, b) => {
                self.check_expr_composition(a, env);
                self.check_expr_composition(b, env);
            }
            Expr::MethodCall(inner, method, args) => {
                self.check_skill_call(inner, method, args, env);
                self.check_expr_composition(inner, env);
                for arg in args {
                    self.check_expr_composition(&arg.node.value, env);
                }
            }
            _ => {}
        }
    }

    fn check_skill_call(
        &mut self,
        inner: &Spanned<Expr>,
        method: &Spanned<String>,
        args: &[Spanned<crate::ast::CallArg>],
        env: &HashMap<String, ForgeType>,
    ) {
        let Expr::FieldAccess(target, namespace) = &inner.node else {
            return;
        };
        let Expr::Ident(prefix) = &target.node else {
            return;
        };
        if prefix != "skill" {
            return;
        }

        let full_name = format!("skill.{}.{}", namespace.node, method.node);
        if let Some(sig) = self.registry.resolve(&full_name) {
            if sig.inputs.len() != args.len() {
                self.errors.push(ResolveError::ArgumentCountMismatch {
                    name: full_name,
                    expected: sig.inputs.len(),
                    actual: args.len(),
                    span_start: method.span.start,
                    span_end: method.span.end,
                });
                return;
            }

            for (arg, expected) in args.iter().zip(&sig.inputs) {
                if let Some(actual) = infer_type(&arg.node.value, env, &self.registry) {
                    if !is_compatible(&actual, expected) {
                        self.errors.push(ResolveError::ArgumentTypeMismatch {
                            name: full_name.clone(),
                            expected: expected.to_string(),
                            actual: actual.to_string(),
                            span_start: arg.span.start,
                            span_end: arg.span.end,
                        });
                    }
                }
            }
            return;
        }

        let legacy_name = format!("skill.{}", namespace.node);
        if self.registry.resolve(&legacy_name).is_none() {
            self.errors.push(ResolveError::UnknownCapability {
                name: full_name,
                span_start: method.span.start,
                span_end: method.span.end,
            });
        }
    }
}

// ── Type inference (partial, POC) ────────────────────────────

/// Infer the output type of an expression (partial — returns None for unknowns).
fn infer_type(
    expr: &Spanned<Expr>,
    env: &HashMap<String, ForgeType>,
    registry: &CapabilityRegistry,
) -> Option<ForgeType> {
    match &expr.node {
        Expr::NumberLit(_) => Some(ForgeType::Number),
        Expr::BoolLit(_) => Some(ForgeType::Bool),
        Expr::Ident(name) => env.get(name).cloned(),
        Expr::Template(_) => Some(ForgeType::Text),
        Expr::Reason(_) => Some(ForgeType::Text),
        Expr::Exec(_) => Some(ForgeType::Text),
        Expr::Command(_) | Expr::CommandMethod(_, _) | Expr::SessionMethod(_, _) => {
            Some(ForgeType::Text)
        }
        Expr::Session(session) => {
            if session.gives.is_some() {
                Some(ForgeType::AgentResult)
            } else {
                Some(ForgeType::Text)
            }
        }
        Expr::Classify(_) => Some(ForgeType::Classification),
        Expr::Search(_) => Some(ForgeType::Results),
        Expr::Paren(inner) => infer_type(inner, env, registry),
        Expr::FieldAccess(inner, field) => {
            if matches!(&inner.node, Expr::Ident(prefix) if prefix == "skill") {
                return registry
                    .resolve(&format!("skill.{}", field.node))
                    .map(|sig| sig.output.clone());
            }
            None
        }
        Expr::MethodCall(inner, method, _) => {
            if let Expr::FieldAccess(target, namespace) = &inner.node {
                if matches!(&target.node, Expr::Ident(prefix) if prefix == "skill") {
                    return registry
                        .resolve(&format!("skill.{}.{}", namespace.node, method.node))
                        .map(|sig| sig.output.clone())
                        .or_else(|| {
                            registry
                                .resolve(&format!("skill.{}", namespace.node))
                                .map(|sig| sig.output.clone())
                        });
                }
            }
            None
        }
        Expr::Compose(parts) => parts
            .last()
            .and_then(|expr| infer_type(expr, env, registry)),
        Expr::Constructor(ctor) => Some(crate::types::from_type_name(&ctor.type_name.node)),
        Expr::ArrayLit(_) => None, // would need element type inference
        _ => None,
    }
}

/// Infer the expected input type for an expression when used as the RHS of `>>`.
fn infer_input_type(
    expr: &Spanned<Expr>,
    _env: &HashMap<String, ForgeType>,
    registry: &CapabilityRegistry,
) -> Option<ForgeType> {
    match &expr.node {
        // Most callables accept Text in the POC
        Expr::Call(_) => Some(ForgeType::Text),
        Expr::Reason(_) => Some(ForgeType::Text),
        Expr::Exec(_) => Some(ForgeType::Text),
        Expr::Command(_) | Expr::CommandMethod(_, _) | Expr::SessionMethod(_, _) => {
            Some(ForgeType::Text)
        }
        Expr::Session(_) => Some(ForgeType::Text),
        Expr::Classify(_) => Some(ForgeType::Text),
        Expr::Search(_) => Some(ForgeType::Text),
        Expr::MethodCall(inner, method, _) => {
            if let Expr::FieldAccess(target, namespace) = &inner.node {
                if matches!(&target.node, Expr::Ident(prefix) if prefix == "skill") {
                    return registry
                        .resolve(&format!("skill.{}.{}", namespace.node, method.node))
                        .and_then(|sig| sig.inputs.first().cloned())
                        .or_else(|| {
                            registry
                                .resolve(&format!("skill.{}", namespace.node))
                                .and_then(|sig| sig.inputs.first().cloned())
                        });
                }
            }
            None
        }
        _ => None,
    }
}

fn params_env(params: &[Spanned<crate::ast::Param>]) -> HashMap<String, ForgeType> {
    params
        .iter()
        .map(|param| {
            (
                param.node.name.clone(),
                from_type_name(&param.node.type_name.node),
            )
        })
        .collect()
}

/// Extract statements from a task body.
fn task_body_stmts(task: &crate::ast::TaskDecl) -> Vec<Spanned<Stmt>> {
    match &task.body.node {
        crate::ast::TaskBody::Do(stmts) => stmts.clone(),
        crate::ast::TaskBody::Is(expr) => {
            vec![Spanned::new(
                Stmt::ExprStmt((**expr).clone()),
                task.body.span,
            )]
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
        assert!(
            matches!(&errs[0], ResolveError::UnknownCapability { name, .. } if name == "magic.wand")
        );
    }

    #[test]
    fn no_use_block_is_ok() {
        let source =
            "task greet\n  needs name: Text\n  gives Text\n  do\n    say \"Hello, {name}!\"\n";
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
        assert!(registry.resolve("data.get").is_some());
        assert!(registry.resolve("data.list").is_some());
        assert!(registry.resolve("data.delete").is_some());
        assert!(registry.resolve("data.embed").is_some());
        assert!(registry.resolve("data.search").is_some());
        assert!(registry.resolve("magic.wand").is_none());
    }

    #[test]
    fn infer_session_type_defaults_to_text() {
        let program = parse("task t\n  gives Text\n  do\n    result = session \"review\" prompt \"check\"\n    give result\n").unwrap();
        let expr = match &program.items[0].node {
            TopLevel::Task(task) => match &task.body.node {
                crate::ast::TaskBody::Do(stmts) => match &stmts[0].node {
                    Stmt::Bind(_, expr) => expr,
                    other => panic!("expected bind, got {:?}", other),
                },
                _ => panic!("expected do block"),
            },
            _ => panic!("expected task"),
        };
        let ty = infer_type(expr, &HashMap::new(), &CapabilityRegistry::builtin());
        assert_eq!(ty, Some(ForgeType::Text));
    }

    #[test]
    fn infer_session_type_with_gives_is_agent_result() {
        let program = parse("task t\n  gives Text\n  do\n    result = session \"review\" prompt \"check\" gives AgentResult\n    give result\n").unwrap();
        let expr = match &program.items[0].node {
            TopLevel::Task(task) => match &task.body.node {
                crate::ast::TaskBody::Do(stmts) => match &stmts[0].node {
                    Stmt::Bind(_, expr) => expr,
                    other => panic!("expected bind, got {:?}", other),
                },
                _ => panic!("expected do block"),
            },
            _ => panic!("expected task"),
        };
        let ty = infer_type(expr, &HashMap::new(), &CapabilityRegistry::builtin());
        assert_eq!(ty, Some(ForgeType::AgentResult));
    }
}
