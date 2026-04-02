// FORGE boundary checker
// See issue #21 for full specification

use crate::ast::{BoundaryKind, Program, TopLevel};
use crate::diagnostic::Diagnostic;

// ── Public API ─────────────────────────────────────────────

pub fn check(programs: &[(&Program, &str)]) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // Phase 1: per-file validation
    for (program, file) in programs {
        let boundary = effective_boundary(program);
        check_endpoint_placement(program, boundary, file, &mut diagnostics);
    }

    diagnostics
}

// ── Helpers ────────────────────────────────────────────────

/// Determine the effective boundary for a program.
/// Files without a directive default to Shared.
fn effective_boundary(program: &Program) -> BoundaryKind {
    program
        .boundary
        .as_ref()
        .map(|b| b.node.kind.node)
        .unwrap_or(BoundaryKind::Shared)
}

// ── Phase 1: Per-file checks ──────────────────────────────

fn check_endpoint_placement(
    program: &Program,
    boundary: BoundaryKind,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if boundary == BoundaryKind::Server {
        return; // endpoints are legal in server boundary
    }

    for item in &program.items {
        if let TopLevel::Endpoint(ep) = &item.node {
            let boundary_name = match boundary {
                BoundaryKind::Client => "client",
                BoundaryKind::Shared => "shared",
                BoundaryKind::Server => unreachable!(),
            };
            diagnostics.push(
                Diagnostic::error(
                    file,
                    format!(
                        "endpoint `{}` is not allowed in {} boundary",
                        ep.name.node, boundary_name
                    ),
                    ep.name.span.start..ep.name.span.end,
                    "endpoints can only be declared in `server` boundary files",
                )
                .with_help("move this endpoint to a file with `#! boundary: server`"),
            );
        }
    }
}
