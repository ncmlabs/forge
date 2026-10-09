//! `forge init <dir> --template <template>` — project scaffolding (#481).
//!
//! Every template is a static table of `(relative path, contents)` pairs whose
//! contents are embedded with `include_str!`, so a built binary carries its
//! templates with it and `forge init` never needs a data directory beside it.
//! `{{name}}` in a file body is replaced by the target directory's base name;
//! nothing else is templated.

use anyhow::{bail, Context, Result};
use std::path::Path;

/// Template names accepted by `--template`, in listing order.
pub const TEMPLATE_NAMES: &[&str] = &["pipeline", "agent", "webhook-bot"];

const PIPELINE: &[(&str, &str)] = &[
    (
        "forge.project.toml",
        include_str!("../templates/init/pipeline/forge.project.toml"),
    ),
    (
        "forge.config.toml",
        include_str!("../templates/init/pipeline/forge.config.toml"),
    ),
    (
        "main.forge",
        include_str!("../templates/init/pipeline/main.forge"),
    ),
    (
        "main.forge.fixtures.json",
        include_str!("../templates/init/pipeline/main.forge.fixtures.json"),
    ),
    (
        "expected.txt",
        include_str!("../templates/init/pipeline/expected.txt"),
    ),
    (
        ".gitignore",
        include_str!("../templates/init/pipeline/.gitignore"),
    ),
    (
        "AGENTS.md",
        include_str!("../templates/init/pipeline/AGENTS.md"),
    ),
];

const AGENT: &[(&str, &str)] = &[
    (
        "forge.project.toml",
        include_str!("../templates/init/agent/forge.project.toml"),
    ),
    (
        "forge.config.toml",
        include_str!("../templates/init/agent/forge.config.toml"),
    ),
    (
        "main.forge",
        include_str!("../templates/init/agent/main.forge"),
    ),
    (
        "main.forge.fixtures.json",
        include_str!("../templates/init/agent/main.forge.fixtures.json"),
    ),
    (
        "expected.txt",
        include_str!("../templates/init/agent/expected.txt"),
    ),
    (
        ".gitignore",
        include_str!("../templates/init/agent/.gitignore"),
    ),
    (
        "AGENTS.md",
        include_str!("../templates/init/agent/AGENTS.md"),
    ),
];

const WEBHOOK_BOT: &[(&str, &str)] = &[
    (
        "forge.project.toml",
        include_str!("../templates/init/webhook-bot/forge.project.toml"),
    ),
    (
        "forge.config.toml",
        include_str!("../templates/init/webhook-bot/forge.config.toml"),
    ),
    (
        "main.forge",
        include_str!("../templates/init/webhook-bot/main.forge"),
    ),
    (
        "main.forge.fixtures.json",
        include_str!("../templates/init/webhook-bot/main.forge.fixtures.json"),
    ),
    (
        "expected.txt",
        include_str!("../templates/init/webhook-bot/expected.txt"),
    ),
    (
        ".gitignore",
        include_str!("../templates/init/webhook-bot/.gitignore"),
    ),
    (
        "AGENTS.md",
        include_str!("../templates/init/webhook-bot/AGENTS.md"),
    ),
];

/// The file table for `template`, or `None` if the name is unknown.
pub fn files(template: &str) -> Option<&'static [(&'static str, &'static str)]> {
    match template {
        "pipeline" => Some(PIPELINE),
        "agent" => Some(AGENT),
        "webhook-bot" => Some(WEBHOOK_BOT),
        _ => None,
    }
}

/// Substitute the one template variable: `{{name}}`.
pub fn render(contents: &str, name: &str) -> String {
    contents.replace("{{name}}", name)
}

/// Write `template` into `dir` and return the relative paths written — or, for
/// `dry_run`, the paths that would be written, with nothing touched.
///
/// A non-empty `dir` is refused unless `force`; with `force`, only the template
/// files are overwritten, never anything else.
pub fn scaffold(
    dir: &Path,
    template: &str,
    force: bool,
    dry_run: bool,
) -> Result<Vec<&'static str>> {
    let Some(files) = files(template) else {
        bail!(
            "unknown template `{template}`; expected one of {}",
            TEMPLATE_NAMES.join(", ")
        );
    };

    if dir.exists() {
        if !dir.is_dir() {
            bail!("{} exists and is not a directory", dir.display());
        }
        if !force && has_entries(dir)? {
            bail!(
                "{} is not empty; use --force to write into it",
                dir.display()
            );
        }
    }

    let paths: Vec<&'static str> = files.iter().map(|(path, _)| *path).collect();
    if dry_run {
        return Ok(paths);
    }

    let name = project_name(dir)?;
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    for (path, contents) in files {
        let target = dir.join(path);
        std::fs::write(&target, render(contents, &name))
            .with_context(|| format!("cannot write {}", target.display()))?;
    }
    Ok(paths)
}

/// `{{name}}` — the target directory's base name. A target that does not exist
/// yet is named from the path as given; `.` and `..` resolve to the directory
/// they point at.
fn project_name(dir: &Path) -> Result<String> {
    let resolved = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    resolved
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .with_context(|| format!("cannot derive a project name from {}", dir.display()))
}

fn has_entries(dir: &Path) -> Result<bool> {
    let mut entries =
        std::fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))?;
    Ok(entries.next().transpose()?.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expected_files() -> Vec<&'static str> {
        vec![
            "forge.project.toml",
            "forge.config.toml",
            "main.forge",
            "main.forge.fixtures.json",
            "expected.txt",
            ".gitignore",
            "AGENTS.md",
        ]
    }

    #[test]
    fn render_substitutes_the_directory_name() {
        assert_eq!(render("name = \"{{name}}\"", "demo"), "name = \"demo\"");
        assert_eq!(render("nothing to do", "demo"), "nothing to do");
    }

    #[test]
    fn every_template_scaffolds_the_same_file_set() {
        let dir = tempfile::tempdir().expect("tempdir");
        for template in TEMPLATE_NAMES {
            let target = dir.path().join(format!("proj-{template}"));
            let written = scaffold(&target, template, false, false).expect("scaffold");
            assert_eq!(written, expected_files(), "template {template}");
            for path in &written {
                assert!(
                    target.join(path).is_file(),
                    "template {template}: {path} missing"
                );
            }
        }
    }

    #[test]
    fn scaffold_substitutes_the_name_in_file_bodies() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("demo");
        scaffold(&target, "pipeline", false, false).expect("scaffold");

        let project = std::fs::read_to_string(target.join("forge.project.toml")).expect("read");
        assert!(project.contains("name = \"demo\""), "{project}");
        assert!(!project.contains("{{name}}"), "{project}");
        let agents = std::fs::read_to_string(target.join("AGENTS.md")).expect("read");
        assert!(agents.starts_with("# demo"), "{agents}");
    }

    #[test]
    fn scaffold_refuses_a_non_empty_directory_without_force() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("keep.txt"), "mine").expect("write");

        let err = scaffold(dir.path(), "pipeline", false, false)
            .expect_err("non-empty dir must be refused")
            .to_string();
        assert!(
            err.contains("is not empty; use --force to write into it"),
            "{err}"
        );
        assert_eq!(
            std::fs::read_dir(dir.path()).expect("read_dir").count(),
            1,
            "a refusal writes nothing"
        );
    }

    #[test]
    fn force_writes_into_a_non_empty_directory_and_keeps_other_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("keep.txt"), "mine").expect("write");

        scaffold(dir.path(), "agent", true, false).expect("forced scaffold");
        assert!(dir.path().join("main.forge").is_file());
        assert!(
            dir.path().join("keep.txt").is_file(),
            "force overwrites template files, it never cleans the directory"
        );
    }

    #[test]
    fn dry_run_lists_the_files_and_writes_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("demo");

        let listed = scaffold(&target, "webhook-bot", false, true).expect("dry run");
        assert_eq!(listed, expected_files());
        assert!(!target.exists(), "dry run must not create the directory");
    }

    #[test]
    fn dry_run_still_refuses_a_non_empty_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("keep.txt"), "mine").expect("write");

        let err = scaffold(dir.path(), "pipeline", false, true)
            .expect_err("audit runs without force: the refusal applies")
            .to_string();
        assert!(err.contains("is not empty"), "{err}");
    }

    #[test]
    fn unknown_template_is_an_error_and_writes_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("demo");

        let err = scaffold(&target, "nope", false, false)
            .expect_err("unknown template must fail")
            .to_string();
        assert!(err.contains("unknown template `nope`"), "{err}");
        assert!(err.contains("pipeline, agent, webhook-bot"), "{err}");
        assert!(!target.exists());
    }
}
