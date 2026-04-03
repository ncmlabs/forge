// FORGE pool executor integration tests — issue #12

use std::sync::Arc;

use forge::ast::*;
use forge::llm::providers::mock::MockProvider;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::confidence::{ConfidentValue, Value};
use forge::runtime::pool::PoolExecutor;
use forge::types::ConfidenceSource;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn spanned<T>(node: T) -> Spanned<T> {
    Spanned::new(node, Span { start: 0, end: 0 })
}

fn mock_registry_with_sequence(responses: Vec<String>) -> Arc<ProviderRegistry> {
    let mock = MockProvider::new("mock").with_responses_sequence(responses);
    let mut reg = ProviderRegistry::new("mock");
    reg.register("mock", Arc::new(mock));
    Arc::new(reg)
}

fn mock_registry_default(response: &str) -> Arc<ProviderRegistry> {
    let mock = MockProvider::new("mock").with_default(response);
    let mut reg = ProviderRegistry::new("mock");
    reg.register("mock", Arc::new(mock));
    Arc::new(reg)
}

fn text_param(name: &str) -> Spanned<Param> {
    spanned(Param {
        name: name.to_string(),
        type_name: spanned(TypeName::Text),
    })
}

fn output_text() -> Spanned<OutputType> {
    spanned(OutputType {
        types: vec![spanned(TypeName::Text)],
    })
}

fn str_expr(s: &str) -> Spanned<Expr> {
    spanned(Expr::Template(vec![spanned(TemplatePart::Text(
        s.to_string(),
    ))]))
}

/// Build a simple task that uses `reason` on the input and gives the result.
fn reason_task(name: &str) -> Spanned<TopLevel> {
    spanned(TopLevel::Task(TaskDecl {
        name: spanned(name.to_string()),
        needs: vec![text_param("input")],
        gives: Some(output_text()),
        body: spanned(TaskBody::Do(vec![
            // result = reason "{input}"
            spanned(Stmt::Bind(
                spanned("result".to_string()),
                spanned(Expr::Reason(Box::new(spanned(Expr::Template(vec![
                    spanned(TemplatePart::Interp(Box::new(spanned(Expr::Ident(
                        "input".to_string(),
                    ))))),
                ]))))),
            )),
            // give result
            spanned(Stmt::Give(
                spanned(Expr::Ident("result".to_string())),
                vec![],
            )),
        ])),
        if_fails: None,
    }))
}

/// Build a simple fallback task that returns a fixed string.
fn fallback_task(name: &str) -> Spanned<TopLevel> {
    spanned(TopLevel::Task(TaskDecl {
        name: spanned(name.to_string()),
        needs: vec![text_param("input")],
        gives: Some(output_text()),
        body: spanned(TaskBody::Is(Box::new(str_expr("fallback result")))),
        if_fails: None,
    }))
}

fn pool_decl_node(
    name: &str,
    worker_type: &str,
    count: f64,
    strategy: PoolStrategy,
    timeout: Option<Duration>,
    fallback: Option<&str>,
) -> PoolDecl {
    PoolDecl {
        name: spanned(name.to_string()),
        worker_type: spanned(worker_type.to_string()),
        worker_count: spanned(count),
        strategy: spanned(strategy),
        timeout: timeout.map(spanned),
        fallback: fallback.map(|f| spanned(f.to_string())),
    }
}

fn program_with(items: Vec<Spanned<TopLevel>>) -> Program {
    Program {
        boundary: None,
        items,
    }
}

fn text_arg(s: &str) -> ConfidentValue {
    ConfidentValue::deterministic(Value::Text(s.to_string()))
}

// ── Fastest strategy ─────────────────────────────────────────────────────────

#[tokio::test]
async fn fastest_returns_result() {
    let registry = mock_registry_with_sequence(vec![
        "answer one".to_string(),
        "answer two".to_string(),
        "answer three".to_string(),
    ]);

    let task = reason_task("checker");
    let program = program_with(vec![task]);
    let decl = pool_decl_node("pool", "checker", 3.0, PoolStrategy::Fastest, None, None);

    let pool = PoolExecutor::new(decl, &program, registry, None).unwrap();
    let result = pool
        .send("check", vec![text_arg("test input")])
        .await
        .unwrap();

    assert!(matches!(result.value, Value::Text(_)));
}

#[tokio::test]
async fn fastest_unknown_worker_type_errors() {
    let registry = mock_registry_default("response");
    let program = program_with(vec![]);
    let decl = pool_decl_node(
        "pool",
        "missing_task",
        3.0,
        PoolStrategy::Fastest,
        None,
        None,
    );

    let result = PoolExecutor::new(decl, &program, registry, None);
    assert!(result.is_err());
}

// ── All strategy ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn all_strategy_collects_results() {
    let registry = mock_registry_with_sequence(vec![
        "result a".to_string(),
        "result b".to_string(),
        "result c".to_string(),
    ]);

    let task = reason_task("worker");
    let program = program_with(vec![task]);
    let decl = pool_decl_node("pool", "worker", 3.0, PoolStrategy::All, None, None);

    let pool = PoolExecutor::new(decl, &program, registry, None).unwrap();
    let result = pool.send("run", vec![text_arg("input")]).await.unwrap();

    match &result.value {
        Value::Array(items) => {
            assert_eq!(items.len(), 3);
            for item in items {
                assert!(matches!(item.value, Value::Text(_)));
            }
        }
        other => panic!("expected Array, got {:?}", other),
    }
}

// ── Majority strategy ────────────────────────────────────────────────────────

#[tokio::test]
async fn majority_finds_consensus() {
    // 2 out of 3 agree
    let registry = mock_registry_with_sequence(vec![
        "yes it is true".to_string(),
        "yes it is true".to_string(),
        "no it is false".to_string(),
    ]);

    let task = reason_task("checker");
    let program = program_with(vec![task]);
    let decl = pool_decl_node("pool", "checker", 3.0, PoolStrategy::Majority, None, None);

    let pool = PoolExecutor::new(decl, &program, registry, None).unwrap();
    let result = pool.send("check", vec![text_arg("claim")]).await.unwrap();

    // Should return consensus answer
    assert!(matches!(result.value, Value::Text(ref s) if s.contains("yes")));
    // Source should be ConsensusAgreement
    assert!(matches!(
        result.source,
        ConfidenceSource::ConsensusAgreement(_)
    ));
}

#[tokio::test]
async fn majority_no_consensus_returns_conflicted() {
    let registry = mock_registry_with_sequence(vec![
        "alpha".to_string(),
        "beta".to_string(),
        "gamma".to_string(),
    ]);

    let task = reason_task("checker");
    let program = program_with(vec![task]);
    let decl = pool_decl_node("pool", "checker", 3.0, PoolStrategy::Majority, None, None);

    let pool = PoolExecutor::new(decl, &program, registry, None).unwrap();
    let result = pool.send("check", vec![text_arg("claim")]).await.unwrap();

    // With all different answers, agreement is 1/3 ≈ 0.33, which is < 0.6 → conflicted
    assert!(result.conflicted());
    assert!(matches!(result.source, ConfidenceSource::ConsensusAgreement(a) if a < 0.6));
}

#[tokio::test]
async fn majority_disagreement_still_returns_value() {
    // Even with fallback declared, majority disagreement returns a conflicted value
    // (fallback is only for hard failures like all workers erroring or timeout)
    let registry = mock_registry_with_sequence(vec![
        "alpha".to_string(),
        "beta".to_string(),
        "gamma".to_string(),
    ]);

    let task = reason_task("checker");
    let fb = fallback_task("fallback_checker");
    let program = program_with(vec![task, fb]);
    let decl = pool_decl_node(
        "pool",
        "checker",
        3.0,
        PoolStrategy::Majority,
        None,
        Some("fallback_checker"),
    );

    let pool = PoolExecutor::new(decl, &program, registry, None).unwrap();
    let result = pool.send("check", vec![text_arg("claim")]).await.unwrap();

    // Returns a conflicted value, not the fallback
    assert!(result.conflicted());
}

// ── Quorum strategy ──────────────────────────────────────────────────────────

#[tokio::test]
async fn quorum_threshold_met() {
    // 3 workers, quorum(2) — 2 agree
    let registry = mock_registry_with_sequence(vec![
        "the answer is yes".to_string(),
        "the answer is yes".to_string(),
        "the answer is no".to_string(),
    ]);

    let task = reason_task("checker");
    let program = program_with(vec![task]);
    let decl = pool_decl_node(
        "pool",
        "checker",
        3.0,
        PoolStrategy::Quorum(2.0),
        None,
        None,
    );

    let pool = PoolExecutor::new(decl, &program, registry, None).unwrap();
    let result = pool.send("check", vec![text_arg("claim")]).await.unwrap();

    assert!(matches!(
        result.source,
        ConfidenceSource::ConsensusAgreement(_)
    ));
}

#[tokio::test]
async fn quorum_threshold_not_met() {
    // 3 workers, quorum(3) — all must agree but they don't
    let registry =
        mock_registry_with_sequence(vec!["yes".to_string(), "yes".to_string(), "no".to_string()]);

    let task = reason_task("checker");
    let program = program_with(vec![task]);
    let decl = pool_decl_node(
        "pool",
        "checker",
        3.0,
        PoolStrategy::Quorum(3.0),
        None,
        None,
    );

    let pool = PoolExecutor::new(decl, &program, registry, None).unwrap();
    let result = pool.send("check", vec![text_arg("claim")]).await;

    assert!(result.is_err());
}

// ── Timeout ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn timeout_does_not_interrupt_fast_workers() {
    // Mock responds instantly, generous timeout — should succeed
    let registry = mock_registry_default("quick response");

    let task = reason_task("worker");
    let program = program_with(vec![task]);
    let timeout = Duration {
        value: 5,
        unit: DurationUnit::Seconds,
    };
    let decl = pool_decl_node(
        "pool",
        "worker",
        2.0,
        PoolStrategy::Fastest,
        Some(timeout),
        None,
    );

    let pool = PoolExecutor::new(decl, &program, registry, None).unwrap();
    let result = pool.send("run", vec![text_arg("input")]).await.unwrap();

    assert!(matches!(result.value, Value::Text(_)));
}

// ── First(n) strategy ────────────────────────────────────────────────────────

#[tokio::test]
async fn first_n_returns_after_enough_workers() {
    let registry = mock_registry_with_sequence(vec![
        "fast answer".to_string(),
        "slow answer".to_string(),
        "slowest answer".to_string(),
    ]);

    let task = reason_task("worker");
    let program = program_with(vec![task]);
    let decl = pool_decl_node("pool", "worker", 3.0, PoolStrategy::First(2.0), None, None);

    let pool = PoolExecutor::new(decl, &program, registry, None).unwrap();
    let result = pool.send("run", vec![text_arg("input")]).await.unwrap();

    assert!(matches!(result.value, Value::Text(_)));
}

// ── Executor integration ─────────────────────────────────────────────────────

#[tokio::test]
async fn pool_via_executor_direct_call() {
    use forge::runtime::executor::TaskExecutor;

    let registry = mock_registry_with_sequence(vec![
        "consensus answer".to_string(),
        "consensus answer".to_string(),
        "different answer".to_string(),
    ]);

    let task = reason_task("checker");
    let pool_item = spanned(TopLevel::Pool(pool_decl_node(
        "my_pool",
        "checker",
        3.0,
        PoolStrategy::Majority,
        None,
        None,
    )));

    // fn main: result = my_pool("check", "input") ; say result
    let main = spanned(TopLevel::FnMain(FnMainDecl {
        body: vec![
            spanned(Stmt::Bind(
                spanned("result".to_string()),
                spanned(Expr::Call(CallExpr {
                    name: spanned("my_pool".to_string()),
                    args: vec![
                        spanned(CallArg {
                            label: None,
                            value: str_expr("check"),
                        }),
                        spanned(CallArg {
                            label: None,
                            value: str_expr("test claim"),
                        }),
                    ],
                })),
            )),
            spanned(Stmt::Say(spanned(Expr::Ident("result".to_string())))),
        ],
    }));

    let program = program_with(vec![task, pool_item, main]);
    let executor = TaskExecutor::new(program, registry, None);
    let result = executor.run().await.unwrap();
    // run() returns a ConfidentValue; say outputs to the internal buffer
    // The fact that it succeeded without error is the key assertion
    assert!(matches!(result.value, Value::Unit | Value::Text(_)));
}

#[tokio::test]
async fn pool_via_send_method_call() {
    use forge::runtime::executor::TaskExecutor;

    let registry = mock_registry_default("agreed answer");

    let task = reason_task("checker");
    let pool_item = spanned(TopLevel::Pool(pool_decl_node(
        "my_pool",
        "checker",
        2.0,
        PoolStrategy::Fastest,
        None,
        None,
    )));

    // fn main: result = my_pool.send("run", "input") ; say result
    let main = spanned(TopLevel::FnMain(FnMainDecl {
        body: vec![
            spanned(Stmt::Bind(
                spanned("result".to_string()),
                spanned(Expr::MethodCall(
                    Box::new(spanned(Expr::Ident("my_pool".to_string()))),
                    spanned("send".to_string()),
                    vec![
                        spanned(CallArg {
                            label: None,
                            value: str_expr("run"),
                        }),
                        spanned(CallArg {
                            label: None,
                            value: str_expr("some input"),
                        }),
                    ],
                )),
            )),
            spanned(Stmt::Say(spanned(Expr::Ident("result".to_string())))),
        ],
    }));

    let program = program_with(vec![task, pool_item, main]);
    let executor = TaskExecutor::new(program, registry, None);
    let result = executor.run().await.unwrap();
    assert!(matches!(result.value, Value::Unit | Value::Text(_)));
}
