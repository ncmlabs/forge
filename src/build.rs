// FORGE build pipeline — package .forge programs as standalone CLI binaries
// See issue #74 for specification

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::compose::{self, ProgramKind, SourceFile};
use crate::manifest::ProjectManifest;

// ── Build pipeline ────────────────────────────────────────────

pub struct BuildPipeline {
    manifest: ProjectManifest,
    base_dir: PathBuf,
    dry_run: bool,
    output_dir: Option<PathBuf>,
    embed_config: Option<PathBuf>,
    release: bool,
}

/// Result of a successful build.
pub struct BuildResult {
    pub binary_path: PathBuf,
    pub program_kind: ProgramKind,
}

/// Result of a dry run (validation only).
pub struct DryRunResult {
    pub program_kind: ProgramKind,
    pub source_count: usize,
    pub symbol_count: usize,
}

impl BuildPipeline {
    pub fn new(manifest: ProjectManifest, base_dir: PathBuf) -> Self {
        Self {
            manifest,
            base_dir,
            dry_run: false,
            output_dir: None,
            embed_config: None,
            release: true,
        }
    }

    pub fn dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }

    pub fn output_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.output_dir = dir;
        self
    }

    pub fn embed_config(mut self, path: Option<PathBuf>) -> Self {
        self.embed_config = path;
        self
    }

    pub fn release(mut self, release: bool) -> Self {
        self.release = release;
        self
    }

    /// Run the full build pipeline.
    pub fn build(&self) -> anyhow::Result<BuildResult> {
        // Step 1: Resolve source files
        let source_paths = self.manifest.resolve_sources(&self.base_dir)?;
        eprintln!("  sources: {} file(s)", source_paths.len());

        // Step 2: Parse all source files
        let mut source_files = Vec::new();
        let mut source_texts = Vec::new();
        for path in &source_paths {
            let source = std::fs::read_to_string(path)
                .map_err(|e| anyhow::anyhow!("cannot read {}: {}", path.display(), e))?;
            let fname = path.display().to_string();
            let program = crate::parser::parse(&source).map_err(|e| {
                let diag = e.to_diagnostic(&fname);
                diag.render(&source);
                anyhow::anyhow!("parse error in {}", fname)
            })?;
            source_texts.push((fname.clone(), source.clone()));
            source_files.push(SourceFile {
                path: fname,
                source,
                program,
            });
        }

        // Step 3: Per-file validation
        let mut diagnostics = Vec::new();
        for sf in &source_files {
            let ctx = crate::resolver::CheckContext::new(&sf.path);
            if let Err(errors) = ctx.check(&sf.program) {
                let registry = crate::resolver::CapabilityRegistry::builtin();
                diagnostics.extend(errors.iter().map(|e| e.to_diagnostic(&sf.path, &registry)));
            }
            diagnostics.extend(crate::checker::check_all(&sf.program, &sf.path));
        }

        // Step 4: Cross-file boundary check (on pre-merge programs)
        let boundary_refs: Vec<_> = source_files
            .iter()
            .map(|sf| (&sf.program, sf.path.as_str()))
            .collect();
        diagnostics.extend(crate::checker::boundary_checker::check(&boundary_refs));

        // Render all diagnostics, but only fail on errors
        if !diagnostics.is_empty() {
            for diag in &diagnostics {
                if let Some(sf) = source_files.iter().find(|sf| sf.path == diag.file) {
                    diag.render(&sf.source);
                }
            }
            let error_count = diagnostics
                .iter()
                .filter(|d| d.kind == crate::diagnostic::DiagnosticKind::Error)
                .count();
            if error_count > 0 {
                anyhow::bail!("validation failed with {} error(s)", error_count);
            }
        }

        // Step 5: Merge programs
        let composed = compose::merge_programs(&source_files).map_err(|errs| {
            anyhow::anyhow!(
                "composition failed:\n{}",
                errs.iter()
                    .map(|e| format!("  {}", e))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        })?;

        // Step 6: Detect program kind
        let kind = compose::detect_kind(&composed.program).ok_or_else(|| {
            anyhow::anyhow!("no entry point found (fn main, agent, endpoint, or system)")
        })?;

        eprintln!("  kind: {:?}", kind);

        if kind == ProgramKind::System {
            anyhow::bail!(
                "system binaries are not yet supported — use `forge run` to execute system programs"
            );
        }

        // Dry run: stop here
        if self.dry_run {
            let symbol_count = composed
                .program
                .items
                .iter()
                .filter(|i| compose::top_level_has_name(&i.node))
                .count();
            eprintln!("  dry run: validation passed");
            eprintln!("  symbols: {}", symbol_count);
            // Return a dummy result — caller checks dry_run flag
            return Ok(BuildResult {
                binary_path: PathBuf::from("<dry-run>"),
                program_kind: kind,
            });
        }

        // Step 7: Resolve embedded config
        let embed_config_content = self.resolve_embed_config()?;

        // Step 8: Generate temporary Rust crate
        let crate_dir =
            self.generate_crate(&source_paths, kind, embed_config_content.as_deref())?;
        eprintln!("  crate: {}", crate_dir.display());

        // Step 9: cargo build
        let binary_path = self.cargo_build(&crate_dir, kind)?;
        eprintln!("  binary: {}", binary_path.display());

        // Step 10: Clean up temp crate
        let _ = std::fs::remove_dir_all(&crate_dir);

        Ok(BuildResult {
            binary_path,
            program_kind: kind,
        })
    }

    fn resolve_embed_config(&self) -> anyhow::Result<Option<String>> {
        // CLI flag takes precedence over manifest
        let config_path = self.embed_config.clone().or_else(|| {
            self.manifest
                .resolve_embedded_config(&self.base_dir)
                .ok()
                .flatten()
        });

        if let Some(path) = config_path {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| anyhow::anyhow!("cannot read config {}: {}", path.display(), e))?;
            Ok(Some(content))
        } else {
            Ok(None)
        }
    }

    fn generate_crate(
        &self,
        source_paths: &[PathBuf],
        kind: ProgramKind,
        embed_config: Option<&str>,
    ) -> anyhow::Result<PathBuf> {
        // Create temp directory
        let crate_dir = std::env::temp_dir().join(format!("forge_build_{}", std::process::id()));
        let src_dir = crate_dir.join("src");
        let sources_dir = crate_dir.join("sources");
        std::fs::create_dir_all(&src_dir)?;
        std::fs::create_dir_all(&sources_dir)?;

        // Copy .forge source files into sources/
        let mut source_entries = Vec::new();
        for path in source_paths {
            let filename = path
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("invalid source path: {}", path.display()))?
                .to_string_lossy()
                .to_string();
            std::fs::copy(path, sources_dir.join(&filename))?;
            source_entries.push(format!(
                "    (\"{}\", include_str!(\"../sources/{}\"))",
                filename, filename
            ));
        }

        // Generate Cargo.toml
        let forge_dep = forge_dependency_spec();
        let cargo_toml = format!(
            r#"[package]
name = "{name}"
version = "{version}"
edition = "2021"

[dependencies]
forge = {forge_dep}
tokio = {{ version = "1", features = ["full"] }}
clap = {{ version = "4", features = ["derive"] }}
anyhow = "1"
dirs = "6"
"#,
            name = self.manifest.project.name,
            version = self.manifest.project.version.as_deref().unwrap_or("0.1.0"),
            forge_dep = forge_dep,
        );
        std::fs::write(crate_dir.join("Cargo.toml"), cargo_toml)?;

        // Generate src/main.rs based on program kind
        let main_rs = match kind {
            ProgramKind::Executable => generate_executable_main(&source_entries, embed_config),
            ProgramKind::AgentCli => {
                // Need to read the agent info for codegen
                let agent_info = self.extract_agent_info(source_paths)?;
                generate_agent_cli_main(&source_entries, embed_config, &agent_info)
            }
            ProgramKind::Server => generate_server_main(&source_entries, embed_config),
            ProgramKind::System => unreachable!("system kind rejected earlier"),
        };
        std::fs::write(src_dir.join("main.rs"), main_rs)?;

        Ok(crate_dir)
    }

    fn extract_agent_info(&self, source_paths: &[PathBuf]) -> anyhow::Result<AgentInfo> {
        // Parse all files and find the agent declaration
        for path in source_paths {
            let source = std::fs::read_to_string(path)?;
            let program = crate::parser::parse(&source).map_err(|e| {
                anyhow::anyhow!(
                    "parse error: {}",
                    e.to_diagnostic(&path.display().to_string()).message
                )
            })?;
            if let Some(agent) = compose::find_agent(&program, None) {
                let handlers: Vec<HandlerInfo> = agent
                    .handlers
                    .iter()
                    .map(|h| {
                        let params: Vec<(String, String)> = h
                            .node
                            .params
                            .iter()
                            .map(|p| (p.node.name.clone(), format!("{:?}", p.node.type_name.node)))
                            .collect();
                        HandlerInfo {
                            event: h.node.event.node.clone(),
                            params,
                        }
                    })
                    .collect();

                return Ok(AgentInfo {
                    name: agent.name.node.clone(),
                    handlers,
                });
            }
        }
        anyhow::bail!("no agent declaration found in source files")
    }

    fn cargo_build(&self, crate_dir: &Path, _kind: ProgramKind) -> anyhow::Result<PathBuf> {
        let mut cmd = Command::new("cargo");
        cmd.arg("build");
        if self.release {
            cmd.arg("--release");
        }
        cmd.current_dir(crate_dir);

        eprintln!("  compiling...");
        let output = cmd
            .output()
            .map_err(|e| anyhow::anyhow!("failed to run cargo: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("cargo build failed:\n{}", stderr);
        }

        // Find the built binary (cargo names it after [package].name)
        let profile = if self.release { "release" } else { "debug" };
        let crate_name = &self.manifest.project.name;
        let built_binary = crate_dir.join("target").join(profile).join(crate_name);

        if !built_binary.exists() {
            anyhow::bail!(
                "expected binary at {} but it doesn't exist",
                built_binary.display()
            );
        }

        // Copy to output location (using manifest output name, which may differ from crate name)
        let output_name = self.manifest.output_name();
        let output_path = self
            .output_dir
            .as_ref()
            .map(|d| d.join(output_name))
            .unwrap_or_else(|| PathBuf::from(output_name));

        // Create output directory if it doesn't exist
        if let Some(parent) = output_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        std::fs::copy(&built_binary, &output_path)?;

        // Make executable on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&output_path)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&output_path, perms)?;
        }

        Ok(output_path)
    }
}

// ── Agent codegen info ────────────────────────────────────────

struct AgentInfo {
    name: String,
    handlers: Vec<HandlerInfo>,
}

struct HandlerInfo {
    event: String,
    params: Vec<(String, String)>, // (name, type_debug_string)
}

fn to_pascal_case(s: &str) -> String {
    s.split(['_', '.'])
        .map(|part| {
            let mut c = part.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect()
}

// ── Forge dependency resolution ───────────────────────────────

fn forge_dependency_spec() -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let cargo_toml = Path::new(manifest_dir).join("Cargo.toml");
    if cargo_toml.exists() {
        format!("{{ path = \"{}\" }}", manifest_dir)
    } else {
        format!("\"{}\"", env!("CARGO_PKG_VERSION"))
    }
}

// ── Template: config resolution ───────────────────────────────

fn config_resolution_code(embed_config: Option<&str>) -> String {
    // For embedded config, we use r##"..."## (double-hash) to avoid conflicts
    // with TOML content that may contain `"#`
    let embedded_block = if let Some(config_toml) = embed_config {
        let mut s = String::from("const EMBEDDED_CONFIG: Option<&str> = Some(r##\"");
        s.push_str(config_toml);
        s.push_str("\"##);\n");
        s
    } else {
        "const EMBEDDED_CONFIG: Option<&str> = None;\n".to_string()
    };

    let mut code = embedded_block;
    code.push_str(
        r#"
fn resolve_config() -> forge::config::ForgeConfig {
    // Standard load checks FORGE_CONFIG env, ./forge.config.toml, ~/.forge/config.toml
    let config = forge::config::ForgeConfig::load_or_default();

    // If standard load found a real config, use it
    if std::env::var("FORGE_CONFIG").is_ok()
        || std::path::Path::new("forge.config.toml").exists()
        || dirs::home_dir()
            .map(|h| h.join(".forge/config.toml").exists())
            .unwrap_or(false)
    {
        return config;
    }

    // Fall back to embedded config
    if let Some(toml_str) = EMBEDDED_CONFIG {
        if let Ok(cfg) = forge::config::ForgeConfig::from_toml_str(toml_str) {
            return cfg;
        }
    }

    config
}
"#,
    );
    code
}

// ── Template: Executable ──────────────────────────────────────

fn generate_executable_main(source_entries: &[String], embed_config: Option<&str>) -> String {
    let sources_array = source_entries.join(",\n    ");
    let config_code = config_resolution_code(embed_config);

    format!(
        r#"use std::sync::Arc;

const SOURCES: &[(&str, &str)] = &[
{sources}
];

{config}

#[tokio::main]
async fn main() -> anyhow::Result<()> {{
    let program = forge::compose::parse_and_merge_sources(SOURCES)
        .map_err(|e| anyhow::anyhow!("{{}}", e))?;
    let config = resolve_config();
    let registry = forge::llm::registry::ProviderRegistry::from_config(config)
        .map_err(|e| anyhow::anyhow!("provider setup failed: {{}}", e))?;
    let cmd_mgr = Arc::new(std::sync::Mutex::new(forge::runtime::command_manager::CommandManager::new()));
    let session_mgr = forge::runtime::session_manager::new_shared_default_session_manager(None);
    let _ = session_mgr.resume_all().await;
    let executor = forge::runtime::executor::TaskExecutor::new(
        program, Arc::new(registry), None,
    ).with_command_manager(cmd_mgr).with_session_manager(session_mgr);
    match executor.run().await {{
        Ok(_) => Ok(()),
        Err(e) => {{
            eprintln!("runtime error: {{}}", e);
            std::process::exit(1);
        }}
    }}
}}
"#,
        sources = sources_array,
        config = config_code,
    )
}

// ── Template: Agent CLI ───────────────────────────────────────

fn generate_agent_cli_main(
    source_entries: &[String],
    embed_config: Option<&str>,
    agent: &AgentInfo,
) -> String {
    let sources_array = source_entries.join(",\n    ");
    let config_code = config_resolution_code(embed_config);

    // Generate subcommand variants
    let mut variants = Vec::new();
    let mut match_arms = Vec::new();

    for handler in &agent.handlers {
        let variant = to_pascal_case(&handler.event);

        if handler.params.is_empty() {
            variants.push(format!(
                "    /// Handler: {event}\n    {variant},",
                event = handler.event,
                variant = variant,
            ));
            match_arms.push(format!(
                r#"        Command::{variant} => {{
            let params = std::collections::HashMap::new();
            dispatch(&agent, "{event}", params).await?;
        }}"#,
                variant = variant,
                event = handler.event,
            ));
        } else {
            let fields: Vec<String> = handler
                .params
                .iter()
                .map(|(name, _)| format!("        {}: String,", name))
                .collect();
            variants.push(format!(
                "    /// Handler: {event}\n    {variant} {{\n{fields}\n    }},",
                event = handler.event,
                variant = variant,
                fields = fields.join("\n"),
            ));

            let param_inserts: Vec<String> = handler
                .params
                .iter()
                .map(|(name, type_dbg)| {
                    // Coerce CLI string args to declared parameter types
                    if type_dbg.contains("Number") {
                        format!(
                            r#"            params.insert("{name}".to_string(), forge::runtime::confidence::ConfidentValue::deterministic(if let Ok(n) = {name}.parse::<f64>() {{ forge::runtime::confidence::Value::Number(n) }} else {{ forge::runtime::confidence::Value::Text({name}) }}));"#,
                            name = name,
                        )
                    } else if type_dbg.contains("Bool") {
                        format!(
                            r#"            params.insert("{name}".to_string(), forge::runtime::confidence::ConfidentValue::deterministic(forge::runtime::confidence::Value::Bool({name} == "true" || {name} == "1")));"#,
                            name = name,
                        )
                    } else {
                        format!(
                            r#"            params.insert("{name}".to_string(), forge::runtime::confidence::ConfidentValue::deterministic(forge::runtime::confidence::Value::Text({name})));"#,
                            name = name,
                        )
                    }
                })
                .collect();

            let destructure_fields: Vec<String> = handler
                .params
                .iter()
                .map(|(name, _)| name.clone())
                .collect();

            match_arms.push(format!(
                r#"        Command::{variant} {{ {destructure} }} => {{
            let mut params = std::collections::HashMap::new();
{inserts}
            dispatch(&agent, "{event}", params).await?;
        }}"#,
                variant = variant,
                event = handler.event,
                destructure = destructure_fields.join(", "),
                inserts = param_inserts.join("\n"),
            ));
        }
    }

    // Add Repl variant
    variants.push("    /// Start interactive REPL session\n    Repl,".to_string());
    match_arms.push(
        r#"        Command::Repl => {
            run_repl(&agent, &agent_decl).await?;
        }"#
        .to_string(),
    );

    format!(
        r#"use clap::{{Parser, Subcommand}};
use std::sync::Arc;
use std::collections::HashMap;

const SOURCES: &[(&str, &str)] = &[
{sources}
];

{config}

#[derive(Parser)]
#[command(name = "{agent_name}", about = "FORGE agent: {agent_name}")]
struct Cli {{
    #[command(subcommand)]
    command: Command,
}}

#[derive(Subcommand)]
enum Command {{
{variants}
}}

async fn dispatch(
    agent: &forge::runtime::agent::AgentProcess,
    event: &str,
    params: HashMap<String, forge::runtime::confidence::ConfidentValue>,
) -> anyhow::Result<()> {{
    match agent.dispatch(event, params).await {{
        Ok(Some(val)) => println!("{{}}", val.value),
        Ok(None) => {{}},
        Err(e) => {{
            eprintln!("error: {{}}", e);
            std::process::exit(1);
        }}
    }}
    Ok(())
}}

async fn run_repl(
    agent: &forge::runtime::agent::AgentProcess,
    agent_decl: &forge::ast::AgentDecl,
) -> anyhow::Result<()> {{
    use std::io::{{BufRead, Write}};

    let handler_names: Vec<String> = agent_decl
        .handlers
        .iter()
        .map(|h| h.node.event.node.clone())
        .collect();
    println!("handlers: {{}}", handler_names.join(", "));
    println!("type 'quit' to exit\n");

    let stdin = std::io::stdin();
    loop {{
        print!("> ");
        std::io::stdout().flush()?;

        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {{
            break;
        }}
        let line = line.trim();
        if line.is_empty() {{
            continue;
        }}
        if line == "quit" || line == "exit" {{
            break;
        }}

        let parts: Vec<&str> = line.splitn(2, ' ').collect();
        let event = parts[0];
        let args_str = parts.get(1).unwrap_or(&"");

        // Simple arg parsing: split on spaces, respecting quotes
        let args = parse_args(args_str);
        let handler = agent_decl.handlers.iter().find(|h| h.node.event.node == event);
        let mut params = HashMap::new();
        if let Some(handler) = handler {{
            for (i, param) in handler.node.params.iter().enumerate() {{
                if let Some(arg) = args.get(i) {{
                    params.insert(
                        param.node.name.clone(),
                        forge::runtime::confidence::ConfidentValue::deterministic(
                            forge::runtime::confidence::Value::Text(arg.clone()),
                        ),
                    );
                }}
            }}
        }}

        match agent.dispatch(event, params).await {{
            Ok(Some(val)) => println!("{{}}", val.value),
            Ok(None) => {{}},
            Err(e) => eprintln!("error: {{}}", e),
        }}
    }}
    Ok(())
}}

fn parse_args(s: &str) -> Vec<String> {{
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in s.chars() {{
        match ch {{
            '"' => in_quotes = !in_quotes,
            ' ' if !in_quotes => {{
                if !current.is_empty() {{
                    args.push(std::mem::take(&mut current));
                }}
            }}
            _ => current.push(ch),
        }}
    }}
    if !current.is_empty() {{
        args.push(current);
    }}
    args
}}

#[tokio::main]
async fn main() -> anyhow::Result<()> {{
    let cli = Cli::parse();
    let program = forge::compose::parse_and_merge_sources(SOURCES)
        .map_err(|e| anyhow::anyhow!("{{}}", e))?;
    let config = resolve_config();
    let registry = forge::llm::registry::ProviderRegistry::from_config(config)
        .map_err(|e| anyhow::anyhow!("provider setup failed: {{}}", e))?;

    let agent_decl = forge::compose::find_agent(&program, Some("{agent_name}"))
        .ok_or_else(|| anyhow::anyhow!("agent '{agent_name}' not found in program"))?;
    let lifecycle_name = agent_decl.lifecycle.as_ref().map(|l| l.node.as_str());
    let states_decl = forge::compose::find_states(&program, lifecycle_name);

    // Create instance registry for find/spawn support
    let instance_registry: forge::runtime::instance_registry::SharedInstanceRegistry =
        std::sync::Arc::new(tokio::sync::RwLock::new(
            forge::runtime::instance_registry::InstanceRegistry::new(),
        ));

    // Open persistent storage for memory across invocations
    let storage = {{
        let db_path = std::path::Path::new(".forge-data").join("store.redb");
        std::fs::create_dir_all(".forge-data").ok();
        forge::runtime::storage::ForgeStorage::open(&db_path)
            .ok()
            .map(|s| std::sync::Arc::new(s))
    }};

    let agent = forge::runtime::agent::AgentProcess::new(
        agent_decl.clone(),
        states_decl.as_ref(),
        Arc::new(registry),
        None,
        program,
        storage,
        Some(instance_registry),
    );

    match cli.command {{
{match_arms}
    }}
    Ok(())
}}
"#,
        sources = sources_array,
        config = config_code,
        agent_name = agent.name,
        variants = variants.join("\n"),
        match_arms = match_arms.join("\n"),
    )
}

// ── Template: Server ──────────────────────────────────────────

fn generate_server_main(source_entries: &[String], embed_config: Option<&str>) -> String {
    let sources_array = source_entries.join(",\n    ");
    let config_code = config_resolution_code(embed_config);

    format!(
        r#"use clap::Parser;
use std::sync::Arc;

const SOURCES: &[(&str, &str)] = &[
{sources}
];

{config}

#[derive(Parser)]
#[command(name = "{name}", about = "FORGE server")]
struct Cli {{
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value = "3000")]
    port: u16,
}}

#[tokio::main]
async fn main() -> anyhow::Result<()> {{
    let cli = Cli::parse();
    let program = forge::compose::parse_and_merge_sources(SOURCES)
        .map_err(|e| anyhow::anyhow!("{{}}", e))?;
    let mut config = resolve_config();

    // Apply CLI overrides
    if let Some(ref mut server) = config.server {{
        server.host = Some(cli.host);
        server.port = Some(cli.port);
    }} else {{
        config.server = Some(forge::config::ServerConfig {{
            host: Some(cli.host),
            port: Some(cli.port),
            cors_origins: None,
        }});
    }}

    let registry = forge::llm::registry::ProviderRegistry::from_config(config.clone())
        .map_err(|e| anyhow::anyhow!("provider setup failed: {{}}", e))?;
    let cmd_mgr = Arc::new(std::sync::Mutex::new(forge::runtime::command_manager::CommandManager::new()));
    let session_mgr = forge::runtime::session_manager::new_shared_default_session_manager(None);
    let _ = session_mgr.resume_all().await;
    let executor = forge::runtime::executor::TaskExecutor::new(
        program, Arc::new(registry), None,
    ).with_command_manager(cmd_mgr).with_session_manager(session_mgr);
    let event_bus = forge::runtime::event_bus::EventBus::new_shared(None);
    let mut server = forge::runtime::http_server::ForgeServer::new(executor, config.server.as_ref())
        .with_event_bus(event_bus);
    if let Some(ref srv_config) = config.server {{
        if let Some(ref secrets) = srv_config.webhook_secrets {{
            server = server.with_webhook_secrets(secrets.clone());
        }}
    }}
    server.run().await
}}
"#,
        sources = sources_array,
        config = config_code,
        name = "forge-server",
    )
}
