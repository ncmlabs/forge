// FORGE checker coordinator
// See issue #16 for the first pass (pure checker)

pub mod pure_checker;

pub use pure_checker::CheckError;

use crate::ast::Program;

pub fn check_all(program: &Program) -> Vec<CheckError> {
    let mut errors = vec![];
    errors.extend(pure_checker::check(program));
    // future: errors.extend(states_checker::check(program));
    // future: errors.extend(boundary_checker::check(program));
    errors
}
