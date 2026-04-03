// Integration tests for forge build pipeline
// See issue #74

use forge::compose::{self, SourceFile};

fn parse_source(name: &str, source: &str) -> SourceFile {
    let program = forge::parser::parse(source).expect("parse failed");
    SourceFile {
        path: name.to_string(),
        source: source.to_string(),
        program,
    }
}

// ── Composition tests ─────────────────────────────────────────

#[test]
fn compose_merges_two_files() {
    let a = parse_source(
        "a.forge",
        "use\n  llm.reason\n\ntask greet\n  needs name: Text\n  gives Text\n  do\n    result = reason \"Say hello to {name}\"\n    give result\n",
    );
    let b = parse_source(
        "b.forge",
        "use\n  llm.reason\n\npure double\n  needs n: Number\n  gives Number\n  do\n    give n * 2\n",
    );

    let composed = compose::merge_programs(&[a, b]).expect("merge should succeed");
    assert_eq!(composed.sources.len(), 2);

    let kind = compose::detect_kind(&composed.program);
    assert_eq!(kind, None); // no fn main, no agent, no endpoint
}

#[test]
fn compose_merges_with_fn_main() {
    let a = parse_source(
        "lib.forge",
        "use\n  llm.reason\n\ntask greet\n  needs name: Text\n  gives Text\n  do\n    result = reason \"hello {name}\"\n    give result\n",
    );
    let b = parse_source("main.forge", "fn main\n  say \"hello\"\n");

    let composed = compose::merge_programs(&[a, b]).expect("merge should succeed");
    assert_eq!(
        compose::detect_kind(&composed.program),
        Some(compose::ProgramKind::Executable)
    );
}

#[test]
fn compose_rejects_duplicate_task_names() {
    let a = parse_source(
        "a.forge",
        "use\n  llm.reason\n\ntask greet\n  needs name: Text\n  gives Text\n  do\n    result = reason \"hello {name}\"\n    give result\n",
    );
    let b = parse_source(
        "b.forge",
        "use\n  llm.reason\n\ntask greet\n  needs x: Text\n  gives Text\n  do\n    result = reason \"hi {x}\"\n    give result\n",
    );

    let err = compose::merge_programs(&[a, b]).unwrap_err();
    assert_eq!(err.len(), 1);
    assert!(err[0].to_string().contains("duplicate symbol `greet`"));
}

#[test]
fn compose_rejects_multiple_fn_main() {
    let a = parse_source("a.forge", "fn main\n  say \"hello\"\n");
    let b = parse_source("b.forge", "fn main\n  say \"world\"\n");

    let err = compose::merge_programs(&[a, b]).unwrap_err();
    assert!(err.iter().any(|e| e.to_string().contains("fn main")));
}

#[test]
fn compose_single_file_works() {
    let f = parse_source("hello.forge", "fn main\n  say \"hello world\"\n");
    let composed = compose::merge_programs(&[f]).expect("should work");
    assert_eq!(
        compose::detect_kind(&composed.program),
        Some(compose::ProgramKind::Executable)
    );
}

// ── Program kind detection ────────────────────────────────────

#[test]
fn detect_kind_executable() {
    let f = parse_source("test.forge", "fn main\n  say \"hello\"\n");
    assert_eq!(
        compose::detect_kind(&f.program),
        Some(compose::ProgramKind::Executable)
    );
}

#[test]
fn detect_kind_agent_cli() {
    let f = parse_source(
        "test.forge",
        "use\n  llm.reason\n\nagent bot\n  on greet(name: Text)\n    result = reason \"hello {name}\"\n    say result\n",
    );
    assert_eq!(
        compose::detect_kind(&f.program),
        Some(compose::ProgramKind::AgentCli)
    );
}

#[test]
fn detect_kind_server() {
    let f = parse_source(
        "test.forge",
        "#! boundary: server\n\nendpoint health() -> Text\n  give \"ok\"\n",
    );
    assert_eq!(
        compose::detect_kind(&f.program),
        Some(compose::ProgramKind::Server)
    );
}

#[test]
fn detect_kind_none_for_pure_library() {
    let f = parse_source(
        "lib.forge",
        "pure add\n  needs a: Number, b: Number\n  gives Number\n  do\n    give a + b\n",
    );
    assert_eq!(compose::detect_kind(&f.program), None);
}

// ── parse_and_merge_sources ───────────────────────────────────

#[test]
fn parse_and_merge_sources_works() {
    let sources: &[(&str, &str)] = &[
        ("main.forge", "fn main\n  say \"hello\"\n"),
        (
            "lib.forge",
            "pure add\n  needs a: Number, b: Number\n  gives Number\n  do\n    give a + b\n",
        ),
    ];

    let program = compose::parse_and_merge_sources(sources).expect("should merge");
    assert_eq!(
        compose::detect_kind(&program),
        Some(compose::ProgramKind::Executable)
    );
}

#[test]
fn parse_and_merge_sources_rejects_invalid() {
    let sources: &[(&str, &str)] = &[("bad.forge", "this is not valid forge")];
    assert!(compose::parse_and_merge_sources(sources).is_err());
}

// ── Manifest tests ────────────────────────────────────────────

#[test]
fn manifest_from_single_file() {
    let manifest = forge::manifest::ProjectManifest::from_single_file(
        std::path::Path::new("examples/hello.forge"),
        None,
    );
    assert_eq!(manifest.project.name, "hello");
    assert_eq!(manifest.output_name(), "hello");
}

#[test]
fn manifest_tictactoe_parses() {
    let manifest = forge::manifest::ProjectManifest::load(std::path::Path::new(
        "examples/tictactoe/forge.project.toml",
    ))
    .expect("should load");
    assert_eq!(manifest.project.name, "tictactoe");
    assert_eq!(manifest.output_name(), "tictactoe");

    let sources = manifest
        .resolve_sources(std::path::Path::new("examples/tictactoe"))
        .expect("should resolve");
    assert_eq!(sources.len(), 4); // entry + 3 sources
}

// ── Compose tictactoe files ───────────────────────────────────

#[test]
fn compose_tictactoe_parses_and_merges() {
    let files = [
        "examples/tictactoe/platform.forge",
        "examples/tictactoe/room_agent.forge",
        "examples/tictactoe/ai_opponent.forge",
        "examples/tictactoe/matchmaking.forge",
    ];

    let source_files: Vec<SourceFile> = files
        .iter()
        .map(|path| {
            let source = std::fs::read_to_string(path).expect("read file");
            let program = forge::parser::parse(&source).expect("parse");
            SourceFile {
                path: path.to_string(),
                source,
                program,
            }
        })
        .collect();

    // Merge should succeed (composition doesn't run checkers)
    let composed = compose::merge_programs(&source_files)
        .expect("tictactoe should merge without symbol conflicts");

    // Detects as AgentCli (has agents; system kind deferred while system runtime is unimplemented)
    let kind = compose::detect_kind(&composed.program);
    assert_eq!(kind, Some(compose::ProgramKind::AgentCli));
}

// ── find_agent / find_states ──────────────────────────────────

#[test]
fn find_agent_in_merged_program() {
    let f = parse_source(
        "agent.forge",
        "use\n  llm.reason\n\nagent mybot\n  on hello(name: Text)\n    say \"hi {name}\"\n",
    );
    let composed = compose::merge_programs(&[f]).unwrap();
    let agent = compose::find_agent(&composed.program, None);
    assert!(agent.is_some());
    assert_eq!(agent.unwrap().name.node, "mybot");
}

#[test]
fn find_agent_by_name() {
    let f = parse_source(
        "agents.forge",
        "use\n  llm.reason\n\nagent alpha\n  on greet\n    say \"alpha\"\n\nagent beta\n  on greet\n    say \"beta\"\n",
    );

    let alpha = compose::find_agent(&f.program, Some("alpha"));
    assert!(alpha.is_some());
    assert_eq!(alpha.unwrap().name.node, "alpha");

    let beta = compose::find_agent(&f.program, Some("beta"));
    assert!(beta.is_some());
    assert_eq!(beta.unwrap().name.node, "beta");

    let gamma = compose::find_agent(&f.program, Some("gamma"));
    assert!(gamma.is_none());
}
