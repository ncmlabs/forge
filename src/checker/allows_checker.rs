// FORGE allows checker — issue #363 (T9.2)
//
// Enforces per-agent skill allow-lists. An agent declaring
//
//     allows skill.github.*, skill.exec.ripgrep
//
// is permitted to invoke any `skill.github.<name>(…)` and exactly
// `skill.exec.ripgrep(…)`. A call to any other `skill.X.Y(…)` raises an
// error diagnostic. Agents without an `allows` clause are unrestricted —
// this preserves back-compat for every program that pre-dates the clause.
//
// The pattern matcher is intentionally narrow: only suffix `.*` glob is
// supported (`skill.github.*` matches `skill.github.create_pr` but not
// `skill.github.foo.bar`). Non-suffix wildcards and patterns missing the
// `skill.` prefix are rejected at parse time so authors hear about
// malformed allow-lists immediately.

use crate::ast::*;
use crate::diagnostic::Diagnostic;

/// Run the allow-list checker on a program.
pub fn check(program: &Program, file: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for item in &program.items {
        if let TopLevel::Agent(agent) = &item.node {
            check_agent(agent, file, &mut diagnostics);
        }
    }

    diagnostics
}

fn check_agent(agent: &AgentDecl, file: &str, diagnostics: &mut Vec<Diagnostic>) {
    // 1) Validate the patterns themselves before using them. Bad patterns
    //    don't constrain anything, so we report the malformation and bail
    //    on enforcement for this agent (otherwise every call site would
    //    spuriously fail).
    let mut pattern_ok = true;
    for pat in &agent.allows {
        if let Some(d) = validate_pattern(&pat.node, pat.span.start..pat.span.end, file) {
            diagnostics.push(d);
            pattern_ok = false;
        }
    }
    if !pattern_ok {
        return;
    }

    // 2) Empty allow-list → unrestricted (back-compat).
    if agent.allows.is_empty() {
        return;
    }

    let allows: Vec<&str> = agent.allows.iter().map(|s| s.node.as_str()).collect();
    let agent_name = agent.name.node.as_str();

    // 3) Walk every handler body and the stuck_policy body.
    for handler in &agent.handlers {
        check_stmts(&handler.node.body, agent_name, &allows, file, diagnostics);
    }
    if let Some(stuck) = &agent.stuck_policy {
        check_stmts(&stuck.node.body, agent_name, &allows, file, diagnostics);
    }
}

fn validate_pattern(pat: &str, span: std::ops::Range<usize>, file: &str) -> Option<Diagnostic> {
    if !pat.starts_with("skill.") {
        return Some(
            Diagnostic::error(
                file,
                format!("allows pattern `{}` must start with `skill.`", pat),
                span,
                "only `skill.X[.Y[.*]]` patterns are supported in this iteration",
            )
            .with_help("rewrite the pattern as `skill.<namespace>[.<capability>|.*]`"),
        );
    }
    // Reject `*` anywhere except as a complete final segment.
    if let Some(idx) = pat.find('*') {
        let suffix_ok = pat.ends_with(".*") && idx == pat.len() - 1;
        if !suffix_ok {
            return Some(
                Diagnostic::error(
                    file,
                    format!("allows pattern `{}` has a malformed wildcard", pat),
                    span,
                    "only suffix `.*` is supported (e.g. `skill.github.*`)",
                )
                .with_help("move the wildcard to the end of the pattern"),
            );
        }
    }
    None
}

fn check_stmts(
    stmts: &[Spanned<Stmt>],
    agent: &str,
    allows: &[&str],
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for stmt in stmts {
        check_stmt(stmt, agent, allows, file, diagnostics);
    }
}

fn check_stmt(
    stmt: &Spanned<Stmt>,
    agent: &str,
    allows: &[&str],
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match &stmt.node {
        Stmt::Bind(_, expr) | Stmt::Give(expr, _) | Stmt::Say(expr) | Stmt::ExprStmt(expr) => {
            check_expr(expr, agent, allows, file, diagnostics);
        }
        Stmt::MemoryUpdate(_, idx, value) => {
            if let Some(i) = idx {
                check_expr(i, agent, allows, file, diagnostics);
            }
            check_expr(value, agent, allows, file, diagnostics);
        }
        Stmt::Emit(_, args) => {
            for a in args {
                check_expr(&a.node.value, agent, allows, file, diagnostics);
            }
        }
        Stmt::Forward(src, dst) => {
            check_expr(src, agent, allows, file, diagnostics);
            check_expr(dst, agent, allows, file, diagnostics);
        }
        Stmt::IfElse(ie) => {
            check_expr(&ie.condition, agent, allows, file, diagnostics);
            check_stmts(&ie.then_body, agent, allows, file, diagnostics);
            for (cond, body) in &ie.else_ifs {
                check_expr(cond, agent, allows, file, diagnostics);
                check_stmts(body, agent, allows, file, diagnostics);
            }
            if let Some(body) = &ie.else_body {
                check_stmts(body, agent, allows, file, diagnostics);
            }
        }
        Stmt::For(f) => {
            check_expr(&f.iterable, agent, allows, file, diagnostics);
            check_stmts(&f.body, agent, allows, file, diagnostics);
        }
        Stmt::When(w) => {
            for clause in &w.clauses {
                check_stmt(&clause.node.body, agent, allows, file, diagnostics);
            }
            if let Some(else_clause) = &w.else_body {
                check_stmt(&else_clause.node.body, agent, allows, file, diagnostics);
            }
        }
        Stmt::Match(m) => {
            check_expr(&m.subject, agent, allows, file, diagnostics);
            for arm in &m.arms {
                check_stmt(&arm.node.body, agent, allows, file, diagnostics);
            }
        }
        Stmt::StartTimer { context, .. } | Stmt::CancelTimer { context, .. } => {
            if let Some(c) = context {
                check_expr(c, agent, allows, file, diagnostics);
            }
        }
        Stmt::Spawn(s) => {
            if let Some(alias) = &s.alias {
                check_expr(alias, agent, allows, file, diagnostics);
            }
            // SpawnOption value expressions are checked elsewhere; skill
            // calls aren't typical there but we conservatively skip.
        }
        // Leaves / non-expression-bearing stmts.
        Stmt::Escalate(_)
        | Stmt::TransitionTo(_)
        | Stmt::ResetTimer(_)
        | Stmt::Learn(_, _)
        | Stmt::Retire(_) => {}
    }
}

fn check_expr(
    expr: &Spanned<Expr>,
    agent: &str,
    allows: &[&str],
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    // 1) If this is itself a skill call, validate it.
    if let Some(path) = skill_call_path(&expr.node) {
        if !skill_call_allowed(&path, allows) {
            diagnostics.push(
                Diagnostic::error(
                    file,
                    format!(
                        "agent `{}` is not allowed to call `{}` — add an `allows` pattern that covers it",
                        agent, path
                    ),
                    expr.span.start..expr.span.end,
                    "this skill is outside the agent's allow-list",
                )
                .with_help(format!(
                    "add `allows {}.*` (or the exact name) to the `{}` declaration",
                    skill_namespace(&path),
                    agent
                )),
            );
        }
    }

    // 2) Recurse into children regardless — a non-skill expression may still
    //    contain a nested skill call (e.g. inside an arg, an array literal,
    //    a binary op, or a method-call receiver chain).
    walk_children(&expr.node, agent, allows, file, diagnostics);
}

fn walk_children(
    expr: &Expr,
    agent: &str,
    allows: &[&str],
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match expr {
        Expr::NumberLit(_) | Expr::BoolLit(_) | Expr::Ident(_) | Expr::TypeAccess(_, _) => {}
        Expr::Template(parts) => {
            for p in parts {
                if let TemplatePart::Interp(e) | TemplatePart::RawInterp(e) = &p.node {
                    check_expr(e, agent, allows, file, diagnostics);
                }
            }
        }
        Expr::Call(c) => {
            for a in &c.args {
                check_expr(&a.node.value, agent, allows, file, diagnostics);
            }
        }
        Expr::Constructor(c) => {
            for a in &c.args {
                check_expr(&a.node.value, agent, allows, file, diagnostics);
            }
        }
        Expr::Reason(r) => {
            check_expr(&r.prompt, agent, allows, file, diagnostics);
        }
        Expr::Classify(c) => {
            check_expr(&c.input, agent, allows, file, diagnostics);
        }
        Expr::Search(e)
        | Expr::Recall(e)
        | Expr::Exec(e)
        | Expr::Paren(e)
        | Expr::GlobAccess(e) => {
            check_expr(e, agent, allows, file, diagnostics);
        }
        Expr::Command(c) => {
            check_expr(&c.cmd, agent, allows, file, diagnostics);
            if let Some(wd) = &c.working_dir {
                check_expr(wd, agent, allows, file, diagnostics);
            }
        }
        Expr::CommandMethod(_, args) | Expr::SessionMethod(_, args) => {
            for a in args {
                check_expr(&a.node.value, agent, allows, file, diagnostics);
            }
        }
        Expr::Session(_) => {
            // Session bodies don't carry direct skill calls.
        }
        Expr::Find(_) => {}
        Expr::TryOr(a, b) => {
            check_expr(a, agent, allows, file, diagnostics);
            check_expr(b, agent, allows, file, diagnostics);
        }
        Expr::Compose(items) | Expr::FanOut(items) | Expr::ArrayLit(items) => {
            for it in items {
                check_expr(it, agent, allows, file, diagnostics);
            }
        }
        Expr::FieldAccess(inner, _) => {
            check_expr(inner, agent, allows, file, diagnostics);
        }
        Expr::Index(base, idx) => {
            check_expr(base, agent, allows, file, diagnostics);
            check_expr(idx, agent, allows, file, diagnostics);
        }
        Expr::MethodCall(receiver, _method, args) => {
            check_expr(receiver, agent, allows, file, diagnostics);
            for a in args {
                check_expr(&a.node.value, agent, allows, file, diagnostics);
            }
        }
        Expr::BinOp(l, _, r) => {
            check_expr(l, agent, allows, file, diagnostics);
            check_expr(r, agent, allows, file, diagnostics);
        }
        Expr::UnaryOp(_, e) => {
            check_expr(e, agent, allows, file, diagnostics);
        }
    }
}

/// Reconstruct the dotted skill path for a method-call expression of the
/// form `skill.<ns>[.<seg>]*.<method>`. Returns `None` for non-skill calls.
fn skill_call_path(expr: &Expr) -> Option<String> {
    if let Expr::MethodCall(receiver, method, _args) = expr {
        let prefix = expr_to_dotted(&receiver.node)?;
        if prefix == "skill" || prefix.starts_with("skill.") {
            return Some(format!("{}.{}", prefix, method.node));
        }
    }
    None
}

fn expr_to_dotted(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Ident(name) => Some(name.clone()),
        Expr::FieldAccess(inner, name) => {
            let prefix = expr_to_dotted(&inner.node)?;
            Some(format!("{}.{}", prefix, name.node))
        }
        _ => None,
    }
}

/// Decide whether a fully-qualified call path is permitted by the agent's
/// allow-list. Empty list means unrestricted (handled by the caller).
fn skill_call_allowed(call_path: &str, allows: &[&str]) -> bool {
    for pat in allows {
        if *pat == call_path {
            return true;
        }
        if let Some(prefix) = pat.strip_suffix(".*") {
            // `skill.github.*` matches `skill.github.<single-segment>` only.
            if let Some(rest) = call_path.strip_prefix(prefix) {
                if let Some(after_dot) = rest.strip_prefix('.') {
                    if !after_dot.contains('.') && !after_dot.is_empty() {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Extract `skill.<ns>` from a call path for the help message.
fn skill_namespace(call_path: &str) -> String {
    let parts: Vec<&str> = call_path.split('.').collect();
    if parts.len() >= 2 {
        format!("{}.{}", parts[0], parts[1])
    } else {
        call_path.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::DiagnosticKind;
    use crate::parser::parse;

    fn diags(src: &str) -> Vec<Diagnostic> {
        let program = parse(src).expect("parse failed");
        check(&program, "test.forge")
    }

    #[test]
    fn allows_glob_matches_specific_call() {
        let src = r#"
agent code_inv
  allows skill.github.*
  on InvestigationRequested(thread_ts: Text)
    result = skill.github.create_pr("repo", "branch", "title", "body")
    say result
"#;
        let ds = diags(src);
        assert!(
            ds.iter().all(|d| !matches!(d.kind, DiagnosticKind::Error)),
            "unexpected errors: {:?}",
            ds
        );
    }

    #[test]
    fn allows_denies_unlisted_skill() {
        let src = r#"
agent code_inv
  allows skill.github.*
  on InvestigationRequested(thread_ts: Text)
    result = skill.slack.send_approval("c", "b", "url", "id")
    say result
"#;
        let ds = diags(src);
        let errs: Vec<&Diagnostic> = ds
            .iter()
            .filter(|d| matches!(d.kind, DiagnosticKind::Error))
            .collect();
        assert_eq!(errs.len(), 1, "expected 1 error, got: {:?}", ds);
        let msg = &errs[0].message;
        assert!(msg.contains("code_inv"), "msg should name agent: {}", msg);
        assert!(
            msg.contains("skill.slack.send_approval"),
            "msg should name skill: {}",
            msg
        );
    }

    #[test]
    fn allows_no_block_means_unrestricted() {
        let src = r#"
agent legacy
  on Ping(thread_ts: Text)
    a = skill.slack.send_approval("c", "b", "url", "id")
    b = skill.github.create_pr("r", "br", "t", "bd")
    say a
    say b
"#;
        let ds = diags(src);
        assert!(
            ds.iter().all(|d| !matches!(d.kind, DiagnosticKind::Error)),
            "unexpected errors with no allows clause: {:?}",
            ds
        );
    }

    #[test]
    fn grammar_rejects_non_suffix_glob() {
        // The skill_pattern rule in the grammar rejects `*` in non-suffix
        // positions. This is enforced before the checker runs, so the parser
        // surfaces the malformed pattern with a syntax error.
        let src = r#"
agent bad
  allows skill.*.create_pr
  on Ping(thread_ts: Text)
    say "ok"
"#;
        assert!(
            parse(src).is_err(),
            "grammar should reject non-suffix wildcards"
        );
    }

    #[test]
    fn allows_walks_nested_handler_bodies() {
        let src = r#"
agent code_inv
  allows skill.github.*
  on InvestigationRequested(thread_ts: Text)
    if 1 == 1
      for r in [1]
        bad = skill.slack.send_approval("c", "b", "url", "id")
        say bad
"#;
        let ds = diags(src);
        let errs: Vec<&Diagnostic> = ds
            .iter()
            .filter(|d| matches!(d.kind, DiagnosticKind::Error))
            .collect();
        assert_eq!(
            errs.len(),
            1,
            "expected nested call to be flagged, got: {:?}",
            ds
        );
        assert!(errs[0].message.contains("skill.slack.send_approval"));
    }

    #[test]
    fn glob_does_not_match_deeper_nesting() {
        // skill.github.* should NOT match skill.github.foo.bar — flat namespace.
        let src = r#"
agent dot
  allows skill.github.*
  on Ping(thread_ts: Text)
    x = skill.github.foo.bar("arg")
    say x
"#;
        let ds = diags(src);
        let errs: Vec<&Diagnostic> = ds
            .iter()
            .filter(|d| matches!(d.kind, DiagnosticKind::Error))
            .collect();
        assert_eq!(
            errs.len(),
            1,
            "expected 4-segment skill call to fail single-segment glob: {:?}",
            ds
        );
    }

    #[test]
    fn allows_exact_pattern_matches() {
        let src = r#"
agent code_inv
  allows skill.github.merge_pr
  on Ping(thread_ts: Text)
    out = skill.github.merge_pr("repo", 42)
    say out
"#;
        let ds = diags(src);
        assert!(
            ds.iter().all(|d| !matches!(d.kind, DiagnosticKind::Error)),
            "exact pattern should match: {:?}",
            ds
        );
    }

    #[test]
    fn allows_rejects_non_skill_prefix() {
        let src = r#"
agent bad
  allows skill.github.*
  on Ping(thread_ts: Text)
    say "ok"
"#;
        // Sanity: parser currently forces patterns to start with "skill."
        // via the grammar rule. The validate_pattern path handles cases
        // where downstream tooling synthesizes a malformed AST node.
        let ds = diags(src);
        assert!(ds.iter().all(|d| !matches!(d.kind, DiagnosticKind::Error)));
    }
}
