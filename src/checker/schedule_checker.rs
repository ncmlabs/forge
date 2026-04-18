// FORGE schedule checker — issue #331
// Enforces compile-time invariants for the new `schedule` block.
// Runtime wiring lives in the WakeService (issue #332).

use std::collections::HashMap;

use crate::ast::{AgentDecl, Duration, Program, ScheduleMode, Span, TopLevel, WhenExpr};
use crate::diagnostic::Diagnostic;

#[derive(Debug)]
pub enum CheckError {
    /// `schedule` block has no `when:` option.
    ScheduleMissingWhen {
        agent: String,
        name: String,
        span_start: usize,
        span_end: usize,
    },
    /// `schedule` block has no `mode:` option.
    ScheduleMissingMode {
        agent: String,
        name: String,
        span_start: usize,
        span_end: usize,
    },
    /// `mode: spawn` declared without a `prompt:` option.
    ScheduleSpawnMissingPrompt {
        agent: String,
        name: String,
        span_start: usize,
        span_end: usize,
    },
    /// `mode: wake` declared without `emit:` and without a paired `on <name>.tick` handler.
    ScheduleWakeMissingPair {
        agent: String,
        name: String,
        span_start: usize,
        span_end: usize,
    },
    /// Two `schedule` blocks in one agent share the same name.
    ScheduleDuplicateName {
        agent: String,
        name: String,
        first_span_start: usize,
        first_span_end: usize,
        dup_span_start: usize,
        dup_span_end: usize,
    },
    /// Same option (when/mode/prompt/emit/precision) specified twice in one block.
    ScheduleDuplicateOption {
        agent: String,
        name: String,
        option: String,
        span_start: usize,
        span_end: usize,
    },
    /// Cron expression failed to parse as a 5-field Unix cron string.
    ScheduleInvalidCron {
        agent: String,
        name: String,
        expr: String,
        reason: String,
        span_start: usize,
        span_end: usize,
    },
    /// `daily at` literal has hour > 23 or minute > 59.
    ScheduleInvalidTime {
        agent: String,
        name: String,
        hour: u8,
        minute: u8,
        span_start: usize,
        span_end: usize,
    },
    /// `every 0s` / `every 0m` / etc. — durations must be > 0.
    ScheduleZeroDuration {
        agent: String,
        name: String,
        span_start: usize,
        span_end: usize,
    },
    /// Schedule name collides with a timer name or handler event name in the same agent.
    ScheduleNameCollision {
        agent: String,
        name: String,
        collides_with: String,
        span_start: usize,
        span_end: usize,
    },

    // ── Warnings ─────────────────────────────────────────────────
    /// `mode: spawn` with extraneous `emit:` (emit is meaningful only for `mode: wake`).
    ScheduleSpawnHasEmit {
        agent: String,
        name: String,
        span_start: usize,
        span_end: usize,
    },
    /// `mode: wake` with extraneous `prompt:` (prompt is meaningful only for `mode: spawn`).
    ScheduleWakeHasPrompt {
        agent: String,
        name: String,
        span_start: usize,
        span_end: usize,
    },
}

impl CheckError {
    pub fn to_diagnostic(&self, file: &str) -> Diagnostic {
        match self {
            CheckError::ScheduleMissingWhen {
                agent,
                name,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!(
                    "schedule `{}` in agent `{}` is missing a `when:` clause",
                    name, agent
                ),
                *span_start..*span_end,
                "every schedule must declare when it fires",
            )
            .with_help("add one of: `when: daily at \"HH:MM\"`, `when: every <duration>`, or `when: cron \"...\"`"),

            CheckError::ScheduleMissingMode {
                agent,
                name,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!(
                    "schedule `{}` in agent `{}` is missing a `mode:` clause",
                    name, agent
                ),
                *span_start..*span_end,
                "every schedule must declare how it fires",
            )
            .with_help("add `mode: spawn` (with a `prompt:`) or `mode: wake` (with an `emit:` or paired `.tick` handler)"),

            CheckError::ScheduleSpawnMissingPrompt {
                agent,
                name,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!(
                    "schedule `{}` in agent `{}` has `mode: spawn` but no `prompt:`",
                    name, agent
                ),
                *span_start..*span_end,
                "`mode: spawn` starts a stateless turn and requires a prompt",
            )
            .with_help("add `prompt: \"...\"` to describe what the spawned turn should do"),

            CheckError::ScheduleWakeMissingPair {
                agent,
                name,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!(
                    "schedule `{}` in agent `{}` has `mode: wake` but no `emit:` and no `on {}.tick` handler",
                    name, agent, name
                ),
                *span_start..*span_end,
                "`mode: wake` must deliver an event — declare one or handle the default tick",
            )
            .with_help("either add `emit: SomeEvent` (and an `on SomeEvent` handler) or add `on {name}.tick` to this agent".replace("{name}", name)),

            CheckError::ScheduleDuplicateName {
                agent,
                name,
                dup_span_start,
                dup_span_end,
                ..
            } => Diagnostic::error(
                file,
                format!(
                    "duplicate schedule name `{}` in agent `{}`",
                    name, agent
                ),
                *dup_span_start..*dup_span_end,
                "a schedule with this name is already declared",
            )
            .with_help("rename one of the schedules — names must be unique within an agent"),

            CheckError::ScheduleDuplicateOption {
                agent,
                name,
                option,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!(
                    "duplicate `{}` option in schedule `{}` (agent `{}`)",
                    option, name, agent
                ),
                *span_start..*span_end,
                "this option is already specified above",
            )
            .with_help("remove the duplicate line"),

            CheckError::ScheduleInvalidCron {
                agent,
                name,
                expr,
                reason,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!(
                    "invalid cron expression in schedule `{}` (agent `{}`): {}",
                    name, agent, reason
                ),
                *span_start..*span_end,
                format!("`{}` is not a valid cron string", expr),
            )
            .with_help("FORGE cron uses standard 5-field Unix syntax: `m h dom mon dow` (e.g. `0 9 * * *` for 09:00 daily)"),

            CheckError::ScheduleInvalidTime {
                agent,
                name,
                hour,
                minute,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!(
                    "invalid time literal in schedule `{}` (agent `{}`): {:02}:{:02}",
                    name, agent, hour, minute
                ),
                *span_start..*span_end,
                "hour must be 0–23 and minute must be 0–59",
            )
            .with_help("use 24-hour format, e.g. `\"09:00\"` or `\"23:45\"`"),

            CheckError::ScheduleZeroDuration {
                agent,
                name,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!(
                    "schedule `{}` in agent `{}` has `when: every 0...` — duration must be positive",
                    name, agent
                ),
                *span_start..*span_end,
                "a zero-length interval cannot fire",
            )
            .with_help("pick a positive duration like `every 30s`, `every 6h`, or `every 1d`"),

            CheckError::ScheduleNameCollision {
                agent,
                name,
                collides_with,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!(
                    "schedule name `{}` in agent `{}` collides with {}",
                    name, agent, collides_with
                ),
                *span_start..*span_end,
                "this name is already used in the same agent",
            )
            .with_help("rename the schedule — names must be unique across timers, schedules, and handler events within an agent"),

            CheckError::ScheduleSpawnHasEmit {
                agent,
                name,
                span_start,
                span_end,
            } => Diagnostic::warning(
                file,
                format!(
                    "schedule `{}` in agent `{}` has `mode: spawn` with an extraneous `emit:`",
                    name, agent
                ),
                *span_start..*span_end,
                "`emit:` is ignored under `mode: spawn`",
            )
            .with_help("remove `emit:`, or change to `mode: wake`"),

            CheckError::ScheduleWakeHasPrompt {
                agent,
                name,
                span_start,
                span_end,
            } => Diagnostic::warning(
                file,
                format!(
                    "schedule `{}` in agent `{}` has `mode: wake` with an extraneous `prompt:`",
                    name, agent
                ),
                *span_start..*span_end,
                "`prompt:` is ignored under `mode: wake`",
            )
            .with_help("remove `prompt:`, or change to `mode: spawn`"),
        }
    }
}

// ── Checker entry ────────────────────────────────────────────────

pub fn check(program: &Program, file: &str) -> Vec<Diagnostic> {
    let mut errors: Vec<CheckError> = Vec::new();
    for item in &program.items {
        if let TopLevel::Agent(agent) = &item.node {
            check_agent(agent, &mut errors);
        }
    }
    errors.iter().map(|e| e.to_diagnostic(file)).collect()
}

fn check_agent(agent: &AgentDecl, errors: &mut Vec<CheckError>) {
    let agent_name = &agent.name.node;

    // Collect sibling names for collision detection.
    let timer_names: HashMap<&str, Span> = agent
        .timers
        .iter()
        .map(|t| (t.node.name.node.as_str(), t.node.name.span))
        .collect();
    let handler_events: HashMap<&str, Span> = agent
        .handlers
        .iter()
        .map(|h| (h.node.event.node.as_str(), h.node.event.span))
        .collect();

    // Track seen schedule names for duplicate detection.
    let mut seen_names: HashMap<&str, (usize, usize)> = HashMap::new();

    for schedule_sp in &agent.schedules {
        let schedule = &schedule_sp.node;
        let schedule_name = schedule.name.node.as_str();
        let name_span = schedule.name.span;

        // 1. Duplicate schedule name within this agent.
        if let Some(&(first_start, first_end)) = seen_names.get(schedule_name) {
            errors.push(CheckError::ScheduleDuplicateName {
                agent: agent_name.clone(),
                name: schedule_name.to_string(),
                first_span_start: first_start,
                first_span_end: first_end,
                dup_span_start: name_span.start,
                dup_span_end: name_span.end,
            });
        } else {
            seen_names.insert(schedule_name, (name_span.start, name_span.end));
        }

        // 2. Duplicate options captured by the parser.
        for dup in &schedule.duplicates {
            errors.push(CheckError::ScheduleDuplicateOption {
                agent: agent_name.clone(),
                name: schedule_name.to_string(),
                option: dup.node.clone(),
                span_start: dup.span.start,
                span_end: dup.span.end,
            });
        }

        // 3. Required `when:` and validation of its contents.
        match &schedule.when {
            None => {
                errors.push(CheckError::ScheduleMissingWhen {
                    agent: agent_name.clone(),
                    name: schedule_name.to_string(),
                    span_start: schedule_sp.span.start,
                    span_end: schedule_sp.span.end,
                });
            }
            Some(when_sp) => {
                validate_when(
                    agent_name,
                    schedule_name,
                    when_sp.node.clone(),
                    when_sp.span,
                    errors,
                );
            }
        }

        // 4. Required `mode:` + coherence with prompt/emit.
        match &schedule.mode {
            None => {
                errors.push(CheckError::ScheduleMissingMode {
                    agent: agent_name.clone(),
                    name: schedule_name.to_string(),
                    span_start: schedule_sp.span.start,
                    span_end: schedule_sp.span.end,
                });
            }
            Some(mode_sp) => match mode_sp.node {
                ScheduleMode::Spawn => {
                    if schedule.prompt.is_none() {
                        errors.push(CheckError::ScheduleSpawnMissingPrompt {
                            agent: agent_name.clone(),
                            name: schedule_name.to_string(),
                            span_start: mode_sp.span.start,
                            span_end: mode_sp.span.end,
                        });
                    }
                    if let Some(emit_sp) = &schedule.emit {
                        errors.push(CheckError::ScheduleSpawnHasEmit {
                            agent: agent_name.clone(),
                            name: schedule_name.to_string(),
                            span_start: emit_sp.span.start,
                            span_end: emit_sp.span.end,
                        });
                    }
                }
                ScheduleMode::Wake => {
                    let tick_event = format!("{}.tick", schedule_name);
                    let has_paired_handler = handler_events.contains_key(tick_event.as_str());
                    if schedule.emit.is_none() && !has_paired_handler {
                        errors.push(CheckError::ScheduleWakeMissingPair {
                            agent: agent_name.clone(),
                            name: schedule_name.to_string(),
                            span_start: mode_sp.span.start,
                            span_end: mode_sp.span.end,
                        });
                    }
                    if let Some(prompt_sp) = &schedule.prompt {
                        errors.push(CheckError::ScheduleWakeHasPrompt {
                            agent: agent_name.clone(),
                            name: schedule_name.to_string(),
                            span_start: prompt_sp.span.start,
                            span_end: prompt_sp.span.end,
                        });
                    }
                }
            },
        }

        // 5. Cross-kind name collision (timer / handler event).
        if let Some(timer_span) = timer_names.get(schedule_name) {
            // Only report if the timer was declared first (otherwise timer side will be the duplicate).
            if timer_span.start < name_span.start {
                errors.push(CheckError::ScheduleNameCollision {
                    agent: agent_name.clone(),
                    name: schedule_name.to_string(),
                    collides_with: format!("timer `{}`", schedule_name),
                    span_start: name_span.start,
                    span_end: name_span.end,
                });
            }
        }
        if let Some(handler_span) = handler_events.get(schedule_name) {
            if handler_span.start < name_span.start {
                errors.push(CheckError::ScheduleNameCollision {
                    agent: agent_name.clone(),
                    name: schedule_name.to_string(),
                    collides_with: format!("handler event `{}`", schedule_name),
                    span_start: name_span.start,
                    span_end: name_span.end,
                });
            }
        }
    }
}

fn validate_when(
    agent_name: &str,
    schedule_name: &str,
    when: WhenExpr,
    when_span: Span,
    errors: &mut Vec<CheckError>,
) {
    match when {
        WhenExpr::DailyAt(tod) => {
            if tod.hour > 23 || tod.minute > 59 {
                errors.push(CheckError::ScheduleInvalidTime {
                    agent: agent_name.to_string(),
                    name: schedule_name.to_string(),
                    hour: tod.hour,
                    minute: tod.minute,
                    span_start: when_span.start,
                    span_end: when_span.end,
                });
            }
        }
        WhenExpr::Every(Duration { value, .. }) => {
            if value == 0 {
                errors.push(CheckError::ScheduleZeroDuration {
                    agent: agent_name.to_string(),
                    name: schedule_name.to_string(),
                    span_start: when_span.start,
                    span_end: when_span.end,
                });
            }
        }
        WhenExpr::Cron(expr) => {
            // FORGE cron is strict 5-field Unix: m h dom mon dow.
            let parser = croner::parser::CronParser::builder()
                .seconds(croner::parser::Seconds::Disallowed)
                .year(croner::parser::Year::Disallowed)
                .build();
            if let Err(e) = parser.parse(&expr) {
                errors.push(CheckError::ScheduleInvalidCron {
                    agent: agent_name.to_string(),
                    name: schedule_name.to_string(),
                    expr,
                    reason: format!("{:?}", e),
                    span_start: when_span.start,
                    span_end: when_span.end,
                });
            }
        }
    }
}
