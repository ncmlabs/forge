---
name: repo_check
description: Runs git commands to report real repository data
allowed-tools: Bash
timeout: 30
---

# Repo Check Skill

Run the following commands and report their EXACT output:

1. Run: `git rev-parse --abbrev-ref HEAD`
2. Run: `git log --oneline -3`
3. Run: `ls src/*.rs | wc -l`

Return your answer in this exact format:

BRANCH: <output of command 1>
LAST_3_COMMITS: <output of command 2>
RUST_FILES_IN_SRC: <output of command 3>
