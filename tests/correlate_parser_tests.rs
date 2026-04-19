// FORGE correlate block parser tests — issue #334
// Golden AST assertions for the `correlate on Event.field` block.

use forge::ast::*;
use forge::parser::parse;

fn only_agent(program: &Program) -> &AgentDecl {
    for item in &program.items {
        if let TopLevel::Agent(a) = &item.node {
            return a;
        }
    }
    panic!("program has no agent");
}

fn only_correlate(program: &Program) -> &CorrelateField {
    let agent = only_agent(program);
    assert_eq!(
        agent.correlates.len(),
        1,
        "expected exactly one correlate block, got {}",
        agent.correlates.len()
    );
    &agent.correlates[0].node
}

#[test]
fn parses_minimal_wake_with_emit() {
    let source = "\
event SlackMention
  thread_ts: Text
agent slack_specialist
  memory persistent
    thread_ts: Text
  correlate on SlackMention.thread_ts
    mode: wake
    emit: SlackReply
  on start
    say \"ok\"
";
    let program = parse(source).expect("parse must succeed");
    let correlate = only_correlate(&program);

    assert_eq!(correlate.event_type.node, "SlackMention");
    assert_eq!(correlate.field_name.node, "thread_ts");
    assert_eq!(
        correlate.mode.as_ref().expect("mode").node,
        ScheduleMode::Wake
    );
    assert_eq!(correlate.emit.as_ref().expect("emit").node, "SlackReply");
    assert!(correlate.duplicates.is_empty());
}

#[test]
fn parses_spawn_mode_without_emit() {
    let source = "\
event Inbound
  key: Text
agent a
  memory persistent
    key: Text
  correlate on Inbound.key
    mode: spawn
  on start
    say \"ok\"
";
    let program = parse(source).expect("parse must succeed");
    let correlate = only_correlate(&program);

    assert_eq!(correlate.event_type.node, "Inbound");
    assert_eq!(correlate.field_name.node, "key");
    assert_eq!(correlate.mode.as_ref().unwrap().node, ScheduleMode::Spawn);
    assert!(correlate.emit.is_none());
}

#[test]
fn captures_duplicate_mode_option() {
    let source = "\
event E
  k: Text
agent a
  memory persistent
    k: Text
  correlate on E.k
    mode: wake
    mode: spawn
    emit: Out
  on start
    say \"ok\"
";
    let program = parse(source).expect("parse must succeed");
    let correlate = only_correlate(&program);
    let dup_names: Vec<&str> = correlate
        .duplicates
        .iter()
        .map(|d| d.node.as_str())
        .collect();
    assert!(
        dup_names.contains(&"mode"),
        "expected duplicate `mode` captured, got {:?}",
        dup_names
    );
}

#[test]
fn parses_multiple_correlate_blocks_in_one_agent() {
    let source = "\
event A
  k: Text
event B
  k: Text
agent a
  memory persistent
    k: Text
  correlate on A.k
    mode: wake
    emit: AOut
  correlate on B.k
    mode: wake
    emit: BOut
  on start
    say \"ok\"
";
    let program = parse(source).expect("parse must succeed");
    let agent = only_agent(&program);
    assert_eq!(agent.correlates.len(), 2);
    assert_eq!(agent.correlates[0].node.event_type.node, "A");
    assert_eq!(agent.correlates[1].node.event_type.node, "B");
}
