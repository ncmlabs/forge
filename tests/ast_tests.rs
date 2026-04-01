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
    let s3 = s.clone();
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
                body: sp(Stmt::Give(sp(Expr::Ident("result".into())), None)),
            }),
            sp(WhenClause {
                predicate: sure,
                body: sp(Stmt::Give(
                    sp(Expr::Ident("result".into())),
                    Some(sp(Expr::Call(CallExpr {
                        name: sp("flag".into()),
                        args: vec![sp(CallArg {
                            label: None,
                            value: sp(Expr::Template(vec![sp(TemplatePart::Text(
                                "low-confidence".into(),
                            ))])),
                        })],
                    }))),
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
                    None,
                )),
            }),
        ],
        else_body: Some(sp(ElseClause {
            body: sp(Stmt::Give(
                sp(Expr::TypeAccess(sp(TypeName::Intent), sp("unknown".into()))),
                None,
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
            sp(Stmt::Give(sp(Expr::Ident("result".into())), None)),
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
            None,
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
                        None,
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
        name: sp("support_bot".into()),
        memory: vec![
            sp(MemoryField {
                name: "history".into(),
                type_name: sp(TypeName::Conversation),
            }),
            sp(MemoryField {
                name: "user".into(),
                type_name: sp(TypeName::Profile),
            }),
        ],
        handlers: vec![
            sp(OnHandler {
                event: sp("message".into()),
                payload_type: Some(sp(TypeName::Text)),
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
                        sp(Expr::Ident("updated_history".into())),
                    )),
                    sp(Stmt::Give(sp(Expr::Ident("response".into())), None)),
                ],
            }),
            sp(OnHandler {
                event: sp("reset".into()),
                payload_type: None,
                body: vec![sp(Stmt::MemoryUpdate(
                    sp("history".into()),
                    sp(Expr::TypeAccess(
                        sp(TypeName::Conversation),
                        sp("empty".into()),
                    )),
                ))],
            }),
        ],
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
                value: sp(Expr::Template(vec![sp(TemplatePart::Text(
                    "world".into(),
                ))])),
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
        Duration { value: 10, unit: DurationUnit::Seconds },
        Duration { value: 5, unit: DurationUnit::Minutes },
        Duration { value: 1, unit: DurationUnit::Hours },
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
                body: sp(TaskBody::Do(vec![sp(Stmt::Say(sp(Expr::Template(
                    vec![
                        sp(TemplatePart::Text("Hello, ".into())),
                        sp(TemplatePart::Interp(Box::new(sp(Expr::Ident(
                            "name".into(),
                        ))))),
                    ],
                ))))])),
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
