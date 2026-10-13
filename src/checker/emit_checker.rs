// FORGE emit checker — issue #532.
//
// E165: an `emit` statement written in `fn main`. The runtime only carries an
// event bus inside agent handlers and endpoints (`emit_precomputed` in
// `src/runtime/executor.rs`), so a `fn main` emit can never run. Tasks and
// flows are deliberately not flagged: a task called from an agent handler
// emits correctly at runtime, so rejecting it would reject working programs.

use crate::ast::*;
use crate::diagnostic::Diagnostic;

/// How to place an emit that works (#532). Verified against a `system` run:
/// a subscriber bound before the emitting agent receives the event, one bound
/// after does not — `on start` runs while later agents are still being bound,
/// so such an emit is published to zero subscribers.
const HELP: &str = "emit from an agent's `on` handler or from an endpoint; an `on start` emit only reaches agents already subscribed when it runs";

/// Run the emit checker over the program: `emit` in `fn main` is E165.
pub fn check(program: &Program, file: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for item in &program.items {
        if let TopLevel::FnMain(main) = &item.node {
            check_stmts(&main.body, file, &mut diagnostics);
        }
    }
    diagnostics
}

fn check_stmts(stmts: &[Spanned<Stmt>], file: &str, diagnostics: &mut Vec<Diagnostic>) {
    for stmt in stmts {
        check_stmt(stmt, file, diagnostics);
    }
}

fn check_stmt(stmt: &Spanned<Stmt>, file: &str, diagnostics: &mut Vec<Diagnostic>) {
    match &stmt.node {
        Stmt::Emit(event, _) => diagnostics.push(emit_in_main(event, file)),
        // Recurse into nested statement bodies.
        Stmt::IfElse(ie) => {
            check_stmts(&ie.then_body, file, diagnostics);
            for (_, body) in &ie.else_ifs {
                check_stmts(body, file, diagnostics);
            }
            if let Some(ref body) = ie.else_body {
                check_stmts(body, file, diagnostics);
            }
        }
        Stmt::For(f) => check_stmts(&f.body, file, diagnostics),
        Stmt::When(w) => {
            for clause in &w.clauses {
                check_stmt(&clause.node.body, file, diagnostics);
            }
            if let Some(else_clause) = &w.else_body {
                check_stmt(&else_clause.node.body, file, diagnostics);
            }
        }
        Stmt::Match(m) => {
            for arm in &m.arms {
                check_stmt(&arm.node.body, file, diagnostics);
            }
        }
        _ => {}
    }
}

/// E165 — the event name is the span, so the caret lands on what cannot be
/// published.
fn emit_in_main(event: &Spanned<String>, file: &str) -> Diagnostic {
    Diagnostic::error(
        "E165",
        file,
        "`emit` needs an agent: `fn main` has no event bus",
        event.span.start..event.span.end,
        "emitted from fn main",
    )
    .with_help(HELP)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn codes(src: &str) -> Vec<&'static str> {
        let program = parse(src).expect("parse failed");
        check(&program, "test.forge")
            .iter()
            .map(|d| d.code)
            .collect()
    }

    #[test]
    fn emit_in_main_is_e165() {
        let src = "fn main\n  emit Kickoff(topic: \"hello\")\n";
        let ds = {
            let program = parse(src).expect("parse failed");
            check(&program, "test.forge")
        };
        assert_eq!(ds.len(), 1, "{ds:?}");
        assert_eq!(ds[0].code, "E165");
        assert_eq!(
            ds[0].message,
            "`emit` needs an agent: `fn main` has no event bus"
        );
        assert_eq!(ds[0].label, "emitted from fn main");
        // The span covers the event name, not the whole statement.
        assert_eq!(&src[ds[0].span.start..ds[0].span.end], "Kickoff");
        assert_eq!(ds[0].help.as_deref(), Some(HELP));
    }

    #[test]
    fn emit_nested_in_main_is_e165() {
        // Every nested statement body `fn main` can hold. `fn main` statements
        // sit at i1, so nested if/for/match bodies sit at i3 and an `else` at
        // i2 (grammar/forge.pest).
        let cases = [
            ("if", "fn main\n  if true\n      emit Kickoff(topic: \"a\")\n"),
            (
                "else",
                "fn main\n  if false\n      say \"no\"\n    else\n      emit Kickoff(topic: \"a\")\n",
            ),
            (
                "else if",
                "fn main\n  if false\n      say \"no\"\n    else if true\n      emit Kickoff(topic: \"a\")\n",
            ),
            (
                "for",
                "fn main\n  for x in [\"a\"]\n      emit Kickoff(topic: x)\n",
            ),
            (
                "when",
                "fn main\n  done = true\n  when done.sure -> emit Kickoff(topic: \"a\")\n",
            ),
            (
                "when else",
                "fn main\n  done = true\n  when done.sure -> say \"no\"\n    else -> emit Kickoff(topic: \"a\")\n",
            ),
            (
                "match arm",
                "fn main\n  match \"a\"\n      _ -> emit Kickoff(topic: \"a\")\n",
            ),
        ];
        for (name, src) in cases {
            assert_eq!(codes(src), vec!["E165"], "{name}: {src}");
        }
    }

    #[test]
    fn emit_outside_main_is_not_e165() {
        // Agents, endpoints, tasks and flows all have a runtime context for
        // emit — a task called from an agent handler emits correctly.
        let cases = [
            (
                "agent handler",
                "event Kickoff\n  topic: Text\n\nagent a\n  on go(topic: Text)\n    emit Kickoff(topic: topic)\n",
            ),
            (
                "endpoint",
                "#! boundary: server\n\nevent Kickoff\n  topic: Text\n\nendpoint go(topic: Text) -> Text\n  emit Kickoff(topic: topic)\n  give \"ok\"\n",
            ),
            (
                "task",
                "event Kickoff\n  topic: Text\n\ntask go\n  needs topic: Text\n  gives Text\n  do\n    emit Kickoff(topic: topic)\n    give \"ok\"\n",
            ),
            (
                "flow stage",
                "event Kickoff\n  topic: Text\n\nflow go\n  needs topic: Text\n  gives Text\n  stage s\n    emit Kickoff(topic: topic)\n    give \"ok\"\n",
            ),
        ];
        for (name, src) in cases {
            assert_eq!(codes(src), Vec::<&str>::new(), "{name}: {src}");
        }
    }
}
