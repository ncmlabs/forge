// Binary tests for `forge schema` (#476), the hand-written JSON Schemas of the
// `--json` contract. Step one: the schema command itself, and its usage error.
// The drift guard — every command's real output validated against
// `forge schema <command>` — lives at the bottom of this file.

use std::process::{Command, Output};

fn forge(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_forge"))
        .args(args)
        .output()
        .expect("failed to execute forge binary")
}

fn parse_stdout(output: &Output) -> serde_json::Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not one JSON document ({e}): {stdout}"))
}

fn assert_exit(output: &Output, expected: i32) {
    assert_eq!(
        output.status.code(),
        Some(expected),
        "exit code mismatch; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// ── forge schema ────────────────────────────────────────────────

#[test]
fn schema_without_argument_prints_the_shared_envelope() {
    let output = forge(&["schema"]);
    assert_exit(&output, 0);
    let doc = parse_stdout(&output);
    assert_eq!(
        doc["$schema"], "https://json-schema.org/draft/2020-12/schema",
        "the schema must declare the 2020-12 dialect"
    );
    for key in [
        "status",
        "command",
        "data",
        "context",
        "next_steps",
        "warnings",
        "cost",
        "error",
        "duration_ms",
    ] {
        assert!(
            doc["properties"][key].is_object(),
            "envelope schema must describe {key}: {doc}"
        );
    }
    assert_eq!(doc["additionalProperties"], false);
}

#[test]
fn schema_all_covers_every_command_with_a_schema() {
    let output = forge(&["schema", "--all"]);
    assert_exit(&output, 0);
    let doc = parse_stdout(&output);
    let map = doc.as_object().expect("--all prints one object");
    let expected = forge::cli_schema::commands();
    assert_eq!(
        map.len(),
        expected.len(),
        "--all must cover exactly the schemas table: {map:?}"
    );
    for command in expected {
        assert!(
            map.contains_key(command),
            "--all is missing {command}: {map:?}"
        );
    }
}

#[test]
fn schema_unknown_command_is_a_usage_error() {
    let output = forge(&["schema", "bogus"]);
    assert_exit(&output, 1);
    assert!(output.stdout.is_empty(), "human mode keeps stdout clean");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown command 'bogus'"), "{stderr}");

    let json = forge(&["schema", "bogus", "--json"]);
    assert_exit(&json, 1);
    let doc = parse_stdout(&json);
    assert_eq!(doc["status"], "error");
    assert_eq!(doc["command"], "schema");
    assert_eq!(doc["error"]["type"], "usage");
    assert!(
        doc["error"]["suggestion"]
            .as_str()
            .unwrap_or_default()
            .contains("forge schema --all"),
        "the suggestion must name the listing command: {doc}"
    );
}
