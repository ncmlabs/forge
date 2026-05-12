#!/usr/bin/env bash
# ngrok-bg.sh — manage a backgrounded ngrok HTTP tunnel for local clone-dev /
# Slack-interactivity testing. Writes pid + url to /tmp so other scripts can
# read them; ngrok's local inspector (127.0.0.1:4040) is the source of truth.
#
# Usage:
#   scripts/ngrok-bg.sh start [PORT] [--domain DOMAIN]   # default PORT=3300
#   scripts/ngrok-bg.sh stop
#   scripts/ngrok-bg.sh status
#   scripts/ngrok-bg.sh url                              # public_url only
#   scripts/ngrok-bg.sh webhook-url [PATH]               # public_url + PATH, default /webhook/approval
#   scripts/ngrok-bg.sh restart [PORT] [--domain DOMAIN]
#   scripts/ngrok-bg.sh log [N]                          # tail last N lines (default 50)

set -euo pipefail

PID_FILE="/tmp/ngrok-bg.pid"
LOG_FILE="/tmp/ngrok-bg.log"
URL_FILE="/tmp/ngrok-bg.url"
PORT_FILE="/tmp/ngrok-bg.port"
INSPECTOR="http://127.0.0.1:4040"

require_ngrok() {
  if ! command -v ngrok >/dev/null 2>&1; then
    echo "ngrok not found on PATH. Install with: brew install ngrok" >&2
    exit 127
  fi
}

require_jq() {
  if ! command -v jq >/dev/null 2>&1; then
    echo "jq not found on PATH. Install with: brew install jq" >&2
    exit 127
  fi
}

is_running() {
  [ -f "$PID_FILE" ] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null
}

inspector_alive() {
  curl -sf -o /dev/null --max-time 2 "$INSPECTOR/api/tunnels"
}

fetch_url() {
  curl -sf --max-time 2 "$INSPECTOR/api/tunnels" 2>/dev/null \
    | jq -r '.tunnels[0].public_url // empty' 2>/dev/null
}

wait_for_url() {
  local url=""
  for _ in $(seq 1 30); do
    url="$(fetch_url || true)"
    if [ -n "$url" ]; then
      echo "$url" > "$URL_FILE"
      printf "%s" "$url"
      return 0
    fi
    sleep 0.3
  done
  return 1
}

cmd_start() {
  require_ngrok
  require_jq

  local port="3300"
  local domain=""
  while [ $# -gt 0 ]; do
    case "$1" in
      --domain)
        domain="${2:-}"
        shift 2
        ;;
      --domain=*)
        domain="${1#--domain=}"
        shift
        ;;
      *)
        if [[ "$1" =~ ^[0-9]+$ ]]; then
          port="$1"
        else
          echo "unknown arg: $1" >&2
          exit 64
        fi
        shift
        ;;
    esac
  done

  if is_running; then
    local existing
    existing="$(fetch_url || true)"
    if [ -n "$existing" ]; then
      echo "ngrok already running (pid $(cat "$PID_FILE")) — url: $existing"
      return 0
    fi
    echo "stale pid file detected — clearing"
    rm -f "$PID_FILE"
  fi

  if inspector_alive; then
    local foreign
    foreign="$(fetch_url || true)"
    echo "another ngrok is listening on $INSPECTOR (url: ${foreign:-?})" >&2
    echo "stop it (ngrok-bg.sh stop / pkill ngrok) and retry" >&2
    exit 1
  fi

  : > "$LOG_FILE"
  local args=(http "$port" --log=stdout --log-format=logfmt)
  if [ -n "$domain" ]; then
    args+=(--domain="$domain")
  fi

  nohup ngrok "${args[@]}" >> "$LOG_FILE" 2>&1 &
  local pid=$!
  echo "$pid"  > "$PID_FILE"
  echo "$port" > "$PORT_FILE"
  : > "$URL_FILE"

  local url
  if ! url="$(wait_for_url)"; then
    echo "ngrok did not expose a public URL within ~9s. Last log:" >&2
    tail -n 30 "$LOG_FILE" >&2 || true
    cmd_stop >/dev/null 2>&1 || true
    exit 1
  fi

  echo "started ngrok (pid $pid)"
  echo "  port      : $port"
  echo "  url       : $url"
  echo "  webhook   : $url/webhook/approval"
  echo "  inspector : $INSPECTOR"
  echo "  log       : $LOG_FILE"
}

cmd_stop() {
  if [ ! -f "$PID_FILE" ]; then
    echo "no pid file — nothing to stop"
    return 0
  fi
  local pid
  pid="$(cat "$PID_FILE")"
  if kill -0 "$pid" 2>/dev/null; then
    kill "$pid" 2>/dev/null || true
    for _ in 1 2 3 4 5; do
      if ! kill -0 "$pid" 2>/dev/null; then
        break
      fi
      sleep 0.3
    done
    if kill -0 "$pid" 2>/dev/null; then
      kill -9 "$pid" 2>/dev/null || true
    fi
    echo "stopped ngrok (pid $pid)"
  else
    echo "pid $pid not alive — cleaning up"
  fi
  rm -f "$PID_FILE" "$URL_FILE" "$PORT_FILE"
}

cmd_status() {
  if ! is_running; then
    echo "ngrok-bg: not running"
    return 1
  fi
  local pid port url etime
  pid="$(cat "$PID_FILE")"
  port="$(cat "$PORT_FILE" 2>/dev/null || echo '?')"
  url="$(fetch_url || true)"
  etime="$(ps -p "$pid" -o etime= 2>/dev/null | tr -d ' ' || echo '?')"
  echo "ngrok-bg: running"
  echo "  pid       : $pid"
  echo "  uptime    : $etime"
  echo "  port      : $port"
  echo "  url       : ${url:-<inspector unreachable>}"
  echo "  webhook   : ${url:+$url/webhook/approval}"
  echo "  inspector : $INSPECTOR"
  echo "  log       : $LOG_FILE"
}

cmd_url() {
  if ! is_running; then
    return 1
  fi
  fetch_url
}

cmd_webhook_url() {
  local path="${1:-/webhook/approval}"
  if ! is_running; then
    return 1
  fi
  local url
  url="$(fetch_url || true)"
  [ -z "$url" ] && return 1
  printf "%s%s\n" "$url" "$path"
}

cmd_restart() {
  cmd_stop >/dev/null 2>&1 || true
  sleep 0.5
  cmd_start "$@"
}

cmd_log() {
  local n="${1:-50}"
  [ -f "$LOG_FILE" ] || { echo "no log file at $LOG_FILE"; return 1; }
  tail -n "$n" "$LOG_FILE"
}

main() {
  local cmd="${1:-status}"
  [ $# -gt 0 ] && shift || true
  case "$cmd" in
    start)        cmd_start "$@" ;;
    stop)         cmd_stop ;;
    status)       cmd_status ;;
    url)          cmd_url ;;
    webhook-url)  cmd_webhook_url "$@" ;;
    restart)      cmd_restart "$@" ;;
    log)          cmd_log "$@" ;;
    -h|--help|help)
      sed -n '2,16p' "$0"
      ;;
    *)
      echo "unknown subcommand: $cmd" >&2
      sed -n '2,16p' "$0" >&2
      exit 64
      ;;
  esac
}

main "$@"
