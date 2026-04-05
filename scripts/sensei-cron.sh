#!/usr/bin/env bash
# sensei-cron.sh — Scheduled maintenance for forge-sensei
# Refreshes knowledge base and runs mastery assessment.
#
# Add to crontab:
#   0 6 * * * bash /path/to/forge/scripts/sensei-cron.sh >> /path/to/forge/.forge-knowledge/cron.log 2>&1
#
# Or use Claude Code schedule skill:
#   /schedule "forge-sensei maintenance" --cron "0 6 * * *" --command "bash scripts/sensei-cron.sh"
set -euo pipefail

FORGE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
source ~/.zshrc 2>/dev/null || true  # for ANTHROPIC_API_KEY

echo "[$(date)] forge-sensei scheduled maintenance starting"

# Build binary if needed
if [ ! -x "$FORGE_ROOT/bin/forge-sensei" ]; then
  echo "Building forge-sensei..."
  bash "$FORGE_ROOT/scripts/build-sensei.sh"
fi

# Re-train knowledge base
echo "Phase 1: Refreshing knowledge base..."
bash "$FORGE_ROOT/scripts/pretrain-sensei.sh" --force

# Run mastery assessment
echo "Phase 2: Running mastery assessment..."
bash ~/.claude/skills/forge-sensei/assess.sh

# Report
echo ""
"$FORGE_ROOT/bin/forge-sensei" status 2>/dev/null || true
echo "[$(date)] forge-sensei scheduled maintenance complete"
