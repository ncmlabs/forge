// FORGE skill bridge integration tests — issue #40
// Tests the full SKILL.md loading pipeline and exec-based skill execution.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use forge::llm::providers::mock::MockProvider;
use forge::llm::registry::ProviderRegistry;
use forge::runtime::confidence::ConfidentValue;
use forge::runtime::executor::TaskExecutor;
use forge::runtime::skill_loader::SkillLoader;
use forge::runtime::skill_registry::SkillRegistry;
use forge::tracer::Tracer;
use forge::types::ConfidenceSource;

// ── Helpers ──────────────────────────────────────────────────────

fn parse_file(path: &str) -> forge::ast::Program {
    let source =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("could not read {}: {}", path, e));
    forge::parser::parse(&source).unwrap_or_else(|e| panic!("parse failed for {}: {:?}", path, e))
}

fn mock_registry(mock: MockProvider) -> Arc<ProviderRegistry> {
    let mut reg = ProviderRegistry::new("mock");
    reg.register("mock", Arc::new(mock));
    Arc::new(reg)
}

fn create_temp_skill(name: &str, description: &str, body: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join(name);
    std::fs::create_dir_all(&skill_dir).unwrap();
    let mut file = std::fs::File::create(skill_dir.join("SKILL.md")).unwrap();
    write!(
        file,
        "---\nname: {}\ndescription: {}\nallowed-tools: Bash\n---\n\n{}",
        name, description, body
    )
    .unwrap();
    dir
}

// ── SKILL.md Loading Tests ──────────────────────────────────────

#[test]
fn skill_loader_parses_local_find_skills() {
    // Parse the real find-skills SKILL.md if it exists locally
    let home = match std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
        Ok(h) => PathBuf::from(h),
        Err(_) => {
            eprintln!("Skipping: HOME/USERPROFILE not set");
            return;
        }
    };
    let skill_path = home.join(".claude/skills/find-skills/SKILL.md");
    if !skill_path.exists() {
        eprintln!("Skipping: ~/.claude/skills/find-skills/SKILL.md not found");
        return;
    }
    let skill = SkillLoader::parse_skill_md(&skill_path).unwrap();
    assert_eq!(skill.manifest.name, "find-skills");
    assert!(
        !skill.manifest.description.is_empty(),
        "description should not be empty"
    );
    assert!(
        !skill.instructions.is_empty(),
        "instructions body should not be empty"
    );
    assert!(
        skill.instructions.contains("npx skills"),
        "instructions should mention npx skills, got first 100 chars: {}",
        &skill.instructions[..100.min(skill.instructions.len())]
    );
}

#[test]
fn skill_loader_discovers_skills_from_directory() {
    let dir = create_temp_skill(
        "test-echo",
        "Echo things back",
        "Run echo with the given argument.",
    );

    let skills = SkillLoader::load_from_dirs(&[dir.path().to_path_buf()]);
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].manifest.name, "test-echo");
    assert_eq!(skills[0].manifest.description, "Echo things back");
    assert!(skills[0].instructions.contains("Run echo"));
}

#[test]
fn skill_loader_handles_missing_directory() {
    let skills = SkillLoader::load_from_dirs(&[PathBuf::from("/nonexistent/path/skills")]);
    assert!(skills.is_empty(), "should return empty for missing dir");
}

#[test]
fn skill_loader_discovers_multiple_skills() {
    let dir = tempfile::tempdir().unwrap();

    // Create two skill subdirectories
    for (name, desc) in &[("skill-a", "First skill"), ("skill-b", "Second skill")] {
        let skill_dir = dir.path().join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        let mut file = std::fs::File::create(skill_dir.join("SKILL.md")).unwrap();
        write!(
            file,
            "---\nname: {}\ndescription: {}\n---\n\nInstructions for {}.",
            name, desc, name
        )
        .unwrap();
    }

    let skills = SkillLoader::load_from_dirs(&[dir.path().to_path_buf()]);
    assert_eq!(skills.len(), 2);
    let names: Vec<&str> = skills.iter().map(|s| s.manifest.name.as_str()).collect();
    assert!(names.contains(&"skill-a"));
    assert!(names.contains(&"skill-b"));
}

// ── Skill Registry Tests ────────────────────────────────────────

#[test]
fn skill_registry_registers_and_retrieves() {
    let dir = create_temp_skill("my-skill", "Test skill", "Do something.");

    let skills = SkillLoader::load_from_dirs(&[dir.path().to_path_buf()]);
    let mut registry = SkillRegistry::new();
    for skill in skills {
        registry.register(skill);
    }

    assert_eq!(registry.skill_count(), 1);
    let skill = registry.get("my-skill").unwrap();
    assert_eq!(skill.manifest.name, "my-skill");
}

#[test]
fn skill_registry_generates_capability_signatures() {
    let dir = create_temp_skill("echo-skill", "Echoes things", "Echo.");

    let skills = SkillLoader::load_from_dirs(&[dir.path().to_path_buf()]);
    let mut registry = SkillRegistry::new();
    for skill in skills {
        registry.register(skill);
    }

    let sigs = registry.capability_signatures();
    assert!(
        sigs.contains_key("skill.echo-skill"),
        "should have skill.echo-skill capability, got: {:?}",
        sigs.keys().collect::<Vec<_>>()
    );
}

// ── Confidence Model Tests ──────────────────────────────────────

#[test]
fn exec_confidence_from_success() {
    let cv = ConfidentValue::from_exec(
        forge::runtime::confidence::Value::Text("output".to_string()),
        0.9,
    );
    assert!(cv.sure(), "exit 0 should produce sure result");
    assert_eq!(cv.confidence, 0.9);
    assert!(matches!(cv.source, ConfidenceSource::ExecResult(0.9)));
}

#[test]
fn exec_confidence_from_failure() {
    let cv = ConfidentValue::from_exec(
        forge::runtime::confidence::Value::Text("error".to_string()),
        0.3,
    );
    assert!(cv.unreliable(), "exit 1 should produce unreliable result");
    assert_eq!(cv.confidence, 0.3);
}

#[test]
fn skill_confidence_capped() {
    let cv = ConfidentValue::from_skill(
        forge::runtime::confidence::Value::Text("result".to_string()),
        1.0,
    );
    assert_eq!(cv.confidence, 0.99, "skill confidence should cap at 0.99");
    assert!(matches!(cv.source, ConfidenceSource::SkillInvocation(_)));
}

// ── Runtime: exec-based skill finder ────────────────────────────

#[tokio::test]
async fn exec_runs_npx_skills_find() {
    // This test uses Unix-only piping (tail) and npx is too slow on Windows CI.
    if cfg!(windows) {
        eprintln!("Skipping: not supported on Windows");
        return;
    }

    // This test calls the real npx skills CLI — skip if not available
    let npx_check = std::process::Command::new("which")
        .arg("npx")
        .output()
        .ok()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !npx_check {
        eprintln!("Skipping: npx not available");
        return;
    }

    // Use a simple exec to call npx skills find
    let source = r#"
task search_skills
  needs query: Text
  gives Text
  do
    result = exec "npx skills find \"{query}\" 2>&1 | tail -5"
    when result.sure -> give result
    else -> give "search failed"

fn main
  say search_skills("react")
"#;

    let program = forge::parser::parse(source).expect("parse failed");
    let mock = MockProvider::new("mock").with_default("mock");
    let tracer = Tracer::with_capture();
    let executor = TaskExecutor::new(program, mock_registry(mock), Some(tracer.clone()));
    let result = executor.run().await;
    assert!(
        result.is_ok(),
        "npx skills find should run: {:?}",
        result.err()
    );
    let outputs = executor.outputs();
    assert!(
        !outputs.is_empty(),
        "should produce output from npx skills find"
    );
    // The output should contain skills.sh links or install counts
    let all_output = outputs.join("\n");
    assert!(
        all_output.contains("skills.sh")
            || all_output.contains("installs")
            || all_output.contains("search failed"),
        "output should contain skill results or graceful failure, got: {}",
        all_output
    );

    // Verify trace events
    let events = tracer.captured_events();
    assert!(
        events.contains(&"exec_call".to_string()),
        "should trace exec_call"
    );
    assert!(
        events.contains(&"exec_return".to_string()),
        "should trace exec_return"
    );
}

// ── Runtime: exec-based agent with multiple commands ─────────────

#[tokio::test]
async fn exec_agent_runs_system_commands() {
    let source = r#"
task system_check
  gives Text
  do
    uname = exec "uname -s"
    when uname.sure -> give uname
    else -> give "unknown"

task file_listing
  needs dir: Text
  gives Text
  do
    listing = exec "ls {dir} | head -3"
    when listing.sure -> give listing
    else -> give "cannot list"

fn main
  say system_check()
  say file_listing(".")
"#;

    let program = forge::parser::parse(source).expect("parse failed");
    let mock = MockProvider::new("mock").with_default("mock");
    let executor = TaskExecutor::new(program, mock_registry(mock), None);
    let result = executor.run().await;
    assert!(
        result.is_ok(),
        "system commands should run: {:?}",
        result.err()
    );
    let outputs = executor.outputs();
    assert!(
        outputs.len() >= 2,
        "should have at least 2 outputs, got: {:?}",
        outputs
    );
    // uname should return Darwin, Linux, or similar
    assert!(
        outputs[0].contains("Darwin")
            || outputs[0].contains("Linux")
            || outputs[0].contains("MINGW")
            || outputs[0] == "unknown",
        "uname should return OS name, got: {}",
        outputs[0]
    );
}

// ── Full pipeline: skill_finder.forge example ───────────────────

#[tokio::test]
async fn skill_finder_example_validates() {
    let program = parse_file("examples/skill_finder.forge");
    let filename = "skill_finder.forge";
    let diags = forge::checker::check_all(&program, filename);
    let errs: Vec<_> = diags
        .iter()
        .filter(|d| matches!(d.kind, forge::diagnostic::DiagnosticKind::Error))
        .collect();
    assert!(
        errs.is_empty(),
        "skill_finder.forge should have no checker errors: {:?}",
        errs.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}
