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
    for arg in ["files", "merge", "manifest", "json", "fields"] {
        assert!(args.contains(&arg), "check must list {arg}: {args:?}");
    }
    let files = check["args"]
        .as_array()
        .unwrap()
        .iter()
        .find(|arg| arg["name"] == "files")
        .unwrap();
    // `required` is clap's own flag, and `check`'s files are
    // `required_unless_present = "manifest"` — clap prints `[FILES]...`, so the
    // index reports `false` here. `parse`'s file is unconditionally required.
    assert_eq!(files["required"], false);
    assert_eq!(files["takes_value"], true);
    assert_eq!(
        files["long"],
        serde_json::Value::Null,
        "files is positional"
    );
    let parse = doc["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|command| command["name"] == "parse")
        .expect("parse is listed");
    let file = parse["args"]
        .as_array()
        .unwrap()
        .iter()
        .find(|arg| arg["name"] == "file")
        .expect("parse takes a file");
    assert_eq!(file["required"], true);
    assert_eq!(file["long"], serde_json::Value::Null);
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

// ── drift guard: real output must validate against the schema ───
//
// Every envelope-producing command is run for real — mock provider, fixtures
// under `tests/fixtures/`, temp dirs for anything that writes — and its stdout
// is validated against the document `forge schema <command>` prints. A payload
// that gains, renames, retypes or drops a key fails here until the schema in
// `schemas/` is updated, which is the point of hand-written schemas.

use std::io::Write;
use std::process::Stdio;

fn forge_env(args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_forge"));
    cmd.args(args);
    for (key, value) in envs {
        cmd.env(key, value);
    }
    cmd.output().expect("failed to execute forge binary")
}

/// `forge ...` with `stdin` written to the child — `wake register` reads the
/// secret from it.
fn forge_stdin(args: &[&str], envs: &[(&str, &str)], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_forge"))
        .args(args)
        .envs(envs.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn forge binary");
    child
        .stdin
        .as_mut()
        .expect("child stdin")
        .write_all(stdin.as_bytes())
        .expect("write child stdin");
    child.wait_with_output().expect("wait for forge binary")
}

fn temp_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("forge-schema-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Run one invocation and validate its envelope against `forge schema
/// <command>`. The validator is compiled from the CLI's own output, so the test
/// exercises what a caller gets, `$defs` references included.
fn assert_validates(command: &str, exit: i32, args: &[&str], envs: &[(&str, &str)]) {
    assert_validates_with(command, exit, args, envs, None)
}

/// As above, for commands that read a secret from stdin (`wake register`).
fn assert_validates_stdin(
    command: &str,
    exit: i32,
    args: &[&str],
    envs: &[(&str, &str)],
    stdin: &str,
) {
    assert_validates_with(command, exit, args, envs, Some(stdin))
}

fn assert_validates_with(
    command: &str,
    exit: i32,
    args: &[&str],
    envs: &[(&str, &str)],
    stdin: Option<&str>,
) {
    let schema_doc = forge(&["schema", command]);
    assert_exit(&schema_doc, 0);
    let schema = parse_stdout(&schema_doc);
    let validator = jsonschema::validator_for(&schema)
        .unwrap_or_else(|e| panic!("`forge schema {command}` is not a usable schema: {e}"));

    let output = match stdin {
        Some(stdin) => forge_stdin(args, envs, stdin),
        None => forge_env(args, envs),
    };
    assert_eq!(
        output.status.code(),
        Some(exit),
        "forge {args:?} exited {:?}, expected {exit}; stderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let doc = parse_stdout(&output);
    let errors: Vec<String> = validator
        .iter_errors(&doc)
        .map(|error| format!("{error} at {}", error.instance_path()))
        .collect();
    assert!(
        errors.is_empty(),
        "forge {args:?} does not match `forge schema {command}`:\n{}\n{doc:#?}",
        errors.join("\n")
    );
}

const MOCK: &[(&str, &str)] = &[("FORGE_MOCK", "1")];

#[test]
fn parse_output_matches_its_schema() {
    assert_validates(
        "parse",
        0,
        &["parse", "--json", "examples/basics/hello.forge"],
        &[],
    );

    // A parse failure, written here rather than kept as a fixture: every
    // committed `.forge` file is classified in `examples/validation.toml` and
    // must parse (`tests/example_validation_tests.rs`), and this one must not.
    let dir = temp_dir("parse");
    let broken = dir.join("broken.forge");
    std::fs::write(&broken, "task broken\n  do\n   say \"three spaces\"\n")
        .expect("write a source that does not parse");
    let broken_arg = broken.to_string_lossy().to_string();
    assert_validates("parse", 1, &["parse", "--json", &broken_arg], &[]);

    assert_validates(
        "parse",
        1,
        &["parse", "--json", "examples/errors/nope.forge"],
        &[],
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_output_matches_its_schema() {
    assert_validates(
        "check",
        0,
        &["check", "--json", "examples/basics/hello.forge"],
        &[],
    );
    assert_validates(
        "check",
        1,
        &["check", "--json", "examples/errors/uncertain_error.forge"],
        &[],
    );
    assert_validates(
        "check",
        2,
        &["check", "--json", "tests/fixtures/warnings_only.forge"],
        &[],
    );
    assert_validates(
        "check",
        1,
        &["check", "--json", "examples/errors/nope.forge"],
        &[],
    );
}

#[test]
fn explain_output_matches_its_schema() {
    assert_validates("explain", 0, &["explain", "--json", "E020"], &[]);
    assert_validates("explain", 0, &["explain", "--json", "--list"], &[]);
    assert_validates("explain", 1, &["explain", "--json", "E999"], &[]);
}

#[test]
fn cost_output_matches_its_schema() {
    assert_validates(
        "cost",
        0,
        &["cost", "--json", "examples/errors/uncertain_error.forge"],
        &[],
    );
    assert_validates(
        "cost",
        0,
        &["cost", "--json", "examples/basics/hello.forge"],
        &[],
    );
    assert_validates(
        "cost",
        1,
        &["cost", "--json", "examples/errors/nope.forge"],
        &[],
    );
}

#[test]
fn build_output_matches_its_schema() {
    assert_validates(
        "build",
        0,
        &[
            "build",
            "--json",
            "--dry-run",
            "examples/basics/hello.forge",
        ],
        &[],
    );
    assert_validates(
        "build",
        1,
        &["build", "--json", "examples/errors/nope.forge"],
        &[],
    );
}

#[test]
fn run_output_matches_its_schema() {
    assert_validates(
        "run",
        0,
        &["run", "--json", "examples/basics/hello.forge"],
        MOCK,
    );
    // Blocked by static checks: the `blocked` payload.
    assert_validates(
        "run",
        1,
        &["run", "--json", "examples/errors/uncertain_error.forge"],
        MOCK,
    );
    // An io failure reaches the envelope before any payload exists (`{}`).
    assert_validates(
        "run",
        1,
        &["run", "--json", "examples/errors/nope.forge"],
        MOCK,
    );
    // Worker `say` lines join `data.output` instead of stdout.
    assert_validates(
        "run",
        0,
        &["run", "--json", "tests/fixtures/pool_say.forge"],
        MOCK,
    );
}

#[test]
fn trace_output_matches_its_schema() {
    assert_validates(
        "trace",
        0,
        &["trace", "--json", "examples/basics/hello.forge"],
        MOCK,
    );
    assert_validates(
        "trace",
        1,
        &["trace", "--json", "examples/errors/uncertain_error.forge"],
        MOCK,
    );
}

#[test]
fn test_output_matches_its_schema() {
    // No fixtures recorded yet: the `io` envelope names the fixture path.
    assert_validates(
        "test",
        1,
        &["test", "--json", "tests/fixtures/echo_agent.forge"],
        MOCK,
    );

    let dir = temp_dir("test");
    let fixtures = dir.join("llm.fixtures.json");
    let fixtures_arg = fixtures.to_string_lossy().to_string();
    let expected = dir.join("expected.txt");
    let expected_arg = expected.to_string_lossy().to_string();

    let recorded = forge_env(
        &[
            "run",
            "--json",
            "tests/fixtures/replay_llm.forge",
            "--record",
            &fixtures_arg,
        ],
        MOCK,
    );
    assert_exit(&recorded, 0);
    std::fs::write(&expected, "mock response\n").expect("write expected output");

    assert_validates(
        "test",
        0,
        &[
            "test",
            "--json",
            "tests/fixtures/replay_llm.forge",
            "--fixtures",
            &fixtures_arg,
        ],
        MOCK,
    );
    assert_validates(
        "test",
        0,
        &[
            "test",
            "--json",
            "tests/fixtures/replay_llm.forge",
            "--fixtures",
            &fixtures_arg,
            "--expect",
            &expected_arg,
        ],
        MOCK,
    );

    // A mismatch is an error envelope that still carries `matched: false`.
    std::fs::write(&expected, "something else\n").expect("rewrite expected output");
    assert_validates(
        "test",
        1,
        &[
            "test",
            "--json",
            "tests/fixtures/replay_llm.forge",
            "--fixtures",
            &fixtures_arg,
            "--expect",
            &expected_arg,
        ],
        MOCK,
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn send_output_matches_its_schema() {
    let dir = temp_dir("send");
    let storage = dir.join("storage");
    let storage_arg = storage.to_string_lossy().to_string();
    let envs: Vec<(&str, &str)> = vec![
        ("FORGE_MOCK", "1"),
        ("FORGE_STORAGE_ROOT", storage_arg.as_str()),
    ];
    assert_validates(
        "send",
        0,
        &["send", "--json", "tests/fixtures/echo_agent.forge", "ping"],
        &envs,
    );
    // A say-ing handler: its lines land in `data.output`.
    assert_validates(
        "send",
        0,
        &["send", "--json", "tests/fixtures/say_agent.forge", "ping"],
        &envs,
    );
    // Unknown event: the same shape, `result: null`, `error.type = runtime`.
    assert_validates(
        "send",
        1,
        &["send", "--json", "tests/fixtures/echo_agent.forge", "nope"],
        &envs,
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wake_output_matches_its_schema() {
    let dir = temp_dir("wake");
    let storage = dir.join("storage");
    std::fs::create_dir_all(&storage).expect("create storage root");
    let storage_arg = storage.to_string_lossy().to_string();
    let envs: Vec<(&str, &str)> = vec![("FORGE_STORAGE_ROOT", storage_arg.as_str())];

    // One invocation per action: the schema has a branch for each.
    assert_validates_stdin(
        "wake",
        0,
        &[
            "wake",
            "register",
            "--json",
            "--agent",
            "a",
            "--trigger",
            "t",
        ],
        &envs,
        "s3cret\n",
    );
    assert_validates(
        "wake",
        0,
        &["wake", "rotate", "--json", "--agent", "a", "--trigger", "t"],
        &envs,
    );
    assert_validates("wake", 0, &["wake", "list", "--json"], &envs);
    assert_validates(
        "wake",
        0,
        &["wake", "delete", "--json", "--agent", "a", "--trigger", "t"],
        &envs,
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn store_output_matches_its_schema() {
    let dir = temp_dir("store");
    let storage = dir.join("storage");
    std::fs::create_dir_all(&storage).expect("create storage root");
    let storage_arg = storage.to_string_lossy().to_string();

    assert_validates(
        "store",
        0,
        &["store", "recover", "--json", "--root", &storage_arg],
        &[],
    );
    assert_validates(
        "store",
        0,
        &[
            "store",
            "recover",
            "--json",
            "--dry-run",
            "--root",
            &storage_arg,
        ],
        &[],
    );
    // A missing root fails before any payload exists.
    assert_validates(
        "store",
        1,
        &[
            "store",
            "recover",
            "--json",
            "--root",
            "/nonexistent/storage",
        ],
        &[],
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn export_import_and_inspect_output_match_their_schemas() {
    let dir = temp_dir("pkg");
    let pkg = dir.join("agent.forgepkg.json");
    let pkg_arg = pkg.to_string_lossy().to_string();
    let into = dir.join("store");
    let into_arg = into.to_string_lossy().to_string();

    assert_validates(
        "export",
        0,
        &[
            "export",
            "--json",
            "tests/fixtures/echo_agent.forge",
            "--output",
            &pkg_arg,
        ],
        &[],
    );
    // `--layers` is split on commas, so the payload echoes a list.
    assert_validates(
        "export",
        0,
        &[
            "export",
            "--json",
            "tests/fixtures/echo_agent.forge",
            "--layers",
            "config,knowledge,memory",
            "--output",
            &pkg_arg,
        ],
        &[],
    );
    assert_validates(
        "export",
        1,
        &["export", "--json", "examples/errors/nope.forge"],
        &[],
    );

    assert_validates("inspect", 0, &["inspect", "--json", &pkg_arg], &[]);
    assert_validates("inspect", 1, &["inspect", "--json", "/nope.json"], &[]);
    assert_validates(
        "import",
        0,
        &["import", "--json", &pkg_arg, "--into", &into_arg],
        &[],
    );
    assert_validates(
        "import",
        1,
        &["import", "--json", "/nope.json", "--into", &into_arg],
        &[],
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn init_output_matches_its_schema() {
    let dir = temp_dir("init");
    let dry = dir.join("dry");
    let dry_arg = dry.to_string_lossy().to_string();
    let real = dir.join("real");
    let real_arg = real.to_string_lossy().to_string();

    // `--dry-run` writes nothing; the plain run reports the files it wrote.
    assert_validates("init", 0, &["init", "--json", &dry_arg, "--dry-run"], &[]);
    assert_validates("init", 0, &["init", "--json", &real_arg], &[]);
    assert!(
        real.join("main.forge").exists(),
        "init must write the template into {}",
        real.display()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ── the guard's own guards ──────────────────────────────────────

/// Every command exercised above, one drift test each. Checked against the
/// schemas the binary embeds in both directions: a schema without a test fails
/// here, and so does a test for a command that has no schema.
const COVERED: &[&str] = &[
    "parse", "check", "explain", "run", "trace", "test", "cost", "build", "export", "import",
    "inspect", "send", "wake", "store", "init",
];

#[test]
fn every_command_with_a_schema_has_a_drift_test() {
    let mut covered = COVERED.to_vec();
    covered.sort_unstable();
    let mut schemas = forge::cli_schema::commands();
    schemas.sort_unstable();
    assert_eq!(
        covered, schemas,
        "COVERED and `forge schema --all` must name the same commands"
    );
}

#[test]
fn every_schema_is_a_valid_2020_12_schema() {
    for (command, schema) in [
        ("(envelope)".to_string(), forge::cli_schema::envelope_only()),
        ("--all".to_string(), forge::cli_schema::all()),
    ] {
        jsonschema::meta::validate(&schema)
            .unwrap_or_else(|e| panic!("{command} is not a valid JSON Schema: {e}"));
    }
    for command in forge::cli_schema::commands() {
        let schema = forge::cli_schema::envelope_for(command).expect("schema");
        jsonschema::meta::validate(&schema)
            .unwrap_or_else(|e| panic!("{command} is not a valid JSON Schema: {e}"));
    }
}

#[test]
fn a_wrong_document_fails_validation() {
    let output = forge(&["check", "--json", "examples/errors/uncertain_error.forge"]);
    assert_exit(&output, 1);
    let real = parse_stdout(&output);

    let schema = parse_stdout(&forge(&["schema", "check"]));
    let validator = jsonschema::validator_for(&schema).expect("compile schema");
    assert!(
        validator.is_valid(&real),
        "the real envelope must validate first"
    );

    // A payload of the wrong type.
    let mut wrong = real.clone();
    wrong["data"]["files"] = serde_json::json!("examples/errors/uncertain_error.forge");
    assert!(!validator.is_valid(&wrong), "files must be an array");

    // A key the command does not emit.
    let mut wrong = real.clone();
    wrong["data"]["undeclared"] = serde_json::json!(1);
    assert!(!validator.is_valid(&wrong), "data is closed");

    // A diagnostic missing a documented field.
    let mut wrong = real.clone();
    wrong["data"]["diagnostics"][0]
        .as_object_mut()
        .expect("diagnostic object")
        .remove("severity");
    assert!(!validator.is_valid(&wrong), "severity is required");

    // An envelope field that is not in the contract.
    let mut wrong = real.clone();
    wrong["status"] = serde_json::json!("nearly");
    assert!(!validator.is_valid(&wrong), "status is an enum");

    // A missing envelope field.
    let mut wrong = real;
    wrong.as_object_mut().expect("envelope").remove("context");
    assert!(!validator.is_valid(&wrong), "context is required");
}
