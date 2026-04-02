// FORGE runtime state machine — issue #17
// Lifecycle enforcement at runtime with HashMap-based transition graph.

use std::collections::HashMap;

use crate::ast::StatesDecl;

// ── Types ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TransitionEdge {
    pub to: String,
    pub condition: Option<String>,
}

#[derive(Debug, Clone)]
pub enum StateError {
    IllegalTransition { from: String, to: String },
}

impl std::fmt::Display for StateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StateError::IllegalTransition { from, to } => {
                write!(f, "no transition from '{}' to '{}'", from, to)
            }
        }
    }
}

// ── State Machine ───────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct StateMachine {
    pub current: String,
    pub graph: HashMap<String, Vec<TransitionEdge>>,
}

impl StateMachine {
    /// Build from a `StatesDecl`. Initial state = first `from` in the first transition.
    pub fn new(decl: &StatesDecl) -> Self {
        let initial = decl.transitions.first()
            .map(|t| t.node.from.node.clone())
            .unwrap_or_else(|| "initial".to_string());

        let mut graph: HashMap<String, Vec<TransitionEdge>> = HashMap::new();
        for t in &decl.transitions {
            graph.entry(t.node.from.node.clone())
                .or_default()
                .push(TransitionEdge {
                    to: t.node.to.node.clone(),
                    condition: None, // conditions not evaluated at runtime yet
                });
        }

        Self { current: initial, graph }
    }

    /// Attempt a transition. Returns error if no valid edge exists from current state.
    pub fn transition(&mut self, to: &str) -> Result<(), StateError> {
        let valid = self.graph.get(&self.current)
            .map(|edges| edges.iter().any(|e| e.to == to))
            .unwrap_or(false);

        if valid {
            self.current = to.to_string();
            Ok(())
        } else {
            Err(StateError::IllegalTransition {
                from: self.current.clone(),
                to: to.to_string(),
            })
        }
    }

    /// Check if the machine is currently in the given state.
    pub fn is_in(&self, state: &str) -> bool {
        self.current == state
    }
}
