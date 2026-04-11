// FORGE checked-in example validation — issue #231.
// Keeps the examples corpus classified, parser/checker-clean where expected,
// and runnable in CI only through mock-safe examples.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use forge::checker;
use forge::checker::boundary_checker;
use forge::compose::{self, SourceFile};
use forge::diagnostic::{Diagnostic, DiagnosticKind};
use forge::llm::providers::mock::MockProvider;
use forge::llm::registry::ProviderRegistry;
use forge::manifest::ProjectManifest;
use forge::resolver::{CapabilityRegistry, CheckContext};
use forge::runtime::command_manager::CommandManager;
use forge::runtime::executor::TaskExecutor;
use forge::runtime::skill_loader::SkillLoader;
use forge::runtime::skill_registry::SkillRegistry;
use forge::types::CapabilitySignature;

#[derive(Debug, serde::Deserialize)]
struct ValidationManifest {
    cases: Vec<ValidationCase>,
}

#[derive(Debug, serde::Deserialize)]
struct ValidationCase {
    name: String,
    paths: Vec<String>,
    check: CheckMode,
    #[serde(default)]
    run: RunMode,
    #[serde(default)]
    merge: bool,
    #[serde(default)]
    expect: Vec<String>,
    manifest: Option<String>,
    config: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum CheckMode {
    Ok,
    Error,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum RunMode {
    #[default]
    None,
    Mock,
    LiveOnly,
}

struct CheckOutcome {
    diagnostics: Vec<Diagnostic>,
    programs: Vec<forge::ast::Program>,
}

#[test]
fn example_manifest_covers_every_forge_example() {
    let root = repo_root();
    let manifest = load_manifest(&root);
    let discovered = discover_forge_examples(&root);
    let classified = classified_paths(&manifest);

    let missing: Vec<_> = discovered.difference(&classified).cloned().collect();
    let stale: Vec<_> = classified.difference(&discovered).cloned().collect();
    let duplicates = duplicate_paths(&manifest);

    assert!(
        missing.is_empty() && stale.is_empty() && duplicates.is_empty(),
        "example validation manifest coverage mismatch\nmissing: {missing:?}\nstale: {stale:?}\nduplicates: {duplicates:?}"
    );
}

#[tokio::test]
async fn example_validation_manifest_passes() {
    let root = repo_root();
    let manifest = load_manifest(&root);
    let mut checked_ok = 0;
    let mut checked_error = 0;
    let mut warnings = 0;
    let mut mock_runs = 0;
    let mut live_skips = 0;

    for case in &manifest.cases {
        let outcome = check_case(&root, case);
        let errors = diagnostics_of_kind(&outcome.diagnostics, DiagnosticKind::Error);
        let case_warnings = diagnostics_of_kind(&outcome.diagnostics, DiagnosticKind::Warning);
        warnings += case_warnings.len();

        match case.check {
            CheckMode::Ok => {
                checked_ok += 1;
                assert!(
                    errors.is_empty(),
                    "case `{}` expected no checker errors, got: {:?}",
                    case.name,
                    errors.iter().map(|d| &d.message).collect::<Vec<_>>()
                );
            }
            CheckMode::Error => {
                checked_error += 1;
                assert!(
                    !errors.is_empty(),
                    "case `{}` expected checker errors but passed",
                    case.name
                );
                assert_expected_diagnostics(case, &outcome.diagnostics);
            }
        }

        match case.run {
            RunMode::None => {}
            RunMode::Mock => {
                assert!(
                    errors.is_empty(),
                    "case `{}` cannot be mock-run because check errors were present",
                    case.name
                );
                assert!(
                    case_warnings.is_empty(),
                    "case `{}` is mock-runnable but has checker warnings: {:?}",
                    case.name,
                    case_warnings.iter().map(|d| &d.message).collect::<Vec<_>>()
                );
                run_mock_case(&outcome.programs, case).await;
                mock_runs += 1;
            }
            RunMode::LiveOnly => {
                live_skips += 1;
                eprintln!("SKIP live-only runtime case: {}", case.name);
            }
        }
    }

    eprintln!(
        "FORGE example validation summary: {checked_ok} ok cases, {checked_error} expected-error cases, {warnings} warnings, {mock_runs} mock runs, {live_skips} live-only runtime skips"
    );
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn load_manifest(root: &Path) -> ValidationManifest {
    let path = root.join("examples/validation.toml");
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()));
    toml::from_str(&source).unwrap_or_else(|e| panic!("invalid {}: {e}", path.display()))
}

fn discover_forge_examples(root: &Path) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    collect_forge_files(&root.join("examples"), root, &mut paths);
    collect_forge_files(&root.join("workflows"), root, &mut paths);
    paths
}

fn collect_forge_files(dir: &Path, root: &Path, out: &mut BTreeSet<String>) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|e| panic!("could not read {}: {e}", dir.display()));
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_forge_files(&path, root, out);
        } else if path.extension().is_some_and(|ext| ext == "forge")
            && !path.components().any(|c| c.as_os_str() == "static")
        {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(rel);
        }
    }
}

fn classified_paths(manifest: &ValidationManifest) -> BTreeSet<String> {
    manifest
        .cases
        .iter()
        .flat_map(|case| case.paths.iter().cloned())
        .collect()
}

fn duplicate_paths(manifest: &ValidationManifest) -> Vec<String> {
    let mut counts = BTreeMap::new();
    for path in manifest.cases.iter().flat_map(|case| case.paths.iter()) {
        *counts.entry(path.clone()).or_insert(0usize) += 1;
    }
    counts
        .into_iter()
        .filter_map(|(path, count)| (count > 1).then_some(path))
        .collect()
}

fn check_case(root: &Path, case: &ValidationCase) -> CheckOutcome {
    let source_files = parse_case_sources(root, case);
    let skill_sigs = skill_signatures(root, case);
    let mut diagnostics = Vec::new();

    if !case.merge {
        let mut programs = Vec::new();
        for source_file in &source_files {
            diagnostics.extend(check_program(
                &source_file.program,
                &source_file.path,
                &skill_sigs,
            ));
            diagnostics.extend(boundary_checker::check(&[(
                &source_file.program,
                source_file.path.as_str(),
            )]));
            programs.push(source_file.program.clone());
        }
        return CheckOutcome {
            diagnostics,
            programs,
        };
    }

    let composed = compose::merge_programs(&source_files).unwrap_or_else(|errs| {
        panic!(
            "case `{}` failed composition: {:?}",
            case.name,
            errs.iter().map(ToString::to_string).collect::<Vec<_>>()
        )
    });

    let filename = case.paths.join("+");
    diagnostics.extend(check_program(&composed.program, &filename, &skill_sigs));

    let boundary_refs: Vec<_> = source_files
        .iter()
        .map(|sf| (&sf.program, sf.path.as_str()))
        .collect();
    diagnostics.extend(boundary_checker::check(&boundary_refs));

    CheckOutcome {
        diagnostics,
        programs: vec![composed.program],
    }
}

fn check_program(
    program: &forge::ast::Program,
    filename: &str,
    skill_sigs: &HashMap<String, CapabilitySignature>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let ctx = if skill_sigs.is_empty() {
        CheckContext::new(filename)
    } else {
        CheckContext::with_skills(filename, skill_sigs.clone())
    };

    if let Err(errors) = ctx.check(program) {
        let registry = if skill_sigs.is_empty() {
            CapabilityRegistry::builtin()
        } else {
            CapabilityRegistry::with_skills(skill_sigs.clone())
        };
        diagnostics.extend(errors.iter().map(|e| e.to_diagnostic(filename, &registry)));
    }

    diagnostics.extend(checker::check_all(program, filename));
    diagnostics
}

fn parse_case_sources(root: &Path, case: &ValidationCase) -> Vec<SourceFile> {
    case.paths
        .iter()
        .map(|rel| {
            let path = root.join(rel);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("case `{}` could not read {}: {e}", case.name, rel));
            let program = forge::parser::parse(&source).unwrap_or_else(|e| {
                panic!("case `{}` parse failed for {}: {:?}", case.name, rel, e)
            });
            SourceFile {
                path: rel.clone(),
                source,
                program,
            }
        })
        .collect()
}

fn skill_signatures(root: &Path, case: &ValidationCase) -> HashMap<String, CapabilitySignature> {
    let mut registry = SkillRegistry::new();

    if let Some(manifest_path) = &case.manifest {
        let path = root.join(manifest_path);
        let manifest = ProjectManifest::load(&path).unwrap_or_else(|e| {
            panic!("case `{}` could not load {}: {e}", case.name, manifest_path)
        });
        let base = path.parent().unwrap_or(root);
        for (_, skill_path) in manifest.resolve_skills(base, &[]).unwrap_or_else(|e| {
            panic!(
                "case `{}` could not resolve manifest skills: {e}",
                case.name
            )
        }) {
            let skill = SkillLoader::parse_skill_md(&skill_path).unwrap_or_else(|e| {
                panic!(
                    "case `{}` could not parse {}: {e}",
                    case.name,
                    skill_path.display()
                )
            });
            registry.register(skill);
        }
    }

    if let Some(config_path) = &case.config {
        let path = root.join(config_path);
        let config = forge::config::ForgeConfig::load(&path)
            .unwrap_or_else(|e| panic!("case `{}` could not load {}: {e}", case.name, config_path));
        if let Some(skills) = config.skills {
            let dirs: Vec<PathBuf> = skills
                .skill_dirs_or_default()
                .into_iter()
                .map(|dir| {
                    if dir.is_absolute() {
                        dir
                    } else {
                        root.join(dir)
                    }
                })
                .collect();
            for skill in SkillLoader::load_from_dirs(&dirs) {
                registry.register(skill);
            }
        }
    }

    registry.capability_signatures()
}

fn diagnostics_of_kind(diags: &[Diagnostic], kind: DiagnosticKind) -> Vec<&Diagnostic> {
    diags
        .iter()
        .filter(|d| std::mem::discriminant(&d.kind) == std::mem::discriminant(&kind))
        .collect()
}

fn assert_expected_diagnostics(case: &ValidationCase, diagnostics: &[Diagnostic]) {
    let messages = diagnostics
        .iter()
        .map(|d| d.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for expected in &case.expect {
        assert!(
            messages.contains(expected),
            "case `{}` expected diagnostic text `{}` in {:?}",
            case.name,
            expected,
            messages
        );
    }
}

async fn run_mock_case(programs: &[forge::ast::Program], case: &ValidationCase) {
    for program in programs {
        let mut providers = ProviderRegistry::new("mock");
        providers.register(
            "mock",
            Arc::new(MockProvider::new("mock").with_default("mock response")),
        );
        let executor = TaskExecutor::new(program.clone(), Arc::new(providers), None)
            .with_config(forge::config::ForgeConfig::default_mock_config())
            .with_command_manager(Arc::new(Mutex::new(CommandManager::new())));
        executor
            .run()
            .await
            .unwrap_or_else(|e| panic!("case `{}` failed mock runtime: {e}", case.name));
    }
}
