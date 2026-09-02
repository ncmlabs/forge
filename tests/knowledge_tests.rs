// FORGE knowledge store integration tests
// Tests the full pipeline: parse → check → runtime for knowledge/recall/learn.

use tempfile::TempDir;

use forge::ast::{Expr, TemplatePart, TopLevel};
use forge::runtime::knowledge_store::KnowledgeStore;

// ── Knowledge Store Unit-Level Integration ──────────────────────────

#[test]
fn knowledge_persists_across_restarts() {
    let tmp = TempDir::new().unwrap();
    let store_path = tmp.path().join("knowledge").to_string_lossy().to_string();

    // Session 1: learn facts
    {
        let mut store = KnowledgeStore::new(&store_path, Some(100), None);
        store.learn_direct("FORGE is a language for oracle-augmented computation");
        store.learn_direct("Agents are first-class in FORGE");
        store.learn_from_interaction("What is FORGE?", "A language for AI agents", 0.95);
        assert_eq!(store.entry_count(), 3);
    }

    // Session 2: reload and verify
    {
        let mut store = KnowledgeStore::new(&store_path, Some(100), None);
        assert_eq!(store.entry_count(), 3);

        let result = store.recall("FORGE language", 1000);
        assert!(
            result.confidence > 0.0,
            "recall should find persisted entries"
        );

        let text = format!("{}", result.value);
        assert!(
            text.contains("FORGE"),
            "recalled text should contain 'FORGE', got: {}",
            text
        );
    }
}

#[test]
fn recall_respects_token_budget() {
    let tmp = TempDir::new().unwrap();
    let store_path = tmp.path().join("knowledge").to_string_lossy().to_string();

    let mut store = KnowledgeStore::new(&store_path, Some(1000), None);

    // Insert many entries with the search term
    for i in 0..50 {
        store.learn_direct(&format!(
            "Fact number {} about Rust programming language and systems design",
            i
        ));
    }

    // Very small budget — should only return a few entries
    let small = store.recall("Rust programming", 20);
    let small_text = format!("{}", small.value);

    // Large budget — should return more entries
    let large = store.recall("Rust programming", 5000);
    let large_text = format!("{}", large.value);

    assert!(
        large_text.len() >= small_text.len(),
        "larger budget should return at least as much text"
    );
}

#[test]
fn max_entries_evicts_lru() {
    let tmp = TempDir::new().unwrap();
    let store_path = tmp.path().join("knowledge").to_string_lossy().to_string();

    let mut store = KnowledgeStore::new(&store_path, Some(5), None);

    // Fill to capacity
    store.learn_direct("alpha fact");
    store.learn_direct("beta fact");
    store.learn_direct("gamma fact");
    store.learn_direct("delta fact");
    store.learn_direct("epsilon fact");
    assert_eq!(store.entry_count(), 5);

    // Access "alpha" so it's recently accessed
    let _ = store.recall("alpha", 1000);

    // Add one more — should evict LRU (beta, since alpha was just accessed)
    store.learn_direct("zeta fact");
    assert_eq!(store.entry_count(), 5);

    // Alpha should still be findable (it was recently accessed)
    let result = store.recall("alpha", 1000);
    let text = format!("{}", result.value);
    assert!(
        text.contains("alpha"),
        "recently accessed entry should survive eviction"
    );
}

#[test]
fn learn_from_document_creates_entries() {
    let tmp = TempDir::new().unwrap();
    let store_path = tmp.path().join("knowledge").to_string_lossy().to_string();

    // Create a test document
    let doc_path = tmp.path().join("test_doc.md");
    std::fs::write(
        &doc_path,
        "# FORGE Reference\n\nFORGE is a programming language for oracle-augmented computation.\n\nAgents are first-class primitives with memory and lifecycle.\n\nThe recall keyword retrieves from knowledge stores.",
    )
    .unwrap();

    let mut store = KnowledgeStore::new(&store_path, Some(100), None);
    let count = store
        .learn_from_document(doc_path.to_str().unwrap())
        .expect("document ingestion should succeed");

    assert!(count > 0, "should create at least one entry from document");
    assert_eq!(store.entry_count(), count);

    // Recall should find document content
    let result = store.recall("recall keyword knowledge", 1000);
    assert!(
        result.confidence > 0.0,
        "should find relevant content from ingested document"
    );
}

#[test]
fn learn_from_missing_document_returns_error() {
    let tmp = TempDir::new().unwrap();
    let store_path = tmp.path().join("knowledge").to_string_lossy().to_string();

    let mut store = KnowledgeStore::new(&store_path, Some(100), None);
    let result = store.learn_from_document("/nonexistent/path/doc.md");

    assert!(result.is_err(), "missing document should return error");
}

#[test]
fn empty_store_recall_returns_zero_confidence() {
    let tmp = TempDir::new().unwrap();
    let store_path = tmp.path().join("knowledge").to_string_lossy().to_string();

    let mut store = KnowledgeStore::new(&store_path, Some(100), None);
    let result = store.recall("anything", 1000);

    assert_eq!(result.confidence, 0.0);
    let text = format!("{}", result.value);
    assert!(text.is_empty(), "empty store should return empty text");
}

#[test]
fn interaction_learning_tracks_source() {
    let tmp = TempDir::new().unwrap();
    let store_path = tmp.path().join("knowledge").to_string_lossy().to_string();

    let mut store = KnowledgeStore::new(&store_path, Some(100), None);
    store.learn_from_interaction("How do I search GPUs?", "Use vastai search offers", 0.85);

    let result = store.recall("search GPUs vastai", 1000);
    assert!(result.confidence > 0.0);

    let text = format!("{}", result.value);
    assert!(text.contains("vastai"), "should recall interaction content");
}

// ── Category and Filtered Export Tests ────────────────────────────

#[test]
fn categorized_learn_and_export_by_category() {
    let tmp = TempDir::new().unwrap();
    let store_path = tmp.path().join("knowledge").to_string_lossy().to_string();

    let mut store = KnowledgeStore::new(&store_path, Some(100), None);

    store.learn_direct_categorized("FORGE uses indentation for blocks", "SYNTAX");
    store.learn_direct_categorized("Tasks are pure functions", "TASKS");
    store.learn_direct_categorized("Flows execute stages in parallel waves", "FLOWS");
    store.learn_direct_categorized("Keywords are lowercase", "SYNTAX");

    let syntax = store.export_by_category("SYNTAX");
    assert_eq!(syntax.len(), 2);
    assert!(syntax
        .iter()
        .all(|e| e.category.as_deref() == Some("SYNTAX")));

    let tasks = store.export_by_category("TASKS");
    assert_eq!(tasks.len(), 1);

    let flows = store.export_by_category("FLOWS");
    assert_eq!(flows.len(), 1);

    // Non-existent category returns empty
    let empty = store.export_by_category("NONEXISTENT");
    assert!(empty.is_empty());
}

#[test]
fn export_above_confidence_filters_correctly() {
    let tmp = TempDir::new().unwrap();
    let store_path = tmp.path().join("knowledge").to_string_lossy().to_string();

    let mut store = KnowledgeStore::new(&store_path, Some(100), None);

    store.learn_direct("high confidence fact"); // confidence 1.0
    store.learn_from_interaction("Q?", "A", 0.9);
    store.learn_from_interaction("Q2?", "A2", 0.5);
    store.learn_from_interaction("Q3?", "A3", 0.3);

    let high = store.export_above_confidence(0.9);
    assert_eq!(high.len(), 2); // 1.0 and 0.9

    let medium = store.export_above_confidence(0.5);
    assert_eq!(medium.len(), 3); // 1.0, 0.9, 0.5

    let all = store.export_above_confidence(0.0);
    assert_eq!(all.len(), 4);
}

#[test]
fn export_filtered_with_custom_predicate() {
    let tmp = TempDir::new().unwrap();
    let store_path = tmp.path().join("knowledge").to_string_lossy().to_string();

    let mut store = KnowledgeStore::new(&store_path, Some(100), None);

    store.learn_direct_categorized("Rust fact", "SYNTAX");
    store.learn_direct_categorized("Python fact", "SYNTAX");
    store.learn_direct("Uncategorized fact");

    // Filter by content containing "Rust"
    let rust_only = store.export_filtered(|e| e.content.contains("Rust"));
    assert_eq!(rust_only.len(), 1);
    assert!(rust_only[0].content.contains("Rust"));
}

#[test]
fn category_persists_across_restarts() {
    let tmp = TempDir::new().unwrap();
    let store_path = tmp.path().join("knowledge").to_string_lossy().to_string();

    {
        let mut store = KnowledgeStore::new(&store_path, Some(100), None);
        store.learn_direct_categorized("persistent categorized fact", "AGENTS");
        store.learn_direct("uncategorized fact");
    }

    // Reload
    let store = KnowledgeStore::new(&store_path, Some(100), None);
    let agents = store.export_by_category("AGENTS");
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].content, "persistent categorized fact");
    assert_eq!(agents[0].category.as_deref(), Some("AGENTS"));
}

#[test]
fn uncategorized_entries_excluded_from_category_export() {
    let tmp = TempDir::new().unwrap();
    let store_path = tmp.path().join("knowledge").to_string_lossy().to_string();

    let mut store = KnowledgeStore::new(&store_path, Some(100), None);

    store.learn_direct("no category");
    store.learn_direct_categorized("has category", "SYNTAX");

    let syntax = store.export_by_category("SYNTAX");
    assert_eq!(syntax.len(), 1);
    assert_eq!(syntax[0].content, "has category");

    // All entries still exported with export_entries
    assert_eq!(store.export_entries().len(), 2);
}

// ── Categorized Interaction and Document Tests (#85) ────────────

#[test]
fn learn_from_interaction_with_category() {
    let tmp = TempDir::new().unwrap();
    let store_path = tmp.path().join("knowledge").to_string_lossy().to_string();

    let mut store = KnowledgeStore::new(&store_path, Some(100), None);

    store.learn_from_interaction("What is FORGE?", "A language for agents", 0.9);
    store.learn_from_interaction_categorized(
        "How do boundaries work?",
        "Server boundary",
        0.85,
        "BOUNDARY",
    );

    let boundary = store.export_by_category("BOUNDARY");
    assert_eq!(boundary.len(), 1);
    assert_eq!(boundary[0].category.as_deref(), Some("BOUNDARY"));
    assert!(boundary[0].content.contains("boundaries"));

    // Uncategorized interaction excluded from category export
    assert_eq!(store.export_entries().len(), 2);
}

#[test]
fn learn_from_document_with_category() {
    let tmp = TempDir::new().unwrap();
    let store_path = tmp.path().join("knowledge").to_string_lossy().to_string();
    let doc_path = tmp.path().join("test.txt");
    std::fs::write(
        &doc_path,
        "This is a test document for categorized learning.",
    )
    .unwrap();

    let mut store = KnowledgeStore::new(&store_path, Some(100), None);

    let count = store
        .learn_from_document_categorized(doc_path.to_str().unwrap(), "DOCS")
        .unwrap();
    assert!(count > 0);

    let docs = store.export_by_category("DOCS");
    assert_eq!(docs.len(), count);
    assert!(docs.iter().all(|e| e.category.as_deref() == Some("DOCS")));
}

// ── Per-repo store scoping (issue #359 / T8.4) ──────────────────────

/// Two project_ids sharing the same root must produce isolated entries:
/// repo A's writes are invisible to repo B's recall, and vice versa.
/// This is the core correctness property the issue calls out — a single
/// process serving multiple repos must not leak PR-decision lessons across
/// repo boundaries.
#[test]
fn two_project_ids_isolated() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("knowledge").to_string_lossy().to_string();

    // Each repo's entry uses a unique keyword the other repo never sees.
    {
        let mut store_a = KnowledgeStore::new_scoped(&root, Some("repo-a"), Some(100), None);
        store_a.learn_direct("ALPHATOKEN exclusively belongs to repo-a's history");
    }
    {
        let mut store_b = KnowledgeStore::new_scoped(&root, Some("repo-b"), Some(100), None);
        store_b.learn_direct("BETATOKEN exclusively belongs to repo-b's history");
    }

    // Reopen each scope. A recall for the OTHER repo's exclusive token must
    // return zero confidence — neither path-isolation nor the recall filter
    // should let the foreign entry through.
    let mut store_a = KnowledgeStore::new_scoped(&root, Some("repo-a"), Some(100), None);
    assert!(
        store_a.recall("ALPHATOKEN", 1000).confidence > 0.0,
        "repo-a should recall its own ALPHATOKEN entry"
    );
    assert_eq!(
        store_a.recall("BETATOKEN", 1000).confidence,
        0.0,
        "repo-a must not see repo-b's BETATOKEN entry"
    );

    let mut store_b = KnowledgeStore::new_scoped(&root, Some("repo-b"), Some(100), None);
    assert!(
        store_b.recall("BETATOKEN", 1000).confidence > 0.0,
        "repo-b should recall its own BETATOKEN entry"
    );
    assert_eq!(
        store_b.recall("ALPHATOKEN", 1000).confidence,
        0.0,
        "repo-b must not see repo-a's ALPHATOKEN entry"
    );
}

/// Filesystem-level isolation: scoped stores must persist under
/// `{root}/{project_id}/knowledge.json`, not at the unscoped legacy path.
#[test]
fn scoped_store_writes_to_project_subdirectory() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("ks").to_string_lossy().to_string();

    {
        let mut store = KnowledgeStore::new_scoped(&root, Some("ncmlabs-forge"), Some(100), None);
        store.learn_direct("scoped persistence check");
    }

    let scoped_json = tmp.path().join("ks/ncmlabs-forge/knowledge.json");
    let legacy_json = tmp.path().join("ks/knowledge.json");
    assert!(
        scoped_json.exists(),
        "scoped store must persist at {}",
        scoped_json.display()
    );
    assert!(
        !legacy_json.exists(),
        "scoped store must NOT write to the legacy unscoped path {}",
        legacy_json.display()
    );
}

/// `project_id = None` keeps the pre-#359 layout exactly: writes land at
/// `{root}/knowledge.json` with no subdirectory. This is the contract the
/// existing 3-arg `new()` constructor preserves.
#[test]
fn none_project_id_uses_legacy_layout() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("ks").to_string_lossy().to_string();

    {
        let mut store = KnowledgeStore::new(&root, Some(100), None);
        store.learn_direct("legacy layout check");
    }

    let legacy_json = tmp.path().join("ks/knowledge.json");
    assert!(
        legacy_json.exists(),
        "unscoped store must persist at the legacy {} path",
        legacy_json.display()
    );
}

/// Defensive recall filter: even if a scoped store somehow loads entries
/// tagged for a different project (e.g. via a mis-configured import or a
/// hand-edited JSON file), recall must drop them. Path-isolation is the
/// primary defence; this filter guards the case where path-isolation is
/// bypassed.
#[test]
fn recall_filters_entries_with_mismatched_project_id() {
    use forge::runtime::knowledge_store::{KnowledgeEntry, KnowledgeSource};

    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("ks").to_string_lossy().to_string();

    let mut store = KnowledgeStore::new_scoped(&root, Some("repo-a"), Some(100), None);
    store.learn_direct("NATIVEAKEY belongs to repo-a");

    // Inject an entry tagged for a different project via merge_imported,
    // which preserves the original project_id. Path-isolation can't help
    // here — the contamination is in-memory and on-disk for this scope.
    let foreign = vec![KnowledgeEntry {
        id: uuid::Uuid::new_v4().to_string(),
        content: "FOREIGNBKEY snuck in from repo-b".to_string(),
        source: KnowledgeSource::Direct,
        confidence: 1.0,
        category: None,
        project_id: Some("repo-b".to_string()),
        created_at: chrono::Utc::now(),
        last_accessed: chrono::Utc::now(),
        access_count: 0,
        success_associations: 0,
    }];
    store.merge_imported(foreign);

    // A query that ONLY matches the foreign entry must score zero — the
    // filter has to drop it before scoring or recall would leak the content.
    let result = store.recall("FOREIGNBKEY snuck", 1000);
    assert_eq!(
        result.confidence, 0.0,
        "scoped recall must drop entries whose project_id differs from the store's scope"
    );

    // But native content is still recallable.
    assert!(
        store.recall("NATIVEAKEY", 1000).confidence > 0.0,
        "scoped recall must still surface entries that match the store's scope"
    );
}

/// Parser smoke test (#359 / T8.4): the new optional `project_id:` clause in
/// a `knowledge store:` block must parse, populate `KnowledgeDecl::project_id`,
/// and accept both literal-Text values (resolvable at config-extraction time)
/// and identifier expressions (T8.5's per-event resolution use case).
#[test]
fn parser_accepts_project_id_clause_in_knowledge_block() {
    let source = r#"
agent scoped_agent
  knowledge store: ".forge-knowledge/test"
    project_id: "literal-repo-slug"
    max_entries: 100

  on ping
    learn "ack"
"#;
    let program = forge::parser::parse(source).expect("parse with literal project_id");
    let agent = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Agent(a) => Some(a),
            _ => None,
        })
        .expect("agent decl");
    let kd = agent.knowledge.as_ref().expect("knowledge decl");
    let pid_expr = kd.node.project_id.as_ref().expect("project_id parsed");
    match &pid_expr.node {
        Expr::Template(parts) => {
            let text: String = parts
                .iter()
                .filter_map(|p| match &p.node {
                    TemplatePart::Text(t) => Some(t.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(text, "literal-repo-slug");
        }
        other => panic!(
            "expected Template expr for literal project_id, got {:?}",
            other
        ),
    }

    // Forward-compat: T8.5 will inject `memory.repo_slug`. The grammar accepts
    // any expression here so the same .forge syntax works once T8.5 lands —
    // T8.4 just doesn't resolve non-literal expressions yet.
    let dynamic_source = r#"
agent dyn_agent
  memory
    repo_slug: Text
  knowledge store: ".forge-knowledge/test"
    project_id: memory.repo_slug

  on ping
    learn "ack"
"#;
    let program =
        forge::parser::parse(dynamic_source).expect("parse with member-access project_id");
    let agent = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Agent(a) => Some(a),
            _ => None,
        })
        .expect("agent decl");
    assert!(
        agent
            .knowledge
            .as_ref()
            .and_then(|kd| kd.node.project_id.as_ref())
            .is_some(),
        "project_id with member-access expr must parse and populate KnowledgeDecl::project_id"
    );
}
