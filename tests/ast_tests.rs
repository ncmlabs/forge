// AST acceptance criteria tests (issue #3)
//
// Verifies:
// 1. All types derive Debug, Clone
// 2. Types cover every construct in pocv2.md syntax examples
// 3. Compiles cleanly (implicit — this file compiles)

use forge::ast::*;

// ── Helpers ───────────────────────────────────────────────────

fn sp<T>(node: T) -> Spanned<T> {
    Spanned::new(node, Span { start: 0, end: 0 })
}

// ── Debug + Clone on all types ────────────────────────────────

#[test]
fn span_is_debug_clone_copy() {
    let s = Span { start: 0, end: 42 };
    let s2 = s; // Copy
    let s3 = s;
    let _ = format!("{:?}", s2);
    let _ = format!("{:?}", s3);
}

#[test]
fn spanned_is_debug_clone() {
    let s = sp(42u32);
    let s2 = s.clone();
    let _ = format!("{:?}", s2);
}

// ── use declaration ───────────────────────────────────────────

/// pocv2.md: `use\n  llm.reason\n  web.search`
#[test]
fn ast_use_decl() {
    let decl = UseDecl {
        capabilities: vec![
            sp("llm.reason".into()),
            sp("llm.classify".into()),
            sp("web.search".into()),
            sp("data.embed".into()),
        ],
    };
    let top = sp(TopLevel::Use(decl.clone()));
    let _ = format!("{:?}", top);
}

// ── simple task ───────────────────────────────────────────────

/// pocv2.md: `task greet\n  needs name: Text\n  gives Text\n  do\n    say "hello {name}"`
#[test]
fn ast_simple_task() {
    let task = TaskDecl {
        name: sp("greet".into()),
        needs: vec![sp(Param {
            name: "name".into(),
            type_name: sp(TypeName::Text),
        })],
        gives: Some(sp(OutputType {
            types: vec![sp(TypeName::Text)],
        })),
        body: sp(TaskBody::Do(vec![sp(Stmt::Say(sp(Expr::Template(vec![
            sp(TemplatePart::Text("hello ".into())),
            sp(TemplatePart::Interp(Box::new(sp(Expr::Ident(
                "name".into(),
            ))))),
        ]))))])),
        if_fails: None,
    };
    let top = sp(TopLevel::Task(task.clone()));
    let _ = format!("{:?}", top);
}

// ── task with confidence predicates and when/else ─────────────

/// pocv2.md: classify_intent task with when/else branches
#[test]
fn ast_confidence_aware_task() {
    let sure_above = sp(ConfidencePred {
        subject: sp("result".into()),
        level: sp(ConfLevel::Sure(Some(0.85))),
    });
    let sure = sp(ConfidencePred {
        subject: sp("result".into()),
        level: sp(ConfLevel::Sure(None)),
    });
    let unsure = sp(ConfidencePred {
        subject: sp("result".into()),
        level: sp(ConfLevel::Unsure),
    });

    let when_block = WhenBlock {
        clauses: vec![
            sp(WhenClause {
                predicate: sure_above,
                body: sp(Stmt::Give(sp(Expr::Ident("result".into())), vec![])),
            }),
            sp(WhenClause {
                predicate: sure,
                body: sp(Stmt::Give(
                    sp(Expr::Ident("result".into())),
                    vec![sp(GiveMeta {
                        key: sp("status".into()),
                        value: sp(Expr::NumberLit(200.0)),
                    })],
                )),
            }),
            sp(WhenClause {
                predicate: unsure,
                body: sp(Stmt::Give(
                    sp(Expr::Call(CallExpr {
                        name: sp("ask_for_clarification".into()),
                        args: vec![sp(CallArg {
                            label: None,
                            value: sp(Expr::Ident("message".into())),
                        })],
                    })),
                    vec![],
                )),
            }),
        ],
        else_body: Some(sp(ElseClause {
            body: sp(Stmt::Give(
                sp(Expr::TypeAccess(sp(TypeName::Intent), sp("unknown".into()))),
                vec![],
            )),
        })),
    };

    let task = TaskDecl {
        name: sp("classify_intent".into()),
        needs: vec![sp(Param {
            name: "message".into(),
            type_name: sp(TypeName::Text),
        })],
        gives: Some(sp(OutputType {
            types: vec![sp(TypeName::Intent)],
        })),
        body: sp(TaskBody::Do(vec![
            sp(Stmt::Bind(
                sp("result".into()),
                sp(Expr::Classify(ClassifyExpr {
                    input: Box::new(sp(Expr::Ident("message".into()))),
                    labels: vec![
                        sp("buy".into()),
                        sp("support".into()),
                        sp("cancel".into()),
                        sp("other".into()),
                    ],
                })),
            )),
            sp(Stmt::When(Box::new(when_block))),
        ])),
        if_fails: None,
    };
    let top = sp(TopLevel::Task(task.clone()));
    let _ = format!("{:?}", top);
}

// ── composition task ──────────────────────────────────────────

/// pocv2.md: `task process_message is classify_intent >> route >> send`
#[test]
fn ast_composition_task() {
    let task = TaskDecl {
        name: sp("process_message".into()),
        needs: vec![],
        gives: None,
        body: sp(TaskBody::Is(Box::new(sp(Expr::Compose(vec![
            sp(Expr::Ident("classify_intent".into())),
            sp(Expr::Ident("route_to_handler".into())),
            sp(Expr::Ident("send_response".into())),
        ]))))),
        if_fails: None,
    };
    let _ = format!("{:?}", task.clone());
}

// ── fan-out ───────────────────────────────────────────────────

/// pocv2.md: `task multi is (A | B | C) >> merge >> D`
#[test]
fn ast_fan_out() {
    let expr = Expr::Compose(vec![
        sp(Expr::FanOut(vec![
            sp(Expr::Ident("A".into())),
            sp(Expr::Ident("B".into())),
            sp(Expr::Ident("C".into())),
        ])),
        sp(Expr::Ident("merge".into())),
        sp(Expr::Ident("D".into())),
    ]);
    let _ = format!("{:?}", expr.clone());
}

// ── task with if_fails ────────────────────────────────────────

/// pocv2.md: safe_search with `if fails` block
#[test]
fn ast_task_with_if_fails() {
    let task = TaskDecl {
        name: sp("safe_search".into()),
        needs: vec![sp(Param {
            name: "query".into(),
            type_name: sp(TypeName::Text),
        })],
        gives: Some(sp(OutputType {
            types: vec![sp(TypeName::Results), sp(TypeName::Failure)],
        })),
        body: sp(TaskBody::Do(vec![
            sp(Stmt::Bind(
                sp("result".into()),
                sp(Expr::Call(CallExpr {
                    name: sp("web_search".into()),
                    args: vec![sp(CallArg {
                        label: None,
                        value: sp(Expr::Ident("query".into())),
                    })],
                })),
            )),
            sp(Stmt::Give(sp(Expr::Ident("result".into())), vec![])),
        ])),
        if_fails: Some(vec![sp(Stmt::Give(
            sp(Expr::Constructor(ConstructorExpr {
                type_name: sp(TypeName::Failure),
                args: vec![
                    sp(CallArg {
                        label: None,
                        value: sp(Expr::Template(vec![sp(TemplatePart::Text(
                            "search unavailable".into(),
                        ))])),
                    }),
                    sp(CallArg {
                        label: Some(sp("retry".into())),
                        value: sp(Expr::BoolLit(true)),
                    }),
                ],
            })),
            vec![],
        ))]),
    };
    let _ = format!("{:?}", task.clone());
}

// ── flow with stages ──────────────────────────────────────────

/// pocv2.md: research flow with gather/synthesize/verify stages
#[test]
fn ast_flow_with_stages() {
    let flow = FlowDecl {
        name: sp("research".into()),
        needs: vec![sp(Param {
            name: "topic".into(),
            type_name: sp(TypeName::Text),
        })],
        gives: Some(sp(OutputType {
            types: vec![sp(TypeName::Report)],
        })),
        stages: vec![
            sp(StageDecl {
                name: sp("gather".into()),
                needs: vec![],
                body: vec![
                    sp(Stmt::Bind(
                        sp("web_results".into()),
                        sp(Expr::Search(Box::new(sp(Expr::Ident("topic".into()))))),
                    )),
                    sp(Stmt::Bind(
                        sp("paper_results".into()),
                        sp(Expr::Search(Box::new(sp(Expr::Template(vec![
                            sp(TemplatePart::Interp(Box::new(sp(Expr::Ident(
                                "topic".into(),
                            ))))),
                            sp(TemplatePart::Text(" research paper".into())),
                        ]))))),
                    )),
                    sp(Stmt::Bind(
                        sp("news".into()),
                        sp(Expr::Search(Box::new(sp(Expr::Template(vec![
                            sp(TemplatePart::Interp(Box::new(sp(Expr::Ident(
                                "topic".into(),
                            ))))),
                            sp(TemplatePart::Text(" news".into())),
                        ]))))),
                    )),
                ],
            }),
            sp(StageDecl {
                name: sp("synthesize".into()),
                needs: vec![sp(NeedsRef {
                    stage: "gather".into(),
                    field: NeedsRefField::Glob,
                })],
                body: vec![sp(Stmt::Bind(
                    sp("draft".into()),
                    sp(Expr::Reason(Box::new(sp(Expr::Template(vec![
                        sp(TemplatePart::Text(
                            "synthesize these sources into a report: ".into(),
                        )),
                        sp(TemplatePart::Interp(Box::new(sp(Expr::GlobAccess(
                            Box::new(sp(Expr::Ident("gather".into()))),
                        ))))),
                    ]))))),
                ))],
            }),
            sp(StageDecl {
                name: sp("verify".into()),
                needs: vec![sp(NeedsRef {
                    stage: "synthesize".into(),
                    field: NeedsRefField::Named("draft".into()),
                })],
                body: vec![
                    sp(Stmt::Bind(
                        sp("checked".into()),
                        sp(Expr::Reason(Box::new(sp(Expr::Template(vec![
                            sp(TemplatePart::Text("fact-check this: ".into())),
                            sp(TemplatePart::Interp(Box::new(sp(Expr::FieldAccess(
                                Box::new(sp(Expr::Ident("synthesize".into()))),
                                sp("draft".into()),
                            ))))),
                        ]))))),
                    )),
                    sp(Stmt::Give(
                        sp(Expr::Constructor(ConstructorExpr {
                            type_name: sp(TypeName::Report),
                            args: vec![sp(CallArg {
                                label: None,
                                value: sp(Expr::Ident("checked".into())),
                            })],
                        })),
                        vec![],
                    )),
                ],
            }),
        ],
    };
    let _ = format!("{:?}", flow.clone());
}

// ── pool ──────────────────────────────────────────────────────

/// pocv2.md: pool with strategies
#[test]
fn ast_pool_fastest() {
    let pool = PoolDecl {
        name: sp("search_workers".into()),
        worker_type: sp("SearchAgent".into()),
        worker_count: sp(3.0),
        strategy: sp(PoolStrategy::Fastest),
        timeout: None,
        fallback: Some(sp("CachedSearch".into())),
    };
    let _ = format!("{:?}", pool.clone());
}

#[test]
fn ast_pool_majority_with_timeout() {
    let pool = PoolDecl {
        name: sp("fact_checkers".into()),
        worker_type: sp("FactChecker".into()),
        worker_count: sp(5.0),
        strategy: sp(PoolStrategy::Majority),
        timeout: Some(sp(Duration {
            value: 10,
            unit: DurationUnit::Seconds,
        })),
        fallback: None,
    };
    let _ = format!("{:?}", pool.clone());
}

#[test]
fn ast_pool_quorum() {
    let pool = PoolDecl {
        name: sp("voters".into()),
        worker_type: sp("Voter".into()),
        worker_count: sp(7.0),
        strategy: sp(PoolStrategy::Quorum(3.0)),
        timeout: None,
        fallback: None,
    };
    let _ = format!("{:?}", pool.clone());
}

#[test]
fn ast_pool_first() {
    let pool = PoolDecl {
        name: sp("fetchers".into()),
        worker_type: sp("Fetcher".into()),
        worker_count: sp(4.0),
        strategy: sp(PoolStrategy::First(2.0)),
        timeout: None,
        fallback: None,
    };
    let _ = format!("{:?}", pool.clone());
}

// ── agent ─────────────────────────────────────────────────────

/// pocv2.md: support_bot agent with memory, handlers, stuck policy
#[test]
fn ast_agent() {
    let agent = AgentDecl {
        exportable: false,
        name: sp("support_bot".into()),
        lifecycle: None,
        memory: vec![
            sp(FieldDef {
                name: "history".into(),
                type_name: sp(TypeName::Conversation),
            }),
            sp(FieldDef {
                name: "user".into(),
                type_name: sp(TypeName::Profile),
            }),
        ],
        memory_persistent: false,
        knowledge: None,
        timers: vec![],
        schedules: vec![],
        correlates: vec![],
        subscriptions: vec![],
        handlers: vec![
            sp(OnHandler {
                event: sp("message".into()),
                params: vec![],
                payload_type: Some(sp(TypeName::Text)),
                requires: vec![],
                body: vec![
                    sp(Stmt::Bind(
                        sp("intent".into()),
                        sp(Expr::Call(CallExpr {
                            name: sp("classify_intent".into()),
                            args: vec![sp(CallArg {
                                label: None,
                                value: sp(Expr::Ident("message".into())),
                            })],
                        })),
                    )),
                    sp(Stmt::Bind(
                        sp("response".into()),
                        sp(Expr::Call(CallExpr {
                            name: sp("route_to_handler".into()),
                            args: vec![
                                sp(CallArg {
                                    label: None,
                                    value: sp(Expr::Ident("intent".into())),
                                }),
                                sp(CallArg {
                                    label: Some(sp("history".into())),
                                    value: sp(Expr::FieldAccess(
                                        Box::new(sp(Expr::Ident("memory".into()))),
                                        sp("history".into()),
                                    )),
                                }),
                            ],
                        })),
                    )),
                    sp(Stmt::MemoryUpdate(
                        sp("history".into()),
                        None,
                        sp(Expr::Ident("updated_history".into())),
                    )),
                    sp(Stmt::Give(sp(Expr::Ident("response".into())), vec![])),
                ],
            }),
            sp(OnHandler {
                event: sp("reset".into()),
                params: vec![],
                payload_type: None,
                requires: vec![],
                body: vec![sp(Stmt::MemoryUpdate(
                    sp("history".into()),
                    None,
                    sp(Expr::TypeAccess(
                        sp(TypeName::Conversation),
                        sp("empty".into()),
                    )),
                ))],
            }),
        ],
        warden_override: Vec::new(),
        stuck_policy: Some(sp(StuckPolicy {
            turns: None,
            body: vec![sp(Stmt::Escalate(sp("human".into())))],
        })),
    };
    let _ = format!("{:?}", agent.clone());
}

/// pocv2.md: `if stuck for 3 turns`
#[test]
fn ast_stuck_policy_with_turns() {
    let policy = StuckPolicy {
        turns: Some(3),
        body: vec![sp(Stmt::Escalate(sp("human".into())))],
    };
    let _ = format!("{:?}", policy.clone());
}

// ── contract ──────────────────────────────────────────────────

/// pocv2.md: `contract Researcher`
#[test]
fn ast_contract() {
    let contract = ContractDecl {
        name: sp("Researcher".into()),
        methods: vec![
            sp(CanSignature {
                name: "search".into(),
                params: vec![sp(Param {
                    name: "query".into(),
                    type_name: sp(TypeName::Text),
                })],
                return_type: sp(TypeName::Results),
            }),
            sp(CanSignature {
                name: "summarize".into(),
                params: vec![sp(Param {
                    name: "sources".into(),
                    type_name: sp(TypeName::Results),
                })],
                return_type: sp(TypeName::Summary),
            }),
        ],
    };
    let _ = format!("{:?}", contract.clone());
}

// ── system ────────────────────────────────────────────────────

/// pocv2.md: analytics_pipeline system
#[test]
fn ast_system() {
    let system = SystemDecl {
        name: sp("analytics_pipeline".into()),
        bindings: vec![
            sp(SystemBinding {
                alias: "ingestion".into(),
                target: "DataIngestor".into(),
            }),
            sp(SystemBinding {
                alias: "analysis".into(),
                target: "Researcher".into(),
            }),
            sp(SystemBinding {
                alias: "reporting".into(),
                target: "ReportWriter".into(),
            }),
        ],
        wiring: vec![sp(Expr::Compose(vec![
            sp(Expr::Ident("ingestion".into())),
            sp(Expr::Ident("analysis".into())),
            sp(Expr::Ident("reporting".into())),
        ]))],
    };
    let _ = format!("{:?}", system.clone());
}

// ── fn main ───────────────────────────────────────────────────

#[test]
fn ast_fn_main() {
    let main = FnMainDecl {
        body: vec![sp(Stmt::ExprStmt(sp(Expr::Call(CallExpr {
            name: sp("greet".into()),
            args: vec![sp(CallArg {
                label: None,
                value: sp(Expr::Template(vec![sp(TemplatePart::Text("world".into()))])),
            })],
        }))))],
    };
    let _ = format!("{:?}", main.clone());
}

// ── expressions coverage ──────────────────────────────────────

#[test]
fn ast_expr_number_lit() {
    let e = Expr::NumberLit(42.0);
    let _ = format!("{:?}", e.clone());
}

#[test]
fn ast_expr_bool_lit() {
    let e = Expr::BoolLit(false);
    let _ = format!("{:?}", e.clone());
}

#[test]
fn ast_expr_try_or() {
    let e = Expr::TryOr(
        Box::new(sp(Expr::Call(CallExpr {
            name: sp("fetch".into()),
            args: vec![],
        }))),
        Box::new(sp(Expr::Ident("cached".into()))),
    );
    let _ = format!("{:?}", e.clone());
}

#[test]
fn ast_expr_paren() {
    let e = Expr::Paren(Box::new(sp(Expr::Ident("x".into()))));
    let _ = format!("{:?}", e.clone());
}

#[test]
fn ast_expr_glob_access() {
    let e = Expr::GlobAccess(Box::new(sp(Expr::Ident("gather".into()))));
    let _ = format!("{:?}", e.clone());
}

// ── confidence levels ─────────────────────────────────────────

#[test]
fn ast_conf_levels() {
    let levels = vec![
        ConfLevel::Sure(None),
        ConfLevel::Sure(Some(0.9)),
        ConfLevel::Unsure,
        ConfLevel::Unreliable,
        ConfLevel::Conflicted,
    ];
    for level in &levels {
        let _ = format!("{:?}", level.clone());
    }
}

// ── type names ────────────────────────────────────────────────

#[test]
fn ast_all_builtin_types() {
    let types = vec![
        TypeName::Text,
        TypeName::Number,
        TypeName::Bool,
        TypeName::Results,
        TypeName::Report,
        TypeName::Intent,
        TypeName::Summary,
        TypeName::Failure,
        TypeName::Classification,
        TypeName::Conversation,
        TypeName::Profile,
        TypeName::SearchResults,
        TypeName::Custom("MyType".into()),
    ];
    for t in &types {
        let _ = format!("{:?}", t.clone());
    }
}

// ── duration units ────────────────────────────────────────────

#[test]
fn ast_duration_units() {
    let durations = vec![
        Duration {
            value: 10,
            unit: DurationUnit::Seconds,
        },
        Duration {
            value: 5,
            unit: DurationUnit::Minutes,
        },
        Duration {
            value: 1,
            unit: DurationUnit::Hours,
        },
    ];
    for d in &durations {
        let _ = format!("{:?}", d.clone());
    }
}

// ── full program ──────────────────────────────────────────────

/// Construct a Program with multiple top-level items
#[test]
fn ast_full_program() {
    let program = Program {
        boundary: None,
        items: vec![
            sp(TopLevel::Use(UseDecl {
                capabilities: vec![sp("llm.reason".into())],
            })),
            sp(TopLevel::Task(TaskDecl {
                name: sp("greet".into()),
                needs: vec![sp(Param {
                    name: "name".into(),
                    type_name: sp(TypeName::Text),
                })],
                gives: Some(sp(OutputType {
                    types: vec![sp(TypeName::Text)],
                })),
                body: sp(TaskBody::Do(vec![sp(Stmt::Say(sp(Expr::Template(vec![
                    sp(TemplatePart::Text("Hello, ".into())),
                    sp(TemplatePart::Interp(Box::new(sp(Expr::Ident(
                        "name".into(),
                    ))))),
                ]))))])),
                if_fails: None,
            })),
        ],
    };
    let program2 = program.clone();
    let _ = format!("{:?}", program2);
}

// ── needs_ref field variants ──────────────────────────────────

#[test]
fn ast_needs_ref_variants() {
    let named = NeedsRef {
        stage: "synthesize".into(),
        field: NeedsRefField::Named("draft".into()),
    };
    let glob = NeedsRef {
        stage: "gather".into(),
        field: NeedsRefField::Glob,
    };
    let _ = format!("{:?} {:?}", named.clone(), glob.clone());
}

// ============================================================
// v3 AST tests
// ============================================================

// ── boundary directive ───────────────────────────────────────

#[test]
fn ast_boundary_directive() {
    let program = Program {
        boundary: Some(sp(BoundaryDirective {
            kind: sp(BoundaryKind::Server),
        })),
        items: vec![],
    };
    let _ = format!("{:?}", program.clone());

    // All three kinds
    let _ = format!(
        "{:?} {:?} {:?}",
        BoundaryKind::Server,
        BoundaryKind::Client,
        BoundaryKind::Shared
    );
}

// ── pure declaration ─────────────────────────────────────────

#[test]
fn ast_pure_decl() {
    let pure = PureDecl {
        name: sp("valid_move".into()),
        needs: vec![
            sp(Param {
                name: "board".into(),
                type_name: sp(TypeName::Array(Box::new(TypeName::Text), Some(9))),
            }),
            sp(Param {
                name: "cell".into(),
                type_name: sp(TypeName::Number),
            }),
        ],
        gives: Some(sp(OutputType {
            types: vec![sp(TypeName::Bool)],
        })),
        body: vec![sp(Stmt::Give(
            sp(Expr::BinOp(
                Box::new(sp(Expr::BinOp(
                    Box::new(sp(Expr::Ident("cell".into()))),
                    sp(BinOp::Ge),
                    Box::new(sp(Expr::NumberLit(0.0))),
                ))),
                sp(BinOp::And),
                Box::new(sp(Expr::BinOp(
                    Box::new(sp(Expr::Ident("cell".into()))),
                    sp(BinOp::Le),
                    Box::new(sp(Expr::NumberLit(8.0))),
                ))),
            )),
            vec![],
        ))],
    };
    let top = sp(TopLevel::Pure(pure.clone()));
    let _ = format!("{:?}", top);
}

// ── event declaration ────────────────────────────────────────

#[test]
fn ast_event_decl() {
    let event = EventDecl {
        name: sp("MoveEvent".into()),
        fields: vec![
            sp(FieldDef {
                name: "room_id".into(),
                type_name: sp(TypeName::Text),
            }),
            sp(FieldDef {
                name: "cell".into(),
                type_name: sp(TypeName::Number),
            }),
            sp(FieldDef {
                name: "board".into(),
                type_name: sp(TypeName::Array(Box::new(TypeName::Text), Some(9))),
            }),
        ],
    };
    let top = sp(TopLevel::Event(event.clone()));
    let _ = format!("{:?}", top);
}

// ── states declaration ───────────────────────────────────────

#[test]
fn ast_states_decl() {
    let states = StatesDecl {
        name: sp("RoomLifecycle".into()),
        transitions: vec![
            sp(StateTransition {
                from: sp("waiting".into()),
                to: sp("playing".into()),
                condition: Some(sp(Expr::Ident("players_full".into()))),
            }),
            sp(StateTransition {
                from: sp("playing".into()),
                to: sp("done".into()),
                condition: Some(sp(Expr::Ident("winner_found".into()))),
            }),
            sp(StateTransition {
                from: sp("playing".into()),
                to: sp("abandoned".into()),
                condition: None,
            }),
        ],
    };
    let top = sp(TopLevel::States(states.clone()));
    let _ = format!("{:?}", top);
}

// ── type definition ──────────────────────────────────────────

#[test]
fn ast_type_def_decl() {
    let type_def = TypeDefDecl {
        name: sp("MoveRequest".into()),
        fields: vec![
            sp(FieldDef {
                name: "room_id".into(),
                type_name: sp(TypeName::Text),
            }),
            sp(FieldDef {
                name: "cell".into(),
                type_name: sp(TypeName::Number),
            }),
            sp(FieldDef {
                name: "token".into(),
                type_name: sp(TypeName::Text),
            }),
        ],
    };
    let top = sp(TopLevel::TypeDef(type_def.clone()));
    let _ = format!("{:?}", top);
}

// ── endpoint declaration ─────────────────────────────────────

#[test]
fn ast_endpoint_decl() {
    let endpoint = EndpointDecl {
        name: sp("move_endpoint".into()),
        params: vec![sp(Param {
            name: "req".into(),
            type_name: sp(TypeName::Custom("MoveRequest".into())),
        })],
        return_type: Some(sp(OutputType {
            types: vec![
                sp(TypeName::Custom("GameState".into())),
                sp(TypeName::Custom("MoveError".into())),
            ],
        })),
        body: vec![sp(Stmt::Give(
            sp(Expr::Call(CallExpr {
                name: sp("process".into()),
                args: vec![sp(CallArg {
                    label: None,
                    value: sp(Expr::Ident("req".into())),
                })],
            })),
            vec![],
        ))],
    };
    let top = sp(TopLevel::Endpoint(endpoint.clone()));
    let _ = format!("{:?}", top);
}

// ── timer field ──────────────────────────────────────────────

#[test]
fn ast_timer_field() {
    let timer = TimerField {
        name: sp("reconnect_window".into()),
        duration: sp(Duration {
            value: 30,
            unit: DurationUnit::Seconds,
        }),
    };
    let _ = format!("{:?}", timer.clone());
}

// ── subscribe declaration ────────────────────────────────────

#[test]
fn ast_subscribe_decl() {
    let sub = SubscribeDecl {
        event_name: sp("MoveEvent".into()),
        filter: Some(sp(Expr::BinOp(
            Box::new(sp(Expr::FieldAccess(
                Box::new(sp(Expr::Ident("event_val".into()))),
                sp("room_id".into()),
            ))),
            sp(BinOp::Eq),
            Box::new(sp(Expr::Ident("target".into()))),
        ))),
    };
    let _ = format!("{:?}", sub.clone());

    let sub_no_filter = SubscribeDecl {
        event_name: sp("GameEndEvent".into()),
        filter: None,
    };
    let _ = format!("{:?}", sub_no_filter.clone());
}

// ── requires clause ──────────────────────────────────────────

#[test]
fn ast_requires_clause() {
    let req = RequiresClause {
        condition: sp(Expr::BinOp(
            Box::new(sp(Expr::Ident("lifecycle".into()))),
            sp(BinOp::Eq),
            Box::new(sp(Expr::Ident("playing".into()))),
        )),
        on_fail: Some(sp(FailPolicy::Silent)),
    };
    let _ = format!("{:?}", req.clone());

    let req_give = RequiresClause {
        condition: sp(Expr::Ident("valid".into())),
        on_fail: Some(sp(FailPolicy::Give(sp(Expr::Template(vec![sp(
            TemplatePart::Text("invalid".into()),
        )]))))),
    };
    let _ = format!("{:?}", req_give.clone());

    // All fail policies
    let _ = format!(
        "{:?} {:?} {:?} {:?}",
        FailPolicy::Silent,
        FailPolicy::Log,
        FailPolicy::Escalate,
        FailPolicy::Crash
    );
}

// ── agent with v3 extensions ─────────────────────────────────

#[test]
fn ast_agent_v3() {
    let agent = AgentDecl {
        exportable: false,
        name: sp("room_agent".into()),
        lifecycle: Some(sp("RoomLifecycle".into())),
        memory: vec![
            sp(FieldDef {
                name: "board".into(),
                type_name: sp(TypeName::Array(Box::new(TypeName::Text), Some(9))),
            }),
            sp(FieldDef {
                name: "turn".into(),
                type_name: sp(TypeName::Number),
            }),
        ],
        memory_persistent: false,
        knowledge: None,
        timers: vec![sp(TimerField {
            name: sp("reconnect_window".into()),
            duration: sp(Duration {
                value: 30,
                unit: DurationUnit::Seconds,
            }),
        })],
        schedules: vec![],
        correlates: vec![],
        subscriptions: vec![sp(SubscribeDecl {
            event_name: sp("MoveEvent".into()),
            filter: None,
        })],
        handlers: vec![
            sp(OnHandler {
                event: sp("move".into()),
                params: vec![
                    sp(Param {
                        name: "player".into(),
                        type_name: sp(TypeName::Text),
                    }),
                    sp(Param {
                        name: "cell".into(),
                        type_name: sp(TypeName::Number),
                    }),
                ],
                payload_type: None,
                requires: vec![sp(RequiresClause {
                    condition: sp(Expr::BinOp(
                        Box::new(sp(Expr::Ident("lifecycle".into()))),
                        sp(BinOp::Eq),
                        Box::new(sp(Expr::Ident("playing".into()))),
                    )),
                    on_fail: Some(sp(FailPolicy::Silent)),
                })],
                body: vec![sp(Stmt::Say(sp(Expr::Ident("player".into()))))],
            }),
            sp(OnHandler {
                event: sp("reconnect_window.expired".into()),
                params: vec![sp(Param {
                    name: "player".into(),
                    type_name: sp(TypeName::Text),
                })],
                payload_type: None,
                requires: vec![],
                body: vec![sp(Stmt::TransitionTo(sp("done".into())))],
            }),
        ],
        warden_override: Vec::new(),
        stuck_policy: None,
    };
    let _ = format!("{:?}", agent.clone());
}

// ── v3 statement types ───────────────────────────────────────

#[test]
fn ast_emit_stmt() {
    let stmt = Stmt::Emit(
        sp("MoveEvent".into()),
        vec![
            sp(CallArg {
                label: None,
                value: sp(Expr::Ident("room".into())),
            }),
            sp(CallArg {
                label: None,
                value: sp(Expr::Ident("cell".into())),
            }),
        ],
    );
    let _ = format!("{:?}", stmt.clone());
}

#[test]
fn ast_transition_stmt() {
    let stmt = Stmt::TransitionTo(sp("playing".into()));
    let _ = format!("{:?}", stmt.clone());
}

#[test]
fn ast_timer_stmts() {
    let start = Stmt::StartTimer {
        name: sp("reconnect_window".into()),
        context: Some(sp(Expr::Ident("player".into()))),
    };
    let cancel = Stmt::CancelTimer {
        name: sp("reconnect_window".into()),
        context: Some(sp(Expr::Ident("player".into()))),
    };
    let reset = Stmt::ResetTimer(sp("turn_limit".into()));
    let _ = format!(
        "{:?} {:?} {:?}",
        start.clone(),
        cancel.clone(),
        reset.clone()
    );
}

#[test]
fn ast_forward_stmt() {
    let stmt = Stmt::Forward(
        sp(Expr::Ident("msg".into())),
        sp(Expr::Ident("target".into())),
    );
    let _ = format!("{:?}", stmt.clone());
}

#[test]
fn ast_memory_update_with_index() {
    let stmt = Stmt::MemoryUpdate(
        sp("board".into()),
        Some(sp(Expr::Ident("cell".into()))),
        sp(Expr::Ident("symbol".into())),
    );
    let _ = format!("{:?}", stmt.clone());
}

// ── match statement ──────────────────────────────────────────

#[test]
fn ast_match_block() {
    let m = MatchBlock {
        subject: sp(Expr::Ident("outcome".into())),
        arms: vec![
            sp(MatchArm {
                pattern: sp(Pattern::Constructor(
                    "Winner".into(),
                    vec![sp(Pattern::Binding("sym".into()))],
                )),
                body: sp(Stmt::Give(sp(Expr::Ident("sym".into())), vec![])),
            }),
            sp(MatchArm {
                pattern: sp(Pattern::Constructor("Draw".into(), vec![])),
                body: sp(Stmt::Give(
                    sp(Expr::Template(vec![sp(TemplatePart::Text("draw".into()))])),
                    vec![],
                )),
            }),
            sp(MatchArm {
                pattern: sp(Pattern::Wildcard),
                body: sp(Stmt::Give(
                    sp(Expr::Template(vec![sp(TemplatePart::Text(
                        "ongoing".into(),
                    ))])),
                    vec![],
                )),
            }),
        ],
    };
    let stmt = Stmt::Match(Box::new(m));
    let _ = format!("{:?}", stmt.clone());
}

// ── if/else block ────────────────────────────────────────────

#[test]
fn ast_if_else_block() {
    let block = IfElseBlock {
        condition: sp(Expr::BinOp(
            Box::new(sp(Expr::Ident("x".into()))),
            sp(BinOp::Gt),
            Box::new(sp(Expr::NumberLit(0.0))),
        )),
        then_body: vec![sp(Stmt::Give(sp(Expr::Ident("x".into())), vec![]))],
        else_ifs: vec![(
            sp(Expr::BinOp(
                Box::new(sp(Expr::Ident("x".into()))),
                sp(BinOp::Eq),
                Box::new(sp(Expr::NumberLit(0.0))),
            )),
            vec![sp(Stmt::Give(sp(Expr::NumberLit(0.0)), vec![]))],
        )],
        else_body: Some(vec![sp(Stmt::Give(
            sp(Expr::UnaryOp(
                sp(UnaryOp::Neg),
                Box::new(sp(Expr::Ident("x".into()))),
            )),
            vec![],
        ))]),
    };
    let stmt = Stmt::IfElse(Box::new(block));
    let _ = format!("{:?}", stmt.clone());
}

// ── for loop ─────────────────────────────────────────────────

#[test]
fn ast_for_loop() {
    let f = ForLoop {
        binding: sp("item".into()),
        iterable: sp(Expr::Ident("list".into())),
        body: vec![sp(Stmt::Say(sp(Expr::Ident("item".into()))))],
    };
    let stmt = Stmt::For(Box::new(f));
    let _ = format!("{:?}", stmt.clone());
}

// ── v3 expression types ──────────────────────────────────────

#[test]
fn ast_array_lit() {
    let e = Expr::ArrayLit(vec![
        sp(Expr::NumberLit(0.0)),
        sp(Expr::NumberLit(1.0)),
        sp(Expr::NumberLit(2.0)),
    ]);
    let _ = format!("{:?}", e.clone());
}

#[test]
fn ast_index_expr() {
    let e = Expr::Index(
        Box::new(sp(Expr::Ident("board".into()))),
        Box::new(sp(Expr::Ident("cell".into()))),
    );
    let _ = format!("{:?}", e.clone());
}

#[test]
fn ast_method_call() {
    let e = Expr::MethodCall(
        Box::new(sp(Expr::Ident("board".into()))),
        sp("none".into()),
        vec![sp(CallArg {
            label: None,
            value: sp(Expr::Ident("empty".into())),
        })],
    );
    let _ = format!("{:?}", e.clone());
}

#[test]
fn ast_bin_ops() {
    let ops = vec![
        BinOp::Add,
        BinOp::Sub,
        BinOp::Mul,
        BinOp::Div,
        BinOp::Eq,
        BinOp::Ne,
        BinOp::Lt,
        BinOp::Gt,
        BinOp::Le,
        BinOp::Ge,
        BinOp::And,
        BinOp::Or,
    ];
    for op in &ops {
        let e = Expr::BinOp(
            Box::new(sp(Expr::Ident("a".into()))),
            sp(*op),
            Box::new(sp(Expr::Ident("b".into()))),
        );
        let _ = format!("{:?}", e.clone());
    }
}

#[test]
fn ast_unary_ops() {
    let not = Expr::UnaryOp(sp(UnaryOp::Not), Box::new(sp(Expr::Ident("x".into()))));
    let neg = Expr::UnaryOp(sp(UnaryOp::Neg), Box::new(sp(Expr::NumberLit(1.0))));
    let _ = format!("{:?} {:?}", not.clone(), neg.clone());
}

#[test]
fn ast_array_type_name() {
    let fixed = TypeName::Array(Box::new(TypeName::Text), Some(9));
    let dynamic = TypeName::Array(Box::new(TypeName::Custom("Player".into())), None);
    let _ = format!("{:?} {:?}", fixed.clone(), dynamic.clone());
}

// ── pattern types ────────────────────────────────────────────

#[test]
fn ast_patterns() {
    let w = Pattern::Wildcard;
    let b = Pattern::Binding("sym".into());
    let c = Pattern::Constructor("Winner".into(), vec![sp(Pattern::Binding("sym".into()))]);
    let nested = Pattern::Constructor(
        "Pair".into(),
        vec![
            sp(Pattern::Constructor(
                "Some".into(),
                vec![sp(Pattern::Binding("x".into()))],
            )),
            sp(Pattern::Wildcard),
        ],
    );
    let _ = format!(
        "{:?} {:?} {:?} {:?}",
        w.clone(),
        b.clone(),
        c.clone(),
        nested.clone()
    );
}
