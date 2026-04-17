// FORGE schedule block parser tests — issue #331
// Golden AST assertions for all three `when` forms, both `mode`s, and optional fields.

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

fn only_schedule(program: &Program) -> &ScheduleField {
    let agent = only_agent(program);
    assert_eq!(
        agent.schedules.len(),
        1,
        "expected exactly one schedule, got {}",
        agent.schedules.len()
    );
    &agent.schedules[0].node
}

// ── `when: every DURATION` ─────────────────────────────────────────────────────

#[test]
fn parses_every_duration_with_spawn_prompt() {
    let source = "\
agent a
  schedule tick
    when: every 30s
    mode: spawn
    prompt: \"do a thing\"

  on start
    say \"ok\"
";
    let program = parse(source).expect("parse must succeed");
    let schedule = only_schedule(&program);

    assert_eq!(schedule.name.node, "tick");
    match schedule.when.as_ref().expect("when").node.clone() {
        WhenExpr::Every(d) => {
            assert_eq!(d.value, 30);
            assert!(matches!(d.unit, DurationUnit::Seconds));
        }
        other => panic!("expected Every, got {:?}", other),
    }
    assert_eq!(schedule.mode.as_ref().unwrap().node, ScheduleMode::Spawn);
    assert!(schedule.prompt.is_some());
    assert!(schedule.emit.is_none());
    assert!(schedule.precision.is_none());
    assert!(schedule.duplicates.is_empty());
}

#[test]
fn parses_every_hours_with_wake_emit() {
    let source = "\
agent a
  schedule drift
    when: every 6h
    mode: wake
    emit: DriftCheckDue

  on DriftCheckDue
    say \"d\"
";
    let program = parse(source).expect("parse must succeed");
    let schedule = only_schedule(&program);

    match schedule.when.as_ref().unwrap().node.clone() {
        WhenExpr::Every(d) => {
            assert_eq!(d.value, 6);
            assert!(matches!(d.unit, DurationUnit::Hours));
        }
        other => panic!("expected Every, got {:?}", other),
    }
    assert_eq!(schedule.mode.as_ref().unwrap().node, ScheduleMode::Wake);
    assert_eq!(schedule.emit.as_ref().unwrap().node, "DriftCheckDue");
    assert!(schedule.prompt.is_none());
}

// ── `when: daily at "HH:MM"` ───────────────────────────────────────────────────

#[test]
fn parses_daily_at_time() {
    let source = "\
agent a
  schedule morning
    when: daily at \"09:00\"
    mode: spawn
    prompt: \"morning roll-up\"

  on start
    say \"ok\"
";
    let program = parse(source).expect("parse must succeed");
    let schedule = only_schedule(&program);

    match schedule.when.as_ref().unwrap().node.clone() {
        WhenExpr::DailyAt(tod) => {
            assert_eq!(tod.hour, 9);
            assert_eq!(tod.minute, 0);
        }
        other => panic!("expected DailyAt, got {:?}", other),
    }
}

#[test]
fn parses_daily_at_single_digit_hour() {
    let source = "\
agent a
  schedule s
    when: daily at \"7:45\"
    mode: spawn
    prompt: \"p\"

  on start
    say \"ok\"
";
    let program = parse(source).expect("parse must succeed");
    let schedule = only_schedule(&program);
    match schedule.when.as_ref().unwrap().node.clone() {
        WhenExpr::DailyAt(tod) => {
            assert_eq!(tod.hour, 7);
            assert_eq!(tod.minute, 45);
        }
        other => panic!("expected DailyAt, got {:?}", other),
    }
}

// ── `when: cron "..."` ─────────────────────────────────────────────────────────

#[test]
fn parses_cron_expression() {
    let source = "\
agent a
  schedule s
    when: cron \"0 9 * * *\"
    mode: spawn
    prompt: \"nine am\"

  on start
    say \"ok\"
";
    let program = parse(source).expect("parse must succeed");
    let schedule = only_schedule(&program);
    match schedule.when.as_ref().unwrap().node.clone() {
        WhenExpr::Cron(s) => assert_eq!(s, "0 9 * * *"),
        other => panic!("expected Cron, got {:?}", other),
    }
}

// ── Precision + multiple schedules ─────────────────────────────────────────────

#[test]
fn parses_precision_high() {
    let source = "\
agent a
  schedule s
    when: every 1s
    mode: wake
    emit: Tick
    precision: high

  on Tick
    say \"ok\"
";
    let program = parse(source).expect("parse must succeed");
    let schedule = only_schedule(&program);
    assert_eq!(schedule.precision.as_ref().unwrap().node, Precision::High);
}

#[test]
fn parses_multiple_schedules_in_one_agent() {
    let source = "\
agent a
  schedule one
    when: every 30s
    mode: spawn
    prompt: \"one\"
  schedule two
    when: every 6h
    mode: wake
    emit: TwoDue

  on TwoDue
    say \"ok\"
";
    let program = parse(source).expect("parse must succeed");
    let agent = only_agent(&program);
    assert_eq!(agent.schedules.len(), 2);
    assert_eq!(agent.schedules[0].node.name.node, "one");
    assert_eq!(agent.schedules[1].node.name.node, "two");
}

// ── Duplicate options are captured (not rejected) ──────────────────────────────

#[test]
fn parser_captures_duplicate_options_into_side_channel() {
    let source = "\
agent a
  schedule s
    when: every 5m
    when: every 10m
    mode: spawn
    prompt: \"p\"

  on start
    say \"ok\"
";
    let program = parse(source).expect("parse must succeed");
    let schedule = only_schedule(&program);
    // The first `when` wins; the second is captured for the checker.
    assert_eq!(schedule.duplicates.len(), 1);
    assert_eq!(schedule.duplicates[0].node, "when");
    match schedule.when.as_ref().unwrap().node.clone() {
        WhenExpr::Every(d) => assert_eq!(d.value, 5),
        other => panic!("expected first Every(5m), got {:?}", other),
    }
}

// ── Schedule slots between timers and subscribes ───────────────────────────────

#[test]
fn schedule_coexists_with_timer_and_subscribe() {
    let source = "\
event Heartbeat
  at: Text

agent a
  timer t: 5m
  schedule s
    when: every 1h
    mode: wake
    emit: Ping
  subscribe Heartbeat

  on Ping
    say \"ping\"
  on Heartbeat(at: Text)
    say at
";
    let program = parse(source).expect("parse must succeed");
    let agent = only_agent(&program);
    assert_eq!(agent.timers.len(), 1);
    assert_eq!(agent.schedules.len(), 1);
    assert_eq!(agent.subscriptions.len(), 1);
}

// ── Shape errors: reject malformed blocks at parse time ────────────────────────

#[test]
fn empty_schedule_block_is_parse_error() {
    let source = "\
agent a
  schedule s

  on start
    say \"ok\"
";
    // Grammar requires at least one option — an empty block should fail to parse.
    assert!(parse(source).is_err());
}

#[test]
fn malformed_time_literal_is_parse_error() {
    // Missing quotes around the time literal.
    let source = "\
agent a
  schedule s
    when: daily at 09:00
    mode: spawn
    prompt: \"p\"

  on start
    say \"ok\"
";
    assert!(parse(source).is_err());
}
