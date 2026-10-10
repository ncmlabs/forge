// FORGE resolution checker — issue #496.
//
// E160: a plain call target that resolves to nothing.
// E161: a constructor pattern that can never match the scrutinee.
// E162: a tag pattern whose case differs from the known tag.
//
// Scrutinees with unknown value sets are never flagged: the checker only
// reasons about values it can prove statically (classify labels, all-literal
// `give`s, and records built by a declared type constructor).

use std::collections::{HashMap, HashSet};

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

/// Statically known values a scrutinee can hold.
#[derive(Clone)]
enum Values {
    /// Literal text values: `classify ... into [...]` labels, or the `give`s of
    /// a pure/task whose every `give` is a Text literal.
    Literals(Vec<String>),
    /// `_type` tag of a record built by a call to a declared type name.
    Record(String),
}

/// Program-level name index, built once per file.
struct Index {
    /// Names that resolve as a bare call target. Declared names only — the
    /// builtins in [`BUILTIN_CALLS`] are kept separate so a suggestion can
    /// prefer a declaration over a builtin at the same edit distance.
    callable: HashSet<String>,
    /// Declared `type` names plus [`BUILTIN_TYPES`].
    types: HashSet<String>,
    /// Pure/task names whose every `give` is a Text literal → those literals.
    literal_gives: HashMap<String, Vec<String>>,
    /// The file is one source of a multi-file project (it imports a package or
    /// carries a boundary directive), so an unresolved name may be declared in
    /// a sibling source the checker cannot see.
    composed: bool,
}

impl Index {
    fn build(program: &Program) -> Self {
        let mut index = Self {
            callable: HashSet::new(),
            types: BUILTIN_TYPES.iter().map(|s| s.to_string()).collect(),
            literal_gives: HashMap::new(),
            composed: program.boundary.is_some()
                || program
                    .items
                    .iter()
                    .any(|i| matches!(i.node, TopLevel::Import(_))),
        };
        for item in &program.items {
            match &item.node {
                TopLevel::Task(d) => {
                    index.callable.insert(d.name.node.clone());
                    if let Some(vals) = task_literal_gives(d) {
                        index.literal_gives.insert(d.name.node.clone(), vals);
                    }
                }
                TopLevel::Pure(d) => {
                    index.callable.insert(d.name.node.clone());
                    if let Some(vals) = literal_gives_in(&d.body) {
                        index.literal_gives.insert(d.name.node.clone(), vals);
                    }
                }
                TopLevel::Flow(d) => {
                    index.callable.insert(d.name.node.clone());
                }
                TopLevel::Pool(d) => {
                    index.callable.insert(d.name.node.clone());
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
        name.starts_with(|c: char| c.is_uppercase())
            || self.callable.contains(name)
            || BUILTIN_CALLS.contains(&name)
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
        self.stmts_with(stmts, &mut HashMap::new(), file, diagnostics);
    }

    fn stmts_with(
        &self,
        stmts: &[Spanned<Stmt>],
        bindings: &mut HashMap<String, Values>,
        file: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        for stmt in stmts {
            self.stmt(stmt, bindings, file, diagnostics);
        }
    }

    fn stmt(
        &self,
        stmt: &Spanned<Stmt>,
        bindings: &mut HashMap<String, Values>,
        file: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        match &stmt.node {
            Stmt::Bind(name, expr) => {
                match bind_value(expr, self) {
                    Some(values) => bindings.insert(name.node.clone(), values),
                    None => bindings.remove(&name.node),
                };
                self.expr(expr, file, diagnostics);
            }
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
                let values = scrutinee_values(&m.subject, bindings);
                for arm in &m.arms {
                    self.pattern(
                        &arm.node.pattern,
                        values.as_ref(),
                        &m.subject,
                        file,
                        diagnostics,
                    );
                    let mut inner = bindings.clone();
                    self.stmt(&arm.node.body, &mut inner, file, diagnostics);
                }
            }
            _ => self.stmt_rest(stmt, bindings, file, diagnostics),
        }
    }

    /// Statement forms that only nest or forward expressions.
    fn stmt_rest(
        &self,
        stmt: &Spanned<Stmt>,
        bindings: &HashMap<String, Values>,
        file: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        match &stmt.node {
            Stmt::IfElse(ie) => {
                self.expr(&ie.condition, file, diagnostics);
                let mut then_b = bindings.clone();
                self.stmts_with(&ie.then_body, &mut then_b, file, diagnostics);
                for (cond, body) in &ie.else_ifs {
                    self.expr(cond, file, diagnostics);
                    let mut b = bindings.clone();
                    self.stmts_with(body, &mut b, file, diagnostics);
                }
                if let Some(body) = &ie.else_body {
                    let mut b = bindings.clone();
                    self.stmts_with(body, &mut b, file, diagnostics);
                }
            }
            Stmt::For(f) => {
                self.expr(&f.iterable, file, diagnostics);
                let mut body_b = bindings.clone();
                body_b.remove(&f.binding.node);
                self.stmts_with(&f.body, &mut body_b, file, diagnostics);
            }
            Stmt::When(w) => {
                for clause in &w.clauses {
                    let mut body_b = bindings.clone();
                    self.stmts_with(
                        std::slice::from_ref(&clause.node.body),
                        &mut body_b,
                        file,
                        diagnostics,
                    );
                }
                if let Some(else_clause) = &w.else_body {
                    let mut body_b = bindings.clone();
                    self.stmts_with(
                        std::slice::from_ref(&else_clause.node.body),
                        &mut body_b,
                        file,
                        diagnostics,
                    );
                }
            }
            Stmt::StartTimer { context, .. } | Stmt::CancelTimer { context, .. } => {
                if let Some(c) = context {
                    self.expr(c, file, diagnostics);
                }
            }
            Stmt::Retire(r) => {
                if let Some(target) = &r.target {
                    self.expr(target, file, diagnostics);
                }
                if let Some(path) = &r.knowledge_export {
                    self.expr(path, file, diagnostics);
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
            Expr::Session(s) => {
                self.expr(&s.name, file, diagnostics);
                for e in [&s.agent, &s.prompt, &s.tools, &s.budget]
                    .into_iter()
                    .flatten()
                {
                    self.expr(e, file, diagnostics);
                }
                for hook in [&s.on_progress, &s.on_complete].into_iter().flatten() {
                    for arg in &hook.node.args {
                        self.expr(&arg.node.value, file, diagnostics);
                    }
                }
                if let Some(isolate) = &s.isolate {
                    self.expr(&isolate.branch, file, diagnostics);
                }
            }
            Expr::Find(f) => {
                // `find "spec_{topic}"` — the alias is a template, so any call
                // inside its interpolations still has to resolve.
                if let FindKind::ByAlias(alias) = &f.kind {
                    self.expr(alias, file, diagnostics);
                }
            }
        }
    }

    /// E161/E162 — a constructor pattern checked against a known value set.
    fn pattern(
        &self,
        pattern: &Spanned<Pattern>,
        values: Option<&Values>,
        subject: &Spanned<Expr>,
        file: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let (Some(values), Pattern::Constructor(name, _)) = (values, &pattern.node) else {
            return;
        };
        match values {
            Values::Literals(literals) => {
                if literals.iter().any(|l| l == name) {
                    return;
                }
                if let Some(exact) = literals.iter().find(|l| l.eq_ignore_ascii_case(name)) {
                    diagnostics.push(case_mismatch(name, exact, pattern, file));
                } else if !self.types.contains(name) {
                    diagnostics.push(impossible_pattern(name, subject, values, pattern, file));
                }
            }
            Values::Record(type_name) => {
                if type_name != name && !self.types.contains(name) {
                    diagnostics.push(impossible_pattern(name, subject, values, pattern, file));
                }
            }
        }
    }
}

/// A binding's statically known value set, if any.
fn bind_value(expr: &Spanned<Expr>, index: &Index) -> Option<Values> {
    match &expr.node {
        Expr::Classify(c) => Some(Values::Literals(
            c.labels.iter().map(|l| l.node.clone()).collect(),
        )),
        Expr::Call(c) if index.literal_gives.contains_key(&c.name.node) => {
            Some(Values::Literals(index.literal_gives[&c.name.node].clone()))
        }
        Expr::Call(c) if index.types.contains(&c.name.node) => {
            Some(Values::Record(c.name.node.clone()))
        }
        _ => None,
    }
}

/// The value set of a scrutinee — only identifiers bound in the same body.
fn scrutinee_values(expr: &Spanned<Expr>, bindings: &HashMap<String, Values>) -> Option<Values> {
    match &expr.node {
        Expr::Ident(name) => bindings.get(name).cloned(),
        _ => None,
    }
}

fn spawn_option_expr(option: &SpawnOption) -> Option<&Spanned<Expr>> {
    match option {
        SpawnOption::ConfidenceCap(e) | SpawnOption::MemoryInit(_, e) => Some(e),
        SpawnOption::Isolate(config) => Some(&config.branch),
        SpawnOption::KnowledgeFilter(_) => None,
    }
}

/// Literal `give` values of a task, or `None` if any `give` is not a Text
/// literal (in which case the task's value set is not statically known).
fn task_literal_gives(decl: &TaskDecl) -> Option<Vec<String>> {
    let mut out = match &decl.body.node {
        TaskBody::Do(stmts) => literal_gives_in(stmts)?,
        TaskBody::Is(_) => return None,
    };
    if let Some(if_fails) = &decl.if_fails {
        out.extend(literal_gives_in(if_fails)?);
    }
    (!out.is_empty()).then_some(out)
}

fn literal_gives_in(stmts: &[Spanned<Stmt>]) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for stmt in stmts {
        out.extend(literal_gives_stmt(stmt)?);
    }
    Some(out)
}

fn literal_gives_stmt(stmt: &Spanned<Stmt>) -> Option<Vec<String>> {
    match &stmt.node {
        Stmt::Give(e, _) => literal_text(e).map(|text| vec![text]),
        Stmt::When(w) => {
            let mut out = Vec::new();
            for clause in &w.clauses {
                out.extend(literal_gives_stmt(&clause.node.body)?);
            }
            if let Some(else_clause) = &w.else_body {
                out.extend(literal_gives_stmt(&else_clause.node.body)?);
            }
            Some(out)
        }
        Stmt::IfElse(ie) => {
            let mut out = literal_gives_in(&ie.then_body)?;
            for (_, body) in &ie.else_ifs {
                out.extend(literal_gives_in(body)?);
            }
            if let Some(body) = &ie.else_body {
                out.extend(literal_gives_in(body)?);
            }
            Some(out)
        }
        Stmt::Match(m) => {
            let mut out = Vec::new();
            for arm in &m.arms {
                out.extend(literal_gives_stmt(&arm.node.body)?);
            }
            Some(out)
        }
        Stmt::For(f) => literal_gives_in(&f.body),
        _ => Some(Vec::new()),
    }
}

/// A template with no interpolation is a compile-time Text literal.
fn literal_text(expr: &Spanned<Expr>) -> Option<String> {
    let Expr::Template(parts) = &expr.node else {
        return None;
    };
    let mut text = String::new();
    for part in parts {
        match &part.node {
            TemplatePart::Text(t) => text.push_str(t),
            TemplatePart::Interp(_) | TemplatePart::RawInterp(_) => return None,
        }
    }
    Some(text)
}

// ── Diagnostics ─────────────────────────────────────────────────

/// How to check a call target that may live in a sibling source of the same
/// project: a bare `forge check` only sees the files it is handed.
const PROJECT_HINT: &str = "if it is declared in another file of this project, check the files together: `forge check --merge <files>` or `forge check --manifest forge.project.toml`";

fn undefined_call(name: &Spanned<String>, index: &Index, file: &str) -> Diagnostic {
    let target = &name.node;
    let closest = closest_name(target, index.callable.iter());
    let mut help = match &closest {
        Some(closest) => format!("did you mean `{closest}`?"),
        None => format!("declare a task or pure named `{target}`"),
    };
    if index.composed || closest.is_none() {
        help.push(' ');
        help.push_str(PROJECT_HINT);
    }
    Diagnostic::error(
        "E160",
        file,
        format!("call to undeclared function `{target}`"),
        name.span.start..name.span.end,
        "no task, pure, flow, pool or built-in matches this name",
    )
    .with_help(help)
}

fn impossible_pattern(
    name: &str,
    subject: &Spanned<Expr>,
    values: &Values,
    pattern: &Spanned<Pattern>,
    file: &str,
) -> Diagnostic {
    let described = describe(values);
    Diagnostic::error(
        "E161",
        file,
        format!(
            "pattern `{name}` never matches {} — it can only be {described}",
            subject_label(subject)
        ),
        pattern.span.start..pattern.span.end,
        "this arm is dead code and falls through",
    )
    .with_help(format!(
        "`{name}` is not a declared or built-in type; use one of {described}, or declare `type {name}`"
    ))
}

fn case_mismatch(name: &str, exact: &str, pattern: &Spanned<Pattern>, file: &str) -> Diagnostic {
    // A lowercase exact spelling cannot be written as a tag pattern (lowercase
    // patterns are bindings), so point at the label instead.
    let help = if exact.starts_with(|c: char| c.is_ascii_uppercase()) {
        format!("use the exact spelling `{exact}`")
    } else {
        format!("capitalise the `classify` label to `{name}` — a lowercase pattern is a binding, not a tag")
    };
    Diagnostic::error(
        "E162",
        file,
        format!("pattern `{name}` never matches `\"{exact}\"` — tag patterns compare exact text"),
        pattern.span.start..pattern.span.end,
        "this arm is dead code and falls through",
    )
    .with_help(help)
}

fn subject_label(subject: &Spanned<Expr>) -> String {
    match &subject.node {
        Expr::Ident(name) => format!("`{name}`"),
        _ => "the scrutinee".to_string(),
    }
}

fn describe(values: &Values) -> String {
    match values {
        Values::Literals(literals) => literals
            .iter()
            .map(|l| format!("\"{l}\""))
            .collect::<Vec<_>>()
            .join(", "),
        Values::Record(type_name) => format!("a `{type_name}` record"),
    }
}

/// Closest candidate within edit distance 2, if any. Ties break by name first
/// so the suggestion never depends on hash order, and a declared name beats a
/// builtin at the same distance.
fn closest_name<'a>(name: &str, declared: impl Iterator<Item = &'a String>) -> Option<&'a str> {
    declared
        .map(|c| (edit_distance(name, c), 0u8, c.as_str()))
        .chain(
            BUILTIN_CALLS
                .iter()
                .map(|b| (edit_distance(name, b), 1u8, *b)),
        )
        .filter(|(d, _, _)| *d <= 2)
        .min_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(b.2)))
        .map(|(_, _, name)| name)
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
        let expected = format!("declare a task or pure named `judge` {PROJECT_HINT}");
        assert_eq!(ds[0].help.as_deref(), Some(expected.as_str()));
    }

    #[test]
    fn resolution_help_points_at_merge_for_multi_file_projects() {
        // `workflows/dev-cycle/agents.forge` calls `repo_config_for`, declared
        // in a sibling manifest source: `forge check <file>` alone cannot see
        // it, so the help must say how to check the files together.
        let src = "task t\n  gives Text\n  do\n    give repo_config_for(\"x\")\n";
        let ds = diags(src);
        assert_eq!(codes(src), vec!["E160"], "{ds:?}");
        let help = ds[0].help.as_deref().expect("help");
        assert!(help.contains("--merge"), "{help}");
        // #495: the project-aware path names the flag that also registers skills.
        assert!(help.contains("--manifest forge.project.toml"), "{help}");
        assert!(
            help.starts_with("declare a task or pure named `repo_config_for`"),
            "{help}"
        );
    }

    #[test]
    fn resolution_help_points_at_merge_when_the_file_has_a_boundary_directive() {
        let src = "#! boundary: server\n\ntask greet\n  gives Text\n  do\n    give \"hi\"\n\nfn main\n  say greeet()\n";
        let ds = diags(src);
        assert_eq!(codes(src), vec!["E160"], "{ds:?}");
        let help = ds[0].help.as_deref().expect("help");
        assert!(help.contains("did you mean `greet`?"), "{help}");
        assert!(help.contains("--merge"), "{help}");
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
    fn resolution_flags_calling_main() {
        // `fn main` is the entry point, not a call target: `Expr::Call` in the
        // executor never resolves it, so `main()` can only fail at runtime.
        // This test used to allow it (issue #496 review, item 2).
        let src = "fn main\n  say main()\n";
        let ds = diags(src);
        assert_eq!(codes(src), vec!["E160"], "{ds:?}");
        assert!(ds[0].message.contains("`main`"), "{:?}", ds[0].message);
    }

    #[test]
    fn resolution_suggestion_breaks_ties_by_name_order() {
        // `ab` is one edit from both `aa` and `ac`: the alphabetically first
        // wins, so the help cannot depend on HashSet iteration order.
        let src = "task aa\n  gives Text\n  do\n    give \"a\"\n\ntask ac\n  gives Text\n  do\n    give \"c\"\n\nfn main\n  say ab()\n";
        let ds = diags(src);
        assert_eq!(codes(src), vec!["E160"], "{ds:?}");
        assert_eq!(ds[0].help.as_deref(), Some("did you mean `aa`?"));
    }

    #[test]
    fn resolution_suggestion_prefers_a_declared_name_over_a_builtin() {
        // `assed` is one edit from the builtin `asset` and from the declared
        // `asses`: the declaration wins.
        let src = "task asses\n  gives Text\n  do\n    give \"a\"\n\nfn main\n  say assed()\n";
        let ds = diags(src);
        assert_eq!(codes(src), vec!["E160"], "{ds:?}");
        assert_eq!(ds[0].help.as_deref(), Some("did you mean `asses`?"));

        // Builtins still get suggested when nothing declared is closer.
        let src = "fn main\n  say winning_line([1, 2])\n";
        let ds = diags(src);
        assert_eq!(codes(src), vec!["E160"], "{ds:?}");
        assert_eq!(ds[0].help.as_deref(), Some("did you mean `winning_lines`?"));
    }

    // ── E160 — walk coverage outside plain statements ───────────

    #[test]
    fn resolution_flags_calls_inside_session_sub_expressions() {
        let src = "fn main\n  s = session \"job\" tools deploy_tools() budget 10\n  say s\n";
        let ds = diags(src);
        assert_eq!(codes(src), vec!["E160"], "{ds:?}");
        assert!(
            ds[0].message.contains("deploy_tools"),
            "{:?}",
            ds[0].message
        );
    }

    #[test]
    fn resolution_flags_calls_inside_find_interpolations() {
        let src = "fn main\n  existing = find \"spec_{agent_key(1)}\"\n  say existing\n";
        let ds = diags(src);
        assert_eq!(codes(src), vec!["E160"], "{ds:?}");
        assert!(ds[0].message.contains("agent_key"), "{:?}", ds[0].message);
    }

    #[test]
    fn resolution_flags_calls_inside_retire_target_and_export() {
        let target = "fn main\n  retire \"spec_{tag_for(1)}\"\n";
        let ds = diags(target);
        assert_eq!(codes(target), vec!["E160"], "{ds:?}");
        assert!(ds[0].message.contains("tag_for"), "{:?}", ds[0].message);

        let export = "task t\n  gives Text\n  do\n    retire \"worker\"\n      with knowledge export: \"{path_for(1)}\"\n    give \"done\"\n";
        let ds = diags(export);
        assert_eq!(codes(export), vec!["E160"], "{ds:?}");
        assert!(ds[0].message.contains("path_for"), "{:?}", ds[0].message);
    }

    // ── E161/E162 — impossible and case-mismatched patterns ─────

    const CLASSIFY_TASK: &str = "task t\n  needs pick: Text\n  gives Text\n  do\n";

    fn classify_task(arms: &str) -> String {
        format!(
            "{CLASSIFY_TASK}    result = classify pick into [\"Buy\"]\n    match result\n{arms}"
        )
    }

    #[test]
    fn resolution_flags_undeclared_variant_against_classify_labels() {
        let src =
            classify_task("      Nonexistent(who) -> give \"never\"\n      _ -> give \"other\"\n");
        let ds = diags(&src);
        assert_eq!(codes(&src), vec!["E161"], "{ds:?}");
        assert!(ds[0].message.contains("Nonexistent"), "{:?}", ds[0].message);
        assert!(ds[0].message.contains("\"Buy\""), "{:?}", ds[0].message);
    }

    #[test]
    fn resolution_allows_classify_label_exact_match() {
        let src = classify_task("      Buy(who) -> give \"buy\"\n      _ -> give \"other\"\n");
        assert_eq!(codes(&src), Vec::<&str>::new(), "{:?}", diags(&src));
    }

    #[test]
    fn resolution_flags_case_mismatched_tag() {
        let src = "task t\n  needs pick: Text\n  gives Text\n  do\n    result = classify pick into [\"positive\"]\n    match result\n      Positive -> give \"up\"\n      _ -> give \"other\"\n";
        let ds = diags(src);
        assert_eq!(codes(src), vec!["E162"], "{ds:?}");
        assert_eq!(
            ds[0].message,
            "pattern `Positive` never matches `\"positive\"` — tag patterns compare exact text"
        );
        assert_eq!(
            ds[0].help.as_deref(),
            Some(
                "capitalise the `classify` label to `Positive` — a lowercase pattern is a binding, not a tag"
            )
        );
    }

    #[test]
    fn resolution_flags_case_mismatched_uppercase_tag() {
        let src = classify_task("      BUY -> give \"buy\"\n      _ -> give \"other\"\n");
        let ds = diags(&src);
        assert_eq!(codes(&src), vec!["E162"], "{ds:?}");
        assert_eq!(ds[0].help.as_deref(), Some("use the exact spelling `Buy`"));
    }

    #[test]
    fn resolution_ignores_unknown_value_sets() {
        // `pick` is a parameter — nothing is known about it.
        let src = "task t\n  needs pick: Text\n  gives Text\n  do\n    match pick\n      Whatever -> give \"no\"\n      _ -> give \"other\"\n";
        assert_eq!(codes(src), Vec::<&str>::new(), "{:?}", diags(src));
    }

    #[test]
    fn resolution_ignores_declared_type_patterns_and_wildcards() {
        // A declared type is never flagged even when the value set is known,
        // and neither are `_` or binding patterns.
        let src = "type Order\n  id: Text\n\ntask t\n  needs pick: Text\n  gives Text\n  do\n    result = classify pick into [\"Buy\"]\n    match result\n      Order -> give \"order\"\n      _ -> give \"other\"\n      anything -> give \"{anything}\"\n";
        assert_eq!(codes(src), Vec::<&str>::new(), "{:?}", diags(src));
    }

    #[test]
    fn resolution_uses_literal_gives_as_the_value_set() {
        let src = "pure verdict\n  gives Text\n  do\n    give \"yes\"\n\ntask t\n  gives Text\n  do\n    v = verdict()\n    match v\n      Nope -> give \"a\"\n      _ -> give \"b\"\n";
        let ds = diags(src);
        assert_eq!(codes(src), vec!["E161"], "{ds:?}");
        assert!(ds[0].message.contains("\"yes\""), "{:?}", ds[0].message);
    }

    #[test]
    fn resolution_ignores_non_literal_gives() {
        let src = "pure verdict\n  needs x: Text\n  gives Text\n  do\n    give x\n\ntask t\n  needs x: Text\n  gives Text\n  do\n    v = verdict(x)\n    match v\n      Nope -> give \"a\"\n      _ -> give \"b\"\n";
        assert_eq!(codes(src), Vec::<&str>::new(), "{:?}", diags(src));
    }

    #[test]
    fn resolution_uses_declared_type_constructors_as_the_value_set() {
        let src = "type Order\n  id: Text\n\ntask t\n  gives Text\n  do\n    o = Order(\"a\")\n    match o\n      Other -> give \"a\"\n      Order -> give \"b\"\n";
        let ds = diags(src);
        assert_eq!(codes(src), vec!["E161"], "{ds:?}");
        assert!(
            ds[0].message.contains("`Order` record"),
            "{:?}",
            ds[0].message
        );
    }

    #[test]
    fn resolution_tracks_only_the_last_binding_before_the_match() {
        let src = "task t\n  needs pick: Text\n  gives Text\n  do\n    result = classify pick into [\"Buy\"]\n    result = pick\n    match result\n      Buy -> give \"buy\"\n      _ -> give \"other\"\n";
        assert_eq!(codes(src), Vec::<&str>::new(), "{:?}", diags(src));
    }
}
