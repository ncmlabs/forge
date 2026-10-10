// FORGE command gate checker — issue #484
// A `command`/`exec` result is an unchecked claim about the outside world:
// until the body dispatches it, no oracle may decide in its place (#431).
// Dataflow tracking mirrors `uncertain_checker`, with a different taint.

use std::collections::HashSet;

use crate::ast::{Expr, Program, Spanned, Stmt, TaskBody, TemplatePart, TopLevel};
use crate::diagnostic::Diagnostic;

// ── Public API ──────────────────────────────────────────────────

pub fn check(program: &Program, file: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for item in &program.items {
        match &item.node {
            TopLevel::Task(d) => {
                if let TaskBody::Do(stmts) = &d.body.node {
                    check_stmts(stmts, file, &mut diagnostics);
                }
            }
            TopLevel::Flow(d) => {
                for stage in &d.stages {
                    check_stmts(&stage.node.body, file, &mut diagnostics);
                }
            }
            TopLevel::Agent(d) => {
                for handler in &d.handlers {
                    check_stmts(&handler.node.body, file, &mut diagnostics);
                }
            }
            TopLevel::FnMain(d) => check_stmts(&d.body, file, &mut diagnostics),
            _ => {}
        }
    }

    diagnostics
}

// ── Unchecked tracking ──────────────────────────────────────────

fn check_stmts(stmts: &[Spanned<Stmt>], file: &str, diagnostics: &mut Vec<Diagnostic>) {
    let mut unchecked: HashSet<String> = HashSet::new();
    // Names bound from a foreground `command` record: only these expose
    // `.success` / `.exit_code`, so only these can be gated by reading them.
    let mut records: HashSet<String> = HashSet::new();

    for stmt in stmts {
        check_stmt(stmt, &mut unchecked, &mut records, file, diagnostics);
    }
}

fn check_stmt(
    stmt: &Spanned<Stmt>,
    unchecked: &mut HashSet<String>,
    records: &mut HashSet<String>,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match &stmt.node {
        Stmt::Bind(name, expr) => {
            match command_binding(expr) {
                Some(Binding::CommandRecord) => {
                    unchecked.insert(name.node.clone());
                    records.insert(name.node.clone());
                }
                Some(Binding::Exec) => {
                    unchecked.insert(name.node.clone());
                    records.remove(&name.node);
                }
                // Background handles are checked through `command.status`.
                Some(Binding::BackgroundHandle) => {
                    unchecked.remove(&name.node);
                    records.remove(&name.node);
                }
                None => {
                    // Reassignment to anything else clears the gate.
                    unchecked.remove(&name.node);
                    records.remove(&name.node);
                    // An oracle verdict must not be built on a raw command
                    // result: that is the #431 shape.
                    if matches!(expr.node, Expr::Reason(_) | Expr::Classify(_)) {
                        gate_expr(expr, unchecked, file, diagnostics);
                    }
                }
            }
        }
        Stmt::Give(expr, metas) => {
            gate_expr(expr, unchecked, file, diagnostics);
            // `give "ok" with detail: x.stdout` promotes the metadata too.
            for meta in metas {
                gate_expr(&meta.node.value, unchecked, file, diagnostics);
            }
        }
        Stmt::Emit(_name, args) => {
            for arg in args {
                gate_expr(&arg.node.value, unchecked, file, diagnostics);
            }
        }
        Stmt::MemoryUpdate(_field, _index, value) => {
            // `memory.out = x.stdout` and `memory.out[i] = x.stdout` both store
            // the value for later decisions.
            gate_expr(value, unchecked, file, diagnostics);
        }
        Stmt::When(when) => {
            // `when x.sure` / `.unsure` / `.unreliable` on the command result
            // itself is a deterministic exit-status check, not an oracle
            // guess: `command` sets confidence 0.9 on exit 0 and 0.3
            // otherwise, so `.sure` (threshold 0.8) is exactly `success` and
            // the failure path lands in `.unsure` / `else`. For `exec`
            // (returns Text, no `.success` field) the same dispatch is the
            // only available gate. Either way the branch is on the command's
            // own verdict, so it clears the gate.
            for clause in &when.clauses {
                unchecked.remove(&clause.node.predicate.node.subject.node);
            }
            for clause in &when.clauses {
                check_stmt(&clause.node.body, unchecked, records, file, diagnostics);
            }
            if let Some(else_clause) = &when.else_body {
                check_stmt(
                    &else_clause.node.body,
                    unchecked,
                    records,
                    file,
                    diagnostics,
                );
            }
        }
        Stmt::Match(m) => {
            // A branch on `.success` / `.exit_code` is the deterministic gate.
            let mut reads = HashSet::new();
            collect_gate_reads(&m.subject, records, &mut reads);
            for name in reads {
                unchecked.remove(&name);
            }
            for arm in &m.arms {
                check_stmt(&arm.node.body, unchecked, records, file, diagnostics);
            }
        }
        Stmt::IfElse(ie) => {
            // The gate counts for both branches and for every statement
            // after the `if`, so clear before walking any body.
            let mut reads = HashSet::new();
            collect_gate_reads(&ie.condition, records, &mut reads);
            for (cond, _) in &ie.else_ifs {
                collect_gate_reads(cond, records, &mut reads);
            }
            for name in reads {
                unchecked.remove(&name);
            }
            for s in &ie.then_body {
                check_stmt(s, unchecked, records, file, diagnostics);
            }
            for (_cond, body) in &ie.else_ifs {
                for s in body {
                    check_stmt(s, unchecked, records, file, diagnostics);
                }
            }
            if let Some(body) = &ie.else_body {
                for s in body {
                    check_stmt(s, unchecked, records, file, diagnostics);
                }
            }
        }
        Stmt::For(f) => {
            for s in &f.body {
                check_stmt(s, unchecked, records, file, diagnostics);
            }
        }
        // `say` is not a decision, and `transition` takes no value.
        _ => {}
    }
}

/// How a `command`/`exec` expression binds: `None` for anything else.
fn command_binding(expr: &Spanned<Expr>) -> Option<Binding> {
    match &expr.node {
        Expr::Command(c) if c.background.as_ref().is_some_and(|b| b.node) => {
            Some(Binding::BackgroundHandle)
        }
        Expr::Command(_) => Some(Binding::CommandRecord),
        Expr::Exec(_) => Some(Binding::Exec),
        _ => None,
    }
}

/// How a `command`/`exec` expression binds.
enum Binding {
    /// Foreground `command`: a record with `.success` / `.exit_code`.
    CommandRecord,
    /// `exec`: returns `Text`, so those fields do not exist.
    Exec,
    /// `command ... background true`: a handle inspected via `command.status`.
    BackgroundHandle,
}

/// True when the bound expression is a foreground `command` record, whose
/// `.success` / `.exit_code` fields are the deterministic gate. Shared with
/// `uncertain_checker` (#484). `exec` returns `Text` (#507) and a background
/// `command` returns a handle, so neither exposes those fields.
pub(super) fn is_command_record(expr: &Spanned<Expr>) -> bool {
    matches!(command_binding(expr), Some(Binding::CommandRecord))
}

// ── Expression walking ──────────────────────────────────────────

/// Base variable name of an access chain: `x.stdout` and `x` both give `x`.
fn base_name(expr: &Spanned<Expr>) -> Option<&str> {
    match &expr.node {
        Expr::Ident(n) => Some(n),
        Expr::FieldAccess(inner, _) | Expr::GlobAccess(inner) => base_name(inner),
        Expr::Paren(inner) => base_name(inner),
        _ => None,
    }
}

/// Report every unchecked command result referenced by `expr`.
fn gate_expr(
    expr: &Spanned<Expr>,
    unchecked: &HashSet<String>,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut names = Vec::new();
    collect_unchecked_refs(expr, unchecked, &mut names);
    for name in names {
        emit_gate_error(&name, expr, file, diagnostics);
    }
}

fn collect_unchecked_refs(
    expr: &Spanned<Expr>,
    unchecked: &HashSet<String>,
    out: &mut Vec<String>,
) {
    match base_name(expr) {
        Some(name) => {
            if unchecked.contains(name) && !out.iter().any(|n| n == name) {
                out.push(name.to_string());
            }
        }
        None => walk_children(expr, &mut |child| {
            collect_unchecked_refs(child, unchecked, out)
        }),
    }
}

/// Collect names of unchecked command records whose `.success` / `.exit_code`
/// is read. `records` holds only foreground `command` bindings, so a read on an
/// `exec` binding or a background handle is never treated as a gate (#507).
/// Shared with `uncertain_checker` (#484): the same read is a deterministic
/// verdict there, so it also satisfies the confidence gate.
pub(super) fn collect_gate_reads(
    expr: &Spanned<Expr>,
    records: &HashSet<String>,
    out: &mut HashSet<String>,
) {
    if let Expr::FieldAccess(inner, field) = &expr.node {
        if field.node == "success" || field.node == "exit_code" {
            if let Some(name) = base_name(inner) {
                if records.contains(name) {
                    out.insert(name.to_string());
                }
            }
        }
    }
    walk_children(expr, &mut |child| collect_gate_reads(child, records, out));
}

fn walk_children(expr: &Spanned<Expr>, f: &mut impl FnMut(&Spanned<Expr>)) {
    match &expr.node {
        Expr::NumberLit(_) | Expr::BoolLit(_) | Expr::Ident(_) | Expr::TypeAccess(_, _) => {}
        Expr::Template(parts) => {
            for p in parts {
                if let TemplatePart::Interp(e) | TemplatePart::RawInterp(e) = &p.node {
                    f(e);
                }
            }
        }
        Expr::Call(c) => {
            for a in &c.args {
                f(&a.node.value);
            }
        }
        Expr::Constructor(c) => {
            for a in &c.args {
                f(&a.node.value);
            }
        }
        Expr::Reason(r) => f(&r.prompt),
        Expr::Classify(c) => f(&c.input),
        Expr::Search(e)
        | Expr::Recall(e)
        | Expr::Exec(e)
        | Expr::Paren(e)
        | Expr::GlobAccess(e) => f(e),
        Expr::Command(c) => {
            f(&c.cmd);
            if let Some(wd) = &c.working_dir {
                f(wd);
            }
        }
        Expr::CommandMethod(_, args) | Expr::SessionMethod(_, args) => {
            for a in args {
                f(&a.node.value);
            }
        }
        Expr::Session(_) | Expr::Find(_) => {}
        Expr::TryOr(a, b) => {
            f(a);
            f(b);
        }
        Expr::Compose(items) | Expr::FanOut(items) | Expr::ArrayLit(items) => {
            for it in items {
                f(it);
            }
        }
        Expr::FieldAccess(inner, _) => f(inner),
        Expr::Index(a, b) => {
            f(a);
            f(b);
        }
        Expr::MethodCall(recv, _, args) => {
            f(recv);
            for a in args {
                f(&a.node.value);
            }
        }
        Expr::BinOp(a, _, b) => {
            f(a);
            f(b);
        }
        Expr::UnaryOp(_, e) => f(e),
    }
}

// ── Diagnostics ─────────────────────────────────────────────────

fn emit_gate_error(
    name: &str,
    expr: &Spanned<Expr>,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    diagnostics.push(
        Diagnostic::error(
            "E150",
            file,
            format!("command result `{name}` used before checking `{name}.success`"),
            expr.span.start..expr.span.end,
            "this decision may overrule a failed command",
        )
        .with_help(
            "branch on x.success (or x.exit_code) first; inside the failure branch an oracle may explain the failure, not decide it",
        ),
    );
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn gate_errors(src: &str) -> Vec<String> {
        let program = parse(src).expect("parse failed");
        check(&program, "test.forge")
            .into_iter()
            .filter(|d| d.code == "E150")
            .map(|d| d.message)
            .collect()
    }

    fn assert_flagged(src: &str) {
        let msgs = gate_errors(src);
        assert_eq!(msgs.len(), 1, "expected one E150, got: {msgs:?}");
        assert!(
            msgs[0].contains("used before checking"),
            "unexpected message: {}",
            msgs[0]
        );
    }

    fn assert_clean(src: &str) {
        let msgs = gate_errors(src);
        assert!(msgs.is_empty(), "unexpected E150: {msgs:?}");
    }

    #[test]
    fn give_of_unchecked_result_is_flagged() {
        assert_flagged(
            r#"
task t
  gives Text
  do
    x = command ["cargo", "test"]
    give x.stdout
"#,
        );
    }

    #[test]
    fn emit_of_unchecked_result_is_flagged() {
        assert_flagged(
            r#"
agent a
  on start
    x = command "cargo test"
    emit Done(out: x.stdout)
"#,
        );
    }

    #[test]
    fn oracle_prompt_of_unchecked_result_is_flagged() {
        assert_flagged(
            r#"
task t
  gives Text
  do
    x = command "cargo test"
    v = reason "judge {x.stdout}"
    when v.sure -> give v
    else -> give "unknown"
"#,
        );
    }

    #[test]
    fn exec_result_is_flagged_too() {
        assert_flagged(
            r#"
task t
  gives Text
  do
    x = exec "cargo test"
    give x
"#,
        );
    }

    #[test]
    fn exec_exit_status_is_not_a_gate() {
        // `exec` returns Text: `x.exit_code` is a type error at runtime, so it
        // cannot be the deterministic gate (#507).
        assert_flagged(
            r#"
task t
  gives Text
  do
    x = exec "cargo test"
    if x.exit_code != 0
      give "failed"
    else
      give x
"#,
        );
    }

    #[test]
    fn give_metadata_is_gated() {
        assert_flagged(
            r#"
task t
  gives Text
  do
    x = command "cargo test"
    give "ok" with detail: x.stdout
"#,
        );
    }

    #[test]
    fn when_dispatch_clears_exec() {
        assert_clean(
            r#"
task t
  gives Text
  do
    x = exec "cargo test"
    when x.sure -> give x
    else -> give "FAILED"
"#,
        );
    }

    #[test]
    fn memory_assignment_is_gated() {
        assert_flagged(
            r#"
agent a
  memory
    out: Text
  on start
    x = command "cargo test"
    memory.out = x.stdout
"#,
        );
    }

    #[test]
    fn memory_index_assignment_is_gated() {
        assert_flagged(
            r#"
agent a
  memory
    out: Text[]
  on start
    x = command "cargo test"
    memory.out[0] = x.stdout
"#,
        );
    }

    #[test]
    fn success_branch_clears_the_gate() {
        assert_clean(
            r#"
task t
  gives Text
  do
    x = command "cargo test"
    if x.success
      give x.stdout
    else
      give "tests failed"
"#,
        );
    }

    #[test]
    fn exit_code_branch_clears_the_gate() {
        assert_clean(
            r#"
task t
  gives Text
  do
    x = command "cargo test"
    if x.exit_code != 0
      give "tests failed"
    else
      give x.stdout
"#,
        );
    }

    #[test]
    fn gate_counts_after_the_if() {
        assert_clean(
            r#"
task t
  gives Text
  do
    x = command "cargo test"
    if x.success
      say "green"
    give x.stdout
"#,
        );
    }

    #[test]
    fn oracle_in_failure_branch_is_allowed() {
        assert_clean(
            r#"
task t
  gives Text
  do
    x = command "cargo test"
    if x.success
      give x.stdout
    else
      v = reason "explain this failure: {x.stderr}"
      when v.sure -> give v
      else -> give "unknown"
"#,
        );
    }

    #[test]
    fn background_command_is_exempt() {
        assert_clean(
            r#"
task t
  gives Text
  do
    h = command "cargo watch" background true
    say h.status
    give "started"
"#,
        );
    }

    #[test]
    fn say_of_unchecked_result_is_allowed() {
        assert_clean(
            r#"
task t
  gives Text
  do
    x = command "cargo test"
    say x.stdout
    give "done"
"#,
        );
    }

    #[test]
    fn confidence_dispatch_clears_the_gate() {
        assert_clean(
            r#"
task t
  gives Text
  do
    x = command "cargo test"
    when x.sure -> give x.stdout
    else -> give "FAILED"
"#,
        );
    }

    #[test]
    fn match_on_exit_code_clears_the_gate() {
        assert_clean(
            r#"
task t
  gives Text
  do
    x = command "cargo test"
    match x.exit_code
      ok -> give x.stdout
      _ -> give "tests failed"
"#,
        );
    }
}
