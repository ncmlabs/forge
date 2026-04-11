#!/usr/bin/env bash
# install-sensei-server.sh — Install/start the macOS LaunchAgent for forge-sensei-server.
set -euo pipefail

LABEL="com.ncmlabs.forge-sensei"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
BIN="$HOME/.forge/bin/forge-sensei-server"
CONFIG="$HOME/.forge/sensei/config.toml"
LOG_DIR="$HOME/.forge/sensei/logs"

usage() {
  echo "Usage: $0 {install|start|stop|restart|status|uninstall}"
}

cmd="${1:-install}"
mkdir -p "$(dirname "$PLIST")" "$LOG_DIR"

case "$cmd" in
  install)
    if [ ! -x "$BIN" ]; then
      echo "Error: $BIN not found. Run: bash scripts/install-sensei.sh --skip-pretrain"
      exit 1
    fi
    cat > "$PLIST" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>$LABEL</string>
  <key>ProgramArguments</key>
  <array>
    <string>$BIN</string>
    <string>--host</string>
    <string>127.0.0.1</string>
    <string>--port</string>
    <string>3000</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>FORGE_CONFIG</key>
    <string>$CONFIG</string>
  </dict>
  <key>WorkingDirectory</key>
  <string>$HOME/.forge/sensei</string>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>$LOG_DIR/server.out.log</string>
  <key>StandardErrorPath</key>
  <string>$LOG_DIR/server.err.log</string>
</dict>
</plist>
PLIST
    launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
    launchctl bootstrap "gui/$(id -u)" "$PLIST"
    launchctl kickstart -k "gui/$(id -u)/$LABEL"
    echo "Installed and started $LABEL"
    ;;
  start)
    launchctl bootstrap "gui/$(id -u)" "$PLIST" 2>/dev/null || true
    launchctl kickstart -k "gui/$(id -u)/$LABEL"
    ;;
  stop)
    launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
    ;;
  restart)
    launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
    launchctl bootstrap "gui/$(id -u)" "$PLIST"
    launchctl kickstart -k "gui/$(id -u)/$LABEL"
    ;;
  status)
    launchctl print "gui/$(id -u)/$LABEL"
    ;;
  uninstall)
    launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
    rm -f "$PLIST"
    echo "Uninstalled $LABEL"
    ;;
  *)
    usage
    exit 1
    ;;
esac
