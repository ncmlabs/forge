// FORGE file watcher for hot-reload development mode. See issue #47.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecursiveMode, Watcher};
use tokio::sync::broadcast;

use crate::runtime::http_server::{print_endpoint_list, SwappableExecutor};

/// Action the watcher signals back to the serve loop.
#[derive(Debug)]
pub enum WatchAction {
    /// Config changed — caller should restart the entire server.
    RestartServer,
}

/// Watch the `.forge` source file (and optionally the config file) for changes.
/// On `.forge` changes: re-parse, re-validate, and hot-swap the executor.
/// On config changes: return `WatchAction::RestartServer`.
pub async fn watch_and_reload(
    file: PathBuf,
    swappable: SwappableExecutor,
    reload_tx: Option<broadcast::Sender<()>>,
    events_tx: Option<broadcast::Sender<String>>,
) -> anyhow::Result<WatchAction> {
    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();

    let mut watcher = notify::recommended_watcher(tx)?;

    // Watch the source file
    watcher.watch(&file, RecursiveMode::NonRecursive)?;

    // Watch the parent directory (catches renames, new files)
    if let Some(parent) = file.parent() {
        watcher.watch(parent, RecursiveMode::NonRecursive)?;
    }

    // Watch the config file if it exists
    let config_path = crate::config::ForgeConfig::resolve_path();
    if let Some(ref cp) = config_path {
        watcher.watch(cp, RecursiveMode::NonRecursive)?;
    }

    eprintln!("Watching for changes...");

    // Debounce loop: collect events within a 300ms window
    let debounce = Duration::from_millis(300);

    loop {
        // Block until the first event
        let first = match rx.recv() {
            Ok(Ok(event)) => event,
            Ok(Err(e)) => {
                eprintln!("watch error: {e}");
                continue;
            }
            Err(_) => return Err(anyhow::anyhow!("watcher channel closed")),
        };

        // Collect additional events within debounce window
        let mut events = vec![first];
        let deadline = Instant::now() + debounce;
        loop {
            let timeout = deadline.saturating_duration_since(Instant::now());
            if timeout.is_zero() {
                break;
            }
            match rx.recv_timeout(timeout) {
                Ok(Ok(event)) => events.push(event),
                Ok(Err(e)) => eprintln!("watch error: {e}"),
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(anyhow::anyhow!("watcher channel closed"));
                }
            }
        }

        // Classify the batch of events
        let mut forge_changed = false;
        let mut config_changed = false;

        for event in &events {
            // Only care about modify/create/remove events
            match event.kind {
                EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_) => {}
                _ => continue,
            }

            for path in &event.paths {
                if is_config_path(path, config_path.as_deref()) {
                    config_changed = true;
                } else if is_forge_file(path) {
                    forge_changed = true;
                }
                // Static file changes are ignored — ServeDir serves them directly
            }
        }

        if config_changed {
            eprintln!("Config file changed -- requesting server restart...");
            return Ok(WatchAction::RestartServer);
        }

        if forge_changed && attempt_reload(&file, &swappable, &events_tx) {
            // Notify connected browsers to reload
            if let Some(ref tx) = reload_tx {
                let _ = tx.send(());
            }
        }
    }
}

fn is_config_path(path: &Path, config_path: Option<&Path>) -> bool {
    if let Some(cp) = config_path {
        // Compare canonical paths if possible, fall back to direct comparison
        let matches = path == cp;
        if matches {
            return true;
        }
        // Try canonical comparison
        if let (Ok(a), Ok(b)) = (path.canonicalize(), cp.canonicalize()) {
            return a == b;
        }
    }
    // Also match by filename in case of symlinks or relative path differences
    path.file_name()
        .map(|n| n == "forge.config.toml")
        .unwrap_or(false)
}

fn is_forge_file(path: &Path) -> bool {
    path.extension().map(|ext| ext == "forge").unwrap_or(false)
}

/// Attempt to reload the executor from the source file.
/// Returns true on success, false on failure.
fn attempt_reload(
    file: &Path,
    swappable: &SwappableExecutor,
    events_tx: &Option<broadcast::Sender<String>>,
) -> bool {
    eprint!("File changed -- reloading... ");

    let source = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("failed to read {}: {e}", file.display());
            return false;
        }
    };

    let fname = file.display().to_string();

    // Parse
    let program = match crate::parser::parse(&source) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("parse error:");
            e.to_diagnostic(&fname).render(&source);
            eprintln!("Reload failed -- keeping previous version.");
            return false;
        }
    };

    // Validate
    let mut diagnostics = Vec::new();

    let ctx = crate::resolver::CheckContext::new(&fname);
    if let Err(errors) = ctx.check(&program) {
        let registry = crate::resolver::CapabilityRegistry::builtin();
        diagnostics.extend(errors.iter().map(|e| e.to_diagnostic(&fname, &registry)));
    }

    diagnostics.extend(crate::checker::check_all(&program, &fname));

    let boundary_refs = vec![(&program, fname.as_str())];
    diagnostics.extend(crate::checker::boundary_checker::check(&boundary_refs));

    if !diagnostics.is_empty() {
        eprintln!("{} error(s):", diagnostics.len());
        crate::diagnostic::render_diagnostics(&source, &diagnostics);
        eprintln!("Reload failed -- keeping previous version.");
        return false;
    }

    // Build new executor
    let config = crate::config::ForgeConfig::load_or_default();

    let trace_env = std::env::var("FORGE_TRACE")
        .map(|v| v == "1")
        .unwrap_or(false);
    let tracer = match (events_tx, trace_env) {
        (Some(tx), _) => Some(crate::tracer::Tracer::with_live(tx.clone())),
        (None, true) => Some(crate::tracer::Tracer::new()),
        (None, false) => None,
    };

    let registry = match crate::llm::registry::ProviderRegistry::from_config(config.clone()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("provider setup failed: {e}");
            eprintln!("Reload failed -- keeping previous version.");
            return false;
        }
    };

    let new_executor =
        crate::runtime::executor::TaskExecutor::new(program, std::sync::Arc::new(registry), tracer)
            .with_config(config);

    let endpoint_count = new_executor.endpoints().len();

    // Swap
    {
        let mut guard = swappable.write().unwrap();
        let endpoints = new_executor.endpoints().clone();
        *guard = new_executor;
        eprintln!("OK ({} endpoint(s))", endpoint_count);
        print_endpoint_list(&endpoints);
    }

    true
}
