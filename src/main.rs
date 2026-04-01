use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;

use forge::ast::TopLevel;
use forge::runtime::agent::AgentProcess;
use forge::runtime::confidence::{ConfidentValue, Value};

#[derive(Parser)]
#[command(
    name = "forge",
    about = "FORGE — an agent-native programming language",
    long_about = "FORGE — an agent-native programming language\n\n\
        Compiles declarative task/flow/agent definitions into LLM-powered programs.\n\
        Configure providers in forge.config.toml or use FORGE_MOCK=1 for testing.",
    version,
    after_help = "\
ENVIRONMENT VARIABLES:
  FORGE_CONFIG         Path to config file (default: ./forge.config.toml)
  FORGE_PROVIDER       Override default LLM provider
  FORGE_MOCK=1         Use mock provider (no API key needed)
  FORGE_BUDGET         Set max cost in USD (e.g., 0.50)
  FORGE_TRACE=1        Enable JSON tracing to stderr
  FORGE_LOG_LEVEL      Log level: quiet, info (default), debug
  ANTHROPIC_API_KEY    API key for Anthropic provider
  OPENAI_API_KEY       API key for OpenAI provider
  GROQ_API_KEY         API key for Groq provider
  TOGETHER_API_KEY     API key for Together provider
  MISTRAL_API_KEY      API key for Mistral provider"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Parse a .forge file and print the AST (useful for debugging grammar)
    Parse {
        /// Path to the .forge source file
        file: PathBuf,
    },
    /// Type-check capabilities, composition types, and purity constraints
    Check {
        /// Path to the .forge source file
        file: PathBuf,
    },
    /// Execute a .forge program
    Run {
        /// Path to the .forge source file
        file: PathBuf,
    },
    /// Execute with full JSON trace output to stderr
    Trace {
        /// Path to the .forge source file
        file: PathBuf,
    },
    /// Start an interactive agent REPL session
    Agent {
        /// Path to a .forge file containing an agent declaration
        file: PathBuf,
    },
    /// Estimate token usage and cost via static analysis (no API calls)
    Cost {
        /// Path to the .forge source file
        file: PathBuf,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Parse { file } => {
            let source = read_source(&file)?;
            match forge::parser::parse(&source) {
                Ok(program) => println!("{:#?}", program),
                Err(e) => {
                    e.to_diagnostic(&file.display().to_string()).render(&source);
                    std::process::exit(1);
                }
            }
        }
        Command::Check { file } => {
            let source = read_source(&file)?;
            let program = parse_or_exit(&source, &file);
            let ctx = forge::resolver::CheckContext::new(&file.display().to_string());
            match ctx.check(&program) {
                Ok(()) => println!("OK"),
                Err(errors) => {
                    let registry = forge::resolver::CapabilityRegistry::builtin();
                    let diagnostics: Vec<_> = errors.iter()
                        .map(|e| e.to_diagnostic(&file.display().to_string(), &registry))
                        .collect();
                    forge::diagnostic::render_diagnostics(&source, &diagnostics);
                    std::process::exit(1);
                }
            }
        }
        Command::Run { file } => {
            run_program(&file, false).await?;
        }
        Command::Trace { file } => {
            run_program(&file, true).await?;
        }
        Command::Agent { file } => {
            run_agent(&file).await?;
        }
        Command::Cost { file } => {
            let source = read_source(&file)?;
            let program = parse_or_exit(&source, &file);
            let config = forge::config::ForgeConfig::load_or_default();
            let estimate = forge::cost_estimator::estimate(&program, &config);
            print!("{}", estimate);
        }
    }

    Ok(())
}

fn read_source(file: &PathBuf) -> anyhow::Result<String> {
    std::fs::read_to_string(file)
        .map_err(|e| anyhow::anyhow!("could not read {}: {}", file.display(), e))
}

fn parse_or_exit(source: &str, file: &PathBuf) -> forge::ast::Program {
    match forge::parser::parse(source) {
        Ok(program) => program,
        Err(e) => {
            e.to_diagnostic(&file.display().to_string()).render(source);
            std::process::exit(1);
        }
    }
}

async fn run_program(file: &PathBuf, trace: bool) -> anyhow::Result<()> {
    let source = read_source(file)?;
    let program = parse_or_exit(&source, file);

    let config = forge::config::ForgeConfig::load_or_default();
    let registry = forge::llm::registry::ProviderRegistry::from_config(config)
        .map_err(|e| anyhow::anyhow!("provider setup failed: {}", e))?;

    let tracer = if trace || std::env::var("FORGE_TRACE").map(|v| v == "1").unwrap_or(false) {
        Some(forge::tracer::Tracer::new())
    } else {
        None
    };

    let executor = forge::runtime::executor::TaskExecutor::new(
        program,
        Arc::new(registry),
        tracer,
    );

    match executor.run().await {
        Ok(_) => {}
        Err(e) => {
            eprintln!("runtime error: {}", e);
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn run_agent(file: &PathBuf) -> anyhow::Result<()> {
    let source = read_source(file)?;
    let program = parse_or_exit(&source, file);

    // Find the agent declaration and optional states
    let agent_decl = program.items.iter()
        .find_map(|item| match &item.node {
            TopLevel::Agent(a) => Some(a.clone()),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("no agent declaration found in {}", file.display()))?;

    let states_decl = program.items.iter()
        .find_map(|item| match &item.node {
            TopLevel::States(s) => Some(s.clone()),
            _ => None,
        });

    let config = forge::config::ForgeConfig::load_or_default();
    let registry = forge::llm::registry::ProviderRegistry::from_config(config)
        .map_err(|e| anyhow::anyhow!("provider setup failed: {}", e))?;

    let agent = AgentProcess::new(
        agent_decl.clone(),
        states_decl.as_ref(),
        Arc::new(registry),
        None,
        program,
    );

    // Print banner
    let handler_names: Vec<&str> = agent_decl.handlers.iter()
        .map(|h| h.node.event.node.as_str())
        .collect();
    let memory_fields: Vec<&str> = agent_decl.memory.iter()
        .map(|f| f.node.name.as_str())
        .collect();

    println!("FORGE Agent: {}", agent_decl.name.node);
    if !memory_fields.is_empty() {
        println!("  memory: {}", memory_fields.join(", "));
    }
    println!("  handlers: {}", handler_names.join(", "));
    println!();
    println!("Type an event name with optional arguments. Examples:");
    println!("  start");
    println!("  answer \"the pure keyword\"");
    println!("  quit");
    println!();

    // REPL loop
    let stdin = io::stdin();
    let mut reader = stdin.lock().lines();

    loop {
        print!("> ");
        io::stdout().flush()?;

        let line = match reader.next() {
            Some(Ok(line)) => line,
            _ => break,
        };

        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed == "quit" || trimmed == "exit" {
            break;
        }

        // Parse input: event_name [arg1] ["quoted arg"] ...
        let (event_name, raw_args) = match trimmed.split_once(char::is_whitespace) {
            Some((name, rest)) => (name.trim(), parse_args(rest.trim())),
            None => (trimmed, vec![]),
        };

        // Match positional args to handler param names
        let handler = agent_decl.handlers.iter()
            .find(|h| h.node.event.node == event_name);
        let mut params = HashMap::new();
        if let Some(h) = handler {
            for (i, param) in h.node.params.iter().enumerate() {
                if let Some(arg) = raw_args.get(i) {
                    params.insert(
                        param.node.name.clone(),
                        ConfidentValue::deterministic(Value::Text(arg.clone())),
                    );
                }
            }
        }

        match agent.dispatch(event_name, params).await {
            Ok(Some(val)) => println!("→ {}", val.value),
            Ok(None) => {}
            Err(e) => eprintln!("error: {}", e),
        }
        println!();
    }

    println!("bye!");
    Ok(())
}

/// Parse arguments from input, respecting quoted strings.
fn parse_args(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                if in_quotes {
                    // End of quoted string
                    args.push(current.clone());
                    current.clear();
                    in_quotes = false;
                } else {
                    // Start of quoted string
                    in_quotes = true;
                }
            }
            ' ' | '\t' if !in_quotes => {
                if !current.is_empty() {
                    args.push(current.clone());
                    current.clear();
                }
            }
            _ => {
                current.push(ch);
            }
        }
    }

    if !current.is_empty() {
        args.push(current);
    }

    args
}
