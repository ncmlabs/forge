// FORGE checker coordinator
// See issue #16 (pure checker), #17 (states checker)

pub mod boundary_checker;
pub mod pure_checker;
pub mod requires_checker;
pub mod states_checker;

use crate::ast::Program;
use crate::diagnostic::Diagnostic;

pub fn check_all(program: &Program, file: &str) -> Vec<Diagnostic> {
    let mut diagnostics = vec![];

    // Pass 1: pure enforcement
    let pure_errors = pure_checker::check(program);
    diagnostics.extend(pure_errors.iter().map(|e| e.to_diagnostic(file)));

    // Pass 2: states / lifecycle enforcement
    diagnostics.extend(states_checker::check(program, file));

    // Pass 3: requires clause enforcement
    diagnostics.extend(requires_checker::check(program, file));

    // future: diagnostics.extend(boundary_checker::check(program, file));
    diagnostics
}
