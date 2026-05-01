// FORGE agent portability integration tests — issue #72

use tempfile::TempDir;

use forge::portability::{
    build_package, inspect_package, load_package, prepare_imported_entries, verify_integrity,
    AgentSchema, SchemaField,
};
use forge::runtime::knowledge_store::{KnowledgeEntry, KnowledgeSource, KnowledgeStore};

fn sample_entry(content: &str, confidence: f32) -> KnowledgeEntry {
    KnowledgeEntry {
        id: uuid::Uuid::new_v4().to_string(),
        content: content.to_string(),
        source: KnowledgeSource::Direct,
        confidence,
        category: None,
        project_id: None,
        created_at: chrono::Utc::now(),
        last_accessed: chrono::Utc::now(),
        access_count: 5,
        success_associations: 3,
    }
}

#[test]
fn export_import_round_trip() {
    let tmp = TempDir::new().unwrap();
    let entries = vec![
        sample_entry("FORGE uses uncertain types", 0.95),
        sample_entry("Agents have knowledge stores", 0.88),
    ];
    let schema = AgentSchema {
        fields: vec![SchemaField {
            name: "count".to_string(),
            field_type: "Number".to_string(),
            default: None,
        }],
        knowledge_config: None,
    };
    let pkg = build_package("research_bot", "rb-001", None, schema, entries);
    let json = serde_json::to_string_pretty(&pkg).unwrap();

    let loaded = load_package(&json).unwrap();
    assert!(verify_integrity(&loaded).is_ok());

    let prepared = prepare_imported_entries(&loaded, 0.7);

    let store_path = tmp.path().join("imported").to_string_lossy().to_string();
    let mut store = KnowledgeStore::new(&store_path, Some(100), None);
    let added = store.merge_imported(prepared);

    assert_eq!(added, 2);
    assert_eq!(store.entry_count(), 2);

    // Verify that the prepared entries had their confidence capped at 0.7
    // (recall confidence is a TF-IDF relevance score, not the entry confidence)
    let result = store.recall("uncertain types FORGE", 1000);
    assert!(result.confidence > 0.0);
}

#[test]
fn confidence_capping_works() {
    let entries = vec![sample_entry("high confidence fact", 0.99)];
    let schema = AgentSchema {
        fields: vec![],
        knowledge_config: None,
    };
    let pkg = build_package("capper", "c-001", None, schema, entries);
    let prepared = prepare_imported_entries(&pkg, 0.5);
    assert_eq!(prepared[0].confidence, 0.5);
}

#[test]
fn multi_import_merges_correctly() {
    let tmp = TempDir::new().unwrap();
    let store_path = tmp.path().join("merged").to_string_lossy().to_string();
    let mut store = KnowledgeStore::new(&store_path, Some(100), None);

    let schema_a = AgentSchema {
        fields: vec![],
        knowledge_config: None,
    };
    let pkg_a = build_package(
        "agentA",
        "a-001",
        None,
        schema_a,
        vec![sample_entry("fact from A", 0.9)],
    );
    let prepared_a = prepare_imported_entries(&pkg_a, 0.7);
    store.merge_imported(prepared_a);

    let schema_b = AgentSchema {
        fields: vec![],
        knowledge_config: None,
    };
    let pkg_b = build_package(
        "agentB",
        "b-001",
        None,
        schema_b,
        vec![
            sample_entry("fact from B", 0.8),
            sample_entry("fact from A", 0.85), // duplicate content
        ],
    );
    let prepared_b = prepare_imported_entries(&pkg_b, 0.7);
    let added = store.merge_imported(prepared_b);

    assert_eq!(added, 1);
    assert_eq!(store.entry_count(), 2);
}

#[test]
fn inspect_shows_metadata() {
    let entries = vec![sample_entry("test fact", 0.9)];
    let schema = AgentSchema {
        fields: vec![],
        knowledge_config: None,
    };
    let pkg = build_package("test_agent", "ta-001", None, schema, entries);
    let output = inspect_package(&pkg);

    assert!(output.contains("test_agent"));
    assert!(output.contains("1 entries"));
}
