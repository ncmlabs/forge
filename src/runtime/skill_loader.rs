// FORGE SKILL.md loader — issue #40
// Parses SKILL.md files from directories and registers them as skills.

use std::path::{Path, PathBuf};

use crate::runtime::skill::{LoadedSkill, SkillCapability, SkillManifest};
use crate::types::{CapabilitySignature, ForgeType};
use serde::Deserialize;

/// Loads SKILL.md files from configured directories.
pub struct SkillLoader;

impl SkillLoader {
    /// Scan directories for SKILL.md files, parse frontmatter, return loaded skills.
    pub fn load_from_dirs(dirs: &[PathBuf]) -> Vec<LoadedSkill> {
        let mut skills = Vec::new();
        for dir in dirs {
            if !dir.exists() {
                continue;
            }
            Self::scan_dir(dir, &mut skills);
        }
        skills
    }

    fn scan_dir(dir: &Path, skills: &mut Vec<LoadedSkill>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Check for SKILL.md inside subdirectory
                let skill_path = path.join("SKILL.md");
                if skill_path.exists() {
                    if let Ok(skill) = Self::parse_skill_md(&skill_path) {
                        skills.push(skill);
                    }
                }
            } else if path.file_name().map(|f| f == "SKILL.md").unwrap_or(false) {
                if let Ok(skill) = Self::parse_skill_md(&path) {
                    skills.push(skill);
                }
            }
        }
    }

    /// Parse a SKILL.md file into a LoadedSkill.
    pub fn parse_skill_md(path: &Path) -> Result<LoadedSkill, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read {}: {}", path.display(), e))?;

        let (frontmatter, body) = split_frontmatter(&content)?;
        let capabilities = frontmatter.parse_capabilities()?;
        let legacy_signature = if frontmatter.capabilities.is_some() {
            None
        } else {
            Some(CapabilitySignature {
                inputs: vec![ForgeType::Text],
                output: ForgeType::Text,
            })
        };

        let manifest = SkillManifest {
            name: frontmatter.name,
            description: frontmatter.description.unwrap_or_default(),
            capabilities,
            legacy_signature,
            default_confidence: 0.85,
            timeout_secs: frontmatter.timeout.unwrap_or(30),
            allowed_tools: frontmatter.allowed_tools,
        };

        Ok(LoadedSkill {
            manifest,
            instructions: body,
            path: path.to_path_buf(),
        })
    }
}

/// Parsed SKILL.md YAML frontmatter.
#[derive(Debug, Deserialize)]
struct SkillFrontmatter {
    name: String,
    description: Option<String>,
    timeout: Option<u64>,
    #[serde(
        rename = "allowed-tools",
        default,
        deserialize_with = "deserialize_tools"
    )]
    allowed_tools: Vec<String>,
    capabilities: Option<Vec<SkillCapabilityFrontmatter>>,
}

#[derive(Debug, Deserialize)]
struct SkillCapabilityFrontmatter {
    name: String,
    inputs: Vec<String>,
    output: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ToolList {
    One(String),
    Many(Vec<String>),
}

impl SkillFrontmatter {
    fn parse_capabilities(&self) -> Result<Vec<SkillCapability>, String> {
        self.capabilities
            .as_ref()
            .map(|caps| {
                caps.iter()
                    .map(|cap| {
                        Ok(SkillCapability {
                            name: cap.name.clone(),
                            signature: CapabilitySignature {
                                inputs: cap
                                    .inputs
                                    .iter()
                                    .map(|ty| parse_forge_type(ty))
                                    .collect::<Result<Vec<_>, _>>()?,
                                output: parse_forge_type(&cap.output)?,
                            },
                        })
                    })
                    .collect()
            })
            .unwrap_or_else(|| Ok(Vec::new()))
    }
}

/// Split a SKILL.md file into frontmatter and body.
fn split_frontmatter(content: &str) -> Result<(SkillFrontmatter, String), String> {
    let content = content.trim();
    if !content.starts_with("---") {
        return Err("SKILL.md must start with --- frontmatter".to_string());
    }

    let after_first = &content[3..];
    let end_idx = after_first
        .find("---")
        .ok_or_else(|| "SKILL.md frontmatter not closed with ---".to_string())?;

    let yaml_str = &after_first[..end_idx].trim();
    let body = after_first[end_idx + 3..].trim().to_string();

    let frontmatter = parse_frontmatter(yaml_str)?;
    Ok((frontmatter, body))
}

fn parse_frontmatter(yaml: &str) -> Result<SkillFrontmatter, String> {
    let frontmatter: SkillFrontmatter =
        serde_yaml::from_str(yaml).map_err(|e| format!("invalid SKILL.md frontmatter: {e}"))?;
    if frontmatter.name.trim().is_empty() {
        return Err("SKILL.md frontmatter must have a 'name' field".to_string());
    }
    Ok(frontmatter)
}

fn deserialize_tools<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let tools = Option::<ToolList>::deserialize(deserializer)?;
    Ok(match tools {
        Some(ToolList::One(tool)) => tool.split_whitespace().map(|s| s.to_string()).collect(),
        Some(ToolList::Many(tools)) => tools,
        None => Vec::new(),
    })
}

fn parse_forge_type(name: &str) -> Result<ForgeType, String> {
    let name = name.trim();
    if let Some(inner) = name.strip_suffix("[]") {
        return Ok(ForgeType::Array(Box::new(parse_forge_type(inner)?), None));
    }
    match name {
        "Text" => Ok(ForgeType::Text),
        "Number" => Ok(ForgeType::Number),
        "Bool" => Ok(ForgeType::Bool),
        "Unit" => Ok(ForgeType::Unit),
        "Results" => Ok(ForgeType::Results),
        "Report" => Ok(ForgeType::Report),
        "Intent" => Ok(ForgeType::Intent),
        "Classification" => Ok(ForgeType::Classification),
        "Summary" => Ok(ForgeType::Summary),
        "Failure" => Ok(ForgeType::Failure),
        "Conversation" => Ok(ForgeType::Conversation),
        "Profile" => Ok(ForgeType::Profile),
        "SearchResults" => Ok(ForgeType::SearchResults),
        "Request" => Ok(ForgeType::Request),
        "Response" => Ok(ForgeType::Response),
        "Headers" => Ok(ForgeType::Headers),
        "Html" => Ok(ForgeType::Html),
        "Embedding" => Ok(ForgeType::Embedding),
        "Duration" => Ok(ForgeType::Duration),
        other => Ok(ForgeType::Named(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_skill_md_frontmatter() {
        let content = r#"---
name: slack-post
description: Post messages to Slack channels
timeout: 15
allowed-tools:
  - Bash
  - WebFetch
---

Follow these instructions to post a message to Slack.

Use curl to call the Slack API.
"#;
        let (fm, body) = split_frontmatter(content).unwrap();
        assert_eq!(fm.name, "slack-post");
        assert_eq!(
            fm.description.as_deref(),
            Some("Post messages to Slack channels")
        );
        assert_eq!(fm.timeout, Some(15));
        assert_eq!(fm.allowed_tools, vec!["Bash", "WebFetch"]);
        assert!(body.contains("Follow these instructions"));
    }

    #[test]
    fn parse_minimal_frontmatter() {
        let content = "---\nname: test-skill\n---\nBody here.";
        let (fm, body) = split_frontmatter(content).unwrap();
        assert_eq!(fm.name, "test-skill");
        assert!(fm.description.is_none());
        assert_eq!(fm.timeout, None);
        assert!(fm.allowed_tools.is_empty());
        assert_eq!(body, "Body here.");
    }

    #[test]
    fn reject_missing_name() {
        let content = "---\ndescription: no name\n---\nBody.";
        assert!(split_frontmatter(content).is_err());
    }

    #[test]
    fn reject_missing_frontmatter() {
        let content = "No frontmatter here.";
        assert!(split_frontmatter(content).is_err());
    }

    #[test]
    fn parse_typed_capabilities() {
        let content = r#"---
name: github
capabilities:
  - name: create_issue
    inputs: [Text, Text, Text]
    output: Text
  - name: list_issues
    inputs: [Text]
    output: Text
---
Body.
"#;
        let (fm, _) = split_frontmatter(content).unwrap();
        let caps = fm.parse_capabilities().unwrap();
        assert_eq!(caps.len(), 2);
        assert_eq!(caps[0].name, "create_issue");
        assert_eq!(caps[0].signature.inputs.len(), 3);
        assert_eq!(caps[1].signature.output, ForgeType::Text);
    }
}
