use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use forge::ast::{Expr, TemplatePart, TopLevel};
use forge::portability::{
    build_package, inspect_package, load_package, prepare_imported_entries, verify_integrity,
    AgentSchema, SchemaField, SchemaKnowledgeConfig,
};
use forge::runtime::agent::AgentProcess;
use forge::runtime::command_manager::CommandManager;
use forge::runtime::confidence::{ConfidentValue, Value};
use forge::runtime::knowledge_store::KnowledgeStore;

/// Result of building an executor: (executor, config, skill_sigs, skill_exec).
type BuildResult = (
    forge::runtime::executor::TaskExecutor,
    forge::config::ForgeConfig,
    HashMap<String, forge::types::CapabilitySignature>,
    Option<Arc<forge::runtime::skill_executor::SkillExecutor>>,
);

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
        /// Merge files before checking (for multi-file projects with cross-file references)
        #[arg(long)]
        merge: bool,
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
        /// Additional source files to merge (for multi-file projects)
        #[arg(long = "source", short = 's')]
        sources: Vec<PathBuf>,
        /// Path to forge.project.toml (enables project-level skill declarations)
        #[arg(long)]
        manifest: Option<PathBuf>,
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
        /// Entry source file for a multi-file build (overrides manifest [build].entry)
        #[arg(long)]
        entry: Option<PathBuf>,
        /// Additional source files for a multi-file build (overrides manifest [build].sources)
        #[arg(long = "source", short = 's')]
        sources: Vec<PathBuf>,
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
        Command::Check { files, merge } => {
            let mut all_diagnostics = Vec::new();
            let mut parsed_programs = Vec::new();

            for file in &files {
                let source = read_source(file)?;
                let program = parse_or_exit(&source, file);
                let fname = file.display().to_string();

                // Per-file: resolver (always runs per-file)
                let ctx = forge::resolver::CheckContext::new(&fname);
                if let Err(errors) = ctx.check(&program) {
                    let registry = forge::resolver::CapabilityRegistry::builtin();
                    all_diagnostics
                        .extend(errors.iter().map(|e| e.to_diagnostic(&fname, &registry)));
                }

                parsed_programs.push((program, fname, source));
            }

            if merge && parsed_programs.len() > 1 {
                // Merge mode: combine all files, then run checkers on merged program.
                // This resolves cross-file references (states, types, functions).
                let source_files: Vec<_> = parsed_programs
                    .iter()
                    .map(|(p, f, s)| forge::compose::SourceFile {
                        path: f.clone(),
                        source: s.clone(),
                        program: p.clone(),
                    })
                    .collect();

                match forge::compose::merge_programs(&source_files) {
                    Ok(composed) => {
                        let merged_source = parsed_programs
                            .iter()
                            .map(|(_, _, s)| s.as_str())
                            .collect::<Vec<_>>()
                            .join("\n");
                        let merged_fname = "<merged>".to_string();
                        all_diagnostics
                            .extend(forge::checker::check_all(&composed.program, &merged_fname));
                        // Store merged program for diagnostic rendering
                        parsed_programs.push((composed.program, merged_fname, merged_source));
                    }
                    Err(errs) => {
                        for e in &errs {
                            eprintln!("Merge error: {e}");
                        }
                        std::process::exit(1);
                    }
                }
            } else {
                // Per-file checkers (pure, states, requires)
                for (program, fname, _) in &parsed_programs {
                    all_diagnostics.extend(forge::checker::check_all(program, fname));
                }
            }

            // Cross-file: boundary checker (always per-file)
            let boundary_refs: Vec<_> = parsed_programs
                .iter()
                .filter(|(_, f, _)| f != "<merged>")
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
            sources,
            manifest,
        } => {
            serve_program(&file, &sources, host, port, watch, manifest.as_deref()).await?;
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
            entry,
            sources,
            output,
            manifest,
            release,
            embed_config,
            dry_run,
        } => {
            build_program(
                &path,
                BuildProgramOptions {
                    entry: entry.as_deref(),
                    sources: &sources,
                    output: output.as_deref(),
                    manifest_path: manifest.as_deref(),
                    release,
                    embed_config,
                    dry_run,
                },
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
///
/// Uses the unified storage root configured via `[storage]` in forge.config.toml
/// (issue #253), with env-var override and knowledge.store_path fallback.
fn open_forge_storage(
    config: &forge::config::ForgeConfig,
) -> anyhow::Result<forge::runtime::storage::SharedStorage> {
    let storage = forge::runtime::storage::ForgeStorage::open_from_config(
        config.storage.as_ref(),
        None,
        "store.redb",
    )
    .map_err(|e| anyhow::anyhow!("failed to open storage: {}", e))?;
    Ok(Arc::new(storage))
}

fn read_source(file: &Path) -> anyhow::Result<String> {
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

/// Build a [`SkillExecutor`] and return skill capability signatures for compile-time validation.
///
/// When a manifest with `[skills]` is provided, only declared skills are loaded (project-level).
/// Otherwise falls back to scanning `skill_dirs` from config (global-level).
fn build_skill_executor(
    config: &forge::config::ForgeConfig,
    providers: &Arc<forge::llm::registry::ProviderRegistry>,
    tracer: Option<&forge::tracer::Tracer>,
    manifest: Option<&forge::manifest::ProjectManifest>,
    base_dir: Option<&Path>,
) -> (
    Option<Arc<forge::runtime::skill_executor::SkillExecutor>>,
    HashMap<String, forge::types::CapabilitySignature>,
) {
    let skills_cfg = match config.skills.as_ref() {
        Some(cfg) => cfg,
        None => {
            // No [skills] in config — try project manifest alone
            if let (Some(m), Some(bd)) = (manifest, base_dir) {
                if m.skills.is_some() {
                    // Manifest declares skills but no config [skills] section;
                    // use defaults for runtime params.
                    let default_cfg = forge::config::SkillsConfig {
                        skill_dirs: None,
                        timeout_secs: None,
                        max_turns: None,
                    };
                    return build_skill_executor_inner(
                        &default_cfg,
                        providers,
                        tracer,
                        Some(m),
                        Some(bd),
                    );
                }
            }
            return (None, HashMap::new());
        }
    };
    build_skill_executor_inner(skills_cfg, providers, tracer, manifest, base_dir)
}

fn build_skill_executor_inner(
    skills_cfg: &forge::config::SkillsConfig,
    providers: &Arc<forge::llm::registry::ProviderRegistry>,
    tracer: Option<&forge::tracer::Tracer>,
    manifest: Option<&forge::manifest::ProjectManifest>,
    base_dir: Option<&Path>,
) -> (
    Option<Arc<forge::runtime::skill_executor::SkillExecutor>>,
    HashMap<String, forge::types::CapabilitySignature>,
) {
    let mut registry = forge::runtime::skill_registry::SkillRegistry::new();

    // Project-level skill resolution (authoritative when present)
    let has_project_skills = if let (Some(m), Some(bd)) = (manifest, base_dir) {
        if m.skills.is_some() {
            let global_dirs = skills_cfg.skill_dirs_or_default();
            match m.resolve_skills(bd, &global_dirs) {
                Ok(resolved) => {
                    // Verify against lock file
                    let lock_path = bd.join("skills-lock.json");
                    if let Ok(Some(lock)) = forge::skill_lock::SkillLockFile::load(&lock_path) {
                        let mismatched = lock.verify(&resolved);
                        for name in &mismatched {
                            eprintln!(
                                "warning: skill '{}' has changed since lock. Run `npx skills add` to update skills-lock.json.",
                                name
                            );
                        }
                    }

                    // Load each resolved skill
                    for (_, skill_path) in &resolved {
                        match forge::runtime::skill_loader::SkillLoader::parse_skill_md(skill_path)
                        {
                            Ok(skill) => registry.register(skill),
                            Err(e) => {
                                eprintln!("warning: failed to load {}: {}", skill_path.display(), e)
                            }
                        }
                    }
                    true
                }
                Err(e) => {
                    eprintln!("error: {}", e);
                    std::process::exit(1);
                }
            }
        } else {
            false
        }
    } else {
        false
    };

    // Fallback: scan skill_dirs (global-level, only when no project skills)
    if !has_project_skills {
        let dirs = skills_cfg.skill_dirs_or_default();
        let loaded = forge::runtime::skill_loader::SkillLoader::load_from_dirs(&dirs);
        for skill in loaded {
            registry.register(skill);
        }
    }

    let signatures = registry.capability_signatures();
    let shared = Arc::new(Mutex::new(registry));
    let mut executor =
        forge::runtime::skill_executor::SkillExecutor::new(Arc::clone(providers), shared);
    executor.max_turns = skills_cfg.max_turns_or_default();
    executor.default_timeout = std::time::Duration::from_secs(skills_cfg.timeout_or_default());
    if let Some(t) = tracer {
        executor = executor.with_tracer(Arc::new(t.clone()));
    }
    (Some(Arc::new(executor)), signatures)
}

async fn run_program(file: &Path, trace: bool) -> anyhow::Result<()> {
    let source = read_source(file)?;
    let program = parse_or_exit(&source, file);
    let fname = file.display().to_string();

    // Load config and skills early — needed for compile-time skill validation
    let config = forge::config::ForgeConfig::load_or_default();
    let config_clone = config.clone();
    let registry = forge::llm::registry::ProviderRegistry::from_config(config)
        .map_err(|e| anyhow::anyhow!("provider setup failed: {}", e))?;
    let providers = Arc::new(registry);

    let tracer = if trace
        || std::env::var("FORGE_TRACE")
            .map(|v| v == "1")
            .unwrap_or(false)
    {
        Some(forge::tracer::Tracer::new())
    } else {
        None
    };

    // Build skill executor to get capability signatures for compile-time validation
    let (skill_exec, skill_sigs) =
        build_skill_executor(&config_clone, &providers, tracer.as_ref(), None, None);

    // Validate before execution (with skill-aware capability registry)
    let mut diagnostics = Vec::new();

    let ctx = if skill_sigs.is_empty() {
        forge::resolver::CheckContext::new(&fname)
    } else {
        forge::resolver::CheckContext::with_skills(&fname, skill_sigs)
    };
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

    let cmd_mgr = Arc::new(Mutex::new(CommandManager::new()));
    let session_mgr =
        forge::runtime::session_manager::new_shared_default_session_manager(tracer.clone());
    let _ = session_mgr.resume_all().await;
    let mut executor =
        forge::runtime::executor::TaskExecutor::new(program, Arc::clone(&providers), tracer)
            .with_config(config_clone)
            .with_command_manager(cmd_mgr)
            .with_session_manager(session_mgr);
    if let Some(se) = skill_exec {
        executor = executor.with_skill_executor(se);
    }

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

    // Load config and skills early — needed for compile-time skill validation
    let config = forge::config::ForgeConfig::load_or_default();
    let config_clone = config.clone();
    let registry = forge::llm::registry::ProviderRegistry::from_config(config)
        .map_err(|e| anyhow::anyhow!("provider setup failed: {}", e))?;
    let providers = Arc::new(registry);

    let tracer = if trace
        || std::env::var("FORGE_TRACE")
            .map(|v| v == "1")
            .unwrap_or(false)
    {
        Some(forge::tracer::Tracer::new())
    } else {
        None
    };

    // Build skill executor with project-level declarations
    let (skill_exec, skill_sigs) = build_skill_executor(
        &config_clone,
        &providers,
        tracer.as_ref(),
        Some(&manifest),
        Some(base_dir),
    );

    // Parse all source files
    let mut source_files = Vec::new();
    let mut diagnostics = Vec::new();

    for path in &source_paths {
        let source = read_source(&path.to_path_buf())?;
        let fname = path.display().to_string();
        let program = parse_or_exit(&source, path);

        // Per-file validation (with skill-aware capability registry)
        let ctx = if skill_sigs.is_empty() {
            forge::resolver::CheckContext::new(&fname)
        } else {
            forge::resolver::CheckContext::with_skills(&fname, skill_sigs.clone())
        };
        if let Err(errors) = ctx.check(&program) {
            let cap_registry = if skill_sigs.is_empty() {
                forge::resolver::CapabilityRegistry::builtin()
            } else {
                forge::resolver::CapabilityRegistry::with_skills(skill_sigs.clone())
            };
            diagnostics.extend(
                errors
                    .iter()
                    .map(|e| e.to_diagnostic(&fname, &cap_registry)),
            );
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

    let cmd_mgr = Arc::new(Mutex::new(CommandManager::new()));
    let session_mgr =
        forge::runtime::session_manager::new_shared_default_session_manager(tracer.clone());
    let _ = session_mgr.resume_all().await;
    let mut executor = forge::runtime::executor::TaskExecutor::new(
        composed.program,
        Arc::clone(&providers),
        tracer,
    )
    .with_config(config_clone)
    .with_command_manager(cmd_mgr)
    .with_session_manager(session_mgr);
    if let Some(se) = skill_exec {
        executor = executor.with_skill_executor(se);
    }

    match executor.run().await {
        Ok(_) => {}
        Err(e) => {
            eprintln!("runtime error: {}", e);
            std::process::exit(1);
        }
    }

    Ok(())
}

struct BuildProgramOptions<'a> {
    entry: Option<&'a Path>,
    sources: &'a [PathBuf],
    output: Option<&'a str>,
    manifest_path: Option<&'a Path>,
    release: bool,
    embed_config: Option<PathBuf>,
    dry_run: bool,
}

async fn build_program(path: &Path, options: BuildProgramOptions<'_>) -> anyhow::Result<()> {
    // Resolve manifest: explicit path, directory with forge.project.toml, or single file
    let (mut manifest, base_dir) = if let Some(mp) = options.manifest_path {
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

    if options.entry.is_some() || !options.sources.is_empty() {
        let entry = options.entry.ok_or_else(|| {
            anyhow::anyhow!("--entry is required when overriding build sources with --source")
        })?;
        let build = manifest.build.get_or_insert(forge::manifest::BuildConfig {
            entry: None,
            output: None,
            sources: None,
        });
        build.entry = Some(entry.to_string_lossy().to_string());
        build.sources = Some(
            options
                .sources
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect(),
        );
    }

    // Resolve output: if -o is a path with dir, split into dir + name
    let (output_dir, output_name_override) = if let Some(out) = options.output {
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
        .dry_run(options.dry_run)
        .release(options.release)
        .embed_config(options.embed_config)
        .output_dir(output_dir);

    let result = pipeline.build()?;

    if options.dry_run {
        eprintln!(
            "dry run complete — validation passed ({:?})",
            result.program_kind
        );
    } else {
        eprintln!("done: {}", result.binary_path.display());
    }

    Ok(())
}

/// Try to build executor from entry file + optional additional sources.
/// When sources are provided, merges them before building (multi-file project support).
/// Returns executor, config, skill signatures, and skill executor (#276).
fn try_build_executor_multi(
    file: &Path,
    sources: &[PathBuf],
    events_tx: Option<tokio::sync::broadcast::Sender<String>>,
    manifest: Option<&forge::manifest::ProjectManifest>,
    base_dir: Option<&Path>,
) -> Result<BuildResult, anyhow::Error> {
    if sources.is_empty() {
        return try_build_executor(file, events_tx, manifest, base_dir);
    }

    // Multi-file: parse all, merge, then build
    let mut all_paths = vec![file.to_path_buf()];
    all_paths.extend(sources.iter().cloned());

    let mut source_files = Vec::new();

    for path in &all_paths {
        let source = read_source(path)?;
        let fname = path.display().to_string();
        let program = match forge::parser::parse(&source) {
            Ok(p) => p,
            Err(e) => {
                e.to_diagnostic(&fname).render(&source);
                return Err(anyhow::anyhow!("parse error in {}", fname));
            }
        };
        source_files.push(forge::compose::SourceFile {
            path: fname,
            source,
            program,
        });
    }

    // Load config, providers, and skills early — needed for skill-aware validation (#276)
    let config = forge::config::ForgeConfig::load_or_default();
    let trace_env = std::env::var("FORGE_TRACE")
        .map(|v| v == "1")
        .unwrap_or(false);
    let tracer = match (&events_tx, trace_env) {
        (Some(tx), _) => Some(forge::tracer::Tracer::with_live(tx.clone())),
        (None, true) => Some(forge::tracer::Tracer::new()),
        (None, false) => None,
    };

    let registry = forge::llm::registry::ProviderRegistry::from_config(config.clone())
        .map_err(|e| anyhow::anyhow!("provider setup failed: {}", e))?;
    let providers = Arc::new(registry);

    let (skill_exec, skill_sigs) =
        build_skill_executor(&config, &providers, tracer.as_ref(), manifest, base_dir);

    // Per-file resolver validation with skill-aware checker (#276)
    let mut diagnostics = Vec::new();
    for sf in &source_files {
        let ctx = if skill_sigs.is_empty() {
            forge::resolver::CheckContext::new(&sf.path)
        } else {
            forge::resolver::CheckContext::with_skills(&sf.path, skill_sigs.clone())
        };
        if let Err(errors) = ctx.check(&sf.program) {
            let cap_registry = if skill_sigs.is_empty() {
                forge::resolver::CapabilityRegistry::builtin()
            } else {
                forge::resolver::CapabilityRegistry::with_skills(skill_sigs.clone())
            };
            diagnostics.extend(
                errors
                    .iter()
                    .map(|e| e.to_diagnostic(&sf.path, &cap_registry)),
            );
        }
    }

    // Cross-file boundary check (pre-merge, per-file)
    let boundary_refs: Vec<_> = source_files
        .iter()
        .map(|sf| (&sf.program, sf.path.as_str()))
        .collect();
    diagnostics.extend(forge::checker::boundary_checker::check(&boundary_refs));

    // Merge programs before semantic checks so cross-file references
    // (e.g. lifecycle states declared in one file, used in another) resolve (#313)
    let composed = forge::compose::merge_programs(&source_files).map_err(|errs| {
        anyhow::anyhow!(
            "{}",
            errs.iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        )
    })?;

    // Semantic checkers on merged program (states, requires, spawn, etc.)
    let merged_fname = source_files
        .first()
        .map(|sf| sf.path.clone())
        .unwrap_or_default();
    diagnostics.extend(forge::checker::check_all(&composed.program, &merged_fname));

    // Render all diagnostics, but only fail on errors (not warnings)
    if !diagnostics.is_empty() {
        for diag in &diagnostics {
            if let Some(sf) = source_files.iter().find(|sf| sf.path == diag.file) {
                diag.render(&sf.source);
            } else if let Some(sf) = source_files.first() {
                diag.render(&sf.source);
            }
        }
        let error_count = diagnostics
            .iter()
            .filter(|d| d.kind == forge::diagnostic::DiagnosticKind::Error)
            .count();
        if error_count > 0 {
            return Err(anyhow::anyhow!("{} diagnostic error(s)", error_count));
        }
    }

    let cmd_mgr = Arc::new(Mutex::new(CommandManager::new()));
    let session_mgr =
        forge::runtime::session_manager::new_shared_default_session_manager(tracer.clone());
    let mut executor = forge::runtime::executor::TaskExecutor::new(
        composed.program,
        Arc::clone(&providers),
        tracer,
    )
    .with_config(config.clone())
    .with_command_manager(cmd_mgr)
    .with_session_manager(session_mgr);
    if let Some(ref se) = skill_exec {
        executor = executor.with_skill_executor(se.clone());
    }

    Ok((executor, config, skill_sigs, skill_exec))
}

/// Try to parse, validate, and build an executor from a single .forge file.
/// Returns the executor, config, skill signatures, and skill executor on success,
/// or renders diagnostics and returns an error.
fn try_build_executor(
    file: &Path,
    events_tx: Option<tokio::sync::broadcast::Sender<String>>,
    manifest: Option<&forge::manifest::ProjectManifest>,
    base_dir: Option<&Path>,
) -> Result<BuildResult, anyhow::Error> {
    let source = read_source(file)?;
    let fname = file.display().to_string();

    let program = match forge::parser::parse(&source) {
        Ok(p) => p,
        Err(e) => {
            e.to_diagnostic(&fname).render(&source);
            return Err(anyhow::anyhow!("parse error in {}", fname));
        }
    };

    // Load config, providers, and skills early — needed for skill-aware validation (#276)
    let config = forge::config::ForgeConfig::load_or_default();

    let trace_env = std::env::var("FORGE_TRACE")
        .map(|v| v == "1")
        .unwrap_or(false);
    let tracer = match (&events_tx, trace_env) {
        (Some(tx), _) => Some(forge::tracer::Tracer::with_live(tx.clone())),
        (None, true) => Some(forge::tracer::Tracer::new()),
        (None, false) => None,
    };

    let registry = forge::llm::registry::ProviderRegistry::from_config(config.clone())
        .map_err(|e| anyhow::anyhow!("provider setup failed: {}", e))?;
    let providers = Arc::new(registry);

    let (skill_exec, skill_sigs) =
        build_skill_executor(&config, &providers, tracer.as_ref(), manifest, base_dir);

    // Validate with skill-aware capability registry (#276)
    let mut diagnostics = Vec::new();

    let ctx = if skill_sigs.is_empty() {
        forge::resolver::CheckContext::new(&fname)
    } else {
        forge::resolver::CheckContext::with_skills(&fname, skill_sigs.clone())
    };
    if let Err(errors) = ctx.check(&program) {
        let cap_registry = if skill_sigs.is_empty() {
            forge::resolver::CapabilityRegistry::builtin()
        } else {
            forge::resolver::CapabilityRegistry::with_skills(skill_sigs.clone())
        };
        diagnostics.extend(
            errors
                .iter()
                .map(|e| e.to_diagnostic(&fname, &cap_registry)),
        );
    }

    diagnostics.extend(forge::checker::check_all(&program, &fname));

    let boundary_refs = vec![(&program, fname.as_str())];
    diagnostics.extend(forge::checker::boundary_checker::check(&boundary_refs));

    // Render all diagnostics, but only fail on errors (not warnings).
    // Matches try_build_executor_multi behavior — warnings should never
    // block `forge serve` from starting.
    if !diagnostics.is_empty() {
        forge::diagnostic::render_diagnostics(&source, &diagnostics);
        let error_count = diagnostics
            .iter()
            .filter(|d| d.kind == forge::diagnostic::DiagnosticKind::Error)
            .count();
        if error_count > 0 {
            return Err(anyhow::anyhow!("{} diagnostic error(s)", error_count));
        }
    }

    let cmd_mgr = Arc::new(Mutex::new(CommandManager::new()));
    let session_mgr =
        forge::runtime::session_manager::new_shared_default_session_manager(tracer.clone());
    let mut executor =
        forge::runtime::executor::TaskExecutor::new(program, Arc::clone(&providers), tracer)
            .with_config(config.clone())
            .with_command_manager(cmd_mgr)
            .with_session_manager(session_mgr);
    if let Some(ref se) = skill_exec {
        executor = executor.with_skill_executor(se.clone());
    }

    Ok((executor, config, skill_sigs, skill_exec))
}

async fn serve_program(
    file: &Path,
    sources: &[PathBuf],
    cli_host: Option<String>,
    cli_port: Option<u16>,
    watch: bool,
    manifest_path: Option<&Path>,
) -> anyhow::Result<()> {
    // Auto-discover forge.config.toml next to the served file when FORGE_CONFIG is not set.
    if std::env::var("FORGE_CONFIG").is_err() {
        if let Some(parent) = file.parent() {
            let local_config = parent.join("forge.config.toml");
            if local_config.exists() {
                std::env::set_var("FORGE_CONFIG", &local_config);
            }
        }
    }

    // Resolve project manifest: explicit --manifest flag, or auto-discover next to the served file.
    let (manifest, base_dir) = if let Some(mp) = manifest_path {
        let m = forge::manifest::ProjectManifest::load(mp)?;
        let bd = mp.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
        (Some(m), Some(bd))
    } else if let Some(parent) = file.parent() {
        let candidate = parent.join("forge.project.toml");
        if candidate.exists() {
            let m = forge::manifest::ProjectManifest::load(&candidate)?;
            (Some(m), Some(parent.to_path_buf()))
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    if watch {
        serve_with_watch(
            file,
            sources,
            cli_host,
            cli_port,
            manifest.as_ref(),
            base_dir.as_deref(),
        )
        .await
    } else {
        // Non-watch mode: build once and serve. Exits on errors.
        let (events_tx, _) = tokio::sync::broadcast::channel::<String>(256);
        let (executor, config, _skill_sigs, _skill_exec) = match try_build_executor_multi(
            file,
            sources,
            Some(events_tx.clone()),
            manifest.as_ref(),
            base_dir.as_deref(),
        ) {
            Ok(r) => r,
            Err(_) => std::process::exit(1),
        };

        // Create shared infrastructure for both HTTP server and system runtime (#140)
        let event_bus = forge::runtime::event_bus::EventBus::new_shared(None);
        let instance_registry: forge::runtime::instance_registry::SharedInstanceRegistry = Arc::new(
            tokio::sync::RwLock::new(forge::runtime::instance_registry::InstanceRegistry::new()),
        );
        let warden_snapshots: forge::runtime::warded::SharedWardenSnapshots =
            Arc::new(tokio::sync::RwLock::new(Vec::new()));

        // Create storage for data.store/data.get/data.list/data.delete (#59)
        // Unified root via [storage] in forge.config.toml (#253).
        let mut inspect_storage: Option<forge::runtime::storage::SharedStorage> = None;
        let executor = match forge::runtime::storage::ForgeStorage::open_from_config(
            config.storage.as_ref(),
            None,
            "server.redb",
        ) {
            Ok(storage) => {
                seed_content_dir(file, &storage);
                let shared = std::sync::Arc::new(storage);
                inspect_storage = Some(shared.clone());
                executor.with_storage(shared)
            }
            Err(e) => {
                eprintln!("Warning: could not open storage: {e}");
                executor
            }
        };

        // Build embedding provider if [embeddings] is configured (#50)
        let executor = if let Some(ref embed_config) = config.embeddings {
            match config.providers.get(&embed_config.provider) {
                Some(provider_config) => {
                    match forge::llm::providers::build_embedding_provider(
                        provider_config,
                        embed_config,
                    ) {
                        Ok(embed_provider) => {
                            let dimensions = embed_provider.embedding_dimensions();
                            let vectors_path = file
                                .parent()
                                .unwrap_or(std::path::Path::new("."))
                                .join(".forge-data/vectors.json");
                            let vector_index = std::sync::Arc::new(tokio::sync::Mutex::new(
                                forge::runtime::vector_index::VectorIndex::new(
                                    dimensions,
                                    Some(&vectors_path),
                                ),
                            ));
                            executor.with_embeddings(embed_provider, vector_index)
                        }
                        Err(e) => {
                            eprintln!("Warning: could not build embedding provider: {e}");
                            executor
                        }
                    }
                }
                None => {
                    eprintln!(
                        "Warning: [embeddings] references unknown provider '{}'",
                        embed_config.provider
                    );
                    executor
                }
            }
        } else {
            executor
        };

        // Create shared knowledge store from agent declaration (#309).
        // The same Arc is passed to both the executor (for endpoint recall) and
        // the system runtime (for agent learn), ensuring a single source of truth.
        let executor = if let Some((store_path, max_entries, retention_days)) =
            extract_knowledge_config(executor.program())
        {
            let ks = KnowledgeStore::new(&store_path, max_entries, retention_days);
            let shared_ks = Arc::new(Mutex::new(ks));
            executor.with_shared_knowledge_store_arc(shared_ks)
        } else {
            executor
        };

        // Build system runtime (if declared) and inject shared infrastructure (#140)
        let topology = executor.extract_topology();
        let system_runtime = match executor.build_system_runtime() {
            Ok(Some(sr)) => {
                let mut sr = sr
                    .with_shared_infrastructure(event_bus.clone(), instance_registry.clone())
                    .with_shared_warden_snapshots(warden_snapshots.clone());
                if let Some(ref storage) = inspect_storage {
                    sr = sr.with_shared_storage(storage.clone());
                }
                Some(sr)
            }
            Ok(None) => None,
            Err(e) => {
                eprintln!("Warning: failed to build system runtime: {e}");
                None
            }
        };

        // Collect signal senders before system runtime is consumed (issue #143)
        let signal_senders = system_runtime
            .as_ref()
            .map(|sr| sr.collect_signal_senders());

        // Cost aggregator (issue #142) — subscribe before server consumes events_tx
        let cost_aggregator = Arc::new(tokio::sync::RwLock::new(
            forge::runtime::cost_aggregator::CostAggregator::new(),
        ));
        forge::runtime::cost_aggregator::spawn_cost_listener(
            events_tx.subscribe(),
            cost_aggregator.clone(),
        );

        // Task-history aggregator (issue #304) — subscribe to `TaskCompleted`
        // on the event bus so the mastery tile has per-task `review_rounds`
        // data. Grab the knowledge-store handle here too, before the executor
        // is moved into the server, so the endpoint can read mastery snapshots.
        let mastery_knowledge_store = executor.knowledge_store_handle();
        let task_history_aggregator = Arc::new(tokio::sync::RwLock::new(
            forge::runtime::task_history_aggregator::TaskHistoryAggregator::default(),
        ));
        forge::runtime::task_history_aggregator::spawn_task_listener(
            event_bus.clone(),
            task_history_aggregator.clone(),
        )
        .await;

        let mut server =
            forge::runtime::http_server::ForgeServer::new(executor, config.server.as_ref())
                .with_event_bus(event_bus)
                .with_events_tx(events_tx)
                .with_instance_registry(instance_registry)
                .with_warden_snapshots(warden_snapshots)
                .with_cost_aggregator(cost_aggregator)
                .with_task_history_aggregator(task_history_aggregator);

        if let Some(ks) = mastery_knowledge_store {
            server = server.with_mastery_knowledge_store(ks);
        }
        if let Some(senders) = signal_senders {
            server = server.with_signal_senders(senders);
        }
        if let Some(storage) = inspect_storage {
            server = server.with_inspect_storage(storage);
        }
        if let Some(topo) = topology {
            server = server.with_topology(topo);
        }

        // Wire webhook secrets from config
        if let Some(ref srv_config) = config.server {
            if let Some(ref secrets) = srv_config.webhook_secrets {
                server = server.with_webhook_secrets(secrets.clone());
            }
        }

        if let Some(host) = cli_host {
            server = server.with_host(host);
        }
        if let Some(port) = cli_port {
            server = server.with_port(port);
        }

        // Spawn system runtime as background task (#140)
        if let Some(sr) = system_runtime {
            tokio::spawn(async move {
                if let Err(e) = sr.start().await {
                    eprintln!("System runtime error: {e}");
                }
            });
        }

        server.run().await
    }
}

/// Seed markdown files from `content/` directory into storage.
/// Files are stored as `page:<slug>` keys (e.g., `content/getting-started.md` → `page:getting-started`).
/// Subdirectories use the filename only (e.g., `content/reference/task.md` → `page:task`).
/// Extract knowledge store config from the first agent declaration in the program.
/// Returns (store_path, max_entries, retention_days) if found.
fn extract_knowledge_config(
    program: &forge::ast::Program,
) -> Option<(String, Option<usize>, Option<u64>)> {
    program
        .items
        .iter()
        .find_map(|item| match &item.node {
            TopLevel::Agent(agent) => agent.knowledge.as_ref(),
            _ => None,
        })
        .and_then(|kd| {
            let store_path = match &kd.node.store_path.node {
                Expr::Template(parts) => parts
                    .iter()
                    .filter_map(|p| match &p.node {
                        TemplatePart::Text(t) => Some(t.as_str()),
                        _ => None,
                    })
                    .collect::<String>(),
                _ => return None,
            };
            let max_entries = kd.node.max_entries.as_ref().map(|m| m.node as usize);
            let retention_days = kd.node.retention.as_ref().map(|r| {
                let dur = &r.node;
                match dur.unit {
                    forge::ast::DurationUnit::Days => dur.value,
                    forge::ast::DurationUnit::Hours => dur.value / 24,
                    forge::ast::DurationUnit::Minutes => dur.value / (24 * 60),
                    forge::ast::DurationUnit::Seconds => dur.value / (24 * 60 * 60),
                }
            });
            Some((store_path, max_entries, retention_days))
        })
}

fn seed_content_dir(file: &Path, storage: &forge::runtime::storage::ForgeStorage) {
    let content_dir = file
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("content");
    if !content_dir.is_dir() {
        return;
    }
    let mut count = 0;
    seed_content_recursive(&content_dir, storage, &mut count);
    if count > 0 {
        eprintln!(
            "  Seeded {count} content pages from {}",
            content_dir.display()
        );
    }
}

fn seed_content_recursive(
    dir: &Path,
    storage: &forge::runtime::storage::ForgeStorage,
    count: &mut usize,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            seed_content_recursive(&path, storage, count);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let slug = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if slug.is_empty() {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                let key = format!("page:{slug}");
                if storage.store(&key, &content).is_ok() {
                    *count += 1;
                }
            }
        }
    }
}

async fn serve_with_watch(
    file: &Path,
    sources: &[PathBuf],
    cli_host: Option<String>,
    cli_port: Option<u16>,
    manifest: Option<&forge::manifest::ProjectManifest>,
    base_dir: Option<&Path>,
) -> anyhow::Result<()> {
    use forge::runtime::watcher::WatchAction;

    // Create events channel outside the loop so SSE connections survive config restarts.
    let (events_tx, _) = tokio::sync::broadcast::channel::<String>(256);

    loop {
        let (executor, config, skill_sigs, skill_exec) = match try_build_executor_multi(
            file,
            sources,
            Some(events_tx.clone()),
            manifest,
            base_dir,
        ) {
            Ok(r) => r,
            Err(_) => std::process::exit(1),
        };

        // Create shared infrastructure for both HTTP server and system runtime (#140)
        let event_bus = forge::runtime::event_bus::EventBus::new_shared(None);
        let instance_registry: forge::runtime::instance_registry::SharedInstanceRegistry = Arc::new(
            tokio::sync::RwLock::new(forge::runtime::instance_registry::InstanceRegistry::new()),
        );
        let warden_snapshots: forge::runtime::warded::SharedWardenSnapshots =
            Arc::new(tokio::sync::RwLock::new(Vec::new()));

        // Create storage for data.store/data.get/data.list/data.delete (#59)
        // Unified root via [storage] in forge.config.toml (#253).
        let mut inspect_storage: Option<forge::runtime::storage::SharedStorage> = None;
        let executor = match forge::runtime::storage::ForgeStorage::open_from_config(
            config.storage.as_ref(),
            None,
            "server.redb",
        ) {
            Ok(storage) => {
                seed_content_dir(file, &storage);
                let shared = std::sync::Arc::new(storage);
                inspect_storage = Some(shared.clone());
                executor.with_storage(shared)
            }
            Err(e) => {
                eprintln!("Warning: could not open storage: {e}");
                executor
            }
        };

        // Build embedding provider if [embeddings] is configured (#50)
        let executor = if let Some(ref embed_config) = config.embeddings {
            match config.providers.get(&embed_config.provider) {
                Some(provider_config) => {
                    match forge::llm::providers::build_embedding_provider(
                        provider_config,
                        embed_config,
                    ) {
                        Ok(embed_provider) => {
                            let dimensions = embed_provider.embedding_dimensions();
                            let vectors_path = file
                                .parent()
                                .unwrap_or(std::path::Path::new("."))
                                .join(".forge-data/vectors.json");
                            let vector_index = std::sync::Arc::new(tokio::sync::Mutex::new(
                                forge::runtime::vector_index::VectorIndex::new(
                                    dimensions,
                                    Some(&vectors_path),
                                ),
                            ));
                            executor.with_embeddings(embed_provider, vector_index)
                        }
                        Err(e) => {
                            eprintln!("Warning: could not build embedding provider: {e}");
                            executor
                        }
                    }
                }
                None => {
                    eprintln!(
                        "Warning: [embeddings] references unknown provider '{}'",
                        embed_config.provider
                    );
                    executor
                }
            }
        } else {
            executor
        };

        // Create shared knowledge store from agent declaration (#309).
        let executor = if let Some((store_path, max_entries, retention_days)) =
            extract_knowledge_config(executor.program())
        {
            let ks = KnowledgeStore::new(&store_path, max_entries, retention_days);
            let shared_ks = Arc::new(Mutex::new(ks));
            executor.with_shared_knowledge_store_arc(shared_ks)
        } else {
            executor
        };

        // Build system runtime (if declared) and inject shared infrastructure (#140)
        let topology = executor.extract_topology();
        let system_runtime = match executor.build_system_runtime() {
            Ok(Some(sr)) => {
                let mut sr = sr
                    .with_shared_infrastructure(event_bus.clone(), instance_registry.clone())
                    .with_shared_warden_snapshots(warden_snapshots.clone());
                if let Some(ref storage) = inspect_storage {
                    sr = sr.with_shared_storage(storage.clone());
                }
                Some(sr)
            }
            Ok(None) => None,
            Err(e) => {
                eprintln!("Warning: failed to build system runtime: {e}");
                None
            }
        };

        // Collect signal senders before system runtime is consumed (issue #143)
        let signal_senders = system_runtime
            .as_ref()
            .map(|sr| sr.collect_signal_senders());

        // Cost aggregator (issue #142)
        let cost_aggregator = Arc::new(tokio::sync::RwLock::new(
            forge::runtime::cost_aggregator::CostAggregator::new(),
        ));
        forge::runtime::cost_aggregator::spawn_cost_listener(
            events_tx.subscribe(),
            cost_aggregator.clone(),
        );

        // Task-history aggregator (issue #304) — see matching block above.
        let mastery_knowledge_store = executor.knowledge_store_handle();
        let task_history_aggregator = Arc::new(tokio::sync::RwLock::new(
            forge::runtime::task_history_aggregator::TaskHistoryAggregator::default(),
        ));
        forge::runtime::task_history_aggregator::spawn_task_listener(
            event_bus.clone(),
            task_history_aggregator.clone(),
        )
        .await;

        let mut server =
            forge::runtime::http_server::ForgeServer::new(executor, config.server.as_ref())
                .with_watch_mode(true)
                .with_event_bus(event_bus)
                .with_events_tx(events_tx.clone())
                .with_instance_registry(instance_registry)
                .with_warden_snapshots(warden_snapshots)
                .with_cost_aggregator(cost_aggregator)
                .with_task_history_aggregator(task_history_aggregator);

        if let Some(ks) = mastery_knowledge_store {
            server = server.with_mastery_knowledge_store(ks);
        }

        if let Some(senders) = signal_senders {
            server = server.with_signal_senders(senders);
        }
        if let Some(storage) = inspect_storage {
            server = server.with_inspect_storage(storage);
        }
        if let Some(topo) = topology {
            server = server.with_topology(topo);
        }

        // Wire webhook secrets from config
        if let Some(ref srv_config) = config.server {
            if let Some(ref secrets) = srv_config.webhook_secrets {
                server = server.with_webhook_secrets(secrets.clone());
            }
        }

        if let Some(ref host) = cli_host {
            server = server.with_host(host.clone());
        }
        if let Some(port) = cli_port {
            server = server.with_port(port);
        }

        // Spawn system runtime as background task (#140)
        if let Some(sr) = system_runtime {
            tokio::spawn(async move {
                if let Err(e) = sr.start().await {
                    eprintln!("System runtime error: {e}");
                }
            });
        }

        let swappable = server.swappable_executor();
        let reload_tx = server.reload_sender();
        let watcher_events_tx = Some(events_tx.clone());

        let watch_file = file.to_path_buf();
        let watcher_handle = tokio::spawn(async move {
            forge::runtime::watcher::watch_and_reload(
                watch_file,
                swappable,
                reload_tx,
                watcher_events_tx,
                skill_sigs,
                skill_exec,
            )
            .await
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

async fn run_agent(file: &Path) -> anyhow::Result<()> {
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
    let storage = if agent_decl.memory_persistent {
        Some(open_forge_storage(&config)?)
    } else {
        None
    };
    let registry = forge::llm::registry::ProviderRegistry::from_config(config)
        .map_err(|e| anyhow::anyhow!("provider setup failed: {}", e))?;

    let agent = AgentProcess::new(
        agent_decl.clone(),
        states_decl.as_ref(),
        Arc::new(registry),
        None,
        program,
        storage,
        None,
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
async fn send_to_agent(file: &Path, event: &str, args: Vec<String>) -> anyhow::Result<()> {
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
    // Always open storage in CLI mode — even non-persistent agents need
    // memory to survive across forge-send invocations
    let storage = Some(open_forge_storage(&config)?);
    let registry = forge::llm::registry::ProviderRegistry::from_config(config)
        .map_err(|e| anyhow::anyhow!("provider setup failed: {}", e))?;

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
        None,
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
