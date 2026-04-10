// FORGE sandbox isolation — issue #194
// Creates and cleans up git worktrees for agent/session filesystem isolation.
// Uses sync std::process::Command — these are fast internal infra ops, not user-facing commands.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Slug a branch name for use as a filesystem directory name.
/// `"feature/fix-123"` → `"feature-fix-123"`
pub fn branch_slug(branch: &str) -> String {
    branch.replace(['/', '\\'], "-")
}

/// Compute the worktree path: `.forge-data/worktrees/{slug}/`
pub fn worktree_path(branch: &str) -> PathBuf {
    Path::new(".forge-data")
        .join("worktrees")
        .join(branch_slug(branch))
}

/// Create a git worktree for the given branch.
/// If the branch does not exist, creates it from HEAD with `-b`.
/// Returns the absolute path to the worktree directory.
pub fn create_worktree(branch: &str) -> Result<PathBuf, String> {
    let wt_path = worktree_path(branch);

    // Collision check — another agent may already be using this branch
    if wt_path.exists() {
        return Err(format!(
            "worktree path already exists: {} — another agent may be using branch '{}'",
            wt_path.display(),
            branch
        ));
    }

    // Ensure parent directory exists
    if let Some(parent) = wt_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create worktree parent dir: {}", e))?;
    }

    let wt_str = wt_path.to_string_lossy().to_string();

    // Try to add worktree with existing branch first
    let output = Command::new("git")
        .args(["worktree", "add", &wt_str, branch])
        .output()
        .map_err(|e| format!("failed to run git worktree add: {}", e))?;

    if !output.status.success() {
        // Branch might not exist — create it with -b
        let output2 = Command::new("git")
            .args(["worktree", "add", "-b", branch, &wt_str])
            .output()
            .map_err(|e| format!("failed to run git worktree add -b: {}", e))?;

        if !output2.status.success() {
            return Err(format!(
                "git worktree add failed: {}",
                String::from_utf8_lossy(&output2.stderr).trim()
            ));
        }
    }

    // Return absolute path
    std::fs::canonicalize(&wt_path).map_err(|e| format!("failed to resolve worktree path: {}", e))
}

/// Remove a git worktree and clean up the directory.
pub fn remove_worktree(branch: &str) -> Result<(), String> {
    let wt_path = worktree_path(branch);

    if !wt_path.exists() {
        return Ok(()); // Already cleaned up
    }

    let wt_str = wt_path.to_string_lossy().to_string();

    let output = Command::new("git")
        .args(["worktree", "remove", "--force", &wt_str])
        .output()
        .map_err(|e| format!("failed to run git worktree remove: {}", e))?;

    if !output.status.success() {
        // Fallback: manually remove directory and prune
        let _ = std::fs::remove_dir_all(&wt_path);
        let _ = Command::new("git").args(["worktree", "prune"]).output();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_slug_replaces_slashes() {
        assert_eq!(branch_slug("feature/fix-123"), "feature-fix-123");
        assert_eq!(branch_slug("main"), "main");
        assert_eq!(branch_slug("a/b/c"), "a-b-c");
        assert_eq!(branch_slug("back\\slash"), "back-slash");
    }

    #[test]
    fn worktree_path_uses_slug() {
        let path = worktree_path("feature/fix-123");
        assert_eq!(path, PathBuf::from(".forge-data/worktrees/feature-fix-123"));
    }

    #[test]
    fn worktree_path_simple_branch() {
        let path = worktree_path("main");
        assert_eq!(path, PathBuf::from(".forge-data/worktrees/main"));
    }
}
