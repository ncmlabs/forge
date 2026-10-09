// FORGE authoring benchmark corpus — issue #483 part 1.
// Every `bench/specs/<NN>-<slug>/` ships five files, its reference must check
// with zero diagnostics (errors *and* warnings — `forge check` fails on both),
// its `meta.toml` must declare a known difficulty and non-empty features, and
// its recorded fixture must replay to exactly `expected.txt` through the real
// binary. The corpus is graded by `forge test`, so a spec that drifts from its
// fixture is a corpus bug, not a model failure.

use std::path::{Path, PathBuf};
use std::process::Command;

use forge::checker;
use forge::checker::boundary_checker;
use forge::diagnostic::Diagnostic;
use forge::resolver::{CapabilityRegistry, CheckContext};
use serde::Deserialize;

const SPECS_DIR: &str = "bench/specs";
const MOCK_CONFIG: &str = "bench/mock.config.toml";

const REQUIRED_FILES: [&str; 5] = [
    "spec.md",
    "reference.forge",
    "reference.forge.fixtures.json",
    "expected.txt",
    "meta.toml",
];

/// The feature vocabulary a spec's `meta.toml` may draw from (#483).
const FEATURES: [&str; 20] = [
    "task",
    "pure",
    "flow",
    "when",
    "match",
    "if",
    "for",
    "agent",
    "states",
    "requires",
    "event",
    "pool",
    "warden",
    "contract",
    "system",
    "command",
    "knowledge",
    "spawn",
    "file",
    "json",
];

const DIFFICULTIES: [&str; 3] = ["easy", "medium", "hard"];

#[derive(Debug, Deserialize)]
struct Meta {
    difficulty: String,
    features: Vec<String>,
    oracle_calls: u64,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Spec directories in listing order, so failures read in corpus order.
fn spec_dirs() -> Vec<PathBuf> {
    let root = repo_root().join(SPECS_DIR);
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", root.display()))
        .map(|entry| entry.expect("readable dir entry").path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    assert!(
        !dirs.is_empty(),
        "no spec directories under {}",
        root.display()
    );
    dirs
}

fn relative(path: &Path) -> String {
    path.strip_prefix(repo_root())
        .unwrap_or(path)
        .display()
        .to_string()
}

/// The parse + resolve + check path of `check_single` in
/// `tests/conformance_runner.rs` — what a single-file `forge check` runs.
/// Copied from `tests/forge_card_tests.rs` on purpose: test files do not
/// import each other, and this helper is the corpus's zero-diagnostic gate.
fn check_single(source: &str, filename: &str) -> Result<Vec<Diagnostic>, String> {
    let program = forge::parser::parse(source).map_err(|e| format!("{e:?}"))?;
    let mut diags = checker::check_all(&program, filename);
    diags.extend(boundary_checker::check(&[(&program, filename)]));
    if let Err(errors) = CheckContext::new(filename).check(&program) {
        let registry = CapabilityRegistry::builtin();
        diags.extend(errors.iter().map(|e| e.to_diagnostic(filename, &registry)));
    }
    Ok(diags)
}

fn load_meta(path: &Path) -> Meta {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    toml::from_str(&text).unwrap_or_else(|e| panic!("{} does not parse: {e}", path.display()))
}

/// Provider calls recorded for the reference — the fixture's `calls` array.
fn recorded_calls(path: &Path) -> usize {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let json: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("{} is not JSON: {e}", path.display()));
    json["calls"]
        .as_array()
        .unwrap_or_else(|| panic!("{} has no `calls` array", path.display()))
        .len()
}

#[test]
fn every_spec_ships_the_required_files() {
    let mut missing = Vec::new();
    for dir in spec_dirs() {
        for file in REQUIRED_FILES {
            let path = dir.join(file);
            if !path.is_file() {
                missing.push(relative(&path));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "every spec directory needs all five corpus files; missing:\n{}",
        missing.join("\n")
    );
}

/// A spec is what the model reads, so it stays short: 5–15 non-blank lines.
#[test]
fn every_spec_is_five_to_fifteen_lines() {
    let mut failures = Vec::new();
    for dir in spec_dirs() {
        let path = dir.join("spec.md");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let lines = text.lines().filter(|line| !line.trim().is_empty()).count();
        if !(5..=15).contains(&lines) {
            failures.push(format!("{}: {lines} non-blank lines", relative(&path)));
        }
    }
    assert!(
        failures.is_empty(),
        "every spec.md must hold 5–15 non-blank lines:\n{}",
        failures.join("\n")
    );
}

#[test]
fn every_reference_checks_with_zero_diagnostics() {
    let mut failures = Vec::new();
    for dir in spec_dirs() {
        let path = dir.join("reference.forge");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let name = relative(&path);
        match check_single(&source, &name) {
            Err(parse_error) => failures.push(format!("{name}: parse failed: {parse_error}")),
            Ok(diags) if !diags.is_empty() => failures.push(format!(
                "{name}: {} diagnostic(s): {}",
                diags.len(),
                diags
                    .iter()
                    .map(|d| format!("[{}] {}", d.code, d.message))
                    .collect::<Vec<_>>()
                    .join("; ")
            )),
            Ok(_) => {}
        }
    }
    assert!(
        failures.is_empty(),
        "every reference solution must check clean:\n{}",
        failures.join("\n")
    );
}

#[test]
fn every_meta_declares_difficulty_features_and_oracle_calls() {
    let mut failures = Vec::new();
    for dir in spec_dirs() {
        let path = dir.join("meta.toml");
        let meta = load_meta(&path);
        let name = relative(&path);

        if !DIFFICULTIES.contains(&meta.difficulty.as_str()) {
            failures.push(format!(
                "{name}: difficulty `{}` is not one of {DIFFICULTIES:?}",
                meta.difficulty
            ));
        }
        if meta.features.is_empty() {
            failures.push(format!("{name}: features must not be empty"));
        }
        for feature in &meta.features {
            if !FEATURES.contains(&feature.as_str()) {
                failures.push(format!("{name}: unknown feature `{feature}`"));
            }
        }
        let recorded = recorded_calls(&dir.join("reference.forge.fixtures.json"));
        if meta.oracle_calls as usize != recorded {
            failures.push(format!(
                "{name}: oracle_calls = {} but the fixture holds {recorded} call(s)",
                meta.oracle_calls
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "meta.toml problems:\n{}",
        failures.join("\n")
    );
}

#[test]
fn spec_count_and_difficulty_spread() {
    let dirs = spec_dirs();
    let mut by_difficulty = std::collections::BTreeMap::new();
    for dir in &dirs {
        let meta = load_meta(&dir.join("meta.toml"));
        *by_difficulty.entry(meta.difficulty).or_insert(0usize) += 1;
    }
    assert_eq!(
        dirs.len(),
        30,
        "the corpus is exactly 30 specs; found {by_difficulty:?}"
    );
    for (difficulty, want) in [("easy", 10usize), ("medium", 12), ("hard", 8)] {
        let got = by_difficulty.get(difficulty).copied().unwrap_or(0);
        assert_eq!(got, want, "{difficulty} specs: want {want}, got {got}");
    }
}

#[test]
fn every_reference_replays_to_its_expected_output() {
    let mut failures = Vec::new();
    for dir in spec_dirs() {
        let reference = dir.join("reference.forge");
        let expected = dir.join("expected.txt");
        let output = Command::new(env!("CARGO_BIN_EXE_forge"))
            .arg("test")
            .arg(&reference)
            .arg("--expect")
            .arg(&expected)
            .current_dir(repo_root())
            .env("FORGE_CONFIG", MOCK_CONFIG)
            .output()
            .unwrap_or_else(|e| panic!("cannot run forge test for {}: {e}", dir.display()));
        if !output.status.success() {
            failures.push(format!(
                "{}: forge test exited {}\n--- stdout\n{}\n--- stderr\n{}",
                relative(&dir),
                output.status,
                String::from_utf8_lossy(&output.stdout).trim_end(),
                String::from_utf8_lossy(&output.stderr).trim_end()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "every reference must replay to its expected output:\n{}",
        failures.join("\n")
    );
}

/// The specs that gate on a `command` result. Each one records the *failure*
/// path, so a solution that skips the success check cannot reproduce its
/// expected output — that is what makes the deterministic gate (#484)
/// measurable.
const GATE_SPECS: [&str; 4] = [
    "23-command-gate",
    "24-command-pipeline",
    "25-command-verify",
    "26-command-argv",
];

/// Replace every `if <x>.success` condition with `if true`, keeping the
/// indentation. Returns the rewritten source and how many gates were forced.
fn force_success(source: &str) -> (String, usize) {
    let mut forced = 0;
    let mut lines = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("if ") && trimmed.contains(".success") {
            forced += 1;
            let indent = &line[..line.len() - trimmed.len()];
            lines.push(format!("{indent}if true"));
        } else {
            lines.push(line.to_string());
        }
    }
    (lines.join("\n") + "\n", forced)
}

#[test]
fn gate_specs_reject_a_solution_that_ignores_success() {
    for spec in GATE_SPECS {
        let dir = repo_root().join(SPECS_DIR).join(spec);
        let source = std::fs::read_to_string(dir.join("reference.forge")).unwrap_or_else(|e| {
            panic!("cannot read {}: {e}", dir.join("reference.forge").display())
        });
        let (ungated, forced) = force_success(&source);
        assert!(
            forced > 0,
            "{spec}: no `if <x>.success` gate to force — does the reference still gate?"
        );

        // The variant needs its own directory: `forge test` looks for the
        // fixture beside the program it is given.
        let temp = tempfile::tempdir().expect("tempdir");
        let program = temp.path().join("reference.forge");
        std::fs::write(&program, &ungated).expect("write ungated variant");
        for file in ["reference.forge.fixtures.json", "expected.txt"] {
            std::fs::copy(dir.join(file), temp.path().join(file))
                .unwrap_or_else(|e| panic!("cannot stage {file} for {spec}: {e}"));
        }

        let output = Command::new(env!("CARGO_BIN_EXE_forge"))
            .arg("test")
            .arg(&program)
            .arg("--expect")
            .arg(temp.path().join("expected.txt"))
            .current_dir(repo_root())
            .env("FORGE_CONFIG", MOCK_CONFIG)
            .output()
            .unwrap_or_else(|e| panic!("cannot run the ungated variant of {spec}: {e}"));
        assert!(
            !output.status.success(),
            "{spec}: an ungated variant (`if true`) reproduced expected.txt, so the spec does \
             not discriminate.\n--- stdout\n{}\n--- stderr\n{}",
            String::from_utf8_lossy(&output.stdout).trim_end(),
            String::from_utf8_lossy(&output.stderr).trim_end()
        );
    }
}
