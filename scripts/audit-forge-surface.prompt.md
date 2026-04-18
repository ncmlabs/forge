# FORGE Surface Audit Protocol

You are running a non-interactive FORGE derived-surface audit. Your job is to detect drift between authoritative source truth and user-facing derived surfaces, and to **fix it in place** so a draft PR can carry the edits for human review.

## Authoritative sources (READ ONLY — never edit)

- `grammar/forge.pest`
- `src/ast.rs`
- `src/parser.rs`
- `src/resolver.rs`
- `src/checker/` (all modules)
- `src/runtime/` (all modules — executor, agent, session_manager, session_adapter, verification, verification_engine, knowledge_store, skill_executor, skill_loader, command_manager, http_server, event_bus, instance_registry, warden, timer_engine)
- `src/main.rs`
- `src/build.rs`
- `tests/` and `conformance/` (read as additional evidence of behavior)

## Derived surfaces (THESE you may edit)

- `docs/forge-reference.md`
- `docs/training-development-workflow.md`
- `skills/claude-code/SKILL.md` (FORGE preamble block)
- `examples/**/*.forge`
- `examples/validation.toml`
- `workflows/**/*.forge`
- `CHANGELOG.md` (add a single `Changed` entry summarizing the audit)

## Hard scope rules

- Never modify paths under `src/`, `grammar/`, `tests/`, `conformance/`, `Cargo.toml`, `Cargo.lock`, `.github/`, or `scripts/`. A post-run path-guard will reject the job if you do.
- Do not rewrite prose wholesale. Prefer surgical edits that make the surface match the implementation.
- Do not invent language features. If the source does not implement it, do not document it.
- Do not change the meaning of examples to make them pass. If an example is semantically wrong against current source, fix the example to demonstrate the real current behavior, or move it under `examples/errors/` and list it in `examples/validation.toml` with `check = "error"`.

## Audit procedure

1. Re-read `AGENTS.md`, `docs/training-development-workflow.md`, and `workflows/dev-cycle.forge` first for grounding.
2. For each authoritative area, check whether derived surfaces accurately reflect it:
   - **Grammar** (`grammar/forge.pest`): compare every top-level rule against `docs/forge-reference.md` syntax sections. Flag missing or renamed rules.
   - **AST** (`src/ast.rs`): for each public enum variant / struct / primitive (`Command`, `Session`, `AgentResult`, `Spawn`, `Find`, `Retire`, `Learn`, `Recall`, `Knowledge`, `Skill`, `Schedule`, raw interpolation, etc.), confirm the reference documents its shape and semantics.
   - **Checkers**: for each checker, confirm the reference documents the constraint it enforces and, where useful, an example that violates it lives in `examples/errors/`.
   - **Runtime primitives**: for `command`, `session`, verification, knowledge store, spawn/find/retire, skill loader, HTTP endpoints, event bus, warden, timer, schedule — confirm reference and at least one working example each (classified in `examples/validation.toml`).
   - **SKILL preamble** (`skills/claude-code/SKILL.md` FORGE preamble block): ensure the list of authoritative files matches what is actually authoritative today. Add newly-introduced modules; remove anything that no longer exists.
3. Use `git log --since='6 weeks ago' --oneline -- src/ grammar/ src/checker/ src/runtime/` to identify recent language-surface changes and prioritize their derived-surface follow-through.
4. Make the minimal set of edits to close each identified drift item.
5. Run locally: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, and `scripts/check-forge-examples.sh`. If any gate fails because of your edits, fix your edits (not the test) until green.
6. Append a single `CHANGELOG.md` entry under `Changed`: `FORGE surface audit (<ISO week>): <one-line summary of drift fixed>`.

## Output contract

When the audit is complete:

1. Print a short plaintext report to stdout beginning with the literal line `AUDIT REPORT:` followed by a bulleted list of drift items addressed (each item: one line, referencing the authoritative source file and the derived surface updated). End the report with the literal line `AUDIT REPORT END`.
2. Do not commit, push, or open a PR — the surrounding workflow will do that based on your edits and the report.
3. If you find zero drift, make no edits and emit an `AUDIT REPORT:` block containing the single line `no drift detected`.
