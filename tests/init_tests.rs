//! `forge init` end-to-end (#481): every template scaffolds a project that
//! checks and replays clean, straight out of the built binary.
//!
//! These run the real binary and the project's own `forge.config.toml` — no
//! `FORGE_CONFIG` override, no `FORGE_MOCK`, no API key — so a template that
//! only works with the repo's config would fail here.

use std::path::Path;
use std::process::Command;

const TEMPLATES: &[&str] = &["pipeline", "agent", "webhook-bot"];

const SCAFFOLDED_FILES: &[&str] = &[
    "forge.project.toml",
    "forge.config.toml",
    "main.forge",
    "main.forge.fixtures.json",
    "expected.txt",
    ".gitignore",
    "AGENTS.md",
];

/// Run the forge binary in `cwd` with no ambient provider configuration.
fn forge(cwd: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_forge"));
    cmd.current_dir(cwd)
        .env_remove("FORGE_CONFIG")
        .env_remove("FORGE_MOCK")
        .env_remove("FORGE_PROVIDER")
        .env_remove("FORGE_TRACE")
        .env_remove("FORGE_BUDGET")
        .env_remove("ANTHROPIC_API_KEY");
    cmd
}

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

#[test]
fn each_template_scaffolds_a_project_that_checks_and_replays_clean() {
    for template in TEMPLATES {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = dir.path().join("demo");

        let init = forge(dir.path())
            .args(["init", "demo", "--template", template])
            .output()
            .expect("init run");
        assert!(init.status.success(), "{template}: {init:?}");
        for file in SCAFFOLDED_FILES {
            assert!(
                project.join(file).is_file(),
                "{template}: {file} was not scaffolded"
            );
        }

        // `forge check` exits non-zero on warnings too, so an empty stderr is
        // the "zero diagnostics" contract.
        let main = project.join("main.forge");
        let expected = project.join("expected.txt");
        let check = forge(&project)
            .args(["check".as_ref(), main.as_os_str()])
            .output()
            .expect("check run");
        assert!(check.status.success(), "{template}: {}", stderr_of(&check));
        assert!(
            stderr_of(&check).is_empty(),
            "{template}: check is not diagnostic-free: {}",
            stderr_of(&check)
        );

        let replay = forge(&project)
            .args([
                "test".as_ref(),
                main.as_os_str(),
                "--expect".as_ref(),
                expected.as_os_str(),
            ])
            .output()
            .expect("test run");
        assert!(
            replay.status.success(),
            "{template}: {}",
            stderr_of(&replay)
        );
    }
}

#[test]
fn init_defaults_to_the_pipeline_template() {
    let dir = tempfile::tempdir().expect("tempdir");

    let init = forge(dir.path())
        .args(["init", "demo"])
        .output()
        .expect("init run");
    assert!(init.status.success(), "{}", stderr_of(&init));

    let main = std::fs::read_to_string(dir.path().join("demo/main.forge")).expect("read main");
    assert!(main.contains("flow brief"), "{main}");
}

#[test]
fn expected_txt_matches_across_line_endings() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = dir.path().join("demo");
    let init = forge(dir.path())
        .args(["init", "demo", "--template", "agent"])
        .output()
        .expect("init run");
    assert!(init.status.success(), "{}", stderr_of(&init));

    // The committed `expected.txt` is LF and the template records LF output; a
    // Windows checkout (`core.autocrlf`) and Windows stdout are CRLF, so
    // `--expect` must not care which side used which. Normalise to LF first so
    // this writes the same bytes on either host.
    let expected = std::fs::read_to_string(project.join("expected.txt")).expect("read expected");
    std::fs::write(
        project.join("expected.txt"),
        expected.replace("\r\n", "\n").replace('\n', "\r\n"),
    )
    .expect("rewrite expected with CRLF");

    let replay = forge(&project)
        .args([
            "test".as_ref(),
            project.join("main.forge").as_os_str(),
            "--expect".as_ref(),
            project.join("expected.txt").as_os_str(),
        ])
        .output()
        .expect("test run");
    assert!(replay.status.success(), "{}", stderr_of(&replay));
}

#[test]
fn init_refuses_an_invalid_project_name() {
    let dir = tempfile::tempdir().expect("tempdir");

    for name in ["bad\"name", "bad\\name", "bad name", "bäd", ".hidden"] {
        let output = forge(dir.path())
            .args(["init", name])
            .output()
            .expect("init run");
        assert_eq!(output.status.code(), Some(1), "{name} must exit 1");
        let stderr = stderr_of(&output);
        assert!(
            stderr.contains(&format!("project name '{name}' is not valid")),
            "{name}: {stderr}"
        );
        assert!(
            stderr.contains("use letters, digits, '-', '_' (rename the directory)"),
            "{name}: {stderr}"
        );
        assert!(
            !dir.path().join(name).exists(),
            "{name}: a refusal writes nothing"
        );
    }
}

#[test]
fn init_accepts_dashes_underscores_and_dots_in_the_name() {
    let dir = tempfile::tempdir().expect("tempdir");

    for name in ["my-app_2", "my.app"] {
        let output = forge(dir.path())
            .args(["init", name])
            .output()
            .expect("init run");
        assert!(output.status.success(), "{name}: {}", stderr_of(&output));
        let project = std::fs::read_to_string(dir.path().join(name).join("forge.project.toml"))
            .expect("read project");
        assert!(project.contains(&format!("name = \"{name}\"")), "{project}");
    }
}

/// Unix symlinks: `--force` must not follow a link out of the project.
#[cfg(unix)]
#[test]
fn init_force_refuses_to_write_through_a_symlink() {
    let outside = tempfile::tempdir().expect("tempdir");
    let outside_file = outside.path().join("outside.txt");
    std::fs::write(&outside_file, "keep me").expect("write outside");

    let dir = tempfile::tempdir().expect("tempdir");
    let project = dir.path().join("demo");
    std::fs::create_dir(&project).expect("create dir");
    std::os::unix::fs::symlink(&outside_file, project.join("main.forge")).expect("symlink");

    let output = forge(dir.path())
        .args(["init", "demo", "--force"])
        .output()
        .expect("init run");
    assert_eq!(output.status.code(), Some(1), "a symlinked target exits 1");
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("main.forge is a symlink; refusing to overwrite through it"),
        "{stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(&outside_file).expect("read outside"),
        "keep me",
        "the file behind the symlink is untouched"
    );
    assert!(
        !project.join("forge.project.toml").exists(),
        "a refusal writes nothing"
    );
    assert!(
        project.join("main.forge").is_symlink(),
        "the symlink is left alone"
    );
}

#[test]
fn init_refuses_a_non_empty_directory_without_force() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = dir.path().join("demo");
    std::fs::create_dir(&project).expect("create dir");
    std::fs::write(project.join("keep.txt"), "mine").expect("write keep");

    let output = forge(dir.path())
        .args(["init", "demo"])
        .output()
        .expect("init run");
    assert_eq!(output.status.code(), Some(1), "refusal exits 1");
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("demo is not empty; use --force to write into it"),
        "{stderr}"
    );
    assert!(
        !project.join("main.forge").exists(),
        "a refusal writes nothing"
    );
    assert!(
        project.join("keep.txt").exists(),
        "a refusal removes nothing"
    );
}

#[test]
fn init_force_writes_into_a_non_empty_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = dir.path().join("demo");
    std::fs::create_dir(&project).expect("create dir");
    std::fs::write(project.join("keep.txt"), "mine").expect("write keep");

    let output = forge(dir.path())
        .args(["init", "demo", "--force"])
        .output()
        .expect("init run");
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert!(project.join("main.forge").is_file());
    assert!(
        project.join("keep.txt").is_file(),
        "--force overwrites the template files, it does not clean the directory"
    );
}

#[test]
fn init_dry_run_lists_the_files_and_writes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");

    let output = forge(dir.path())
        .args(["init", "demo", "--dry-run"])
        .output()
        .expect("init run");
    assert!(output.status.success(), "{}", stderr_of(&output));

    let stdout = String::from_utf8_lossy(&output.stdout);
    for file in SCAFFOLDED_FILES {
        assert!(
            stdout.contains(file),
            "dry run does not list {file}: {stdout}"
        );
    }
    assert!(
        !dir.path().join("demo").exists(),
        "a dry run creates nothing"
    );
}

#[test]
fn init_json_emits_one_envelope_listing_the_files() {
    let dir = tempfile::tempdir().expect("tempdir");

    let output = forge(dir.path())
        .args(["init", "demo", "--json"])
        .output()
        .expect("init run");
    assert!(output.status.success(), "{}", stderr_of(&output));

    let env: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout is one JSON envelope");
    assert_eq!(env["command"], "init");
    assert_eq!(env["status"], "success");
    assert_eq!(env["data"]["template"], "pipeline");
    let files = env["data"]["files"].as_array().expect("files array");
    assert_eq!(files.len(), SCAFFOLDED_FILES.len(), "{env}");
    assert!(dir.path().join("demo/main.forge").is_file());
}

#[test]
fn init_rejects_an_unknown_template() {
    let dir = tempfile::tempdir().expect("tempdir");

    let output = forge(dir.path())
        .args(["init", "demo", "--template", "nope"])
        .output()
        .expect("init run");
    assert_eq!(output.status.code(), Some(1), "unknown template exits 1");
    let stderr = stderr_of(&output);
    assert!(stderr.contains("unknown template `nope`"), "{stderr}");
    assert!(stderr.contains("pipeline, agent, webhook-bot"), "{stderr}");
    assert!(!dir.path().join("demo").exists());
}
