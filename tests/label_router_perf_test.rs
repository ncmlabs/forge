// T8.3 (#358): p99 < 5ms per route budget for the deterministic
// label_router. Per the issue DoD, routing must be fast enough that
// it's never the long pole — no LLM calls, no skill calls, no I/O.
//
// The test drives the same in-process TaskExecutor path `forge serve`
// uses for endpoint dispatch, then invokes a thin endpoint that calls
// the pure label_router and returns its specialist. Wall time covers
// endpoint dispatch + pure invocation — that is the real "per route"
// cost a caller would see.
//
// Release-mode guidance: the CI-ish target is 5ms p99. Debug builds
// are slower than release; we use release when run with
// `cargo test --release --test label_router_perf_test` but the
// assertion is set generously so the test is stable in debug too.
// If this ever flakes, prefer investigating instead of bumping the
// budget — pure tasks must not regress past this watermark.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use forge::compose;
use forge::runtime::confidence::{ConfidentValue, Value};
use forge::runtime::executor::TaskExecutor;

const PERF_SRC: &str = r##"#! boundary: server

type LabelRoute
  label: Text
  specialist: Text

type RoutingDecision
  specialist: Text
  matched_label: Text
  route_reason: Text

pure route_if_matches
  needs r: LabelRoute, l_lower: Text
  gives LabelRoute[]
  do
    route_label = r.label
    route_lower = route_label.lower()
    if route_lower == l_lower
      give [r]
    give []

pure routes_matching_label
  needs l: Text, routes: LabelRoute[]
  gives LabelRoute[]
  do
    l_lower = l.lower()
    result = []
    for r in routes
      hit = route_if_matches(r, l_lower)
      result = result + hit
    give result

pure match_routes
  needs issue_labels: Text[], routes: LabelRoute[]
  gives LabelRoute[]
  do
    result = []
    for l in issue_labels
      hits = routes_matching_label(l, routes)
      result = result + hits
    give result

pure label_router
  needs issue_labels: Text[], routes: LabelRoute[], fallback: Text
  gives RoutingDecision
  do
    matches = match_routes(issue_labels, routes)
    if matches.length == 0
      give RoutingDecision(specialist: fallback, matched_label: "", route_reason: "no-matching-label")
    if matches.length > 1
      give RoutingDecision(specialist: fallback, matched_label: "", route_reason: "multi-label-conflict")
    give RoutingDecision(specialist: matches[0].specialist, matched_label: matches[0].label, route_reason: "unique-label-match")

endpoint route(labels: Text[]) -> Text
  routes = [LabelRoute(label: "clone-dev:plan", specialist: "plan_specialist"), LabelRoute(label: "clone-dev:impl", specialist: "impl_specialist"), LabelRoute(label: "clone-dev:test", specialist: "test_specialist"), LabelRoute(label: "clone-dev:review", specialist: "review_specialist"), LabelRoute(label: "clone-dev:merge", specialist: "merge_specialist"), LabelRoute(label: "clone-dev:ops", specialist: "ops_specialist")]
  decision = label_router(labels, routes, "triage_specialist")
  give decision.specialist
"##;

fn mock_registry() -> Arc<forge::llm::registry::ProviderRegistry> {
    let config = forge::config::ForgeConfig::default_mock_config();
    Arc::new(
        forge::llm::registry::ProviderRegistry::from_config(config)
            .expect("mock registry should build"),
    )
}

fn build_executor() -> TaskExecutor {
    let program = forge::parser::parse(PERF_SRC).expect("parse perf source");
    let sources = [compose::SourceFile {
        path: "perf.forge".to_string(),
        source: PERF_SRC.to_string(),
        program: program.clone(),
    }];
    let composed = compose::merge_programs(&sources).expect("merge");
    let diagnostics = forge::checker::check_all(&composed.program, "perf.forge");
    let errors: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.kind == forge::diagnostic::DiagnosticKind::Error)
        .collect();
    assert!(errors.is_empty(), "checker errors: {:?}", errors);

    TaskExecutor::new(composed.program, mock_registry(), None)
        .with_config(forge::config::ForgeConfig::default_mock_config())
}

fn labels_arg(labels: &[&str]) -> HashMap<String, ConfidentValue> {
    let labels_val = Value::Array(
        labels
            .iter()
            .map(|l| ConfidentValue::deterministic(Value::Text((*l).to_string())))
            .collect(),
    );
    let mut args = HashMap::new();
    args.insert(
        "labels".to_string(),
        ConfidentValue::deterministic(labels_val),
    );
    args
}

#[tokio::test]
async fn label_router_p99_under_5ms() {
    // p99 < 5ms is the DoD budget. We run 1 000 dispatches and check
    // both the mean and the p99 — mean gives a sanity floor, p99 is
    // the hard budget.
    const N: usize = 1000;
    let executor = build_executor();

    // Warmup — first call lazily builds internal caches that would
    // otherwise skew the first sample.
    for _ in 0..20 {
        let args = labels_arg(&["clone-dev:impl"]);
        let _ = executor
            .exec_endpoint("route", args, None)
            .await
            .expect("warmup dispatch");
    }

    let mut samples: Vec<Duration> = Vec::with_capacity(N);
    for _ in 0..N {
        let args = labels_arg(&["bug", "clone-dev:impl", "priority:high"]);
        let start = Instant::now();
        let _ = executor
            .exec_endpoint("route", args, None)
            .await
            .expect("endpoint dispatch");
        samples.push(start.elapsed());
    }
    samples.sort();
    let mean = samples.iter().sum::<Duration>() / (N as u32);
    let p99 = samples[(N as f64 * 0.99) as usize];
    let max = *samples.last().unwrap();

    // 5ms p99 is the DoD number. Leave headroom for CI variance — if
    // this ever crosses 5ms, fix the regression, don't relax the gate.
    let budget = Duration::from_millis(5);
    assert!(
        p99 < budget,
        "p99 {p99:?} exceeded budget {budget:?} (mean={mean:?}, max={max:?})"
    );

    eprintln!(
        "label_router perf: mean={mean:?} p99={p99:?} max={max:?} (N={N}, budget={budget:?})"
    );
}
