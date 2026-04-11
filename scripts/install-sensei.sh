#!/usr/bin/env bash
# install-sensei.sh — Install forge-sensei CLI + server to ~/.forge/sensei/
#
# Usage: bash scripts/install-sensei.sh [--skip-pretrain] [--force-config]
set -euo pipefail

FORGE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
INSTALL_DIR="$HOME/.forge/sensei"
BIN_DIR="$HOME/.forge/bin"
SKIP_PRETRAIN=false
FORCE_CONFIG=false

for arg in "$@"; do
  case "$arg" in
    --skip-pretrain) SKIP_PRETRAIN=true ;;
    --force-config) FORCE_CONFIG=true ;;
    --force)
      echo "Note: --force no longer overwrites config; use --force-config for that."
      ;;
  esac
done

echo "=== Installing forge-sensei ==="
echo "  CLI:      $BIN_DIR/forge-sensei"
echo "  Server:   $BIN_DIR/forge-sensei-server"
echo "  Runtime:  $INSTALL_DIR/"
echo ""

mkdir -p "$INSTALL_DIR" "$BIN_DIR"

echo "Building forge-sensei CLI..."
cargo run --manifest-path "$FORGE_ROOT/Cargo.toml" -- build \
  "$FORGE_ROOT/workflows/forge-sensei" \
  -o "$INSTALL_DIR/forge-sensei-bin"

echo "Building forge-sensei server..."
cargo run --manifest-path "$FORGE_ROOT/Cargo.toml" -- build \
  "$FORGE_ROOT/workflows/forge-sensei" \
  --entry web.forge \
  --source core.forge \
  --source agent.forge \
  -o "$INSTALL_DIR/forge-sensei-server-bin"

echo "  CLI binary:    $INSTALL_DIR/forge-sensei-bin"
echo "  Server binary: $INSTALL_DIR/forge-sensei-server-bin"

cat > "$BIN_DIR/forge-sensei" <<'WRAPPER'
#!/bin/sh
# forge-sensei wrapper — sets config path and supports server-backed CLI mode.
SENSEI_DIR="$HOME/.forge/sensei"
STATE_FILE="$SENSEI_DIR/state.json"
STALE_HOURS=24

if [ "$1" = "status" ] && [ -f "$SENSEI_DIR/knowledge.json" ]; then
  if [ -f "$STATE_FILE" ]; then
    last_ts=$(grep -o '"last_eval":"[^"]*"' "$STATE_FILE" 2>/dev/null | cut -d'"' -f4)
    if [ -n "$last_ts" ]; then
      last_epoch=$(date -j -f "%Y-%m-%dT%H:%M:%S" "$last_ts" +%s 2>/dev/null || echo 0)
      now_epoch=$(date +%s)
      hours_since=$(( (now_epoch - last_epoch) / 3600 ))
      if [ "$hours_since" -gt "$STALE_HOURS" ]; then
        echo "Note: last evaluation was ${hours_since}h ago. Consider running: bash scripts/pretrain-toolkit.sh --verify-only"
      fi
    fi
  else
    echo "Note: no evaluation recorded yet. Run: bash scripts/pretrain-toolkit.sh --verify-only"
  fi
fi

FORGE_CONFIG="${FORGE_CONFIG:-$SENSEI_DIR/config.toml}" \
  exec "$SENSEI_DIR/forge-sensei-bin" "$@"
WRAPPER
chmod +x "$BIN_DIR/forge-sensei"

cat > "$BIN_DIR/forge-sensei-server" <<'WRAPPER'
#!/bin/sh
# forge-sensei-server wrapper — runs the long-lived HTTP daemon.
SENSEI_DIR="$HOME/.forge/sensei"
cd "$SENSEI_DIR"
FORGE_CONFIG="${FORGE_CONFIG:-$SENSEI_DIR/config.toml}" \
  exec "$SENSEI_DIR/forge-sensei-server-bin" "$@"
WRAPPER
chmod +x "$BIN_DIR/forge-sensei-server"

if [ ! -f "$INSTALL_DIR/config.toml" ] || [ "$FORCE_CONFIG" = true ]; then
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

[providers.ollama]
type         = "openai-compat"
model        = "gemma4:e4b"
base_url     = "http://localhost:11434/v1"
api_key      = "not-required"
timeout_secs = 120

[providers.ollama.capabilities]
quality_tier = "balanced"
TOML
  echo "  Config: $INSTALL_DIR/config.toml"
else
  echo "  Config: $INSTALL_DIR/config.toml (kept existing)"
fi

if [ "$SKIP_PRETRAIN" = false ]; then
  echo ""
  echo "Pre-training curriculum..."
  SENSEI_BIN="$BIN_DIR/forge-sensei" bash "$FORGE_ROOT/scripts/pretrain-toolkit.sh" --force
else
  echo ""
  echo "Skipping pre-training"
fi

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
fi

echo ""
echo "=== Installation complete ==="
echo "  CLI:       ~/.forge/bin/forge-sensei"
echo "  Server:    ~/.forge/bin/forge-sensei-server"
echo "  Config:    ~/.forge/sensei/config.toml"
echo "  Knowledge: ~/.forge/sensei/knowledge.json"
echo ""
echo "Server mode:"
echo "  forge-sensei-server --host 127.0.0.1 --port 3000"
echo "  FORGE_SENSEI_SERVER=http://127.0.0.1:3000 forge-sensei status"
