// FORGE task executor
// Walks the AST, evaluates expressions, dispatches on confidence predicates,
// calls LLM providers. See issue #9.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::ast::*;
use crate::llm::registry::ProviderRegistry;
use crate::llm::CompletionRequest;
use crate::runtime::confidence::{ConfidentValue, Value};
use crate::tracer::{Tracer, LLMResponseInfo};
// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("undefined variable: {name}")]
    UndefinedVariable { name: String },

    #[error("type error: expected {expected}, got {got}")]
    TypeError { expected: String, got: String },

    #[error("no fn main entry point — add a fn main block")]
    NoMainFunction,

    #[error("not callable: {name}")]
    NotCallable { name: String },

    #[error("division by zero")]
    DivisionByZero,

    #[error("index out of bounds: {index} (length {len})")]
    IndexOutOfBounds { index: usize, len: usize },

    #[error("LLM error: {0}")]
    LLMError(String),

    #[error("not yet implemented: {0}")]
    Unsupported(String),

    #[error("flow error: {0}")]
    FlowError(String),

    // Control flow — not a real error, used to propagate `give` values
    #[error("give signal (internal)")]
    GiveSignal(ConfidentValue),
}

// ── Environment (scope stack) ─────────────────────────────────────────────────

struct Env {
    scopes: Vec<HashMap<String, ConfidentValue>>,
}

impl Env {
    fn new() -> Self {
        Self { scopes: vec![HashMap::new()] }
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn bind(&mut self, name: &str, value: ConfidentValue) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), value);
        }
    }

    fn lookup(&self, name: &str) -> Result<&ConfidentValue, RuntimeError> {
        for scope in self.scopes.iter().rev() {
            if let Some(val) = scope.get(name) {
                return Ok(val);
            }
        }
        Err(RuntimeError::UndefinedVariable { name: name.to_string() })
    }

    fn top_scope_bindings(&self) -> HashMap<String, ConfidentValue> {
        self.scopes.last().cloned().unwrap_or_default()
    }
}

// ── Task Executor ─────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct TaskExecutor {
    program:   Program,
    providers: Arc<ProviderRegistry>,
    tracer:    Option<Tracer>,
    task_map:  HashMap<String, TaskDecl>,
    pure_map:  HashMap<String, PureDecl>,
    flow_map:  HashMap<String, FlowDecl>,
    output:    Arc<Mutex<Vec<String>>>,
}

impl TaskExecutor {
    pub fn new(
        program: Program,
        providers: Arc<ProviderRegistry>,
        tracer: Option<Tracer>,
    ) -> Self {
        let mut task_map = HashMap::new();
        let mut pure_map = HashMap::new();
        let mut flow_map = HashMap::new();

        for item in &program.items {
            match &item.node {
                TopLevel::Task(t) => {
                    task_map.insert(t.name.node.clone(), t.clone());
                }
                TopLevel::Pure(p) => {
                    pure_map.insert(p.name.node.clone(), p.clone());
                }
                TopLevel::Flow(f) => {
                    flow_map.insert(f.name.node.clone(), f.clone());
                }
                _ => {}
            }
        }

        Self {
            program,
            providers,
            tracer,
            task_map,
            pure_map,
            flow_map,
            output: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Get collected `say` output (for testing)
    pub fn outputs(&self) -> Vec<String> {
        self.output.lock().unwrap().clone()
    }

    /// Run the program starting from `fn main`
    pub async fn run(&self) -> Result<ConfidentValue, RuntimeError> {
        let main_decl = self.program.items.iter()
            .find_map(|item| match &item.node {
                TopLevel::FnMain(m) => Some(m),
                _ => None,
            })
            .ok_or(RuntimeError::NoMainFunction)?;

        let mut env = Env::new();
        self.exec_stmts(&main_decl.body, &mut env).await
    }

    // ── Statement execution ───────────────────────────────────────────────────

    fn exec_stmts<'a>(
        &'a self,
        stmts: &'a [Spanned<Stmt>],
        env: &'a mut Env,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ConfidentValue, RuntimeError>> + Send + 'a>> {
        Box::pin(async move {
        for stmt in stmts {
            match self.exec_stmt(stmt, env).await {
                Ok(()) => {}
                Err(RuntimeError::GiveSignal(val)) => return Ok(val),
                Err(e) => return Err(e),
            }
        }
        Ok(ConfidentValue::deterministic(Value::Unit))
        })
    }

    fn exec_stmt<'a>(
        &'a self,
        stmt: &'a Spanned<Stmt>,
        env: &'a mut Env,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), RuntimeError>> + Send + 'a>> {
        Box::pin(async move {
        match &stmt.node {
            Stmt::Bind(name, expr) => {
                let val = self.eval_expr(expr, env).await?;
                env.bind(&name.node, val);
            }

            Stmt::Say(expr) => {
                let val = self.eval_expr(expr, env).await?;
                let text = format!("{}", val.value);
                println!("{}", text);
                self.output.lock().unwrap().push(text);
            }

            Stmt::Give(expr, _with) => {
                let val = self.eval_expr(expr, env).await?;
                return Err(RuntimeError::GiveSignal(val));
            }

            Stmt::ExprStmt(expr) => {
                self.eval_expr(expr, env).await?;
            }

            // ── Control flow ──────────────────────────────────────────────

            Stmt::When(when_block) => {
                for clause in &when_block.clauses {
                    let pred = &clause.node.predicate.node;
                    let subject_val = env.lookup(&pred.subject.node)?;
                    let matches = match &pred.level.node {
                        ConfLevel::Sure(None)    => subject_val.sure(),
                        ConfLevel::Sure(Some(t)) => subject_val.sure_above(*t as f32),
                        ConfLevel::Unsure        => subject_val.unsure(),
                        ConfLevel::Unreliable    => subject_val.unreliable(),
                        ConfLevel::Conflicted    => subject_val.conflicted(),
                    };

                    if let Some(ref tracer) = self.tracer {
                        tracer.when_dispatch(
                            &pred.subject.node,
                            &format!("{:?}", pred.level.node),
                            matches,
                        );
                    }

                    if matches {
                        self.exec_stmt(&clause.node.body, env).await?;
                        return Ok(());
                    }
                }
                if let Some(else_clause) = &when_block.else_body {
                    if let Some(ref tracer) = self.tracer {
                        tracer.when_dispatch("_", "else", true);
                    }
                    self.exec_stmt(&else_clause.node.body, env).await?;
                }
            }

            Stmt::IfElse(block) => {
                let cond = self.eval_expr(&block.condition, env).await?;
                if truthy(&cond) {
                    self.exec_stmts(&block.then_body, env).await?;
                } else {
                    let mut handled = false;
                    for (cond_expr, body) in &block.else_ifs {
                        let c = self.eval_expr(cond_expr, env).await?;
                        if truthy(&c) {
                            self.exec_stmts(body, env).await?;
                            handled = true;
                            break;
                        }
                    }
                    if !handled {
                        if let Some(body) = &block.else_body {
                            self.exec_stmts(body, env).await?;
                        }
                    }
                }
            }

            Stmt::Match(block) => {
                let subject = self.eval_expr(&block.subject, env).await?;
                for arm in &block.arms {
                    if let Some(bindings) = match_pattern(&arm.node.pattern, &subject) {
                        env.push_scope();
                        for (name, val) in bindings {
                            env.bind(&name, val);
                        }
                        self.exec_stmt(&arm.node.body, env).await?;
                        env.pop_scope();
                        return Ok(());
                    }
                }
            }

            Stmt::For(for_loop) => {
                let iterable = self.eval_expr(&for_loop.iterable, env).await?;
                let items = match &iterable.value {
                    Value::Array(v) | Value::List(v) => v.clone(),
                    other => return Err(RuntimeError::TypeError {
                        expected: "Array or List".to_string(),
                        got: format!("{}", other),
                    }),
                };
                for item in items {
                    env.push_scope();
                    env.bind(&for_loop.binding.node, item);
                    match self.exec_stmts(&for_loop.body, env).await {
                        Ok(_) => { env.pop_scope(); }
                        Err(e) => { env.pop_scope(); return Err(e); }
                    }
                }
            }

            // ── Stubs for agent features (issue #11) ──────────────────────

            Stmt::Emit(name, _args) => {
                eprintln!("[forge] stub: emit {} (issue #11)", name.node);
            }
            Stmt::TransitionTo(state) => {
                eprintln!("[forge] stub: transition to {} (issue #11)", state.node);
            }
            Stmt::StartTimer { name, .. } => {
                eprintln!("[forge] stub: start timer {} (issue #11)", name.node);
            }
            Stmt::CancelTimer { name, .. } => {
                eprintln!("[forge] stub: cancel timer {} (issue #11)", name.node);
            }
            Stmt::ResetTimer(name) => {
                eprintln!("[forge] stub: reset timer {} (issue #11)", name.node);
            }
            Stmt::Forward(_expr, _target) => {
                eprintln!("[forge] stub: forward (issue #11)");
            }
            Stmt::MemoryUpdate(field, _idx, _expr) => {
                eprintln!("[forge] stub: memory.{} update (issue #11)", field.node);
            }
            Stmt::Escalate(target) => {
                eprintln!("[forge] stub: escalate to {} (issue #11)", target.node);
            }
        }
        Ok(())
        })
    }

    // ── Expression evaluation ─────────────────────────────────────────────────

    fn eval_expr<'a>(
        &'a self,
        expr: &'a Spanned<Expr>,
        env: &'a mut Env,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ConfidentValue, RuntimeError>> + Send + 'a>> {
        Box::pin(async move {
        match &expr.node {
            Expr::NumberLit(n) => {
                Ok(ConfidentValue::deterministic(Value::Number(*n)))
            }

            Expr::BoolLit(b) => {
                Ok(ConfidentValue::deterministic(Value::Bool(*b)))
            }

            Expr::Ident(name) => {
                Ok(env.lookup(name)?.clone())
            }

            Expr::Template(parts) => {
                let mut result = String::new();
                let mut min_conf: f32 = 1.0;
                for part in parts {
                    match &part.node {
                        TemplatePart::Text(s) => result.push_str(s),
                        TemplatePart::Interp(inner_expr) => {
                            let val = self.eval_expr(inner_expr, env).await?;
                            min_conf = min_conf.min(val.confidence);
                            result.push_str(&format!("{}", val.value));
                        }
                    }
                }
                if min_conf >= 1.0 {
                    Ok(ConfidentValue::deterministic(Value::Text(result)))
                } else {
                    Ok(ConfidentValue::derived(Value::Text(result), min_conf))
                }
            }

            Expr::ArrayLit(elems) => {
                let mut items = Vec::new();
                let mut min_conf: f32 = 1.0;
                for elem in elems {
                    let val = self.eval_expr(elem, env).await?;
                    min_conf = min_conf.min(val.confidence);
                    items.push(val);
                }
                if min_conf >= 1.0 {
                    Ok(ConfidentValue::deterministic(Value::Array(items)))
                } else {
                    Ok(ConfidentValue::derived(Value::Array(items), min_conf))
                }
            }

            Expr::Paren(inner) => {
                self.eval_expr(inner, env).await
            }

            Expr::BinOp(left, op, right) => {
                let lval = self.eval_expr(left, env).await?;
                let rval = self.eval_expr(right, env).await?;
                let min_conf = lval.confidence.min(rval.confidence);
                let result = eval_binop(&lval.value, &op.node, &rval.value)?;
                if min_conf >= 1.0 {
                    Ok(ConfidentValue::deterministic(result))
                } else {
                    Ok(ConfidentValue::derived(result, min_conf))
                }
            }

            Expr::UnaryOp(op, inner) => {
                let val = self.eval_expr(inner, env).await?;
                let result = match (&op.node, &val.value) {
                    (UnaryOp::Neg, Value::Number(n)) => Value::Number(-n),
                    (UnaryOp::Not, Value::Bool(b)) => Value::Bool(!b),
                    (UnaryOp::Not, other) => Value::Bool(!truthy_val(other)),
                    (UnaryOp::Neg, other) => return Err(RuntimeError::TypeError {
                        expected: "Number".to_string(),
                        got: format!("{}", other),
                    }),
                };
                if val.confidence >= 1.0 {
                    Ok(ConfidentValue::deterministic(result))
                } else {
                    Ok(ConfidentValue::derived(result, val.confidence))
                }
            }

            // ── Access expressions ────────────────────────────────────────

            Expr::FieldAccess(obj_expr, field) => {
                let obj = self.eval_expr(obj_expr, env).await?;
                match &obj.value {
                    Value::Record(fields) => {
                        fields.get(&field.node).cloned().ok_or_else(|| {
                            RuntimeError::UndefinedVariable {
                                name: format!(".{}", field.node),
                            }
                        })
                    }
                    _ => Err(RuntimeError::TypeError {
                        expected: "Record".to_string(),
                        got: format!("{}", obj.value),
                    }),
                }
            }

            Expr::Index(obj_expr, idx_expr) => {
                let obj = self.eval_expr(obj_expr, env).await?;
                let idx = self.eval_expr(idx_expr, env).await?;
                match (&obj.value, &idx.value) {
                    (Value::Array(items) | Value::List(items), Value::Number(n)) => {
                        let i = *n as usize;
                        items.get(i).cloned().ok_or(RuntimeError::IndexOutOfBounds {
                            index: i,
                            len: items.len(),
                        })
                    }
                    _ => Err(RuntimeError::TypeError {
                        expected: "Array[Number]".to_string(),
                        got: format!("{}[{}]", obj.value, idx.value),
                    }),
                }
            }

            Expr::MethodCall(obj_expr, method, args) => {
                let obj = self.eval_expr(obj_expr, env).await?;
                match method.node.as_str() {
                    "len" | "count" => {
                        let len = match &obj.value {
                            Value::Array(v) | Value::List(v) => v.len(),
                            Value::Text(s) => s.len(),
                            _ => return Err(RuntimeError::TypeError {
                                expected: "Array, List, or Text".to_string(),
                                got: format!("{}", obj.value),
                            }),
                        };
                        Ok(ConfidentValue::deterministic(Value::Number(len as f64)))
                    }
                    "contains" | "any" => {
                        if args.is_empty() {
                            return Err(RuntimeError::TypeError {
                                expected: "1 argument".to_string(),
                                got: "0 arguments".to_string(),
                            });
                        }
                        let needle = self.eval_expr(&args[0].node.value, env).await?;
                        let found = match &obj.value {
                            Value::Array(v) | Value::List(v) => {
                                v.iter().any(|item| values_equal(&item.value, &needle.value))
                            }
                            Value::Text(s) => {
                                if let Value::Text(needle_s) = &needle.value {
                                    s.contains(needle_s.as_str())
                                } else {
                                    false
                                }
                            }
                            _ => false,
                        };
                        Ok(ConfidentValue::deterministic(Value::Bool(found)))
                    }
                    "none" => {
                        if args.is_empty() {
                            return Err(RuntimeError::TypeError {
                                expected: "1 argument".to_string(),
                                got: "0 arguments".to_string(),
                            });
                        }
                        let needle = self.eval_expr(&args[0].node.value, env).await?;
                        let none = match &obj.value {
                            Value::Array(v) | Value::List(v) => {
                                !v.iter().any(|item| values_equal(&item.value, &needle.value))
                            }
                            _ => true,
                        };
                        Ok(ConfidentValue::deterministic(Value::Bool(none)))
                    }
                    other => Err(RuntimeError::Unsupported(
                        format!("method .{}()", other),
                    )),
                }
            }

            Expr::TypeAccess(_type_name, variant) => {
                Ok(ConfidentValue::deterministic(Value::Text(variant.node.clone())))
            }

            Expr::GlobAccess(inner) => {
                self.eval_expr(inner, env).await
            }

            // ── Calls ─────────────────────────────────────────────────────

            Expr::Call(call) => {
                let name = &call.name.node;
                let mut arg_vals = Vec::new();
                for arg in &call.args {
                    arg_vals.push(self.eval_expr(&arg.node.value, env).await?);
                }

                if let Some(task_decl) = self.task_map.get(name).cloned() {
                    if let Some(ref tracer) = self.tracer {
                        tracer.task_call(name);
                    }
                    let result = self.call_task(&task_decl, arg_vals).await;
                    if let Some(ref tracer) = self.tracer {
                        tracer.task_return(name, result.is_ok());
                    }
                    result
                } else if let Some(pure_decl) = self.pure_map.get(name).cloned() {
                    self.call_pure(&pure_decl, arg_vals).await
                } else if let Some(flow_decl) = self.flow_map.get(name).cloned() {
                    self.call_flow(&flow_decl, arg_vals).await
                } else {
                    Err(RuntimeError::NotCallable { name: name.clone() })
                }
            }

            Expr::Constructor(ctor) => {
                let mut fields = HashMap::new();
                for (i, arg) in ctor.args.iter().enumerate() {
                    let val = self.eval_expr(&arg.node.value, env).await?;
                    let key = arg.node.label.as_ref()
                        .map(|l| l.node.clone())
                        .unwrap_or_else(|| format!("_{}", i));
                    fields.insert(key, val);
                }
                Ok(ConfidentValue::deterministic(Value::Record(fields)))
            }

            // ── LLM expressions ───────────────────────────────────────────

            Expr::Reason(prompt_expr) => {
                let prompt = self.eval_expr(prompt_expr, env).await?;
                let prompt_text = format!("{}", prompt.value);

                if let Some(ref tracer) = self.tracer {
                    tracer.llm_request("reason", &prompt_text);
                }

                let request = CompletionRequest::simple(&prompt_text);
                let response = self.providers.resolve_and_complete(request, None).await
                    .map_err(|e| RuntimeError::LLMError(e.to_string()))?;
                let confidence = response.estimate_confidence();

                if let Some(ref tracer) = self.tracer {
                    tracer.llm_response(&LLMResponseInfo {
                        operation: "reason",
                        provider: &response.provider_name,
                        model: &response.model_used,
                        tokens_in: response.tokens_in,
                        tokens_out: response.tokens_out,
                        cost_usd: response.cost_usd,
                        confidence,
                    });
                }

                Ok(ConfidentValue::from_llm(Value::Text(response.content), confidence))
            }

            Expr::Classify(classify) => {
                let input = self.eval_expr(&classify.input, env).await?;
                let labels: Vec<String> = classify.labels.iter()
                    .map(|l| l.node.clone())
                    .collect();
                let prompt = format!(
                    "Classify the following into exactly one of these categories: {}\n\nInput: {}\n\nRespond with just the category name.",
                    labels.join(", "),
                    input.value,
                );

                if let Some(ref tracer) = self.tracer {
                    tracer.llm_request("classify", &prompt);
                }

                let request = CompletionRequest::simple(&prompt).with_temperature(0.0);
                let response = self.providers.resolve_and_complete(request, None).await
                    .map_err(|e| RuntimeError::LLMError(e.to_string()))?;
                let confidence = response.estimate_confidence();

                if let Some(ref tracer) = self.tracer {
                    tracer.llm_response(&LLMResponseInfo {
                        operation: "classify",
                        provider: &response.provider_name,
                        model: &response.model_used,
                        tokens_in: response.tokens_in,
                        tokens_out: response.tokens_out,
                        cost_usd: response.cost_usd,
                        confidence,
                    });
                }

                Ok(ConfidentValue::from_llm(
                    Value::Text(response.content.trim().to_string()),
                    confidence,
                ))
            }

            Expr::Search(_) => {
                Ok(ConfidentValue::deterministic(Value::List(vec![])))
            }

            // ── Composition ───────────────────────────────────────────────

            Expr::TryOr(primary, fallback) => {
                match self.eval_expr(primary, env).await {
                    Ok(val) => Ok(val),
                    Err(_) => self.eval_expr(fallback, env).await,
                }
            }

            Expr::Compose(parts) => {
                if parts.is_empty() {
                    return Ok(ConfidentValue::deterministic(Value::Unit));
                }
                let mut current = self.eval_expr(&parts[0], env).await?;
                for part in &parts[1..] {
                    env.push_scope();
                    env.bind("_pipe", current);
                    current = self.eval_expr(part, env).await?;
                    env.pop_scope();
                }
                Ok(current)
            }

            Expr::FanOut(parts) => {
                let mut results = Vec::new();
                for part in parts {
                    results.push(self.eval_expr(part, env).await?);
                }
                let min_conf = results.iter()
                    .map(|r| r.confidence)
                    .fold(1.0_f32, f32::min);
                Ok(ConfidentValue::derived(Value::Array(results), min_conf))
            }
        }
        })
    }

    // ── Task/Pure call helpers ─────────────────────────────────────────────────

    async fn call_task(
        &self,
        decl: &TaskDecl,
        args: Vec<ConfidentValue>,
    ) -> Result<ConfidentValue, RuntimeError> {
        let mut env = Env::new();
        for (i, param) in decl.needs.iter().enumerate() {
            let val = args.get(i).cloned()
                .unwrap_or(ConfidentValue::deterministic(Value::Unit));
            env.bind(&param.node.name, val);
        }

        match &decl.body.node {
            TaskBody::Do(stmts) => self.exec_stmts(stmts, &mut env).await,
            TaskBody::Is(expr) => self.eval_expr(expr, &mut env).await,
        }
    }

    async fn call_pure(
        &self,
        decl: &PureDecl,
        args: Vec<ConfidentValue>,
    ) -> Result<ConfidentValue, RuntimeError> {
        let mut env = Env::new();
        for (i, param) in decl.needs.iter().enumerate() {
            let val = args.get(i).cloned()
                .unwrap_or(ConfidentValue::deterministic(Value::Unit));
            env.bind(&param.node.name, val);
        }
        let result = self.exec_stmts(&decl.body, &mut env).await?;
        // Pure functions always return deterministic confidence
        Ok(ConfidentValue::deterministic(result.value))
    }

    // ── Flow execution ────────────────────────────────────────────────────────

    async fn call_flow(
        &self,
        decl: &FlowDecl,
        args: Vec<ConfidentValue>,
    ) -> Result<ConfidentValue, RuntimeError> {
        use crate::planner::FlowPlanner;

        let graph = FlowPlanner::dependency_graph(decl)
            .map_err(|e| RuntimeError::FlowError(e.to_string()))?;
        let waves = FlowPlanner::execution_waves(&graph)
            .map_err(|e| RuntimeError::FlowError(e.to_string()))?;

        // Build stage lookup
        let stage_map: HashMap<String, StageDecl> = decl.stages.iter()
            .map(|s| (s.node.name.node.clone(), s.node.clone()))
            .collect();

        // Bind flow parameters
        let mut flow_args: HashMap<String, ConfidentValue> = HashMap::new();
        for (i, param) in decl.needs.iter().enumerate() {
            let val = args.get(i).cloned()
                .unwrap_or(ConfidentValue::deterministic(Value::Unit));
            flow_args.insert(param.node.name.clone(), val);
        }

        let mut stage_outputs: HashMap<String, HashMap<String, ConfidentValue>> = HashMap::new();
        let mut last_result = ConfidentValue::deterministic(Value::Unit);

        if let Some(ref tracer) = self.tracer {
            tracer.flow_start(&decl.name.node, waves.len());
        }

        for (wave_idx, wave) in waves.iter().enumerate() {
            if let Some(ref tracer) = self.tracer {
                tracer.wave_start(wave_idx, wave);
            }

            if wave.len() == 1 {
                // Single stage — no spawn overhead
                let stage_name = &wave[0];
                let stage_decl = &stage_map[stage_name];
                let (bindings, give_val) = self.execute_stage(
                    stage_name, stage_decl, &flow_args, &stage_outputs,
                ).await?;
                if let Some(gv) = give_val {
                    last_result = gv;
                }
                stage_outputs.insert(stage_name.clone(), bindings);
            } else {
                // Multiple stages — run concurrently
                let mut handles = Vec::new();
                for stage_name in wave {
                    let stage_decl = stage_map[stage_name].clone();
                    let flow_args_clone = flow_args.clone();
                    let stage_outputs_clone = stage_outputs.clone();
                    let executor_clone = self.clone();
                    let name = stage_name.clone();

                    handles.push(tokio::spawn(async move {
                        let result = executor_clone.execute_stage(
                            &name, &stage_decl, &flow_args_clone, &stage_outputs_clone,
                        ).await;
                        (name, result)
                    }));
                }

                for handle in handles {
                    let (name, result) = handle.await
                        .map_err(|e| RuntimeError::FlowError(format!("stage join error: {}", e)))?;
                    let (bindings, give_val) = result?;
                    if let Some(gv) = give_val {
                        last_result = gv;
                    }
                    stage_outputs.insert(name, bindings);
                }
            }

            if let Some(ref tracer) = self.tracer {
                tracer.wave_complete(wave_idx);
            }
        }

        if let Some(ref tracer) = self.tracer {
            tracer.flow_complete(&decl.name.node);
        }

        Ok(last_result)
    }

    async fn execute_stage(
        &self,
        stage_name: &str,
        stage_decl: &StageDecl,
        flow_args: &HashMap<String, ConfidentValue>,
        stage_outputs: &HashMap<String, HashMap<String, ConfidentValue>>,
    ) -> Result<(HashMap<String, ConfidentValue>, Option<ConfidentValue>), RuntimeError> {
        if let Some(ref tracer) = self.tracer {
            tracer.stage_start(stage_name);
        }

        let mut env = Env::new();

        // Bind flow arguments
        for (name, val) in flow_args {
            env.bind(name, val.clone());
        }

        // Bind stage dependencies as Records
        let mut needed_stages: HashMap<String, Option<Vec<String>>> = HashMap::new();
        for needs_ref in &stage_decl.needs {
            let stage = &needs_ref.node.stage;
            match &needs_ref.node.field {
                NeedsRefField::Glob => {
                    needed_stages.insert(stage.clone(), None);
                }
                NeedsRefField::Named(field) => {
                    needed_stages.entry(stage.clone())
                        .and_modify(|v| {
                            if let Some(fields) = v {
                                fields.push(field.clone());
                            }
                        })
                        .or_insert(Some(vec![field.clone()]));
                }
            }
        }

        for (dep_stage, fields_opt) in &needed_stages {
            let dep_bindings = stage_outputs.get(dep_stage)
                .ok_or_else(|| RuntimeError::FlowError(
                    format!("stage '{}' output not available for stage '{}'", dep_stage, stage_name)
                ))?;

            let record: HashMap<String, ConfidentValue> = match fields_opt {
                None => dep_bindings.clone(),
                Some(fields) => {
                    fields.iter()
                        .filter_map(|f| dep_bindings.get(f).map(|v| (f.clone(), v.clone())))
                        .collect()
                }
            };

            env.bind(dep_stage, ConfidentValue::deterministic(
                Value::Record(record)
            ));
        }

        // Push scope so we can extract stage-produced bindings
        env.push_scope();

        // exec_stmts catches GiveSignal and returns Ok(val).
        // If no give, it returns Ok(Unit). Distinguish by checking the value.
        let result = self.exec_stmts(&stage_decl.body, &mut env).await?;
        let give_value = match &result.value {
            Value::Unit => None,
            _ => Some(result),
        };

        let stage_bindings = env.top_scope_bindings();

        if let Some(ref tracer) = self.tracer {
            tracer.stage_complete(stage_name, give_value.is_some());
        }

        Ok((stage_bindings, give_value))
    }
}

// ── Helper functions ──────────────────────────────────────────────────────────

fn truthy(cv: &ConfidentValue) -> bool {
    truthy_val(&cv.value)
}

fn truthy_val(val: &Value) -> bool {
    match val {
        Value::Bool(b) => *b,
        Value::Text(s) => !s.is_empty(),
        Value::Number(n) => *n != 0.0,
        Value::Unit => false,
        Value::List(v) | Value::Array(v) => !v.is_empty(),
        Value::Record(m) => !m.is_empty(),
    }
}

fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Text(a), Value::Text(b)) => a == b,
        (Value::Number(a), Value::Number(b)) => (a - b).abs() < f64::EPSILON,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Unit, Value::Unit) => true,
        _ => false,
    }
}

fn eval_binop(left: &Value, op: &BinOp, right: &Value) -> Result<Value, RuntimeError> {
    match (left, op, right) {
        // Arithmetic
        (Value::Number(a), BinOp::Add, Value::Number(b)) => Ok(Value::Number(a + b)),
        (Value::Number(a), BinOp::Sub, Value::Number(b)) => Ok(Value::Number(a - b)),
        (Value::Number(a), BinOp::Mul, Value::Number(b)) => Ok(Value::Number(a * b)),
        (Value::Number(a), BinOp::Div, Value::Number(b)) => {
            if *b == 0.0 { return Err(RuntimeError::DivisionByZero); }
            Ok(Value::Number(a / b))
        }
        // String concatenation
        (Value::Text(a), BinOp::Add, Value::Text(b)) => {
            Ok(Value::Text(format!("{}{}", a, b)))
        }
        // Numeric comparison
        (Value::Number(a), BinOp::Lt, Value::Number(b)) => Ok(Value::Bool(a < b)),
        (Value::Number(a), BinOp::Gt, Value::Number(b)) => Ok(Value::Bool(a > b)),
        (Value::Number(a), BinOp::Le, Value::Number(b)) => Ok(Value::Bool(a <= b)),
        (Value::Number(a), BinOp::Ge, Value::Number(b)) => Ok(Value::Bool(a >= b)),
        // Equality — works on all types
        (_, BinOp::Eq, _) => Ok(Value::Bool(values_equal(left, right))),
        (_, BinOp::Ne, _) => Ok(Value::Bool(!values_equal(left, right))),
        // Boolean logic
        (Value::Bool(a), BinOp::And, Value::Bool(b)) => Ok(Value::Bool(*a && *b)),
        (Value::Bool(a), BinOp::Or, Value::Bool(b)) => Ok(Value::Bool(*a || *b)),
        (_, BinOp::And, _) => Ok(Value::Bool(truthy_val(left) && truthy_val(right))),
        (_, BinOp::Or, _) => Ok(Value::Bool(truthy_val(left) || truthy_val(right))),
        _ => Err(RuntimeError::TypeError {
            expected: format!("compatible types for {:?}", op),
            got: format!("{} and {}", left, right),
        }),
    }
}

fn match_pattern(
    pattern: &Spanned<Pattern>,
    subject: &ConfidentValue,
) -> Option<Vec<(String, ConfidentValue)>> {
    match &pattern.node {
        Pattern::Wildcard => Some(vec![]),
        Pattern::Binding(name) => Some(vec![(name.clone(), subject.clone())]),
        Pattern::Constructor(name, _sub_pats) => {
            match &subject.value {
                Value::Text(s) if s == name => Some(vec![]),
                _ => None,
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::providers::mock::MockProvider;
    use crate::llm::registry::ProviderRegistry;
    use crate::parser;
    use crate::types::ConfidenceSource;

    async fn run_forge(source: &str) -> (Result<ConfidentValue, RuntimeError>, Vec<String>) {
        run_forge_with_mock(source, MockProvider::new("mock").with_default("mock response")).await
    }

    async fn run_forge_with_mock(
        source: &str,
        mock: MockProvider,
    ) -> (Result<ConfidentValue, RuntimeError>, Vec<String>) {
        let program = parser::parse(source).expect("parse failed");
        let mut registry = ProviderRegistry::new("mock");
        registry.register("mock", Arc::new(mock));
        let executor = TaskExecutor::new(program, Arc::new(registry), None);
        let result = executor.run().await;
        let outputs = executor.outputs();
        (result, outputs)
    }

    #[tokio::test]
    async fn test_say_template() {
        let (result, outputs) = run_forge(r#"
fn main
  say "hello world"
"#).await;
        assert!(result.is_ok());
        assert_eq!(outputs, vec!["hello world"]);
    }

    #[tokio::test]
    async fn test_bind_and_say() {
        let (result, outputs) = run_forge(r#"
fn main
  name = "FORGE"
  say "Hello, {name}!"
"#).await;
        assert!(result.is_ok());
        assert_eq!(outputs, vec!["Hello, FORGE!"]);
    }

    #[tokio::test]
    async fn test_template_escapes_render_as_control_characters() {
        let (result, outputs) = run_forge(r#"
fn main
  language = "rust"
  say "Line 1\nLanguage: {language}\tend\\"
"#).await;
        assert!(result.is_ok());
        assert_eq!(outputs, vec!["Line 1\nLanguage: rust\tend\\"]);
    }

    #[tokio::test]
    async fn test_give_returns_value() {
        let (result, _) = run_forge("fn main\n  give 42\n").await;
        let val = result.unwrap();
        assert!(matches!(val.value, Value::Number(n) if (n - 42.0).abs() < f64::EPSILON));
    }

    #[tokio::test]
    async fn test_binop_arithmetic() {
        let (result, _) = run_forge(r#"
fn main
  x = 10 + 5
  y = x * 2
  give y
"#).await;
        let val = result.unwrap();
        assert!(matches!(val.value, Value::Number(n) if (n - 30.0).abs() < f64::EPSILON));
    }

    #[tokio::test]
    async fn test_binop_comparison() {
        let (result, _) = run_forge("fn main\n  give 5 > 3\n").await;
        assert!(matches!(result.unwrap().value, Value::Bool(true)));
    }

    #[tokio::test]
    async fn test_unary_neg() {
        let (result, _) = run_forge("fn main\n  give -42\n").await;
        let val = result.unwrap();
        assert!(matches!(val.value, Value::Number(n) if (n - -42.0).abs() < f64::EPSILON));
    }

    #[tokio::test]
    async fn test_unary_not() {
        let (result, _) = run_forge("fn main\n  give not true\n").await;
        assert!(matches!(result.unwrap().value, Value::Bool(false)));
    }

    #[tokio::test]
    async fn test_undefined_variable_error() {
        let (result, _) = run_forge("fn main\n  say foo\n").await;
        assert!(matches!(result, Err(RuntimeError::UndefinedVariable { .. })));
    }

    #[tokio::test]
    async fn test_no_main_error() {
        let (result, _) = run_forge(r#"
task greet
  needs name: Text
  gives Text
  do
    say "hello"
"#).await;
        assert!(matches!(result, Err(RuntimeError::NoMainFunction)));
    }

    #[tokio::test]
    async fn test_array_literal() {
        let (result, _) = run_forge("fn main\n  give [1, 2, 3]\n").await;
        assert!(matches!(result.unwrap().value, Value::Array(ref v) if v.len() == 3));
    }

    #[tokio::test]
    async fn test_if_else_true_branch() {
        // fn main: i1=2sp for stmts, if body at i3=6sp, else at i2=4sp
        let src = "fn main\n  x = 10\n  if x > 5\n      say \"big\"\n    else\n      say \"small\"\n";
        let (_, outputs) = run_forge(src).await;
        assert_eq!(outputs, vec!["big"]);
    }

    #[tokio::test]
    async fn test_if_else_false_branch() {
        let src = "fn main\n  x = 2\n  if x > 5\n      say \"big\"\n    else\n      say \"small\"\n";
        let (_, outputs) = run_forge(src).await;
        assert_eq!(outputs, vec!["small"]);
    }

    #[tokio::test]
    async fn test_for_loop() {
        // for body at i3=6sp
        let src = "fn main\n  items = [1, 2, 3]\n  for item in items\n      say \"{item}\"\n";
        let (_, outputs) = run_forge(src).await;
        assert_eq!(outputs, vec!["1", "2", "3"]);
    }

    #[tokio::test]
    async fn test_match_wildcard() {
        // match arms at i3=6sp
        let src = "fn main\n  x = \"hello\"\n  match x\n      _ -> say \"matched\"\n";
        let (_, outputs) = run_forge(src).await;
        assert_eq!(outputs, vec!["matched"]);
    }

    #[tokio::test]
    async fn test_match_binding() {
        let src = "fn main\n  x = \"hello\"\n  match x\n      val -> say val\n";
        let (_, outputs) = run_forge(src).await;
        assert_eq!(outputs, vec!["hello"]);
    }

    #[tokio::test]
    async fn test_task_call() {
        let (_, outputs) = run_forge(r#"
task greet
  needs name: Text
  gives Text
  do
    give "Hello, {name}!"

fn main
  result = greet("World")
  say result
"#).await;
        assert_eq!(outputs, vec!["Hello, World!"]);
    }

    #[tokio::test]
    async fn test_pure_call_deterministic() {
        let src = "pure add\n  needs a: Number, b: Number\n  gives Number\n  do\n    give a + b\n\nfn main\n  give add(3, 4)\n";
        let (result, _) = run_forge(src).await;
        let val = result.unwrap();
        assert!(matches!(val.value, Value::Number(n) if (n - 7.0).abs() < f64::EPSILON));
        assert_eq!(val.confidence, 1.0);
        assert!(matches!(val.source, ConfidenceSource::Deterministic));
    }

    #[tokio::test]
    async fn test_reason_mock() {
        let mock = MockProvider::new("mock").with_default("The answer is 42.");
        let (result, _) = run_forge_with_mock(r#"
fn main
  give reason "What is the meaning of life?"
"#, mock).await;
        let val = result.unwrap();
        assert!(matches!(val.value, Value::Text(ref s) if s == "The answer is 42."));
        assert!(val.confidence > 0.0);
        assert!(matches!(val.source, ConfidenceSource::LLMDirect(_)));
    }

    #[tokio::test]
    async fn test_when_sure_branch() {
        let mock = MockProvider::new("mock").with_default("support");
        let (_, outputs) = run_forge_with_mock(r#"
use
  llm.classify

task classify_intent
  needs message: Text
  gives Text
  do
    result = classify message into ["buy", "support", "cancel"]
    when result.sure -> give result
    when result.unsure -> give "unclear"
    else -> give "unknown"

fn main
  out = classify_intent("help me")
  say out
"#, mock).await;
        assert_eq!(outputs, vec!["support"]);
    }

    #[tokio::test]
    async fn test_when_else_branch() {
        let mock = MockProvider::new("mock")
            .with_default("I'm not sure, I think it might be possibly something, I don't know, it depends, unclear");
        let (_, outputs) = run_forge_with_mock(r#"
use
  llm.classify

task classify_intent
  needs message: Text
  gives Text
  do
    result = classify message into ["buy", "support", "cancel"]
    when result.sure -> give result
    else -> give "unknown"

fn main
  out = classify_intent("help")
  say out
"#, mock).await;
        assert_eq!(outputs, vec!["unknown"]);
    }

    #[tokio::test]
    async fn test_string_equality() {
        let src = "fn main\n  x = \"hello\"\n  if x == \"hello\"\n      say \"yes\"\n    else\n      say \"no\"\n";
        let (_, outputs) = run_forge(src).await;
        assert_eq!(outputs, vec!["yes"]);
    }

    #[tokio::test]
    async fn test_boolean_logic() {
        let (result, _) = run_forge("fn main\n  give true and false\n").await;
        assert!(matches!(result.unwrap().value, Value::Bool(false)));
    }

    #[tokio::test]
    async fn test_division_by_zero() {
        let (result, _) = run_forge("fn main\n  give 1 / 0\n").await;
        assert!(matches!(result, Err(RuntimeError::DivisionByZero)));
    }

    #[tokio::test]
    async fn test_text_concat() {
        let (result, _) = run_forge(r#"fn main
  give "hello" + " " + "world"
"#).await;
        assert!(matches!(result.unwrap().value, Value::Text(ref s) if s == "hello world"));
    }

    #[tokio::test]
    async fn test_try_or() {
        let (result, _) = run_forge("fn main\n  give try undefined_var or 42\n").await;
        let val = result.unwrap();
        assert!(matches!(val.value, Value::Number(n) if (n - 42.0).abs() < f64::EPSILON));
    }

    // ── Flow tests ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_flow_single_stage() {
        let (result, outputs) = run_forge(r#"
flow greetflow
  needs name: Text
  gives Text

  stage greet
    give "Hello, {name}!"

fn main
  result = greetflow("World")
  say result
"#).await;
        assert!(result.is_ok());
        assert_eq!(outputs, vec!["Hello, World!"]);
    }

    #[tokio::test]
    async fn test_flow_multi_wave_with_deps() {
        let (result, outputs) = run_forge(r#"
flow pipeline
  needs input: Text
  gives Text

  stage first
    msg = "{input} processed"

  stage second
    needs first.*
    give "{first.msg} and refined"

fn main
  result = pipeline("data")
  say result
"#).await;
        assert!(result.is_ok());
        assert_eq!(outputs, vec!["data processed and refined"]);
    }

    #[tokio::test]
    async fn test_flow_parallel_independent_stages() {
        let (result, _) = run_forge(r#"
flow parallel
  needs x: Text
  gives Text

  stage a
    val = "A:{x}"

  stage b
    val = "B:{x}"

  stage combine
    needs a.val, b.val
    give "{a.val}+{b.val}"

fn main
  give parallel("test")
"#).await;
        let val = result.unwrap();
        assert!(matches!(val.value, Value::Text(ref s) if s == "A:test+B:test"));
    }

    #[tokio::test]
    async fn test_flow_with_llm_reason() {
        let mock = MockProvider::new("mock")
            .with_response("synthesize", "synthesis result")
            .with_default("other response");
        let (result, outputs) = run_forge_with_mock(r#"
flow analyze
  needs topic: Text
  gives Text

  stage gather
    info = "facts about {topic}"

  stage synthesize
    needs gather.*
    give reason "synthesize {gather.info}"

fn main
  result = analyze("AI")
  say result
"#, mock).await;
        assert!(result.is_ok());
        assert_eq!(outputs, vec!["synthesis result"]);
    }

    #[tokio::test]
    async fn test_flow_cycle_detection() {
        let (result, _) = run_forge(r#"
flow cyclic
  needs x: Text
  gives Text

  stage a
    needs b.*
    val = "a"

  stage b
    needs a.*
    val = "b"

fn main
  give cyclic("test")
"#).await;
        assert!(matches!(result, Err(RuntimeError::FlowError(ref msg)) if msg.contains("cycle")));
    }
}
