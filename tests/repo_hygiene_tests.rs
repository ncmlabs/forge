// Repository hygiene (#479 review): a test run must not leave runtime state in
// the repository. Agents declare `knowledge store: ".forge-knowledge/..."`
// relative to the cwd, so running the dev-cycle programs (or the clone-dev
// tests) from the repo root used to write `.forge-knowledge/pr-history/
// knowledge.json` into `git status` — and one copy was committed.

use std::process::Command;

fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn runtime_state_is_not_tracked_and_stays_ignored() {
    let tracked = Command::new("git")
        .args(["ls-files", ".forge-knowledge", ".forge-data"])
        .current_dir(repo_root())
        .output()
        .expect("failed to run git ls-files");
    assert!(
        tracked.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&tracked.stderr)
    );
    let tracked = String::from_utf8_lossy(&tracked.stdout);
    assert!(
        tracked.trim().is_empty(),
        "runtime state must never be committed: {tracked}"
    );

    // Both directories are ignored, so `git add -A` cannot pick the state up
    // again even when a test run recreates it.
    let ignore = std::fs::read_to_string(repo_root().join(".gitignore")).expect(".gitignore");
    for dir in [".forge-knowledge/", ".forge-data/"] {
        assert!(
            ignore.lines().any(|line| line.trim() == dir),
            ".gitignore must list {dir}: {ignore}"
        );
    }
}
