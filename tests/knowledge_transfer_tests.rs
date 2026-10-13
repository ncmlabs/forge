// FORGE knowledge transfer integration tests (issue #167)
// Tests the toolkit agent knowledge transfer pipeline:
//   parent export_by_category → confidence cap → merge_imported → child recall

use std::sync::Arc;

use tempfile::TempDir;

use forge::llm::providers::mock::MockProvider;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::executor::TaskExecutor;
use forge::runtime::knowledge_store::{KnowledgeEntry, KnowledgeSource, KnowledgeStore};

// ── Helper ──────────────────────────────────────────────────

fn make_entry(content: &str, confidence: f32, category: &str) -> KnowledgeEntry {
    KnowledgeEntry {
        id: uuid::Uuid::new_v4().to_string(),
        content: content.to_string(),
        source: KnowledgeSource::Direct,
        confidence,
        category: Some(category.to_string()),
        project_id: None,
        created_at: chrono::Utc::now(),
        last_accessed: chrono::Utc::now(),
        access_count: 0,
        success_associations: 0,
    }
}

// ── Tests ───────────────────────────────────────────────────

#[test]
fn export_by_category_with_confidence_cap() {
    let tmp = TempDir::new().unwrap();
    let store_path = tmp.path().join("parent").to_string_lossy().to_string();

    let mut parent = KnowledgeStore::new(&store_path, Some(100), None);
    parent.learn_direct_categorized("FORGE tasks use task keyword", "TASKS");
    parent.learn_direct_categorized("FORGE flows use flow keyword", "FLOWS");
    parent.learn_direct_categorized("FORGE agents use agent keyword", "AGENTS");
    parent.learn_direct_categorized("Another TASKS fact about gives clause", "TASKS");

    // Export only TASKS category
    let mut transferred = parent.export_by_category("TASKS");
    assert_eq!(transferred.len(), 2);

    // Apply confidence cap (mirrors executor.rs logic)
    let cap = 0.8;
    for entry in &mut transferred {
        entry.confidence = entry.confidence.min(cap);
        entry.source = KnowledgeSource::AgentTransfer {
            source_agent: "parent".to_string(),
        };
    }

    // All entries should be capped
    assert!(transferred.iter().all(|e| e.confidence <= cap));

    // Merge into child store
    let child_path = tmp.path().join("child").to_string_lossy().to_string();
    let mut child = KnowledgeStore::new(&child_path, Some(100), None);
    let added = child.merge_imported(transferred);
    assert_eq!(added, 2);
    assert_eq!(child.entry_count(), 2);

    // Child should be able to recall TASKS knowledge
    let result = child.recall("FORGE tasks", 1000);
    assert!(
        result.confidence > 0.0,
        "child should recall transferred knowledge"
    );
    let text = format!("{}", result.value);
    assert!(
        text.contains("task"),
        "recalled text should contain task content"
    );
}

#[test]
fn merge_preserves_category_and_agent_transfer_source() {
    let tmp = TempDir::new().unwrap();

    // Create entries tagged as AgentTransfer
    let mut entries = vec![
        make_entry("FORGE task declaration pattern", 0.8, "TASKS"),
        make_entry("FORGE flow pipeline stages", 0.7, "FLOWS"),
    ];
    for entry in &mut entries {
        entry.source = KnowledgeSource::AgentTransfer {
            source_agent: "forge_sensei".to_string(),
        };
    }

    let store_path = tmp.path().join("child").to_string_lossy().to_string();
    let mut store = KnowledgeStore::new(&store_path, Some(100), None);
    store.merge_imported(entries);

    // Verify category preserved
    let tasks = store.export_by_category("TASKS");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].category.as_deref(), Some("TASKS"));

    // Verify source preserved
    match &tasks[0].source {
        KnowledgeSource::AgentTransfer { source_agent } => {
            assert_eq!(source_agent, "forge_sensei");
        }
        other => panic!("expected AgentTransfer source, got {:?}", other),
    }

    let flows = store.export_by_category("FLOWS");
    assert_eq!(flows.len(), 1);
}

#[test]
fn full_transfer_cycle_seed_export_cap_merge_recall() {
    let tmp = TempDir::new().unwrap();

    // 1. Parent seeds categorized knowledge
    let parent_path = tmp.path().join("parent").to_string_lossy().to_string();
    let mut parent = KnowledgeStore::new(&parent_path, Some(1000), None);

    parent.learn_direct_categorized(
        "FORGE basic task declaration pattern. A task is the primary compute unit. Syntax: task name needs params gives ReturnType do body",
        "TASKS",
    );
    parent.learn_direct_categorized(
        "FORGE task with reason primitive. Tasks can call reason for LLM inference: result = reason \"prompt\"",
        "TASKS",
    );
    parent.learn_direct_categorized(
        "FORGE flow declaration pattern. Flows are multi-stage pipelines: flow name needs params gives ReturnType with stage declarations",
        "FLOWS",
    );
    parent.learn_direct_categorized(
        "FORGE agent handler pattern. Agents respond to events via on handlers: on event_name(params) body",
        "AGENTS",
    );

    // 2. Export TASKS only (simulates spawn with knowledge where category == "TASKS")
    let mut transferred = parent.export_by_category("TASKS");
    assert_eq!(
        transferred.len(),
        2,
        "should export exactly 2 TASKS entries"
    );

    // 3. Apply confidence cap (simulates confidence_cap: 0.8)
    for entry in &mut transferred {
        entry.confidence = entry.confidence.min(0.8);
        entry.source = KnowledgeSource::AgentTransfer {
            source_agent: "forge_sensei".to_string(),
        };
    }

    // 4. Merge into child's knowledge store
    let child_path = tmp.path().join("child").to_string_lossy().to_string();
    let mut child = KnowledgeStore::new(&child_path, Some(1000), None);
    let added = child.merge_imported(transferred);
    assert_eq!(added, 2);

    // 5. Child recalls TASKS knowledge
    let result = child.recall("FORGE task declaration needs gives", 2000);
    assert!(
        result.confidence > 0.0,
        "child should find relevant TASKS knowledge"
    );
    let text = format!("{}", result.value);
    assert!(
        text.contains("task") && text.contains("compute unit"),
        "recall should return task declaration pattern, got: {}",
        text
    );

    // 6. Verify FLOWS and AGENTS were NOT transferred
    let flows = child.export_by_category("FLOWS");
    assert!(flows.is_empty(), "FLOWS should not be in child store");
    let agents = child.export_by_category("AGENTS");
    assert!(agents.is_empty(), "AGENTS should not be in child store");

    // 7. Simulate child learning (feedback loop)
    child.learn_direct_categorized(
        "Generated task pattern: task greet needs name: Text gives Text do give \"Hello {name}\"",
        "TASKS",
    );
    assert_eq!(child.entry_count(), 3);

    // 8. Export child's learned insight back (simulates LearnedInsight → parent absorbs)
    let feedback = child.export_filtered(|e| e.content.contains("Generated task pattern"));
    assert_eq!(feedback.len(), 1);

    let before = parent.entry_count();
    parent.merge_imported(feedback);
    assert_eq!(
        parent.entry_count(),
        before + 1,
        "parent should absorb child's learned insight"
    );
}

// ── #538: the filter is an expression evaluated at spawn ────

/// Run a program whose parent spawns a child while filtering on
/// `with knowledge where <filter>`, and return the child's transferred entries.
/// The parent seeds one TASKS and one FLOWS entry and binds `topic` to
/// `topic_value`, so only the filter form under test decides what the child gets.
async fn entries_after_spawn_with_filter(
    filter: &str,
    topic_value: &str,
) -> Result<Vec<KnowledgeEntry>, forge::runtime::executor::RuntimeError> {
    let tmp = TempDir::new().unwrap();
    let parent_path = tmp.path().join("parent").to_string_lossy().to_string();
    let child_path = tmp.path().join("child").to_string_lossy().to_string();

    let source = format!(
        r#"
agent worker
  memory
    topic: Text
  knowledge store: "{child_path}"
  on start
    say "worker up"

  if stuck for 3 turns
    escalate to human

agent parent
  memory
    topic: Text
  knowledge store: "{parent_path}"
  on start
    learn "TASKS fact: tasks use the task keyword" category: "TASKS"
    learn "FLOWS fact: flows use the flow keyword" category: "FLOWS"
    topic = {topic_value}
    spawn worker as "worker_1"
      with knowledge where {filter}

fn main
  spawn parent as "p"
"#
    );

    let program = forge::parser::parse(&source).expect("parse failed");
    let mock = MockProvider::new("mock").with_default("mock response");
    let mut registry = ProviderRegistry::new("mock");
    registry.register("mock", Arc::new(mock));
    let executor = TaskExecutor::new(program, Arc::new(registry), None);
    executor.run().await?;

    // The transfer is written to the child's declared store during the spawn
    // statement, so the file is complete when `run` returns.
    Ok(KnowledgeStore::new(&child_path, None, None).export_entries())
}

async fn assert_only_tasks_transferred(filter: &str) {
    let entries = entries_after_spawn_with_filter(filter, r#""TASKS""#)
        .await
        .expect("program should run");
    assert_eq!(
        entries.len(),
        1,
        "filter `{filter}` must transfer exactly the TASKS entry, got: {:?}",
        entries
            .iter()
            .map(|e| (e.content.as_str(), e.category.as_deref()))
            .collect::<Vec<_>>()
    );
    assert_eq!(entries[0].category.as_deref(), Some("TASKS"));
    assert!(
        entries[0].content.contains("TASKS fact"),
        "got: {}",
        entries[0].content
    );
}

#[tokio::test]
async fn spawn_knowledge_filter_literal_is_evaluated_at_spawn() {
    assert_only_tasks_transferred(r#"category == "TASKS""#).await;
}

#[tokio::test]
async fn spawn_knowledge_filter_ident_is_evaluated_at_spawn() {
    assert_only_tasks_transferred("category == topic").await;
}

#[tokio::test]
async fn spawn_knowledge_filter_interpolation_is_evaluated_at_spawn() {
    assert_only_tasks_transferred(r#"category == "{topic}""#).await;
}

#[tokio::test]
async fn spawn_knowledge_filter_non_text_is_a_type_error() {
    let err = entries_after_spawn_with_filter("category == topic", "42")
        .await
        .expect_err("a Number category must fail the spawn, not filter silently");
    assert!(
        matches!(
            &err,
            forge::runtime::executor::RuntimeError::TypeError { expected, .. } if expected == "Text"
        ),
        "expected a Text type error, got: {err:?}"
    );
}
