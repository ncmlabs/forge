// FORGE AST node definitions (v3)
// All types derive Debug, Clone. Every node carries Span info via Spanned<T>.

// ── Span ──────────────────────────────────────────────────────

/// Byte-offset span in source text, for error reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

/// A node paired with its source location.
#[derive(Debug, Clone)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
}

impl<T> Spanned<T> {
    pub fn new(node: T, span: Span) -> Self {
        Self { node, span }
    }
}

// ── Program ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Program {
    pub boundary: Option<Spanned<BoundaryDirective>>,
    pub items: Vec<Spanned<TopLevel>>,
}

// ── Top-level declarations ────────────────────────────────────

#[derive(Debug, Clone)]
pub enum TopLevel {
    Use(UseDecl),
    Task(TaskDecl),
    Pure(PureDecl),
    Flow(FlowDecl),
    Agent(Box<AgentDecl>),
    Pool(PoolDecl),
    Warden(WardenDecl),
    Contract(ContractDecl),
    System(SystemDecl),
    Event(EventDecl),
    States(StatesDecl),
    Endpoint(EndpointDecl),
    TypeDef(TypeDefDecl),
    FnMain(FnMainDecl),
    Import(ImportDecl),
}

// ── Boundary directive ────────────────────────────────────────

/// File-level boundary: `#! boundary: server|client|shared`
#[derive(Debug, Clone)]
pub struct BoundaryDirective {
    pub kind: Spanned<BoundaryKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryKind {
    Server,
    Client,
    Shared,
}

// ── use ───────────────────────────────────────────────────────

/// `use` declaration: a list of dot-separated capability paths.
#[derive(Debug, Clone)]
pub struct UseDecl {
    pub capabilities: Vec<Spanned<String>>,
}

// ── task ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TaskDecl {
    pub name: Spanned<String>,
    pub needs: Vec<Spanned<Param>>,
    pub gives: Option<Spanned<OutputType>>,
    pub body: Spanned<TaskBody>,
    pub if_fails: Option<Vec<Spanned<Stmt>>>,
}

#[derive(Debug, Clone)]
pub enum TaskBody {
    Do(Vec<Spanned<Stmt>>),
    Is(Box<Spanned<Expr>>),
}

// ── pure ──────────────────────────────────────────────────────

/// Deterministic function — no LLM, no side effects, compile-enforced.
#[derive(Debug, Clone)]
pub struct PureDecl {
    pub name: Spanned<String>,
    pub needs: Vec<Spanned<Param>>,
    pub gives: Option<Spanned<OutputType>>,
    pub body: Vec<Spanned<Stmt>>,
}

// ── event ─────────────────────────────────────────────────────

/// Typed broadcast stream declaration.
#[derive(Debug, Clone)]
pub struct EventDecl {
    pub name: Spanned<String>,
    pub fields: Vec<Spanned<FieldDef>>,
}

/// Named typed field — used in events, type defs, and memory blocks.
#[derive(Debug, Clone)]
pub struct FieldDef {
    pub name: String,
    pub type_name: Spanned<TypeName>,
}

// ── states ────────────────────────────────────────────────────

/// Lifecycle state machine declaration.
#[derive(Debug, Clone)]
pub struct StatesDecl {
    pub name: Spanned<String>,
    pub transitions: Vec<Spanned<StateTransition>>,
}

#[derive(Debug, Clone)]
pub struct StateTransition {
    pub from: Spanned<String>,
    pub to: Spanned<String>,
    pub condition: Option<Spanned<Expr>>,
}

// ── type ──────────────────────────────────────────────────────

/// Record type definition (for shared boundary types).
#[derive(Debug, Clone)]
pub struct TypeDefDecl {
    pub name: Spanned<String>,
    pub fields: Vec<Spanned<FieldDef>>,
}

// ── endpoint ──────────────────────────────────────────────────

/// Server entry point declaration.
#[derive(Debug, Clone)]
pub struct EndpointDecl {
    pub name: Spanned<String>,
    pub params: Vec<Spanned<Param>>,
    pub return_type: Option<Spanned<OutputType>>,
    pub body: Vec<Spanned<Stmt>>,
}

// ── flow ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct FlowDecl {
    pub name: Spanned<String>,
    pub needs: Vec<Spanned<Param>>,
    pub gives: Option<Spanned<OutputType>>,
    pub stages: Vec<Spanned<StageDecl>>,
}

#[derive(Debug, Clone)]
pub struct StageDecl {
    pub name: Spanned<String>,
    pub needs: Vec<Spanned<NeedsRef>>,
    pub body: Vec<Spanned<Stmt>>,
}

/// Reference to a previous stage's output: `stage.field` or `stage.*`.
#[derive(Debug, Clone)]
pub struct NeedsRef {
    pub stage: String,
    pub field: NeedsRefField,
}

#[derive(Debug, Clone)]
pub enum NeedsRefField {
    Named(String),
    Glob,
}

// ── agent ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AgentDecl {
    pub exportable: bool,
    pub name: Spanned<String>,
    pub lifecycle: Option<Spanned<String>>,
    pub memory: Vec<Spanned<FieldDef>>,
    pub memory_persistent: bool,
    pub knowledge: Option<Spanned<KnowledgeDecl>>,
    /// Per-agent skill allow-list (issue #363 / T9.2). Empty = unrestricted.
    /// Patterns are dotted skill paths, optionally suffixed with `.*` for
    /// single-segment glob (e.g. `skill.github.*` matches `skill.github.create_pr`
    /// but not `skill.github.foo.bar`). The allows_checker pass enforces this
    /// against `skill.X.Y(...)` call sites in handler bodies.
    pub allows: Vec<Spanned<String>>,
    pub timers: Vec<Spanned<TimerField>>,
    pub schedules: Vec<Spanned<ScheduleField>>,
    pub correlates: Vec<Spanned<CorrelateField>>,
    pub webhooks: Vec<Spanned<WebhookField>>,
    pub subscriptions: Vec<Spanned<SubscribeDecl>>,
    pub warden_override: Vec<Spanned<WardPolicy>>,
    pub handlers: Vec<Spanned<OnHandler>>,
    pub stuck_policy: Option<Spanned<StuckPolicy>>,
}

// ── knowledge ────────────────────────────────────────────────

/// Persistent searchable knowledge store declaration inside an agent.
#[derive(Debug, Clone)]
pub struct KnowledgeDecl {
    pub store_path: Spanned<Expr>,
    /// Optional repo / project scope (issue #359 / T8.4). When present, the
    /// runtime resolves persistence to `{store_path}/{project_id}/knowledge.json`
    /// and filters recall to that project. T8.5 will thread the per-repo
    /// `RepoConfig.slug` from `clone-dev.toml` into this slot.
    pub project_id: Option<Spanned<Expr>>,
    pub max_entries: Option<Spanned<f64>>,
    pub retention: Option<Spanned<Duration>>,
    pub imports: Vec<Spanned<String>>,
}

/// Import declaration: `import knowledge from "pkg.forgepkg.json" as name`
#[derive(Debug, Clone)]
pub struct ImportDecl {
    pub layers: Vec<Spanned<ImportLayer>>,
    pub source: Spanned<Expr>,
    pub alias: Spanned<String>,
}

#[derive(Debug, Clone)]
pub enum ImportLayer {
    Knowledge,
    Memory,
    Config,
}

/// Source for a `learn` statement.
#[derive(Debug, Clone)]
pub enum LearnSource {
    /// `learn "fact"`
    Direct(Spanned<Expr>),
    /// `learn from interaction(question, answer, confidence)`
    FromInteraction(Vec<Spanned<CallArg>>),
    /// `learn from document("path")`
    FromDocument(Spanned<Expr>),
}

/// Timer declaration inside an agent.
#[derive(Debug, Clone)]
pub struct TimerField {
    pub name: Spanned<String>,
    pub duration: Spanned<Duration>,
}

/// Schedule declaration inside an agent — durable, cross-session wall-clock trigger.
/// All option fields are `Option` so the checker can report missing required options
/// with span-annotated errors (the parser is deliberately liberal).
#[derive(Debug, Clone)]
pub struct ScheduleField {
    pub name: Spanned<String>,
    pub when: Option<Spanned<WhenExpr>>,
    pub mode: Option<Spanned<ScheduleMode>>,
    pub prompt: Option<Spanned<Expr>>,
    pub emit: Option<Spanned<String>>,
    pub precision: Option<Spanned<Precision>>,
    /// Option names that were specified more than once inside this block.
    /// Populated by the parser; the checker consumes to emit duplicate-option errors.
    pub duplicates: Vec<Spanned<String>>,
}

#[derive(Debug, Clone)]
pub enum WhenExpr {
    DailyAt(TimeOfDay),
    Every(Duration),
    Cron(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeOfDay {
    pub hour: u8,
    pub minute: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleMode {
    Spawn,
    Wake,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precision {
    High,
}

/// Correlate declaration inside an agent — event-to-session routing via a
/// persisted correlation key. Peer to `ScheduleField`; parser stays liberal
/// so the checker owns semantic errors.
#[derive(Debug, Clone)]
pub struct CorrelateField {
    pub event_type: Spanned<String>,
    pub field_name: Spanned<String>,
    pub mode: Option<Spanned<ScheduleMode>>,
    pub emit: Option<Spanned<String>>,
    pub duplicates: Vec<Spanned<String>>,
}

/// Webhook trigger declaration — HMAC-verified inbound HTTP wake source
/// (issue #335). Peer to `CorrelateField`; parser stays liberal so the
/// checker owns semantic errors. Runtime wiring in `WebhookDriver`.
#[derive(Debug, Clone)]
pub struct WebhookField {
    pub name: Spanned<String>,
    pub mode: Option<Spanned<ScheduleMode>>,
    pub emit: Option<Spanned<String>>,
    pub duplicates: Vec<Spanned<String>>,
}

/// Event subscription inside an agent.
#[derive(Debug, Clone)]
pub struct SubscribeDecl {
    pub event_name: Spanned<String>,
    pub filter: Option<Spanned<Expr>>,
}

#[derive(Debug, Clone)]
pub struct OnHandler {
    pub event: Spanned<String>,
    pub params: Vec<Spanned<Param>>,
    pub payload_type: Option<Spanned<TypeName>>,
    pub requires: Vec<Spanned<RequiresClause>>,
    pub body: Vec<Spanned<Stmt>>,
}

/// Precondition guard on a handler.
#[derive(Debug, Clone)]
pub struct RequiresClause {
    pub condition: Spanned<Expr>,
    pub on_fail: Option<Spanned<FailPolicy>>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum FailPolicy {
    Silent,
    Give(Spanned<Expr>),
    Log,
    Escalate,
    Crash,
}

#[derive(Debug, Clone)]
pub struct StuckPolicy {
    pub turns: Option<u64>,
    pub body: Vec<Spanned<Stmt>>,
}

// ── pool ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PoolDecl {
    pub name: Spanned<String>,
    pub worker_type: Spanned<String>,
    pub worker_count: Spanned<f64>,
    pub strategy: Spanned<PoolStrategy>,
    pub timeout: Option<Spanned<Duration>>,
    pub fallback: Option<Spanned<String>>,
}

#[derive(Debug, Clone)]
pub enum PoolStrategy {
    Fastest,
    All,
    Majority,
    Quorum(f64),
    First(f64),
}

#[derive(Debug, Clone)]
pub struct Duration {
    pub value: u64,
    pub unit: DurationUnit,
}

impl Duration {
    pub fn to_std(&self) -> std::time::Duration {
        match self.unit {
            DurationUnit::Seconds => std::time::Duration::from_secs(self.value),
            DurationUnit::Minutes => std::time::Duration::from_secs(self.value * 60),
            DurationUnit::Hours => std::time::Duration::from_secs(self.value * 3600),
            DurationUnit::Days => std::time::Duration::from_secs(self.value * 86400),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum DurationUnit {
    Seconds,
    Minutes,
    Hours,
    Days,
}

// ── warden ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct WardenDecl {
    pub name: Spanned<String>,
    pub manages: Vec<Spanned<String>>,
    pub policies: Vec<Spanned<WardPolicy>>,
    pub max_retries: Option<Spanned<MaxRetries>>,
}

#[derive(Debug, Clone)]
pub struct WardPolicy {
    pub failure_type: Spanned<FailureType>,
    pub response: Spanned<WardResponse>,
    pub scope: Spanned<WardScope>,
    pub after_clauses: Vec<Spanned<AfterClause>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailureType {
    Stuck,
    Crash,
    Hallucination,
    Contradiction,
    Budget,
    Timeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WardResponse {
    Nudge,
    Downgrade,
    Restart,
    Replace,
    Escalate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WardScope {
    This,
    Downstream,
    All,
}

#[derive(Debug, Clone)]
pub struct AfterClause {
    pub count: u64,
    pub response: Spanned<WardResponse>,
}

#[derive(Debug, Clone)]
pub struct MaxRetries {
    pub count: u64,
    pub window: Spanned<Duration>,
}

// ── contract ──────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ContractDecl {
    pub name: Spanned<String>,
    pub methods: Vec<Spanned<CanSignature>>,
}

#[derive(Debug, Clone)]
pub struct CanSignature {
    pub name: String,
    pub params: Vec<Spanned<Param>>,
    pub return_type: Spanned<TypeName>,
}

// ── system ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SystemDecl {
    pub name: Spanned<String>,
    pub bindings: Vec<Spanned<SystemBinding>>,
    pub wiring: Vec<Spanned<Expr>>,
}

#[derive(Debug, Clone)]
pub struct SystemBinding {
    pub alias: String,
    pub target: String,
}

// ── fn main ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct FnMainDecl {
    pub body: Vec<Spanned<Stmt>>,
}

// ── Shared types ──────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub type_name: Spanned<TypeName>,
}

/// Output type: `Text or Failure` becomes multiple entries.
#[derive(Debug, Clone)]
pub struct OutputType {
    pub types: Vec<Spanned<TypeName>>,
}

#[derive(Debug, Clone)]
pub enum TypeName {
    Text,
    Number,
    Bool,
    Results,
    Report,
    Intent,
    Summary,
    Failure,
    Classification,
    Conversation,
    Profile,
    SearchResults,
    Request,
    Response,
    Headers,
    Html,
    AgentResult,
    Custom(String),
    /// Array type: `Text[9]` = Array(Text, Some(9)), `Player[]` = Array(Custom("Player"), None)
    Array(Box<TypeName>, Option<usize>),
}

// ── Expressions ───────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Expr {
    /// Number literal: `42`, `0.95`
    NumberLit(f64),
    /// Boolean literal: `true`, `false`
    BoolLit(bool),
    /// Identifier: `x`, `result`, `greet`
    Ident(String),
    /// Template string: `"hello {name}"`
    Template(Vec<Spanned<TemplatePart>>),
    /// Function/task call: `classify_intent(message)`
    Call(CallExpr),
    /// Constructor: `Failure("msg", retry: true)`
    Constructor(ConstructorExpr),
    /// `reason "prompt"` or `reason "prompt" for <phase>` (#361)
    Reason(ReasonExpr),
    /// `classify expr into ["a", "b"]` or `... for <phase>` (#361)
    Classify(ClassifyExpr),
    /// `search "query"`
    Search(Box<Spanned<Expr>>),
    /// `recall "query"` — retrieve from agent knowledge store
    Recall(Box<Spanned<Expr>>),
    /// `exec "command"` — direct CLI execution, returns uncertain<Text>
    Exec(Box<Spanned<Expr>>),
    /// `command "cmd" [in "dir"] [timeout 60s] [background true]`
    Command(CommandExpr),
    /// `command.status(handle)` / `command.output(handle)` / `command.cancel(handle)`
    CommandMethod(Spanned<String>, Vec<Spanned<CallArg>>),
    /// `session.status(id)` — imperative session method call (issue #192)
    SessionMethod(Spanned<String>, Vec<Spanned<CallArg>>),
    /// `session "label" prompt prompt_text agent "claude" ...`
    Session(Box<SessionExpr>),
    /// `find "alias"` / `find all template [where lifecycle == state]`
    Find(Box<FindExpr>),
    /// `try expr or expr`
    TryOr(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
    /// `A >> B >> C` — composition chain
    Compose(Vec<Spanned<Expr>>),
    /// `(A | B | C)` — fan-out parallelism
    FanOut(Vec<Spanned<Expr>>),
    /// `expr.field` — field access
    FieldAccess(Box<Spanned<Expr>>, Spanned<String>),
    /// `expr.*` — glob access
    GlobAccess(Box<Spanned<Expr>>),
    /// `Intent.unknown` — type dot access
    TypeAccess(Spanned<TypeName>, Spanned<String>),
    /// Parenthesized expression
    Paren(Box<Spanned<Expr>>),
    /// Array literal: `[1, 2, 3]`
    ArrayLit(Vec<Spanned<Expr>>),
    /// Indexing: `board[cell]`
    Index(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
    /// Method call: `board.none(empty)`
    MethodCall(Box<Spanned<Expr>>, Spanned<String>, Vec<Spanned<CallArg>>),
    /// Binary operation: `x + y`, `x == y`, `x and y`
    BinOp(Box<Spanned<Expr>>, Spanned<BinOp>, Box<Spanned<Expr>>),
    /// Unary operation: `not x`, `-x`
    UnaryOp(Spanned<UnaryOp>, Box<Spanned<Expr>>),
}

#[derive(Debug, Clone)]
pub struct CommandExpr {
    pub cmd: Box<Spanned<Expr>>,
    pub working_dir: Option<Box<Spanned<Expr>>>,
    pub timeout: Option<Spanned<Duration>>,
    pub background: Option<Spanned<bool>>,
    pub env: Option<Vec<Spanned<EnvEntry>>>,
}

#[derive(Debug, Clone)]
pub struct SessionExpr {
    pub name: Box<Spanned<Expr>>,
    pub agent: Option<Box<Spanned<Expr>>>,
    pub prompt: Option<Box<Spanned<Expr>>>,
    pub tools: Option<Box<Spanned<Expr>>>,
    pub timeout: Option<Spanned<Duration>>,
    pub budget: Option<Box<Spanned<Expr>>>,
    pub gives: Option<Spanned<TypeName>>,
    pub on_progress: Option<Spanned<SessionHook>>,
    pub on_complete: Option<Spanned<SessionHook>>,
    pub isolate: Option<IsolateConfig>,
}

#[derive(Debug, Clone)]
pub struct SessionHook {
    pub event: Spanned<String>,
    pub args: Vec<Spanned<CallArg>>,
}

#[derive(Debug, Clone)]
pub struct EnvEntry {
    pub key: Spanned<String>,
    pub value: Spanned<Expr>,
}

#[derive(Debug, Clone)]
pub enum TemplatePart {
    Text(String),
    Interp(Box<Spanned<Expr>>),
    /// `{!expr}` — raw interpolation, skips HTML escaping in Html context.
    RawInterp(Box<Spanned<Expr>>),
}

#[derive(Debug, Clone)]
pub struct CallExpr {
    pub name: Spanned<String>,
    pub args: Vec<Spanned<CallArg>>,
}

#[derive(Debug, Clone)]
pub struct CallArg {
    pub label: Option<Spanned<String>>,
    pub value: Spanned<Expr>,
}

#[derive(Debug, Clone)]
pub struct ConstructorExpr {
    pub type_name: Spanned<TypeName>,
    pub args: Vec<Spanned<CallArg>>,
}

#[derive(Debug, Clone)]
pub struct ClassifyExpr {
    pub input: Box<Spanned<Expr>>,
    pub labels: Vec<Spanned<String>>,
    /// Optional routing phase (#361). When set, the executor consults the
    /// configured `[llm.routing]` table to dispatch to a specific provider
    /// chain instead of the runtime default.
    pub phase: Option<Spanned<String>>,
}

#[derive(Debug, Clone)]
pub struct ReasonExpr {
    pub prompt: Box<Spanned<Expr>>,
    /// Optional routing phase (#361). See `ClassifyExpr::phase`.
    pub phase: Option<Spanned<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Not,
    Neg,
}

// ── Statements ────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Stmt {
    /// `name = expr`
    Bind(Spanned<String>, Spanned<Expr>),
    /// `give expr` with optional `with key: value` metadata
    Give(Spanned<Expr>, Vec<Spanned<GiveMeta>>),
    /// `say expr`
    Say(Spanned<Expr>),
    /// `when`/`else` block (confidence-only branching)
    When(Box<WhenBlock>),
    /// `escalate to target`
    Escalate(Spanned<String>),
    /// `memory.field = expr` or `memory.field[idx] = expr`
    MemoryUpdate(Spanned<String>, Option<Spanned<Expr>>, Spanned<Expr>),
    /// Bare expression as statement
    ExprStmt(Spanned<Expr>),
    /// `emit EventName(args)`
    Emit(Spanned<String>, Vec<Spanned<CallArg>>),
    /// `transition to state_name`
    TransitionTo(Spanned<String>),
    /// `start timer_name [for context]`
    StartTimer {
        name: Spanned<String>,
        context: Option<Spanned<Expr>>,
    },
    /// `cancel timer_name [for context]`
    CancelTimer {
        name: Spanned<String>,
        context: Option<Spanned<Expr>>,
    },
    /// `reset timer_name`
    ResetTimer(Spanned<String>),
    /// `forward expr to expr`
    Forward(Spanned<Expr>, Spanned<Expr>),
    /// `learn "fact"` / `learn from interaction(...)` / `learn from document(...)`
    /// Optional second field: `category: "name"` expression.
    Learn(Spanned<LearnSource>, Option<Spanned<Expr>>),
    /// `match expr` with pattern arms
    Match(Box<MatchBlock>),
    /// `if`/`else if`/`else` block
    IfElse(Box<IfElseBlock>),
    /// `for binding in iterable`
    For(Box<ForLoop>),
    /// `[binding =] spawn template [as "alias"]` with optional knowledge/memory transfer
    Spawn(Box<SpawnStmt>),
    /// `retire ["alias"]` with optional knowledge export
    Retire(Box<RetireStmt>),
}

#[derive(Debug, Clone)]
pub struct WhenBlock {
    pub clauses: Vec<Spanned<WhenClause>>,
    pub else_body: Option<Spanned<ElseClause>>,
}

#[derive(Debug, Clone)]
pub struct WhenClause {
    pub predicate: Spanned<ConfidencePred>,
    pub body: Spanned<Stmt>,
}

#[derive(Debug, Clone)]
pub struct ElseClause {
    pub body: Spanned<Stmt>,
}

// ── Match (structural pattern matching) ───────────────────────

#[derive(Debug, Clone)]
pub struct MatchBlock {
    pub subject: Spanned<Expr>,
    pub arms: Vec<Spanned<MatchArm>>,
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: Spanned<Pattern>,
    pub body: Spanned<Stmt>,
}

#[derive(Debug, Clone)]
pub enum Pattern {
    Wildcard,
    Binding(String),
    Constructor(String, Vec<Spanned<Pattern>>),
}

// ── If/Else ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct IfElseBlock {
    pub condition: Spanned<Expr>,
    pub then_body: Vec<Spanned<Stmt>>,
    pub else_ifs: Vec<(Spanned<Expr>, Vec<Spanned<Stmt>>)>,
    pub else_body: Option<Vec<Spanned<Stmt>>>,
}

// ── For loop ──────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ForLoop {
    pub binding: Spanned<String>,
    pub iterable: Spanned<Expr>,
    pub body: Vec<Spanned<Stmt>>,
}

// ── Spawn (runtime agent creation, issue #83) ───────────────

/// `spawn agent_name as "alias"` with optional knowledge/memory transfer options.
#[derive(Debug, Clone)]
pub struct SpawnStmt {
    /// Optional variable binding for the spawned instance UUID.
    pub binding: Option<Spanned<String>>,
    /// Name of the agent declaration to use as template.
    pub template: Spanned<String>,
    /// Optional alias for the spawned instance (template string).
    pub alias: Option<Spanned<Expr>>,
    /// Spawn options: knowledge filter, confidence cap, memory init.
    pub options: Vec<Spanned<SpawnOption>>,
}

/// Options for the spawn statement.
#[derive(Debug, Clone)]
pub enum SpawnOption {
    /// `with knowledge where category == "X"`
    KnowledgeFilter(Spanned<String>),
    /// `with confidence_cap: 0.8`
    ConfidenceCap(Spanned<Expr>),
    /// `with memory field: value`
    MemoryInit(Spanned<String>, Spanned<Expr>),
    /// `isolate worktree "branch"`
    Isolate(IsolateConfig),
}

// ── Sandbox isolation (issue #194) ────────────────────────────

/// Isolation strategy for sandbox creation.
#[derive(Debug, Clone)]
pub enum IsolateStrategy {
    Worktree,
}

/// Configuration for sandbox isolation on spawn or session.
#[derive(Debug, Clone)]
pub struct IsolateConfig {
    pub strategy: Spanned<IsolateStrategy>,
    pub branch: Box<Spanned<Expr>>,
}

// ── Find expression (runtime instance discovery, issue #84) ─

/// `find "alias"` or `find all template [where lifecycle == state]`
#[derive(Debug, Clone)]
pub struct FindExpr {
    pub kind: FindKind,
}

/// The kind of find query.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum FindKind {
    /// `find "alias"` — single lookup by alias
    ByAlias(Spanned<Expr>),
    /// `find all template` — all instances of a template
    AllByTemplate(Spanned<String>),
    /// `find all template where lifecycle == state`
    AllByTemplateFiltered(Spanned<String>, Spanned<String>),
}

// ── Retire (graceful agent termination, issue #86) ──────────

/// `retire ["alias"]` with optional knowledge export.
#[derive(Debug, Clone)]
pub struct RetireStmt {
    /// Optional target alias — `None` means retire self.
    pub target: Option<Spanned<Expr>>,
    /// Optional knowledge export file path.
    pub knowledge_export: Option<Spanned<Expr>>,
}

// ── Give metadata ────────────────────────────────────────────

/// Key-value metadata in `give expr with key: value, ...`
#[derive(Debug, Clone)]
pub struct GiveMeta {
    pub key: Spanned<String>,
    pub value: Spanned<Expr>,
}

// ── Confidence predicates ─────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ConfidencePred {
    pub subject: Spanned<String>,
    pub level: Spanned<ConfLevel>,
}

#[derive(Debug, Clone)]
pub enum ConfLevel {
    /// `.sure` or `.sure(above: 0.9)`
    Sure(Option<f64>),
    /// `.unsure`
    Unsure,
    /// `.unreliable`
    Unreliable,
    /// `.conflicted`
    Conflicted,
}
