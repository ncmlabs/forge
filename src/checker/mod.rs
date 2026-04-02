// FORGE checker coordinator
// See issue #16 (pure checker), #17 (states checker)

pub mod pure_checker;

use crate::ast::Program;
use crate::diagnostic::Diagnostic;

pub fn check_all(program: &Program, file: &str) -> Vec<Diagnostic> {
    let mut diagnostics = vec![];

    // Pass 1: pure enforcement
    let pure_errors = pure_checker::check(program);
    diagnostics.extend(pure_errors.iter().map(|e| e.to_diagnostic(file)));

    // future: diagnostics.extend(states_checker::check(program, file));
    // future: diagnostics.extend(boundary_checker::check(program, file));
    diagnostics
}
