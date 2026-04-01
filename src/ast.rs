// FORGE AST node definitions (issue #3)
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
    pub items: Vec<Spanned<TopLevel>>,
}

// ── Top-level declarations ────────────────────────────────────

#[derive(Debug, Clone)]
pub enum TopLevel {
    Use(UseDecl),
    Task(TaskDecl),
    Flow(FlowDecl),
    Agent(AgentDecl),
    Pool(PoolDecl),
    Contract(ContractDecl),
    System(SystemDecl),
    FnMain(FnMainDecl),
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
    pub name: Spanned<String>,
    pub memory: Vec<Spanned<MemoryField>>,
    pub handlers: Vec<Spanned<OnHandler>>,
    pub stuck_policy: Option<Spanned<StuckPolicy>>,
}

#[derive(Debug, Clone)]
pub struct MemoryField {
    pub name: String,
    pub type_name: Spanned<TypeName>,
}

#[derive(Debug, Clone)]
pub struct OnHandler {
    pub event: Spanned<String>,
    pub payload_type: Option<Spanned<TypeName>>,
    pub body: Vec<Spanned<Stmt>>,
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

#[derive(Debug, Clone, Copy)]
pub enum DurationUnit {
    Seconds,
    Minutes,
    Hours,
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
    Custom(String),
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
    /// `reason "prompt"`
    Reason(Box<Spanned<Expr>>),
    /// `classify expr into ["a", "b"]`
    Classify(ClassifyExpr),
    /// `search "query"`
    Search(Box<Spanned<Expr>>),
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
}

#[derive(Debug, Clone)]
pub enum TemplatePart {
    Text(String),
    Interp(Box<Spanned<Expr>>),
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
}

// ── Statements ────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Stmt {
    /// `name = expr`
    Bind(Spanned<String>, Spanned<Expr>),
    /// `give expr` with optional `with` clause
    Give(Spanned<Expr>, Option<Spanned<Expr>>),
    /// `say expr`
    Say(Spanned<Expr>),
    /// `when`/`else` block
    When(Box<WhenBlock>),
    /// `escalate to target`
    Escalate(Spanned<String>),
    /// `memory.field = expr`
    MemoryUpdate(Spanned<String>, Spanned<Expr>),
    /// Bare expression as statement
    ExprStmt(Spanned<Expr>),
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
