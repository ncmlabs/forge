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

    // The committed `expected.txt` is LF and the template records LF output;
    // Windows stdout is CRLF, so `--expect` must not care which side used which.
    let expected = std::fs::read_to_string(project.join("expected.txt")).expect("read expected");
    std::fs::write(project.join("expected.txt"), expected.replace('\n', "\r\n"))
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
