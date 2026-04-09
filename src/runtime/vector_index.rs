// FORGE vector index — issue #50
// In-memory vector store with cosine similarity search and JSON persistence.
// Brute-force approach is sufficient for < 10K documents (typical agent scale).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

use serde::{Deserialize, Serialize};

pub type SharedVectorIndex = Arc<Mutex<VectorIndex>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorEntry {
    pub id: String,
    pub content: String,
    pub embedding: Vec<f32>,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub id: String,
    pub content: String,
    pub score: f32,
    pub metadata: HashMap<String, String>,
}

pub struct VectorIndex {
    entries: Vec<VectorEntry>,
    dimensions: usize,
    store_path: Option<PathBuf>,
}

impl VectorIndex {
    pub fn new(dimensions: usize, store_path: Option<&Path>) -> Self {
        let mut index = Self {
            entries: Vec::new(),
            dimensions,
            store_path: store_path.map(|p| p.to_path_buf()),
        };
        // Try loading persisted data
        if let Some(ref path) = index.store_path {
            if path.exists() {
                if let Ok(data) = std::fs::read_to_string(path) {
                    if let Ok(entries) = serde_json::from_str::<Vec<VectorEntry>>(&data) {
                        index.entries = entries;
                    }
                }
            }
        }
        index
    }

    /// Insert a vector entry. The embedding is normalized to unit length for fast cosine search.
    pub fn insert(
        &mut self,
        id: &str,
        content: &str,
        mut embedding: Vec<f32>,
        metadata: HashMap<String, String>,
    ) -> Result<(), String> {
        if embedding.len() != self.dimensions {
            return Err(format!(
                "embedding dimension mismatch: expected {}, got {}",
                self.dimensions,
                embedding.len()
            ));
        }

        // Normalize to unit vector (pre-compute for fast dot-product search)
        let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut embedding {
                *x /= norm;
            }
        }

        // Remove existing entry with same ID (upsert)
        self.entries.retain(|e| e.id != id);

        self.entries.push(VectorEntry {
            id: id.to_string(),
            content: content.to_string(),
            embedding,
            metadata,
        });

        self.persist();
        Ok(())
    }

    /// Search for the top-k most similar entries by cosine similarity.
    /// Assumes embeddings are pre-normalized, so cosine = dot product.
    pub fn search(&self, query_embedding: &[f32], top_k: usize) -> Vec<SearchResult> {
        if query_embedding.len() != self.dimensions || self.entries.is_empty() {
            return Vec::new();
        }

        // Normalize query
        let norm: f32 = query_embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        let query_norm: Vec<f32> = if norm > 0.0 {
            query_embedding.iter().map(|x| x / norm).collect()
        } else {
            return Vec::new();
        };

        // Brute-force dot product (= cosine similarity for unit vectors)
        let mut scored: Vec<(usize, f32)> = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                let dot: f32 = entry
                    .embedding
                    .iter()
                    .zip(query_norm.iter())
                    .map(|(a, b)| a * b)
                    .sum();
                (i, dot)
            })
            .collect();

        // Sort descending by score
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        scored
            .into_iter()
            .take(top_k)
            .map(|(i, score)| {
                let entry = &self.entries[i];
                SearchResult {
                    id: entry.id.clone(),
                    content: entry.content.clone(),
                    score,
                    metadata: entry.metadata.clone(),
                }
            })
            .collect()
    }

    pub fn delete(&mut self, id: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        let removed = self.entries.len() < before;
        if removed {
            self.persist();
        }
        removed
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    fn persist(&self) {
        if let Some(ref path) = self.store_path {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(data) = serde_json::to_string(&self.entries) {
                let _ = std::fs::write(path, data);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_search() {
        let mut index = VectorIndex::new(3, None);

        index
            .insert("a", "hello world", vec![1.0, 0.0, 0.0], HashMap::new())
            .unwrap();
        index
            .insert("b", "goodbye world", vec![0.0, 1.0, 0.0], HashMap::new())
            .unwrap();
        index
            .insert("c", "similar to hello", vec![0.9, 0.1, 0.0], HashMap::new())
            .unwrap();

        let results = index.search(&[1.0, 0.0, 0.0], 2);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].id, "a"); // exact match first
        assert_eq!(results[1].id, "c"); // similar second
        assert!(results[0].score > results[1].score);
    }

    #[test]
    fn dimension_mismatch_rejected() {
        let mut index = VectorIndex::new(3, None);
        assert!(index
            .insert("x", "test", vec![1.0, 0.0], HashMap::new())
            .is_err());
    }

    #[test]
    fn upsert_replaces_existing() {
        let mut index = VectorIndex::new(3, None);
        index
            .insert("a", "v1", vec![1.0, 0.0, 0.0], HashMap::new())
            .unwrap();
        index
            .insert("a", "v2", vec![0.0, 1.0, 0.0], HashMap::new())
            .unwrap();
        assert_eq!(index.entry_count(), 1);

        let results = index.search(&[0.0, 1.0, 0.0], 1);
        assert_eq!(results[0].content, "v2");
    }

    #[test]
    fn delete_entry() {
        let mut index = VectorIndex::new(3, None);
        index
            .insert("a", "test", vec![1.0, 0.0, 0.0], HashMap::new())
            .unwrap();
        assert!(index.delete("a"));
        assert!(!index.delete("a"));
        assert_eq!(index.entry_count(), 0);
    }

    #[test]
    fn persistence_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vectors.json");

        {
            let mut index = VectorIndex::new(3, Some(&path));
            index
                .insert("a", "persisted", vec![1.0, 0.0, 0.0], HashMap::new())
                .unwrap();
        }

        let index = VectorIndex::new(3, Some(&path));
        assert_eq!(index.entry_count(), 1);
        let results = index.search(&[1.0, 0.0, 0.0], 1);
        assert_eq!(results[0].content, "persisted");
    }

    #[test]
    fn empty_index_search() {
        let index = VectorIndex::new(3, None);
        let results = index.search(&[1.0, 0.0, 0.0], 5);
        assert!(results.is_empty());
    }
}
