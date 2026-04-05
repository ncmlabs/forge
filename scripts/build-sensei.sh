#!/usr/bin/env bash
# build-sensei.sh — Build the forge-sensei agent as a standalone binary
set -euo pipefail

FORGE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SENSEI_SOURCE="$FORGE_ROOT/workflows/forge-sensei.forge"
SENSEI_BIN="$FORGE_ROOT/bin/forge-sensei"
HASH_FILE="$FORGE_ROOT/bin/.sensei-build-hash"

# ── Prerequisites ─────────────────────────────────────────────
if ! command -v cargo &>/dev/null; then
  echo "Error: cargo not found. Install Rust: https://rustup.rs"
  exit 1
fi

# ── Skip-if-unchanged ────────────────────────────────────────
CURRENT_HASH=$(shasum -a 256 "$SENSEI_SOURCE" | cut -d' ' -f1)
if [ -f "$HASH_FILE" ] && [ -x "$SENSEI_BIN" ]; then
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

ELAPSED=$((SECONDS - START))
echo "$CURRENT_HASH" > "$HASH_FILE"

# ── Smoke test ────────────────────────────────────────────────
if "$SENSEI_BIN" status &>/dev/null; then
  echo ""
  echo "Binary ready: bin/forge-sensei (${ELAPSED}s)"
  echo "Run: bin/forge-sensei --help"
else
  echo ""
  echo "Binary ready: bin/forge-sensei (${ELAPSED}s)"
  echo "Note: smoke test returned non-zero (may need pretrain first)"
fi
