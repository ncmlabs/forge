#!/usr/bin/env bash
# install-sensei-server.sh — Thin wrapper around the cross-platform
# StartupManager built into forge-sensei-server (issue #254).
# Retained for backward compatibility with existing docs and tooling.
set -euo pipefail

LABEL="com.ncmlabs.forge-sensei"
BIN="$HOME/.forge/bin/forge-sensei-server"

usage() {
  echo "Usage: $0 {install|start|stop|restart|status|uninstall}"
}

cmd="${1:-install}"

if [ ! -x "$BIN" ]; then
  echo "Error: $BIN not found. Run: bash scripts/install-sensei.sh --skip-pretrain"
  exit 1
fi

case "$cmd" in
  install)
    "$BIN" install-service --label "$LABEL"
    ;;
  start)
    "$BIN" service start --label "$LABEL"
    ;;
  stop)
    "$BIN" service stop --label "$LABEL"
    ;;
  restart)
    "$BIN" service stop --label "$LABEL" || true
    "$BIN" service start --label "$LABEL"
    ;;
  status)
    "$BIN" service status --label "$LABEL"
    ;;
  uninstall)
    "$BIN" service uninstall --label "$LABEL"
    ;;
  *)
    usage
    exit 1
    ;;
esac
