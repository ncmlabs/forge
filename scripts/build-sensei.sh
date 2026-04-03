#!/usr/bin/env bash
# build-sensei.sh — Build the forge-sensei agent as a standalone binary
set -euo pipefail

FORGE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
mkdir -p "$FORGE_ROOT/bin"

echo "Building forge-sensei..."
cargo run --manifest-path "$FORGE_ROOT/Cargo.toml" -- build \
  "$FORGE_ROOT/workflows/forge-sensei.forge" \
  -o "$FORGE_ROOT/bin/forge-sensei"

echo ""
echo "Binary ready: bin/forge-sensei"
echo "Run: bin/forge-sensei --help"
