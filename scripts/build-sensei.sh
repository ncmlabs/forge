#!/usr/bin/env bash
# build-sensei.sh — Build the forge-sensei agent as a standalone binary
set -euo pipefail

FORGE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# Multi-file project: use directory with forge.project.toml
SENSEI_SOURCE="$FORGE_ROOT/workflows/forge-sensei"
# Fallback to single-file if directory doesn't exist
if [ ! -d "$SENSEI_SOURCE" ]; then
  SENSEI_SOURCE="$FORGE_ROOT/workflows/forge-sensei.forge"
fi
SENSEI_BIN="$FORGE_ROOT/bin/forge-sensei"
SENSEI_SERVER_BIN="$FORGE_ROOT/bin/forge-sensei-server"
HASH_FILE="$FORGE_ROOT/bin/.sensei-build-hash"

# ── Prerequisites ─────────────────────────────────────────────
if ! command -v cargo &>/dev/null; then
  echo "Error: cargo not found. Install Rust: https://rustup.rs"
  exit 1
fi

# ── Skip-if-unchanged ────────────────────────────────────────
if [ -d "$SENSEI_SOURCE" ]; then
  CURRENT_HASH=$(find "$SENSEI_SOURCE" -name '*.forge' -o -name '*.toml' | sort | xargs cat | shasum -a 256 | cut -d' ' -f1)
else
  CURRENT_HASH=$(shasum -a 256 "$SENSEI_SOURCE" | cut -d' ' -f1)
fi
if [ -f "$HASH_FILE" ] && [ -x "$SENSEI_BIN" ] && [ -x "$SENSEI_SERVER_BIN" ]; then
  CACHED_HASH=$(cat "$HASH_FILE")
  if [ "$CURRENT_HASH" = "$CACHED_HASH" ]; then
    echo "Binary up to date (source unchanged)."
    exit 0
  fi
fi

# ── Build ─────────────────────────────────────────────────────
mkdir -p "$FORGE_ROOT/bin"
echo "Building forge-sensei..."
START=$SECONDS

cargo run --manifest-path "$FORGE_ROOT/Cargo.toml" -- build \
  "$SENSEI_SOURCE" \
  -o "$SENSEI_BIN"

cargo run --manifest-path "$FORGE_ROOT/Cargo.toml" -- build \
  "$SENSEI_SOURCE" \
  --entry web.forge \
  --source core.forge \
  --source agent.forge \
  -o "$SENSEI_SERVER_BIN"

ELAPSED=$((SECONDS - START))
echo "$CURRENT_HASH" > "$HASH_FILE"

# ── Smoke test ────────────────────────────────────────────────
if "$SENSEI_BIN" status &>/dev/null; then
  echo ""
  echo "Binaries ready: bin/forge-sensei, bin/forge-sensei-server (${ELAPSED}s)"
  echo "Run: bin/forge-sensei --help"
else
  echo ""
  echo "Binaries ready: bin/forge-sensei, bin/forge-sensei-server (${ELAPSED}s)"
  echo "Note: smoke test returned non-zero (may need pretrain first)"
fi
