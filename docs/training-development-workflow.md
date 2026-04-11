# Training Development Workflow

This document is the session-start reference for FORGE training, documentation, and language-development tasks. It complements `workflows/dev-cycle.forge`, which models the same lifecycle as executable FORGE.

## Lifecycle

Map every implementation or documentation task to the `IssueLifecycle` state machine:

```text
open -> planning -> in_progress -> testing -> review_ready -> merged
```

Use the GitHub issue as the working thread. The issue should capture the target outcome, acceptance criteria, verification performed, known gaps, and PR link.

## Startup Checklist

1. Read this file.
2. Read `workflows/dev-cycle.forge`.
3. Read the GitHub issue and its acceptance criteria.
4. Cross-check the issue against `docs/forge-reference.md`.
5. Prefer repo source truth over memory: grammar, AST, parser, checker, runtime, examples, and tests.

## Documentation and Skill Updates

For language-learning or agent-training work:

- Audit `grammar/forge.pest`, `src/ast.rs`, `src/parser.rs`, checkers, resolver, runtime, CLI, examples, and tests before updating docs.
- Update `docs/forge-reference.md` for implemented constructs that are missing, stale, or contradicted by source.
- Update local agent skills that teach FORGE when their guidance lags implemented syntax.
- Keep positive examples, expected-error examples, and manifest or multi-file examples in separate validation buckets.
- Record any real-runtime or real-LLM validation that could not be run.

## Verification

Run the fastest targeted checks while editing, then finish with the repository gates unless the issue explicitly exempts them:

```bash
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

For language and runtime changes, also run a `.forge` example that exercises the changed surface. For documentation-only updates, run representative parser/checker examples and note whether real-LLM execution was skipped.
