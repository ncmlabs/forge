# Boundary Checker Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement compile-time enforcement that server code doesn't leak into client bundles and vice versa (issue #21).

**Architecture:** Two-phase boundary checker — Phase 1 validates per-file rules (endpoint placement, shared type serializability), Phase 2 merges symbol tables across files and rejects cross-boundary references. The checker accepts multiple parsed Programs unlike the single-file checkers. It integrates into `main.rs` alongside but separate from `check_all()`.

**Tech Stack:** Rust, existing FORGE AST/parser/diagnostic infrastructure.

**Spec:** `docs/superpowers/specs/2026-04-02-boundary-checker-design.md`

---

### Task 1: Scaffold boundary_checker module with per-file endpoint placement check

**Files:**
- Create: `src/checker/boundary_checker.rs`
- Modify: `src/checker/mod.rs:1-8`
- Create: `tests/boundary_tests.rs`

- [ ] **Step 1: Write the failing test for endpoint in client boundary**

In `tests/boundary_tests.rs`:

```rust
// Tests for FORGE boundary checker (issue #21)

use forge::diagnostic::{Diagnostic, DiagnosticKind};
use forge::parser::parse;

/// Parse multiple (source, filename) pairs and run boundary checker.
fn check_boundary(sources: &[(&str, &str)]) -> Vec<Diagnostic> {
    let parsed: Vec<_> = sources
        .iter()
        .map(|(src, name)| {
            let program = parse(src).expect(&format!("parse failed for {}", name));
            (program, name.to_string())
        })
        .collect();
    let refs: Vec<_> = parsed.iter().map(|(p, n)| (p, n.as_str())).collect();
    forge::checker::boundary_checker::check(&refs)
}

fn errors(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
    diags.iter().filter(|d| matches!(d.kind, DiagnosticKind::Error)).collect()
}

fn warnings(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
    diags.iter().filter(|d| matches!(d.kind, DiagnosticKind::Warning)).collect()
}

// ── Endpoint placement ──────────────────────────────────────

#[test]
fn endpoint_in_client_boundary_is_error() {
    let source = "\
#! boundary: client

endpoint login(user: Text, pass: Text)
  give \"ok\"
";
    let diags = check_boundary(&[(source, "client.forge")]);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("login"));
    assert!(errs[0].message.contains("client"));
}
```

- [ ] **Step 2: Create minimal boundary_checker.rs that compiles but fails the test**

In `src/checker/boundary_checker.rs`:

```rust
// FORGE boundary checker
// See issue #21 for full specification

use crate::ast::Program;
use crate::diagnostic::Diagnostic;

pub fn check(_programs: &[(&Program, &str)]) -> Vec<Diagnostic> {
    Vec::new()
}
```

- [ ] **Step 3: Register the module in mod.rs**

In `src/checker/mod.rs`, add after line 3 (`pub mod states_checker;`):

```rust
pub mod boundary_checker;
```

Do NOT add it to `check_all()` — boundary checker has a different signature.

- [ ] **Step 4: Run test to verify it fails**

Run: `cargo test --test boundary_tests endpoint_in_client_boundary_is_error -- --nocapture`
Expected: FAIL — `assert_eq!(errs.len(), 1)` fails because checker returns empty vec.

- [ ] **Step 5: Implement endpoint placement check**

Replace `src/checker/boundary_checker.rs` with:

```rust
// FORGE boundary checker
// See issue #21 for full specification

use crate::ast::{BoundaryKind, Program, TopLevel};
use crate::diagnostic::Diagnostic;

// ── Public API ─────────────────────────────────────────────

pub fn check(programs: &[(&Program, &str)]) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // Phase 1: per-file validation
    for (program, file) in programs {
        let boundary = effective_boundary(program);
        check_endpoint_placement(program, boundary, file, &mut diagnostics);
    }

    diagnostics
}

// ── Helpers ────────────────────────────────────────────────

/// Determine the effective boundary for a program.
/// Files without a directive default to Shared.
fn effective_boundary(program: &Program) -> BoundaryKind {
    program
        .boundary
        .as_ref()
        .map(|b| b.node.kind.node)
        .unwrap_or(BoundaryKind::Shared)
}

// ── Phase 1: Per-file checks ──────────────────────────────

fn check_endpoint_placement(
    program: &Program,
    boundary: BoundaryKind,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if boundary == BoundaryKind::Server {
        return; // endpoints are legal in server boundary
    }

    for item in &program.items {
        if let TopLevel::Endpoint(ep) = &item.node {
            let boundary_name = match boundary {
                BoundaryKind::Client => "client",
                BoundaryKind::Shared => "shared",
                BoundaryKind::Server => unreachable!(),
            };
            diagnostics.push(
                Diagnostic::error(
                    file,
                    format!(
                        "endpoint `{}` is not allowed in {} boundary",
                        ep.name.node, boundary_name
                    ),
                    ep.name.span.start..ep.name.span.end,
                    format!(
                        "endpoints can only be declared in `server` boundary files"
                    ),
                )
                .with_help("move this endpoint to a file with `#! boundary: server`"),
            );
        }
    }
}
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo test --test boundary_tests endpoint_in_client_boundary_is_error -- --nocapture`
Expected: PASS

- [ ] **Step 7: Add test for endpoint in shared boundary**

Append to `tests/boundary_tests.rs`:

```rust
#[test]
fn endpoint_in_shared_boundary_is_error() {
    let source = "\
#! boundary: shared

endpoint health()
  give \"ok\"
";
    let diags = check_boundary(&[(source, "shared.forge")]);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("health"));
    assert!(errs[0].message.contains("shared"));
}

#[test]
fn endpoint_in_server_boundary_is_ok() {
    let source = "\
#! boundary: server

endpoint health()
  give \"ok\"
";
    let diags = check_boundary(&[(source, "server.forge")]);
    assert!(diags.is_empty());
}

#[test]
fn endpoint_in_file_without_boundary_is_error() {
    // No boundary directive = defaults to shared
    let source = "\
endpoint health()
  give \"ok\"
";
    let diags = check_boundary(&[(source, "no_boundary.forge")]);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("shared"));
}
```

- [ ] **Step 8: Run all boundary tests**

Run: `cargo test --test boundary_tests -- --nocapture`
Expected: All 4 tests PASS

- [ ] **Step 9: Commit**

```bash
git add src/checker/boundary_checker.rs src/checker/mod.rs tests/boundary_tests.rs
git commit -m "feat: scaffold boundary_checker with endpoint placement check (issue #21)"
```

---

### Task 2: Cross-file symbol table construction and reference validation

**Files:**
- Modify: `src/checker/boundary_checker.rs`
- Modify: `tests/boundary_tests.rs`

- [ ] **Step 1: Write failing test for client referencing server symbol**

Append to `tests/boundary_tests.rs`:

```rust
// ── Cross-boundary reference checks ─────────────────────────

#[test]
fn client_referencing_server_task_is_error() {
    let server = "\
#! boundary: server

task process_secret
  needs data: Text
  gives Text
  do
    give data
";
    let client = "\
#! boundary: client

task show_ui
  needs input: Text
  gives Text
  do
    result = process_secret(input)
    give result
";
    let diags = check_boundary(&[(server, "server.forge"), (client, "client.forge")]);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("process_secret"));
    assert!(errs[0].message.contains("server"));
    assert_eq!(errs[0].file, "client.forge");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test boundary_tests client_referencing_server_task_is_error -- --nocapture`
Expected: FAIL — checker doesn't do cross-boundary checks yet.

- [ ] **Step 3: Add symbol registry types and construction**

Add to `src/checker/boundary_checker.rs`, after the imports:

```rust
use std::collections::{HashMap, HashSet};

use crate::ast::{
    BoundaryKind, Expr, Program, Spanned, Stmt, TemplatePart, TopLevel, TypeName,
};
use crate::diagnostic::Diagnostic;

// ── Symbol registry ────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
enum SymbolKind {
    Task,
    Pure,
    Flow,
    Agent,
    Pool,
    Event,
    States,
    TypeDef,
    Endpoint,
    Contract,
    System,
}

struct BoundaryRegistry {
    /// Maps symbol name -> (boundary, kind, file, span start, span end)
    symbols: HashMap<String, (BoundaryKind, SymbolKind, String, usize, usize)>,
    /// Quick lookup: names in server boundary
    server_symbols: HashSet<String>,
    /// Quick lookup: names in client boundary
    client_symbols: HashSet<String>,
}

impl BoundaryRegistry {
    fn build(programs: &[(&Program, &str)]) -> Self {
        let mut symbols = HashMap::new();
        let mut server_symbols = HashSet::new();
        let mut client_symbols = HashSet::new();

        for (program, file) in programs {
            let boundary = effective_boundary(program);

            for item in &program.items {
                let (name, kind) = match &item.node {
                    TopLevel::Task(t) => (t.name.node.clone(), SymbolKind::Task),
                    TopLevel::Pure(p) => (p.name.node.clone(), SymbolKind::Pure),
                    TopLevel::Flow(f) => (f.name.node.clone(), SymbolKind::Flow),
                    TopLevel::Agent(a) => (a.name.node.clone(), SymbolKind::Agent),
                    TopLevel::Pool(p) => (p.name.node.clone(), SymbolKind::Pool),
                    TopLevel::Event(e) => (e.name.node.clone(), SymbolKind::Event),
                    TopLevel::States(s) => (s.name.node.clone(), SymbolKind::States),
                    TopLevel::TypeDef(t) => (t.name.node.clone(), SymbolKind::TypeDef),
                    TopLevel::Endpoint(e) => (e.name.node.clone(), SymbolKind::Endpoint),
                    TopLevel::Contract(c) => (c.name.node.clone(), SymbolKind::Contract),
                    TopLevel::System(s) => (s.name.node.clone(), SymbolKind::System),
                    TopLevel::FnMain(_) | TopLevel::Use(_) => continue,
                };

                let span_start = item.span.start;
                let span_end = item.span.end;

                match boundary {
                    BoundaryKind::Server => { server_symbols.insert(name.clone()); }
                    BoundaryKind::Client => { client_symbols.insert(name.clone()); }
                    BoundaryKind::Shared => {}
                }

                symbols.insert(name, (boundary, kind, file.to_string(), span_start, span_end));
            }
        }

        Self { symbols, server_symbols, client_symbols }
    }
}
```

- [ ] **Step 4: Add expression/statement walker for reference collection**

Add to `src/checker/boundary_checker.rs`:

```rust
// ── Reference walker ───────────────────────────────────────

/// Check all identifier references in a statement list against boundary rules.
fn check_refs_in_stmts(
    stmts: &[Spanned<Stmt>],
    file_boundary: BoundaryKind,
    registry: &BoundaryRegistry,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for stmt in stmts {
        check_refs_in_stmt(stmt, file_boundary, registry, file, diagnostics);
    }
}

fn check_refs_in_stmt(
    stmt: &Spanned<Stmt>,
    file_boundary: BoundaryKind,
    registry: &BoundaryRegistry,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match &stmt.node {
        Stmt::Bind(_, expr) | Stmt::Say(expr) | Stmt::ExprStmt(expr) => {
            check_refs_in_expr(expr, file_boundary, registry, file, diagnostics);
        }
        Stmt::Give(expr, with_expr) => {
            check_refs_in_expr(expr, file_boundary, registry, file, diagnostics);
            if let Some(w) = with_expr {
                check_refs_in_expr(w, file_boundary, registry, file, diagnostics);
            }
        }
        Stmt::Emit(name, args) => {
            check_name_ref(&name.node, name.span.start, name.span.end, file_boundary, registry, file, diagnostics);
            for arg in args {
                check_refs_in_expr(&arg.node.value, file_boundary, registry, file, diagnostics);
            }
        }
        Stmt::Escalate(name) => {
            check_name_ref(&name.node, name.span.start, name.span.end, file_boundary, registry, file, diagnostics);
        }
        Stmt::Forward(a, b) => {
            check_refs_in_expr(a, file_boundary, registry, file, diagnostics);
            check_refs_in_expr(b, file_boundary, registry, file, diagnostics);
        }
        Stmt::IfElse(ie) => {
            check_refs_in_expr(&ie.condition, file_boundary, registry, file, diagnostics);
            check_refs_in_stmts(&ie.then_body, file_boundary, registry, file, diagnostics);
            for (cond, body) in &ie.else_ifs {
                check_refs_in_expr(cond, file_boundary, registry, file, diagnostics);
                check_refs_in_stmts(body, file_boundary, registry, file, diagnostics);
            }
            if let Some(body) = &ie.else_body {
                check_refs_in_stmts(body, file_boundary, registry, file, diagnostics);
            }
        }
        Stmt::When(when) => {
            for clause in &when.clauses {
                check_refs_in_stmt(&clause.node.body, file_boundary, registry, file, diagnostics);
            }
            if let Some(else_clause) = &when.else_body {
                check_refs_in_stmt(&else_clause.node.body, file_boundary, registry, file, diagnostics);
            }
        }
        Stmt::Match(m) => {
            check_refs_in_expr(&m.subject, file_boundary, registry, file, diagnostics);
            for arm in &m.arms {
                check_refs_in_stmt(&arm.node.body, file_boundary, registry, file, diagnostics);
            }
        }
        Stmt::For(f) => {
            check_refs_in_expr(&f.iterable, file_boundary, registry, file, diagnostics);
            check_refs_in_stmts(&f.body, file_boundary, registry, file, diagnostics);
        }
        Stmt::MemoryUpdate(_, idx, expr) => {
            if let Some(idx_expr) = idx {
                check_refs_in_expr(idx_expr, file_boundary, registry, file, diagnostics);
            }
            check_refs_in_expr(expr, file_boundary, registry, file, diagnostics);
        }
        // TransitionTo, StartTimer, CancelTimer, ResetTimer — no cross-boundary refs
        _ => {}
    }
}

fn check_refs_in_expr(
    expr: &Spanned<Expr>,
    file_boundary: BoundaryKind,
    registry: &BoundaryRegistry,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match &expr.node {
        Expr::Ident(name) => {
            check_name_ref(name, expr.span.start, expr.span.end, file_boundary, registry, file, diagnostics);
        }
        Expr::Call(c) => {
            check_name_ref(&c.name.node, c.name.span.start, c.name.span.end, file_boundary, registry, file, diagnostics);
            for arg in &c.args {
                check_refs_in_expr(&arg.node.value, file_boundary, registry, file, diagnostics);
            }
        }
        Expr::Constructor(c) => {
            if let TypeName::Custom(name) = &c.type_name.node {
                check_name_ref(name, c.type_name.span.start, c.type_name.span.end, file_boundary, registry, file, diagnostics);
            }
            for arg in &c.args {
                check_refs_in_expr(&arg.node.value, file_boundary, registry, file, diagnostics);
            }
        }
        Expr::Reason(inner) | Expr::Search(inner) => {
            check_refs_in_expr(inner, file_boundary, registry, file, diagnostics);
        }
        Expr::Classify(c) => {
            check_refs_in_expr(&c.input, file_boundary, registry, file, diagnostics);
        }
        Expr::TryOr(a, b) | Expr::BinOp(a, _, b) | Expr::Index(a, b) => {
            check_refs_in_expr(a, file_boundary, registry, file, diagnostics);
            check_refs_in_expr(b, file_boundary, registry, file, diagnostics);
        }
        Expr::Compose(parts) | Expr::FanOut(parts) => {
            for p in parts {
                check_refs_in_expr(p, file_boundary, registry, file, diagnostics);
            }
        }
        Expr::UnaryOp(_, a) | Expr::Paren(a) | Expr::FieldAccess(a, _) | Expr::GlobAccess(a) => {
            check_refs_in_expr(a, file_boundary, registry, file, diagnostics);
        }
        Expr::MethodCall(inner, _, args) => {
            check_refs_in_expr(inner, file_boundary, registry, file, diagnostics);
            for arg in args {
                check_refs_in_expr(&arg.node.value, file_boundary, registry, file, diagnostics);
            }
        }
        Expr::ArrayLit(elems) => {
            for e in elems {
                check_refs_in_expr(e, file_boundary, registry, file, diagnostics);
            }
        }
        Expr::Template(parts) => {
            for part in parts {
                if let TemplatePart::Interp(inner) = &part.node {
                    check_refs_in_expr(inner, file_boundary, registry, file, diagnostics);
                }
            }
        }
        // Leaves: literals, type access — no cross-boundary refs
        Expr::NumberLit(_) | Expr::BoolLit(_) | Expr::TypeAccess(_, _) => {}
    }
}

/// Check if a name reference crosses a boundary it shouldn't.
fn check_name_ref(
    name: &str,
    span_start: usize,
    span_end: usize,
    file_boundary: BoundaryKind,
    registry: &BoundaryRegistry,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match file_boundary {
        BoundaryKind::Client => {
            if registry.server_symbols.contains(name) {
                diagnostics.push(
                    Diagnostic::error(
                        file,
                        format!("client code references server-only symbol `{}`", name),
                        span_start..span_end,
                        format!("`{}` is declared in a server boundary file", name),
                    )
                    .with_help("use a shared type or endpoint to communicate across boundaries"),
                );
            }
        }
        BoundaryKind::Server => {
            if registry.client_symbols.contains(name) {
                diagnostics.push(
                    Diagnostic::error(
                        file,
                        format!("server code references client-only symbol `{}`", name),
                        span_start..span_end,
                        format!("`{}` is declared in a client boundary file", name),
                    )
                    .with_help("use a shared type or endpoint to communicate across boundaries"),
                );
            }
        }
        BoundaryKind::Shared => {
            // Shared code cannot reference server or client symbols
            if registry.server_symbols.contains(name) {
                diagnostics.push(
                    Diagnostic::error(
                        file,
                        format!("shared code references server-only symbol `{}`", name),
                        span_start..span_end,
                        format!("`{}` is declared in a server boundary file", name),
                    )
                    .with_help("shared code can only reference other shared declarations"),
                );
            }
            if registry.client_symbols.contains(name) {
                diagnostics.push(
                    Diagnostic::error(
                        file,
                        format!("shared code references client-only symbol `{}`", name),
                        span_start..span_end,
                        format!("`{}` is declared in a client boundary file", name),
                    )
                    .with_help("shared code can only reference other shared declarations"),
                );
            }
        }
    }
}
```

- [ ] **Step 5: Wire cross-boundary checking into the main `check` function**

Update the `check` function in `src/checker/boundary_checker.rs` to add Phase 2:

```rust
pub fn check(programs: &[(&Program, &str)]) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // Phase 1: per-file validation
    for (program, file) in programs {
        let boundary = effective_boundary(program);
        check_endpoint_placement(program, boundary, file, &mut diagnostics);
    }

    // Phase 2: cross-file symbol table + reference validation
    let registry = BoundaryRegistry::build(programs);
    for (program, file) in programs {
        let boundary = effective_boundary(program);
        check_cross_boundary_refs(program, boundary, &registry, file, &mut diagnostics);
    }

    diagnostics
}
```

Add the `check_cross_boundary_refs` function:

```rust
fn check_cross_boundary_refs(
    program: &Program,
    boundary: BoundaryKind,
    registry: &BoundaryRegistry,
    file: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for item in &program.items {
        match &item.node {
            TopLevel::Task(t) => {
                let stmts = match &t.body.node {
                    crate::ast::TaskBody::Do(stmts) => stmts.clone(),
                    crate::ast::TaskBody::Is(expr) => {
                        vec![Spanned::new(Stmt::ExprStmt((**expr).clone()), t.body.span)]
                    }
                };
                check_refs_in_stmts(&stmts, boundary, registry, file, diagnostics);
            }
            TopLevel::Pure(p) => {
                check_refs_in_stmts(&p.body, boundary, registry, file, diagnostics);
            }
            TopLevel::Flow(f) => {
                for stage in &f.stages {
                    check_refs_in_stmts(&stage.node.body, boundary, registry, file, diagnostics);
                }
            }
            TopLevel::Agent(a) => {
                for handler in &a.handlers {
                    check_refs_in_stmts(&handler.node.body, boundary, registry, file, diagnostics);
                }
            }
            TopLevel::Endpoint(ep) => {
                check_refs_in_stmts(&ep.body, boundary, registry, file, diagnostics);
            }
            TopLevel::FnMain(m) => {
                check_refs_in_stmts(&m.body, boundary, registry, file, diagnostics);
            }
            _ => {}
        }
    }
}
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo test --test boundary_tests client_referencing_server_task_is_error -- --nocapture`
Expected: PASS

- [ ] **Step 7: Add test for server referencing client symbol**

Append to `tests/boundary_tests.rs`:

```rust
#[test]
fn server_referencing_client_declaration_is_error() {
    let client = "\
#! boundary: client

pure render_ui
  needs data: Text
  gives Text
  do
    give data
";
    let server = "\
#! boundary: server

task process
  needs input: Text
  gives Text
  do
    result = render_ui(input)
    give result
";
    let diags = check_boundary(&[(client, "client.forge"), (server, "server.forge")]);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("render_ui"));
    assert!(errs[0].message.contains("client"));
    assert_eq!(errs[0].file, "server.forge");
}
```

- [ ] **Step 8: Run to verify**

Run: `cargo test --test boundary_tests server_referencing_client_declaration_is_error -- --nocapture`
Expected: PASS (already implemented)

- [ ] **Step 9: Add test for valid shared access from both boundaries**

Append to `tests/boundary_tests.rs`:

```rust
#[test]
fn shared_type_accessible_from_server_and_client() {
    let shared = "\
#! boundary: shared

type Message
  content: Text
  sender: Text
";
    let server = "\
#! boundary: server

task process
  needs msg: Text
  gives Text
  do
    m = Message(content: msg, sender: \"system\")
    give msg
";
    let client = "\
#! boundary: client

task display
  needs msg: Text
  gives Text
  do
    m = Message(content: msg, sender: \"user\")
    give msg
";
    let diags = check_boundary(&[
        (shared, "shared.forge"),
        (server, "server.forge"),
        (client, "client.forge"),
    ]);
    assert!(errors(&diags).is_empty());
}
```

- [ ] **Step 10: Add test for same-boundary references**

Append to `tests/boundary_tests.rs`:

```rust
#[test]
fn same_boundary_references_are_ok() {
    let server1 = "\
#! boundary: server

pure validate
  needs x: Text
  gives Bool
  do
    give true
";
    let server2 = "\
#! boundary: server

task process
  needs input: Text
  gives Text
  do
    ok = validate(input)
    give input
";
    let diags = check_boundary(&[(server1, "server1.forge"), (server2, "server2.forge")]);
    assert!(errors(&diags).is_empty());
}
```

- [ ] **Step 11: Run all boundary tests**

Run: `cargo test --test boundary_tests -- --nocapture`
Expected: All tests PASS

- [ ] **Step 12: Commit**

```bash
git add src/checker/boundary_checker.rs tests/boundary_tests.rs
git commit -m "feat: add cross-file boundary reference validation (issue #21)"
```

---

### Task 3: Shared type serializability check

**Files:**
- Modify: `src/checker/boundary_checker.rs`
- Modify: `tests/boundary_tests.rs`

- [ ] **Step 1: Write failing test for non-serializable shared type**

Append to `tests/boundary_tests.rs`:

```rust
// ── Shared type serializability ─────────────────────────────

#[test]
fn shared_type_with_agent_field_is_error() {
    let shared = "\
#! boundary: shared

type Session
  user: Text
  handler: MyAgent
";
    let server = "\
#! boundary: server

agent MyAgent
  on ping(msg: Text)
    say msg
";
    let diags = check_boundary(&[(shared, "shared.forge"), (server, "server.forge")]);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("Session"));
    assert!(errs[0].message.contains("handler"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test boundary_tests shared_type_with_agent_field_is_error -- --nocapture`
Expected: FAIL

- [ ] **Step 3: Implement shared type serializability check**

Add to `src/checker/boundary_checker.rs`, a new function called after registry construction:

```rust
fn check_shared_serializability(
    programs: &[(&Program, &str)],
    registry: &BoundaryRegistry,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for (program, file) in programs {
        let boundary = effective_boundary(program);
        if boundary != BoundaryKind::Shared {
            continue;
        }

        for item in &program.items {
            if let TopLevel::TypeDef(typedef) = &item.node {
                for field in &typedef.fields {
                    if let TypeName::Custom(ref_name) = &field.node.type_name.node {
                        // Check if this custom type is an agent, pool, or flow
                        if let Some((_, kind, _, _, _)) = registry.symbols.get(ref_name.as_str()) {
                            let non_serializable = matches!(
                                kind,
                                SymbolKind::Agent | SymbolKind::Pool | SymbolKind::Flow
                            );
                            if non_serializable {
                                let kind_name = match kind {
                                    SymbolKind::Agent => "agent",
                                    SymbolKind::Pool => "pool",
                                    SymbolKind::Flow => "flow",
                                    _ => unreachable!(),
                                };
                                diagnostics.push(
                                    Diagnostic::error(
                                        *file,
                                        format!(
                                            "shared type `{}` contains non-serializable field `{}`",
                                            typedef.name.node, field.node.name
                                        ),
                                        field.node.type_name.span.start..field.node.type_name.span.end,
                                        format!(
                                            "`{}` is an {} reference, which cannot cross the wire",
                                            ref_name, kind_name
                                        ),
                                    )
                                    .with_help("use a shared type or primitive type for shared boundary fields"),
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}
```

Wire it into the `check` function, after the registry is built:

```rust
    // Phase 2: cross-file symbol table + reference validation
    let registry = BoundaryRegistry::build(programs);

    // Phase 2a: shared type serializability
    check_shared_serializability(programs, &registry, &mut diagnostics);

    // Phase 2b: cross-boundary reference validation
    for (program, file) in programs {
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --test boundary_tests shared_type_with_agent_field_is_error -- --nocapture`
Expected: PASS

- [ ] **Step 5: Add test for shared type with pool field**

Append to `tests/boundary_tests.rs`:

```rust
#[test]
fn shared_type_with_pool_field_is_error() {
    let shared = "\
#! boundary: shared

type Config
  name: Text
  workers: MyPool
";
    let server = "\
#! boundary: server

task worker
  needs x: Text
  gives Text
  do
    give x

pool MyPool
  of worker
  workers 3
  strategy fastest
";
    let diags = check_boundary(&[(shared, "shared.forge"), (server, "server.forge")]);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("Config"));
    assert!(errs[0].message.contains("workers"));
}

#[test]
fn shared_type_with_only_primitive_fields_is_ok() {
    let shared = "\
#! boundary: shared

type Message
  content: Text
  count: Number
  valid: Bool
";
    let diags = check_boundary(&[(shared, "shared.forge")]);
    assert!(diags.is_empty());
}
```

- [ ] **Step 6: Run all boundary tests**

Run: `cargo test --test boundary_tests -- --nocapture`
Expected: All tests PASS

- [ ] **Step 7: Commit**

```bash
git add src/checker/boundary_checker.rs tests/boundary_tests.rs
git commit -m "feat: add shared type serializability check (issue #21)"
```

---

### Task 4: Update CLI to support multi-file boundary checking

**Files:**
- Modify: `src/main.rs:38-70` (Command enum) and `src/main.rs:87-109` (Check handler)

- [ ] **Step 1: Write failing test for multi-file CLI check (manual verification)**

This task is a CLI integration change. We'll verify manually and with the existing tests.

First, update the `Check` command to accept multiple files. In `src/main.rs`, change:

```rust
    Check {
        /// Path to the .forge source file
        file: PathBuf,
    },
```

to:

```rust
    Check {
        /// Paths to .forge source files
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
```

- [ ] **Step 2: Update the Check handler**

Replace the `Command::Check { file }` match arm (lines 87-109) with:

```rust
        Command::Check { files } => {
            let mut all_diagnostics = Vec::new();
            let mut parsed_programs = Vec::new();

            for file in &files {
                let source = read_source(file)?;
                let program = parse_or_exit(&source, file);
                let fname = file.display().to_string();

                // Per-file: resolver
                let ctx = forge::resolver::CheckContext::new(&fname);
                if let Err(errors) = ctx.check(&program) {
                    let registry = forge::resolver::CapabilityRegistry::builtin();
                    all_diagnostics.extend(
                        errors.iter().map(|e| e.to_diagnostic(&fname, &registry)),
                    );
                }

                // Per-file: checker (pure, states, requires)
                all_diagnostics.extend(forge::checker::check_all(&program, &fname));

                parsed_programs.push((program, fname, source));
            }

            // Cross-file: boundary checker
            let boundary_refs: Vec<_> = parsed_programs
                .iter()
                .map(|(p, f, _)| (p, f.as_str()))
                .collect();
            all_diagnostics.extend(
                forge::checker::boundary_checker::check(&boundary_refs),
            );

            if all_diagnostics.is_empty() {
                println!("OK");
            } else {
                // Render diagnostics for each file with its source
                for diag in &all_diagnostics {
                    if let Some((_, _, source)) = parsed_programs
                        .iter()
                        .find(|(_, f, _)| f == &diag.file)
                    {
                        diag.render(source);
                    }
                }
                std::process::exit(1);
            }
        }
```

- [ ] **Step 3: Update run_program to also call boundary checker on single file**

In the `run_program` function (around line 160), add after the `check_all` call:

```rust
    diagnostics.extend(forge::checker::check_all(&program, &fname));

    // Boundary checker (single-file — catches endpoint placement etc.)
    let boundary_refs = vec![(&program, fname.as_str())];
    diagnostics.extend(forge::checker::boundary_checker::check(&boundary_refs));
```

- [ ] **Step 4: Verify all existing tests still pass**

Run: `cargo test`
Expected: All tests PASS (no regressions)

- [ ] **Step 5: Verify CLI works with single file**

Run: `cargo run -- check examples/hello.forge`
Expected: `OK` (single file still works with Vec<PathBuf>)

- [ ] **Step 6: Commit**

```bash
git add src/main.rs
git commit -m "feat: update CLI Check command to support multi-file boundary checking (issue #21)"
```

---

### Task 5: Edge case tests and full acceptance criteria

**Files:**
- Modify: `tests/boundary_tests.rs`

- [ ] **Step 1: Add test for server-only declarations absent from client symbol table**

Append to `tests/boundary_tests.rs`:

```rust
// ── Acceptance criteria ─────────────────────────────────────

#[test]
fn server_agent_invisible_to_client() {
    let server = "\
#! boundary: server

agent SecretAgent
  on process(data: Text)
    say data
";
    let client = "\
#! boundary: client

task show
  needs input: Text
  gives Text
  do
    result = SecretAgent(input)
    give result
";
    let diags = check_boundary(&[(server, "server.forge"), (client, "client.forge")]);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("SecretAgent"));
    assert!(errs[0].message.contains("server"));
}
```

- [ ] **Step 2: Add test for no-boundary file treated as shared**

Append to `tests/boundary_tests.rs`:

```rust
#[test]
fn file_without_boundary_defaults_to_shared() {
    // No boundary = shared. Shared cannot reference server symbols.
    let server = "\
#! boundary: server

task secret
  needs x: Text
  gives Text
  do
    give x
";
    let no_boundary = "\
task caller
  needs x: Text
  gives Text
  do
    result = secret(x)
    give result
";
    let diags = check_boundary(&[(server, "server.forge"), (no_boundary, "utils.forge")]);
    let errs = errors(&diags);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].message.contains("secret"));
    assert_eq!(errs[0].file, "utils.forge");
}
```

- [ ] **Step 3: Add test for empty file**

Append to `tests/boundary_tests.rs`:

```rust
#[test]
fn empty_file_no_errors() {
    // An empty .forge file parses to an empty program with no boundary
    let diags = check_boundary(&[("", "empty.forge")]);
    assert!(diags.is_empty());
}
```

Note: if parsing an empty string fails, use a minimal valid source instead (e.g., a comment-only file or a single task). Adjust based on what the parser accepts.

- [ ] **Step 4: Add test for all-shared project**

Append to `tests/boundary_tests.rs`:

```rust
#[test]
fn all_shared_files_no_boundary_violations() {
    let file1 = "\
#! boundary: shared

pure helper
  needs x: Text
  gives Text
  do
    give x
";
    let file2 = "\
#! boundary: shared

task process
  needs x: Text
  gives Text
  do
    result = helper(x)
    give result
";
    let diags = check_boundary(&[(file1, "helpers.forge"), (file2, "main.forge")]);
    assert!(errors(&diags).is_empty());
}
```

- [ ] **Step 5: Add test for client-only code with no cross-boundary refs**

Append to `tests/boundary_tests.rs`:

```rust
#[test]
fn client_only_code_no_errors() {
    let client = "\
#! boundary: client

task render
  needs data: Text
  gives Text
  do
    give data

pure format
  needs x: Text
  gives Text
  do
    give x
";
    let diags = check_boundary(&[(client, "client.forge")]);
    assert!(diags.is_empty());
}
```

- [ ] **Step 6: Run all boundary tests**

Run: `cargo test --test boundary_tests -- --nocapture`
Expected: All tests PASS

- [ ] **Step 7: Run full test suite to verify no regressions**

Run: `cargo test`
Expected: All tests PASS

- [ ] **Step 8: Commit**

```bash
git add tests/boundary_tests.rs
git commit -m "test: add boundary checker edge cases and acceptance criteria (issue #21)"
```

---

### Task 6: Final verification and cleanup

**Files:**
- No new files

- [ ] **Step 1: Run the full test suite**

Run: `cargo test`
Expected: All tests PASS

- [ ] **Step 2: Run clippy for lint check**

Run: `cargo clippy -- -D warnings`
Expected: No warnings

- [ ] **Step 3: Fix any clippy warnings**

Address any clippy issues in `src/checker/boundary_checker.rs` (common: unused variables, unnecessary clones, match arm patterns).

- [ ] **Step 4: Run boundary tests one final time**

Run: `cargo test --test boundary_tests -- --nocapture`
Expected: All tests PASS with clear output

- [ ] **Step 5: Final commit if any cleanup was needed**

```bash
git add -A
git commit -m "chore: clippy cleanup for boundary checker"
```
