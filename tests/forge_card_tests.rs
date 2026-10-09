// FORGE card conformance — issue #480.
// Every fenced `forge` block in docs/forge-card.md must parse and check clean
// (the same path `forge check` takes), and every `forge-error` block must fail.
// The card's promise is that an agent reading only the card is never taught code
// the compiler rejects, so the card is only as good as this test.

use std::path::PathBuf;

use forge::checker;
use forge::checker::boundary_checker;
use forge::diagnostic::{Diagnostic, DiagnosticKind};
use forge::resolver::{CapabilityRegistry, CheckContext};

const CARD: &str = "docs/forge-card.md";
const MAX_CARD_LINES: usize = 300;

/// A fenced block: its info string, the 1-based line where its body starts, and the body.
struct Snippet {
    info: String,
    line: usize,
    source: String,
}

fn card_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(CARD);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()))
}

/// Split markdown into fenced blocks, tagging each with the fence info string.
fn snippets(markdown: &str) -> Vec<Snippet> {
    let mut blocks = Vec::new();
    let mut open: Option<Snippet> = None;

    for (index, line) in markdown.lines().enumerate() {
        if let Some(info) = line.strip_prefix("```") {
            match open.take() {
                Some(done) => blocks.push(done),
                None => {
                    open = Some(Snippet {
                        info: info.trim().to_string(),
                        line: index + 2,
                        source: String::new(),
                    })
                }
            }
            continue;
        }
        if let Some(block) = open.as_mut() {
            block.source.push_str(line);
            block.source.push('\n');
        }
    }

    assert!(open.is_none(), "unclosed fenced block in {CARD}");
    blocks
}

/// The parse + resolve + check path of `check_single` in
/// `tests/conformance_runner.rs` — what a single-file `forge check` runs.
/// The parse failure is returned as text instead of panicking so that
/// intentionally broken (`forge-error`) blocks can be asserted on too.
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

fn blocks_with_info(info: &str) -> Vec<Snippet> {
    snippets(&card_source())
        .into_iter()
        .filter(|block| block.info == info)
        .collect()
}

#[test]
fn card_is_at_most_300_lines() {
    let lines = card_source().lines().count();
    assert!(
        lines <= MAX_CARD_LINES,
        "{CARD} is {lines} lines; the card must stay at or under {MAX_CARD_LINES}"
    );
}

#[test]
fn every_forge_snippet_checks_clean() {
    let checked = blocks_with_info("forge");
    // Guard against the extractor silently matching nothing after a card rewrite.
    assert!(
        checked.len() >= 10,
        "expected at least 10 `forge` snippets in {CARD}, found {}",
        checked.len()
    );

    let mut failures = Vec::new();
    for (index, block) in checked.iter().enumerate() {
        let filename = format!("{CARD}#forge-{}", index + 1);
        match check_single(&block.source, &filename) {
            Err(parse_error) => failures.push(format!(
                "{CARD}:{} (forge-{}): parse failed: {parse_error}",
                block.line,
                index + 1
            )),
            // `forge check` exits non-zero on warnings too, so the card stays
            // diagnostic-free, not merely error-free.
            Ok(diags) if !diags.is_empty() => failures.push(format!(
                "{CARD}:{} (forge-{}): {} diagnostic(s): {}",
                block.line,
                index + 1,
                diags.len(),
                diags
                    .iter()
                    .map(|d| d.message.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            )),
            Ok(_) => {}
        }
    }

    assert!(
        failures.is_empty(),
        "every `forge` snippet in the card must check clean:\n{}",
        failures.join("\n")
    );
}

#[test]
fn every_forge_error_snippet_fails() {
    let expected = blocks_with_info("forge-error");
    assert!(
        expected.len() >= 2,
        "expected at least 2 `forge-error` snippets in {CARD}, found {}",
        expected.len()
    );

    let mut failures = Vec::new();
    for (index, block) in expected.iter().enumerate() {
        let filename = format!("{CARD}#forge-error-{}", index + 1);
        match check_single(&block.source, &filename) {
            // A parse failure is a legitimate way for a syntax-trap block to fail.
            Err(_) => {}
            Ok(diags) if !diags.iter().any(|d| d.kind == DiagnosticKind::Error) => {
                failures.push(format!(
                    "{CARD}:{} (forge-error-{}): expected a checker error, found {} diagnostic(s)",
                    block.line,
                    index + 1,
                    diags.len()
                ))
            }
            Ok(_) => {}
        }
    }

    assert!(
        failures.is_empty(),
        "every `forge-error` snippet in the card must fail:\n{}",
        failures.join("\n")
    );
}
