# FORGE Repository Workflow

This file defines repository-local agent instructions for FORGE. These instructions override broader default workflow guidance when working in this repository.

## Canonical Sources

- Re-read `docs/training-development-workflow.md` at the start of every session.
- Read `workflows/dev-cycle.forge` at the start of every session and map work to the `IssueLifecycle` state machine:
  - `open -> planning -> in_progress -> testing -> review_ready -> merged`
- Treat these files as the primary grounding sources for implementation details and design intent:
  - `docs/forge-reference.md`
  - `forge-principles.md`
  - `roadmap.md`
  - `CHANGELOG.md`

## Session Start

Before making substantive changes:

1. Read `docs/training-development-workflow.md`.
2. Read `workflows/dev-cycle.forge`.
3. Read the relevant GitHub issue with `gh issue view <N>`.
4. Read the issue acceptance criteria carefully; they are the exit gate.
5. Cross-check the issue against `docs/forge-reference.md` because the spec may be newer than the issue.

## Issue-First Workflow

- Every code change starts from a GitHub issue. No exceptions.
- Treat the GitHub issue as the working thread for the task.
- If the issue and spec diverge, the spec is the source of truth unless explicitly superseded by authoritative design input from Claudiu.
- Claudiu (`4bidden`) is the language designer. Treat his design answers as authoritative implementation input. Do not redesign or soften them.

## Planning Rules

- Use plan mode before implementation.
- Explore the codebase before asking questions. Read files; do not guess.
- Check `forge-principles.md` for every design decision and derive the answer from principles and `roadmap.md` whenever possible.
- Do not ask Claudiu to choose between design approaches when the answer is derivable from repo principles, roadmap, or spec.
- If a design question is asked and Claudiu answers with grammar, AST types, or rationale, incorporate that answer directly.
- Include a principles checklist in the plan using whichever principles are relevant:
  - Principle I: confidence values assigned correctly
  - Principle II: expression handled in `pure_checker`
  - Principle III: token cost tracked
  - Principle VIII: lifecycle events traced

## Git And Branch Rules

- Always branch from `development`, never `main`.
- Always open PRs against `development`, never `main`.
- Branch naming:
  - `feature/<issue-id>-<short-description>` for features
  - `fix/<issue-id>-<short-description>` for bug fixes
- Do not commit `CLAUDE.md` or `.claude/` contents.
- Do not commit `.env` files, credentials, or other secrets.

## Workspace Hygiene And Self-Healing

- Treat the repository root checkout as a shared base clone, not the default implementation workspace.
- Before starting issue work, sync the base clone first:
  - `git fetch origin`
  - `git checkout development`
  - `git pull --ff-only origin development`
- Then create a fresh issue-specific worktree from `origin/development` and do the implementation there, not in the shared root checkout.
- Standard worktree locations:
  - `../forge-<issue-id>` for the working tree
  - `feature/<issue-id>-<short-description>` or `fix/<issue-id>-<short-description>` for the branch
- Required bootstrap sequence for every issue:
  - `git fetch origin`
  - `git checkout development`
  - `git pull --ff-only origin development`
  - `git worktree add ../forge-<issue-id> -b <branch-name> origin/development`
  - `cd ../forge-<issue-id>`
- If the current checkout is dirty, do not try to clean it, inspect it repeatedly, or work around it in place. Self-heal by moving the task into a fresh worktree.
- Do not use destructive cleanup commands such as `git reset --hard` or `git checkout --` to recover from the shared checkout state.
- After merge, remove the issue worktree and delete the local branch from the base clone:
  - `git worktree remove ../forge-<issue-id>`
  - `git branch -d <branch-name>`

## Implementation Rules

- Follow existing FORGE codebase patterns instead of introducing new ones casually.
- Common patterns to preserve:
  - `Spanned<T>` for AST nodes with source spans
  - `Result<T, anyhow::Error>` for public API error propagation
  - `Arc<Mutex<T>>` for shared async state
  - `HashMap<String, Decl>` for registries by declaration name
  - `Env` scope stack for block scoping
  - `async/await` with tokio for I/O
  - `MockProvider` in tests instead of real API keys
- Keep scope tight. Do not add features, refactor unrelated code, or add configurability beyond the issue.

## TaskExecutor Wiring Rule

When adding a new runtime component through a `with_*` builder on `TaskExecutor`:

1. Grep for all `TaskExecutor::new()` call sites across the entire codebase.
2. Wire the new dependency into every call site.
3. Do not assume passing tests are sufficient if real construction paths were not exercised.

## Verification Gates

All three verification gates are required unless the task is explicitly exempted:

1. Test:
   - `cargo test`
2. Format and lint:
   - `cargo fmt --check`
   - `cargo clippy --all-targets -- -D warnings`
3. Real-world validation:
   - For language, runtime, and skill changes, run a real `.forge` example with `cargo run -- run ...` against a real LLM.
   - For skill changes, use a project with `forge.project.toml` declaring skills.
   - For language features, write or use a `.forge` program that exercises the feature.
   - For UI work, validate in a real browser via Playwright: screenshot each page, interact with controls, and check the browser console for errors.

- Re-read the issue acceptance criteria during testing and verify each one explicitly.
- If any acceptance criterion is ambiguous, resolve that ambiguity before shipping.

## Principles Audit

Run a principles audit after implementation and before commit. Treat principle violations as test failures.

- Principle I. Honesty: confidence values are correct and uncertainty is explicit
- Principle II. Purity: new expressions are handled in `pure_checker` where required
- Principle III. Economy: token cost is tracked and zero for non-LLM paths
- Principle IV. Determinism: pure functions do not perform LLM or side effects
- Principle V. Containment: cleanup happens on agent retire
- Principle VII. Accountability: spawned agents are tracked
- Principle VIII. Traceability: all lifecycle events are traced
- Principle IX. Separation: checker/runtime separation is preserved

## Changelog And Shipping

- Update `CHANGELOG.md` for every user-facing change.
- When an issue is confirmed done, update `roadmap.md` in the same closeout loop so the relevant milestone, issue status, and Phase progress counters stay accurate.
- Use Keep a Changelog categories:
  - `Added`
  - `Changed`
  - `Fixed`
  - `Removed`
  - `Deprecated`
  - `Security`
- Every PR description must include:
  - a short summary of the implementation
  - the issue acceptance criteria copied or restated as a checklist
  - an explicit outcome for each acceptance criterion
  - the exact verification performed (`cargo test`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, real-world validation, and any targeted tests)
  - any known gaps, follow-up issues, or intentionally deferred behavior
- When acceptance criteria pass and verification is green, commit, push, and open the PR to `development` without asking for additional permission.
- Monitor CI after opening the PR.
- If CI fails, inspect the failed logs, fix the issue, push again, and re-check CI.
- After merge, close the GitHub issue manually because PRs targeting `development` do not auto-close issues reliably.

## Collaboration Style

- Naming new constructs may require multiple rounds. Favor precise, forge-era naming and present honest trade-offs.
- FORGE owns orchestration semantics such as agents, systems, events, wardens, skills, state machines, and tracing.
- Apps built with FORGE own delivery surfaces such as Slack, TUI, browser, and CLI UX.
- Do not add UI milestones to the language issue tracker.
