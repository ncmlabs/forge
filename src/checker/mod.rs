// FORGE checker coordinator
// See issues #16 (pure), #17 (states), #18 (requires), #21 (boundary), #24 (warden), #26 (uncertain)

pub mod boundary_checker;
pub mod correlate_checker;
pub mod pure_checker;
pub mod requires_checker;
pub mod schedule_checker;
pub mod spawn_checker;
pub mod states_checker;
pub mod uncertain_checker;
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

    // Pass 4: uncertain value enforcement (Principle I — Honesty)
    diagnostics.extend(uncertain_checker::check(program, file));

    // Pass 5: warden policy enforcement
    diagnostics.extend(warden_checker::check(program, file));

    // Pass 6: spawn statement validation (Principle VII — Accountability)
    diagnostics.extend(spawn_checker::check(program, file));

    // Pass 7: schedule block validation (Principle I/III — honesty + token economy)
    diagnostics.extend(schedule_checker::check(program, file));

    // Pass 8: correlate block validation (Principle I/II — honesty + determinism)
    diagnostics.extend(correlate_checker::check(program, file));

    // boundary_checker::check() is called separately from main.rs (multi-program signature)
    diagnostics
}
