// FORGE forge-sensei parser tests
// Validates the AST structure of workflows/forge-sensei.forge after parsing.

use forge::ast::{Program, TopLevel};

fn parse_file(path: &str) -> Program {
    let source =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("could not read {}: {}", path, e));
    forge::parser::parse(&source).unwrap_or_else(|e| panic!("parse failed for {}: {:?}", path, e))
}

// ── Test 1: full program AST structure counts ───────────────────

#[test]
fn parse_sensei_full_program_ast_structure() {
    let program = parse_file("workflows/forge-sensei.forge");

    let mut types = 0;
    let mut events = 0;
    let mut states = 0;
    let mut pures = 0;
    let mut tasks = 0;
    let mut flows = 0;
    let mut contracts = 0;
    let mut agents = 0;
    let mut wardens = 0;

    for item in &program.items {
        match &item.node {
            TopLevel::TypeDef(_) => types += 1,
            TopLevel::Event(_) => events += 1,
            TopLevel::States(_) => states += 1,
            TopLevel::Pure(_) => pures += 1,
            TopLevel::Task(_) => tasks += 1,
            TopLevel::Flow(_) => flows += 1,
            TopLevel::Contract(_) => contracts += 1,
            TopLevel::Agent(_) => agents += 1,
            TopLevel::Warden(_) => wardens += 1,
            _ => {}
        }
    }

    assert_eq!(
        types, 2,
        "expected 2 type defs (QueryResult, AssessmentResult)"
    );
    assert_eq!(
        events, 3,
        "expected 3 events (LearnedInsight, AssessmentCompleted, KnowledgeGapFound)"
    );
    assert_eq!(
        states, 2,
        "expected 2 states (MasteryLevel, SpecialistPhase)"
    );
    assert_eq!(pures, 4, "expected 4 pure functions");
    assert_eq!(tasks, 5, "expected 5 tasks");
    assert_eq!(flows, 2, "expected 2 flows (answer_query, review_code)");
    assert_eq!(contracts, 1, "expected 1 contract (ForgeTutor)");
    assert_eq!(agents, 2, "expected 2 agents (forge_sensei, specialist)");
    assert_eq!(wardens, 1, "expected 1 warden (sensei_warden)");
}

// ── Test 2: forge_sensei agent handler completeness ─────────────

#[test]
fn parse_sensei_agent_handlers_complete() {
    let program = parse_file("workflows/forge-sensei.forge");

    let sensei = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Agent(a) if a.name.node == "forge_sensei" => Some(a.as_ref()),
            _ => None,
        })
        .expect("forge_sensei agent not found");

    let handler_names: Vec<&str> = sensei
        .handlers
        .iter()
        .map(|h| h.node.event.node.as_str())
        .collect();

    let expected = [
        "start",
        "ingest",
        "ingest_fact",
        "query",
        "review",
        "learn_from_session",
        "deep_dive",
        "assess_detailed",
        "batch_assess",
        "status",
        "self_assess.expired",
    ];

    for name in &expected {
        assert!(
            handler_names.contains(name),
            "missing handler '{}' in forge_sensei; found: {:?}",
            name,
            handler_names
        );
    }

    assert_eq!(
        handler_names.len(),
        expected.len(),
        "handler count mismatch; found: {:?}",
        handler_names
    );
}

// ── Test 3: warden policies ─────────────────────────────────────

#[test]
fn parse_sensei_warden_policies() {
    let program = parse_file("workflows/forge-sensei.forge");

    let warden = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Warden(w) => Some(w),
            _ => None,
        })
        .expect("warden not found");

    assert_eq!(warden.name.node, "sensei_warden");

    let managed: Vec<&str> = warden.manages.iter().map(|m| m.node.as_str()).collect();
    assert!(
        managed.contains(&"forge_sensei"),
        "warden should manage forge_sensei"
    );
    assert!(
        managed.contains(&"specialist"),
        "warden should manage specialist"
    );

    assert!(
        warden.policies.len() >= 5,
        "expected at least 5 warden policies, found {}",
        warden.policies.len()
    );
}
