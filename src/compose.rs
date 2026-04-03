// FORGE multi-file composition engine
// See issue #74 for specification

use std::collections::HashMap;

use crate::ast::{AgentDecl, Program, Spanned, StatesDecl, TopLevel};

// ── Types ─────────────────────────────────────────────────────

/// A parsed source file ready for composition.
pub struct SourceFile {
    pub path: String,
    pub source: String,
    pub program: Program,
}

/// Result of merging multiple source files into one program.
#[derive(Debug)]
pub struct ComposedProgram {
    pub program: Program,
    /// File path → source text, for diagnostic rendering.
    pub sources: HashMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ComposeError {
    #[error("duplicate symbol `{name}` defined in {file1} and {file2}")]
    DuplicateSymbol {
        name: String,
        file1: String,
        file2: String,
    },

    #[error("multiple `fn main` blocks found: {file1} and {file2}")]
    MultipleFnMain { file1: String, file2: String },

    #[error("multiple `system` declarations found: {file1} and {file2}")]
    MultipleSystem { file1: String, file2: String },
}

// ── Core merge ────────────────────────────────────────────────

/// Merge multiple parsed programs into a single program.
///
/// Rules:
/// - At most one `fn main` across all files
/// - At most one `system` declaration across all files
/// - No duplicate top-level symbol names (task, pure, flow, agent, pool, etc.)
/// - `use` declarations pass through (resolver validates per-file)
/// - Merged program has `boundary: None` (boundary checker runs on pre-merge programs)
pub fn merge_programs(files: &[SourceFile]) -> Result<ComposedProgram, Vec<ComposeError>> {
    let mut errors = Vec::new();
    let mut symbols: HashMap<String, &str> = HashMap::new(); // name → origin file
    let mut fn_main_file: Option<&str> = None;
    let mut system_file: Option<&str> = None;
    let mut all_items: Vec<Spanned<TopLevel>> = Vec::new();
    let mut sources = HashMap::new();

    for file in files {
        sources.insert(file.path.clone(), file.source.clone());

        for item in &file.program.items {
            // Check singletons
            match &item.node {
                TopLevel::FnMain(_) => {
                    if let Some(prev) = fn_main_file {
                        errors.push(ComposeError::MultipleFnMain {
                            file1: prev.to_string(),
                            file2: file.path.clone(),
                        });
                    } else {
                        fn_main_file = Some(&file.path);
                    }
                }
                TopLevel::System(_) => {
                    if let Some(prev) = system_file {
                        errors.push(ComposeError::MultipleSystem {
                            file1: prev.to_string(),
                            file2: file.path.clone(),
                        });
                    } else {
                        system_file = Some(&file.path);
                    }
                }
                _ => {}
            }

            // Check named symbols for duplicates (skip Use, FnMain, Import)
            if let Some(name) = top_level_name(&item.node) {
                check_symbol(name, &file.path, &mut symbols, &mut errors);
            }

            all_items.push(item.clone());
        }
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    let program = Program {
        boundary: None,
        items: all_items,
    };

    Ok(ComposedProgram { program, sources })
}

fn check_symbol<'a>(
    name: &str,
    file: &'a str,
    symbols: &mut HashMap<String, &'a str>,
    errors: &mut Vec<ComposeError>,
) {
    if let Some(prev_file) = symbols.get(name) {
        errors.push(ComposeError::DuplicateSymbol {
            name: name.to_string(),
            file1: prev_file.to_string(),
            file2: file.to_string(),
        });
    } else {
        symbols.insert(name.to_string(), file);
    }
}

/// Check whether a top-level item has a named symbol (used for counting).
pub fn top_level_has_name(item: &TopLevel) -> bool {
    top_level_name(item).is_some()
}

/// Extract the name of a top-level declaration, if it's a named symbol.
/// Returns None for Use, FnMain, and Import (which don't participate in symbol dedup).
fn top_level_name(item: &TopLevel) -> Option<&str> {
    match item {
        TopLevel::Task(d) => Some(&d.name.node),
        TopLevel::Pure(d) => Some(&d.name.node),
        TopLevel::Flow(d) => Some(&d.name.node),
        TopLevel::Agent(d) => Some(&d.name.node),
        TopLevel::Pool(d) => Some(&d.name.node),
        TopLevel::Event(d) => Some(&d.name.node),
        TopLevel::States(d) => Some(&d.name.node),
        TopLevel::TypeDef(d) => Some(&d.name.node),
        TopLevel::Endpoint(d) => Some(&d.name.node),
        TopLevel::Contract(d) => Some(&d.name.node),
        TopLevel::Warden(d) => Some(&d.name.node),
        TopLevel::System(d) => Some(&d.name.node),
        TopLevel::Use(_) | TopLevel::FnMain(_) | TopLevel::Import(_) => None,
    }
}

// ── Public helpers for generated binaries ─────────────────────

/// Parse embedded (filename, content) pairs and merge into a single Program.
/// Used by generated binaries that embed .forge sources via `include_str!`.
pub fn parse_and_merge_sources(sources: &[(&str, &str)]) -> Result<Program, String> {
    let mut files = Vec::with_capacity(sources.len());

    for (name, content) in sources {
        let program = crate::parser::parse(content)
            .map_err(|e| format!("parse error in {}: {}", name, e.to_diagnostic(name).message))?;
        files.push(SourceFile {
            path: name.to_string(),
            source: content.to_string(),
            program,
        });
    }

    let composed = merge_programs(&files).map_err(|errs| {
        errs.iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    })?;

    Ok(composed.program)
}

/// Find the (first or named) agent declaration in a merged program.
pub fn find_agent(program: &Program, name: Option<&str>) -> Option<AgentDecl> {
    program.items.iter().find_map(|item| match &item.node {
        TopLevel::Agent(a) => {
            if let Some(n) = name {
                if a.name.node == n {
                    Some(a.as_ref().clone())
                } else {
                    None
                }
            } else {
                Some(a.as_ref().clone())
            }
        }
        _ => None,
    })
}

/// Find a states declaration by name, or the first one if no name given.
pub fn find_states(program: &Program, name: Option<&str>) -> Option<StatesDecl> {
    program.items.iter().find_map(|item| match &item.node {
        TopLevel::States(s) => {
            if let Some(n) = name {
                if s.name.node == n {
                    Some(s.clone())
                } else {
                    None
                }
            } else {
                Some(s.clone())
            }
        }
        _ => None,
    })
}

// ── Program kind detection ────────────────────────────────────

/// The detected execution mode of a merged FORGE program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramKind {
    /// Has `fn main` — standard executable.
    Executable,
    /// Has agent(s) but no `fn main` — agent CLI with handler subcommands.
    AgentCli,
    /// Has endpoint(s) — HTTP server.
    Server,
    /// Has `system` declaration.
    System,
}

/// Detect the program kind from a (merged) program's top-level declarations.
///
/// Priority: fn main → system → endpoints → agent.
pub fn detect_kind(program: &Program) -> Option<ProgramKind> {
    let mut has_fn_main = false;
    let mut has_system = false;
    let mut has_endpoints = false;
    let mut has_agent = false;

    for item in &program.items {
        match &item.node {
            TopLevel::FnMain(_) => has_fn_main = true,
            TopLevel::System(_) => has_system = true,
            TopLevel::Endpoint(_) => has_endpoints = true,
            TopLevel::Agent(_) => has_agent = true,
            _ => {}
        }
    }

    if has_fn_main {
        Some(ProgramKind::Executable)
    } else if has_system && !has_agent && !has_endpoints {
        // Pure system declarations (no fallback to agent/server mode)
        Some(ProgramKind::System)
    } else if has_endpoints {
        Some(ProgramKind::Server)
    } else if has_agent {
        Some(ProgramKind::AgentCli)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_source(name: &str, source: &str) -> SourceFile {
        let program = crate::parser::parse(source).expect("parse failed");
        SourceFile {
            path: name.to_string(),
            source: source.to_string(),
            program,
        }
    }

    #[test]
    fn merge_two_files() {
        let a = parse_source(
            "a.forge",
            "use\n  llm.reason\n\ntask greet\n  needs name: Text\n  gives Text\n  do\n    result = reason \"Say hello to {name}\"\n    give result\n",
        );
        let b = parse_source(
            "b.forge",
            "use\n  llm.reason\n\npure double\n  needs n: Number\n  gives Number\n  do\n    give n * 2\n",
        );

        let composed = merge_programs(&[a, b]).expect("merge should succeed");
        // Both files' items present
        let names: Vec<_> = composed
            .program
            .items
            .iter()
            .filter_map(|i| top_level_name(&i.node))
            .collect();
        assert!(names.contains(&"greet"));
        assert!(names.contains(&"double"));
    }

    #[test]
    fn rejects_duplicate_symbols() {
        let a = parse_source(
            "a.forge",
            "use\n  llm.reason\n\ntask greet\n  needs name: Text\n  gives Text\n  do\n    result = reason \"hello {name}\"\n    give result\n",
        );
        let b = parse_source(
            "b.forge",
            "use\n  llm.reason\n\ntask greet\n  needs name: Text\n  gives Text\n  do\n    result = reason \"hi {name}\"\n    give result\n",
        );

        let err = merge_programs(&[a, b]).unwrap_err();
        assert_eq!(err.len(), 1);
        assert!(err[0].to_string().contains("duplicate symbol `greet`"));
    }

    #[test]
    fn rejects_multiple_fn_main() {
        let a = parse_source("a.forge", "fn main\n  say \"hello\"\n");
        let b = parse_source("b.forge", "fn main\n  say \"world\"\n");

        let err = merge_programs(&[a, b]).unwrap_err();
        assert!(err
            .iter()
            .any(|e| matches!(e, ComposeError::MultipleFnMain { .. })));
    }

    #[test]
    fn detect_kind_executable() {
        let f = parse_source("test.forge", "fn main\n  say \"hello\"\n");
        assert_eq!(detect_kind(&f.program), Some(ProgramKind::Executable));
    }

    #[test]
    fn detect_kind_agent_cli() {
        let f = parse_source(
            "test.forge",
            "use\n  llm.reason\n\nagent bot\n  on greet(name: Text)\n    result = reason \"hello {name}\"\n    say result\n",
        );
        assert_eq!(detect_kind(&f.program), Some(ProgramKind::AgentCli));
    }

    #[test]
    fn single_file_merge() {
        let f = parse_source("hello.forge", "fn main\n  say \"hello world\"\n");
        let composed = merge_programs(&[f]).expect("single file merge should work");
        assert_eq!(
            detect_kind(&composed.program),
            Some(ProgramKind::Executable)
        );
    }
}
