#!/usr/bin/env bash
# build-sensei.sh — Build the forge-sensei agent as a standalone binary
set -euo pipefail

FORGE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SENSEI_ROOT="$FORGE_ROOT/workflows/forge-sensei"
SENSEI_CLIENT_SOURCE="$SENSEI_ROOT/client"
SENSEI_SERVER_SOURCE="$SENSEI_ROOT/server"
SENSEI_BIN="$FORGE_ROOT/bin/forge-sensei"
SENSEI_SERVER_BIN="$FORGE_ROOT/bin/forge-sensei-server"
HASH_FILE="$FORGE_ROOT/bin/.sensei-build-hash"

# ── Prerequisites ─────────────────────────────────────────────
if ! command -v cargo &>/dev/null; then
  echo "Error: cargo not found. Install Rust: https://rustup.rs"
  exit 1
fi

# ── Skip-if-unchanged ────────────────────────────────────────
CURRENT_HASH=$(find "$SENSEI_ROOT" \( -name '*.forge' -o -name '*.toml' \) -type f | sort | xargs cat | shasum -a 256 | cut -d' ' -f1)
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
  "$SENSEI_SERVER_SOURCE" \
  -o "$SENSEI_SERVER_BIN"

cargo run --manifest-path "$FORGE_ROOT/Cargo.toml" -- build \
  "$SENSEI_CLIENT_SOURCE" \
  -o "$SENSEI_BIN"

ELAPSED=$((SECONDS - START))
echo "$CURRENT_HASH" > "$HASH_FILE"

# ── Smoke test ────────────────────────────────────────────────
if "$SENSEI_BIN" --help &>/dev/null; then
  echo ""
  echo "Binaries ready: bin/forge-sensei, bin/forge-sensei-server (${ELAPSED}s)"
  echo "Run: bin/forge-sensei --help"
else
  echo ""
  echo "Binaries ready: bin/forge-sensei, bin/forge-sensei-server (${ELAPSED}s)"
  echo "Note: CLI help smoke test returned non-zero"
fi
