use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use forge::ast::{Expr, TemplatePart, TopLevel};
use forge::portability::{
    build_package, inspect_package, load_package, prepare_imported_entries, verify_integrity,
    AgentSchema, SchemaField, SchemaKnowledgeConfig,
};
use forge::runtime::agent::AgentProcess;
use forge::runtime::confidence::{ConfidentValue, Value};
use forge::runtime::knowledge_store::KnowledgeStore;

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
        /// Paths to .forge source files
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Execute a .forge program
    Run {
        /// Path to the .forge source file
        file: Option<PathBuf>,
        /// Path to a forge.project.toml manifest for multi-file execution
        #[arg(long, conflicts_with = "file")]
        manifest: Option<PathBuf>,
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
    /// Start an HTTP server for endpoint declarations
    Serve {
        /// Path to a .forge file containing endpoint declarations
        file: PathBuf,
        /// Host to bind to (overrides config)
        #[arg(long)]
        host: Option<String>,
        /// Port to bind to (overrides config)
        #[arg(long)]
        port: Option<u16>,
        /// Watch for file changes and hot-reload (dev mode)
        #[arg(long)]
        watch: bool,
    },
    /// Export an agent's knowledge and config as a .forgepkg.json package
    Export {
        /// Path to the .forge source file
        file: PathBuf,
        /// Name of the agent to export (if file has multiple agents)
        #[arg(long)]
        agent: Option<String>,
        /// Layers to export (comma-separated: config,knowledge,memory)
        #[arg(long, default_value = "config,knowledge")]
        layers: String,
        /// Output file path
        #[arg(long, short)]
        output: Option<PathBuf>,
    },
    /// Import knowledge from a .forgepkg.json package into a local store
    Import {
        /// Path to the .forgepkg.json package
        package: PathBuf,
        /// Target knowledge store path
        #[arg(long)]
        into: String,
        /// Confidence cap for imported entries (0.0-1.0)
        #[arg(long, default_value = "0.7")]
        confidence_cap: f32,
    },
    /// Inspect a .forgepkg.json package
    Inspect {
        /// Path to the .forgepkg.json package
        package: PathBuf,
    },
    /// Send a single event to an agent non-interactively and print the result
    Send {
        /// Path to a .forge file containing an agent declaration
        file: PathBuf,
        /// Event name to dispatch (e.g., "query", "ingest", "status")
        event: String,
        /// Arguments for the event handler (positional, matching handler params)
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// Build a standalone binary from a .forge project
    Build {
        /// Path to a .forge file or directory containing forge.project.toml
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Output binary name
        #[arg(long, short)]
        output: Option<String>,
        /// Path to forge.project.toml (overrides auto-detection)
        #[arg(long)]
        manifest: Option<PathBuf>,
        /// Build with optimizations
        #[arg(long)]
        release: bool,
        /// Embed a forge.config.toml as default config
        #[arg(long)]
        embed_config: Option<PathBuf>,
        /// Validate without compiling
        #[arg(long)]
        dry_run: bool,
    },
    /// Generate skeleton FORGE code from a plain-text system description
    Fleet {
        /// Plain-text system description (e.g., "a chat system with moderator and logger")
        #[arg(long)]
        spec: String,
        /// Output directory (default: print to stdout)
        #[arg(long, short)]
        output: Option<PathBuf>,
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
        Command::Check { files } => {
            let mut all_diagnostics = Vec::new();
            let mut parsed_programs = Vec::new();

            for file in &files {
                let source = read_source(file)?;
                let program = parse_or_exit(&source, file);
                let fname = file.display().to_string();

                // Per-file: resolver
                let ctx = forge::resolver::CheckContext::new(&fname);
                if let Err(errors) = ctx.check(&program) {
                    let registry = forge::resolver::CapabilityRegistry::builtin();
                    all_diagnostics
                        .extend(errors.iter().map(|e| e.to_diagnostic(&fname, &registry)));
                }

                // Per-file: checker (pure, states, requires)
                all_diagnostics.extend(forge::checker::check_all(&program, &fname));

                parsed_programs.push((program, fname, source));
            }

            // Cross-file: boundary checker
            let boundary_refs: Vec<_> = parsed_programs
                .iter()
                .map(|(p, f, _)| (p, f.as_str()))
                .collect();
            all_diagnostics.extend(forge::checker::boundary_checker::check(&boundary_refs));

            if all_diagnostics.is_empty() {
                println!("OK");
            } else {
                // Render diagnostics for each file with its source
                for diag in &all_diagnostics {
                    if let Some((_, _, source)) =
                        parsed_programs.iter().find(|(_, f, _)| f == &diag.file)
                    {
                        diag.render(source);
                    }
                }
                std::process::exit(1);
            }
        }
        Command::Run { file, manifest } => {
            if let Some(manifest_path) = manifest {
                run_manifest(&manifest_path, false).await?;
            } else if let Some(file) = file {
                run_program(&file, false).await?;
            } else {
                anyhow::bail!("either a .forge file or --manifest is required");
            }
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
        Command::Serve {
            file,
            host,
            port,
            watch,
        } => {
            serve_program(&file, host, port, watch).await?;
        }
        Command::Export {
            file,
            agent,
            layers,
            output,
        } => {
            let source = read_source(&file)?;
            let program = parse_or_exit(&source, &file);

            // Find the agent declaration
            let agent_decl = if let Some(ref name) = agent {
                program
                    .items
                    .iter()
                    .find_map(|item| match &item.node {
                        TopLevel::Agent(a) if a.name.node == *name => Some(a.as_ref().clone()),
                        _ => None,
                    })
                    .ok_or_else(|| {
                        anyhow::anyhow!("no agent named '{}' found in {}", name, file.display())
                    })?
            } else {
                program
                    .items
                    .iter()
                    .find_map(|item| match &item.node {
                        TopLevel::Agent(a) => Some(a.as_ref().clone()),
                        _ => None,
                    })
                    .ok_or_else(|| {
                        anyhow::anyhow!("no agent declaration found in {}", file.display())
                    })?
            };

            let layer_set: Vec<&str> = layers.split(',').map(|s| s.trim()).collect();

            // Build schema from AST if "config" layer requested
            let schema = if layer_set.contains(&"config") {
                let fields: Vec<SchemaField> = agent_decl
                    .memory
                    .iter()
                    .map(|f| SchemaField {
                        name: f.node.name.clone(),
                        field_type: format!("{:?}", f.node.type_name.node),
                        default: None,
                    })
                    .collect();

                let knowledge_config =
                    agent_decl
                        .knowledge
                        .as_ref()
                        .map(|k| SchemaKnowledgeConfig {
                            max_entries: k.node.max_entries.as_ref().map(|m| m.node as usize),
                            retention_days: k.node.retention.as_ref().map(|r| {
                                use forge::ast::DurationUnit;
                                let dur = &r.node;
                                match dur.unit {
                                    DurationUnit::Days => dur.value,
                                    DurationUnit::Hours => dur.value / 24,
                                    DurationUnit::Minutes => dur.value / 1440,
                                    DurationUnit::Seconds => dur.value / 86400,
                                }
                            }),
                        });

                AgentSchema {
                    fields,
                    knowledge_config,
                }
            } else {
                AgentSchema {
                    fields: vec![],
                    knowledge_config: None,
                }
            };

            // Load knowledge entries if "knowledge" layer requested
            let knowledge = if layer_set.contains(&"knowledge") {
                if let Some(ref k) = agent_decl.knowledge {
                    let store_path = match &k.node.store_path.node {
                        Expr::Template(parts) => {
                            let mut s = String::new();
                            for p in parts {
                                if let TemplatePart::Text(t) = &p.node {
                                    s.push_str(t);
                                }
                            }
                            s
                        }
                        _ => String::new(),
                    };
                    if !store_path.is_empty() {
                        let store = KnowledgeStore::new(&store_path, None, None);
                        store.export_entries()
                    } else {
                        vec![]
                    }
                } else {
                    vec![]
                }
            } else {
                vec![]
            };

            let agent_name = agent_decl.name.node.clone();
            let pkg = build_package(&agent_name, &agent_name, None, schema, knowledge);
            let json = serde_json::to_string_pretty(&pkg)?;

            match output {
                Some(path) => {
                    std::fs::write(&path, &json)?;
                    println!("exported to {}", path.display());
                }
                None => {
                    let default_path = format!("{}.forgepkg.json", agent_name);
                    std::fs::write(&default_path, &json)?;
                    println!("exported to {}", default_path);
                }
            }
        }
        Command::Import {
            package,
            into,
            confidence_cap,
        } => {
            let json = std::fs::read_to_string(&package)
                .map_err(|e| anyhow::anyhow!("could not read {}: {}", package.display(), e))?;
            let pkg =
                load_package(&json).map_err(|e| anyhow::anyhow!("invalid package: {:?}", e))?;
            verify_integrity(&pkg)
                .map_err(|e| anyhow::anyhow!("integrity check failed: {:?}", e))?;

            let entries = prepare_imported_entries(&pkg, confidence_cap);
            let mut store = KnowledgeStore::new(&into, None, None);
            let count = store.merge_imported(entries);

            println!("imported {} entries into {}", count, into);
        }
        Command::Inspect { package } => {
            let json = std::fs::read_to_string(&package)
                .map_err(|e| anyhow::anyhow!("could not read {}: {}", package.display(), e))?;
            let pkg =
                load_package(&json).map_err(|e| anyhow::anyhow!("invalid package: {:?}", e))?;
            print!("{}", inspect_package(&pkg));
        }
        Command::Send { file, event, args } => {
            send_to_agent(&file, &event, args).await?;
        }
        Command::Build {
            path,
            output,
            manifest,
            release,
            embed_config,
            dry_run,
        } => {
            build_program(
                &path,
                output.as_deref(),
                manifest.as_deref(),
                release,
                embed_config,
                dry_run,
            )
            .await?;
        }
        Command::Fleet { spec, output } => {
            let result = forge::fleet::generate(&spec)?;
            match output {
                Some(dir) => {
                    forge::fleet::write_to_dir(&result, &dir)?;
                }
                None => {
                    print!("{}", result.source);
                }
            }
        }
    }

    Ok(())
}

/// Open the FORGE persistent storage database (issue #48/#57).
fn open_forge_storage() -> anyhow::Result<forge::runtime::storage::SharedStorage> {
    let db_path = std::path::Path::new(".forge-data").join("store.redb");
    std::fs::create_dir_all(".forge-data")?;
    let storage = forge::runtime::storage::ForgeStorage::open(&db_path)
        .map_err(|e| anyhow::anyhow!("failed to open storage: {}", e))?;
    Ok(Arc::new(storage))
}

fn read_source(file: &PathBuf) -> anyhow::Result<String> {
    std::fs::read_to_string(file)
        .map_err(|e| anyhow::anyhow!("could not read {}: {}", file.display(), e))
}

fn parse_or_exit(source: &str, file: &Path) -> forge::ast::Program {
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
    let fname = file.display().to_string();

    // Validate before execution
    let mut diagnostics = Vec::new();

    let ctx = forge::resolver::CheckContext::new(&fname);
    if let Err(errors) = ctx.check(&program) {
        let registry = forge::resolver::CapabilityRegistry::builtin();
        diagnostics.extend(errors.iter().map(|e| e.to_diagnostic(&fname, &registry)));
    }

    diagnostics.extend(forge::checker::check_all(&program, &fname));

    // Boundary checker (single-file only — catches endpoint placement and per-file rules;
    // cross-file checks require `forge check` with multiple files)
    let boundary_refs = vec![(&program, fname.as_str())];
    diagnostics.extend(forge::checker::boundary_checker::check(&boundary_refs));

    if !diagnostics.is_empty() {
        forge::diagnostic::render_diagnostics(&source, &diagnostics);
        std::process::exit(1);
    }

    let config = forge::config::ForgeConfig::load_or_default();
    let config_clone = config.clone();
    let registry = forge::llm::registry::ProviderRegistry::from_config(config)
        .map_err(|e| anyhow::anyhow!("provider setup failed: {}", e))?;

    let tracer = if trace
        || std::env::var("FORGE_TRACE")
            .map(|v| v == "1")
            .unwrap_or(false)
    {
        Some(forge::tracer::Tracer::new())
    } else {
        None
    };

    let executor = forge::runtime::executor::TaskExecutor::new(program, Arc::new(registry), tracer)
        .with_config(config_clone);

    match executor.run().await {
        Ok(_) => {}
        Err(e) => {
            eprintln!("runtime error: {}", e);
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn run_manifest(manifest_path: &Path, trace: bool) -> anyhow::Result<()> {
    let manifest = forge::manifest::ProjectManifest::load(manifest_path)?;
    let base_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let source_paths = manifest.resolve_sources(base_dir)?;

    // Parse all source files
    let mut source_files = Vec::new();
    let mut diagnostics = Vec::new();

    for path in &source_paths {
        let source = read_source(&path.to_path_buf())?;
        let fname = path.display().to_string();
        let program = parse_or_exit(&source, path);

        // Per-file validation
        let ctx = forge::resolver::CheckContext::new(&fname);
        if let Err(errors) = ctx.check(&program) {
            let registry = forge::resolver::CapabilityRegistry::builtin();
            diagnostics.extend(errors.iter().map(|e| e.to_diagnostic(&fname, &registry)));
        }
        diagnostics.extend(forge::checker::check_all(&program, &fname));

        source_files.push(forge::compose::SourceFile {
            path: fname,
            source,
            program,
        });
    }

    // Cross-file boundary check
    let boundary_refs: Vec<_> = source_files
        .iter()
        .map(|sf| (&sf.program, sf.path.as_str()))
        .collect();
    diagnostics.extend(forge::checker::boundary_checker::check(&boundary_refs));

    if !diagnostics.is_empty() {
        for diag in &diagnostics {
            if let Some(sf) = source_files.iter().find(|sf| sf.path == diag.file) {
                diag.render(&sf.source);
            }
        }
        std::process::exit(1);
    }

    // Merge and execute
    let composed = forge::compose::merge_programs(&source_files).map_err(|errs| {
        anyhow::anyhow!(
            "{}",
            errs.iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        )
    })?;

    let config = forge::config::ForgeConfig::load_or_default();
    let config_clone = config.clone();
    let registry = forge::llm::registry::ProviderRegistry::from_config(config)
        .map_err(|e| anyhow::anyhow!("provider setup failed: {}", e))?;

    let tracer = if trace
        || std::env::var("FORGE_TRACE")
            .map(|v| v == "1")
            .unwrap_or(false)
    {
        Some(forge::tracer::Tracer::new())
    } else {
        None
    };

    let executor =
        forge::runtime::executor::TaskExecutor::new(composed.program, Arc::new(registry), tracer)
            .with_config(config_clone);

    match executor.run().await {
        Ok(_) => {}
        Err(e) => {
            eprintln!("runtime error: {}", e);
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn build_program(
    path: &Path,
    output: Option<&str>,
    manifest_path: Option<&Path>,
    release: bool,
    embed_config: Option<PathBuf>,
    dry_run: bool,
) -> anyhow::Result<()> {
    // Resolve manifest: explicit path, directory with forge.project.toml, or single file
    let (mut manifest, base_dir) = if let Some(mp) = manifest_path {
        let manifest = forge::manifest::ProjectManifest::load(mp)?;
        let base = mp.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
        (manifest, base)
    } else if path.is_file() && path.extension().map(|e| e == "forge").unwrap_or(false) {
        // Single-file shortcut — use file stem for crate name, not the output path
        let manifest = forge::manifest::ProjectManifest::from_single_file(path, None);
        let base = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        (manifest, base)
    } else if path.is_dir() {
        let manifest_file = path.join("forge.project.toml");
        if !manifest_file.exists() {
            anyhow::bail!("no forge.project.toml found in {}", path.display());
        }
        let manifest = forge::manifest::ProjectManifest::load(&manifest_file)?;
        (manifest, path.to_path_buf())
    } else {
        anyhow::bail!(
            "expected a .forge file, directory with forge.project.toml, or --manifest flag"
        );
    };

    // Resolve output: if -o is a path with dir, split into dir + name
    let (output_dir, output_name_override) = if let Some(out) = output {
        let out_path = Path::new(out);
        if let Some(parent) = out_path.parent() {
            if !parent.as_os_str().is_empty() {
                (
                    Some(parent.to_path_buf()),
                    out_path
                        .file_name()
                        .map(|f| f.to_string_lossy().to_string()),
                )
            } else {
                (None, Some(out.to_string()))
            }
        } else {
            (None, Some(out.to_string()))
        }
    } else {
        (None, None)
    };

    // Override the manifest output name if -o was given
    if let Some(name) = &output_name_override {
        if let Some(ref mut build) = manifest.build {
            build.output = Some(name.clone());
        } else {
            manifest.build = Some(forge::manifest::BuildConfig {
                entry: None,
                output: Some(name.clone()),
                sources: None,
            });
        }
    }

    let output_display = manifest.output_name();
    eprintln!("building {} ...", output_display);

    let pipeline = forge::build::BuildPipeline::new(manifest, base_dir)
        .dry_run(dry_run)
        .release(release)
        .embed_config(embed_config)
        .output_dir(output_dir);

    let result = pipeline.build()?;

    if dry_run {
        eprintln!(
            "dry run complete — validation passed ({:?})",
            result.program_kind
        );
    } else {
        eprintln!("done: {}", result.binary_path.display());
    }

    Ok(())
}

/// Try to parse, validate, and build an executor from a .forge file.
/// Returns the executor and config on success, or renders diagnostics and returns an error.
fn try_build_executor(
    file: &Path,
) -> Result<
    (
        forge::runtime::executor::TaskExecutor,
        forge::config::ForgeConfig,
    ),
    anyhow::Error,
> {
    let source = read_source(&file.to_path_buf())?;
    let fname = file.display().to_string();

    let program = match forge::parser::parse(&source) {
        Ok(p) => p,
        Err(e) => {
            e.to_diagnostic(&fname).render(&source);
            return Err(anyhow::anyhow!("parse error in {}", fname));
        }
    };

    let mut diagnostics = Vec::new();

    let ctx = forge::resolver::CheckContext::new(&fname);
    if let Err(errors) = ctx.check(&program) {
        let registry = forge::resolver::CapabilityRegistry::builtin();
        diagnostics.extend(errors.iter().map(|e| e.to_diagnostic(&fname, &registry)));
    }

    diagnostics.extend(forge::checker::check_all(&program, &fname));

    let boundary_refs = vec![(&program, fname.as_str())];
    diagnostics.extend(forge::checker::boundary_checker::check(&boundary_refs));

    if !diagnostics.is_empty() {
        forge::diagnostic::render_diagnostics(&source, &diagnostics);
        return Err(anyhow::anyhow!("{} diagnostic error(s)", diagnostics.len()));
    }

    let config = forge::config::ForgeConfig::load_or_default();

    let tracer = if std::env::var("FORGE_TRACE")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        Some(forge::tracer::Tracer::new())
    } else {
        None
    };

    let registry = forge::llm::registry::ProviderRegistry::from_config(config.clone())
        .map_err(|e| anyhow::anyhow!("provider setup failed: {}", e))?;

    let executor = forge::runtime::executor::TaskExecutor::new(program, Arc::new(registry), tracer)
        .with_config(config.clone());

    Ok((executor, config))
}

async fn serve_program(
    file: &PathBuf,
    cli_host: Option<String>,
    cli_port: Option<u16>,
    watch: bool,
) -> anyhow::Result<()> {
    if watch {
        serve_with_watch(file, cli_host, cli_port).await
    } else {
        // Non-watch mode: build once and serve. Exits on errors.
        let (executor, config) = match try_build_executor(file) {
            Ok(r) => r,
            Err(_) => std::process::exit(1),
        };

        let mut server =
            forge::runtime::http_server::ForgeServer::new(executor, config.server.as_ref());

        if let Some(host) = cli_host {
            server = server.with_host(host);
        }
        if let Some(port) = cli_port {
            server = server.with_port(port);
        }

        server.run().await
    }
}

async fn serve_with_watch(
    file: &Path,
    cli_host: Option<String>,
    cli_port: Option<u16>,
) -> anyhow::Result<()> {
    use forge::runtime::watcher::WatchAction;

    loop {
        let (executor, config) = match try_build_executor(file) {
            Ok(r) => r,
            Err(_) => std::process::exit(1),
        };

        let mut server =
            forge::runtime::http_server::ForgeServer::new(executor, config.server.as_ref())
                .with_watch_mode(true);

        if let Some(ref host) = cli_host {
            server = server.with_host(host.clone());
        }
        if let Some(port) = cli_port {
            server = server.with_port(port);
        }

        let swappable = server.swappable_executor();
        let reload_tx = server.reload_sender();

        let watch_file = file.to_path_buf();
        let watcher_handle = tokio::spawn(async move {
            forge::runtime::watcher::watch_and_reload(watch_file, swappable, reload_tx).await
        });

        tokio::select! {
            result = server.run() => {
                // Server stopped (SIGINT) — watcher task is dropped and cancelled
                return result;
            }
            watch_result = watcher_handle => {
                match watch_result {
                    Ok(Ok(WatchAction::RestartServer)) => {
                        eprintln!("Config changed -- restarting server...");
                        continue;
                    }
                    Ok(Err(e)) => {
                        eprintln!("Watcher error: {e}");
                        return Err(e);
                    }
                    Err(e) => {
                        eprintln!("Watcher task failed: {e}");
                        return Err(anyhow::anyhow!("watcher task failed: {e}"));
                    }
                }
            }
        }
    }
}

async fn run_agent(file: &PathBuf) -> anyhow::Result<()> {
    let source = read_source(file)?;
    let program = parse_or_exit(&source, file);

    // Find the agent declaration and optional states
    let agent_decl = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Agent(a) => Some(a.as_ref().clone()),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("no agent declaration found in {}", file.display()))?;

    let states_decl = program.items.iter().find_map(|item| match &item.node {
        TopLevel::States(s) => Some(s.clone()),
        _ => None,
    });

    let config = forge::config::ForgeConfig::load_or_default();
    let registry = forge::llm::registry::ProviderRegistry::from_config(config)
        .map_err(|e| anyhow::anyhow!("provider setup failed: {}", e))?;

    // Open persistent storage if any agent declares memory persistent
    let storage = if agent_decl.memory_persistent {
        Some(open_forge_storage()?)
    } else {
        None
    };

    let agent = AgentProcess::new(
        agent_decl.clone(),
        states_decl.as_ref(),
        Arc::new(registry),
        None,
        program,
        storage,
        None,
    );

    // Print banner
    let handler_names: Vec<&str> = agent_decl
        .handlers
        .iter()
        .map(|h| h.node.event.node.as_str())
        .collect();
    let memory_fields: Vec<&str> = agent_decl
        .memory
        .iter()
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
        let handler = agent_decl
            .handlers
            .iter()
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
    let chars = input.chars().peekable();

    for ch in chars {
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

/// Send a single event to an agent non-interactively, print the result, and exit.
async fn send_to_agent(file: &PathBuf, event: &str, args: Vec<String>) -> anyhow::Result<()> {
    let source = read_source(file)?;
    let program = parse_or_exit(&source, file);

    let agent_decl = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Agent(a) => Some(a.as_ref().clone()),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("no agent declaration found in {}", file.display()))?;

    let states_decl = program.items.iter().find_map(|item| match &item.node {
        TopLevel::States(s) => Some(s.clone()),
        _ => None,
    });

    let config = forge::config::ForgeConfig::load_or_default();
    let registry = forge::llm::registry::ProviderRegistry::from_config(config)
        .map_err(|e| anyhow::anyhow!("provider setup failed: {}", e))?;

    // Always open storage in CLI mode — even non-persistent agents need
    // memory to survive across forge-send invocations
    let storage = Some(open_forge_storage()?);

    // Create instance registry for find/spawn support
    let instance_registry: forge::runtime::instance_registry::SharedInstanceRegistry = Arc::new(
        tokio::sync::RwLock::new(forge::runtime::instance_registry::InstanceRegistry::new()),
    );

    let agent = AgentProcess::new(
        agent_decl.clone(),
        states_decl.as_ref(),
        Arc::new(registry),
        None,
        program,
        storage,
        Some(instance_registry),
    );

    // Build params from positional args matching handler param names
    let handler = agent_decl
        .handlers
        .iter()
        .find(|h| h.node.event.node == event);
    let mut params = HashMap::new();
    if let Some(h) = handler {
        for (i, param) in h.node.params.iter().enumerate() {
            if let Some(arg) = args.get(i) {
                // Coerce CLI string args to declared parameter types
                use forge::ast::TypeName;
                let value = match &param.node.type_name.node {
                    TypeName::Number => {
                        if let Ok(n) = arg.parse::<f64>() {
                            Value::Number(n)
                        } else {
                            Value::Text(arg.clone())
                        }
                    }
                    TypeName::Bool => Value::Bool(arg == "true" || arg == "1"),
                    _ => Value::Text(arg.clone()),
                };
                params.insert(
                    param.node.name.clone(),
                    ConfidentValue::deterministic(value),
                );
            }
        }
    }

    match agent.dispatch(event, params).await {
        Ok(Some(val)) => println!("{}", val.value),
        Ok(None) => {}
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
    }

    Ok(())
}
