// FORGE parser: pest grammar → parse validation
// AST construction will be added in issue #3/#4

use pest::Parser;
use pest_derive::Parser;

use crate::ast::Program;

#[derive(Parser)]
#[grammar = "grammar/forge.pest"]
pub struct ForgeParser;

pub fn parse(source: &str) -> anyhow::Result<Program> {
    ForgeParser::parse(Rule::program, source)
        .map_err(|e| anyhow::anyhow!("parse error:\n{}", e))?;

    // Grammar validated; AST construction is issue #3/#4
    Ok(Program { items: vec![] })
}
