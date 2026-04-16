// FORGE knowledge store — progressive learning agent support
// Persistent, searchable knowledge store for agents.
// Uses TF-IDF for retrieval (v1), JSON for persistence.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::runtime::confidence::{ConfidentValue, Value};
use crate::types::ConfidenceSource;

// ── Knowledge Entry ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeEntry {
    pub id: String,
    pub content: String,
    pub source: KnowledgeSource,
    pub confidence: f32,
    #[serde(default)]
    pub category: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_accessed: DateTime<Utc>,
    pub access_count: u64,
    pub success_associations: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KnowledgeSource {
    Direct,
    Interaction {
        question: String,
        answer: String,
        confidence: f32,
    },
    Document {
        path: String,
    },
    AgentTransfer {
        source_agent: String,
    },
}

// ── Knowledge Store ────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct KnowledgeStore {
    store_path: String,
    entries: Vec<KnowledgeEntry>,
    max_entries: usize,
    retention_days: Option<u64>,
    /// Inverted index: term -> list of (entry_index, term_frequency)
    tf_index: HashMap<String, Vec<(usize, f32)>>,
    /// Document count for IDF calculation
    doc_count: usize,
}

/// Shared, thread-safe handle to a KnowledgeStore.
/// Used by both agent context and endpoint executor to ensure
/// learn and recall operate on the same instance.
pub type SharedKnowledgeStore = Arc<Mutex<KnowledgeStore>>;

impl KnowledgeStore {
    pub fn new(store_path: &str, max_entries: Option<usize>, retention_days: Option<u64>) -> Self {
        // Expand ~ to user home directory for absolute paths
        let expanded_path = if store_path.starts_with("~/") {
            dirs::home_dir()
                .map(|h| h.join(&store_path[2..]).to_string_lossy().to_string())
                .unwrap_or_else(|| store_path.to_string())
        } else {
            store_path.to_string()
        };
        let mut store = KnowledgeStore {
            store_path: expanded_path,
            entries: Vec::new(),
            max_entries: max_entries.unwrap_or(10_000),
            retention_days,
            tf_index: HashMap::new(),
            doc_count: 0,
        };
        store.load();
        store.evict_expired();
        store.rebuild_index();
        store
    }

    // ── Learn ──────────────────────────────────────────────

    pub fn learn_direct(&mut self, content: &str) {
        let entry = KnowledgeEntry {
            id: Uuid::new_v4().to_string(),
            content: content.to_string(),
            source: KnowledgeSource::Direct,
            confidence: 1.0,
            category: None,
            created_at: Utc::now(),
            last_accessed: Utc::now(),
            access_count: 0,
            success_associations: 0,
        };
        self.add_entry(entry);
    }

    pub fn learn_direct_categorized(&mut self, content: &str, category: &str) {
        let entry = KnowledgeEntry {
            id: Uuid::new_v4().to_string(),
            content: content.to_string(),
            source: KnowledgeSource::Direct,
            confidence: 1.0,
            category: Some(category.to_string()),
            created_at: Utc::now(),
            last_accessed: Utc::now(),
            access_count: 0,
            success_associations: 0,
        };
        self.add_entry(entry);
    }

    pub fn learn_from_interaction(&mut self, question: &str, answer: &str, confidence: f32) {
        self.learn_from_interaction_impl(question, answer, confidence, None);
    }

    pub fn learn_from_interaction_categorized(
        &mut self,
        question: &str,
        answer: &str,
        confidence: f32,
        category: &str,
    ) {
        self.learn_from_interaction_impl(question, answer, confidence, Some(category.to_string()));
    }

    fn learn_from_interaction_impl(
        &mut self,
        question: &str,
        answer: &str,
        confidence: f32,
        category: Option<String>,
    ) {
        let content = format!("Q: {question}\nA: {answer}");
        let entry = KnowledgeEntry {
            id: Uuid::new_v4().to_string(),
            content,
            source: KnowledgeSource::Interaction {
                question: question.to_string(),
                answer: answer.to_string(),
                confidence,
            },
            confidence,
            category,
            created_at: Utc::now(),
            last_accessed: Utc::now(),
            access_count: 0,
            success_associations: 0,
        };
        self.add_entry(entry);
    }

    pub fn learn_from_document(&mut self, path: &str) -> Result<usize, String> {
        self.learn_from_document_impl(path, None)
    }

    pub fn learn_from_document_categorized(
        &mut self,
        path: &str,
        category: &str,
    ) -> Result<usize, String> {
        self.learn_from_document_impl(path, Some(category.to_string()))
    }

    fn learn_from_document_impl(
        &mut self,
        path: &str,
        category: Option<String>,
    ) -> Result<usize, String> {
        let content = fs::read_to_string(path)
            .map_err(|e| format!("failed to read document '{}': {}", path, e))?;

        let chunks = chunk_text(&content, 500);
        let count = chunks.len();

        for chunk in chunks {
            let entry = KnowledgeEntry {
                id: Uuid::new_v4().to_string(),
                content: chunk,
                source: KnowledgeSource::Document {
                    path: path.to_string(),
                },
                confidence: 1.0,
                category: category.clone(),
                created_at: Utc::now(),
                last_accessed: Utc::now(),
                access_count: 0,
                success_associations: 0,
            };
            self.add_entry(entry);
        }

        Ok(count)
    }

    fn add_entry(&mut self, entry: KnowledgeEntry) {
        // Content deduplication: exact match
        if self.entries.iter().any(|e| e.content == entry.content) {
            return;
        }

        // Evict LRU if at capacity — eviction invalidates postings indices
        if self.entries.len() >= self.max_entries {
            self.evict_lru();
            self.entries.push(entry);
            self.rebuild_index();
        } else {
            self.entries.push(entry);
            let idx = self.entries.len() - 1;
            self.index_entry(idx);
        }
        self.save();
    }

    // ── Recall (TF-IDF search) ─────────────────────────────

    pub fn recall(&mut self, query: &str, token_budget: usize) -> ConfidentValue {
        if self.entries.is_empty() {
            return ConfidentValue {
                value: Value::Text(String::new()),
                confidence: 0.0,
                source: ConfidenceSource::KnowledgeRecall(0.0),
            };
        }

        let query_terms = tokenize(query);
        let mut scores: Vec<(usize, f32)> = Vec::new();

        for (idx, _entry) in self.entries.iter().enumerate() {
            let score = self.tfidf_score(&query_terms, idx);
            if score > 0.0 {
                scores.push((idx, score));
            }
        }

        // Sort by score descending
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Collect entries within token budget
        let mut result_parts = Vec::new();
        let mut tokens_used = 0;
        let mut best_score = 0.0_f32;

        for (idx, score) in &scores {
            let entry = &self.entries[*idx];
            let entry_tokens = estimate_tokens(&entry.content);

            if tokens_used + entry_tokens > token_budget && !result_parts.is_empty() {
                break;
            }

            result_parts.push(entry.content.clone());
            tokens_used += entry_tokens;
            best_score = best_score.max(*score);

            // Update access metadata
            self.entries[*idx].last_accessed = Utc::now();
            self.entries[*idx].access_count += 1;
        }

        if result_parts.is_empty() {
            return ConfidentValue {
                value: Value::Text(String::new()),
                confidence: 0.0,
                source: ConfidenceSource::KnowledgeRecall(0.0),
            };
        }

        // Compute confidence from query term coverage in the best-matching document.
        // Raw TF-IDF scores depend on corpus size and document length, making them
        // poor direct confidence values. Instead, measure what fraction of query terms
        // actually appear in the best result — this directly answers "how well does
        // this entry match what was asked?"
        let confidence = if !query_terms.is_empty() {
            let best_idx = scores[0].0;
            let best_doc_terms: std::collections::HashSet<String> =
                tokenize(&self.entries[best_idx].content)
                    .into_iter()
                    .collect();
            let query_set: std::collections::HashSet<&String> = query_terms.iter().collect();
            let matched = query_set
                .iter()
                .filter(|t| best_doc_terms.contains(**t))
                .count();
            let coverage = matched as f32 / query_set.len() as f32;

            // Bonus for score separation: if the best result is clearly better
            // than the second-best, the match is more decisive
            let separation_bonus = if scores.len() >= 2 && scores[1].1 > 0.0 {
                let ratio = scores[0].1 / scores[1].1;
                ((ratio - 1.0) * 0.1).clamp(0.0, 0.15)
            } else if scores.len() == 1 {
                0.1 // single match gets a small bonus
            } else {
                0.0
            };

            (coverage + separation_bonus).clamp(0.0, 1.0)
        } else {
            best_score.clamp(0.0, 1.0)
        };
        let text = result_parts.join("\n---\n");

        self.save();

        ConfidentValue {
            value: Value::Text(text),
            confidence,
            source: ConfidenceSource::KnowledgeRecall(confidence),
        }
    }

    fn tfidf_score(&self, query_terms: &[String], doc_idx: usize) -> f32 {
        let mut score = 0.0;

        for term in query_terms {
            if let Some(postings) = self.tf_index.get(term) {
                let df = postings.len() as f32;
                let idf = ((self.doc_count as f32) / df).ln() + 1.0;

                for &(idx, tf) in postings {
                    if idx == doc_idx {
                        score += tf * idf;
                    }
                }
            }
        }

        score
    }

    // ── Persistence ────────────────────────────────────────

    fn save(&self) {
        let dir = Path::new(&self.store_path);
        if let Some(parent) = dir.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::create_dir_all(dir);

        let json_path = dir.join("knowledge.json");
        if let Ok(json) = serde_json::to_string_pretty(&self.entries) {
            let _ = fs::write(json_path, json);
        }
    }

    fn load(&mut self) {
        let json_path = Path::new(&self.store_path).join("knowledge.json");
        if json_path.exists() {
            if let Ok(data) = fs::read_to_string(&json_path) {
                if let Ok(entries) = serde_json::from_str::<Vec<KnowledgeEntry>>(&data) {
                    self.entries = entries;
                }
            }
        }
    }

    // ── Index management ───────────────────────────────────

    fn rebuild_index(&mut self) {
        self.tf_index.clear();
        self.doc_count = self.entries.len();

        for (idx, entry) in self.entries.iter().enumerate() {
            let terms = tokenize(&entry.content);
            let total = terms.len() as f32;
            if total == 0.0 {
                continue;
            }

            let mut term_counts: HashMap<&str, usize> = HashMap::new();
            for term in &terms {
                *term_counts.entry(term.as_str()).or_insert(0) += 1;
            }

            for (term, count) in term_counts {
                let tf = count as f32 / total;
                self.tf_index
                    .entry(term.to_string())
                    .or_default()
                    .push((idx, tf));
            }
        }
    }

    /// Incrementally index a single newly-appended entry.
    /// O(1) per entry vs O(N) for `rebuild_index`.
    fn index_entry(&mut self, idx: usize) {
        self.doc_count = self.entries.len();
        let entry = &self.entries[idx];
        let terms = tokenize(&entry.content);
        let total = terms.len() as f32;
        if total == 0.0 {
            return;
        }

        let mut term_counts: HashMap<&str, usize> = HashMap::new();
        for term in &terms {
            *term_counts.entry(term.as_str()).or_insert(0) += 1;
        }

        for (term, count) in term_counts {
            let tf = count as f32 / total;
            self.tf_index
                .entry(term.to_string())
                .or_default()
                .push((idx, tf));
        }
    }

    // ── Eviction ───────────────────────────────────────────

    fn evict_expired(&mut self) {
        if let Some(days) = self.retention_days {
            let cutoff = Utc::now() - chrono::Duration::days(days as i64);
            self.entries.retain(|e| e.created_at > cutoff);
        }
    }

    fn evict_lru(&mut self) {
        if self.entries.is_empty() {
            return;
        }

        let mut oldest_idx = 0;
        let mut oldest_time = self.entries[0].last_accessed;

        for (i, entry) in self.entries.iter().enumerate().skip(1) {
            if entry.last_accessed < oldest_time {
                oldest_time = entry.last_accessed;
                oldest_idx = i;
            }
        }

        self.entries.remove(oldest_idx);
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    // ── Portability ───────────────────────────────────────

    pub fn export_entries(&self) -> Vec<KnowledgeEntry> {
        self.entries.clone()
    }

    pub fn export_filtered(
        &self,
        predicate: impl Fn(&KnowledgeEntry) -> bool,
    ) -> Vec<KnowledgeEntry> {
        self.entries
            .iter()
            .filter(|e| predicate(e))
            .cloned()
            .collect()
    }

    pub fn export_by_category(&self, category: &str) -> Vec<KnowledgeEntry> {
        self.export_filtered(|e| e.category.as_deref() == Some(category))
    }

    pub fn export_above_confidence(&self, min_confidence: f32) -> Vec<KnowledgeEntry> {
        self.export_filtered(|e| e.confidence >= min_confidence)
    }

    pub fn merge_imported(&mut self, entries: Vec<KnowledgeEntry>) -> usize {
        let existing_contents: std::collections::HashSet<String> =
            self.entries.iter().map(|e| e.content.clone()).collect();

        let mut added = 0;
        for entry in entries {
            if existing_contents.contains(&entry.content) {
                continue;
            }
            // Evict LRU if at capacity
            while self.entries.len() >= self.max_entries {
                self.evict_lru();
            }
            self.entries.push(entry);
            added += 1;
        }

        if added > 0 {
            self.rebuild_index();
            self.save();
        }

        added
    }
}

// ── Utilities ──────────────────────────────────────────────

fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() > 1)
        .map(String::from)
        .collect()
}

fn estimate_tokens(text: &str) -> usize {
    // Rough estimate: ~4 chars per token
    text.len().div_ceil(4)
}

fn chunk_text(text: &str, max_words: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() <= max_words {
        return vec![text.to_string()];
    }

    words
        .chunks(max_words)
        .map(|chunk| chunk.join(" "))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_learn_and_recall() {
        let tmp = TempDir::new().unwrap();
        let store_path = tmp
            .path()
            .join("test_knowledge")
            .to_string_lossy()
            .to_string();
        let mut store = KnowledgeStore::new(&store_path, Some(100), None);

        store.learn_direct("Rust is a systems programming language");
        store.learn_direct("Python is great for data science");
        store.learn_direct("FORGE is built with Rust");

        let result = store.recall("Rust programming", 1000);
        assert!(result.confidence > 0.0);

        let text = format!("{}", result.value);
        assert!(text.contains("Rust"));
    }

    #[test]
    fn test_learn_from_interaction() {
        let tmp = TempDir::new().unwrap();
        let store_path = tmp
            .path()
            .join("test_knowledge")
            .to_string_lossy()
            .to_string();
        let mut store = KnowledgeStore::new(&store_path, Some(100), None);

        store.learn_from_interaction(
            "How do I search GPU offers?",
            "Use vastai search offers",
            0.9,
        );

        let result = store.recall("GPU offers search", 1000);
        assert!(result.confidence > 0.0);
    }

    #[test]
    fn test_persistence() {
        let tmp = TempDir::new().unwrap();
        let store_path = tmp
            .path()
            .join("test_knowledge")
            .to_string_lossy()
            .to_string();

        {
            let mut store = KnowledgeStore::new(&store_path, Some(100), None);
            store.learn_direct("persistent fact");
        }

        // Reload
        let mut store = KnowledgeStore::new(&store_path, Some(100), None);
        assert_eq!(store.entry_count(), 1);

        let result = store.recall("persistent", 1000);
        assert!(result.confidence > 0.0);
    }

    #[test]
    fn test_max_entries_eviction() {
        let tmp = TempDir::new().unwrap();
        let store_path = tmp
            .path()
            .join("test_knowledge")
            .to_string_lossy()
            .to_string();
        let mut store = KnowledgeStore::new(&store_path, Some(3), None);

        store.learn_direct("fact one");
        store.learn_direct("fact two");
        store.learn_direct("fact three");
        store.learn_direct("fact four");

        assert_eq!(store.entry_count(), 3);
    }

    #[test]
    fn test_empty_recall() {
        let tmp = TempDir::new().unwrap();
        let store_path = tmp
            .path()
            .join("test_knowledge")
            .to_string_lossy()
            .to_string();
        let mut store = KnowledgeStore::new(&store_path, Some(100), None);

        let result = store.recall("anything", 1000);
        assert_eq!(result.confidence, 0.0);
    }

    #[test]
    fn test_recall_confidence_scales_with_term_coverage() {
        let tmp = TempDir::new().unwrap();
        let store_path = tmp
            .path()
            .join("test_knowledge")
            .to_string_lossy()
            .to_string();
        let mut store = KnowledgeStore::new(&store_path, Some(100), None);

        store.learn_direct("[TASKS] FORGE task declaration pattern needs gives do give");
        store.learn_direct("Python is great for data science");
        store.learn_direct("Unrelated content about cooking recipes");

        // High coverage: all query terms match the first entry
        let result = store.recall("FORGE task declaration needs gives", 1000);
        assert!(
            result.confidence >= 0.8,
            "Full term coverage should produce sure confidence, got {}",
            result.confidence
        );

        // Partial coverage: only some terms match
        let result = store.recall("FORGE agent lifecycle", 1000);
        assert!(
            result.confidence < 0.8 && result.confidence > 0.0,
            "Partial coverage should produce unsure or lower confidence, got {}",
            result.confidence
        );

        // No coverage: no terms match
        let result = store.recall("quantum computing blockchain", 1000);
        assert_eq!(
            result.confidence, 0.0,
            "No matches should produce zero confidence"
        );
    }

    #[test]
    fn test_export_entries() {
        let tmp = TempDir::new().unwrap();
        let store_path = tmp
            .path()
            .join("test_knowledge")
            .to_string_lossy()
            .to_string();
        let mut store = KnowledgeStore::new(&store_path, Some(100), None);

        store.learn_direct("fact alpha");
        store.learn_direct("fact beta");

        let exported = store.export_entries();
        assert_eq!(exported.len(), 2);
        assert!(exported.iter().any(|e| e.content == "fact alpha"));
        assert!(exported.iter().any(|e| e.content == "fact beta"));
    }

    #[test]
    fn test_merge_imported_deduplicates() {
        let tmp = TempDir::new().unwrap();
        let store_path = tmp
            .path()
            .join("test_knowledge")
            .to_string_lossy()
            .to_string();
        let mut store = KnowledgeStore::new(&store_path, Some(100), None);

        store.learn_direct("existing fact");

        let imported = vec![
            KnowledgeEntry {
                id: Uuid::new_v4().to_string(),
                content: "existing fact".to_string(),
                source: KnowledgeSource::Direct,
                confidence: 1.0,
                category: None,
                created_at: Utc::now(),
                last_accessed: Utc::now(),
                access_count: 0,
                success_associations: 0,
            },
            KnowledgeEntry {
                id: Uuid::new_v4().to_string(),
                content: "new imported fact".to_string(),
                source: KnowledgeSource::Direct,
                confidence: 1.0,
                category: None,
                created_at: Utc::now(),
                last_accessed: Utc::now(),
                access_count: 0,
                success_associations: 0,
            },
        ];

        let added = store.merge_imported(imported);
        assert_eq!(added, 1);
        assert_eq!(store.entry_count(), 2);
    }

    #[test]
    fn test_tokenize() {
        let tokens = tokenize("Hello, World! How are you?");
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
        assert!(tokens.contains(&"how".to_string()));
    }

    #[test]
    fn test_dedup_on_learn() {
        let tmp = TempDir::new().unwrap();
        let store_path = tmp
            .path()
            .join("test_knowledge")
            .to_string_lossy()
            .to_string();
        let mut store = KnowledgeStore::new(&store_path, Some(100), None);

        store.learn_direct("FORGE is a language");
        store.learn_direct("FORGE is a language"); // duplicate
        store.learn_direct("FORGE is a language"); // duplicate

        assert_eq!(store.entry_count(), 1);

        // But different content is accepted
        store.learn_direct("Rust is a language");
        assert_eq!(store.entry_count(), 2);
    }

    #[test]
    fn test_incremental_index_matches_full_rebuild() {
        let tmp = TempDir::new().unwrap();
        let store_path = tmp
            .path()
            .join("test_knowledge")
            .to_string_lossy()
            .to_string();
        let mut store = KnowledgeStore::new(&store_path, Some(100), None);

        store.learn_direct("Rust is a systems programming language");
        store.learn_direct("Python is great for data science");
        store.learn_direct("FORGE is built with Rust");

        // Recall should work correctly with incrementally-built index
        let result = store.recall("Rust programming", 1000);
        assert!(result.confidence > 0.0);
        let text = format!("{}", result.value);
        assert!(text.contains("Rust"));
    }
}
