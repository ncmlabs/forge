# 2026-05 Clone-Dev Playground Proof Run

This note pins the target shape for the T11.3 proof run tracked by
`ncmlabs/forge#372`.

## Target

The proof-run target is `ncmlabs/forge-playground` as a TypeScript proof-run
tracker app on the `main` branch. The app is intentionally useful outside the
proof run: it tracks clone-dev proof runs, tasks, approvals, CI status, review
rounds, time-to-merge, and mastery movement.

Issue `ncmlabs/forge#409` intentionally resets the older Rust/Go playground
state. Any Rust-specific seed issues, Go-oriented smoke issues, or stale
clone-dev branches must be superseded before restarting `#372`.

## Branch Requirement

Clone-dev currently clones the repository default branch:

```sh
gh repo clone ncmlabs/forge-playground <workdir>
```

Before starting a proof run, confirm the GitHub default branch is `main`:

```sh
gh repo view ncmlabs/forge-playground --json defaultBranchRef \
  --jq '.defaultBranchRef.name'
```

Expected output:

```text
main
```

The halted `2026-05-18-run1` attempt failed because the proof surface drifted
between Rust and Go scaffolds. The reset tracked by `#409` replaces that
synthetic surface with the TypeScript tracker.

## Preflight

Run this before retrying `#372`:

```sh
tmp="$(mktemp -d)"
gh repo clone ncmlabs/forge-playground "$tmp/forge-playground"
cd "$tmp/forge-playground"

test "$(git branch --show-current)" = "main"
test -f package.json
test -f package-lock.json
test -f src/server/app.ts
test -f src/client/App.tsx
test -f data/proof-runs.seed.json

npm ci
npm run typecheck
npm test
npm run build
```

Also confirm the proof queue is clean:

```sh
gh issue list --repo ncmlabs/forge-playground \
  --state open \
  --label clone-dev \
  --json number,title,labels
```

Expected result: exactly 10 open `clone-dev` issues, all app-specific and
distributed across `clone-dev:review`, `clone-dev:impl`, `clone-dev:plan`,
`clone-dev:security`, and `clone-dev:ops`.

If any command fails, do not start the proof run. Fix the playground baseline,
queue, or proof-run config first.

## Runtime Config

Use the playground config in that repository:

```sh
export FORGE_CLONEDEV_CONFIG=/path/to/forge-playground/.forge/clone-dev.toml
```

The relevant default is:

```toml
[defaults]
test_cmd = "npm run typecheck && npm test && npm run build"
```

That command is intentionally TypeScript-specific and must stay aligned with
the reset app and the reseeded issue queue.
