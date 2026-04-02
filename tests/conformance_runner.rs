// FORGE conformance suite runner — issue #27
// Reads JSON test files from conformance/ and validates them against the compiler and runtime.

use std::path::PathBuf;
use std::sync::Arc;

use forge::checker;
use forge::checker::boundary_checker;
use forge::diagnostic::{Diagnostic, DiagnosticKind};
use forge::llm::providers::mock::MockProvider;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::executor::TaskExecutor;
use forge::tracer::Tracer;

// ── Test data types ─────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct ConformanceTest {
    name: String,
    #[allow(dead_code)]
    category: String,
    #[allow(dead_code)]
    subcategory: Option<String>,
    #[allow(dead_code)]
    description: String,
    input: InputKind,
    expected: Expected,
    mock_responses: Option<Vec<MockResponse>>,
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum InputKind {
    Multi(Vec<FileInput>),
    Single(String),
}

#[derive(serde::Deserialize)]
struct FileInput {
    file: String,
    source: String,
}

#[derive(serde::Deserialize)]
struct Expected {
    outcome: String,
    error_contains: Option<Vec<String>>,
    error_kind: Option<String>,
    trace_shape: Option<Vec<String>>,
}

#[derive(serde::Deserialize)]
struct MockResponse {
    text: String,
    #[allow(dead_code)]
    confidence: Option<f64>,
}

// ── Helpers ─────────────────────────────────────────────────────────

fn discover_tests() -> Vec<PathBuf> {
    let conformance_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("conformance");
    let mut tests = Vec::new();
    collect_json_files(&conformance_dir, &mut tests);
    tests.sort();
    tests
}

fn collect_json_files(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_json_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "json") {
                // Skip the schema file
                if path.file_name().is_some_and(|n| n != "schema.json") {
                    out.push(path);
                }
            }
        }
    }
}

fn mock_registry(mock: MockProvider) -> Arc<ProviderRegistry> {
    let mut reg = ProviderRegistry::new("mock");
    reg.register("mock", Arc::new(mock));
    Arc::new(reg)
}

fn check_single(source: &str, filename: &str) -> Vec<Diagnostic> {
    let program = forge::parser::parse(source).expect("parse should succeed for checker tests");
    let mut diags = checker::check_all(&program, filename);
    diags.extend(boundary_checker::check(&[(&program, filename)]));
    diags
}

fn check_multi(files: &[(String, String)]) -> Vec<Diagnostic> {
    let programs: Vec<_> = files
        .iter()
        .map(|(filename, source)| {
            let program = forge::parser::parse(source)
                .unwrap_or_else(|e| panic!("parse failed for {}: {:?}", filename, e));
            (program, filename.clone())
        })
        .collect();

    let mut diags = Vec::new();
    for (program, filename) in &programs {
        diags.extend(checker::check_all(program, filename));
    }
    let refs: Vec<_> = programs.iter().map(|(p, f)| (p, f.as_str())).collect();
    diags.extend(boundary_checker::check(&refs));
    diags
}

fn matches_error(diag: &Diagnostic, substrings: &[String], expected_kind: &str) -> bool {
    let kind_matches = match expected_kind {
        "warning" => matches!(diag.kind, DiagnosticKind::Warning),
        _ => matches!(diag.kind, DiagnosticKind::Error),
    };
    kind_matches && substrings.iter().all(|s| diag.message.contains(s))
}

/// Check that `expected` is an ordered subsequence of `actual`.
fn is_subsequence(expected: &[String], actual: &[String]) -> bool {
    let mut ei = 0;
    for event in actual {
        if ei < expected.len() && event == &expected[ei] {
            ei += 1;
        }
    }
    ei == expected.len()
}

// ── Main test ───────────────────────────────────────────────────────

#[tokio::test]
async fn conformance_suite() {
    let test_files = discover_tests();
    assert!(
        !test_files.is_empty(),
        "no conformance test files found under conformance/"
    );

    let mut passed = 0;
    let mut failed = 0;
    let mut failures = Vec::new();

    for path in &test_files {
        let content = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("could not read {}: {}", path.display(), e));
        let test: ConformanceTest = serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("invalid JSON in {}: {}", path.display(), e));

        let result = run_single_test(&test).await;
        match result {
            Ok(()) => {
                passed += 1;
            }
            Err(msg) => {
                failed += 1;
                failures.push(format!(
                    "FAIL [{}] ({}): {}",
                    test.name,
                    path.display(),
                    msg
                ));
            }
        }
    }

    if !failures.is_empty() {
        for f in &failures {
            eprintln!("{}", f);
        }
        panic!(
            "conformance suite: {} passed, {} failed out of {} tests",
            passed,
            failed,
            passed + failed
        );
    }

    eprintln!("conformance suite: all {} tests passed", passed);
}

async fn run_single_test(test: &ConformanceTest) -> Result<(), String> {
    match test.expected.outcome.as_str() {
        "parse_ok" => test_parse_ok(test),
        "parse_error" => test_parse_error(test),
        "compile_ok" => test_compile_ok(test),
        "compile_error" => test_compile_error(test),
        "run_ok" => test_run_ok(test).await,
        "run_error" => test_run_error(test).await,
        other => Err(format!("unknown outcome: {}", other)),
    }
}

// ── Outcome handlers ────────────────────────────────────────────────

fn test_parse_ok(test: &ConformanceTest) -> Result<(), String> {
    let source = single_source(&test.input)?;
    forge::parser::parse(&source)
        .map(|_| ())
        .map_err(|e| format!("expected parse_ok but got error: {:?}", e))
}

fn test_parse_error(test: &ConformanceTest) -> Result<(), String> {
    let source = single_source(&test.input)?;
    match forge::parser::parse(&source) {
        Ok(_) => Err("expected parse_error but parse succeeded".to_string()),
        Err(e) => {
            let err_msg = format!("{:?}", e);
            check_error_contains(&test.expected, &err_msg)
        }
    }
}

fn test_compile_ok(test: &ConformanceTest) -> Result<(), String> {
    let diags = run_checkers(&test.input)?;
    let errors: Vec<_> = diags
        .iter()
        .filter(|d| matches!(d.kind, DiagnosticKind::Error))
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "expected compile_ok but got {} errors: {:?}",
            errors.len(),
            errors.iter().map(|d| &d.message).collect::<Vec<_>>()
        ))
    }
}

fn test_compile_error(test: &ConformanceTest) -> Result<(), String> {
    let diags = run_checkers(&test.input)?;
    let error_contains = test.expected.error_contains.as_deref().unwrap_or(&[]);
    let expected_kind = test.expected.error_kind.as_deref().unwrap_or("error");

    let has_match = diags
        .iter()
        .any(|d| matches_error(d, error_contains, expected_kind));
    if has_match {
        Ok(())
    } else {
        Err(format!(
            "expected compile_error with {:?} (kind={}) but got diagnostics: {:?}",
            error_contains,
            expected_kind,
            diags
                .iter()
                .map(|d| format!("[{:?}] {}", d.kind, d.message))
                .collect::<Vec<_>>()
        ))
    }
}

async fn test_run_ok(test: &ConformanceTest) -> Result<(), String> {
    let source = single_source(&test.input)?;
    let program = forge::parser::parse(&source).map_err(|e| format!("parse failed: {:?}", e))?;

    let mock = build_mock(test);
    let tracer = Tracer::with_capture();
    let executor = TaskExecutor::new(program, mock_registry(mock), Some(tracer.clone()));

    executor
        .run()
        .await
        .map_err(|e| format!("expected run_ok but got runtime error: {:?}", e))?;

    if let Some(ref expected_shape) = test.expected.trace_shape {
        let actual = tracer.captured_events();
        if !is_subsequence(expected_shape, &actual) {
            return Err(format!(
                "trace shape mismatch:\n  expected subsequence: {:?}\n  actual events: {:?}",
                expected_shape, actual
            ));
        }
    }

    Ok(())
}

async fn test_run_error(test: &ConformanceTest) -> Result<(), String> {
    let source = single_source(&test.input)?;
    let program = forge::parser::parse(&source).map_err(|e| format!("parse failed: {:?}", e))?;

    let mock = build_mock(test);
    let tracer = Tracer::with_capture();
    let executor = TaskExecutor::new(program, mock_registry(mock), Some(tracer.clone()));

    match executor.run().await {
        Ok(_) => Err("expected run_error but execution succeeded".to_string()),
        Err(e) => {
            let err_msg = format!("{:?}", e);
            check_error_contains(&test.expected, &err_msg)
        }
    }
}

// ── Utility functions ───────────────────────────────────────────────

fn single_source(input: &InputKind) -> Result<String, String> {
    match input {
        InputKind::Single(s) => Ok(s.clone()),
        InputKind::Multi(_) => Err("expected single-file input but got multi-file".to_string()),
    }
}

fn run_checkers(input: &InputKind) -> Result<Vec<Diagnostic>, String> {
    match input {
        InputKind::Single(source) => Ok(check_single(source, "test.forge")),
        InputKind::Multi(files) => {
            let pairs: Vec<_> = files
                .iter()
                .map(|f| (f.file.clone(), f.source.clone()))
                .collect();
            Ok(check_multi(&pairs))
        }
    }
}

fn check_error_contains(expected: &Expected, err_msg: &str) -> Result<(), String> {
    if let Some(ref substrings) = expected.error_contains {
        for s in substrings {
            if !err_msg.contains(s) {
                return Err(format!(
                    "error message does not contain '{}', got: {}",
                    s, err_msg
                ));
            }
        }
    }
    Ok(())
}

fn build_mock(test: &ConformanceTest) -> MockProvider {
    let mock = MockProvider::new("mock");
    match test.mock_responses {
        Some(ref responses) if !responses.is_empty() => {
            let texts: Vec<String> = responses.iter().map(|r| r.text.clone()).collect();
            mock.with_responses_sequence(texts)
        }
        _ => mock.with_default("mock response"),
    }
}
