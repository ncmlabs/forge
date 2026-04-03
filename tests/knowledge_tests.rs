// FORGE knowledge store integration tests
// Tests the full pipeline: parse → check → runtime for knowledge/recall/learn.

use tempfile::TempDir;

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
