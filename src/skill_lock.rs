// FORGE skill lock file — integrity verification for installed skills.
// Compatible with the skills-lock.json format produced by `npx skills add`.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
pub struct SkillLockFile {
    pub version: u32,
    pub skills: HashMap<String, LockedSkill>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LockedSkill {
    pub source: Option<String>,
    #[serde(rename = "sourceType")]
    pub source_type: Option<String>,
    #[serde(rename = "computedHash")]
    pub computed_hash: String,
}

impl SkillLockFile {
    /// Load a lock file from disk. Returns `Ok(None)` if the file doesn't exist.
    pub fn load(path: &Path) -> anyhow::Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("cannot read {}: {}", path.display(), e))?;
        let lock: SkillLockFile = serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("invalid lock file {}: {}", path.display(), e))?;
        Ok(Some(lock))
    }

    /// Verify resolved skills against lock file hashes.
    /// Returns the names of skills whose SKILL.md content doesn't match.
    pub fn verify(&self, resolved_skills: &[(String, PathBuf)]) -> Vec<String> {
        let mut mismatched = Vec::new();
        for (name, path) in resolved_skills {
            if let Some(locked) = self.skills.get(name) {
                if let Ok(hash) = Self::hash_file(path) {
                    if hash != locked.computed_hash {
                        mismatched.push(name.clone());
                    }
                }
            }
        }
        mismatched
    }

    /// Compute SHA-256 hash of a file's contents, returned as hex string.
    pub fn hash_file(path: &Path) -> anyhow::Result<String> {
        let content = std::fs::read(path)
            .map_err(|e| anyhow::anyhow!("cannot read {}: {}", path.display(), e))?;
        let mut hasher = Sha256::new();
        hasher.update(&content);
        Ok(format!("{:x}", hasher.finalize()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_existing_lock_format() {
        let json = r#"{
  "version": 1,
  "skills": {
    "playwright-cli": {
      "source": "microsoft/playwright-cli",
      "sourceType": "github",
      "computedHash": "45b90b409fb80cf08ee4e8b72377bddacfac3d1c1fac69bff5fd5342fad26adf"
    }
  }
}"#;
        let lock: SkillLockFile = serde_json::from_str(json).unwrap();
        assert_eq!(lock.version, 1);
        assert_eq!(lock.skills.len(), 1);
        let skill = &lock.skills["playwright-cli"];
        assert_eq!(skill.source.as_deref(), Some("microsoft/playwright-cli"));
        assert_eq!(skill.source_type.as_deref(), Some("github"));
        assert!(!skill.computed_hash.is_empty());
    }

    #[test]
    fn load_missing_file_returns_none() {
        let result = SkillLockFile::load(Path::new("/tmp/nonexistent-lock-file.json")).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn hash_file_produces_consistent_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("test.md");
        std::fs::write(&file, "hello world").unwrap();

        let h1 = SkillLockFile::hash_file(&file).unwrap();
        let h2 = SkillLockFile::hash_file(&file).unwrap();
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // SHA-256 hex
    }

    #[test]
    fn verify_matching_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("SKILL.md");
        std::fs::write(&file, "---\nname: test\n---\nContent").unwrap();
        let hash = SkillLockFile::hash_file(&file).unwrap();

        let lock = SkillLockFile {
            version: 1,
            skills: HashMap::from([(
                "test".into(),
                LockedSkill {
                    source: None,
                    source_type: None,
                    computed_hash: hash,
                },
            )]),
        };

        let mismatched = lock.verify(&[("test".into(), file)]);
        assert!(mismatched.is_empty());
    }

    #[test]
    fn verify_mismatched_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("SKILL.md");
        std::fs::write(&file, "---\nname: test\n---\nChanged content").unwrap();

        let lock = SkillLockFile {
            version: 1,
            skills: HashMap::from([(
                "test".into(),
                LockedSkill {
                    source: None,
                    source_type: None,
                    computed_hash:
                        "0000000000000000000000000000000000000000000000000000000000000000".into(),
                },
            )]),
        };

        let mismatched = lock.verify(&[("test".into(), file)]);
        assert_eq!(mismatched, vec!["test"]);
    }

    #[test]
    fn verify_unlocked_skill_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("SKILL.md");
        std::fs::write(&file, "content").unwrap();

        let lock = SkillLockFile {
            version: 1,
            skills: HashMap::new(),
        };

        // Skill not in lock file → not checked, no mismatch
        let mismatched = lock.verify(&[("unlocked".into(), file)]);
        assert!(mismatched.is_empty());
    }
}
