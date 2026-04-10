---
name: repo_check
description: Runs a single git command to prove real tool execution
allowed-tools: Bash
timeout: 30
---

# Repo Check Skill

Run this single command using the bash_exec tool:

```
git log --oneline -3
```

Return ONLY the raw output of the command. Nothing else.
