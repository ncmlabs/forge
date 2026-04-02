// FORGE states checker
// See issue #17 for full specification

use std::collections::{HashMap, HashSet};

use crate::ast::{AgentDecl, BinOp, Expr, Program, Spanned, Stmt, TopLevel};
use crate::diagnostic::Diagnostic;

// ── Lifecycle guard classification ───────────────────────────

#[derive(Debug)]
enum LifecycleGuard {
    /// `requires lifecycle == SomeState`
    ExactState(String, usize, usize),
    /// Guard mentions lifecycle but is too complex to analyze statically
    Opaque(usize, usize),
}

// ── Transition targets collector ─────────────────────────────

fn collect_transitions(stmts: &[Spanned<Stmt>], targets: &mut Vec<Spanned<String>>) {
    for stmt in stmts {
        collect_transitions_stmt(stmt, targets);
    }
}

fn collect_transitions_stmt(stmt: &Spanned<Stmt>, targets: &mut Vec<Spanned<String>>) {
    match &stmt.node {
        Stmt::TransitionTo(name) => {
            targets.push(name.clone());
        }
        Stmt::IfElse(ie) => {
            collect_transitions(&ie.then_body, targets);
            for (_, body) in &ie.else_ifs {
                collect_transitions(body, targets);
            }
            if let Some(body) = &ie.else_body {
                collect_transitions(body, targets);
            }
        }
        Stmt::Match(m) => {
            for arm in &m.arms {
                collect_transitions_stmt(&arm.node.body, targets);
            }
        }
        Stmt::When(when) => {
            for clause in &when.clauses {
                collect_transitions_stmt(&clause.node.body, targets);
            }
            if let Some(else_clause) = &when.else_body {
                collect_transitions_stmt(&else_clause.node.body, targets);
            }
        }
        Stmt::For(f) => {
            collect_transitions(&f.body, targets);
        }
        _ => {}
    }
}

// ── Lifecycle guard extraction ────────────────────────────────

fn expr_mentions_lifecycle(expr: &Spanned<Expr>) -> bool {
    match &expr.node {
        Expr::Ident(name) => name == "lifecycle",
        Expr::BinOp(a, _, b) => expr_mentions_lifecycle(a) || expr_mentions_lifecycle(b),
        Expr::Paren(inner) => expr_mentions_lifecycle(inner),
        Expr::UnaryOp(_, inner) => expr_mentions_lifecycle(inner),
        _ => false,
    }
}

fn extract_lifecycle_guard(expr: &Spanned<Expr>) -> Option<LifecycleGuard> {
    match &expr.node {
        Expr::BinOp(lhs, op, rhs) => {
            match op.node {
                BinOp::Eq => {
                    // `lifecycle == state` or `state == lifecycle`
                    if let Expr::Ident(name) = &lhs.node {
                        if name == "lifecycle" {
                            if let Expr::Ident(state) = &rhs.node {
                                return Some(LifecycleGuard::ExactState(
                                    state.clone(),
                                    expr.span.start,
                                    expr.span.end,
                                ));
                            }
                        }
                    }
                    if let Expr::Ident(name) = &rhs.node {
                        if name == "lifecycle" {
                            if let Expr::Ident(state) = &lhs.node {
                                return Some(LifecycleGuard::ExactState(
                                    state.clone(),
                                    expr.span.start,
                                    expr.span.end,
                                ));
                            }
                        }
                    }
                    // Check if either side mentions lifecycle but pattern doesn't match cleanly
                    if expr_mentions_lifecycle(expr) {
                        return Some(LifecycleGuard::Opaque(expr.span.start, expr.span.end));
                    }
                    None
                }
                BinOp::Ne => {
                    if expr_mentions_lifecycle(lhs) || expr_mentions_lifecycle(rhs) {
                        Some(LifecycleGuard::Opaque(expr.span.start, expr.span.end))
                    } else {
                        None
                    }
                }
                BinOp::Or => {
                    if expr_mentions_lifecycle(expr) {
                        Some(LifecycleGuard::Opaque(expr.span.start, expr.span.end))
                    } else {
                        None
                    }
                }
                _ => {
                    if expr_mentions_lifecycle(expr) {
                        Some(LifecycleGuard::Opaque(expr.span.start, expr.span.end))
                    } else {
                        None
                    }
                }
            }
        }
        Expr::Paren(inner) => extract_lifecycle_guard(inner),
        _ => {
            if expr_mentions_lifecycle(expr) {
                Some(LifecycleGuard::Opaque(expr.span.start, expr.span.end))
            } else {
                None
            }
        }
    }
}

// ── Phase 1: Registry ─────────────────────────────────────────

struct StatesRegistry {
    /// states_name -> (set of state names, set of (from, to) edges)
    decls: HashMap<String, (HashSet<String>, HashSet<(String, String)>)>,
    /// decl name -> span of the declaration name
    spans: HashMap<String, Spanned<String>>,
}

impl StatesRegistry {
    fn build(program: &Program) -> Self {
        let mut decls: HashMap<String, (HashSet<String>, HashSet<(String, String)>)> =
            HashMap::new();
        let mut spans: HashMap<String, Spanned<String>> = HashMap::new();

        for item in &program.items {
            if let TopLevel::States(states) = &item.node {
                let name = states.name.node.clone();
                let mut state_names: HashSet<String> = HashSet::new();
                let mut edges: HashSet<(String, String)> = HashSet::new();

                for transition in &states.transitions {
                    let from = transition.node.from.node.clone();
                    let to = transition.node.to.node.clone();
                    state_names.insert(from.clone());
                    state_names.insert(to.clone());
                    edges.insert((from, to));
                }

                decls.insert(name.clone(), (state_names, edges));
                spans.insert(name, states.name.clone());
            }
        }

        Self { decls, spans }
    }

    fn get(&self, name: &str) -> Option<&(HashSet<String>, HashSet<(String, String)>)> {
        self.decls.get(name)
    }
}

// ── Phase 2 & 3: Validation ───────────────────────────────────

pub fn check(program: &Program, file: &str) -> Vec<Diagnostic> {
    let registry = StatesRegistry::build(program);
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    // Phase 2: per-agent validation
    for item in &program.items {
        if let TopLevel::Agent(agent) = &item.node {
            check_agent(agent, &registry, file, &mut diagnostics);
        }
    }

    // Phase 3: structural warnings
    for item in &program.items {
        if let TopLevel::States(states) = &item.node {
            check_structural(states.name.node.as_str(), &registry, file, &mut diagnostics);
        }
    }

    diagnostics
}

fn check_agent(
    agent: &AgentDecl,
    registry: &StatesRegistry,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let lifecycle_ref = match &agent.lifecycle {
        Some(lc) => lc,
        None => return, // no lifecycle, skip
    };

    let lifecycle_name = &lifecycle_ref.node;

    // Resolve lifecycle to a StatesDecl
    let (state_names, edges) = match registry.get(lifecycle_name) {
        Some(data) => data,
        None => {
            diagnostics.push(
                Diagnostic::error(
                    file,
                    format!("unknown lifecycle `{}`", lifecycle_name),
                    lifecycle_ref.span.start..lifecycle_ref.span.end,
                    format!("`{}` is not a declared `states` block", lifecycle_name),
                )
                .with_help("declare a `states` block with this name"),
            );
            return;
        }
    };

    // Per-handler validation
    for handler in &agent.handlers {
        check_handler(
            &handler.node,
            state_names,
            edges,
            file,
            diagnostics,
        );
    }
}

fn check_handler(
    handler: &crate::ast::OnHandler,
    state_names: &HashSet<String>,
    edges: &HashSet<(String, String)>,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    // Extract lifecycle guards from requires clauses
    let mut lifecycle_guards: Vec<LifecycleGuard> = Vec::new();

    for req in &handler.requires {
        if let Some(guard) = extract_lifecycle_guard(&req.node.condition) {
            lifecycle_guards.push(guard);
        }
    }

    // Check for conflicting guards (multiple lifecycle requires)
    if lifecycle_guards.len() > 1 {
        // Report on the second guard's span
        if let Some(second) = lifecycle_guards.get(1) {
            let (start, end) = match second {
                LifecycleGuard::ExactState(_, s, e) => (*s, *e),
                LifecycleGuard::Opaque(s, e) => (*s, *e),
            };
            diagnostics.push(
                Diagnostic::error(
                    file,
                    format!("conflicting lifecycle guards in handler `{}`", handler.event.node),
                    start..end,
                    "multiple lifecycle guards found in the same handler",
                )
                .with_help("use at most one `requires lifecycle == ...` per handler"),
            );
        }
    }

    // Validate guard state names
    for guard in &lifecycle_guards {
        match guard {
            LifecycleGuard::ExactState(state, start, end) => {
                if !state_names.contains(state.as_str()) {
                    diagnostics.push(
                        Diagnostic::error(
                            file,
                            format!("unknown state `{}` in lifecycle guard", state),
                            *start..*end,
                            format!("`{}` is not defined in the states block", state),
                        )
                        .with_help("check the states block for valid state names"),
                    );
                }
            }
            LifecycleGuard::Opaque(start, end) => {
                diagnostics.push(
                    Diagnostic::warning(
                        file,
                        format!(
                            "lifecycle guard in handler `{}` is too complex for static analysis",
                            handler.event.node
                        ),
                        *start..*end,
                        "opaque lifecycle guard — cannot verify transitions statically",
                    )
                    .with_help("use `requires lifecycle == StateName` for precise analysis"),
                );
            }
        }
    }

    // Collect all transition targets
    let mut transition_targets: Vec<Spanned<String>> = Vec::new();
    collect_transitions(&handler.body, &mut transition_targets);

    if transition_targets.is_empty() {
        return; // no transitions, nothing more to check
    }

    // Determine current state from guard (for legality check)
    let guarded_state: Option<String> = lifecycle_guards.iter().find_map(|g| {
        if let LifecycleGuard::ExactState(s, _, _) = g {
            Some(s.clone())
        } else {
            None
        }
    });

    let has_lifecycle_guard = !lifecycle_guards.is_empty();

    // Check each transition target
    for target in &transition_targets {
        let target_name = &target.node;

        // State existence check
        if !state_names.contains(target_name.as_str()) {
            diagnostics.push(
                Diagnostic::error(
                    file,
                    format!("unknown state `{}` in transition", target_name),
                    target.span.start..target.span.end,
                    format!("`{}` is not defined in the states block", target_name),
                )
                .with_help("check the states block for valid state names"),
            );
            continue; // no point checking legality for unknown states
        }

        // Unguarded transition check
        if !has_lifecycle_guard {
            diagnostics.push(
                Diagnostic::error(
                    file,
                    format!(
                        "unguarded transition to `{}` in handler `{}`",
                        target_name, handler.event.node
                    ),
                    target.span.start..target.span.end,
                    "transition without a lifecycle guard",
                )
                .with_help(
                    "add `requires lifecycle == CurrentState` to guard this transition",
                ),
            );
            continue;
        }

        // Transition legality check (only possible if we have an exact state guard)
        if let Some(from_state) = &guarded_state {
            if !edges.contains(&(from_state.clone(), target_name.clone())) {
                diagnostics.push(
                    Diagnostic::error(
                        file,
                        format!(
                            "illegal transition from `{}` to `{}`",
                            from_state, target_name
                        ),
                        target.span.start..target.span.end,
                        format!(
                            "no edge `{} -> {}` in the states block",
                            from_state, target_name
                        ),
                    )
                    .with_help("add this transition to the states block, or fix the state name"),
                );
            }
        }
        // If guard is Opaque, we can't check legality — but we already warned about opacity
    }

    // Unguarded transition if handler has transitions but only opaque guards
    // (no ExactState guard found but lifecycle guards exist as opaque)
    // Already warned about opaque — skip additional noise
}

fn check_structural(
    states_name: &str,
    registry: &StatesRegistry,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let (state_names, edges) = match registry.get(states_name) {
        Some(data) => data,
        None => return,
    };

    // Compute initial states (states that appear as `from` but not as `to` in any edge)
    let all_from: HashSet<&String> = edges.iter().map(|(from, _)| from).collect();
    let all_to: HashSet<&String> = edges.iter().map(|(_, to)| to).collect();

    // Initial states: appear as `from`, not as `to` (or simply: not pointed to by any edge)
    let initial_states: HashSet<&String> =
        all_from.iter().copied().filter(|s| !all_to.contains(*s)).collect();

    let registry_span = registry.spans.get(states_name);
    let decl_span = registry_span
        .map(|s| s.span.start..s.span.end)
        .unwrap_or(0..0);

    for state in state_names {
        let outgoing: Vec<_> = edges.iter().filter(|(from, _)| from == state).collect();
        let incoming: Vec<_> = edges.iter().filter(|(_, to)| to == state).collect();

        // Terminal state: no outgoing edges
        if outgoing.is_empty() {
            diagnostics.push(
                Diagnostic::warning(
                    file,
                    format!(
                        "terminal state `{}` in `{}` has no outgoing transitions",
                        state, states_name
                    ),
                    decl_span.clone(),
                    format!("`{}` is a terminal state (no exits)", state),
                )
                .with_help("add outgoing transitions or mark as intentionally terminal"),
            );
        }

        // Unreachable state: no incoming edges and not initial
        if incoming.is_empty() && !initial_states.contains(state) {
            diagnostics.push(
                Diagnostic::warning(
                    file,
                    format!(
                        "unreachable state `{}` in `{}` has no incoming transitions",
                        state, states_name
                    ),
                    decl_span.clone(),
                    format!("`{}` is unreachable (no entries)", state),
                )
                .with_help("add incoming transitions or remove this state"),
            );
        }
    }
}
