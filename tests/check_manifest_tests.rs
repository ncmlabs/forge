//! `forge check --manifest` (#495): a project's declared skills must resolve
//! during static checking, and the manifest's sources must be checked exactly
//! as `forge run --manifest` blocks on them — per source file, plus the
//! cross-file boundary check.
//!
//! Every test runs the real binary inside a fixture under `tests/fixtures/`,
//! with no ambient provider or skill configuration: only what a test passes can
//! register a skill.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A self-contained skill project: every source checks clean on its own.
fn project() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skill-project")
}

/// A project whose `main.forge` uses a lifecycle declared in `states.forge`.
fn cross_project() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/manifest-cross-state")
}

fn forge(args: &[&str]) -> Output {
    forge_in(&project(), args, &[])
}

fn forge_in(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_forge"));
    cmd.current_dir(dir)
        .args(args)
        // Own HOME so a developer's ~/.forge/config.toml cannot register skills.
        .env("HOME", dir)
        .env_remove("FORGE_CONFIG")
        .env_remove("FORGE_APP_CONFIG")
        .env_remove("FORGE_MOCK")
        .env_remove("FORGE_PROVIDER")
        .env_remove("FORGE_OUTPUT");
    for (key, value) in envs {
        cmd.env(key, value);
    }
    cmd.output().expect("failed to execute forge binary")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn assert_exit(output: &Output, expected: i32) {
    assert_eq!(
        output.status.code(),
        Some(expected),
        "exit code mismatch; stderr: {}",
        stderr_of(output)
    );
}

#[test]
fn without_manifest_a_declared_skill_is_unknown() {
    let output = forge(&["check", "skill_only.forge"]);
    assert_exit(&output, 1);
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("E010") && stderr.contains("unknown capability 'skill.greeter.hello'"),
        "expected the E010 unknown-capability error: {stderr}"
    );
}

#[test]
fn manifest_resolves_declared_skills_and_checks_its_sources() {
    // No files: every source the manifest declares is checked (per file).
    let output = forge(&["check", "--manifest", "forge.project.toml"]);
    assert_exit(&output, 0);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "OK\n");
}

#[test]
fn manifest_check_passes_what_manifest_run_executes() {
    // The fixture is self-contained per source, so `check --manifest` is OK and
    // `run --manifest` is not blocked by a check error (#495).
    let output = forge(&["check", "--manifest", "forge.project.toml"]);
    assert_exit(&output, 0);

    let run = forge_in(
        &project(),
        &["run", "--manifest", "forge.project.toml"],
        &[("FORGE_MOCK", "1")],
    );
    assert_exit(&run, 0);
    assert!(
        String::from_utf8_lossy(&run.stdout).contains("greeter ready"),
        "run should execute the project: {}",
        String::from_utf8_lossy(&run.stdout)
    );
    let stderr = stderr_of(&run);
    assert!(
        !stderr.contains("[E"),
        "run must not be blocked by a check diagnostic: {stderr}"
    );
}

#[test]
fn manifest_sources_are_checked_per_file_exactly_as_run_blocks() {
    // `main.forge` uses a lifecycle declared in `states.forge`, which no single
    // file can resolve: `run --manifest` blocks on it, so `check --manifest`
    // must report the same E040 instead of a false OK (#495).
    let dir = cross_project();
    let check = forge_in(&dir, &["check", "--manifest", "forge.project.toml"], &[]);
    assert_exit(&check, 1);
    let stderr = stderr_of(&check);
    assert!(
        stderr.contains("E040") && stderr.contains("unknown lifecycle `GreetingPhase`"),
        "expected the E040 unknown-lifecycle error: {stderr}"
    );

    let run = forge_in(
        &dir,
        &["run", "--manifest", "forge.project.toml"],
        &[("FORGE_MOCK", "1")],
    );
    assert_exit(&run, 1);
    let stderr = stderr_of(&run);
    assert!(
        stderr.contains("E040") && stderr.contains("unknown lifecycle `GreetingPhase`"),
        "run must block on the same diagnostic: {stderr}"
    );
}

#[test]
fn explicit_files_replace_the_manifest_sources() {
    // Skills come from the manifest, but an explicit list is what gets checked.
    let output = forge(&[
        "check",
        "--manifest",
        "forge.project.toml",
        "--json",
        "skill_only.forge",
    ]);
    assert_exit(&output, 0);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let doc: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not one JSON document ({e}): {stdout}"));
    assert_eq!(
        doc["data"]["files"],
        serde_json::json!(["skill_only.forge"])
    );
    assert!(
        !stderr_of(&output).contains("E010"),
        "the declared skill must resolve: {}",
        stderr_of(&output)
    );
}

#[test]
fn manifest_does_not_resolve_an_undeclared_skill() {
    let output = forge(&[
        "check",
        "--manifest",
        "forge.project.toml",
        "undeclared.forge",
    ]);
    assert_exit(&output, 1);
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("E010") && stderr.contains("unknown capability 'skill.ghost.boo'"),
        "expected the E010 unknown-capability error: {stderr}"
    );
    // The help lists what the manifest did register.
    assert!(
        stderr.contains("available skill capabilities: skill.greeter.hello"),
        "expected the registered capabilities in the help: {stderr}"
    );
}

#[test]
fn manifest_check_keeps_the_json_envelope() {
    let output = forge(&["check", "--manifest", "forge.project.toml", "--json"]);
    assert_exit(&output, 0);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let doc: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not one JSON document ({e}): {stdout}"));
    assert_eq!(doc["status"], "success");
    assert_eq!(doc["command"], "check");
    assert_eq!(doc["data"]["files"][0], "main.forge");
    assert_eq!(doc["data"]["files"][1], "states.forge");
}

#[test]
fn forge_config_skill_dirs_are_honoured_without_a_manifest() {
    // The run path's fallback: no manifest, but FORGE_CONFIG declares skill_dirs.
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        tmp.path().join("forge.config.toml"),
        "[llm]\ndefault = \"mock\"\n\n[providers.mock]\ntype = \"mock\"\n\n[skills]\nskill_dirs = [\"skills\"]\n",
    )
    .expect("write config");
    let config = tmp.path().join("forge.config.toml");
    let output = forge_in(
        &project(),
        &["check", "skill_only.forge"],
        &[("FORGE_CONFIG", config.to_str().expect("utf-8 path"))],
    );
    assert_exit(&output, 0);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "OK\n");
}
