//! `forge check --manifest` (#495): a project's declared skills must resolve
//! during static checking, on the same code path `forge run --manifest` uses.
//!
//! Every test runs the real binary inside `tests/fixtures/skill-project`, a
//! self-contained skill project (`forge.project.toml`, `skills/<name>/SKILL.md`,
//! sources), with no ambient provider or skill configuration: only what a test
//! passes can register a skill.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn project() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skill-project")
}

fn forge(args: &[&str]) -> Output {
    forge_with_env(args, &[])
}

fn forge_with_env(args: &[&str], envs: &[(&str, &str)]) -> Output {
    let dir = project();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_forge"));
    cmd.current_dir(&dir)
        .args(args)
        // Own HOME so a developer's ~/.forge/config.toml cannot register skills.
        .env("HOME", &dir)
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
    // No files: the manifest's sources are checked as one composition, so the
    // `GreetingPhase` lifecycle declared in `states.forge` resolves too.
    let output = forge(&["check", "--manifest", "forge.project.toml"]);
    assert_exit(&output, 0);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "OK\n");
}

#[test]
fn explicit_files_are_checked_per_file_with_manifest_skills_registered() {
    // Skills come from the manifest, but explicit files replace the manifest's
    // sources and are not merged without --merge: `main.forge` alone cannot see
    // `GreetingPhase`.
    let output = forge(&["check", "--manifest", "forge.project.toml", "main.forge"]);
    assert_exit(&output, 1);
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("E040") && stderr.contains("unknown lifecycle `GreetingPhase`"),
        "expected the E040 unknown-lifecycle error: {stderr}"
    );
    assert!(
        !stderr.contains("E010"),
        "the declared skill must resolve: {stderr}"
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
    let output = forge_with_env(
        &["check", "skill_only.forge"],
        &[("FORGE_CONFIG", config.to_str().expect("utf-8 path"))],
    );
    assert_exit(&output, 0);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "OK\n");
}
