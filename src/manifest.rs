// FORGE project manifest (forge.project.toml) parser
// See issue #74 for specification

use serde::Deserialize;
use std::path::{Path, PathBuf};

// ── Manifest types ────────────────────────────────────────────

/// Top-level forge.project.toml structure.
#[derive(Debug, Deserialize)]
pub struct ProjectManifest {
    pub project: ProjectMeta,
    pub build: Option<BuildConfig>,
    pub config: Option<ConfigEmbed>,
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
        }
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
        let manifest = ProjectManifest::from_single_file(Path::new("examples/hello.forge"), None);
        assert_eq!(manifest.project.name, "hello");
        assert_eq!(manifest.output_name(), "hello");
    }

    #[test]
    fn single_file_with_output_override() {
        let manifest = ProjectManifest::from_single_file(
            Path::new("examples/quiz_tutor.forge"),
            Some("forge-tutor"),
        );
        assert_eq!(manifest.project.name, "forge-tutor");
        assert_eq!(manifest.output_name(), "forge-tutor");
    }
}
