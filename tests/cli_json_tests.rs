// Binary tests for the JSON output envelope, semantic exit codes and
// `--fields` (#475). Each test runs the real `forge` binary so the assertions
// cover the process-level contract: one JSON document on stdout, and the
// documented exit code.

use std::process::{Command, Output};

fn forge(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_forge"))
        .args(args)
        .output()
        .expect("failed to execute forge binary")
}

fn forge_env(args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_forge"));
    cmd.args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.output().expect("failed to execute forge binary")
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

// ── check ───────────────────────────────────────────────────────

#[test]
fn check_json_error_file_reports_code_and_next_step() {
    let output = forge(&["check", "--json", "examples/errors/uncertain_error.forge"]);
    assert_exit(&output, 1);
    let doc = parse_stdout(&output);
    assert_eq!(doc["status"], "error");
    assert_eq!(doc["command"], "check");
    assert_eq!(doc["data"]["diagnostics"][0]["code"], "E020");
    assert_eq!(doc["data"]["diagnostics"][0]["severity"], "error");
    assert_eq!(doc["error"]["code"], "E020");
    let steps = doc["next_steps"].as_array().expect("next_steps array");
    assert!(
        steps.iter().any(|s| s
            .as_str()
            .unwrap_or_default()
            .contains("forge explain E020")),
        "next_steps must suggest `forge explain E020`: {steps:?}"
    );
    // line/col come from the span (the `give result` line).
    assert_eq!(doc["data"]["diagnostics"][0]["line"], 9);
    assert!(doc["data"]["diagnostics"][0]["col"].as_u64().unwrap() > 0);
}

#[test]
fn check_json_warnings_only_exits_two() {
    // tests/fixtures/warnings_only.forge produces exactly one W041 warning.
    let output = forge(&["check", "--json", "tests/fixtures/warnings_only.forge"]);
    assert_exit(&output, 2);
    let doc = parse_stdout(&output);
    assert_eq!(doc["status"], "warning");
    assert!(doc["error"].is_null(), "warnings are not errors: {doc}");
    assert_eq!(doc["warnings"].as_array().unwrap().len(), 1);
    assert!(doc["warnings"][0].as_str().unwrap().starts_with("W041"));
    let severity = doc["data"]["diagnostics"][0]["severity"].as_str().unwrap();
    assert_eq!(severity, "warning");
}

#[test]
fn check_json_clean_file_exits_zero() {
    let output = forge(&["check", "--json", "examples/basics/hello.forge"]);
    assert_exit(&output, 0);
    let doc = parse_stdout(&output);
    assert_eq!(doc["status"], "success");
    assert_eq!(doc["data"]["diagnostics"].as_array().unwrap().len(), 0);
    assert_eq!(doc["cost"], 0.0);
}

#[test]
fn check_fields_trims_data_to_listed_keys() {
    let output = forge(&[
        "check",
        "--json",
        "--fields",
        "diagnostics",
        "examples/errors/uncertain_error.forge",
    ]);
    assert_exit(&output, 1);
    let doc = parse_stdout(&output);
    let data = doc["data"].as_object().expect("data object");
    assert_eq!(data.len(), 1, "data should only hold diagnostics: {data:?}");
    assert!(data.contains_key("diagnostics"));
}

#[test]
fn forge_output_env_enables_json_mode() {
    let output = forge_env(
        &["check", "examples/basics/hello.forge"],
        &[("FORGE_OUTPUT", "json")],
    );
    assert_exit(&output, 0);
    let doc = parse_stdout(&output);
    assert_eq!(doc["status"], "success");
    assert_eq!(doc["command"], "check");
}

// ── human mode stays human ──────────────────────────────────────

#[test]
fn human_mode_output_is_unchanged() {
    let output = forge(&["check", "examples/basics/hello.forge"]);
    assert_exit(&output, 0);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "OK\n");
}

// ── run and trace ───────────────────────────────────────────────

#[test]
fn run_json_keeps_say_output_out_of_stdout() {
    let output = forge_env(
        &["run", "--json", "examples/basics/hello.forge"],
        &[("FORGE_MOCK", "1")],
    );
    assert_exit(&output, 0);
    let doc = parse_stdout(&output);
    assert_eq!(doc["status"], "success");
    assert_eq!(doc["command"], "run");
    assert_eq!(doc["data"]["output"], serde_json::json!(["Hello, World!"]));
    assert_eq!(doc["cost"], 0.0);
    assert!(doc["error"].is_null());
}

#[test]
fn run_human_mode_still_prints_say_output() {
    let output = forge_env(
        &["run", "examples/basics/hello.forge"],
        &[("FORGE_MOCK", "1")],
    );
    assert_exit(&output, 0);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "Hello, World!\n");
}

#[test]
fn run_json_reports_parse_and_blocked_runs() {
    let missing = forge(&["run", "--json", "examples/errors/nope.forge"]);
    assert_exit(&missing, 1);
    let doc = parse_stdout(&missing);
    assert_eq!(doc["status"], "error");
    assert_eq!(doc["error"]["type"], "io");

    // FORGE_MOCK keeps config resolution from failing before the static checks
    // run; the run is then blocked by the E020 diagnostic.
    let blocked = forge_env(
        &["run", "--json", "examples/errors/uncertain_error.forge"],
        &[("FORGE_MOCK", "1")],
    );
    assert_exit(&blocked, 1);
    let doc = parse_stdout(&blocked);
    assert_eq!(doc["status"], "error");
    assert_eq!(doc["error"]["type"], "check");
    assert_eq!(doc["data"]["blocked"], true);
    assert_eq!(doc["data"]["diagnostics"][0]["code"], "E020");
}

#[test]
fn trace_json_envelope_names_the_trace_command() {
    let output = forge_env(
        &["trace", "--json", "examples/basics/hello.forge"],
        &[("FORGE_MOCK", "1")],
    );
    assert_exit(&output, 0);
    let doc = parse_stdout(&output);
    assert_eq!(doc["command"], "trace");
    assert_eq!(doc["data"]["output"], serde_json::json!(["Hello, World!"]));
}

// ── pool workers (#475) ─────────────────────────────────────────

#[test]
fn run_json_pool_worker_say_lands_in_data_output() {
    // tests/fixtures/pool_say.forge runs two LLM workers that each `say`.
    // `parse_stdout` deserializes the *whole* stdout as one JSON document, so a
    // worker `say` reaching stdout makes this test fail.
    let output = forge_env(
        &["run", "--json", "tests/fixtures/pool_say.forge"],
        &[("FORGE_MOCK", "1")],
    );
    assert_exit(&output, 0);
    let doc = parse_stdout(&output);
    assert_eq!(doc["status"], "success");
    assert_eq!(
        doc["data"]["output"],
        serde_json::json!([
            "worker says mock response",
            "worker says mock response",
            "pool done"
        ])
    );
    assert_eq!(doc["cost"], 0.0);
}

#[test]
fn run_json_pool_worker_spend_lands_in_cost() {
    let dir = temp_dir("pool-cost");
    let config = priced_mock_config(&dir);
    let output = forge_env(
        &["run", "--json", "tests/fixtures/pool_say.forge"],
        &[("FORGE_CONFIG", &config)],
    );
    assert_exit(&output, 0);
    let doc = parse_stdout(&output);
    assert!(
        doc["cost"].as_f64().unwrap() > 0.0,
        "two billed pool workers must show up in cost: {doc}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ── test (record / replay, #478) ────────────────────────────────

#[test]
fn test_json_replays_recorded_fixtures() {
    let dir = temp_dir("test-replay");
    let fixtures = dir.join("llm.fixtures.json");
    let fixtures_arg = fixtures.to_string_lossy().to_string();

    // Record once on the mock provider (the fixture file is the only artifact).
    let recorded = forge_env(
        &[
            "run",
            "--json",
            "tests/fixtures/replay_llm.forge",
            "--record",
            &fixtures_arg,
        ],
        &[("FORGE_MOCK", "1")],
    );
    assert_exit(&recorded, 0);
    assert!(fixtures.exists(), "recording must write {}", fixtures_arg);

    let replayed = forge_env(
        &[
            "test",
            "--json",
            "tests/fixtures/replay_llm.forge",
            "--fixtures",
            &fixtures_arg,
        ],
        &[("FORGE_MOCK", "1")],
    );
    assert_exit(&replayed, 0);
    let doc = parse_stdout(&replayed);
    assert_eq!(doc["command"], "test");
    assert_eq!(doc["data"]["output"], serde_json::json!(["mock response"]));
    assert_eq!(doc["cost"], 0.0);

    // --expect compares the replayed stdout against a file.
    let expect = dir.join("expected.txt");
    std::fs::write(&expect, "mock response\n").unwrap();
    let matched = forge_env(
        &[
            "test",
            "--json",
            "tests/fixtures/replay_llm.forge",
            "--fixtures",
            &fixtures_arg,
            "--expect",
            expect.to_str().unwrap(),
        ],
        &[("FORGE_MOCK", "1")],
    );
    assert_exit(&matched, 0);
    let doc = parse_stdout(&matched);
    assert_eq!(doc["data"]["matched"], true);

    std::fs::write(&expect, "something else\n").unwrap();
    let mismatched = forge_env(
        &[
            "test",
            "--json",
            "tests/fixtures/replay_llm.forge",
            "--fixtures",
            &fixtures_arg,
            "--expect",
            expect.to_str().unwrap(),
        ],
        &[("FORGE_MOCK", "1")],
    );
    assert_exit(&mismatched, 1);
    let doc = parse_stdout(&mismatched);
    assert_eq!(doc["status"], "error");
    assert_eq!(doc["data"]["matched"], false);
    assert_eq!(doc["error"]["type"], "runtime");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_json_without_fixtures_exits_one() {
    let output = forge_env(
        &["test", "--json", "tests/fixtures/echo_agent.forge"],
        &[("FORGE_MOCK", "1")],
    );
    assert_exit(&output, 1);
    let doc = parse_stdout(&output);
    assert_eq!(doc["status"], "error");
    assert_eq!(doc["error"]["type"], "io");
    assert!(doc["error"]["suggestion"]
        .as_str()
        .unwrap()
        .contains("--record"));
}

// ── cost, build, export/import/inspect ──────────────────────────

#[test]
fn cost_json_estimates_without_llm_calls() {
    let output = forge(&["cost", "--json", "examples/errors/uncertain_error.forge"]);
    assert_exit(&output, 0);
    let doc = parse_stdout(&output);
    assert_eq!(doc["status"], "success");
    assert_eq!(doc["data"]["operations"][0]["kind"], "reason");
    assert!(doc["data"]["estimated_cost_usd"].as_f64().unwrap() > 0.0);
}

#[test]
fn build_json_dry_run_validates() {
    let output = forge(&[
        "build",
        "--json",
        "--dry-run",
        "examples/basics/hello.forge",
    ]);
    assert_exit(&output, 0);
    let doc = parse_stdout(&output);
    assert_eq!(doc["status"], "success");
    assert_eq!(doc["data"]["dry_run"], true);
    assert_eq!(doc["data"]["built"], false);
}

#[test]
fn package_round_trip_json() {
    let dir = temp_dir("pkg");
    let pkg = dir.join("agent.forgepkg.json");
    let store = dir.join("store");
    let pkg_arg = pkg.to_string_lossy().to_string();
    let store_arg = store.to_string_lossy().to_string();

    let exported = forge(&[
        "export",
        "--json",
        "tests/fixtures/echo_agent.forge",
        "--output",
        &pkg_arg,
    ]);
    assert_exit(&exported, 0);
    let doc = parse_stdout(&exported);
    assert_eq!(doc["command"], "export");
    assert!(pkg.exists(), "export must write the package file");

    let inspected = forge(&["inspect", "--json", &pkg_arg]);
    assert_exit(&inspected, 0);
    let doc = parse_stdout(&inspected);
    assert_eq!(doc["status"], "success");
    assert!(doc["data"]["inspection"]
        .as_str()
        .unwrap()
        .contains("FORGE Package"));

    let imported = forge(&["import", "--json", &pkg_arg, "--into", &store_arg]);
    assert_exit(&imported, 0);
    let doc = parse_stdout(&imported);
    assert_eq!(doc["data"]["imported"], 0);
    assert_eq!(doc["data"]["into"], store_arg);

    let _ = std::fs::remove_dir_all(&dir);
}

// ── send, wake, store ───────────────────────────────────────────

#[test]
fn send_json_returns_the_handler_value() {
    let dir = temp_dir("send");
    let storage = dir.join("storage");
    let output = forge_env(
        &["send", "--json", "tests/fixtures/echo_agent.forge", "ping"],
        &[
            ("FORGE_MOCK", "1"),
            ("FORGE_STORAGE_ROOT", storage.to_str().unwrap()),
        ],
    );
    assert_exit(&output, 0);
    let doc = parse_stdout(&output);
    assert_eq!(doc["status"], "success");
    assert_eq!(doc["data"]["agent"], "echo");
    assert_eq!(doc["data"]["result"], "pong");
    // The send path carries a cost tracker now: a deterministic handler spent
    // nothing, and that is a measured `0.0`, not `null` (#475).
    assert_eq!(doc["cost"], 0.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn send_json_reports_llm_handler_spend() {
    let dir = temp_dir("send-cost");
    let storage = dir.join("storage");
    let config = priced_mock_config(&dir);
    let output = forge_env(
        &["send", "--json", "tests/fixtures/say_agent.forge", "ask"],
        &[
            ("FORGE_CONFIG", &config),
            ("FORGE_STORAGE_ROOT", storage.to_str().unwrap()),
        ],
    );
    assert_exit(&output, 0);
    let doc = parse_stdout(&output);
    assert_eq!(doc["status"], "success");
    assert!(
        doc["cost"].as_f64().unwrap() > 0.0,
        "the handler's LLM spend must reach the envelope's cost: {doc}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn send_json_keeps_the_handler_say_out_of_stdout() {
    let dir = temp_dir("send-say");
    let storage = dir.join("storage");
    let envs = [
        ("FORGE_MOCK", "1"),
        ("FORGE_STORAGE_ROOT", storage.to_str().unwrap()),
    ];

    let output = forge_env(
        &["send", "--json", "tests/fixtures/say_agent.forge", "ping"],
        &envs,
    );
    assert_exit(&output, 0);
    // One JSON document on the whole stdout: a handler `say` leaking here would
    // break this parse.
    let doc = parse_stdout(&output);
    assert_eq!(doc["status"], "success");
    assert_eq!(doc["data"]["result"], "pong");
    assert_eq!(
        doc["data"]["output"],
        serde_json::json!(["handler says ping"])
    );

    // Human mode keeps the handler's `say` and the returned value.
    let human = forge_env(&["send", "tests/fixtures/say_agent.forge", "ping"], &envs);
    assert_exit(&human, 0);
    let stdout = String::from_utf8_lossy(&human.stdout);
    assert!(
        stdout.contains("handler says ping") && stdout.contains("pong"),
        "human send should print the say and the value: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wake_and_store_json() {
    let dir = temp_dir("wake");
    let storage = dir.join("storage");
    let storage_arg = storage.to_string_lossy().to_string();
    std::fs::create_dir_all(&storage).unwrap();

    let listed = forge_env(
        &["wake", "list", "--json"],
        &[("FORGE_STORAGE_ROOT", &storage_arg)],
    );
    assert_exit(&listed, 0);
    let doc = parse_stdout(&listed);
    assert_eq!(doc["command"], "wake");
    assert_eq!(doc["data"]["action"], "list");
    assert_eq!(doc["data"]["triggers"].as_array().unwrap().len(), 0);

    let recovered = forge(&["store", "recover", "--json", "--root", &storage_arg]);
    assert_exit(&recovered, 0);
    let doc = parse_stdout(&recovered);
    assert_eq!(doc["data"]["action"], "recover");
    assert_eq!(doc["data"]["broken"], 0);
    let _ = std::fs::remove_dir_all(&dir);
}

// ── out-of-scope commands ───────────────────────────────────────

#[test]
fn out_of_scope_commands_report_unsupported() {
    let cases: Vec<Vec<&str>> = vec![
        vec!["serve", "--json", "examples/basics/hello.forge"],
        vec!["agent", "--json", "examples/basics/hello.forge"],
        vec!["agent-inspect", "--json", "examples/basics/hello.forge"],
        vec!["fleet", "--json", "--spec", "a logger"],
    ];
    for args in cases {
        let output = forge_env(&args, &[("FORGE_MOCK", "1")]);
        assert_exit(&output, 1);
        let doc = parse_stdout(&output);
        let command = doc["command"].as_str().unwrap().to_string();
        assert_eq!(doc["status"], "error");
        assert_eq!(doc["error"]["type"], "unsupported");
        assert!(
            doc["error"]["message"].as_str().unwrap().contains(&command),
            "message should name the command: {doc}"
        );
    }
}

// ── usage errors (#475) ─────────────────────────────────────────

#[test]
fn usage_error_exits_one_with_a_json_envelope() {
    // Missing required argument.
    let missing = forge(&["check", "--json"]);
    assert_exit(&missing, 1);
    let doc = parse_stdout(&missing);
    assert_eq!(doc["status"], "error");
    assert_eq!(doc["command"], "check");
    assert_eq!(doc["error"]["type"], "usage");
    assert_eq!(doc["error"]["suggestion"], "run forge check --help");
    let message = doc["error"]["message"].as_str().unwrap();
    assert!(message.contains("Usage"), "clap's message: {message}");
    assert!(!message.contains('\u{1b}'), "no ANSI in JSON: {message:?}");

    // Unknown flag.
    let unknown = forge(&["run", "--json", "--bogus", "examples/basics/hello.forge"]);
    assert_exit(&unknown, 1);
    let doc = parse_stdout(&unknown);
    assert_eq!(doc["command"], "run");
    assert_eq!(doc["error"]["type"], "usage");
    assert_eq!(doc["error"]["suggestion"], "run forge run --help");

    // FORGE_OUTPUT=json is the other JSON trigger.
    let env_mode = forge_env(&["check"], &[("FORGE_OUTPUT", "json")]);
    assert_exit(&env_mode, 1);
    let doc = parse_stdout(&env_mode);
    assert_eq!(doc["command"], "check");
    assert_eq!(doc["error"]["type"], "usage");

    // No subcommand at all: empty `command`, top-level help suggestion.
    let bare = forge(&["--json"]);
    assert_exit(&bare, 1);
    let doc = parse_stdout(&bare);
    assert_eq!(doc["command"], "");
    assert_eq!(doc["error"]["suggestion"], "run forge --help");
}

#[test]
fn usage_error_exits_one_in_human_mode() {
    for args in [
        vec!["check"],
        vec!["run", "--bogus", "examples/basics/hello.forge"],
    ] {
        let output = forge_env(&args, &[("FORGE_MOCK", "1")]);
        // Exit 1, never clap's 2: exit 2 means "warnings only" and nothing else.
        assert_ne!(
            output.status.code(),
            Some(2),
            "a usage error must not collide with the warnings-only code: {args:?}"
        );
        assert_exit(&output, 1);
        assert!(
            output.stdout.is_empty(),
            "human usage errors keep stdout empty: {:?}",
            String::from_utf8_lossy(&output.stdout)
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("Usage"),
            "clap's message goes to stderr for {args:?}: {stderr}"
        );
    }
}

#[test]
fn help_and_version_keep_exit_zero() {
    let help = forge(&["--help"]);
    assert_exit(&help, 0);
    assert!(String::from_utf8_lossy(&help.stdout).contains("Usage"));

    let sub_help = forge(&["check", "--help"]);
    assert_exit(&sub_help, 0);
    assert!(String::from_utf8_lossy(&sub_help.stdout).contains("Usage"));

    let version = forge(&["--version"]);
    assert_exit(&version, 0);
    assert!(
        String::from_utf8_lossy(&version.stdout).starts_with("forge "),
        "version output: {:?}",
        String::from_utf8_lossy(&version.stdout)
    );
}

fn temp_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("forge-cli-json-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// A `forge.config.toml` whose mock provider bills like a paid one, so tests
/// can assert that LLM spend reaches the envelope's `cost` without a real
/// provider (the default mock is free).
fn priced_mock_config(dir: &std::path::Path) -> String {
    let path = dir.join("forge.config.toml");
    std::fs::write(
        &path,
        "[llm]\n\
         default = \"mock\"\n\n\
         [providers.mock]\n\
         type = \"mock\"\n\n\
         [providers.mock.capabilities]\n\
         cost_per_1k_input = 1.0\n\
         cost_per_1k_output = 1.0\n",
    )
    .expect("write priced mock config");
    path.to_string_lossy().to_string()
}

// ── parse and explain ───────────────────────────────────────────

#[test]
fn parse_json_reports_ast_and_parse_errors() {
    let output = forge(&["parse", "--json", "examples/basics/hello.forge"]);
    assert_exit(&output, 0);
    let doc = parse_stdout(&output);
    assert_eq!(doc["status"], "success");
    assert_eq!(doc["command"], "parse");
    assert!(
        doc["data"]["ast"].as_str().unwrap().contains("FnMain"),
        "parse --json should carry the AST tree: {}",
        doc["data"]["ast"]
    );

    let bad = forge(&["parse", "--json", "examples/errors/does-not-exist.forge"]);
    assert_exit(&bad, 1);
    let doc = parse_stdout(&bad);
    assert_eq!(doc["status"], "error");
    assert_eq!(doc["error"]["type"], "io");
}

#[test]
fn explain_json_lists_and_explains_codes() {
    let one = forge(&["explain", "--json", "E020"]);
    assert_exit(&one, 0);
    let doc = parse_stdout(&one);
    assert_eq!(doc["data"]["code"], "E020");
    assert!(!doc["data"]["explain"].as_str().unwrap().is_empty());

    let list = forge(&["explain", "--json", "--list"]);
    assert_exit(&list, 0);
    let doc = parse_stdout(&list);
    assert!(doc["data"]["codes"].as_array().unwrap().len() > 10);

    let unknown = forge(&["explain", "--json", "E999"]);
    assert_exit(&unknown, 1);
    let doc = parse_stdout(&unknown);
    assert_eq!(doc["status"], "error");
    assert_eq!(doc["error"]["code"], serde_json::Value::Null);
}
