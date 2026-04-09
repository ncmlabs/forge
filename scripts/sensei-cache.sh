#!/usr/bin/env bash
# sensei-cache.sh — Manage forge-sensei caches and knowledge store
set -euo pipefail

FORGE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
KNOWLEDGE_DIR="$FORGE_ROOT/.forge-knowledge"
HOOK_CACHE="/tmp/forge-sensei-cache"

case "${1:-help}" in
  clean)
    echo "Cleaning caches..."
    rm -rf "$HOOK_CACHE"
    rm -f "$FORGE_ROOT/bin/.sensei-build-hash"
    rm -f "$KNOWLEDGE_DIR/pretrain-manifest.sha256"
    echo "Done. Hook cache, build hash, and pretrain manifest cleared."
    ;;
  reset)
    echo "Resetting knowledge store..."
    rm -rf "$KNOWLEDGE_DIR/sensei"
    rm -rf "$KNOWLEDGE_DIR/specialist"
    rm -rf "$HOOK_CACHE"
    rm -f "$FORGE_ROOT/bin/.sensei-build-hash"
    rm -f "$KNOWLEDGE_DIR/pretrain-manifest.sha256"
    echo "Done. Knowledge store and all caches cleared."
    echo "Run: bash scripts/pretrain-sensei.sh to re-initialize."
    ;;
  stats)
    echo "=== forge-sensei Cache Stats ==="
    if [ -d "$KNOWLEDGE_DIR/sensei" ]; then
      STORE="$KNOWLEDGE_DIR/sensei/knowledge.json"
      if [ -f "$STORE" ]; then
        if command -v jq &>/dev/null; then
          ENTRIES=$(jq 'length' "$STORE" 2>/dev/null || echo "?")
        else
          ENTRIES="?(jq not found)"
        fi
        SIZE=$(du -sh "$STORE" 2>/dev/null | cut -f1)
        echo "Knowledge store: $ENTRIES entries ($SIZE)"
      else
        echo "Knowledge store: empty (no knowledge.json)"
      fi
    else
      echo "Knowledge store: not initialized"
    fi
    if [ -d "$HOOK_CACHE" ]; then
      CACHE_COUNT=$(ls -1 "$HOOK_CACHE" 2>/dev/null | wc -l | tr -d ' ')
      echo "Hook cache: $CACHE_COUNT entries"
    else
      echo "Hook cache: empty"
    fi
    if [ -f "$KNOWLEDGE_DIR/assessment-history.jsonl" ]; then
      ASSESSMENTS=$(wc -l < "$KNOWLEDGE_DIR/assessment-history.jsonl" | tr -d ' ')
      if command -v jq &>/dev/null; then
        LAST=$(tail -1 "$KNOWLEDGE_DIR/assessment-history.jsonl" | jq -r '.timestamp' 2>/dev/null || echo "?")
      else
        LAST="?(jq not found)"
      fi
      echo "Assessments: $ASSESSMENTS runs (last: $LAST)"
    else
      echo "Assessments: none recorded"
    fi
    ;;
  help|*)
    echo "Usage: bash scripts/sensei-cache.sh <command>"
    echo ""
    echo "Commands:"
    echo "  clean   Remove hook cache, build hash, pretrain manifest"
    echo "  reset   Full wipe: knowledge store + all caches"
    echo "  stats   Show knowledge store size, cache entries, assessment history"
    ;;
esac
