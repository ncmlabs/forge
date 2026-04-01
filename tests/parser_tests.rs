// Parser integration tests: verify pest Pairs → AST transformation.

use forge::ast::*;
use forge::parser::parse;

// ── Test helpers ─────────────────────────────────────────────

/// Wrap a statement in a minimal task for isolated testing.
fn parse_task_with(body: &str) -> Program {
    let src = format!("task test_task\n  do\n    {}\n", body);
    parse(&src).unwrap_or_else(|e| panic!("parse failed:\n{}\nsource:\n{}", e, src))
}

/// Extract the first statement from a task's do block.
fn first_stmt(prog: &Program) -> &Stmt {
    match &prog.items[0].node {
        TopLevel::Task(t) => match &t.body.node {
            TaskBody::Do(stmts) => &stmts[0].node,
            _ => panic!("expected do block"),
        },
        _ => panic!("expected task"),
    }
}

/// Extract the expression from a bind statement.
fn bind_expr(stmt: &Stmt) -> &Expr {
    match stmt {
        Stmt::Bind(_, expr) => &expr.node,
        _ => panic!("expected bind, got {:?}", stmt),
    }
}

// ── Literal / Type tests ─────────────────────────────────────

#[test]
fn parse_number_literal() {
    let prog = parse_task_with("x = 42");
    let expr = bind_expr(first_stmt(&prog));
    match expr {
        Expr::NumberLit(n) => assert_eq!(*n, 42.0),
        _ => panic!("expected NumberLit, got {:?}", expr),
    }
}

#[test]
fn parse_float_literal() {
    let prog = parse_task_with("x = 0.95");
    let expr = bind_expr(first_stmt(&prog));
    match expr {
        Expr::NumberLit(n) => assert!((n - 0.95).abs() < f64::EPSILON),
        _ => panic!("expected NumberLit"),
    }
}

#[test]
fn parse_bool_literals() {
    let prog = parse_task_with("x = true");
    match bind_expr(first_stmt(&prog)) {
        Expr::BoolLit(true) => {}
        other => panic!("expected true, got {:?}", other),
    }

    let prog = parse_task_with("x = false");
    match bind_expr(first_stmt(&prog)) {
        Expr::BoolLit(false) => {}
        other => panic!("expected false, got {:?}", other),
    }
}

#[test]
fn parse_template_with_interpolation() {
    let prog = parse_task_with("x = \"hello {name}!\"");
    match bind_expr(first_stmt(&prog)) {
        Expr::Template(parts) => {
            assert_eq!(parts.len(), 3);
            match &parts[0].node {
                TemplatePart::Text(t) => assert_eq!(t, "hello "),
                _ => panic!("expected text part"),
            }
            match &parts[1].node {
                TemplatePart::Interp(e) => match &e.node {
                    Expr::Ident(name) => assert_eq!(name, "name"),
                    _ => panic!("expected ident in interp"),
                },
                _ => panic!("expected interp part"),
            }
            match &parts[2].node {
                TemplatePart::Text(t) => assert_eq!(t, "!"),
                _ => panic!("expected text part"),
            }
        }
        other => panic!("expected Template, got {:?}", other),
    }
}

#[test]
fn parse_template_decodes_escape_sequences() {
    let prog = parse_task_with(r#"x = "line 1\nline 2\t\"quoted\"\\done""#);
    match bind_expr(first_stmt(&prog)) {
        Expr::Template(parts) => {
            assert_eq!(parts.len(), 1);
            match &parts[0].node {
                TemplatePart::Text(t) => {
                    assert_eq!(t, "line 1\nline 2\t\"quoted\"\\done");
                }
                _ => panic!("expected text part"),
            }
        }
        other => panic!("expected Template, got {:?}", other),
    }
}

#[test]
fn parse_template_mixes_escapes_and_interpolation() {
    let prog = parse_task_with(r#"x = "x\n{name}\tend""#);
    match bind_expr(first_stmt(&prog)) {
        Expr::Template(parts) => {
            assert_eq!(parts.len(), 3);
            match &parts[0].node {
                TemplatePart::Text(t) => assert_eq!(t, "x\n"),
                _ => panic!("expected text part"),
            }
            match &parts[1].node {
                TemplatePart::Interp(e) => match &e.node {
                    Expr::Ident(name) => assert_eq!(name, "name"),
                    _ => panic!("expected ident in interp"),
                },
                _ => panic!("expected interp part"),
            }
            match &parts[2].node {
                TemplatePart::Text(t) => assert_eq!(t, "\tend"),
                _ => panic!("expected text part"),
            }
        }
        other => panic!("expected Template, got {:?}", other),
    }
}

#[test]
fn parse_builtin_and_custom_types() {
    let src = "task t\n  needs x: Text, y: Number, z: MyType\n  do\n    say x\n";
    let prog = parse(src).unwrap();
    match &prog.items[0].node {
        TopLevel::Task(t) => {
            assert_eq!(t.needs.len(), 3);
            assert!(matches!(&t.needs[0].node.type_name.node, TypeName::Text));
            assert!(matches!(&t.needs[1].node.type_name.node, TypeName::Number));
            match &t.needs[2].node.type_name.node {
                TypeName::Custom(name) => assert_eq!(name, "MyType"),
                _ => panic!("expected Custom type"),
            }
        }
        _ => panic!("expected task"),
    }
}

#[test]
fn parse_array_types() {
    let src = "task t\n  needs board: Text[9], items: Number[]\n  do\n    say board\n";
    let prog = parse(src).unwrap();
    match &prog.items[0].node {
        TopLevel::Task(t) => {
            match &t.needs[0].node.type_name.node {
                TypeName::Array(inner, Some(9)) => {
                    assert!(matches!(inner.as_ref(), TypeName::Text))
                }
                other => panic!("expected Text[9], got {:?}", other),
            }
            match &t.needs[1].node.type_name.node {
                TypeName::Array(inner, None) => {
                    assert!(matches!(inner.as_ref(), TypeName::Number))
                }
                other => panic!("expected Number[], got {:?}", other),
            }
        }
        _ => panic!("expected task"),
    }
}

// ── Expression tests ─────────────────────────────────────────

#[test]
fn parse_reason_expression() {
    let prog = parse_task_with("x = reason \"think about this\"");
    match bind_expr(first_stmt(&prog)) {
        Expr::Reason(inner) => match &inner.node {
            Expr::Template(parts) => match &parts[0].node {
                TemplatePart::Text(t) => assert_eq!(t, "think about this"),
                _ => panic!("expected text"),
            },
            _ => panic!("expected template"),
        },
        other => panic!("expected Reason, got {:?}", other),
    }
}

#[test]
fn parse_classify_expression() {
    let prog = parse_task_with("x = classify message into [\"buy\", \"sell\"]");
    match bind_expr(first_stmt(&prog)) {
        Expr::Classify(c) => {
            match &c.input.node {
                Expr::Ident(name) => assert_eq!(name, "message"),
                _ => panic!("expected ident input"),
            }
            assert_eq!(c.labels.len(), 2);
            assert_eq!(c.labels[0].node, "buy");
            assert_eq!(c.labels[1].node, "sell");
        }
        other => panic!("expected Classify, got {:?}", other),
    }
}

#[test]
fn parse_classify_labels_decode_escapes() {
    let prog = parse_task_with(r#"x = classify message into ["line 1\nline 2", "tab\tlabel"]"#);
    match bind_expr(first_stmt(&prog)) {
        Expr::Classify(c) => {
            assert_eq!(c.labels[0].node, "line 1\nline 2");
            assert_eq!(c.labels[1].node, "tab\tlabel");
        }
        other => panic!("expected Classify, got {:?}", other),
    }
}

#[test]
fn parse_search_expression() {
    let prog = parse_task_with("x = search \"find something\"");
    match bind_expr(first_stmt(&prog)) {
        Expr::Search(inner) => match &inner.node {
            Expr::Template(_) => {}
            _ => panic!("expected template"),
        },
        other => panic!("expected Search, got {:?}", other),
    }
}

#[test]
fn parse_try_or_expression() {
    let prog = parse_task_with("x = try search \"query\" or \"default\"");
    match bind_expr(first_stmt(&prog)) {
        Expr::TryOr(try_e, or_e) => {
            assert!(matches!(&try_e.node, Expr::Search(_)));
            assert!(matches!(&or_e.node, Expr::Template(_)));
        }
        other => panic!("expected TryOr, got {:?}", other),
    }
}

#[test]
fn parse_try_or_with_arithmetic() {
    let prog = parse_task_with("x = try a + b or c");
    match bind_expr(first_stmt(&prog)) {
        Expr::TryOr(try_e, or_e) => {
            assert!(matches!(&try_e.node, Expr::BinOp(_, _, _)));
            assert!(matches!(&or_e.node, Expr::Ident(_)));
        }
        other => panic!("expected TryOr, got {:?}", other),
    }
}

#[test]
fn parse_try_or_with_and_comparison() {
    let prog = parse_task_with("x = try a > 0 and b or fallback");
    match bind_expr(first_stmt(&prog)) {
        Expr::TryOr(try_e, or_e) => {
            assert!(matches!(&try_e.node, Expr::BinOp(_, _, _)));
            assert!(matches!(&or_e.node, Expr::Ident(_)));
        }
        other => panic!("expected TryOr, got {:?}", other),
    }
}

#[test]
fn parse_call_expression() {
    let prog = parse_task_with("x = greet(name: \"world\")");
    match bind_expr(first_stmt(&prog)) {
        Expr::Call(call) => {
            assert_eq!(call.name.node, "greet");
            assert_eq!(call.args.len(), 1);
            assert_eq!(call.args[0].node.label.as_ref().unwrap().node, "name");
        }
        other => panic!("expected Call, got {:?}", other),
    }
}

#[test]
fn parse_constructor_expression() {
    let prog = parse_task_with("x = Failure(\"oops\")");
    match bind_expr(first_stmt(&prog)) {
        Expr::Constructor(c) => {
            assert!(matches!(&c.type_name.node, TypeName::Failure));
            assert_eq!(c.args.len(), 1);
        }
        other => panic!("expected Constructor, got {:?}", other),
    }
}

#[test]
fn parse_field_and_glob_access() {
    let prog = parse_task_with("x = result.value");
    match bind_expr(first_stmt(&prog)) {
        Expr::FieldAccess(base, field) => {
            assert!(matches!(&base.node, Expr::Ident(n) if n == "result"));
            assert_eq!(field.node, "value");
        }
        other => panic!("expected FieldAccess, got {:?}", other),
    }

    let prog = parse_task_with("x = data.*");
    match bind_expr(first_stmt(&prog)) {
        Expr::GlobAccess(base) => {
            assert!(matches!(&base.node, Expr::Ident(n) if n == "data"));
        }
        other => panic!("expected GlobAccess, got {:?}", other),
    }
}

#[test]
fn parse_type_access() {
    let prog = parse_task_with("x = Intent.unknown");
    match bind_expr(first_stmt(&prog)) {
        Expr::TypeAccess(ty, field) => {
            assert!(matches!(&ty.node, TypeName::Intent));
            assert_eq!(field.node, "unknown");
        }
        other => panic!("expected TypeAccess, got {:?}", other),
    }
}

#[test]
fn parse_compose_expression() {
    let src = "task t\n  is extract >> summarize >> format\n";
    let prog = parse(src).unwrap();
    match &prog.items[0].node {
        TopLevel::Task(t) => match &t.body.node {
            TaskBody::Is(expr) => match &expr.node {
                Expr::Compose(parts) => {
                    assert_eq!(parts.len(), 3);
                    assert!(matches!(&parts[0].node, Expr::Ident(n) if n == "extract"));
                    assert!(matches!(&parts[1].node, Expr::Ident(n) if n == "summarize"));
                    assert!(matches!(&parts[2].node, Expr::Ident(n) if n == "format"));
                }
                other => panic!("expected Compose, got {:?}", other),
            },
            _ => panic!("expected is clause"),
        },
        _ => panic!("expected task"),
    }
}

#[test]
fn parse_fan_out_expression() {
    let prog = parse_task_with("x = (fast_check | deep_check | simple_check)");
    match bind_expr(first_stmt(&prog)) {
        Expr::FanOut(parts) => {
            assert_eq!(parts.len(), 3);
        }
        other => panic!("expected FanOut, got {:?}", other),
    }
}

#[test]
fn parse_operator_precedence_mul_before_add() {
    // 1 + 2 * 3 should parse as BinOp(1, Add, BinOp(2, Mul, 3))
    let prog = parse_task_with("x = 1 + 2 * 3");
    match bind_expr(first_stmt(&prog)) {
        Expr::BinOp(left, op, right) => {
            assert_eq!(op.node, BinOp::Add);
            assert!(matches!(&left.node, Expr::NumberLit(n) if *n == 1.0));
            match &right.node {
                Expr::BinOp(rl, rop, rr) => {
                    assert_eq!(rop.node, BinOp::Mul);
                    assert!(matches!(&rl.node, Expr::NumberLit(n) if *n == 2.0));
                    assert!(matches!(&rr.node, Expr::NumberLit(n) if *n == 3.0));
                }
                other => panic!("expected inner BinOp, got {:?}", other),
            }
        }
        other => panic!("expected BinOp, got {:?}", other),
    }
}

#[test]
fn parse_comparison_and_logical() {
    // x > 0 and y == true
    let prog = parse_task_with("x = a > 0 and b == true");
    match bind_expr(first_stmt(&prog)) {
        Expr::BinOp(left, op, right) => {
            assert_eq!(op.node, BinOp::And);
            match &left.node {
                Expr::BinOp(_, lop, _) => assert_eq!(lop.node, BinOp::Gt),
                _ => panic!("expected comparison"),
            }
            match &right.node {
                Expr::BinOp(_, rop, _) => assert_eq!(rop.node, BinOp::Eq),
                _ => panic!("expected comparison"),
            }
        }
        other => panic!("expected And BinOp, got {:?}", other),
    }
}

#[test]
fn parse_unary_not() {
    let prog = parse_task_with("x = not done");
    match bind_expr(first_stmt(&prog)) {
        Expr::UnaryOp(op, _) => assert_eq!(op.node, UnaryOp::Not),
        other => panic!("expected UnaryOp, got {:?}", other),
    }
}

#[test]
fn parse_index_expression() {
    let prog = parse_task_with("x = board[0]");
    match bind_expr(first_stmt(&prog)) {
        Expr::Index(base, idx) => {
            assert!(matches!(&base.node, Expr::Ident(n) if n == "board"));
            assert!(matches!(&idx.node, Expr::NumberLit(n) if *n == 0.0));
        }
        other => panic!("expected Index, got {:?}", other),
    }
}

#[test]
fn parse_method_call() {
    let prog = parse_task_with("x = items.count(active)");
    match bind_expr(first_stmt(&prog)) {
        Expr::MethodCall(base, method, args) => {
            assert!(matches!(&base.node, Expr::Ident(n) if n == "items"));
            assert_eq!(method.node, "count");
            assert_eq!(args.len(), 1);
        }
        other => panic!("expected MethodCall, got {:?}", other),
    }
}

#[test]
fn parse_array_literal() {
    let prog = parse_task_with("x = [1, 2, 3]");
    match bind_expr(first_stmt(&prog)) {
        Expr::ArrayLit(elems) => {
            assert_eq!(elems.len(), 3);
            assert!(matches!(&elems[0].node, Expr::NumberLit(n) if *n == 1.0));
        }
        other => panic!("expected ArrayLit, got {:?}", other),
    }
}

// ── Statement tests ──────────────────────────────────────────

#[test]
fn parse_give_statement() {
    let prog = parse_task_with("give result");
    match first_stmt(&prog) {
        Stmt::Give(expr, None) => {
            assert!(matches!(&expr.node, Expr::Ident(n) if n == "result"));
        }
        other => panic!("expected Give, got {:?}", other),
    }
}

#[test]
fn parse_say_statement() {
    let prog = parse_task_with("say \"hello\"");
    match first_stmt(&prog) {
        Stmt::Say(expr) => {
            assert!(matches!(&expr.node, Expr::Template(_)));
        }
        other => panic!("expected Say, got {:?}", other),
    }
}

#[test]
fn parse_when_block_stmt() {
    let src = "task t\n  do\n    when x.sure -> give x\n    when x.unsure -> say \"hmm\"\n    else -> give \"unknown\"\n";
    let prog = parse(src).unwrap();
    match first_stmt(&prog) {
        Stmt::When(block) => {
            assert_eq!(block.clauses.len(), 2);
            assert!(matches!(
                &block.clauses[0].node.predicate.node.level.node,
                ConfLevel::Sure(None)
            ));
            assert!(matches!(
                &block.clauses[1].node.predicate.node.level.node,
                ConfLevel::Unsure
            ));
            assert!(block.else_body.is_some());
        }
        other => panic!("expected When, got {:?}", other),
    }
}

#[test]
fn parse_when_sure_with_threshold() {
    let src = "task t\n  do\n    when r.sure(above: 0.9) -> give r\n";
    let prog = parse(src).unwrap();
    match first_stmt(&prog) {
        Stmt::When(block) => {
            match &block.clauses[0].node.predicate.node.level.node {
                ConfLevel::Sure(Some(t)) => assert!((t - 0.9).abs() < f64::EPSILON),
                other => panic!("expected Sure with threshold, got {:?}", other),
            }
        }
        other => panic!("expected When, got {:?}", other),
    }
}

#[test]
fn parse_match_statement() {
    let src = "task t\n  do\n    match status\n      Active -> say \"active\"\n      Inactive(x) -> say x\n      _ -> say \"other\"\n";
    let prog = parse(src).unwrap();
    match first_stmt(&prog) {
        Stmt::Match(block) => {
            assert!(matches!(&block.subject.node, Expr::Ident(n) if n == "status"));
            assert_eq!(block.arms.len(), 3);
            assert!(matches!(&block.arms[0].node.pattern.node, Pattern::Constructor(n, _) if n == "Active"));
            assert!(matches!(&block.arms[1].node.pattern.node, Pattern::Constructor(n, args) if n == "Inactive" && args.len() == 1));
            assert!(matches!(&block.arms[2].node.pattern.node, Pattern::Wildcard));
        }
        other => panic!("expected Match, got {:?}", other),
    }
}

#[test]
fn parse_if_else_statement() {
    let src = "task t\n  do\n    if x > 0\n      say \"positive\"\n    else if x == 0\n      say \"zero\"\n    else\n      say \"negative\"\n";
    let prog = parse(src).unwrap();
    match first_stmt(&prog) {
        Stmt::IfElse(block) => {
            match &block.condition.node {
                Expr::BinOp(_, op, _) => assert_eq!(op.node, BinOp::Gt),
                _ => panic!("expected comparison"),
            }
            assert_eq!(block.then_body.len(), 1);
            assert_eq!(block.else_ifs.len(), 1);
            assert!(block.else_body.is_some());
        }
        other => panic!("expected IfElse, got {:?}", other),
    }
}

#[test]
fn parse_for_loop_stmt() {
    let src = "task t\n  do\n    for item in items\n      say item\n";
    let prog = parse(src).unwrap();
    match first_stmt(&prog) {
        Stmt::For(f) => {
            assert_eq!(f.binding.node, "item");
            assert!(matches!(&f.iterable.node, Expr::Ident(n) if n == "items"));
            assert_eq!(f.body.len(), 1);
        }
        other => panic!("expected For, got {:?}", other),
    }
}

#[test]
fn parse_memory_update() {
    let src = "agent a\n  memory\n    count: Number\n  on tick\n    memory.count = memory.count + 1\n";
    let prog = parse(src).unwrap();
    match &prog.items[0].node {
        TopLevel::Agent(a) => match &a.handlers[0].node.body[0].node {
            Stmt::MemoryUpdate(field, None, _) => assert_eq!(field.node, "count"),
            other => panic!("expected MemoryUpdate, got {:?}", other),
        },
        _ => panic!("expected agent"),
    }
}

#[test]
fn parse_memory_update_with_index() {
    let src = "agent a\n  memory\n    board: Text[9]\n  on move(cell: Number)\n    memory.board[cell] = \"X\"\n";
    let prog = parse(src).unwrap();
    match &prog.items[0].node {
        TopLevel::Agent(a) => match &a.handlers[0].node.body[0].node {
            Stmt::MemoryUpdate(field, Some(_idx), _) => assert_eq!(field.node, "board"),
            other => panic!("expected indexed MemoryUpdate, got {:?}", other),
        },
        _ => panic!("expected agent"),
    }
}

// ── Declaration tests ────────────────────────────────────────

#[test]
fn parse_use_declaration() {
    let src = "use\n  llm.reason\n  llm.classify\n";
    let prog = parse(src).unwrap();
    match &prog.items[0].node {
        TopLevel::Use(u) => {
            assert_eq!(u.capabilities.len(), 2);
            assert_eq!(u.capabilities[0].node, "llm.reason");
            assert_eq!(u.capabilities[1].node, "llm.classify");
        }
        other => panic!("expected Use, got {:?}", other),
    }
}

#[test]
fn parse_task_do_variant() {
    let src = "task greet\n  needs name: Text\n  gives Text\n  do\n    say \"hi\"\n";
    let prog = parse(src).unwrap();
    match &prog.items[0].node {
        TopLevel::Task(t) => {
            assert_eq!(t.name.node, "greet");
            assert_eq!(t.needs.len(), 1);
            assert!(t.gives.is_some());
            assert!(matches!(&t.body.node, TaskBody::Do(stmts) if stmts.len() == 1));
        }
        other => panic!("expected Task, got {:?}", other),
    }
}

#[test]
fn parse_task_is_variant() {
    let src = "task pipeline\n  is extract >> transform >> load\n";
    let prog = parse(src).unwrap();
    match &prog.items[0].node {
        TopLevel::Task(t) => {
            assert_eq!(t.name.node, "pipeline");
            assert!(matches!(&t.body.node, TaskBody::Is(_)));
        }
        other => panic!("expected Task, got {:?}", other),
    }
}

#[test]
fn parse_task_with_if_fails() {
    let src = "task risky\n  do\n    say \"trying\"\n  if fails\n    say \"failed\"\n";
    let prog = parse(src).unwrap();
    match &prog.items[0].node {
        TopLevel::Task(t) => {
            assert!(t.if_fails.is_some());
            assert_eq!(t.if_fails.as_ref().unwrap().len(), 1);
        }
        other => panic!("expected Task, got {:?}", other),
    }
}

#[test]
fn parse_pure_declaration() {
    let src = "pure add\n  needs a: Number, b: Number\n  gives Number\n  do\n    give a + b\n";
    let prog = parse(src).unwrap();
    match &prog.items[0].node {
        TopLevel::Pure(p) => {
            assert_eq!(p.name.node, "add");
            assert_eq!(p.needs.len(), 2);
            assert!(p.gives.is_some());
            assert_eq!(p.body.len(), 1);
        }
        other => panic!("expected Pure, got {:?}", other),
    }
}

#[test]
fn parse_event_declaration() {
    let src = "event UserJoined\n  user: Text\n  room: Text\n";
    let prog = parse(src).unwrap();
    match &prog.items[0].node {
        TopLevel::Event(e) => {
            assert_eq!(e.name.node, "UserJoined");
            assert_eq!(e.fields.len(), 2);
            assert_eq!(e.fields[0].node.name, "user");
            assert_eq!(e.fields[1].node.name, "room");
        }
        other => panic!("expected Event, got {:?}", other),
    }
}

#[test]
fn parse_states_declaration() {
    let src = "states Phase\n  idle -> active when ready\n  active -> done\n";
    let prog = parse(src).unwrap();
    match &prog.items[0].node {
        TopLevel::States(s) => {
            assert_eq!(s.name.node, "Phase");
            assert_eq!(s.transitions.len(), 2);
            assert_eq!(s.transitions[0].node.from.node, "idle");
            assert_eq!(s.transitions[0].node.to.node, "active");
            assert!(s.transitions[0].node.condition.is_some());
            assert!(s.transitions[1].node.condition.is_none());
        }
        other => panic!("expected States, got {:?}", other),
    }
}

#[test]
fn parse_type_definition() {
    let src = "type Result\n  value: Text\n  code: Number\n";
    let prog = parse(src).unwrap();
    match &prog.items[0].node {
        TopLevel::TypeDef(t) => {
            assert_eq!(t.name.node, "Result");
            assert_eq!(t.fields.len(), 2);
        }
        other => panic!("expected TypeDef, got {:?}", other),
    }
}

#[test]
fn parse_endpoint_declaration() {
    let src = "endpoint handle(req: Text) -> Text\n  give req\n";
    let prog = parse(src).unwrap();
    match &prog.items[0].node {
        TopLevel::Endpoint(e) => {
            assert_eq!(e.name.node, "handle");
            assert_eq!(e.params.len(), 1);
            assert!(e.return_type.is_some());
            assert_eq!(e.body.len(), 1);
        }
        other => panic!("expected Endpoint, got {:?}", other),
    }
}

#[test]
fn parse_flow_declaration() {
    let src = "flow pipeline\n  needs input: Text\n  gives Report\n\n  stage extract\n    result = reason input\n    give result\n\n  stage transform\n    needs extract.result\n    give extract.result\n";
    let prog = parse(src).unwrap();
    match &prog.items[0].node {
        TopLevel::Flow(f) => {
            assert_eq!(f.name.node, "pipeline");
            assert_eq!(f.needs.len(), 1);
            assert!(f.gives.is_some());
            assert_eq!(f.stages.len(), 2);
            assert_eq!(f.stages[0].node.name.node, "extract");
            assert_eq!(f.stages[1].node.name.node, "transform");
            assert_eq!(f.stages[1].node.needs.len(), 1);
        }
        other => panic!("expected Flow, got {:?}", other),
    }
}

#[test]
fn parse_agent_declaration() {
    let src = "agent helper\n  lifecycle: Phase\n  memory\n    count: Number\n  timer idle_check: 30s\n  subscribe Alert where level == \"high\"\n\n  on message(text: Text)\n    requires text != \"\" on fail: silent\n    say text\n    memory.count = memory.count + 1\n\n  if stuck\n    escalate to supervisor\n";
    let prog = parse(src).unwrap();
    match &prog.items[0].node {
        TopLevel::Agent(a) => {
            assert_eq!(a.name.node, "helper");
            assert!(a.lifecycle.is_some());
            assert_eq!(a.memory.len(), 1);
            assert_eq!(a.timers.len(), 1);
            assert_eq!(a.timers[0].node.name.node, "idle_check");
            assert_eq!(a.subscriptions.len(), 1);
            assert_eq!(a.subscriptions[0].node.event_name.node, "Alert");
            assert!(a.subscriptions[0].node.filter.is_some());
            assert_eq!(a.handlers.len(), 1);
            assert_eq!(a.handlers[0].node.requires.len(), 1);
            assert!(a.stuck_policy.is_some());
        }
        other => panic!("expected Agent, got {:?}", other),
    }
}

#[test]
fn parse_pool_declaration() {
    let src = "pool workers\n  workers: helper * 5\n  strategy: majority\n  timeout: 30s\n  fallback: default_handler\n";
    let prog = parse(src).unwrap();
    match &prog.items[0].node {
        TopLevel::Pool(p) => {
            assert_eq!(p.name.node, "workers");
            assert_eq!(p.worker_type.node, "helper");
            assert_eq!(p.worker_count.node, 5.0);
            assert!(matches!(&p.strategy.node, PoolStrategy::Majority));
            assert!(p.timeout.is_some());
            assert!(p.fallback.is_some());
        }
        other => panic!("expected Pool, got {:?}", other),
    }
}

#[test]
fn parse_contract_declaration() {
    let src = "contract Greeter\n  can greet(name: Text) -> Text\n  can farewell(name: Text) -> Text\n";
    let prog = parse(src).unwrap();
    match &prog.items[0].node {
        TopLevel::Contract(c) => {
            assert_eq!(c.name.node, "Greeter");
            assert_eq!(c.methods.len(), 2);
            assert_eq!(c.methods[0].node.name, "greet");
            assert_eq!(c.methods[1].node.name, "farewell");
        }
        other => panic!("expected Contract, got {:?}", other),
    }
}

#[test]
fn parse_system_declaration() {
    let src = "system app\n  use\n    svc: my_service\n    db: my_db\n  svc >> db\n";
    let prog = parse(src).unwrap();
    match &prog.items[0].node {
        TopLevel::System(s) => {
            assert_eq!(s.name.node, "app");
            assert_eq!(s.bindings.len(), 2);
            assert_eq!(s.bindings[0].node.alias, "svc");
            assert_eq!(s.bindings[0].node.target, "my_service");
            assert_eq!(s.wiring.len(), 1);
        }
        other => panic!("expected System, got {:?}", other),
    }
}

#[test]
fn parse_fn_main_declaration() {
    let src = "fn main\n  say \"hello\"\n";
    let prog = parse(src).unwrap();
    match &prog.items[0].node {
        TopLevel::FnMain(f) => {
            assert_eq!(f.body.len(), 1);
        }
        other => panic!("expected FnMain, got {:?}", other),
    }
}

#[test]
fn parse_boundary_directive() {
    let src = "#! boundary: server\ntask t\n  do\n    say \"hi\"\n";
    let prog = parse(src).unwrap();
    assert!(prog.boundary.is_some());
    assert_eq!(
        prog.boundary.unwrap().node.kind.node,
        BoundaryKind::Server
    );
}

// ── Full file tests ──────────────────────────────────────────

#[test]
fn parse_hello_forge_file() {
    let src = std::fs::read_to_string("examples/hello.forge").unwrap();
    let prog = parse(&src).unwrap();
    assert_eq!(prog.items.len(), 2);
    match &prog.items[0].node {
        TopLevel::Task(t) => {
            assert_eq!(t.name.node, "greet");
            assert_eq!(t.needs.len(), 1);
            assert!(matches!(&t.body.node, TaskBody::Do(stmts) if stmts.len() == 1));
        }
        _ => panic!("expected task"),
    }
    assert!(matches!(&prog.items[1].node, TopLevel::FnMain(_)));
}

#[test]
fn parse_classify_forge_file() {
    let src = std::fs::read_to_string("examples/classify.forge").unwrap();
    let prog = parse(&src).unwrap();
    assert_eq!(prog.items.len(), 3);
    assert!(matches!(&prog.items[0].node, TopLevel::Use(_)));
    match &prog.items[1].node {
        TopLevel::Task(t) => {
            assert_eq!(t.name.node, "classify_intent");
            match &t.body.node {
                TaskBody::Do(stmts) => {
                    assert_eq!(stmts.len(), 2);
                    assert!(matches!(&stmts[0].node, Stmt::Bind(_, _)));
                    assert!(matches!(&stmts[1].node, Stmt::When(_)));
                }
                _ => panic!("expected do block"),
            }
        }
        _ => panic!("expected task"),
    }
    assert!(matches!(&prog.items[2].node, TopLevel::FnMain(_)));
}

#[test]
fn parse_room_agent_forge_file() {
    let src = std::fs::read_to_string("examples/tictactoe/room_agent.forge").unwrap();
    let prog = parse(&src).unwrap();
    assert!(prog.boundary.is_some());
    // event, event, states, type, agent = 5 items
    assert!(prog.items.len() >= 4);
}

#[test]
fn parse_platform_forge_file() {
    let src = std::fs::read_to_string("examples/tictactoe/platform.forge").unwrap();
    let prog = parse(&src).unwrap();
    // use, pure, pure, contract, pool, system = 6 items
    assert!(prog.items.len() >= 5);
}

// ── Error tests ──────────────────────────────────────────────

#[test]
fn parse_error_includes_line_col() {
    let result = parse("task\n");
    assert!(result.is_err());
    let err = result.unwrap_err();
    // Verify it's a Syntax variant with span info
    match &err {
        forge::parser::ParseError::Syntax { span_start, span_end, message } => {
            assert!(*span_start <= *span_end, "span should be valid");
            assert!(!message.is_empty(), "should have a message");
        }
        other => panic!("expected Syntax error, got: {:?}", other),
    }
}

#[test]
fn parse_error_on_invalid_syntax() {
    let result = parse("this is not valid forge");
    assert!(result.is_err());
}

// ── Span preservation tests ─────────────────────────────────

#[test]
fn spans_are_preserved() {
    let src = "task greet\n  do\n    say \"hi\"\n";
    let prog = parse(src).unwrap();
    let task_span = &prog.items[0].span;
    assert_eq!(task_span.start, 0);
    assert!(task_span.end > 0);

    match &prog.items[0].node {
        TopLevel::Task(t) => {
            assert_eq!(&src[t.name.span.start..t.name.span.end], "greet");
        }
        _ => panic!("expected task"),
    }
}
