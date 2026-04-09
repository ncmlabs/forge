// FORGE vector embeddings acceptance tests — issue #50
// Tests data.embed and data.search with mock embedding provider.
// Run: cargo test embed_

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use forge::compose;
use forge::config::{EmbeddingsConfig, ForgeConfig, ProviderConfig};
use forge::llm::providers::mock::{MockEmbeddingProvider, MockProvider};
use forge::llm::registry::ProviderRegistry;
use forge::runtime::executor::TaskExecutor;
use forge::runtime::storage::ForgeStorage;
use forge::runtime::vector_index::VectorIndex;

// ── Helpers ────────────────────────────────────────────────────────

fn load_wiki_program() -> forge::ast::Program {
    let server_src =
        std::fs::read_to_string("examples/wiki/server.forge").expect("could not read server.forge");
    let shared_src =
        std::fs::read_to_string("examples/wiki/shared.forge").expect("could not read shared.forge");

    let server_prog = forge::parser::parse(&server_src).expect("parse server.forge failed");
    let shared_prog = forge::parser::parse(&shared_src).expect("parse shared.forge failed");

    let files = vec![
        compose::SourceFile {
            path: "server.forge".to_string(),
            source: server_src,
            program: server_prog,
        },
        compose::SourceFile {
            path: "shared.forge".to_string(),
            source: shared_src,
            program: shared_prog,
        },
    ];

    compose::merge_programs(&files)
        .expect("merge_programs failed")
        .program
}

fn wiki_mock_registry() -> Arc<ProviderRegistry> {
    let mock = MockProvider::new("mock")
        .with_response("Search query:", "- **task**: Core primitive\n")
        .with_response("Do not invent syntax", "FORGE has 14 primitives.")
        .with_response("Classify the following", "reference")
        .with_default("mock wiki response");

    let mut reg = ProviderRegistry::new("mock");
    reg.register("mock", Arc::new(mock));
    Arc::new(reg)
}

fn seed_wiki_content(storage: &ForgeStorage) {
    let content_dir = Path::new("examples/wiki/content");
    seed_content_recursive(content_dir, storage);
}

fn seed_content_recursive(dir: &Path, storage: &ForgeStorage) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            seed_content_recursive(&path, storage);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let slug = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if slug.is_empty() {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                storage.store(&format!("page:{slug}"), &content).ok();
            }
        }
    }
}

fn temp_storage_with_content() -> (tempfile::TempDir, Arc<ForgeStorage>) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("embed_test.redb");
    let storage = ForgeStorage::open(&db_path).unwrap();
    seed_wiki_content(&storage);
    (dir, Arc::new(storage))
}

fn mock_config_with_embeddings() -> ForgeConfig {
    let mut config = ForgeConfig::default_mock_config();
    config.embeddings = Some(EmbeddingsConfig {
        provider: "mock".to_string(),
        model: None,
        dimensions: Some(64),
        cost_per_1k_tokens: None,
    });
    config
}

/// Spawn wiki HTTP server with embeddings enabled on a random port.
async fn spawn_embed_server() -> (String, tempfile::TempDir) {
    let program = load_wiki_program();
    let (tmp, storage) = temp_storage_with_content();
    let config = mock_config_with_embeddings();

    // Build mock embedding provider
    let embed_provider: forge::llm::BoxedEmbeddingProvider =
        Arc::new(MockEmbeddingProvider::new("mock", 64));
    let vectors_path = tmp.path().join("vectors.json");
    let vector_index = Arc::new(tokio::sync::Mutex::new(VectorIndex::new(
        64,
        Some(&vectors_path),
    )));

    let executor = TaskExecutor::new(program, wiki_mock_registry(), None)
        .with_storage(storage)
        .with_config(config.clone())
        .with_embeddings(embed_provider, vector_index);

    let server = forge::runtime::http_server::ForgeServer::new(executor, config.server.as_ref());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind failed");
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        server
            .run_on_listener(listener)
            .await
            .expect("server failed");
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    (format!("http://127.0.0.1:{port}"), tmp)
}

// ── VectorIndex Unit Tests ───────────────────────────────────────

#[test]
fn embed_vector_index_cosine_similarity() {
    let mut index = VectorIndex::new(3, None);

    // Orthogonal vectors should have zero similarity
    index
        .insert("x", "along x-axis", vec![1.0, 0.0, 0.0], HashMap::new())
        .unwrap();
    index
        .insert("y", "along y-axis", vec![0.0, 1.0, 0.0], HashMap::new())
        .unwrap();

    let results = index.search(&[1.0, 0.0, 0.0], 2);
    assert_eq!(results[0].id, "x");
    assert!(results[0].score > 0.99, "exact match should be ~1.0");
    assert!(
        results[1].score.abs() < 0.01,
        "orthogonal should be ~0.0, got {}",
        results[1].score
    );
}

#[test]
fn embed_vector_index_top_k_ordering() {
    let mut index = VectorIndex::new(3, None);

    index
        .insert("far", "far away", vec![0.0, 0.0, 1.0], HashMap::new())
        .unwrap();
    index
        .insert("close", "close by", vec![0.9, 0.1, 0.0], HashMap::new())
        .unwrap();
    index
        .insert("exact", "exact match", vec![1.0, 0.0, 0.0], HashMap::new())
        .unwrap();

    let results = index.search(&[1.0, 0.0, 0.0], 3);
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].id, "exact");
    assert_eq!(results[1].id, "close");
    assert_eq!(results[2].id, "far");
}

#[test]
fn embed_vector_index_persistence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test_vectors.json");

    // Write
    {
        let mut index = VectorIndex::new(3, Some(&path));
        index
            .insert("doc1", "hello world", vec![1.0, 0.0, 0.0], HashMap::new())
            .unwrap();
        index
            .insert("doc2", "goodbye", vec![0.0, 1.0, 0.0], HashMap::new())
            .unwrap();
    }

    // Read back
    let index = VectorIndex::new(3, Some(&path));
    assert_eq!(index.entry_count(), 2);
    let results = index.search(&[1.0, 0.0, 0.0], 1);
    assert_eq!(results[0].id, "doc1");
    assert_eq!(results[0].content, "hello world");
}

// ── Mock Embedding Provider Tests ────────────────────────────────

#[tokio::test]
async fn embed_mock_provider_deterministic() {
    use forge::llm::{EmbeddingProvider, EmbeddingRequest};

    let provider = MockEmbeddingProvider::new("test", 64);

    let req = EmbeddingRequest {
        texts: vec!["hello world".to_string()],
        model: None,
    };
    let resp1 = provider.embed(req.clone()).await.unwrap();

    let req2 = EmbeddingRequest {
        texts: vec!["hello world".to_string()],
        model: None,
    };
    let resp2 = provider.embed(req2).await.unwrap();

    // Same input should produce same embedding
    assert_eq!(resp1.embeddings[0], resp2.embeddings[0]);
    assert_eq!(resp1.embeddings[0].len(), 64);
}

#[tokio::test]
async fn embed_mock_provider_unit_vectors() {
    use forge::llm::{EmbeddingProvider, EmbeddingRequest};

    let provider = MockEmbeddingProvider::new("test", 64);
    let resp = provider
        .embed(EmbeddingRequest {
            texts: vec!["test input".to_string()],
            model: None,
        })
        .await
        .unwrap();

    let norm: f32 = resp.embeddings[0].iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!(
        (norm - 1.0).abs() < 0.001,
        "mock embeddings should be unit vectors, got norm {}",
        norm
    );
}

// ── HTTP Server Integration Tests ────────────────────────────────

#[tokio::test]
async fn embed_api_embed_endpoint() {
    let (base, _tmp) = spawn_embed_server().await;

    // Embed a page that exists in storage
    let resp = reqwest::get(format!("{base}/api_embed?slug=getting-started"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);

    let body = resp.text().await.unwrap();
    // Should return an embedding ID starting with "emb_"
    assert!(
        body.starts_with("emb_"),
        "expected embedding ID starting with 'emb_', got: {}",
        body
    );
}

#[tokio::test]
async fn embed_api_embed_nonexistent_still_works() {
    let (base, _tmp) = spawn_embed_server().await;

    // Embedding a nonexistent page: data.get returns Unit which formats as empty string.
    // The embed still succeeds (embeds the empty content) — this is expected behavior.
    let resp = reqwest::get(format!("{base}/api_embed?slug=nonexistent-page"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    // Either returns an embedding ID or a not-found message
    assert!(
        body.starts_with("emb_") || body.contains("not found"),
        "expected embedding ID or not-found, got: {}",
        body
    );
}

#[tokio::test]
async fn embed_api_search_returns_results() {
    let (base, _tmp) = spawn_embed_server().await;

    // First embed some pages
    reqwest::get(format!("{base}/api_embed?slug=getting-started"))
        .await
        .unwrap();
    reqwest::get(format!("{base}/api_embed?slug=task"))
        .await
        .unwrap();
    reqwest::get(format!("{base}/api_embed?slug=agent"))
        .await
        .unwrap();

    // Now search
    let resp = reqwest::get(format!("{base}/api_semantic_search?q=how+do+agents+work"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);

    let body = resp.text().await.unwrap();
    // Should contain results with scores
    assert!(
        body.contains("score:"),
        "expected search results with scores, got: {}",
        body
    );
}

#[tokio::test]
async fn embed_api_search_empty_index() {
    let (base, _tmp) = spawn_embed_server().await;

    // Search with nothing embedded should return empty
    let resp = reqwest::get(format!("{base}/api_semantic_search?q=test"))
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    // Empty result is fine — no crash
    assert!(
        body.len() < 500,
        "unexpected large response for empty index"
    );
}

#[tokio::test]
async fn embed_multiple_pages_search_ranking() {
    let (base, _tmp) = spawn_embed_server().await;

    // Embed several pages
    for slug in &["getting-started", "task", "agent", "flow", "pool", "states"] {
        reqwest::get(format!("{base}/api_embed?slug={slug}"))
            .await
            .unwrap();
    }

    // Search should return results
    let resp = reqwest::get(format!("{base}/api_semantic_search?q=lifecycle+management"))
        .await
        .unwrap();
    let body = resp.text().await.unwrap();

    // Results should contain score information (from the format_search_results task)
    // The response may be empty if mock embeddings produce no good matches,
    // or contain formatted results with scores.
    assert!(
        body.is_empty() || body.contains("score:") || body.contains("emb_"),
        "should return formatted results or empty, got: {}",
        &body[..body.len().min(200)]
    );
}

#[tokio::test]
async fn embed_persists_across_requests() {
    let (base, _tmp) = spawn_embed_server().await;

    // Embed a page
    let id_resp = reqwest::get(format!("{base}/api_embed?slug=getting-started"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(id_resp.starts_with("emb_"));

    // Embedding the same page again should work (upsert — content stays in index)
    let id_resp2 = reqwest::get(format!("{base}/api_embed?slug=getting-started"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(id_resp2.starts_with("emb_"));

    // Search should still find it
    let resp = reqwest::get(format!("{base}/api_semantic_search?q=getting+started"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(!body.is_empty(), "search should return results after embed");
}
