use clap::{Parser, Subcommand};
use std::path::PathBuf;

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

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Parse { file } => {
            let source = std::fs::read_to_string(&file)
                .map_err(|e| anyhow::anyhow!("could not read {}: {}", file.display(), e))?;
            let program = forge::parser::parse(&source)?;
            println!("{:#?}", program);
        }
        Command::Check { file: _ } => {
            eprintln!("not yet implemented: check");
        }
        Command::Run { file: _ } => {
            eprintln!("not yet implemented: run");
        }
        Command::Trace { file: _ } => {
            eprintln!("not yet implemented: trace");
        }
        Command::Cost { file: _ } => {
            eprintln!("not yet implemented: cost");
        }
    }

    Ok(())
}
