// FORGE webhook block parser tests — issue #335
// Golden AST assertions for the `webhook TRIGGER` block on agents.

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

fn only_webhook(program: &Program) -> &WebhookField {
    let agent = only_agent(program);
    assert_eq!(
        agent.webhooks.len(),
        1,
        "expected exactly one webhook block, got {}",
        agent.webhooks.len()
    );
    &agent.webhooks[0].node
}

#[test]
fn parses_minimal_wake_with_emit() {
    let source = "\
event PrMerged
  repo: Text
agent mastermind
  webhook pr_merged
    mode: wake
    emit: PrMerged
  on start
    say \"ok\"
";
    let program = parse(source).expect("parse must succeed");
    let webhook = only_webhook(&program);

    assert_eq!(webhook.name.node, "pr_merged");
    assert_eq!(
        webhook.mode.as_ref().expect("mode").node,
        ScheduleMode::Wake
    );
    assert_eq!(webhook.emit.as_ref().expect("emit").node, "PrMerged");
    assert!(webhook.duplicates.is_empty());
}

#[test]
fn parses_spawn_mode_without_emit() {
    let source = "\
event Poke
  tag: Text
agent a
  webhook kicker
    mode: spawn
  on start
    say \"ok\"
";
    let program = parse(source).expect("parse must succeed");
    let webhook = only_webhook(&program);

    assert_eq!(webhook.name.node, "kicker");
    assert_eq!(webhook.mode.as_ref().unwrap().node, ScheduleMode::Spawn);
    assert!(webhook.emit.is_none());
}

#[test]
fn captures_duplicate_mode_option() {
    let source = "\
event E
  k: Text
agent a
  webhook t
    mode: wake
    mode: spawn
    emit: Out
  on start
    say \"ok\"
";
    let program = parse(source).expect("parse must succeed");
    let webhook = only_webhook(&program);
    let dup_names: Vec<&str> = webhook.duplicates.iter().map(|d| d.node.as_str()).collect();
    assert!(
        dup_names.contains(&"mode"),
        "expected duplicate `mode` captured, got {:?}",
        dup_names
    );
}

#[test]
fn parses_multiple_webhook_blocks_in_one_agent() {
    let source = "\
event A
  k: Text
event B
  k: Text
agent a
  webhook first
    mode: wake
    emit: A
  webhook second
    mode: wake
    emit: B
  on start
    say \"ok\"
";
    let program = parse(source).expect("parse must succeed");
    let agent = only_agent(&program);
    assert_eq!(agent.webhooks.len(), 2);
    assert_eq!(agent.webhooks[0].node.name.node, "first");
    assert_eq!(agent.webhooks[1].node.name.node, "second");
}

#[test]
fn webhook_coexists_with_correlate_and_schedule() {
    let source = "\
event PrMerged
  thread_ts: Text
agent mastermind
  memory persistent
    thread_ts: Text
  schedule heartbeat
    when: every 30s
    mode: spawn
  correlate on PrMerged.thread_ts
    mode: wake
    emit: PrMerged
  webhook pr_merged
    mode: wake
    emit: PrMerged
  on start
    say \"ok\"
  on PrMerged
    say \"hit\"
";
    let program = parse(source).expect("parse must succeed");
    let agent = only_agent(&program);
    assert_eq!(agent.schedules.len(), 1);
    assert_eq!(agent.correlates.len(), 1);
    assert_eq!(agent.webhooks.len(), 1);
}
