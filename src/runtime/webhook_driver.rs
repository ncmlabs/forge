// FORGE webhook driver — issue #335
// Third driver on `WakeService` after `CronDriver` (tick-driven, #332) and
// `CorrelationDriver` (event-driven, #334). Event source is an inbound HTTP
// request to `POST /wake/{agent}/{trigger}`; HMAC verification and rate
// limiting happen in the HTTP handler. This driver holds the static registry
// of declared `webhook` blocks and tells the handler how to route a verified
// request (which event to emit, whether to rehydrate the specialist first).

use std::collections::HashMap;
use std::sync::Arc;

use crate::ast::{AgentDecl, Program, ScheduleMode, TopLevel};

/// One declared webhook trigger on a specific agent. Built at program load
/// from `webhook TRIGGER_NAME { mode, emit }` blocks in the AST.
#[derive(Debug, Clone)]
pub struct WebhookRegistration {
    pub agent: String,
    pub trigger: String,
    pub mode: ScheduleMode,
    pub emit_event: String,
}

/// Static dispatcher for declared webhook triggers. Cheap to clone — internals
/// are behind `Arc`. Unlike `CorrelationDriver` there is no runtime storage
/// lookup at match time; per-`(agent, trigger)` HMAC secrets are consulted at
/// the HTTP layer via `ForgeStorage::lookup_wake_secret`.
#[derive(Clone, Default)]
pub struct WebhookDriver {
    registry: Arc<HashMap<(String, String), WebhookRegistration>>,
}

impl WebhookDriver {
    pub fn new(registrations: Vec<WebhookRegistration>) -> Self {
        let mut registry = HashMap::new();
        for reg in registrations {
            registry.insert((reg.agent.clone(), reg.trigger.clone()), reg);
        }
        Self {
            registry: Arc::new(registry),
        }
    }

    /// Return the registration for `(agent, trigger)`, or `None` if no such
    /// webhook block is declared. Absence here is a 404; a signature mismatch
    /// is a 401 (a different code path).
    pub fn match_webhook(&self, agent: &str, trigger: &str) -> Option<&WebhookRegistration> {
        self.registry.get(&(agent.to_string(), trigger.to_string()))
    }

    /// Count of registered triggers — exposed for the startup log line.
    pub fn len(&self) -> usize {
        self.registry.len()
    }

    pub fn is_empty(&self) -> bool {
        self.registry.is_empty()
    }
}

/// Build a driver by walking every `webhook` block in the program. The
/// `checker::webhook_checker` pass has already validated each block by the
/// time this runs, so we silently drop blocks missing `mode:` or `emit:`
/// rather than re-surfacing errors.
pub fn build_from_program(program: &Program) -> WebhookDriver {
    let mut regs: Vec<WebhookRegistration> = Vec::new();
    for item in &program.items {
        if let TopLevel::Agent(agent) = &item.node {
            collect_agent_webhooks(agent, &mut regs);
        }
    }
    WebhookDriver::new(regs)
}

fn collect_agent_webhooks(agent: &AgentDecl, out: &mut Vec<WebhookRegistration>) {
    for wh_sp in &agent.webhooks {
        let wh = &wh_sp.node;
        let (Some(mode_sp), Some(emit_sp)) = (&wh.mode, &wh.emit) else {
            continue;
        };
        out.push(WebhookRegistration {
            agent: agent.name.node.clone(),
            trigger: wh.name.node.clone(),
            mode: mode_sp.node,
            emit_event: emit_sp.node.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(agent: &str, trigger: &str, emit: &str, mode: ScheduleMode) -> WebhookRegistration {
        WebhookRegistration {
            agent: agent.to_string(),
            trigger: trigger.to_string(),
            mode,
            emit_event: emit.to_string(),
        }
    }

    #[test]
    fn match_returns_hit_for_registered_pair() {
        let driver = WebhookDriver::new(vec![reg(
            "mastermind",
            "pr_merged",
            "PrMerged",
            ScheduleMode::Wake,
        )]);
        let m = driver
            .match_webhook("mastermind", "pr_merged")
            .expect("hit");
        assert_eq!(m.agent, "mastermind");
        assert_eq!(m.trigger, "pr_merged");
        assert_eq!(m.emit_event, "PrMerged");
        assert_eq!(m.mode, ScheduleMode::Wake);
    }

    #[test]
    fn match_returns_none_for_unknown_agent() {
        let driver = WebhookDriver::new(vec![reg(
            "mastermind",
            "pr_merged",
            "PrMerged",
            ScheduleMode::Wake,
        )]);
        assert!(driver.match_webhook("other", "pr_merged").is_none());
    }

    #[test]
    fn match_returns_none_for_unknown_trigger_on_known_agent() {
        let driver = WebhookDriver::new(vec![reg(
            "mastermind",
            "pr_merged",
            "PrMerged",
            ScheduleMode::Wake,
        )]);
        assert!(driver.match_webhook("mastermind", "other").is_none());
    }

    #[test]
    fn multiple_agents_are_isolated() {
        let driver = WebhookDriver::new(vec![
            reg("a", "t", "EventA", ScheduleMode::Wake),
            reg("b", "t", "EventB", ScheduleMode::Spawn),
        ]);
        assert_eq!(driver.match_webhook("a", "t").unwrap().emit_event, "EventA");
        assert_eq!(driver.match_webhook("b", "t").unwrap().emit_event, "EventB");
        assert_eq!(driver.len(), 2);
    }

    #[test]
    fn empty_driver_matches_nothing() {
        let driver = WebhookDriver::default();
        assert!(driver.is_empty());
        assert!(driver.match_webhook("a", "t").is_none());
    }

    #[test]
    fn build_from_program_parses_declared_webhooks() {
        use crate::parser::parse;
        let source = "\
event PrMerged
  repo: Text
agent mastermind
  webhook pr_merged
    mode: wake
    emit: PrMerged
  webhook missing_emit
    mode: wake
  on PrMerged
    say \"ok\"
";
        let program = parse(source).expect("parse");
        let driver = build_from_program(&program);
        // Only the complete webhook registers; the incomplete one is
        // silently skipped (the checker surfaces the error separately).
        assert_eq!(driver.len(), 1);
        let hit = driver
            .match_webhook("mastermind", "pr_merged")
            .expect("hit");
        assert_eq!(hit.emit_event, "PrMerged");
        assert_eq!(hit.mode, ScheduleMode::Wake);
        assert!(driver.match_webhook("mastermind", "missing_emit").is_none());
    }
}
