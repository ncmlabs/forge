//! `forge run --record` / `forge test` end-to-end (#478).
//!
//! These run the real binary, so any test that needs a fixture mode set lives
//! here: the mode is a process-global, and an integration test binary owns its
//! process.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// Run the forge binary in `cwd` with a fixed config and no ambient env.
fn forge(config: &Path, cwd: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_forge"));
    cmd.env("FORGE_CONFIG", config)
        .current_dir(cwd)
        .env_remove("FORGE_MOCK")
        .env_remove("FORGE_PROVIDER")
        .env_remove("FORGE_TRACE")
        .env_remove("FORGE_BUDGET")
        .env_remove("ANTHROPIC_API_KEY");
    cmd
}

fn mock_config() -> PathBuf {
    repo_path("config/mock.config.toml")
}

/// Copy `examples/llm/classify.forge` into `dir` so the run leaves its
/// `.forge-data` sidecar and fixtures there instead of in the repo.
fn stage_program(dir: &Path) {
    std::fs::copy(
        repo_path("examples/llm/classify.forge"),
        dir.join("classify.forge"),
    )
    .expect("stage classify.forge");
}

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

#[test]
fn record_then_replay_round_trip_via_cli() {
    let dir = tempfile::tempdir().expect("tempdir");
    stage_program(dir.path());

    let recorded = forge(&mock_config(), dir.path())
        .args(["run", "classify.forge", "--record"])
        .output()
        .expect("record run");
    assert!(
        recorded.status.success(),
        "record run failed: {}",
        stderr_of(&recorded)
    );
    assert!(
        !recorded.stdout.is_empty(),
        "recording still runs the program and prints its output"
    );
    assert!(
        dir.path().join("classify.forge.fixtures.json").exists(),
        "default fixture path is <program>.fixtures.json beside the program"
    );

    let replayed = forge(&mock_config(), dir.path())
        .args(["test", "classify.forge"])
        .output()
        .expect("replay run");
    assert!(
        replayed.status.success(),
        "replay run failed: {}",
        stderr_of(&replayed)
    );
    assert_eq!(
        String::from_utf8_lossy(&replayed.stdout),
        String::from_utf8_lossy(&recorded.stdout),
        "replay reproduces the recorded stdout"
    );
}

#[test]
fn replay_makes_no_network_call() {
    let dir = tempfile::tempdir().expect("tempdir");
    stage_program(dir.path());

    let recorded = forge(&mock_config(), dir.path())
        .args(["run", "classify.forge", "--record"])
        .output()
        .expect("record run");
    assert!(recorded.status.success(), "{}", stderr_of(&recorded));

    // Same provider name, but a real HTTP provider pointed at a dead port:
    // a replay hit must return the fixture without ever dialling out.
    let dead = dir.path().join("dead.config.toml");
    std::fs::write(
        &dead,
        "[llm]\ndefault = \"mock\"\n\n[providers.mock]\ntype = \"openai-compat\"\n\
         base_url = \"http://127.0.0.1:1/v1\"\nmodel = \"dead\"\n",
    )
    .expect("write dead config");

    let replayed = forge(&dead, dir.path())
        .args(["test", "classify.forge"])
        .output()
        .expect("replay run");
    assert!(
        replayed.status.success(),
        "replay must not reach the network: {}",
        stderr_of(&replayed)
    );
    assert_eq!(
        String::from_utf8_lossy(&replayed.stdout),
        String::from_utf8_lossy(&recorded.stdout)
    );
}

#[test]
fn test_without_fixtures_exits_nonzero_with_instructions() {
    let dir = tempfile::tempdir().expect("tempdir");
    stage_program(dir.path());

    let output = forge(&mock_config(), dir.path())
        .args(["test", "classify.forge"])
        .output()
        .expect("replay run");
    assert!(
        !output.status.success(),
        "a missing fixture file is a hard error"
    );
    let stderr = stderr_of(&output);
    assert!(stderr.contains("no fixtures at"), "{stderr}");
    assert!(
        stderr.contains("classify.forge.fixtures.json"),
        "the error names the missing file: {stderr}"
    );
    assert!(
        stderr.contains("record them with: forge run classify.forge --record"),
        "{stderr}"
    );
}

#[test]
fn record_and_test_accept_an_explicit_fixture_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    stage_program(dir.path());

    let recorded = forge(&mock_config(), dir.path())
        .args([
            "run",
            "classify.forge",
            "--record",
            "out/custom.fixtures.json",
        ])
        .output()
        .expect("record run");
    assert!(recorded.status.success(), "{}", stderr_of(&recorded));
    assert!(
        dir.path().join("out/custom.fixtures.json").exists(),
        "--record <path> writes where asked, creating parent directories"
    );
    assert!(
        !dir.path().join("classify.forge.fixtures.json").exists(),
        "an explicit path replaces the default"
    );

    let replayed = forge(&mock_config(), dir.path())
        .args([
            "test",
            "classify.forge",
            "--fixtures",
            "out/custom.fixtures.json",
        ])
        .output()
        .expect("replay run");
    assert!(replayed.status.success(), "{}", stderr_of(&replayed));
    assert_eq!(
        String::from_utf8_lossy(&replayed.stdout),
        String::from_utf8_lossy(&recorded.stdout)
    );
}

#[test]
fn record_rejects_manifest_runs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let output = forge(&mock_config(), dir.path())
        .args(["run", "--record", "--manifest", "forge.project.toml"])
        .output()
        .expect("record run");
    assert!(
        !output.status.success(),
        "--record cannot record a manifest"
    );
    assert!(
        stderr_of(&output).contains("--manifest recording is not supported yet"),
        "{}",
        stderr_of(&output)
    );
}
