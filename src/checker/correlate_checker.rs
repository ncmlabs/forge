// FORGE correlate checker — issue #334
// Compile-time invariants for the `correlate on Event.field` block.
// Runtime wiring lives in CorrelationDriver (src/runtime/correlation_driver.rs).

use std::collections::{HashMap, HashSet};

use crate::ast::{AgentDecl, EventDecl, Program, ScheduleMode, TopLevel, TypeName};
use crate::diagnostic::Diagnostic;

#[derive(Debug)]
pub enum CheckError {
    /// `correlate on Event.field` references an event name that is not declared.
    CorrelateUnknownEvent {
        agent: String,
        event: String,
        span_start: usize,
        span_end: usize,
    },
    /// The field is not a member of the referenced event.
    CorrelateUnknownField {
        agent: String,
        event: String,
        field: String,
        span_start: usize,
        span_end: usize,
    },
    /// The field exists but is not of type `Text`.
    CorrelateFieldNotText {
        agent: String,
        event: String,
        field: String,
        actual_type: String,
        span_start: usize,
        span_end: usize,
    },
    /// The agent does not declare a `memory persistent` field with the same name
    /// and `Text` type.
    CorrelateMissingMemoryField {
        agent: String,
        field: String,
        span_start: usize,
        span_end: usize,
    },
    /// `correlate` block has no `mode:` option.
    CorrelateMissingMode {
        agent: String,
        event: String,
        field: String,
        span_start: usize,
        span_end: usize,
    },
    /// `mode: wake` with neither `emit:` nor a paired `on <Event>` handler.
    CorrelateWakeMissingPair {
        agent: String,
        event: String,
        field: String,
        span_start: usize,
        span_end: usize,
    },
    /// Two correlate blocks in one agent share the same (event, field) pair.
    CorrelateDuplicate {
        agent: String,
        event: String,
        field: String,
        span_start: usize,
        span_end: usize,
    },
    /// Same option (mode/emit) specified twice in one block.
    CorrelateDuplicateOption {
        agent: String,
        event: String,
        field: String,
        option: String,
        span_start: usize,
        span_end: usize,
    },
}

impl CheckError {
    pub fn to_diagnostic(&self, file: &str) -> Diagnostic {
        match self {
            CheckError::CorrelateUnknownEvent {
                agent,
                event,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!(
                    "correlate block in agent `{}` references unknown event `{}`",
                    agent, event
                ),
                *span_start..*span_end,
                "no `event` declaration with this name is in scope",
            )
            .with_help("declare the event first with `event <name>` at the top level"),

            CheckError::CorrelateUnknownField {
                agent,
                event,
                field,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!(
                    "event `{}` has no field `{}` (required by correlate block in agent `{}`)",
                    event, field, agent
                ),
                *span_start..*span_end,
                "the correlation field must exist on the event",
            )
            .with_help("either add the field to the event, or correlate on an existing field"),

            CheckError::CorrelateFieldNotText {
                agent,
                event,
                field,
                actual_type,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!(
                    "correlate field `{}.{}` must be `Text` but is `{}` (in agent `{}`)",
                    event, field, actual_type, agent
                ),
                *span_start..*span_end,
                "correlation keys are compared as strings and must be Text-typed",
            )
            .with_help("change the event field to `Text`, or correlate on a different field"),

            CheckError::CorrelateMissingMemoryField {
                agent,
                field,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!(
                    "agent `{}` correlates on field `{}` but has no matching `memory persistent` field with type `Text`",
                    agent, field
                ),
                *span_start..*span_end,
                "the specialist needs to persist the correlation key between sessions",
            )
            .with_help(format!(
                "add `{}: Text` to the `memory persistent` block",
                field
            )),

            CheckError::CorrelateMissingMode {
                agent,
                event,
                field,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!(
                    "correlate block `{}.{}` in agent `{}` is missing a `mode:` clause",
                    event, field, agent
                ),
                *span_start..*span_end,
                "every correlate must declare how it routes matched events",
            )
            .with_help("add `mode: wake` (rehydrates the specialist) or `mode: spawn`"),

            CheckError::CorrelateWakeMissingPair {
                agent,
                event,
                field,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!(
                    "correlate `{}.{}` in agent `{}` has `mode: wake` but no `emit:` and no `on {}` handler",
                    event, field, agent, event
                ),
                *span_start..*span_end,
                "`mode: wake` must deliver an event — declare one or handle the inbound event directly",
            )
            .with_help(format!(
                "either add `emit: SomeEvent` (and an `on SomeEvent` handler) or add `on {}` to this agent",
                event
            )),

            CheckError::CorrelateDuplicate {
                agent,
                event,
                field,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!(
                    "duplicate correlate block `{}.{}` in agent `{}`",
                    event, field, agent
                ),
                *span_start..*span_end,
                "a correlate block for this event/field is already declared",
            )
            .with_help("only one correlate block per (event, field) pair is allowed in a single agent"),

            CheckError::CorrelateDuplicateOption {
                agent,
                event,
                field,
                option,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!(
                    "duplicate `{}` option in correlate block `{}.{}` (agent `{}`)",
                    option, event, field, agent
                ),
                *span_start..*span_end,
                "this option is already specified above",
            )
            .with_help("remove the duplicate line"),
        }
    }
}

// ── Checker entry ────────────────────────────────────────────────

pub fn check(program: &Program, file: &str) -> Vec<Diagnostic> {
    // Collect declared events by name with their field types.
    let mut events: HashMap<String, &EventDecl> = HashMap::new();
    for item in &program.items {
        if let TopLevel::Event(ev) = &item.node {
            events.insert(ev.name.node.clone(), ev);
        }
    }

    let mut errors: Vec<CheckError> = Vec::new();
    for item in &program.items {
        if let TopLevel::Agent(agent) = &item.node {
            check_agent(agent, &events, &mut errors);
        }
    }
    errors.iter().map(|e| e.to_diagnostic(file)).collect()
}

fn type_label(t: &TypeName) -> String {
    match t {
        TypeName::Text => "Text".into(),
        TypeName::Number => "Number".into(),
        TypeName::Bool => "Bool".into(),
        TypeName::Html => "Html".into(),
        TypeName::Custom(s) => s.clone(),
        other => format!("{:?}", other),
    }
}

fn is_text(t: &TypeName) -> bool {
    matches!(t, TypeName::Text)
}

fn check_agent(
    agent: &AgentDecl,
    events: &HashMap<String, &EventDecl>,
    errors: &mut Vec<CheckError>,
) {
    let agent_name = &agent.name.node;
    let handler_events: HashSet<&str> = agent
        .handlers
        .iter()
        .map(|h| h.node.event.node.as_str())
        .collect();

    // Track (event, field) for duplicate detection across blocks.
    let mut seen_pairs: HashSet<(String, String)> = HashSet::new();

    for correlate_sp in &agent.correlates {
        let correlate = &correlate_sp.node;
        let event_name = correlate.event_type.node.as_str();
        let field_name = correlate.field_name.node.as_str();
        let block_span = correlate_sp.span;

        // 1. Duplicate options flagged by the parser.
        for dup in &correlate.duplicates {
            errors.push(CheckError::CorrelateDuplicateOption {
                agent: agent_name.clone(),
                event: event_name.to_string(),
                field: field_name.to_string(),
                option: dup.node.clone(),
                span_start: dup.span.start,
                span_end: dup.span.end,
            });
        }

        // 2. Duplicate (event, field) pair within this agent.
        let pair_key = (event_name.to_string(), field_name.to_string());
        if !seen_pairs.insert(pair_key) {
            errors.push(CheckError::CorrelateDuplicate {
                agent: agent_name.clone(),
                event: event_name.to_string(),
                field: field_name.to_string(),
                span_start: block_span.start,
                span_end: block_span.end,
            });
        }

        // 3. Event must be declared and the field must exist and be Text.
        match events.get(event_name) {
            None => {
                errors.push(CheckError::CorrelateUnknownEvent {
                    agent: agent_name.clone(),
                    event: event_name.to_string(),
                    span_start: correlate.event_type.span.start,
                    span_end: correlate.event_type.span.end,
                });
            }
            Some(ev) => {
                let matching_field = ev
                    .fields
                    .iter()
                    .find(|f| f.node.name == field_name)
                    .map(|f| &f.node.type_name.node);
                match matching_field {
                    None => {
                        errors.push(CheckError::CorrelateUnknownField {
                            agent: agent_name.clone(),
                            event: event_name.to_string(),
                            field: field_name.to_string(),
                            span_start: correlate.field_name.span.start,
                            span_end: correlate.field_name.span.end,
                        });
                    }
                    Some(t) if !is_text(t) => {
                        errors.push(CheckError::CorrelateFieldNotText {
                            agent: agent_name.clone(),
                            event: event_name.to_string(),
                            field: field_name.to_string(),
                            actual_type: type_label(t),
                            span_start: correlate.field_name.span.start,
                            span_end: correlate.field_name.span.end,
                        });
                    }
                    Some(_) => {}
                }
            }
        }

        // 4. Agent must declare a matching `memory persistent` field with type Text.
        let mem_ok = agent.memory_persistent
            && agent
                .memory
                .iter()
                .any(|m| m.node.name == field_name && is_text(&m.node.type_name.node));
        if !mem_ok {
            errors.push(CheckError::CorrelateMissingMemoryField {
                agent: agent_name.clone(),
                field: field_name.to_string(),
                span_start: correlate.field_name.span.start,
                span_end: correlate.field_name.span.end,
            });
        }

        // 5. Required `mode:` + wake/emit coherence.
        match &correlate.mode {
            None => {
                errors.push(CheckError::CorrelateMissingMode {
                    agent: agent_name.clone(),
                    event: event_name.to_string(),
                    field: field_name.to_string(),
                    span_start: block_span.start,
                    span_end: block_span.end,
                });
            }
            Some(mode_sp) => {
                if matches!(mode_sp.node, ScheduleMode::Wake)
                    && correlate.emit.is_none()
                    && !handler_events.contains(event_name)
                {
                    errors.push(CheckError::CorrelateWakeMissingPair {
                        agent: agent_name.clone(),
                        event: event_name.to_string(),
                        field: field_name.to_string(),
                        span_start: mode_sp.span.start,
                        span_end: mode_sp.span.end,
                    });
                }
            }
        }
    }
}
