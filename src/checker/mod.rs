// FORGE checker coordinator
// See issues #16 (pure), #17 (states), #18 (requires), #21 (boundary), #24 (warden)

pub mod boundary_checker;
pub mod pure_checker;
pub mod requires_checker;
pub mod states_checker;
pub mod warden_checker;

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

    // Pass 4: warden policy enforcement
    diagnostics.extend(warden_checker::check(program, file));

    // boundary_checker::check() is called separately from main.rs (multi-program signature)
    diagnostics
}
