#!/usr/bin/env bash
# install-sensei.sh — Install forge-sensei to ~/.forge/sensei/
#
# Creates a production installation with:
#   ~/.forge/bin/forge-sensei    — wrapper script (on PATH)
#   ~/.forge/sensei/             — knowledge store, config, state
#
# Usage: bash scripts/install-sensei.sh [--skip-pretrain] [--force]
set -euo pipefail

FORGE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
INSTALL_DIR="$HOME/.forge/sensei"
BIN_DIR="$HOME/.forge/bin"
SKIP_PRETRAIN=false
FORCE=false

for arg in "$@"; do
  case "$arg" in
    --skip-pretrain) SKIP_PRETRAIN=true ;;
    --force) FORCE=true ;;
  esac
done

echo "=== Installing forge-sensei ==="
echo "  Binary:    $BIN_DIR/forge-sensei"
echo "  Runtime:   $INSTALL_DIR/"
echo ""

# ── 1. Create directories ────────────────────────────────────
mkdir -p "$INSTALL_DIR" "$BIN_DIR"

# ── 2. Build the binary ──────────────────────────────────────
echo "Building forge-sensei..."
# Concatenate multi-file project (workaround for multi-file checker issue)
COMBINED="/tmp/sensei-combined-$$.forge"
cat "$FORGE_ROOT/workflows/forge-sensei/core.forge" \
    "$FORGE_ROOT/workflows/forge-sensei/agent.forge" > "$COMBINED"

cargo run --manifest-path "$FORGE_ROOT/Cargo.toml" -- build \
  "$COMBINED" \
  -o "$INSTALL_DIR/forge-sensei-bin"
rm -f "$COMBINED"

echo "  Binary built: $INSTALL_DIR/forge-sensei-bin"

# ── 3. Create wrapper script ─────────────────────────────────
cat > "$BIN_DIR/forge-sensei" <<'WRAPPER'
#!/bin/sh
# forge-sensei wrapper — sets app-specific config path
FORGE_APP_CONFIG="$HOME/.forge/sensei/config.toml" \
  exec "$HOME/.forge/sensei/forge-sensei-bin" "$@"
WRAPPER
chmod +x "$BIN_DIR/forge-sensei"
echo "  Wrapper: $BIN_DIR/forge-sensei"

# ── 4. Create sensei config if not exists ─────────────────────
if [ ! -f "$INSTALL_DIR/config.toml" ] || [ "$FORCE" = true ]; then
  cat > "$INSTALL_DIR/config.toml" <<'TOML'
# forge-sensei LLM configuration
# This config is independent of the forge compiler config.
# Edit to point to your preferred LLM provider.

[llm]
default = "ollama"

[llm.routing]
fast     = "ollama"
balanced = "ollama"

[llm.budget]
max_cost_usd     = 0.00
max_total_tokens = 100000

# ── Ollama (local GPU) ──────────────────────────────────────
# Change base_url to match your Ollama server
[providers.ollama]
type         = "openai-compat"
model        = "gemma4:e4b"
base_url     = "http://localhost:11434/v1"
api_key      = "not-required"
timeout_secs = 120
quality_tier = "balanced"

# ── Anthropic (cloud, optional) ─────────────────────────────
# Uncomment and set your API key to use Claude
# [providers.anthropic]
# type    = "anthropic"
# model   = "claude-sonnet-4-20250514"
# api_key = "${ANTHROPIC_API_KEY}"
TOML
  echo "  Config: $INSTALL_DIR/config.toml"
  echo "  (edit this file to configure your LLM provider)"
else
  echo "  Config: $INSTALL_DIR/config.toml (kept existing)"
fi

# ── 5. Run pre-training ──────────────────────────────────────
if [ "$SKIP_PRETRAIN" = false ]; then
  echo ""
  echo "Pre-training curriculum..."
  SENSEI_BIN="$BIN_DIR/forge-sensei" bash "$FORGE_ROOT/scripts/pretrain-toolkit.sh" --force
else
  echo ""
  echo "Skipping pre-training (use --force to re-run later)"
fi

# ── 6. Add ~/.forge/bin to PATH ──────────────────────────────
SHELL_RC=""
if [ -f "$HOME/.zshrc" ]; then
  SHELL_RC="$HOME/.zshrc"
elif [ -f "$HOME/.bashrc" ]; then
  SHELL_RC="$HOME/.bashrc"
elif [ -f "$HOME/.profile" ]; then
  SHELL_RC="$HOME/.profile"
fi

if [ -n "$SHELL_RC" ] && ! grep -q '\.forge/bin' "$SHELL_RC" 2>/dev/null; then
  echo '' >> "$SHELL_RC"
  echo '# FORGE tools' >> "$SHELL_RC"
  echo 'export PATH="$HOME/.forge/bin:$PATH"' >> "$SHELL_RC"
  echo ""
  echo "Added ~/.forge/bin to PATH in $SHELL_RC"
  echo "Run: source $SHELL_RC (or open a new terminal)"
fi

# ── 7. Summary ───────────────────────────────────────────────
echo ""
echo "=== Installation complete ==="
echo ""
echo "  Binary:    ~/.forge/bin/forge-sensei"
echo "  Config:    ~/.forge/sensei/config.toml"
echo "  Knowledge: ~/.forge/sensei/knowledge.json"
echo ""
echo "Commands:"
echo "  forge-sensei status                 — check mastery level"
echo "  forge-sensei query \"question\"       — ask about FORGE"
echo "  forge-sensei review \"code\"          — review FORGE code"
echo ""
echo "To reconfigure LLM provider:"
echo "  edit ~/.forge/sensei/config.toml"
