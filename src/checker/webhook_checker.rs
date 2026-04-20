// FORGE webhook checker — issue #335
// Compile-time invariants for the `webhook TRIGGER` block.
// Runtime wiring lives in WebhookDriver (src/runtime/webhook_driver.rs).

use std::collections::{HashMap, HashSet};

use crate::ast::{AgentDecl, EventDecl, Program, ScheduleMode, TopLevel};
use crate::diagnostic::Diagnostic;

#[derive(Debug)]
pub enum CheckError {
    /// `webhook` block is missing a `mode:` clause.
    WebhookMissingMode {
        agent: String,
        trigger: String,
        span_start: usize,
        span_end: usize,
    },
    /// `webhook` block is missing an `emit:` clause — required so every fired
    /// webhook delivers a typed event.
    WebhookMissingEmit {
        agent: String,
        trigger: String,
        span_start: usize,
        span_end: usize,
    },
    /// `emit:` references an event name that is not declared at the top level.
    WebhookUnknownEvent {
        agent: String,
        trigger: String,
        event: String,
        span_start: usize,
        span_end: usize,
    },
    /// `mode: wake` with no `on <Event>` handler and no paired `emit:` event
    /// that the agent handles — the wake would be silent.
    WebhookWakeMissingPair {
        agent: String,
        trigger: String,
        event: String,
        span_start: usize,
        span_end: usize,
    },
    /// Two webhook blocks on the same agent share the same trigger name.
    WebhookDuplicate {
        agent: String,
        trigger: String,
        span_start: usize,
        span_end: usize,
    },
    /// Same option (mode/emit) specified twice in one block.
    WebhookDuplicateOption {
        agent: String,
        trigger: String,
        option: String,
        span_start: usize,
        span_end: usize,
    },
}

impl CheckError {
    pub fn to_diagnostic(&self, file: &str) -> Diagnostic {
        match self {
            CheckError::WebhookMissingMode {
                agent,
                trigger,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!(
                    "webhook block `{}` in agent `{}` is missing a `mode:` clause",
                    trigger, agent
                ),
                *span_start..*span_end,
                "every webhook must declare how it routes the inbound request",
            )
            .with_help("add `mode: wake` (rehydrates the specialist) or `mode: spawn`"),

            CheckError::WebhookMissingEmit {
                agent,
                trigger,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!(
                    "webhook block `{}` in agent `{}` is missing an `emit:` clause",
                    trigger, agent
                ),
                *span_start..*span_end,
                "webhooks must deliver a typed event so the receiver is a declared handler",
            )
            .with_help("add `emit: SomeEvent` naming a top-level event declaration"),

            CheckError::WebhookUnknownEvent {
                agent,
                trigger,
                event,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!(
                    "webhook `{}` in agent `{}` emits unknown event `{}`",
                    trigger, agent, event
                ),
                *span_start..*span_end,
                "no `event` declaration with this name is in scope",
            )
            .with_help("declare the event first with `event <name>` at the top level"),

            CheckError::WebhookWakeMissingPair {
                agent,
                trigger,
                event,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!(
                    "webhook `{}` in agent `{}` has `mode: wake` with `emit: {}` but the agent has no `on {}` handler",
                    trigger, agent, event, event
                ),
                *span_start..*span_end,
                "`mode: wake` rehydrates the session only if the emitted event is handled there",
            )
            .with_help(format!(
                "add `on {}` to this agent, or switch to `mode: spawn`",
                event
            )),

            CheckError::WebhookDuplicate {
                agent,
                trigger,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!(
                    "duplicate webhook block `{}` in agent `{}`",
                    trigger, agent
                ),
                *span_start..*span_end,
                "a webhook with this trigger name is already declared",
            )
            .with_help("only one webhook block per trigger name is allowed in a single agent"),

            CheckError::WebhookDuplicateOption {
                agent,
                trigger,
                option,
                span_start,
                span_end,
            } => Diagnostic::error(
                file,
                format!(
                    "duplicate `{}` option in webhook block `{}` (agent `{}`)",
                    option, trigger, agent
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

    let mut seen_triggers: HashSet<String> = HashSet::new();

    for webhook_sp in &agent.webhooks {
        let webhook = &webhook_sp.node;
        let trigger_name = webhook.name.node.as_str();
        let block_span = webhook_sp.span;

        // 1. Duplicate options flagged by the parser.
        for dup in &webhook.duplicates {
            errors.push(CheckError::WebhookDuplicateOption {
                agent: agent_name.clone(),
                trigger: trigger_name.to_string(),
                option: dup.node.clone(),
                span_start: dup.span.start,
                span_end: dup.span.end,
            });
        }

        // 2. Duplicate trigger name within this agent.
        if !seen_triggers.insert(trigger_name.to_string()) {
            errors.push(CheckError::WebhookDuplicate {
                agent: agent_name.clone(),
                trigger: trigger_name.to_string(),
                span_start: block_span.start,
                span_end: block_span.end,
            });
        }

        // 3. Required `mode:`.
        if webhook.mode.is_none() {
            errors.push(CheckError::WebhookMissingMode {
                agent: agent_name.clone(),
                trigger: trigger_name.to_string(),
                span_start: block_span.start,
                span_end: block_span.end,
            });
        }

        // 4. Required `emit:`.
        match &webhook.emit {
            None => {
                errors.push(CheckError::WebhookMissingEmit {
                    agent: agent_name.clone(),
                    trigger: trigger_name.to_string(),
                    span_start: block_span.start,
                    span_end: block_span.end,
                });
            }
            Some(emit_sp) => {
                let event_name = emit_sp.node.as_str();

                // 5. Emitted event must be declared.
                if !events.contains_key(event_name) {
                    errors.push(CheckError::WebhookUnknownEvent {
                        agent: agent_name.clone(),
                        trigger: trigger_name.to_string(),
                        event: event_name.to_string(),
                        span_start: emit_sp.span.start,
                        span_end: emit_sp.span.end,
                    });
                    continue;
                }

                // 6. mode: wake requires an on-handler for the emitted event,
                // otherwise the rehydration delivers an event no handler reads.
                if let Some(mode_sp) = &webhook.mode {
                    if matches!(mode_sp.node, ScheduleMode::Wake)
                        && !handler_events.contains(event_name)
                    {
                        errors.push(CheckError::WebhookWakeMissingPair {
                            agent: agent_name.clone(),
                            trigger: trigger_name.to_string(),
                            event: event_name.to_string(),
                            span_start: mode_sp.span.start,
                            span_end: mode_sp.span.end,
                        });
                    }
                }
            }
        }
    }
}
