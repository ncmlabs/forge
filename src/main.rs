use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "forge", about = "FORGE — an agent-native programming language")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Parse a .forge file and print the AST
    Parse { file: PathBuf },
    /// Type-check and resolve capabilities
    Check { file: PathBuf },
    /// Execute a .forge file
    Run { file: PathBuf },
    /// Execute with full JSON trace to stderr
    Trace { file: PathBuf },
    /// Estimate token cost (static analysis)
    Cost { file: PathBuf },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Parse { file } => {
            let source = std::fs::read_to_string(&file)
                .map_err(|e| anyhow::anyhow!("could not read {}: {}", file.display(), e))?;
            let program = forge::parser::parse(&source)?;
            println!("{:#?}", program);
        }
        Command::Check { file } => {
            let source = std::fs::read_to_string(&file)
                .map_err(|e| anyhow::anyhow!("could not read {}: {}", file.display(), e))?;
            let program = forge::parser::parse(&source)?;
            let ctx = forge::resolver::CheckContext::new(&source, &file.display().to_string());
            match ctx.check(&program) {
                Ok(()) => println!("OK"),
                Err(errors) => {
                    for e in &errors {
                        eprintln!("{e}");
                    }
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
        Command::Cost { file: _ } => {
            eprintln!("not yet implemented: cost");
        }
    }

    Ok(())
}

async fn run_program(file: &PathBuf, trace: bool) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(file)
        .map_err(|e| anyhow::anyhow!("could not read {}: {}", file.display(), e))?;
    let program = forge::parser::parse(&source)?;

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
