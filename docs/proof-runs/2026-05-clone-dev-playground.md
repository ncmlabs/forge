# 2026-05 Clone-Dev Playground Proof Run

This note pins the target shape for the T11.3 proof run tracked by
`ncmlabs/forge#372`.

## Target

The proof-run target is `ncmlabs/forge-playground` as a Rust crate on the
`main` branch. The 10 seeded proof issues in `ncmlabs/forge-playground` #13-#22
are Rust tasks and intentionally reference `Cargo.toml`, `src/math.rs`,
`src/routes.rs`, `cargo test`, `cargo doc`, `axum`, `tokio`, `serde`, and
`serde_json`.

Do not reseed those issues as Go tasks unless the proof target is explicitly
changed in a later issue.

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

The halted `2026-05-18-run1` attempt failed because the GitHub default branch
was still `test/clone-dev-e2e`, which contained the older Go playground. The
tester then ran `cargo test --quiet` in a checkout with no `Cargo.toml`.

## Preflight

Run this before retrying #372:

```sh
tmp="$(mktemp -d)"
gh repo clone ncmlabs/forge-playground "$tmp/forge-playground"
cd "$tmp/forge-playground"

test "$(git branch --show-current)" = "main"
test -f Cargo.toml
test -f src/math.rs
test -f src/routes.rs

cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --quiet
cargo doc --no-deps
```

If any command fails, do not start the proof run. Fix the playground baseline or
the proof-run config first.

## Runtime Config

Use the playground config in that repository:

```sh
export FORGE_CLONEDEV_CONFIG=/path/to/forge-playground/.forge/clone-dev.toml
```

The relevant default is:

```toml
[defaults]
test_cmd = "cargo test --quiet"
```

That command is intentionally Rust-specific and must stay aligned with the
seeded issue set.
