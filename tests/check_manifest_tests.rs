//! `forge check --manifest` (#495): a project's declared skills must resolve
//! during static checking, and the manifest's sources must be checked exactly
//! as `forge run --manifest` blocks on them — as one composed program, with the
//! resolver and the cross-file boundary check per file (#516), and every
//! diagnostic naming the source file its span belongs to (#519).
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

/// A project whose second source (`b.forge`) carries the only error.
fn second_source_project() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/manifest-diag-second")
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
fn manifest_sources_are_checked_as_one_composed_program() {
    // `main.forge` uses a lifecycle declared in `states.forge`: no single file
    // can resolve it, but the composed project can (#516). `check --manifest`
    // must not report the false E040 that per-file checking produced, and
    // `run --manifest` must not be blocked by it either.
    let dir = cross_project();
    let check = forge_in(&dir, &["check", "--manifest", "forge.project.toml"], &[]);
    assert_exit(&check, 0);
    let stderr = stderr_of(&check);
    assert!(
        !stderr.contains("E040"),
        "a lifecycle declared in another source must resolve: {stderr}"
    );
    assert_eq!(String::from_utf8_lossy(&check.stdout), "OK\n");

    let run = forge_in(
        &dir,
        &["run", "--manifest", "forge.project.toml"],
        &[("FORGE_MOCK", "1")],
    );
    assert_exit(&run, 0);
    assert!(
        String::from_utf8_lossy(&run.stdout).contains("cross-state project ready"),
        "run should execute the project: {}",
        String::from_utf8_lossy(&run.stdout)
    );
    assert!(
        !stderr_of(&run).contains("[E"),
        "run must not be blocked by a check diagnostic: {}",
        stderr_of(&run)
    );
}

#[test]
fn manifest_diagnostic_names_the_second_source_file() {
    // The only error is in `b.forge` (line 7: `lifecycle: Missing`). The merged
    // program's spans are cumulative, so a wrong mapping would name `a.forge`
    // and print a line from the wrong place (#519).
    let dir = second_source_project();
    let output = forge_in(&dir, &["check", "--manifest", "forge.project.toml"], &[]);
    assert_exit(&output, 1);
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("E040") && stderr.contains("unknown lifecycle `Missing`"),
        "expected the E040 unknown-lifecycle error: {stderr}"
    );
    assert!(
        stderr.contains("b.forge:7:14"),
        "the diagnostic must name b.forge at the offending lifecycle: {stderr}"
    );
    assert!(
        !stderr.contains("a.forge"),
        "the clean first source must not be named: {stderr}"
    );
    assert!(
        stderr.contains("  lifecycle: Missing"),
        "the printed line must be the one in b.forge: {stderr}"
    );
}

#[test]
fn manifest_diagnostic_json_names_the_second_source_file() {
    let dir = second_source_project();
    let output = forge_in(
        &dir,
        &["check", "--manifest", "forge.project.toml", "--json"],
        &[],
    );
    assert_exit(&output, 1);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let doc: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not one JSON document ({e}): {stdout}"));
    let diagnostics = doc["data"]["diagnostics"]
        .as_array()
        .unwrap_or_else(|| panic!("no diagnostics array: {doc}"));
    assert_eq!(diagnostics.len(), 1, "{doc}");
    assert_eq!(diagnostics[0]["code"], "E040");
    assert_eq!(diagnostics[0]["file"], "b.forge");
    assert_eq!(diagnostics[0]["line"], 7);
    assert_eq!(diagnostics[0]["col"], 14);
    assert_eq!(diagnostics[0]["message"], "unknown lifecycle `Missing`");
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
