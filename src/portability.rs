// FORGE agent portability — export, import, and inspect agent knowledge packages.
//
// Defines the ForgePackage format (.forge-pkg) for transferring agent expertise
// between runtimes. Includes integrity verification via SHA-256 hashing and
// version-gated import validation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::runtime::knowledge_store::{KnowledgeEntry, KnowledgeSource};

// ── Package format ────────────────────────────────────────────

/// Top-level portable agent package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgePackage {
    pub format_version: String,
    pub metadata: PackageMetadata,
    pub schema: AgentSchema,
    pub knowledge: Vec<KnowledgeEntry>,
    pub integrity: PackageIntegrity,
}

/// Metadata describing the exported agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageMetadata {
    pub agent_name: String,
    pub agent_id: String,
    pub exported_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub expertise: ExpertiseMetrics,
}

/// Quantitative summary of the agent's knowledge base.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpertiseMetrics {
    pub total_entries: usize,
    pub avg_confidence: f32,
    pub top_domains: Vec<String>,
}

/// Schema definition carried by the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSchema {
    pub fields: Vec<SchemaField>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub knowledge_config: Option<SchemaKnowledgeConfig>,
}

/// A single field in the agent schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaField {
    pub name: String,
    pub field_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

/// Knowledge-store configuration carried inside the schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaKnowledgeConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_entries: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention_days: Option<u64>,
}

/// Integrity envelope — hashes computed at export time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageIntegrity {
    pub knowledge_hash: String,
    pub package_hash: String,
}

// ── Hashing ───────────────────────────────────────────────────

/// Compute a hex-encoded SHA-256 digest of the given bytes.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

// ── Build ─────────────────────────────────────────────────────

/// Build a [`ForgePackage`] from its constituent parts.
///
/// Computes `knowledge_hash` over the serialised knowledge entries and
/// `package_hash` over the entire serialised package (with a placeholder
/// package_hash replaced after the fact).
pub fn build_package(
    agent_name: &str,
    agent_id: &str,
    description: Option<String>,
    schema: AgentSchema,
    knowledge: Vec<KnowledgeEntry>,
) -> ForgePackage {
    // Expertise metrics
    let total_entries = knowledge.len();
    let avg_confidence = if total_entries > 0 {
        knowledge.iter().map(|e| e.confidence).sum::<f32>() / total_entries as f32
    } else {
        0.0
    };

    // Derive top domains from Document sources
    let mut domain_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for entry in &knowledge {
        if let KnowledgeSource::Document { path } = &entry.source {
            *domain_counts.entry(path.clone()).or_insert(0) += 1;
        }
    }
    let mut domains: Vec<(String, usize)> = domain_counts.into_iter().collect();
    domains.sort_by(|a, b| b.1.cmp(&a.1));
    let top_domains: Vec<String> = domains.into_iter().take(5).map(|(d, _)| d).collect();

    let expertise = ExpertiseMetrics {
        total_entries,
        avg_confidence,
        top_domains,
    };

    let metadata = PackageMetadata {
        agent_name: agent_name.to_string(),
        agent_id: agent_id.to_string(),
        exported_at: Utc::now(),
        description,
        expertise,
    };

    // Knowledge hash
    let knowledge_json = serde_json::to_string(&knowledge).unwrap_or_default();
    let knowledge_hash = sha256_hex(knowledge_json.as_bytes());

    // Build with placeholder package hash, then recompute
    let mut pkg = ForgePackage {
        format_version: "1.0".to_string(),
        metadata,
        schema,
        knowledge,
        integrity: PackageIntegrity {
            knowledge_hash,
            package_hash: String::new(),
        },
    };

    let pkg_json = serde_json::to_string(&pkg).unwrap_or_default();
    pkg.integrity.package_hash = sha256_hex(pkg_json.as_bytes());

    pkg
}

// ── Import ────────────────────────────────────────────────────

/// Errors that can occur when loading or validating a package.
#[derive(Debug)]
pub enum ImportError {
    DeserializationFailed(String),
    UnsupportedVersion(String),
    IntegrityMismatch { expected: String, actual: String },
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::DeserializationFailed(msg) => {
                write!(f, "deserialization failed: {msg}")
            }
            ImportError::UnsupportedVersion(v) => {
                write!(f, "unsupported package version: {v}")
            }
            ImportError::IntegrityMismatch { expected, actual } => {
                write!(f, "integrity mismatch: expected {expected}, got {actual}")
            }
        }
    }
}

/// Deserialize a JSON string into a [`ForgePackage`], rejecting unknown versions.
pub fn load_package(json: &str) -> Result<ForgePackage, ImportError> {
    let pkg: ForgePackage = serde_json::from_str(json)
        .map_err(|e| ImportError::DeserializationFailed(e.to_string()))?;

    if pkg.format_version != "1.0" {
        return Err(ImportError::UnsupportedVersion(pkg.format_version.clone()));
    }

    Ok(pkg)
}

/// Verify the knowledge hash inside the package matches a fresh computation.
pub fn verify_integrity(pkg: &ForgePackage) -> Result<(), ImportError> {
    let knowledge_json = serde_json::to_string(&pkg.knowledge).unwrap_or_default();
    let actual = sha256_hex(knowledge_json.as_bytes());

    if actual != pkg.integrity.knowledge_hash {
        return Err(ImportError::IntegrityMismatch {
            expected: pkg.integrity.knowledge_hash.clone(),
            actual,
        });
    }

    Ok(())
}

/// Rewrite imported entries for the receiving agent.
///
/// Each entry gets:
/// - a fresh UUID
/// - `KnowledgeSource::AgentTransfer` with the source agent name
/// - confidence capped at `max_confidence`
/// - counters (`access_count`, `success_associations`) reset to 0
pub fn prepare_imported_entries(pkg: &ForgePackage, max_confidence: f32) -> Vec<KnowledgeEntry> {
    let source_agent = pkg.metadata.agent_name.clone();

    pkg.knowledge
        .iter()
        .map(|entry| KnowledgeEntry {
            id: Uuid::new_v4().to_string(),
            content: entry.content.clone(),
            source: KnowledgeSource::AgentTransfer {
                source_agent: source_agent.clone(),
            },
            confidence: entry.confidence.min(max_confidence),
            category: entry.category.clone(),
            created_at: Utc::now(),
            last_accessed: Utc::now(),
            access_count: 0,
            success_associations: 0,
        })
        .collect()
}

// ── Inspect ───────────────────────────────────────────────────

/// Return a human-readable summary of the package contents.
pub fn inspect_package(pkg: &ForgePackage) -> String {
    let mut out = String::new();

    out.push_str(&format!("FORGE Package v{}\n", pkg.format_version));
    out.push_str(&format!(
        "Agent: {} ({})\n",
        pkg.metadata.agent_name, pkg.metadata.agent_id
    ));
    out.push_str(&format!("Exported: {}\n", pkg.metadata.exported_at));

    if let Some(desc) = &pkg.metadata.description {
        out.push_str(&format!("Description: {desc}\n"));
    }

    let ex = &pkg.metadata.expertise;
    out.push_str(&format!(
        "Knowledge: {} entries, avg confidence {:.2}\n",
        ex.total_entries, ex.avg_confidence,
    ));

    if !ex.top_domains.is_empty() {
        out.push_str(&format!("Top domains: {}\n", ex.top_domains.join(", ")));
    }

    out.push_str(&format!("Schema fields: {}\n", pkg.schema.fields.len()));
    out.push_str(&format!(
        "Integrity hash: {}\n",
        pkg.integrity.knowledge_hash
    ));

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entries() -> Vec<KnowledgeEntry> {
        vec![KnowledgeEntry {
            id: "test-1".to_string(),
            content: "Rust is great".to_string(),
            source: KnowledgeSource::Direct,
            confidence: 0.9,
            category: None,
            created_at: Utc::now(),
            last_accessed: Utc::now(),
            access_count: 5,
            success_associations: 2,
        }]
    }

    fn sample_schema() -> AgentSchema {
        AgentSchema {
            fields: vec![SchemaField {
                name: "role".to_string(),
                field_type: "string".to_string(),
                default: Some("assistant".to_string()),
            }],
            knowledge_config: Some(SchemaKnowledgeConfig {
                max_entries: Some(1000),
                retention_days: None,
            }),
        }
    }

    #[test]
    fn test_build_and_verify() {
        let pkg = build_package(
            "test-agent",
            "agent-001",
            None,
            sample_schema(),
            sample_entries(),
        );
        assert_eq!(pkg.format_version, "1.0");
        assert!(verify_integrity(&pkg).is_ok());
    }

    #[test]
    fn test_roundtrip() {
        let pkg = build_package(
            "test-agent",
            "agent-001",
            Some("A test".into()),
            sample_schema(),
            sample_entries(),
        );
        let json = serde_json::to_string_pretty(&pkg).unwrap();
        let loaded = load_package(&json).unwrap();
        assert!(verify_integrity(&loaded).is_ok());
        assert_eq!(loaded.metadata.agent_name, "test-agent");
    }

    #[test]
    fn test_unsupported_version() {
        let mut pkg = build_package("a", "b", None, sample_schema(), sample_entries());
        pkg.format_version = "99.0".to_string();
        let json = serde_json::to_string(&pkg).unwrap();
        let err = load_package(&json).unwrap_err();
        assert!(matches!(err, ImportError::UnsupportedVersion(_)));
    }

    #[test]
    fn test_integrity_mismatch() {
        let mut pkg = build_package("a", "b", None, sample_schema(), sample_entries());
        pkg.integrity.knowledge_hash = "tampered".to_string();
        assert!(matches!(
            verify_integrity(&pkg),
            Err(ImportError::IntegrityMismatch { .. })
        ));
    }

    #[test]
    fn test_prepare_imported_entries() {
        let pkg = build_package(
            "source-agent",
            "s-001",
            None,
            sample_schema(),
            sample_entries(),
        );
        let imported = prepare_imported_entries(&pkg, 0.8);

        assert_eq!(imported.len(), 1);
        assert!(imported[0].confidence <= 0.8);
        assert_eq!(imported[0].access_count, 0);
        assert_eq!(imported[0].success_associations, 0);
        assert!(
            matches!(&imported[0].source, KnowledgeSource::AgentTransfer { source_agent } if source_agent == "source-agent")
        );
    }

    #[test]
    fn test_inspect_package() {
        let pkg = build_package(
            "inspector",
            "i-001",
            Some("demo".into()),
            sample_schema(),
            sample_entries(),
        );
        let output = inspect_package(&pkg);
        assert!(output.contains("inspector"));
        assert!(output.contains("demo"));
        assert!(output.contains("1 entries"));
    }
}
