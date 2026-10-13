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
fn check_explicit_files_without_merge_are_checked_independently() {
    // Two unrelated programs, each with its own `fn main`: without `--merge`
    // every file is checked on its own, so a batch of examples stays OK (#544).
    let files = [
        "examples/basics/hello.forge",
        "examples/basics/structured_config.forge",
    ];
    let output = forge(&["check", files[0], files[1]]);
    assert_exit(&output, 0);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "OK\n");

    // With `--merge` they are one program, and two `fn main` blocks cannot merge.
    let merged = forge(&["check", "--merge", files[0], files[1]]);
    assert_exit(&merged, 1);
    let stderr = String::from_utf8_lossy(&merged.stderr);
    assert!(
        stderr.contains("multiple `fn main` blocks found"),
        "expected the merge error: {stderr}"
    );
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
fn run_json_duration_ms_is_wall_clock_time() {
    // #479: the clock used to start inside `emit`, so every envelope reported
    // ~0 ms. A run whose `command` sleeps 200 ms must report at least that.
    let output = forge_env(
        &["run", "--json", "tests/fixtures/slow_command.forge"],
        &[("FORGE_MOCK", "1")],
    );
    assert_exit(&output, 0);
    let doc = parse_stdout(&output);
    let duration = doc["duration_ms"]
        .as_u64()
        .expect("duration_ms is a number");
    assert!(
        duration >= 200,
        "duration_ms must be the command's wall-clock time, got {duration} ms: {doc}"
    );
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
fn run_json_spawned_child_say_lands_in_data_output() {
    // tests/fixtures/spawn_child_say.forge prints only from a spawned agent's
    // `on start`; the child shares the parent's buffer, so the line is in
    // `data.output` and stdout stays one JSON document.
    let output = forge_env(
        &["run", "--json", "tests/fixtures/spawn_child_say.forge"],
        &[("FORGE_MOCK", "1")],
    );
    assert_exit(&output, 0);
    let doc = parse_stdout(&output);
    assert_eq!(doc["status"], "success");
    assert_eq!(doc["data"]["output"], serde_json::json!(["CHILD-SAYS"]));
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

// ── end-of-run summary (#479) ───────────────────────────────────

#[test]
fn run_json_summary_counts_calls_tokens_and_matches_cost() {
    let dir = temp_dir("summary-cost");
    let config = priced_mock_config(&dir);
    // pool_say runs two billed LLM workers, so the summary must show calls,
    // tokens and the same USD as the envelope's `cost`.
    let output = forge_env(
        &["run", "--json", "tests/fixtures/pool_say.forge"],
        &[("FORGE_CONFIG", &config)],
    );
    assert_exit(&output, 0);
    let doc = parse_stdout(&output);
    let summary = &doc["data"]["summary"];
    let calls = summary["llm_calls"].as_u64().expect("llm_calls number");
    // Exactly the two workers: `>= 1` would pass if one worker's call were
    // dropped (#479 review).
    assert_eq!(
        calls, 2,
        "both pool workers' LLM calls must be counted: {doc}"
    );
    assert!(summary["tokens_in"].as_u64().unwrap() > 0, "{doc}");
    assert!(summary["tokens_out"].as_u64().unwrap() > 0, "{doc}");
    assert_eq!(
        summary["cost_usd"].as_f64().unwrap(),
        doc["cost"].as_f64().unwrap(),
        "data.summary.cost_usd must match the envelope cost: {doc}"
    );
    let p50 = summary["confidence"]["p50"].as_f64().expect("p50");
    let min = summary["confidence"]["min"].as_f64().expect("min");
    assert!(min <= p50, "the spread must be a spread: {doc}");
    assert!(summary["duration_ms"].as_u64().unwrap() > 0, "{doc}");
    // JSON mode carries the summary in the envelope, never on stderr.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("LLM call"),
        "JSON mode must not print the human summary line: {stderr:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_json_summary_of_a_zero_llm_program_is_zero() {
    // examples/basics/hello.forge makes no LLM call at all.
    let output = forge_env(
        &["run", "--json", "examples/basics/hello.forge"],
        &[("FORGE_MOCK", "1")],
    );
    assert_exit(&output, 0);
    let doc = parse_stdout(&output);
    let summary = &doc["data"]["summary"];
    assert_eq!(summary["llm_calls"], 0);
    assert_eq!(summary["tokens_in"], 0);
    assert_eq!(summary["tokens_out"], 0);
    assert_eq!(summary["cost_usd"], 0.0);
    assert_eq!(summary["warden_events"], 0);
    assert!(
        summary["confidence"].is_null(),
        "no calls → no confidence spread: {doc}"
    );
}

#[test]
fn run_json_summary_bills_skill_llm_calls() {
    // A skill's LLM call must be counted AND billed (#479 review): the skill
    // bridge used to emit `llm_response` without touching the cost tracker, so
    // this run read `1 LLM call · 0 tok · $0.000` while the envelope's `cost`
    // was 0.0 too.
    let dir = temp_dir("skill-cost");
    let config = dir.join("forge.config.toml");
    std::fs::write(
        &config,
        "[llm]\n\
         default = \"mock\"\n\n\
         [providers.mock]\n\
         type = \"mock\"\n\n\
         [providers.mock.capabilities]\n\
         cost_per_1k_input = 1.0\n\
         cost_per_1k_output = 1.0\n\n\
         [skills]\n\
         skill_dirs = [\"tests/fixtures/sentinel/skills\"]\n",
    )
    .expect("write priced skill config");
    let config = config.to_string_lossy().to_string();

    let output = forge_env(
        &["run", "--json", "tests/fixtures/skill_llm.forge"],
        &[("FORGE_CONFIG", &config)],
    );
    assert_exit(&output, 0);
    let doc = parse_stdout(&output);
    let summary = &doc["data"]["summary"];
    assert!(
        summary["llm_calls"].as_u64().unwrap() >= 1,
        "the skill's LLM call must be counted: {doc}"
    );
    assert!(
        summary["tokens_in"].as_u64().unwrap() > 0,
        "the skill's input tokens must be billed: {doc}"
    );
    assert!(
        summary["tokens_out"].as_u64().unwrap() > 0,
        "the skill's output tokens must be billed: {doc}"
    );
    let cost = summary["cost_usd"].as_f64().expect("skill cost");
    assert!(cost > 0.0, "the skill's cost must be billed: {doc}");
    assert_eq!(
        cost,
        doc["cost"].as_f64().unwrap(),
        "data.summary.cost_usd must match the envelope cost: {doc}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn trace_json_carries_the_summary_too() {
    let output = forge_env(
        &["trace", "--json", "examples/basics/hello.forge"],
        &[("FORGE_MOCK", "1")],
    );
    assert_exit(&output, 0);
    let doc = parse_stdout(&output);
    assert_eq!(doc["command"], "trace");
    assert_eq!(doc["data"]["summary"]["llm_calls"], 0);
}

#[test]
fn run_human_mode_prints_one_summary_line_on_stderr() {
    // The summary goes to stderr so it never mixes with the program's output;
    // `say` stays the only thing on stdout.
    let output = forge_env(
        &["run", "examples/basics/hello.forge"],
        &[("FORGE_MOCK", "1")],
    );
    assert_exit(&output, 0);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "Hello, World!\n");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(
        lines.len(),
        1,
        "exactly one summary line and no trace events: {stderr:?}"
    );
    assert!(
        lines[0].starts_with("0 LLM calls · 0 tok · $0.000 · "),
        "zero-LLM programs show zero calls and cost: {stderr:?}"
    );
    assert!(
        lines[0].ends_with('s'),
        "the line ends in a duration: {stderr:?}"
    );
}

#[test]
fn run_quiet_log_level_suppresses_the_human_summary() {
    let output = forge_env(
        &["run", "examples/basics/hello.forge"],
        &[("FORGE_MOCK", "1"), ("FORGE_LOG_LEVEL", "quiet")],
    );
    assert_exit(&output, 0);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("LLM call"),
        "FORGE_LOG_LEVEL=quiet must silence the summary: {stderr:?}"
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "Hello, World!\n");
}

// ── config errors (#509) ────────────────────────────────────────

/// Write a config whose `[llm.routing]` table is exactly `routing_body` and
/// return its path, so a routing failure survives all the way to the CLI.
fn routing_config(label: &str, routing_body: &str) -> String {
    let dir = temp_dir(label);
    let path = dir.join("forge.config.toml");
    std::fs::write(
        &path,
        format!(
            "[llm]\ndefault = \"mock\"\n\n[llm.routing]\n{routing_body}\n\n[providers.mock]\ntype = \"mock\"\n"
        ),
    )
    .expect("write config with an invalid [llm.routing]");
    path.to_string_lossy().to_string()
}

/// Run a real program under a broken `[llm.routing]` and return the envelope,
/// asserting the process-level contract: exit 1 and `error.type == "config"`.
fn config_error_doc(label: &str, routing_body: &str) -> serde_json::Value {
    let config = routing_config(label, routing_body);
    let output = forge_env(
        &["run", "--json", "tests/fixtures/replay_llm.forge"],
        &[("FORGE_CONFIG", &config)],
    );
    assert_exit(&output, 1);
    let doc = parse_stdout(&output);
    assert_eq!(doc["status"], "error");
    assert_eq!(doc["error"]["type"], "config", "{doc}");
    doc
}

#[test]
fn run_json_types_wrong_routing_value_as_config() {
    let doc = config_error_doc("routing-type", "plan = 7");
    let message = doc["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("llm.routing"), "{message}");
    assert!(message.contains("phase 'plan'"), "{message}");
    assert!(
        message.contains("must be a provider name or a list of provider names (got integer)"),
        "{message}"
    );
}

#[test]
fn run_json_types_unknown_routing_provider_as_config() {
    let doc = config_error_doc("routing-unknown", "plan = [\"mock\", \"ghost\"]");
    let message = doc["error"]["message"].as_str().unwrap_or_default();
    // A config typo, not an outage: the old text was "provider … is unavailable".
    assert!(message.contains("invalid [llm] configuration"), "{message}");
    assert!(!message.contains("is unavailable"), "{message}");
    assert!(message.contains("phase 'plan'"), "{message}");
    assert!(message.contains("unknown provider 'ghost'"), "{message}");
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

// ── run --dry-run (#477) ────────────────────────────────────────

/// One `reason` operation and one `command` effect, checking clean.
const DRY_RUN_PROGRAM: &str = r#"task plan
  gives Text
  do
    draft = reason "plan the work"
    when draft.sure -> give draft
    else -> give "uncertain"

task run_it
  gives Text
  do
    result = command "echo hi"
    if result.success
      give result.stdout
    else
      give "command failed"

fn main
  say plan()
  say run_it()
"#;

/// The same shape, but the command writes `marker` — so a dry run that
/// executed it can be detected.
fn touch_program(marker: &std::path::Path) -> String {
    format!(
        r#"task run_it
  gives Text
  do
    result = command "touch {}"
    if result.success
      give result.stdout
    else
      give "command failed"

fn main
  say run_it()
"#,
        template_escape(marker)
    )
}

/// A path as a FORGE template string: Windows separators are backslashes, and
/// `\\`, `\"`, `\{`, `\}` are the only escapes the parser accepts in a template.
fn template_escape(path: &std::path::Path) -> String {
    path.display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('{', "\\{")
        .replace('}', "\\}")
}

/// A config whose only provider points at a closed port: a dry run that built
/// a provider (or called one) would fail instead of exiting 0.
fn unreachable_provider_config(dir: &std::path::Path) -> String {
    let path = dir.join("forge.config.toml");
    std::fs::write(
        &path,
        "[llm]\n\
         default = \"unreachable\"\n\n\
         [providers.unreachable]\n\
         type = \"openai-compat\"\n\
         model = \"test-model\"\n\
         api_key = \"test-key\"\n\
         base_url = \"http://127.0.0.1:9\"\n",
    )
    .expect("write unreachable provider config");
    path.to_string_lossy().to_string()
}

#[test]
fn run_dry_run_json_lists_operations_and_effects() {
    let dir = temp_dir("dry-run-json");
    let program = dir.join("plan.forge");
    std::fs::write(&program, DRY_RUN_PROGRAM).expect("write dry-run program");

    let output = forge(&["run", "--dry-run", "--json", program.to_str().unwrap()]);
    assert_exit(&output, 0);
    let doc = parse_stdout(&output);
    assert_eq!(doc["command"], "run");
    assert_eq!(doc["data"]["dry_run"], true);
    let operations = doc["data"]["operations"].as_array().unwrap();
    assert_eq!(operations.len(), 1, "one LLM operation expected: {doc}");
    assert_eq!(operations[0]["kind"], "reason");
    let effects = doc["data"]["effects"].as_array().unwrap();
    assert_eq!(effects.len(), 1, "one effect expected: {doc}");
    assert_eq!(effects[0]["kind"], "command");
    assert_eq!(effects[0]["location"], "task run_it");
    // A dry run executes nothing, so the envelope's `cost` (USD spent) is 0.0
    // and the plan estimate stays in `data` (#528).
    assert_eq!(doc["cost"], 0.0);
    assert!(doc["data"]["estimated_cost_usd"].as_f64().unwrap() > 0.0);
}

#[test]
fn dry_run_and_cost_envelopes_report_zero_spend() {
    let dir = temp_dir("dry-run-zero-cost");
    let program = dir.join("plan.forge");
    std::fs::write(&program, DRY_RUN_PROGRAM).expect("write dry-run program");

    let cases: [&[&str]; 2] = [&["run", "--dry-run", "--json"], &["cost", "--json"]];
    for args in cases {
        let mut argv = args.to_vec();
        argv.push(program.to_str().unwrap());
        let output = forge(&argv);
        assert_exit(&output, 0);
        let doc = parse_stdout(&output);
        assert_eq!(
            doc["cost"], 0.0,
            "`forge {}` spends nothing, so `cost` must be 0.0: {doc}",
            args[0]
        );
        assert!(
            doc["data"]["estimated_cost_usd"].as_f64().unwrap() > 0.0,
            "the estimate must stay in `data.estimated_cost_usd`: {doc}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_dry_run_never_builds_a_provider() {
    let dir = temp_dir("dry-run-provider");
    let program = dir.join("plan.forge");
    std::fs::write(&program, DRY_RUN_PROGRAM).expect("write dry-run program");
    let config = unreachable_provider_config(&dir);

    let output = forge_env(
        &["run", "--dry-run", program.to_str().unwrap()],
        &[("FORGE_CONFIG", &config)],
    );
    assert_exit(&output, 0);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Effects:"),
        "the plan must be printed: {stdout}"
    );
    assert!(
        stdout.contains("nothing was executed"),
        "the plan must say nothing ran: {stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("falling back"),
        "the unreachable provider config must load, not fall back to mock: {stderr}"
    );
}

#[test]
fn run_dry_run_never_constructs_a_provider() {
    let dir = temp_dir("dry-run-construction");
    let program = dir.join("plan.forge");
    std::fs::write(&program, DRY_RUN_PROGRAM).expect("write dry-run program");
    // A provider type the factory rejects: if the dry-run path ever built the
    // registry — the closest thing to the issue's "provider that panics" — this
    // command would fail instead of printing the plan.
    let config = dir.join("forge.config.toml");
    std::fs::write(
        &config,
        "[llm]\n\
         default = \"unbuildable\"\n\n\
         [providers.unbuildable]\n\
         type = \"no-such-provider-type\"\n",
    )
    .expect("write unbuildable provider config");

    let output = forge_env(
        &["run", "--dry-run", program.to_str().unwrap()],
        &[("FORGE_CONFIG", config.to_str().unwrap())],
    );
    assert_exit(&output, 0);
    assert!(String::from_utf8_lossy(&output.stdout).contains("nothing was executed"));
}

#[test]
fn run_dry_run_spawns_no_process() {
    let dir = temp_dir("dry-run-touch");
    let marker = dir.join("ran");
    let program = dir.join("touch.forge");
    std::fs::write(&program, touch_program(&marker)).expect("write touch program");

    let output = forge(&["run", "--dry-run", program.to_str().unwrap()]);
    assert_exit(&output, 0);
    assert!(
        !marker.exists(),
        "a dry run must not execute the command: {}",
        marker.display()
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("nothing was executed"));
}

/// Warnings only (W080: a spawned agent with no failure policy), so the plan is
/// still printed — at `forge check`'s warnings-only exit code 2.
const WARNINGS_PROGRAM: &str = r#"agent specialist
  on start
    say "specialist up"

agent foreman
  on dispatch(topic: Text)
    child = spawn specialist as "spec_{topic}"
    say "spawned {child}"

task plan
  gives Text
  do
    draft = reason "plan the work"
    when draft.sure -> give draft
    else -> give "uncertain"

fn main
  say plan()
"#;

#[test]
fn run_dry_run_json_prints_the_plan_at_warnings_only() {
    let dir = temp_dir("dry-run-warning");
    let program = dir.join("warn.forge");
    std::fs::write(&program, WARNINGS_PROGRAM).expect("write warnings program");

    let output = forge(&["run", "--dry-run", "--json", program.to_str().unwrap()]);
    assert_exit(&output, 2);
    let doc = parse_stdout(&output);
    assert_eq!(doc["status"], "warning");
    assert_eq!(doc["data"]["dry_run"], true);
    assert_eq!(doc["data"]["operations"][0]["kind"], "reason");
    assert_eq!(doc["data"]["effects"][0]["kind"], "spawn");
    assert!(
        doc["warnings"][0].as_str().unwrap().starts_with("W080"),
        "the W080 warning must be listed in the envelope: {doc}"
    );
}

#[test]
fn cost_json_keeps_every_field_and_adds_effects() {
    let output = forge(&["cost", "--json", "examples/errors/uncertain_error.forge"]);
    assert_exit(&output, 0);
    let doc = parse_stdout(&output);
    assert_eq!(doc["command"], "cost");
    let data = doc["data"].as_object().expect("cost data is an object");
    for key in [
        "file",
        "operations",
        "effects",
        "total_tokens_in",
        "total_tokens_out",
        "estimated_cost_usd",
    ] {
        assert!(data.contains_key(key), "cost --json lost `{key}`: {doc}");
    }
    let operation = doc["data"]["operations"][0]
        .as_object()
        .expect("cost operation is an object");
    for key in [
        "kind",
        "location",
        "estimated_tokens_in",
        "estimated_tokens_out",
        "estimated_cost_usd",
    ] {
        assert!(
            operation.contains_key(key),
            "cost operation lost `{key}`: {doc}"
        );
    }
    assert!(
        data.get("dry_run").is_none(),
        "`cost` is not a dry run: {doc}"
    );
}

#[test]
fn run_dry_run_blocks_on_errors_and_refuses_manifest() {
    let blocked = forge(&[
        "run",
        "--dry-run",
        "--json",
        "examples/errors/uncertain_error.forge",
    ]);
    assert_exit(&blocked, 1);
    let doc = parse_stdout(&blocked);
    assert_eq!(doc["status"], "error");
    assert_eq!(doc["error"]["type"], "check");
    assert_eq!(doc["data"]["dry_run"], true);

    let manifest = forge(&["run", "--dry-run", "--manifest", "forge.project.toml"]);
    assert_exit(&manifest, 1);
    assert!(
        String::from_utf8_lossy(&manifest.stderr)
            .contains("--manifest is not supported with --dry-run"),
        "stderr: {}",
        String::from_utf8_lossy(&manifest.stderr)
    );

    // --record needs a provider, so the command line itself is invalid.
    let record = forge(&[
        "run",
        "--dry-run",
        "examples/basics/hello.forge",
        "--record",
    ]);
    assert_exit(&record, 1);
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

// ── #485 — surfaces removed with the web app stack ──────────────

#[test]
fn serve_watch_reports_the_v0_3_removal() {
    let human = forge(&["serve", "--watch", "examples/basics/hello.forge"]);
    assert_exit(&human, 1);
    let stderr = String::from_utf8_lossy(&human.stderr);
    assert!(
        stderr.contains(
            "`forge serve --watch` was removed in v0.3: hot reload belongs to the app, not the language (see CHANGELOG, #485)"
        ),
        "stderr: {stderr}"
    );

    // `--json` mode gets the usual usage envelope rather than clap's stderr text.
    let json = forge(&["serve", "--json", "--watch", "examples/basics/hello.forge"]);
    assert_exit(&json, 1);
    let doc = parse_stdout(&json);
    assert_eq!(doc["command"], "serve");
    assert_eq!(doc["error"]["type"], "usage");
    assert!(
        doc["error"]["message"].as_str().unwrap().contains(
            "`forge serve --watch` was removed in v0.3: hot reload belongs to the app, not the language (see CHANGELOG, #485)"
        ),
        "{doc}"
    );
}

#[test]
fn config_with_server_static_reports_the_v0_3_removal() {
    let dir = temp_dir("server-static");
    let config = dir.join("forge.config.toml");
    std::fs::write(
        &config,
        "[llm]\n\
         default = \"mock\"\n\n\
         [providers.mock]\n\
         type = \"mock\"\n\n\
         [server.static]\n\
         root = \"public\"\n",
    )
    .expect("write config");

    let output = forge_env(
        &["check", "--json", "examples/basics/hello.forge"],
        &[("FORGE_CONFIG", config.to_str().unwrap())],
    );
    assert_exit(&output, 1);
    let doc = parse_stdout(&output);
    assert_eq!(doc["status"], "error");
    assert_eq!(
        doc["error"]["message"],
        "`[server.static]` was removed in v0.3: serve static files from the app or a web server (see CHANGELOG, #485)"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn asset_call_is_an_e160_with_the_removal_help() {
    let dir = temp_dir("asset-call");
    let program = dir.join("asset.forge");
    std::fs::write(&program, "fn main\n  say asset(\"logo.png\")\n").expect("write program");

    let output = forge(&["check", "--json", program.to_str().unwrap()]);
    assert_exit(&output, 1);
    let doc = parse_stdout(&output);
    let diagnostic = &doc["data"]["diagnostics"][0];
    assert_eq!(diagnostic["code"], "E160");
    assert_eq!(
        diagnostic["help"],
        "`asset()` was removed in v0.3 with static file serving (see CHANGELOG, #485)"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
