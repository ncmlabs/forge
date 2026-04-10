// FORGE project manifest (forge.project.toml) parser
// See issue #74 for specification

use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ── Manifest types ────────────────────────────────────────────

/// Top-level forge.project.toml structure.
#[derive(Debug, Deserialize)]
pub struct ProjectManifest {
    pub project: ProjectMeta,
    pub build: Option<BuildConfig>,
    pub config: Option<ConfigEmbed>,
    pub skills: Option<HashMap<String, SkillDeclaration>>,
    pub adapters: Option<HashMap<String, AdapterReference>>,
}

/// A single adapter declaration in the [adapters] table.
/// Keys are adapter names (matching session `agent` field); values control
/// where to find the adapter's ADAPTER.toml.
#[derive(Debug, Deserialize, Clone)]
pub struct AdapterReference {
    /// Local path to adapter directory (relative to project root).
    pub path: Option<String>,
    /// Remote source identifier (future: registry).
    pub source: Option<String>,
}

/// A single skill declaration in the [skills] table.
/// Keys are skill names; values control where to find the skill.
#[derive(Debug, Deserialize, Clone)]
pub struct SkillDeclaration {
    /// Local path to skill directory (relative to project root).
    pub path: Option<String>,
    /// Remote source identifier (e.g., "microsoft/playwright-cli").
    pub source: Option<String>,
}

/// [project] section — required.
#[derive(Debug, Deserialize)]
pub struct ProjectMeta {
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
}

/// [build] section — optional, controls compilation behavior.
#[derive(Debug, Deserialize)]
pub struct BuildConfig {
    /// Entry point file (must contain fn main, agent, or system).
    pub entry: Option<String>,
    /// Output binary name (defaults to project name).
    pub output: Option<String>,
    /// Additional source files beyond entry.
    pub sources: Option<Vec<String>>,
}

/// [config] section — optional, embeds a forge.config.toml at build time.
#[derive(Debug, Deserialize)]
pub struct ConfigEmbed {
    /// Path to a forge.config.toml to embed in the binary.
    pub embed: Option<String>,
}

// ── Loading and resolution ────────────────────────────────────

impl ProjectManifest {
    /// Load a manifest from a TOML file.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("cannot read {}: {}", path.display(), e))?;
        let manifest: ProjectManifest = toml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("invalid manifest {}: {}", path.display(), e))?;
        Ok(manifest)
    }

    /// Output binary name — from [build].output, or project name.
    pub fn output_name(&self) -> &str {
        self.build
            .as_ref()
            .and_then(|b| b.output.as_deref())
            .unwrap_or(&self.project.name)
    }

    /// Resolve all source file paths relative to the manifest's parent directory.
    /// Returns (entry, all_sources) where entry is the first file.
    pub fn resolve_sources(&self, base_dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
        let mut paths = Vec::new();

        if let Some(build) = &self.build {
            // Add entry point first, if specified
            if let Some(entry) = &build.entry {
                let p = base_dir.join(entry);
                if !p.exists() {
                    anyhow::bail!("entry file not found: {}", p.display());
                }
                paths.push(p);
            }

            // Add additional sources
            if let Some(sources) = &build.sources {
                for src in sources {
                    let p = base_dir.join(src);
                    if !p.exists() {
                        anyhow::bail!("source file not found: {}", p.display());
                    }
                    paths.push(p);
                }
            }
        }

        if paths.is_empty() {
            anyhow::bail!("no source files specified in manifest");
        }

        Ok(paths)
    }

    /// Resolve the embedded config file path, if specified.
    pub fn resolve_embedded_config(&self, base_dir: &Path) -> anyhow::Result<Option<PathBuf>> {
        if let Some(cfg) = &self.config {
            if let Some(embed) = &cfg.embed {
                let p = base_dir.join(embed);
                if !p.exists() {
                    anyhow::bail!("embedded config not found: {}", p.display());
                }
                return Ok(Some(p));
            }
        }
        Ok(None)
    }

    /// Resolve declared skills to their SKILL.md paths on disk.
    ///
    /// Resolution chain per skill:
    ///   1. Explicit `path` (if declared) → `{base_dir}/{path}/SKILL.md`
    ///   2. Project local: `{base_dir}/skills/{name}/SKILL.md`
    ///   3. Project installed: `{base_dir}/.agents/skills/{name}/SKILL.md`
    ///   4. Global dirs (from config `skill_dirs`, default `~/.forge/skills/`)
    ///
    /// Returns `(skill_name, path_to_skill_md)` for each declared skill,
    /// or an error listing all searched paths for missing skills.
    pub fn resolve_skills(
        &self,
        base_dir: &Path,
        global_dirs: &[PathBuf],
    ) -> anyhow::Result<Vec<(String, PathBuf)>> {
        let skills = match &self.skills {
            Some(s) => s,
            None => return Ok(Vec::new()),
        };

        let mut resolved = Vec::new();

        for (name, decl) in skills {
            let mut searched = Vec::new();

            // 1. Explicit path
            if let Some(ref path) = decl.path {
                let skill_md = base_dir.join(path).join("SKILL.md");
                searched.push(skill_md.clone());
                if skill_md.exists() {
                    resolved.push((name.clone(), skill_md));
                    continue;
                }
                // Explicit path is authoritative — don't fall through
                anyhow::bail!(
                    "skill '{}' path not found: {}\n  hint: create {}/SKILL.md or fix the path in [skills.{}]",
                    name,
                    skill_md.display(),
                    base_dir.join(path).display(),
                    name,
                );
            }

            // 2. Project local: ./skills/{name}/
            let local = base_dir.join("skills").join(name).join("SKILL.md");
            searched.push(local.clone());
            if local.exists() {
                resolved.push((name.clone(), local));
                continue;
            }

            // 3. Project installed: .agents/skills/{name}/
            let agents = base_dir.join(".agents/skills").join(name).join("SKILL.md");
            searched.push(agents.clone());
            if agents.exists() {
                resolved.push((name.clone(), agents));
                continue;
            }

            // 4. Global dirs
            let mut found = false;
            for dir in global_dirs {
                let global = dir.join(name).join("SKILL.md");
                searched.push(global.clone());
                if global.exists() {
                    resolved.push((name.clone(), global));
                    found = true;
                    break;
                }
            }
            if found {
                continue;
            }

            // Not found anywhere
            let searched_list = searched
                .iter()
                .map(|p| format!("    {}", p.display()))
                .collect::<Vec<_>>()
                .join("\n");
            anyhow::bail!(
                "skill '{}' declared in forge.project.toml but not found\n  searched:\n{}\n  hint: run `npx skills add {}` or set path = \"...\" in [skills.{}]",
                name,
                searched_list,
                name,
                name,
            );
        }

        Ok(resolved)
    }

    /// Create a virtual manifest for single-file builds (no forge.project.toml needed).
    pub fn from_single_file(file: &Path, output: Option<&str>) -> Self {
        let name = output
            .map(|s| s.to_string())
            .or_else(|| file.file_stem().map(|s| s.to_string_lossy().to_string()))
            .unwrap_or_else(|| "forge-program".to_string());

        let entry = file
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| file.display().to_string());

        ProjectManifest {
            project: ProjectMeta {
                name: name.clone(),
                version: Some("0.1.0".to_string()),
                description: None,
            },
            build: Some(BuildConfig {
                entry: Some(entry),
                output: Some(name),
                sources: None,
            }),
            config: None,
            skills: None,
            adapters: None,
        }
    }

    /// Resolve declared adapters to their ADAPTER.toml paths on disk.
    ///
    /// Resolution chain per adapter:
    ///   1. Explicit `path` (if declared) → `{base_dir}/{path}/ADAPTER.toml`
    ///   2. Project local: `{base_dir}/adapters/{name}/ADAPTER.toml`
    ///   3. Project installed: `{base_dir}/.forge-data/adapters/{name}/ADAPTER.toml`
    ///   4. Built-in dirs (shipped with forge binary)
    pub fn resolve_adapters(
        &self,
        base_dir: &Path,
        builtin_dirs: &[PathBuf],
    ) -> anyhow::Result<Vec<(String, PathBuf)>> {
        let adapters = match &self.adapters {
            Some(a) => a,
            None => return Ok(Vec::new()),
        };

        let mut resolved = Vec::new();

        for (name, decl) in adapters {
            let mut searched = Vec::new();

            // 1. Explicit path
            if let Some(ref path) = decl.path {
                let adapter_toml = base_dir.join(path).join("ADAPTER.toml");
                searched.push(adapter_toml.clone());
                if adapter_toml.exists() {
                    resolved.push((name.clone(), adapter_toml));
                    continue;
                }
                anyhow::bail!(
                    "adapter '{}' path not found: {}\n  hint: create {}/ADAPTER.toml or fix the path in [adapters.{}]",
                    name,
                    adapter_toml.display(),
                    base_dir.join(path).display(),
                    name,
                );
            }

            // 2. Project local: ./adapters/{name}/
            let local = base_dir.join("adapters").join(name).join("ADAPTER.toml");
            searched.push(local.clone());
            if local.exists() {
                resolved.push((name.clone(), local));
                continue;
            }

            // 3. Project installed: .forge-data/adapters/{name}/
            let installed = base_dir
                .join(".forge-data/adapters")
                .join(name)
                .join("ADAPTER.toml");
            searched.push(installed.clone());
            if installed.exists() {
                resolved.push((name.clone(), installed));
                continue;
            }

            // 4. Built-in dirs
            let mut found = false;
            for dir in builtin_dirs {
                let builtin = dir.join(name).join("ADAPTER.toml");
                searched.push(builtin.clone());
                if builtin.exists() {
                    resolved.push((name.clone(), builtin));
                    found = true;
                    break;
                }
            }
            if found {
                continue;
            }

            let searched_list = searched
                .iter()
                .map(|p| format!("    {}", p.display()))
                .collect::<Vec<_>>()
                .join("\n");
            anyhow::bail!(
                "adapter '{}' declared in forge.project.toml but not found\n  searched:\n{}\n  hint: create adapters/{}/ADAPTER.toml or set path = \"...\" in [adapters.{}]",
                name,
                searched_list,
                name,
                name,
            );
        }

        Ok(resolved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_manifest() {
        let toml = r#"
[project]
name = "hello"
"#;
        let manifest: ProjectManifest = toml::from_str(toml).unwrap();
        assert_eq!(manifest.project.name, "hello");
        assert!(manifest.build.is_none());
        assert!(manifest.config.is_none());
    }

    #[test]
    fn parse_full_manifest() {
        let toml = r#"
[project]
name = "tictactoe"
version = "0.1.0"

[build]
entry = "platform.forge"
output = "tictactoe"
sources = [
  "room_agent.forge",
  "ai_opponent.forge",
  "matchmaking.forge",
]

[config]
embed = "forge.config.toml"
"#;
        let manifest: ProjectManifest = toml::from_str(toml).unwrap();
        assert_eq!(manifest.project.name, "tictactoe");
        assert_eq!(manifest.output_name(), "tictactoe");
        let build = manifest.build.as_ref().unwrap();
        assert_eq!(build.entry.as_deref(), Some("platform.forge"));
        assert_eq!(build.sources.as_ref().unwrap().len(), 3);
        assert_eq!(
            manifest.config.as_ref().unwrap().embed.as_deref(),
            Some("forge.config.toml")
        );
    }

    #[test]
    fn output_name_defaults_to_project() {
        let toml = r#"
[project]
name = "myapp"

[build]
entry = "main.forge"
"#;
        let manifest: ProjectManifest = toml::from_str(toml).unwrap();
        assert_eq!(manifest.output_name(), "myapp");
    }

    #[test]
    fn single_file_virtual_manifest() {
        let manifest =
            ProjectManifest::from_single_file(Path::new("examples/basics/hello.forge"), None);
        assert_eq!(manifest.project.name, "hello");
        assert_eq!(manifest.output_name(), "hello");
    }

    #[test]
    fn single_file_with_output_override() {
        let manifest = ProjectManifest::from_single_file(
            Path::new("examples/agents/quiz_tutor.forge"),
            Some("forge-tutor"),
        );
        assert_eq!(manifest.project.name, "forge-tutor");
        assert_eq!(manifest.output_name(), "forge-tutor");
    }

    #[test]
    fn parse_manifest_with_skills() {
        let toml = r#"
[project]
name = "myapp"

[skills]
github = {}
slack = { path = "skills/slack" }
playwright = { source = "microsoft/playwright-cli" }
"#;
        let manifest: ProjectManifest = toml::from_str(toml).unwrap();
        let skills = manifest.skills.as_ref().unwrap();
        assert_eq!(skills.len(), 3);
        assert!(skills["github"].path.is_none());
        assert!(skills["github"].source.is_none());
        assert_eq!(skills["slack"].path.as_deref(), Some("skills/slack"));
        assert_eq!(
            skills["playwright"].source.as_deref(),
            Some("microsoft/playwright-cli")
        );
    }

    #[test]
    fn parse_manifest_without_skills() {
        let toml = r#"
[project]
name = "hello"
"#;
        let manifest: ProjectManifest = toml::from_str(toml).unwrap();
        assert!(manifest.skills.is_none());
    }

    #[test]
    fn resolve_skills_from_local_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("skills/myskill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "---\nname: myskill\n---\nHello").unwrap();

        let manifest = ProjectManifest {
            project: ProjectMeta {
                name: "test".into(),
                version: None,
                description: None,
            },
            build: None,
            config: None,
            skills: Some(HashMap::from([(
                "myskill".into(),
                SkillDeclaration {
                    path: None,
                    source: None,
                },
            )])),
            adapters: None,
        };

        let resolved = manifest.resolve_skills(tmp.path(), &[]).unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].0, "myskill");
        assert!(resolved[0].1.ends_with("SKILL.md"));
    }

    #[test]
    fn resolve_skills_from_agents_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join(".agents/skills/myskill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "---\nname: myskill\n---\nHello").unwrap();

        let manifest = ProjectManifest {
            project: ProjectMeta {
                name: "test".into(),
                version: None,
                description: None,
            },
            build: None,
            config: None,
            skills: Some(HashMap::from([(
                "myskill".into(),
                SkillDeclaration {
                    path: None,
                    source: None,
                },
            )])),
            adapters: None,
        };

        let resolved = manifest.resolve_skills(tmp.path(), &[]).unwrap();
        assert_eq!(resolved.len(), 1);
        assert!(resolved[0].1.to_string_lossy().contains(".agents/skills"));
    }

    #[test]
    fn resolve_skills_explicit_path() {
        let tmp = tempfile::tempdir().unwrap();
        let custom = tmp.path().join("custom/loc");
        std::fs::create_dir_all(&custom).unwrap();
        std::fs::write(custom.join("SKILL.md"), "---\nname: x\n---\nOk").unwrap();

        let manifest = ProjectManifest {
            project: ProjectMeta {
                name: "test".into(),
                version: None,
                description: None,
            },
            build: None,
            config: None,
            skills: Some(HashMap::from([(
                "x".into(),
                SkillDeclaration {
                    path: Some("custom/loc".into()),
                    source: None,
                },
            )])),
            adapters: None,
        };

        let resolved = manifest.resolve_skills(tmp.path(), &[]).unwrap();
        assert_eq!(resolved.len(), 1);
        assert!(resolved[0].1.to_string_lossy().contains("custom/loc"));
    }

    #[test]
    fn resolve_skills_local_wins_over_global() {
        let tmp = tempfile::tempdir().unwrap();
        // Create both local and global
        let local = tmp.path().join("skills/myskill");
        std::fs::create_dir_all(&local).unwrap();
        std::fs::write(local.join("SKILL.md"), "---\nname: myskill\n---\nLocal").unwrap();

        let global_dir = tmp.path().join("global");
        let global = global_dir.join("myskill");
        std::fs::create_dir_all(&global).unwrap();
        std::fs::write(global.join("SKILL.md"), "---\nname: myskill\n---\nGlobal").unwrap();

        let manifest = ProjectManifest {
            project: ProjectMeta {
                name: "test".into(),
                version: None,
                description: None,
            },
            build: None,
            config: None,
            skills: Some(HashMap::from([(
                "myskill".into(),
                SkillDeclaration {
                    path: None,
                    source: None,
                },
            )])),
            adapters: None,
        };

        let resolved = manifest.resolve_skills(tmp.path(), &[global_dir]).unwrap();
        assert_eq!(resolved.len(), 1);
        // Should find local, not global (use OS-agnostic path check)
        let path_str = resolved[0].1.to_string_lossy();
        assert!(
            path_str.contains("skills/myskill") || path_str.contains("skills\\myskill"),
            "should resolve from local skills dir, got: {}",
            path_str
        );
        assert!(!path_str.contains("global"));
    }

    #[test]
    fn resolve_skills_missing_gives_clear_error() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest = ProjectManifest {
            project: ProjectMeta {
                name: "test".into(),
                version: None,
                description: None,
            },
            build: None,
            config: None,
            skills: Some(HashMap::from([(
                "nonexistent".into(),
                SkillDeclaration {
                    path: None,
                    source: None,
                },
            )])),
            adapters: None,
        };

        let err = manifest.resolve_skills(tmp.path(), &[]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("nonexistent"));
        assert!(msg.contains("declared in forge.project.toml but not found"));
        assert!(msg.contains("searched:"));
    }

    #[test]
    fn resolve_skills_explicit_path_missing_gives_error() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest = ProjectManifest {
            project: ProjectMeta {
                name: "test".into(),
                version: None,
                description: None,
            },
            build: None,
            config: None,
            skills: Some(HashMap::from([(
                "x".into(),
                SkillDeclaration {
                    path: Some("does/not/exist".into()),
                    source: None,
                },
            )])),
            adapters: None,
        };

        let err = manifest.resolve_skills(tmp.path(), &[]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("path not found"));
        assert!(msg.contains("does/not/exist"));
    }

    #[test]
    fn parse_manifest_with_adapters() {
        let toml = r#"
[project]
name = "myapp"

[adapters]
claude = {}
codex = {}
opencode = { path = "adapters/opencode" }
"#;
        let manifest: ProjectManifest = toml::from_str(toml).unwrap();
        let adapters = manifest.adapters.as_ref().unwrap();
        assert_eq!(adapters.len(), 3);
        assert!(adapters["claude"].path.is_none());
        assert_eq!(
            adapters["opencode"].path.as_deref(),
            Some("adapters/opencode")
        );
    }

    #[test]
    fn resolve_adapters_from_local_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let adapter_dir = tmp.path().join("adapters/myagent");
        std::fs::create_dir_all(&adapter_dir).unwrap();
        std::fs::write(
            adapter_dir.join("ADAPTER.toml"),
            "[adapter]\nname = \"myagent\"\ncommand = \"myagent\"",
        )
        .unwrap();

        let manifest = ProjectManifest {
            project: ProjectMeta {
                name: "test".into(),
                version: None,
                description: None,
            },
            build: None,
            config: None,
            skills: None,
            adapters: Some(HashMap::from([(
                "myagent".into(),
                AdapterReference {
                    path: None,
                    source: None,
                },
            )])),
        };

        let resolved = manifest.resolve_adapters(tmp.path(), &[]).unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].0, "myagent");
        assert!(resolved[0].1.ends_with("ADAPTER.toml"));
    }

    #[test]
    fn resolve_adapters_from_builtin_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let builtin_dir = tmp.path().join("builtin");
        let adapter = builtin_dir.join("claude");
        std::fs::create_dir_all(&adapter).unwrap();
        std::fs::write(
            adapter.join("ADAPTER.toml"),
            "[adapter]\nname = \"claude\"\ncommand = \"claude\"",
        )
        .unwrap();

        let manifest = ProjectManifest {
            project: ProjectMeta {
                name: "test".into(),
                version: None,
                description: None,
            },
            build: None,
            config: None,
            skills: None,
            adapters: Some(HashMap::from([(
                "claude".into(),
                AdapterReference {
                    path: None,
                    source: None,
                },
            )])),
        };

        let resolved = manifest
            .resolve_adapters(tmp.path(), &[builtin_dir])
            .unwrap();
        assert_eq!(resolved.len(), 1);
        assert!(resolved[0].1.to_string_lossy().contains("builtin"));
    }

    #[test]
    fn resolve_adapters_missing_gives_clear_error() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest = ProjectManifest {
            project: ProjectMeta {
                name: "test".into(),
                version: None,
                description: None,
            },
            build: None,
            config: None,
            skills: None,
            adapters: Some(HashMap::from([(
                "nonexistent".into(),
                AdapterReference {
                    path: None,
                    source: None,
                },
            )])),
        };

        let err = manifest.resolve_adapters(tmp.path(), &[]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("nonexistent"));
        assert!(msg.contains("declared in forge.project.toml but not found"));
    }

    #[test]
    fn resolve_skills_from_global_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let global_dir = tmp.path().join("global-skills");
        let skill = global_dir.join("myskill");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), "---\nname: myskill\n---\nGlobal").unwrap();

        let manifest = ProjectManifest {
            project: ProjectMeta {
                name: "test".into(),
                version: None,
                description: None,
            },
            build: None,
            config: None,
            skills: Some(HashMap::from([(
                "myskill".into(),
                SkillDeclaration {
                    path: None,
                    source: None,
                },
            )])),
            adapters: None,
        };

        let resolved = manifest.resolve_skills(tmp.path(), &[global_dir]).unwrap();
        assert_eq!(resolved.len(), 1);
        assert!(resolved[0].1.to_string_lossy().contains("global-skills"));
    }
}
