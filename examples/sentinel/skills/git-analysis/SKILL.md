---
name: git_analysis
description: Deep agentic analysis of git repository health and trends
allowed-tools: Bash
timeout: 60
---

# Git Repository Analysis

You are analyzing a git repository to produce a weekly health report.
Run commands to gather data, then synthesize findings.

## Steps

1. Run `git log --oneline -50` to see recent commit history
2. Run `git shortlog -sn --since='30 days ago'` for contributor activity
3. Run `git diff --stat HEAD~20` to see what files are changing most
4. Based on the results, probe deeper:
   - If many changes in one directory: `git log --oneline -10 -- <dir>`
   - If few recent commits: `git log --since='60 days ago' --oneline` for longer velocity view
5. Run `git branch -a` to check branch hygiene
6. Run `wc -l src/**/*.rs | sort -rn | head -10` for largest source files

## Output Format

Return a structured report with these sections:

### Summary
One-paragraph overall health assessment.

### Velocity
- Commits per week (last 4 weeks)
- Active contributors

### Hotspots
- Most-changed files (top 5)
- Directories with highest churn

### Branch Health
- Total branches
- Stale branches (no commits in 14+ days)

### Risk Areas
- Files with rapid change + few contributors
- Unusually large files

### Recommendations
- Top 3 actionable items to improve repo health
