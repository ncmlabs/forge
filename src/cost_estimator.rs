use crate::ast::*;
use crate::config::ForgeConfig;

#[derive(Debug, Clone)]
pub struct OpEstimate {
    pub kind: &'static str,
    pub location: String,
    pub estimated_tokens_in: u32,
    pub estimated_tokens_out: u32,
    pub estimated_cost_usd: f32,
}

/// A side effect the program performs when it runs: a `command`, `exec`, HTTP
/// call, `file.write`, `session` or agent `spawn` (#477). `location` is the
/// declaration the site sits in, the same context string operations use.
#[derive(Debug, Clone)]
pub struct EffectSite {
    pub kind: &'static str,
    pub location: String,
}

#[derive(Debug, Clone)]
pub struct CostEstimate {
    pub operations: Vec<OpEstimate>,
    /// Effect sites in walk order. An effect costs no tokens, so it never
    /// appears in `operations`.
    pub effects: Vec<EffectSite>,
    pub total_tokens_in: u32,
    pub total_tokens_out: u32,
    pub total_cost_usd: f32,
}

impl std::fmt::Display for CostEstimate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.operations.is_empty() {
            return writeln!(f, "No LLM operations found.");
        }

        writeln!(
            f,
            "{:<12} {:<30} {:>10} {:>10} {:>10}",
            "Operation", "Location", "Tokens In", "Tokens Out", "Est. Cost"
        )?;
        writeln!(f, "{}", "-".repeat(76))?;

        for op in &self.operations {
            writeln!(
                f,
                "{:<12} {:<30} {:>10} {:>10} {:>10.4}",
                op.kind,
                op.location,
                op.estimated_tokens_in,
                op.estimated_tokens_out,
                op.estimated_cost_usd
            )?;
        }

        writeln!(f, "{}", "-".repeat(76))?;
        writeln!(
            f,
            "{:<12} {:<30} {:>10} {:>10} {:>10.4}",
            "Total", "", self.total_tokens_in, self.total_tokens_out, self.total_cost_usd
        )?;
        Ok(())
    }
}

struct CostWalker {
    operations: Vec<OpEstimate>,
    effects: Vec<EffectSite>,
    cost_per_1k_in: f32,
    cost_per_1k_out: f32,
}

impl CostWalker {
    fn new(config: &ForgeConfig) -> Self {
        let default_provider = config.providers.get(&config.llm.default);
        let (cost_in, cost_out) = default_provider
            .and_then(|p| p.capabilities.as_ref())
            .map(|c| {
                (
                    c.cost_per_1k_input.unwrap_or(0.003),
                    c.cost_per_1k_output.unwrap_or(0.015),
                )
            })
            .unwrap_or((0.003, 0.015));

        Self {
            operations: Vec::new(),
            effects: Vec::new(),
            cost_per_1k_in: cost_in,
            cost_per_1k_out: cost_out,
        }
    }

    /// Record one effect site. Sites are facts, not estimates: a loop body
    /// holds one site, not one per estimated iteration.
    fn add_effect(&mut self, kind: &'static str, location: &str) {
        self.effects.push(EffectSite {
            kind,
            location: location.to_string(),
        });
    }

    fn add_op(&mut self, kind: &'static str, location: String, tokens_in: u32, tokens_out: u32) {
        let cost = (tokens_in as f32 * self.cost_per_1k_in
            + tokens_out as f32 * self.cost_per_1k_out)
            / 1000.0;
        self.add_explicit_cost(kind, location, tokens_in, tokens_out, cost);
    }

    fn add_explicit_cost(
        &mut self,
        kind: &'static str,
        location: String,
        tokens_in: u32,
        tokens_out: u32,
        cost: f32,
    ) {
        self.operations.push(OpEstimate {
            kind,
            location,
            estimated_tokens_in: tokens_in,
            estimated_tokens_out: tokens_out,
            estimated_cost_usd: cost,
        });
    }

    fn walk_program(&mut self, program: &Program) {
        for item in &program.items {
            match &item.node {
                TopLevel::Task(task) => {
                    let ctx = format!("task {}", task.name.node);
                    match &task.body.node {
                        TaskBody::Do(stmts) => self.walk_stmts(stmts, &ctx, 1),
                        TaskBody::Is(expr) => self.walk_expr(expr, &ctx, 1),
                    }
                }
                TopLevel::Pure(_) => {} // no LLM ops
                TopLevel::Flow(flow) => {
                    for stage in &flow.stages {
                        let ctx = format!("flow {}.{}", flow.name.node, stage.node.name.node);
                        self.walk_stmts(&stage.node.body, &ctx, 1);
                    }
                }
                TopLevel::Agent(agent) => {
                    for handler in &agent.handlers {
                        let ctx =
                            format!("agent {}.on {}", agent.name.node, handler.node.event.node);
                        self.walk_stmts(&handler.node.body, &ctx, 1);
                    }
                }
                TopLevel::Endpoint(ep) => {
                    let ctx = format!("endpoint {}", ep.name.node);
                    self.walk_stmts(&ep.body, &ctx, 1);
                }
                TopLevel::FnMain(main) => {
                    self.walk_stmts(&main.body, "fn main", 1);
                }
                _ => {}
            }
        }
    }

    fn walk_stmts(&mut self, stmts: &[Spanned<Stmt>], context: &str, multiplier: u32) {
        for stmt in stmts {
            self.walk_stmt(stmt, context, multiplier);
        }
    }

    fn walk_stmt(&mut self, stmt: &Spanned<Stmt>, context: &str, multiplier: u32) {
        match &stmt.node {
            Stmt::Bind(_, expr) | Stmt::Say(expr) | Stmt::ExprStmt(expr) => {
                self.walk_expr(expr, context, multiplier);
            }
            Stmt::Give(expr, metas) => {
                self.walk_expr(expr, context, multiplier);
                for meta in metas {
                    self.walk_expr(&meta.node.value, context, multiplier);
                }
            }
            Stmt::When(when) => {
                for clause in &when.clauses {
                    self.walk_stmt(&clause.node.body, context, multiplier);
                }
                if let Some(else_clause) = &when.else_body {
                    self.walk_stmt(&else_clause.node.body, context, multiplier);
                }
            }
            Stmt::Match(m) => {
                self.walk_expr(&m.subject, context, multiplier);
                for arm in &m.arms {
                    self.walk_stmt(&arm.node.body, context, multiplier);
                }
            }
            Stmt::IfElse(ie) => {
                self.walk_expr(&ie.condition, context, multiplier);
                self.walk_stmts(&ie.then_body, context, multiplier);
                for (cond, body) in &ie.else_ifs {
                    self.walk_expr(cond, context, multiplier);
                    self.walk_stmts(body, context, multiplier);
                }
                if let Some(body) = &ie.else_body {
                    self.walk_stmts(body, context, multiplier);
                }
            }
            Stmt::For(f) => {
                self.walk_expr(&f.iterable, context, multiplier);
                // Heuristic: assume 5 iterations
                self.walk_stmts(&f.body, context, multiplier * 5);
            }
            Stmt::Forward(a, b) => {
                self.walk_expr(a, context, multiplier);
                self.walk_expr(b, context, multiplier);
            }
            Stmt::Emit(_, args) => {
                for arg in args {
                    self.walk_expr(&arg.node.value, context, multiplier);
                }
            }
            Stmt::Spawn(spawn) => {
                // spawn has zero token cost (no LLM call) but starts an agent
                self.add_effect("spawn", context);
                if let Some(alias) = &spawn.alias {
                    self.walk_expr(alias, context, multiplier);
                }
                for option in &spawn.options {
                    match &option.node {
                        SpawnOption::ConfidenceCap(expr) => {
                            self.walk_expr(expr, context, multiplier)
                        }
                        SpawnOption::MemoryInit(_, expr) => {
                            self.walk_expr(expr, context, multiplier)
                        }
                        SpawnOption::Isolate(config) => {
                            self.walk_expr(&config.branch, context, multiplier)
                        }
                        // `where category == "X"` is a literal, not an expression.
                        SpawnOption::KnowledgeFilter(_) => {}
                    }
                }
            }
            Stmt::MemoryUpdate(_, index, value) => {
                if let Some(index) = index {
                    self.walk_expr(index, context, multiplier);
                }
                self.walk_expr(value, context, multiplier);
            }
            Stmt::StartTimer {
                context: timer_context,
                ..
            }
            | Stmt::CancelTimer {
                context: timer_context,
                ..
            } => {
                if let Some(expr) = timer_context {
                    self.walk_expr(expr, context, multiplier);
                }
            }
            Stmt::Learn(source, category) => {
                match &source.node {
                    LearnSource::Direct(expr) | LearnSource::FromDocument(expr) => {
                        self.walk_expr(expr, context, multiplier)
                    }
                    LearnSource::FromInteraction(args) => {
                        for arg in args {
                            self.walk_expr(&arg.node.value, context, multiplier);
                        }
                    }
                }
                if let Some(category) = category {
                    self.walk_expr(category, context, multiplier);
                }
            }
            Stmt::Retire(retire) => {
                if let Some(target) = &retire.target {
                    self.walk_expr(target, context, multiplier);
                }
                if let Some(export) = &retire.knowledge_export {
                    self.walk_expr(export, context, multiplier);
                }
            }
            // These carry a bare name (state, timer, escalation target), never an
            // expression — so there is nothing to walk. Listed explicitly, with
            // no catch-all arm, so a new `Stmt` variant is a compile error here
            // instead of a silent hole in the plan (#477).
            Stmt::Escalate(_) | Stmt::TransitionTo(_) | Stmt::ResetTimer(_) => {}
        }
    }

    fn walk_expr(&mut self, expr: &Spanned<Expr>, context: &str, multiplier: u32) {
        match &expr.node {
            Expr::Reason(reason) => {
                let tokens_in = estimate_prompt_tokens(&reason.prompt) * multiplier;
                let tokens_out = 1000 * multiplier;
                self.add_op("reason", context.to_string(), tokens_in, tokens_out);
            }
            Expr::Classify(cls) => {
                let tokens_in = estimate_prompt_tokens(&cls.input) * multiplier;
                let tokens_out = 50 * multiplier;
                self.add_op("classify", context.to_string(), tokens_in, tokens_out);
            }
            Expr::Search(inner) => {
                let tokens_in = estimate_prompt_tokens(inner) * multiplier;
                let tokens_out = 200 * multiplier;
                self.add_op("search", context.to_string(), tokens_in, tokens_out);
            }
            Expr::Exec(_) => {
                // exec has zero token cost (direct CLI, no LLM)
                self.add_effect("exec", context);
            }
            Expr::Command(_) | Expr::CommandMethod(_, _) => {
                // command has zero token cost (direct CLI, no LLM)
                self.add_effect("command", context);
            }
            Expr::SessionMethod(_, _) => {
                // a session handle method drives an already-running session
                self.add_effect("session", context);
            }
            Expr::Session(session) => {
                self.add_effect("session", context);
                self.walk_expr(&session.name, context, multiplier);
                if let Some(ref agent) = session.agent {
                    self.walk_expr(agent, context, multiplier);
                }
                if let Some(ref prompt) = session.prompt {
                    self.walk_expr(prompt, context, multiplier);
                }
                if let Some(ref tools) = session.tools {
                    self.walk_expr(tools, context, multiplier);
                }
                if let Some(ref budget) = session.budget {
                    self.walk_expr(budget, context, multiplier);
                    if let Expr::NumberLit(amount) = &budget.node {
                        self.add_explicit_cost(
                            "session",
                            context.to_string(),
                            0,
                            0,
                            *amount as f32 * multiplier as f32,
                        );
                    }
                }
                if let Some(ref hook) = session.on_progress {
                    for arg in &hook.node.args {
                        self.walk_expr(&arg.node.value, context, multiplier);
                    }
                }
                if let Some(ref hook) = session.on_complete {
                    for arg in &hook.node.args {
                        self.walk_expr(&arg.node.value, context, multiplier);
                    }
                }
            }
            Expr::TryOr(a, b) => {
                self.walk_expr(a, context, multiplier);
                self.walk_expr(b, context, multiplier);
            }
            Expr::Compose(parts) => {
                for p in parts {
                    self.walk_expr(p, context, multiplier);
                }
            }
            Expr::FanOut(parts) => {
                for p in parts {
                    self.walk_expr(p, context, multiplier);
                }
            }
            Expr::BinOp(a, _, b) => {
                self.walk_expr(a, context, multiplier);
                self.walk_expr(b, context, multiplier);
            }
            Expr::UnaryOp(_, a) => {
                self.walk_expr(a, context, multiplier);
            }
            Expr::Call(c) => {
                for arg in &c.args {
                    self.walk_expr(&arg.node.value, context, multiplier);
                }
            }
            Expr::Constructor(c) => {
                for arg in &c.args {
                    self.walk_expr(&arg.node.value, context, multiplier);
                }
            }
            Expr::FieldAccess(inner, _) | Expr::GlobAccess(inner) => {
                self.walk_expr(inner, context, multiplier);
            }
            Expr::Index(a, b) => {
                self.walk_expr(a, context, multiplier);
                self.walk_expr(b, context, multiplier);
            }
            Expr::MethodCall(inner, method, args) => {
                // `file.write` and `web.fetch`/`web.post` are effects; every
                // other namespace method is not (yet) one.
                if let Expr::Ident(ns) = &inner.node {
                    match (ns.as_str(), method.node.as_str()) {
                        ("file", "write") => self.add_effect("file.write", context),
                        ("web", "fetch") | ("web", "post") => self.add_effect("http", context),
                        _ => {}
                    }
                }
                self.walk_expr(inner, context, multiplier);
                for arg in args {
                    self.walk_expr(&arg.node.value, context, multiplier);
                }
            }
            Expr::Paren(inner) => {
                self.walk_expr(inner, context, multiplier);
            }
            Expr::ArrayLit(elems) => {
                for e in elems {
                    self.walk_expr(e, context, multiplier);
                }
            }
            Expr::Template(parts) => {
                for part in parts {
                    match &part.node {
                        TemplatePart::Interp(inner) | TemplatePart::RawInterp(inner) => {
                            self.walk_expr(inner, context, multiplier);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

fn estimate_prompt_tokens(expr: &Spanned<Expr>) -> u32 {
    match &expr.node {
        Expr::Template(parts) => {
            let char_count: usize = parts
                .iter()
                .map(|p| match &p.node {
                    TemplatePart::Text(s) => s.len(),
                    TemplatePart::Interp(_) | TemplatePart::RawInterp(_) => 50, // assume ~50 chars for interpolated values
                })
                .sum();
            // ~4 chars per token
            (char_count as u32 / 4).max(10)
        }
        _ => 500, // default for dynamic expressions
    }
}

pub fn estimate(program: &Program, config: &ForgeConfig) -> CostEstimate {
    let mut walker = CostWalker::new(config);
    walker.walk_program(program);

    let total_in: u32 = walker
        .operations
        .iter()
        .map(|o| o.estimated_tokens_in)
        .sum();
    let total_out: u32 = walker
        .operations
        .iter()
        .map(|o| o.estimated_tokens_out)
        .sum();
    let total_cost: f32 = walker.operations.iter().map(|o| o.estimated_cost_usd).sum();

    CostEstimate {
        operations: walker.operations,
        effects: walker.effects,
        total_tokens_in: total_in,
        total_tokens_out: total_out,
        total_cost_usd: total_cost,
    }
}

#[cfg(test)]
mod tests {
    use crate::config::ForgeConfig;
    use crate::parser::parse;

    #[test]
    fn session_budget_literal_adds_explicit_cost() {
        let program =
            parse("task t\n  gives Text\n  do\n    result = session \"review\" prompt \"check\" budget 2.5\n    give result\n")
                .unwrap();
        let estimate =
            crate::cost_estimator::estimate(&program, &ForgeConfig::default_mock_config());
        assert!(
            estimate.operations.iter().any(
                |op| op.kind == "session" && (op.estimated_cost_usd - 2.5).abs() < f32::EPSILON
            ),
            "expected explicit session budget cost, got {:?}",
            estimate.operations
        );
    }

    #[test]
    fn session_without_budget_does_not_invent_cost() {
        let program =
            parse("task t\n  gives Text\n  do\n    result = session \"review\" prompt \"check\"\n    give result\n")
                .unwrap();
        let estimate =
            crate::cost_estimator::estimate(&program, &ForgeConfig::default_mock_config());
        assert!(
            estimate.operations.iter().all(|op| op.kind != "session"),
            "session without explicit budget should not create a synthetic cost: {:?}",
            estimate.operations
        );
    }

    /// Effect sites for `source`, with the guarantee that an effect alone adds
    /// no operation and no tokens (issue #477: effects are not LLM ops).
    fn effect_sites(source: &str) -> Vec<(&'static str, String)> {
        let program = parse(source).unwrap();
        let estimate =
            crate::cost_estimator::estimate(&program, &ForgeConfig::default_mock_config());
        assert!(
            estimate.operations.is_empty(),
            "effects must not add operations: {:?}",
            estimate.operations
        );
        assert_eq!(
            (estimate.total_tokens_in, estimate.total_tokens_out),
            (0, 0),
            "effects must not change token totals"
        );
        estimate
            .effects
            .iter()
            .map(|e| (e.kind, e.location.clone()))
            .collect()
    }

    #[test]
    fn exec_is_an_effect_site() {
        assert_eq!(
            effect_sites("fn main\n  out = exec \"ls\"\n"),
            vec![("exec", "fn main".to_string())]
        );
    }

    #[test]
    fn command_is_an_effect_site() {
        assert_eq!(
            effect_sites("task t\n  gives Text\n  do\n    result = command \"echo hi\"\n    give result.stdout\n"),
            vec![("command", "task t".to_string())]
        );
    }

    #[test]
    fn session_is_an_effect_site() {
        assert_eq!(
            effect_sites("fn main\n  result = session \"review\" prompt \"check\"\n"),
            vec![("session", "fn main".to_string())]
        );
    }

    #[test]
    fn spawn_is_an_effect_site() {
        assert_eq!(
            effect_sites("fn main\n  spawn Worker as \"w\"\n"),
            vec![("spawn", "fn main".to_string())]
        );
    }

    /// The runtime evaluates the alias and every spawn option, so an `exec`
    /// hidden in one of them is a real effect site (issue #477 review).
    #[test]
    fn spawn_walks_its_alias_and_options() {
        assert_eq!(
            effect_sites(
                r#"agent W
  on start
    say "up"

fn main
  spawn W as "w-{exec "touch alias"}"
      with confidence_cap: exec "touch cap"
      with memory note: exec "touch mem"
      isolate worktree "b-{exec "touch iso"}"
"#
            ),
            vec![
                ("spawn", "fn main".to_string()),
                ("exec", "fn main".to_string()),
                ("exec", "fn main".to_string()),
                ("exec", "fn main".to_string()),
                ("exec", "fn main".to_string()),
            ]
        );
    }

    #[test]
    fn memory_update_walks_its_value() {
        assert_eq!(
            effect_sites(
                "agent A\n  memory\n    log: Text\n  on tick\n    memory.log = exec \"touch mem\"\n"
            ),
            vec![("exec", "agent A.on tick".to_string())]
        );
    }

    #[test]
    fn start_timer_walks_its_context() {
        assert_eq!(
            effect_sites("agent A\n  timer t: 5m\n  on tick\n    start t for exec \"touch t1\"\n"),
            vec![("exec", "agent A.on tick".to_string())]
        );
    }

    #[test]
    fn cancel_timer_walks_its_context() {
        assert_eq!(
            effect_sites("agent A\n  timer t: 5m\n  on tick\n    cancel t for exec \"touch t2\"\n"),
            vec![("exec", "agent A.on tick".to_string())]
        );
    }

    #[test]
    fn retire_walks_its_target_and_export() {
        assert_eq!(
            effect_sites(
                r#"agent A
  on tick
    retire "t-{exec "touch r"}"
      with knowledge export: "e-{exec "touch r2"}"
"#
            ),
            vec![
                ("exec", "agent A.on tick".to_string()),
                ("exec", "agent A.on tick".to_string()),
            ]
        );
    }

    #[test]
    fn learn_walks_its_source_and_category() {
        assert_eq!(
            effect_sites(
                r#"agent A
  on tick
    learn "f-{exec "touch l"}" category: "c-{exec "touch l2"}"
"#
            ),
            vec![
                ("exec", "agent A.on tick".to_string()),
                ("exec", "agent A.on tick".to_string()),
            ]
        );
    }

    #[test]
    fn file_write_is_an_effect_site() {
        assert_eq!(
            effect_sites("fn main\n  file.write(\"x\", \"y\")\n"),
            vec![("file.write", "fn main".to_string())]
        );
    }

    #[test]
    fn web_fetch_is_an_http_effect_site() {
        assert_eq!(
            effect_sites("fn main\n  body = web.fetch(\"http://example.com\")\n"),
            vec![("http", "fn main".to_string())]
        );
    }
}
