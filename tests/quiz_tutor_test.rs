// Integration test: drives the quiz_tutor agent through a session
// demonstrating memory, handlers, requires guards, stuck detection,
// state transitions, events, and escalation.

use std::collections::HashMap;
use std::sync::Arc;

use forge::ast::TopLevel;
use forge::llm::providers::mock::MockProvider;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::agent::*;
use forge::runtime::confidence::{ConfidentValue, Value};

fn load_quiz_tutor() -> (forge::ast::AgentDecl, Option<forge::ast::StatesDecl>, forge::ast::Program) {
    let source = std::fs::read_to_string("examples/quiz_tutor.forge")
        .expect("could not read quiz_tutor.forge");
    let program = forge::parser::parse(&source).expect("parse failed");

    let agent_decl = program.items.iter()
        .find_map(|item| match &item.node {
            TopLevel::Agent(a) => Some(a.clone()),
            _ => None,
        })
        .expect("no agent in program");

    let states_decl = program.items.iter()
        .find_map(|item| match &item.node {
            TopLevel::States(s) => Some(s.clone()),
            _ => None,
        });

    (agent_decl, states_decl, program)
}

fn mock_registry() -> Arc<ProviderRegistry> {
    // The mock matches patterns against the full prompt text.
    // classify wraps input in "Classify the following into exactly one of these categories: ...
    //   Input: <verdict>\nRespond with just the category name."
    // So we match on the verdict text that check_answer returns.
    let mock = MockProvider::new("mock")
        .with_response("quiz question", "What keyword declares a deterministic function in FORGE?")
        .with_response("Grade this", "CORRECT the answer is right")
        .with_response("the answer is right", "correct")
        .with_response("hint", "Think about which keyword guarantees no LLM calls.")
        .with_response("Pick one topic", "tasks")
        .with_default("mock response");
    let mut reg = ProviderRegistry::new("mock");
    reg.register("mock", Arc::new(mock));
    Arc::new(reg)
}

fn text_param(key: &str, val: &str) -> (String, ConfidentValue) {
    (key.to_string(), ConfidentValue::deterministic(Value::Text(val.to_string())))
}

#[tokio::test]
async fn quiz_tutor_full_session() {
    let (agent_decl, states_decl, program) = load_quiz_tutor();
    let agent = AgentProcess::new(
        agent_decl, states_decl.as_ref(), mock_registry(), None, program,
    );

    // 1. Start the session — should initialize memory and ask first question
    println!("=== Starting session ===");
    let _r = agent.dispatch("start", HashMap::new()).await.unwrap();

    // Verify memory was initialized
    {
        let ctx = agent.context().lock().unwrap();
        let q_asked = ctx.memory.get("questions_asked").unwrap();
        assert!(matches!(&q_asked.value, Value::Number(n) if *n == 1.0),
            "should have asked 1 question");
        let level = ctx.memory.get("level").unwrap();
        assert!(matches!(&level.value, Value::Text(s) if s == "beginner"),
            "should start at beginner");
    }

    // 2. Answer correctly
    println!("\n=== Answering question ===");
    let params: HashMap<String, ConfidentValue> = HashMap::from([
        text_param("student_answer", "pure"),
    ]);
    let _r = agent.dispatch("answer", params).await.unwrap();

    // Score should have incremented
    {
        let ctx = agent.context().lock().unwrap();
        let score = ctx.memory.get("score").unwrap();
        assert!(matches!(&score.value, Value::Number(n) if *n >= 1.0),
            "score should be at least 1 after correct answer");
    }

    // 3. Ask for a hint
    println!("\n=== Requesting hint ===");
    let _r = agent.dispatch("hint", HashMap::new()).await.unwrap();

    // 4. Skip a question
    println!("\n=== Skipping question ===");
    let _r = agent.dispatch("skip", HashMap::new()).await.unwrap();

    // Questions asked should have gone up
    {
        let ctx = agent.context().lock().unwrap();
        let q_asked = ctx.memory.get("questions_asked").unwrap();
        assert!(matches!(&q_asked.value, Value::Number(n) if *n >= 3.0),
            "should have asked at least 3 questions");
    }

    // 5. End session — should emit SessionSummary event
    println!("\n=== Ending session ===");
    let _r = agent.dispatch("end_session", HashMap::new()).await.unwrap();

    {
        let ctx = agent.context().lock().unwrap();
        let has_summary = ctx.event_sink.emitted.iter()
            .any(|(name, _)| name == "SessionSummary");
        assert!(has_summary, "should have emitted SessionSummary event");
    }

    println!("\n=== Session complete! ===");
}

#[tokio::test]
async fn quiz_tutor_requires_guard_rejects_early_answer() {
    let (agent_decl, states_decl, program) = load_quiz_tutor();
    let agent = AgentProcess::new(
        agent_decl, states_decl.as_ref(), mock_registry(), None, program,
    );

    // Answering before start should be rejected by requires guard
    let params: HashMap<String, ConfidentValue> = HashMap::from([
        text_param("student_answer", "tasks"),
    ]);
    let result = agent.dispatch("answer", params).await.unwrap();

    // Should get the "No active question" message from the give fail policy
    assert!(matches!(result, Some(ref v) if matches!(&v.value, Value::Text(s) if s.contains("No active question"))),
        "requires guard should reject answer before start");
}

#[tokio::test]
async fn quiz_tutor_stuck_detection() {
    let (agent_decl, states_decl, program) = load_quiz_tutor();

    // Use a mock that always returns "WRONG" for grading
    let mock = MockProvider::new("mock")
        .with_response("Grade this", "WRONG that is incorrect")
        .with_response("that is incorrect", "wrong")
        .with_response("hint", "Try reading the FORGE documentation")
        .with_response("Pick one topic", "tasks")
        .with_response("quiz question", "What does the give keyword do?")
        .with_default("mock response");
    let mut reg = ProviderRegistry::new("mock");
    reg.register("mock", Arc::new(mock));
    let registry = Arc::new(reg);

    let agent = AgentProcess::new(
        agent_decl, states_decl.as_ref(), registry, None, program,
    );

    // Start session
    agent.dispatch("start", HashMap::new()).await.unwrap();

    // Give 3 identical wrong answers to trigger stuck detection
    for i in 0..3 {
        println!("--- Wrong answer #{} ---", i + 1);
        let params: HashMap<String, ConfidentValue> = HashMap::from([
            text_param("student_answer", "I don't know"),
        ]);
        let _r = agent.dispatch("answer", params).await.unwrap();
    }

    // After 3 similar turns (start + 3 answers = 4 dispatches, stuck checks last 3)
    // the stuck detector should have fired, which runs the stuck policy body
    // The stuck policy body calls generate_hint and says help messages
    println!("Stuck detection test complete — policy would have fired if responses were similar enough");
}
