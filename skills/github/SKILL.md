---
name: github
description: GitHub operations via the gh CLI — issues, PRs, branches, and CI checks.
timeout: 120
allowed-tools:
  - Bash(gh:*)
capabilities:
  - name: create_issue
    inputs: [Text, Text, Text]
    output: Text
  - name: list_issues
    inputs: [Text, Text]
    output: Text
  - name: create_branch
    inputs: [Text, Text]
    output: Text
  - name: create_pr
    inputs: [Text, Text, Text, Text]
    output: Text
  - name: check_ci
    inputs: [Text, Text]
    output: Text
  - name: merge_pr
    inputs: [Text, Text]
    output: Text
  - name: delete_branch
    inputs: [Text, Text]
    output: Text
  - name: close_issue
    inputs: [Text, Text]
    output: Text
---

# GitHub Skill

Use this skill to perform GitHub operations via the `gh` CLI.
Only run commands that start with `gh`. Do not run arbitrary shell commands.

## Prerequisites

The `gh` CLI must be installed and authenticated. Before any operation, verify:

```bash
gh auth status
```

If this fails, return `"ERROR: gh not authenticated. Run 'gh auth login' to authenticate."` and stop.

## Capabilities

### `create_issue(repo, title, body)`

Create a new issue on a GitHub repository.

```bash
gh issue create -R "$repo" --title "$title" --body "$body"
```

Return the issue URL printed by `gh` on success.

To add labels, append `--label "bug,urgent"`. To assign, append `--assignee "@me"`.

### `list_issues(repo, filter)`

List issues with optional filtering. The `filter` parameter accepts space-separated flags.

```bash
gh issue list -R "$repo" --json number,title,state,labels,assignees --limit 30
```

Common filters (append to command):
- `--state open` or `--state closed`
- `--label "bug"`
- `--assignee "@me"`
- `--search "query"`

Return the JSON array output.

### `create_branch(repo, branch_name)`

Create a new branch from the default branch HEAD.

```bash
gh api "repos/$repo/git/refs" \
  -f "ref=refs/heads/$branch_name" \
  -f "sha=$(gh api "repos/$repo/git/ref/heads/main" -q '.object.sha')"
```

If the default branch is not `main`, first determine it:

```bash
gh repo view "$repo" --json defaultBranchRef -q '.defaultBranchRef.name'
```

Return confirmation with the branch name and SHA on success.

### `create_pr(repo, branch, title, body)`

Create a pull request from the given branch to the default branch.

```bash
gh pr create -R "$repo" --head "$branch" --title "$title" --body "$body"
```

To target a different base branch, append `--base "development"`.

Return the PR URL printed by `gh` on success.

### `check_ci(repo, ref)`

Check CI status for a PR number or git ref.

For a PR number:

```bash
gh pr checks "$ref" -R "$repo" --json name,state,conclusion
```

For a commit SHA:

```bash
gh run list -R "$repo" --commit "$ref" --json status,conclusion,name --limit 10
```

Return the JSON output. Summarize pass/fail counts if multiple checks exist.

### `merge_pr(repo, pr_number)`

Merge a pull request.

First verify the PR is mergeable:

```bash
gh pr view "$pr_number" -R "$repo" --json mergeable,mergeStateStatus -q '.mergeable'
```

If mergeable, proceed:

```bash
gh pr merge "$pr_number" -R "$repo" --merge
```

Use `--merge` by default. Only use `--squash` or `--rebase` if the caller specifies it in the arguments.

Return the merge result message.

### `delete_branch(repo, branch_name)`

Delete a remote branch.

First verify the branch exists and is not the default branch:

```bash
gh api "repos/$repo/git/ref/heads/$branch_name" -q '.ref'
```

If it exists and is not the default branch:

```bash
gh api -X DELETE "repos/$repo/git/refs/heads/$branch_name"
```

Return confirmation or error.

### `close_issue(repo, issue_number)`

Close an issue.

```bash
gh issue close "$issue_number" -R "$repo"
```

To close with a comment:

```bash
gh issue close "$issue_number" -R "$repo" --comment "Closing: resolved"
```

Return confirmation message.

## Error Handling

### Authentication failures

If `gh auth status` returns non-zero or any command returns `"not logged in"`:

Return `"ERROR: gh not authenticated. Run 'gh auth login' to authenticate."` — do not retry.

### Rate limits

If a command returns `"API rate limit exceeded"` or HTTP 403 with rate-limit headers:

Return `"ERROR: GitHub API rate limit exceeded. Retry after rate limit resets."` — do not retry.

### Not found

If a command returns `"Could not resolve"`, `"not found"`, or HTTP 404:

Return `"ERROR: Resource not found — verify the repo, issue number, or branch name."` — do not guess alternatives.

### Merge conflicts

If `gh pr merge` fails with `"not mergeable"`, `"merge conflict"`, or `"blocked"`:

Return `"ERROR: PR cannot be merged — conflicts or failing checks. Resolve before retrying."` — do not force merge.

## Safety Rules

- Only run commands starting with `gh`. No arbitrary shell commands.
- Never force-push or delete the default branch.
- Always verify a resource exists before attempting destructive operations (delete_branch, merge_pr).
- Use `--merge` strategy by default. Only use `--squash` or `--rebase` when explicitly requested.
- Return the full `gh` command that was run alongside the result for auditability.
- Do not store or log authentication tokens.

## Session Adapter Mapping

| FORGE session field | gh CLI |
|---|---|
| `repo` | `-R "$repo"` flag on all commands |
| `output_mode = json` | `--json` flag for structured output |
| `timeout` | Managed by FORGE skill executor |
| `auth` | `gh auth status` pre-check |

## AgentResult Mapping

| gh output | AgentResult field |
|---|---|
| Issue/PR URL | `output` |
| JSON list | `output` (structured) |
| Error message | `output` (prefixed with `ERROR:`) |
| Command executed | `metadata.command` |
