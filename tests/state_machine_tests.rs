use forge::runtime::state_machine::{StateMachine, StateError};
use forge::ast::{StatesDecl, StateTransition, Spanned, Span};

fn spanned<T>(node: T) -> Spanned<T> {
    Spanned::new(node, Span { start: 0, end: 0 })
}

fn game_phase_decl() -> StatesDecl {
    StatesDecl {
        name: spanned("GamePhase".to_string()),
        transitions: vec![
            spanned(StateTransition {
                from: spanned("waiting".to_string()),
                to: spanned("playing".to_string()),
                condition: None,
            }),
            spanned(StateTransition {
                from: spanned("playing".to_string()),
                to: spanned("finished".to_string()),
                condition: None,
            }),
        ],
    }
}

#[test]
fn initial_state_is_first_from() {
    let sm = StateMachine::new(&game_phase_decl());
    assert!(sm.is_in("waiting"));
    assert!(!sm.is_in("playing"));
}

#[test]
fn legal_transition_succeeds() {
    let mut sm = StateMachine::new(&game_phase_decl());
    assert!(sm.transition("playing").is_ok());
    assert!(sm.is_in("playing"));
}

#[test]
fn illegal_transition_returns_error() {
    let mut sm = StateMachine::new(&game_phase_decl());
    let result = sm.transition("finished");
    assert!(matches!(result, Err(StateError::IllegalTransition { ref from, ref to })
        if from == "waiting" && to == "finished"));
}

#[test]
fn chained_legal_transitions() {
    let mut sm = StateMachine::new(&game_phase_decl());
    sm.transition("playing").unwrap();
    sm.transition("finished").unwrap();
    assert!(sm.is_in("finished"));
}

#[test]
fn terminal_state_rejects_all_transitions() {
    let mut sm = StateMachine::new(&game_phase_decl());
    sm.transition("playing").unwrap();
    sm.transition("finished").unwrap();
    let result = sm.transition("waiting");
    assert!(matches!(result, Err(StateError::IllegalTransition { .. })));
}
