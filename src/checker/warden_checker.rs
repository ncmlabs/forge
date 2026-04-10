// FORGE warden checker
// Validates warden declarations: managed names exist, enums valid,
// escalation ladder ordering, coverage warnings.
// See issue #24 for specification.

use std::collections::HashSet;

use crate::ast::{FailureType, Program, TopLevel, WardenDecl};
use crate::diagnostic::Diagnostic;

/// Run all warden checks on a single program.
pub fn check(program: &Program, file: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // Collect all declared top-level names for existence checks
    let declared_names: HashSet<String> = program
        .items
        .iter()
        .filter_map(|item| match &item.node {
            TopLevel::Agent(d) => Some(d.name.node.clone()),
            TopLevel::Warden(d) => Some(d.name.node.clone()),
            TopLevel::Flow(d) => Some(d.name.node.clone()),
            TopLevel::Pool(d) => Some(d.name.node.clone()),
            _ => None,
        })
        .collect();

    for item in &program.items {
        if let TopLevel::Warden(warden) = &item.node {
            check_manages_exist(warden, &declared_names, file, &mut diagnostics);
            check_escalation_ladder(warden, file, &mut diagnostics);
            check_failure_type_coverage(warden, file, &mut diagnostics);
        }
    }

    diagnostics
}

/// Check that all names in the `manages` list refer to declared agents or wardens.
fn check_manages_exist(
    warden: &WardenDecl,
    declared: &HashSet<String>,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for managed in &warden.manages {
        if !declared.contains(&managed.node) {
            diagnostics.push(
                Diagnostic::error(
                    file,
                    format!(
                        "warden `{}` manages `{}` which is not declared",
                        warden.name.node, managed.node
                    ),
                    managed.span.start..managed.span.end,
                    "not found in this file",
                )
                .with_help("declare an agent or warden with this name"),
            );
        }
    }
}

/// Check that `after` clauses escalate (response levels must increase).
fn check_escalation_ladder(warden: &WardenDecl, file: &str, diagnostics: &mut Vec<Diagnostic>) {
    for policy in &warden.policies {
        let mut prev_response = policy.node.response.node;
        let mut prev_count = 0u64;

        for after in &policy.node.after_clauses {
            // Count must increase
            if after.node.count <= prev_count {
                diagnostics.push(Diagnostic::error(
                    file,
                    format!(
                        "warden `{}`: `after` count must increase (got {} after {})",
                        warden.name.node, after.node.count, prev_count
                    ),
                    after.span.start..after.span.end,
                    "count must be greater than previous",
                ));
            }

            // Response must escalate (Nudge < Restart < Replace < Escalate)
            if after.node.response.node <= prev_response {
                diagnostics.push(
                    Diagnostic::error(
                        file,
                        format!(
                            "warden `{}`: escalation ladder must increase severity (got {:?} after {:?})",
                            warden.name.node, after.node.response.node, prev_response
                        ),
                        after.node.response.span.start..after.node.response.span.end,
                        "must be more severe than previous response",
                    )
                    .with_help("responses must escalate: nudge → downgrade → restart → replace → escalate"),
                );
            }

            prev_response = after.node.response.node;
            prev_count = after.node.count;
        }
    }
}

/// Warn if a warden doesn't cover all five failure types.
fn check_failure_type_coverage(warden: &WardenDecl, file: &str, diagnostics: &mut Vec<Diagnostic>) {
    let all_types = [
        FailureType::Stuck,
        FailureType::Crash,
        FailureType::Hallucination,
        FailureType::Contradiction,
        FailureType::Budget,
        FailureType::Timeout,
    ];

    let covered: HashSet<FailureType> = warden
        .policies
        .iter()
        .map(|p| p.node.failure_type.node)
        .collect();

    let missing: Vec<&str> = all_types
        .iter()
        .filter(|t| !covered.contains(t))
        .map(|t| match t {
            FailureType::Stuck => "stuck",
            FailureType::Crash => "crash",
            FailureType::Hallucination => "hallucination",
            FailureType::Contradiction => "contradiction",
            FailureType::Budget => "budget",
            FailureType::Timeout => "timeout",
        })
        .collect();

    if !missing.is_empty() {
        diagnostics.push(
            Diagnostic::warning(
                file,
                format!(
                    "warden `{}` does not cover failure types: {}",
                    warden.name.node,
                    missing.join(", ")
                ),
                warden.name.span.start..warden.name.span.end,
                "incomplete failure coverage",
            )
            .with_help("consider adding policies for all failure types"),
        );
    }
}
