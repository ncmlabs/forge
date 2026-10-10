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
/// A non-empty `dir` is refused unless `force`, the directory's base name must
/// be a valid project name, and no template path may be a symlink; every
/// refusal applies to `dry_run` too, because a preview must predict the run it
/// previews. With `force`, only the template files are overwritten, never
/// anything else.
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

    let name = project_name(dir)?;
    validate_name(&name)?;

    // Check every target before writing any of them: `--force` must not follow
    // a symlink out of the project, and a refusal must leave `dir` untouched.
    for (path, _) in files {
        let target = dir.join(path);
        if is_symlink(&target)? {
            bail!(
                "{} is a symlink; refusing to overwrite through it",
                target.display()
            );
        }
    }

    let paths: Vec<&'static str> = files.iter().map(|(path, _)| *path).collect();
    if dry_run {
        return Ok(paths);
    }

    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    for (path, contents) in files {
        let target = dir.join(path);
        std::fs::write(&target, render(contents, &name))
            .with_context(|| format!("cannot write {}", target.display()))?;
    }
    Ok(paths)
}

/// `symlink_metadata` does not follow the link, so this is true only for the
/// path itself; a missing path is simply not a symlink.
fn is_symlink(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => Ok(meta.file_type().is_symlink()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("cannot inspect {}", path.display())),
    }
}

/// The name lands in `forge.project.toml`, `AGENTS.md` and `main.forge` bodies,
/// so the accepted set is deliberately narrow: `[A-Za-z0-9_-]`, plus `.` that
/// is not leading. Anything else is refused before a single file is written —
/// the directory is renamed, not the generated file patched.
fn validate_name(name: &str) -> Result<()> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
    if name.is_empty() || name.starts_with('.') || !name.chars().all(allowed) {
        bail!(
            "project name '{name}' is not valid; use letters, digits, '-', '_' (rename the directory)"
        );
    }
    Ok(())
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
        let project = dir.path().join("demo");
        std::fs::create_dir(&project).expect("create dir");
        std::fs::write(project.join("keep.txt"), "mine").expect("write");

        scaffold(&project, "agent", true, false).expect("forced scaffold");
        assert!(project.join("main.forge").is_file());
        assert!(
            project.join("keep.txt").is_file(),
            "force overwrites template files, it never cleans the directory"
        );
    }

    #[test]
    fn invalid_project_names_are_refused_with_nothing_written() {
        let dir = tempfile::tempdir().expect("tempdir");
        // `bad\name` is a path on Windows (`name` inside `bad`), not a name, so
        // the rule sees the valid base name `name` there; it is unix-only.
        let names = ["bad\"name", "bad\\name", "bad name", "bäd", ".hidden"]
            .into_iter()
            .filter(|name| !name.contains(std::path::MAIN_SEPARATOR));
        for name in names {
            let target = dir.path().join(name);
            let err = scaffold(&target, "pipeline", false, false)
                .expect_err("invalid name must be refused")
                .to_string();
            assert!(
                err.contains(&format!("project name '{name}' is not valid")),
                "{name}: {err}"
            );
            assert!(
                err.contains("use letters, digits, '-', '_' (rename the directory)"),
                "{name}: {err}"
            );
            assert!(!target.exists(), "{name}: a refusal writes nothing");
        }
    }

    #[test]
    fn names_with_dashes_underscores_and_dots_are_accepted() {
        let dir = tempfile::tempdir().expect("tempdir");
        for name in ["my-app_2", "my.app", "App2"] {
            let target = dir.path().join(name);
            scaffold(&target, "pipeline", false, false).expect("scaffold");
            let project =
                std::fs::read_to_string(target.join("forge.project.toml")).expect("read project");
            assert!(project.contains(&format!("name = \"{name}\"")), "{project}");
        }
    }

    /// Unix symlinks: `--force` must not write through a link that points out
    /// of the project, and the refusal must happen before the first write.
    #[cfg(unix)]
    #[test]
    fn force_refuses_to_write_through_a_symlink() {
        let outside = tempfile::tempdir().expect("tempdir");
        let outside_file = outside.path().join("outside.txt");
        std::fs::write(&outside_file, "keep me").expect("write outside");

        let dir = tempfile::tempdir().expect("tempdir");
        let project = dir.path().join("demo");
        std::fs::create_dir(&project).expect("create dir");
        std::os::unix::fs::symlink(&outside_file, project.join("main.forge")).expect("symlink");

        let err = scaffold(&project, "pipeline", true, false)
            .expect_err("a symlinked target must be refused")
            .to_string();
        assert!(
            err.contains("main.forge is a symlink; refusing to overwrite through it"),
            "{err}"
        );
        assert_eq!(
            std::fs::read_to_string(&outside_file).expect("read outside"),
            "keep me",
            "the file behind the symlink is untouched"
        );
        assert!(
            !project.join("forge.project.toml").exists(),
            "all targets are checked before the first write"
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
