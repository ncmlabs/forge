// FORGE AST node definitions
// See issue #3 for full implementation

/// Top-level program: a list of declarations
#[derive(Debug, Clone)]
pub struct Program {
    pub items: Vec<TopLevel>,
}

/// A top-level declaration
#[derive(Debug, Clone)]
pub enum TopLevel {
    // Variants will be added in issue #3
}
