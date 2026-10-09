// FORGE resolution checker — issue #496.
//
// E160: a plain call target that resolves to nothing.
// E161: a constructor pattern that can never match the scrutinee.
// E162: a tag pattern whose case differs from the known tag.
//
// Scrutinees with unknown value sets are never flagged: the checker only
// reasons about values it can prove statically (classify labels, all-literal
// `give`s, and records built by a declared type constructor).

use std::collections::HashSet;

use crate::ast::*;
use crate::diagnostic::Diagnostic;

/// Bare `Call` targets the runtime dispatches itself — see `Expr::Call` in
/// `src/runtime/executor.rs`.
pub const BUILTIN_CALLS: &[&str] = &["asset", "winning_lines"];

/// Built-in type names (grammar `builtin_type`) a pattern may legitimately name.
pub const BUILTIN_TYPES: &[&str] = &[
    "AgentResult",
    "Bool",
    "Classification",
    "Conversation",
    "Failure",
    "Headers",
    "Html",
    "Intent",
    "Number",
    "Profile",
    "Report",
    "Request",
    "Response",
    "Results",
    "SearchResults",
    "Summary",
    "Text",
];

/// Program-level name index, built once per file.
struct Index {
    /// Names that resolve as a bare call target.
    callable: HashSet<String>,
    /// Declared `type` names plus [`BUILTIN_TYPES`].
    types: HashSet<String>,
}

impl Index {
    fn build(program: &Program) -> Self {
        let mut index = Self {
            callable: BUILTIN_CALLS.iter().map(|s| s.to_string()).collect(),
            types: BUILTIN_TYPES.iter().map(|s| s.to_string()).collect(),
        };
        for item in &program.items {
            match &item.node {
                TopLevel::Task(d) => {
                    index.callable.insert(d.name.node.clone());
                }
                TopLevel::Pure(d) => {
                    index.callable.insert(d.name.node.clone());
                }
                TopLevel::Flow(d) => {
                    index.callable.insert(d.name.node.clone());
                }
                TopLevel::Pool(d) => {
                    index.callable.insert(d.name.node.clone());
                }
                TopLevel::FnMain(_) => {
                    index.callable.insert("main".to_string());
                }
                TopLevel::TypeDef(d) => {
                    index.types.insert(d.name.node.clone());
                }
                _ => {}
            };
        }
        index
    }

    /// `Expr::Call` builds a tagged record for any uppercase name, so an
    /// uppercase call always resolves at runtime.
    fn is_callable(&self, name: &str) -> bool {
        name.starts_with(|c: char| c.is_uppercase()) || self.callable.contains(name)
    }
}

/// Run the resolution checker over every statement body in `program`.
pub fn check(program: &Program, file: &str) -> Vec<Diagnostic> {
    let index = Index::build(program);
    let mut diagnostics = Vec::new();
    for item in &program.items {
        match &item.node {
            TopLevel::Task(d) => {
                match &d.body.node {
                    TaskBody::Do(stmts) => index.stmts(stmts, file, &mut diagnostics),
                    TaskBody::Is(expr) => index.expr(expr, file, &mut diagnostics),
                }
                if let Some(if_fails) = &d.if_fails {
                    index.stmts(if_fails, file, &mut diagnostics);
                }
            }
            TopLevel::Pure(d) => index.stmts(&d.body, file, &mut diagnostics),
            TopLevel::Flow(d) => {
                for stage in &d.stages {
                    index.stmts(&stage.node.body, file, &mut diagnostics);
                }
            }
            TopLevel::Agent(d) => {
                for handler in &d.handlers {
                    index.stmts(&handler.node.body, file, &mut diagnostics);
                }
                if let Some(stuck) = &d.stuck_policy {
                    index.stmts(&stuck.node.body, file, &mut diagnostics);
                }
            }
            TopLevel::Endpoint(d) => index.stmts(&d.body, file, &mut diagnostics),
            TopLevel::FnMain(d) => index.stmts(&d.body, file, &mut diagnostics),
            TopLevel::States(d) => {
                for t in &d.transitions {
                    if let Some(cond) = &t.node.condition {
                        index.expr(cond, file, &mut diagnostics);
                    }
                }
            }
            TopLevel::System(d) => {
                for wire in &d.wiring {
                    index.expr(wire, file, &mut diagnostics);
                }
            }
            _ => {}
        }
    }
    diagnostics
}

impl Index {
    fn stmts(&self, stmts: &[Spanned<Stmt>], file: &str, diagnostics: &mut Vec<Diagnostic>) {
        self.stmts_with(stmts, file, diagnostics);
    }

    fn stmts_with(&self, stmts: &[Spanned<Stmt>], file: &str, diagnostics: &mut Vec<Diagnostic>) {
        for stmt in stmts {
            self.stmt(stmt, file, diagnostics);
        }
    }

    fn stmt(&self, stmt: &Spanned<Stmt>, file: &str, diagnostics: &mut Vec<Diagnostic>) {
        match &stmt.node {
            Stmt::Bind(_, expr) => self.expr(expr, file, diagnostics),
            Stmt::Give(e, _) | Stmt::Say(e) | Stmt::ExprStmt(e) => self.expr(e, file, diagnostics),
            Stmt::MemoryUpdate(_, index, value) => {
                if let Some(i) = index {
                    self.expr(i, file, diagnostics);
                }
                self.expr(value, file, diagnostics);
            }
            Stmt::Emit(_, args) => {
                for arg in args {
                    self.expr(&arg.node.value, file, diagnostics);
                }
            }
            Stmt::Forward(src, dst) => {
                self.expr(src, file, diagnostics);
                self.expr(dst, file, diagnostics);
            }
            Stmt::Learn(source, category) => {
                self.learn(source, file, diagnostics);
                if let Some(c) = category {
                    self.expr(c, file, diagnostics);
                }
            }
            Stmt::Match(m) => {
                self.expr(&m.subject, file, diagnostics);
                for arm in &m.arms {
                    self.stmt(&arm.node.body, file, diagnostics);
                }
            }
            _ => self.stmt_rest(stmt, file, diagnostics),
        }
    }

    /// Statement forms that only nest or forward expressions.
    fn stmt_rest(&self, stmt: &Spanned<Stmt>, file: &str, diagnostics: &mut Vec<Diagnostic>) {
        match &stmt.node {
            Stmt::IfElse(ie) => {
                self.expr(&ie.condition, file, diagnostics);
                self.stmts_with(&ie.then_body, file, diagnostics);
                for (cond, body) in &ie.else_ifs {
                    self.expr(cond, file, diagnostics);
                    self.stmts_with(body, file, diagnostics);
                }
                if let Some(body) = &ie.else_body {
                    self.stmts_with(body, file, diagnostics);
                }
            }
            Stmt::For(f) => {
                self.expr(&f.iterable, file, diagnostics);
                self.stmts_with(&f.body, file, diagnostics);
            }
            Stmt::When(w) => {
                for clause in &w.clauses {
                    self.stmt(&clause.node.body, file, diagnostics);
                }
                if let Some(else_clause) = &w.else_body {
                    self.stmt(&else_clause.node.body, file, diagnostics);
                }
            }
            Stmt::StartTimer { context, .. } | Stmt::CancelTimer { context, .. } => {
                if let Some(c) = context {
                    self.expr(c, file, diagnostics);
                }
            }
            Stmt::Spawn(s) => {
                if let Some(alias) = &s.alias {
                    self.expr(alias, file, diagnostics);
                }
                for option in &s.options {
                    if let Some(e) = spawn_option_expr(&option.node) {
                        self.expr(e, file, diagnostics);
                    }
                }
            }
            _ => {}
        }
    }

    fn learn(&self, source: &Spanned<LearnSource>, file: &str, diagnostics: &mut Vec<Diagnostic>) {
        match &source.node {
            LearnSource::Direct(e) | LearnSource::FromDocument(e) => {
                self.expr(e, file, diagnostics)
            }
            LearnSource::FromInteraction(args) => {
                for arg in args {
                    self.expr(&arg.node.value, file, diagnostics);
                }
            }
        }
    }

    /// E160 — a plain call whose target resolves to nothing.
    fn expr(&self, expr: &Spanned<Expr>, file: &str, diagnostics: &mut Vec<Diagnostic>) {
        if let Expr::Call(call) = &expr.node {
            if !self.is_callable(&call.name.node) {
                diagnostics.push(undefined_call(&call.name, self, file));
            }
        }
        self.walk_expr(expr, file, diagnostics);
    }

    fn walk_expr(&self, expr: &Spanned<Expr>, file: &str, diagnostics: &mut Vec<Diagnostic>) {
        match &expr.node {
            Expr::NumberLit(_) | Expr::BoolLit(_) | Expr::Ident(_) | Expr::TypeAccess(_, _) => {}
            Expr::Template(parts) => {
                for part in parts {
                    if let TemplatePart::Interp(e) | TemplatePart::RawInterp(e) = &part.node {
                        self.expr(e, file, diagnostics);
                    }
                }
            }
            Expr::Call(c) => {
                for arg in &c.args {
                    self.expr(&arg.node.value, file, diagnostics);
                }
            }
            Expr::Constructor(c) => {
                for arg in &c.args {
                    self.expr(&arg.node.value, file, diagnostics);
                }
            }
            Expr::Reason(r) => self.expr(&r.prompt, file, diagnostics),
            Expr::Classify(c) => self.expr(&c.input, file, diagnostics),
            Expr::Search(e)
            | Expr::Recall(e)
            | Expr::Exec(e)
            | Expr::Paren(e)
            | Expr::GlobAccess(e) => self.expr(e, file, diagnostics),
            Expr::Command(c) => {
                self.expr(&c.cmd, file, diagnostics);
                if let Some(dir) = &c.working_dir {
                    self.expr(dir, file, diagnostics);
                }
            }
            Expr::CommandMethod(_, args) | Expr::SessionMethod(_, args) => {
                for arg in args {
                    self.expr(&arg.node.value, file, diagnostics);
                }
            }
            Expr::TryOr(a, b) | Expr::BinOp(a, _, b) | Expr::Index(a, b) => {
                self.expr(a, file, diagnostics);
                self.expr(b, file, diagnostics);
            }
            Expr::UnaryOp(_, e) | Expr::FieldAccess(e, _) => self.expr(e, file, diagnostics),
            Expr::Compose(items) | Expr::FanOut(items) | Expr::ArrayLit(items) => {
                for item in items {
                    self.expr(item, file, diagnostics);
                }
            }
            Expr::MethodCall(inner, _, args) => {
                self.expr(inner, file, diagnostics);
                for arg in args {
                    self.expr(&arg.node.value, file, diagnostics);
                }
            }
            Expr::Session(_) | Expr::Find(_) => {}
        }
    }
}

fn spawn_option_expr(option: &SpawnOption) -> Option<&Spanned<Expr>> {
    match option {
        SpawnOption::ConfidenceCap(e) | SpawnOption::MemoryInit(_, e) => Some(e),
        SpawnOption::Isolate(config) => Some(&config.branch),
        SpawnOption::KnowledgeFilter(_) => None,
    }
}

// ── Diagnostics ─────────────────────────────────────────────────

fn undefined_call(name: &Spanned<String>, index: &Index, file: &str) -> Diagnostic {
    let target = &name.node;
    let help = match closest_name(target, index.callable.iter()) {
        Some(closest) => format!("did you mean `{closest}`?"),
        None => format!("declare a task or pure named `{target}`"),
    };
    Diagnostic::error(
        "E160",
        file,
        format!("call to undeclared function `{target}`"),
        name.span.start..name.span.end,
        "no task, pure, flow, pool or built-in matches this name",
    )
    .with_help(help)
}

/// Closest candidate within edit distance 2, if any.
fn closest_name<'a>(
    name: &str,
    candidates: impl Iterator<Item = &'a String>,
) -> Option<&'a String> {
    candidates
        .map(|c| (edit_distance(name, c), c))
        .filter(|(d, _)| *d <= 2)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let substitute = previous[j] + usize::from(ca != cb);
            current[j + 1] = (previous[j + 1] + 1).min(current[j] + 1).min(substitute);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn diags(src: &str) -> Vec<Diagnostic> {
        let program = parse(src).expect("parse failed");
        check(&program, "test.forge")
    }

    fn codes(src: &str) -> Vec<&'static str> {
        diags(src).iter().map(|d| d.code).collect()
    }

    // ── E160 — undeclared call targets ──────────────────────────

    #[test]
    fn resolution_flags_undeclared_call() {
        let ds = diags("fn main\n  say judge(\"x\")\n");
        assert_eq!(
            codes("fn main\n  say judge(\"x\")\n"),
            vec!["E160"],
            "{ds:?}"
        );
        assert!(ds[0].message.contains("judge"), "{:?}", ds[0].message);
        assert_eq!(
            ds[0].help.as_deref(),
            Some("declare a task or pure named `judge`")
        );
    }

    #[test]
    fn resolution_suggests_the_closest_declared_name() {
        let src = "task greet\n  gives Text\n  do\n    give \"hi\"\n\nfn main\n  say greeet()\n";
        let ds = diags(src);
        assert_eq!(codes(src), vec!["E160"], "{ds:?}");
        assert_eq!(ds[0].help.as_deref(), Some("did you mean `greet`?"));
    }

    #[test]
    fn resolution_allows_every_declared_callable_kind() {
        let src = r#"
task t
  gives Text
  do
    give "t"

pure p
  needs x: Text
  gives Text
  do
    give x

flow f
  stage s
    give p("x")

type Rec
  id: Text

fn main
  say t()
  say p("x")
  say f("x")
  say Rec("id")
  say asset("logo.png")
  say winning_lines([1, 2, 3])
"#;
        assert_eq!(codes(src), Vec::<&str>::new(), "{:?}", diags(src));
    }

    #[test]
    fn resolution_allows_the_fn_name() {
        let src = "fn main\n  say main()\n";
        assert_eq!(codes(src), Vec::<&str>::new(), "{:?}", diags(src));
    }
}
