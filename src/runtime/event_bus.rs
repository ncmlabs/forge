// FORGE event bus — issue #19
// Typed broadcast event delivery with filtering.
// Principle VIII: every emit/delivery/failure is traced.
// Principle V: bounded channels, drop-on-full with trace (no silent loss).
// Principle II: event routing is deterministic; filters evaluated agent-side.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, RwLock};

use crate::ast::{Expr, Spanned};
use crate::runtime::confidence::ConfidentValue;
use crate::tracer::Tracer;

// ── Types ───────────────────────────────────────────────────────────────────

/// Payload delivered through the event bus.
#[derive(Debug, Clone)]
pub struct EventPayload {
    pub event_name: String,
    pub args: Vec<ConfidentValue>,
    pub source_agent: String,
    pub fields: HashMap<String, ConfidentValue>,
}

/// A registered subscriber on the bus.
pub struct Subscriber {
    pub agent_id: String,
    pub filter: Option<Spanned<Expr>>,
    pub sender: mpsc::Sender<EventPayload>,
}

/// Central event bus for inter-agent communication.
pub struct EventBus {
    subscribers: HashMap<String, Vec<Subscriber>>,
    /// Routing table for system wiring: source_agent → list of target agents.
    /// When an event is published by a source agent, it is also forwarded
    /// to all target agents in the routing table.
    routes: HashMap<String, Vec<String>>,
    tracer: Option<Tracer>,
    channel_capacity: usize,
}

pub type SharedEventBus = Arc<RwLock<EventBus>>;

// ── Implementation ──────────────────────────────────────────────────────────

const DEFAULT_CHANNEL_CAPACITY: usize = 64;

impl EventBus {
    pub fn new(tracer: Option<Tracer>) -> Self {
        Self {
            subscribers: HashMap::new(),
            routes: HashMap::new(),
            tracer,
            channel_capacity: DEFAULT_CHANNEL_CAPACITY,
        }
    }

    pub fn new_shared(tracer: Option<Tracer>) -> SharedEventBus {
        Arc::new(RwLock::new(Self::new(tracer)))
    }

    /// Register a subscriber for an event. Returns the receiving channel.
    pub fn subscribe(
        &mut self,
        event_name: &str,
        agent_id: &str,
        filter: Option<Spanned<Expr>>,
    ) -> mpsc::Receiver<EventPayload> {
        let (tx, rx) = mpsc::channel(self.channel_capacity);
        self.subscribers
            .entry(event_name.to_string())
            .or_default()
            .push(Subscriber {
                agent_id: agent_id.to_string(),
                filter,
                sender: tx,
            });
        rx
    }

    /// Add a routing rule: events from `source_agent` are forwarded to `target_agent`.
    /// Used by SystemRuntime to implement wiring (e.g., `a >> b`).
    pub fn add_route(&mut self, source_agent: &str, target_agent: &str) {
        self.routes
            .entry(source_agent.to_string())
            .or_default()
            .push(target_agent.to_string());
    }

    /// Publish an event to all subscribers matching the event name.
    /// Also applies routing rules to forward events to downstream agents.
    /// Filter evaluation is agent-side — the bus delivers to all name-matched subscribers.
    /// Returns the number of successful deliveries.
    pub fn publish(&self, payload: &EventPayload) -> usize {
        let subs = match self.subscribers.get(&payload.event_name) {
            Some(s) => s,
            None => {
                if let Some(ref t) = self.tracer {
                    t.event_emit(&payload.source_agent, &payload.event_name, 0);
                }
                return 0;
            }
        };

        let mut delivered = 0;
        for sub in subs {
            match sub.sender.try_send(payload.clone()) {
                Ok(()) => {
                    if let Some(ref t) = self.tracer {
                        t.event_delivered(&payload.event_name, &sub.agent_id);
                    }
                    delivered += 1;
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    if let Some(ref t) = self.tracer {
                        t.event_delivery_failed(&payload.event_name, &sub.agent_id, "channel full");
                    }
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    if let Some(ref t) = self.tracer {
                        t.event_delivery_failed(
                            &payload.event_name,
                            &sub.agent_id,
                            "subscriber disconnected",
                        );
                    }
                }
            }
        }

        // Apply routing rules: forward to downstream agents
        if let Some(targets) = self.routes.get(&payload.source_agent) {
            for target in targets {
                delivered += if self.forward(payload, target) { 1 } else { 0 };
            }
        }

        if let Some(ref t) = self.tracer {
            t.event_emit(&payload.source_agent, &payload.event_name, delivered);
        }
        delivered
    }

    /// Forward an event to a specific agent by ID.
    /// Returns true if the agent was found and the event was delivered.
    pub fn forward(&self, payload: &EventPayload, target_agent: &str) -> bool {
        for subs in self.subscribers.values() {
            for sub in subs {
                if sub.agent_id == target_agent {
                    match sub.sender.try_send(payload.clone()) {
                        Ok(()) => {
                            if let Some(ref t) = self.tracer {
                                t.event_delivered(&payload.event_name, target_agent);
                            }
                            return true;
                        }
                        Err(_) => {
                            if let Some(ref t) = self.tracer {
                                t.event_delivery_failed(
                                    &payload.event_name,
                                    target_agent,
                                    "forward failed",
                                );
                            }
                            return false;
                        }
                    }
                }
            }
        }
        false
    }

    /// Number of subscribers for a given event name.
    pub fn subscriber_count(&self, event_name: &str) -> usize {
        self.subscribers.get(event_name).map_or(0, |s| s.len())
    }

    /// Close the bus: drop all subscribers, closing their channels.
    /// Any agent `run()` loops will terminate when their receivers see closure.
    pub fn close(&mut self) {
        self.subscribers.clear();
    }
}

// ── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(name: &str, source: &str) -> EventPayload {
        EventPayload {
            event_name: name.to_string(),
            args: vec![],
            source_agent: source.to_string(),
            fields: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn subscribe_and_publish() {
        let mut bus = EventBus::new(None);
        let mut rx = bus.subscribe("Foo", "agent-a", None);
        let count = bus.publish(&payload("Foo", "agent-b"));
        assert_eq!(count, 1);
        let event = rx.recv().await.unwrap();
        assert_eq!(event.event_name, "Foo");
        assert_eq!(event.source_agent, "agent-b");
    }

    #[tokio::test]
    async fn publish_no_subscribers() {
        let bus = EventBus::new(None);
        let count = bus.publish(&payload("Foo", "agent-a"));
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn multiple_subscribers() {
        let mut bus = EventBus::new(None);
        let mut rx1 = bus.subscribe("Foo", "agent-a", None);
        let mut rx2 = bus.subscribe("Foo", "agent-b", None);
        let count = bus.publish(&payload("Foo", "agent-c"));
        assert_eq!(count, 2);
        assert!(rx1.recv().await.is_some());
        assert!(rx2.recv().await.is_some());
    }

    #[tokio::test]
    async fn different_events_isolated() {
        let mut bus = EventBus::new(None);
        let mut rx_foo = bus.subscribe("Foo", "agent-a", None);
        let mut rx_bar = bus.subscribe("Bar", "agent-b", None);

        bus.publish(&payload("Foo", "src"));
        bus.publish(&payload("Bar", "src"));

        let e1 = rx_foo.recv().await.unwrap();
        assert_eq!(e1.event_name, "Foo");
        let e2 = rx_bar.recv().await.unwrap();
        assert_eq!(e2.event_name, "Bar");
    }

    #[tokio::test]
    async fn forward_to_specific_agent() {
        let mut bus = EventBus::new(None);
        let mut rx_a = bus.subscribe("Foo", "agent-a", None);
        let mut rx_b = bus.subscribe("Foo", "agent-b", None);

        let ok = bus.forward(&payload("Foo", "src"), "agent-b");
        assert!(ok);

        // agent-b should have received it
        assert!(rx_b.recv().await.is_some());
        // agent-a should NOT have received it
        assert!(rx_a.try_recv().is_err());
    }

    #[tokio::test]
    async fn forward_unknown_agent() {
        let bus = EventBus::new(None);
        let ok = bus.forward(&payload("Foo", "src"), "nobody");
        assert!(!ok);
    }

    #[tokio::test]
    async fn channel_full_drops() {
        let mut bus = EventBus::new(None);
        bus.channel_capacity = 2;
        // Re-create with small capacity — subscribe uses channel_capacity at call time
        let (tx, mut rx) = mpsc::channel(2);
        bus.subscribers
            .entry("Foo".into())
            .or_default()
            .push(Subscriber {
                agent_id: "agent-a".into(),
                filter: None,
                sender: tx,
            });

        // Fill channel
        bus.publish(&payload("Foo", "src"));
        bus.publish(&payload("Foo", "src"));
        // Third should drop
        let count = bus.publish(&payload("Foo", "src"));
        assert_eq!(count, 0);

        // Drain and verify only 2 arrived
        assert!(rx.recv().await.is_some());
        assert!(rx.recv().await.is_some());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn subscriber_count_empty() {
        let bus = EventBus::new(None);
        assert_eq!(bus.subscriber_count("Foo"), 0);
    }

    #[test]
    fn subscriber_count_after_subscribe() {
        let mut bus = EventBus::new(None);
        let _rx = bus.subscribe("Foo", "agent-a", None);
        let _rx2 = bus.subscribe("Foo", "agent-b", None);
        assert_eq!(bus.subscriber_count("Foo"), 2);
        assert_eq!(bus.subscriber_count("Bar"), 0);
    }
}
