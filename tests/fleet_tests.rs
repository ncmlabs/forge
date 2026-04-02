use forge::fleet;

/// Helper: generate from spec and assert it parses.
fn assert_generates_valid_forge(spec: &str) -> String {
    let result = fleet::generate(spec)
        .unwrap_or_else(|e| panic!("fleet::generate failed for spec '{}': {}", spec, e));
    // Double-check: parse again to be sure
    forge::parser::parse(&result.source)
        .unwrap_or_else(|e| panic!("generated code failed to parse for spec '{}': {}", spec, e));
    result.source
}

// === Round-trip parse tests ===

#[test]
fn chat_system_with_moderator_and_logger() {
    let source = assert_generates_valid_forge("a chat system with moderator and logger");
    assert!(source.contains("agent moderator"));
    assert!(source.contains("agent logger"));
    assert!(source.contains("system chat"));
    assert!(source.contains("llm.reason"));
}

#[test]
fn monitoring_system_with_alerter_and_reporter() {
    let source = assert_generates_valid_forge("a monitoring system with alerter and reporter");
    assert!(source.contains("agent alerter"));
    assert!(source.contains("agent reporter"));
}

#[test]
fn email_pipeline_with_flow() {
    let source = assert_generates_valid_forge(
        "an email pipeline that filters then categorizes then archives",
    );
    assert!(source.contains("flow pipeline"));
    assert!(source.contains("stage filters"));
    assert!(source.contains("stage categorizes"));
    assert!(source.contains("stage archives"));
}

#[test]
fn single_word_spec() {
    let source = assert_generates_valid_forge("chatbot");
    assert!(!source.is_empty());
    // Should produce at least a default agent
    assert!(source.contains("agent"));
}

#[test]
fn system_with_three_agents() {
    let source = assert_generates_valid_forge("a system with reader, processor, and writer");
    assert!(source.contains("agent reader"));
    assert!(source.contains("agent processor"));
    assert!(source.contains("agent writer"));
}

#[test]
fn classify_capability_detected() {
    let source = assert_generates_valid_forge("a system that classifies tickets with sorter");
    assert!(source.contains("llm.classify"));
}

#[test]
fn search_capability_detected() {
    let source =
        assert_generates_valid_forge("a research system that searches the web with finder");
    assert!(source.contains("web.search"));
}

// === Keyword collision tests ===

#[test]
fn keyword_agent_name_sanitized() {
    // "agent" and "task" are FORGE keywords — should be sanitized
    let source = assert_generates_valid_forge("a system with agent and task");
    // Should not produce `agent agent` (which would be a keyword collision)
    assert!(!source.contains("agent agent\n"));
    assert!(!source.contains("agent task\n"));
}

// === Edge cases ===

#[test]
fn empty_spec_produces_output() {
    let source = assert_generates_valid_forge("");
    assert!(!source.is_empty());
}

#[test]
fn spec_with_special_characters() {
    let source = assert_generates_valid_forge("a system! with @logger & checker...");
    assert!(!source.is_empty());
}

// === File output ===

#[test]
fn generates_files_list() {
    let result = fleet::generate("a chat system with moderator").unwrap();
    assert_eq!(result.files.len(), 1);
    assert!(result.files[0].0.ends_with(".forge"));
}

// === States structure ===

#[test]
fn agents_have_lifecycle_states() {
    let source = assert_generates_valid_forge("a system with worker");
    assert!(source.contains("states WorkerLifecycle"));
    assert!(source.contains("lifecycle: WorkerLifecycle"));
    assert!(source.contains("idle -> active"));
}

// === System wiring ===

#[test]
fn multi_agent_system_has_composition() {
    let source = assert_generates_valid_forge("a system with alpha and beta");
    assert!(source.contains(">>"));
}

#[test]
fn single_agent_system_no_composition() {
    let source = assert_generates_valid_forge("a system with worker");
    // Single agent — no >> composition needed
    assert!(!source.contains(">>"));
}
