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

#[derive(Debug, Clone)]
pub struct CostEstimate {
    pub operations: Vec<OpEstimate>,
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
            cost_per_1k_in: cost_in,
            cost_per_1k_out: cost_out,
        }
    }

    fn add_op(&mut self, kind: &'static str, location: String, tokens_in: u32, tokens_out: u32) {
        let cost = (tokens_in as f32 * self.cost_per_1k_in
            + tokens_out as f32 * self.cost_per_1k_out)
            / 1000.0;
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
            _ => {}
        }
    }

    fn walk_expr(&mut self, expr: &Spanned<Expr>, context: &str, multiplier: u32) {
        match &expr.node {
            Expr::Reason(inner) => {
                let tokens_in = estimate_prompt_tokens(inner) * multiplier;
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
            Expr::MethodCall(inner, _, args) => {
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
        total_tokens_in: total_in,
        total_tokens_out: total_out,
        total_cost_usd: total_cost,
    }
}
