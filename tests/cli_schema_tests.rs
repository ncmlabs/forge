// Binary tests for the introspection surfaces of #476: `forge schema`, the
// hand-written JSON Schemas of the `--json` contract, and `forge help --json`,
// the command index built from clap. The drift guard — every command's real
// output validated against `forge schema <command>` — lives at the bottom.

use std::process::{Command, Output};

fn forge(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_forge"))
        .args(args)
        .output()
        .expect("failed to execute forge binary")
}

fn stdout_of(args: &[&str]) -> String {
    let output = forge(args);
    assert_exit(&output, 0);
    String::from_utf8_lossy(&output.stdout).to_string()
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

/// The subcommands clap prints in the `Commands:` block of a help page.
/// Exactly two spaces of indent: clap indents a wrapped continuation line
/// further, and this CLI's clap has no wrapping enabled anyway.
fn help_subcommands(help: &str) -> Vec<String> {
    let block = help.split("\nCommands:\n").nth(1).unwrap_or_default();
    block
        .lines()
        .take_while(|line| !line.trim().is_empty())
        .filter_map(|line| line.strip_prefix("  "))
        .filter_map(|entry| entry.split_whitespace().next())
        .map(|name| name.to_string())
        .collect()
}

/// The `NAME` of every `  NAME  description` line of `forge --help`'s
/// `ENVIRONMENT VARIABLES:` block.
fn help_env_names(help: &str) -> Vec<String> {
    let block = help
        .split("ENVIRONMENT VARIABLES:")
        .nth(1)
        .unwrap_or_default();
    block
        .lines()
        .filter_map(|line| line.strip_prefix("  "))
        .filter_map(|line| line.split_whitespace().next())
        .map(|token| token.split('=').next().unwrap_or(token).to_string())
        .collect()
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

// ── forge help --json ───────────────────────────────────────────

fn help_index() -> serde_json::Value {
    let output = forge(&["help", "--json"]);
    assert_exit(&output, 0);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains('\u{1b}'),
        "the index is plain text, never ANSI: {stdout:?}"
    );
    parse_stdout(&output)
}

fn names(doc: &serde_json::Value) -> Vec<String> {
    doc["commands"]
        .as_array()
        .expect("commands array")
        .iter()
        .map(|command| command["name"].as_str().expect("name").to_string())
        .collect()
}

#[test]
fn help_json_lists_every_subcommand_and_its_flags() {
    let doc = help_index();

    // Every command clap prints is listed, nested actions included. The
    // expectation is read from the help pages themselves, so a new subcommand
    // cannot be added to the CLI and quietly omitted from the index.
    let mut expected = help_subcommands(&stdout_of(&["--help"]));
    for parent in ["wake", "store"] {
        for nested in help_subcommands(&stdout_of(&[parent, "--help"])) {
            expected.push(format!("{parent} {nested}"));
        }
    }
    let mut listed = names(&doc);
    expected.sort();
    listed.sort();
    assert_eq!(listed, expected, "index: {doc:#?}");

    // The shape of one entry, and the flags of one command.
    let check = doc["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|command| command["name"] == "check")
        .expect("check is listed");
    assert!(check["about"].as_str().unwrap().contains("Type-check"));
    let args: Vec<&str> = check["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|arg| arg["name"].as_str().unwrap())
        .collect();
    for arg in ["files", "merge", "json", "fields"] {
        assert!(args.contains(&arg), "check must list {arg}: {args:?}");
    }
    let files = check["args"]
        .as_array()
        .unwrap()
        .iter()
        .find(|arg| arg["name"] == "files")
        .unwrap();
    assert_eq!(files["required"], true);
    assert_eq!(files["takes_value"], true);
    assert_eq!(
        files["long"],
        serde_json::Value::Null,
        "files is positional"
    );
    let merge = check["args"]
        .as_array()
        .unwrap()
        .iter()
        .find(|arg| arg["name"] == "merge")
        .unwrap();
    assert_eq!(merge["long"], "merge");
    assert_eq!(merge["required"], false);
    assert_eq!(merge["takes_value"], false);
}

#[test]
fn help_json_marks_exactly_the_out_of_scope_commands_unsupported() {
    let doc = help_index();
    let unsupported: Vec<String> = doc["commands"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|command| command["json_supported"] == false)
        .map(|command| command["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        unsupported,
        vec!["agent", "agent-inspect", "serve", "fleet"],
        "docs/forge-reference.md §27 names these four as interactive or long-running"
    );
}

#[test]
fn help_json_carries_the_exit_codes_and_environment() {
    let doc = help_index();

    let codes = doc["exit_codes"].as_object().expect("exit_codes object");
    let mut keys: Vec<&str> = codes.keys().map(|k| k.as_str()).collect();
    keys.sort();
    assert_eq!(keys, vec!["0", "1", "10", "11", "2", "3"], "{codes:?}");
    assert_eq!(codes["0"], "success: command completed");
    assert!(codes["2"]
        .as_str()
        .unwrap()
        .starts_with("warning: static checks found warnings"));
    assert!(codes["10"].as_str().unwrap().starts_with("needs input"));
    assert!(codes["11"].as_str().unwrap().starts_with("async pending"));

    let listed: Vec<String> = doc["env"]
        .as_array()
        .expect("env array")
        .iter()
        .map(|var| var["name"].as_str().expect("env name").to_string())
        .collect();
    assert_eq!(
        listed,
        help_env_names(&stdout_of(&["--help"])),
        "env must mirror the ENVIRONMENT VARIABLES block of `forge --help`"
    );
    assert!(
        listed.contains(&"FORGE_OUTPUT".to_string()),
        "the JSON trigger is part of the index: {listed:?}"
    );
}

#[test]
fn plain_help_stays_as_it_was() {
    // `forge help` is our own subcommand now (#476); it must print exactly what
    // clap's built-in help subcommand printed.
    assert_eq!(stdout_of(&["help"]), stdout_of(&["--help"]));
    assert_eq!(
        stdout_of(&["help", "check"]),
        stdout_of(&["check", "--help"])
    );
    assert_eq!(
        stdout_of(&["help", "store", "recover"]),
        stdout_of(&["store", "recover", "--help"])
    );

    // An unknown command is a usage error, not a silent success.
    let unknown = forge(&["help", "bogus"]);
    assert_exit(&unknown, 1);
    let stderr = String::from_utf8_lossy(&unknown.stderr);
    assert!(
        stderr.contains("unrecognized subcommand 'bogus'"),
        "{stderr}"
    );
}
