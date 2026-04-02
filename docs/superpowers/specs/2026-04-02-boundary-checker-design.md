# Boundary Checker: Server/Client Code Split Enforcement

Issue: ncmlabs/forge#21

## Context

FORGE's Principle IX (Boundary) states: "Code that must be correct must be separated from code that might be wrong." The `boundary` primitive enforces code partition at the compiler level — `boundary server` code cannot leak into `boundary client` bundles, and `boundary shared` types are the only things that cross the wire.

The parser already handles `#! boundary: server|client|shared` directives (AST: `BoundaryDirective`, `BoundaryKind`) and `endpoint` declarations (`EndpointDecl`). The resolver (#6) provides symbol resolution. What's missing is the compile-time checker that enforces cross-boundary isolation.

Unlike the other checkers (pure, states, requires) which operate on a single `Program`, boundary checking is a **link-time** operation that requires visibility across multiple files. Each `.forge` file belongs to exactly one boundary, and cross-boundary validation merges symbol tables from all files.

## Scope

One deliverable: `src/checker/boundary_checker.rs` — compile-time enforcement that server code doesn't leak into client bundles and vice versa.

Out of scope: wire protocol generation, compile targets (`--boundary` flag), runtime boundary enforcement.

## Checker Design

### Entry point

```rust
pub fn check(programs: &[(&Program, &str)]) -> Vec<Diagnostic>
```

Each tuple is `(parsed_program, filename)`. This differs from other checkers' `check(program, file)` signatures because boundary checking requires multi-file context.

### Phase 1: Per-file validation

These checks run independently on each file before cross-file analysis.

#### Rule 1: Default boundary

Files without a `#! boundary:` directive are treated as `shared`. The checker assigns `BoundaryKind::Shared` to any `Program` where `program.boundary` is `None`.

#### Rule 2: Endpoint placement

`endpoint` declarations are only legal inside `server` boundary files.

- `endpoint` in `client` → **error**: "endpoint `E` is not allowed in client boundary"
- `endpoint` in `shared` → **error**: "endpoint `E` is not allowed in shared boundary"

#### Rule 3: Shared type serializability

Types declared in `shared` boundary files must contain only serializable fields. Non-serializable types are those that reference agents, pools, or other non-data declarations.

For POC: walk `TypeDefDecl` fields in shared files. If a field's `TypeName` is `Custom(name)` and that name resolves to an `Agent`, `Pool`, or `Flow` declaration (from any file), emit **error**: "shared type `T` contains non-serializable field `f` of type `A` (agent reference)".

This requires the cross-file symbol registry (Phase 2) to resolve what `Custom` names point to, so serializability checking runs after symbol table construction.

### Phase 2: Cross-file symbol table construction

Walk all programs and build three registries:

```rust
struct BoundaryRegistry {
    server_symbols: HashMap<String, SymbolInfo>,
    client_symbols: HashMap<String, SymbolInfo>,
    shared_symbols: HashMap<String, SymbolInfo>,
}

struct SymbolInfo {
    kind: SymbolKind,  // Agent, Task, Pure, Flow, Pool, Event, States, TypeDef, Endpoint, Contract, System
    file: String,
    span: Span,
}
```

For each file, determine its effective boundary (explicit directive or default `shared`). Extract all top-level declaration names: Task, Pure, Flow, Agent, Pool, Event, States, TypeDef, Endpoint, Contract, System, FnMain (though FnMain has no name — skip it).

Insert each name into the appropriate boundary map (`server_symbols`, `client_symbols`, or `shared_symbols`).

### Phase 3: Cross-boundary reference validation

Walk every declaration body in every file. For each identifier reference (`Expr::Ident`, `Expr::Call`, `Expr::Constructor`), check:

- **In `client` files**: if the referenced name is in `server_symbols` → **error**: "client code references server-only symbol `S`"
- **In `server` files**: if the referenced name is in `client_symbols` → **error**: "server code references client-only symbol `S`"
- References to `shared_symbols` → always allowed from any boundary
- References to symbols within the same boundary → always allowed
- References to unknown symbols → not this checker's concern (resolver handles it)

#### Reference collection

Walk statements and expressions recursively (same pattern as `pure_checker`'s walk). Collect identifiers from:

- `Expr::Ident(name)` — direct reference
- `Expr::Call(c)` — `c.name` is a reference
- `Expr::Constructor(c)` — `c.type_name` if `Custom(name)`
- `Stmt::Emit(name, _)` — event reference
- `Stmt::Escalate(name)` — agent reference
- `Stmt::Forward(_, target)` — possible agent reference

For each collected name, look it up in the opposite boundary's symbol table.

### Error variants

| Check | Kind | Message pattern |
|-------|------|----------------|
| Endpoint in client | Error | endpoint `E` is not allowed in client boundary |
| Endpoint in shared | Error | endpoint `E` is not allowed in shared boundary |
| Client refs server symbol | Error | client code references server-only symbol `S` |
| Server refs client symbol | Error | server code references client-only symbol `S` |
| Non-serializable shared type | Error | shared type `T` contains non-serializable field `f` (agent/pool reference) |

All errors include:
- File path and span for precise location
- Help text suggesting the fix (move to shared, use endpoint, restructure)

## Integration

### `src/checker/mod.rs`

Add the module declaration:

```rust
pub mod boundary_checker;
```

Do NOT add boundary_checker to `check_all()` — it requires multi-file context. Instead, it's called separately.

### `src/main.rs`

For `forge check <file>`: single-file mode continues to work. Boundary checking of a single file runs the per-file rules only (endpoint placement, shared serializability with limited scope).

For future `forge check <dir>` or `forge build`: parse all files, then call `boundary_checker::check()` with the full set.

For now, update `forge check` to accept multiple file arguments:

```rust
Command::Check { files: Vec<PathBuf> }
```

Parse each file, run per-file checkers (`check_all`), then run `boundary_checker::check()` on the combined set.

## Files modified

| File | Change |
|------|--------|
| `src/checker/boundary_checker.rs` | **New** — boundary enforcement checker |
| `src/checker/mod.rs` | Add `pub mod boundary_checker;` |
| `src/main.rs` | Update `Check` command to accept multiple files, call boundary checker |
| `tests/boundary_tests.rs` | **New** — boundary checker tests |

## Test plan

### Test helper

```rust
fn check_boundary(sources: &[(&str, &str)]) -> Vec<Diagnostic> {
    // sources: &[(forge_source, filename)]
    // Parses each source, runs boundary_checker::check on the combined set
}
```

### Error cases

1. **Client accessing server agent memory** — `client.forge` calls a task defined in `server.forge` → compile error
2. **Server referencing client-only declaration** — `server.forge` calls a pure function defined in `client.forge` → compile error
3. **Endpoint in client boundary** — `#! boundary: client` file contains `endpoint` → compile error
4. **Endpoint in shared boundary** — `#! boundary: shared` file contains `endpoint` → compile error
5. **Non-serializable shared type** — shared file defines `type Msg` with field `agent: MyAgent` where `MyAgent` is an agent → compile error

### Acceptance cases

6. **Server-only declarations absent from client symbol table** — verify `server_symbols` doesn't appear in client's visible scope
7. **Shared types used across boundaries** — type defined in shared file, referenced from both server and client → no error
8. **Endpoint in server boundary** — valid, no error
9. **File without boundary directive** — treated as shared, no error
10. **Same-boundary references** — server file referencing another server file's declarations → no error
11. **Client-only code with no cross-boundary refs** — no errors

### Edge cases

12. **Empty file** — no boundary, no declarations → no errors
13. **All files shared** — no boundary violations possible
14. **Symbol name exists in both server and client** — name collision, but each lives in its own boundary. Not an error for the boundary checker (resolver might care).

## Verification

1. `cargo test` — all existing tests pass
2. `cargo test --test boundary_tests` — all new boundary checker tests pass
3. `forge check server.forge client.forge shared.forge` — multi-file check works
4. Create test `.forge` files with illegal cross-boundary references — `forge check` reports errors correctly
