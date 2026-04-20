#!/usr/bin/env bash
# End-to-end acceptance for #354 — cross-project handoff **outgoing half**
# driven through the real clone-dev-skeleton (same project that ships the
# T4.1 task-graph machinery).
#
# Exercises the bus sequence that the outgoing half must produce:
#
#   POST /cross_project_requested     # endpoint smoke (this PR)
#      → CrossProjectRequested        # emitted onto the bus
#      → [mastermind handler runs]
#      → skill.github.create_labeled_issue
#         ↳ stubbed `gh` on $PATH records its argv to gh-calls.log,
#           echoes a fake issue URL back to the runtime
#      → TaskBlocked(task_id=T7, blocked_on=[issue_url])
#         ↳ mastermind writes the blocker edge into its task graph
#
# The complementary webhook-wake-up leg (PR merge → TaskCompleted) is
# covered by the sibling e2e.sh in this same directory — that script
# already witnesses the return path end-to-end. Together they close
# the #300 round trip.
#
# We boot the skeleton from *its own project directory* so forge.project.toml
# resolves `skill.github`/`skill.slack` against the repo skills without any
# synthetic wiring. The skeleton's own forge.config.toml normally wants a
# real LLM provider; we override with FORGE_CONFIG pointing at a scratch
# mock-only config because the outgoing handler itself does not call
# `reason` on the skill-call path we're exercising.
#
# Usage:
#   cargo build
#   examples/agents/cross-project-bridge/tests/round_trip.sh
#
# Returns non-zero on any assertion failure. Cleans up the temp dir and
# any background processes on exit.

set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../../../.." && pwd)"
skeleton_dir="$repo_root/examples/agents/clone-dev-skeleton"
tmp_root="$(mktemp -d -t forge-round-trip-XXXXXX)"
server_pid=""
sse_pid=""
trap 'rm -rf "$tmp_root"; [ -n "$server_pid" ] && kill "$server_pid" 2>/dev/null || true; [ -n "$sse_pid" ] && kill "$sse_pid" 2>/dev/null || true' EXIT

# ── 1. `gh` stub on PATH ────────────────────────────────────────
# One line per invocation, argv joined by `|`. Stdout is a fake issue
# URL so the runtime resolves `create_labeled_issue` as confident Text.
mkdir -p "$tmp_root/bin"
cat > "$tmp_root/bin/gh" <<'GH'
#!/usr/bin/env bash
printf '%s\n' "$(printf '%s|' "$@")" >> "$GH_CALLS_LOG"
if [ "${1:-}" = "auth" ] && [ "${2:-}" = "status" ]; then
  echo "Logged in (stub)."
  exit 0
fi
if [ "${1:-}" = "issue" ] && [ "${2:-}" = "create" ]; then
  echo "https://github.com/stub/repo-b/issues/999"
  exit 0
fi
echo "stub-gh: unexpected argv: $*" 1>&2
exit 64
GH
chmod +x "$tmp_root/bin/gh"
export GH_CALLS_LOG="$tmp_root/gh-calls.log"
: > "$GH_CALLS_LOG"
export PATH="$tmp_root/bin:$PATH"

# ── 2. Mock-only config to avoid a real LLM provider requirement ─
cat > "$tmp_root/forge.config.toml" <<EOF
[storage]
root = "$tmp_root/.forge-data"

[llm]
default = "mock"

[providers.mock]
type = "mock"
EOF
export FORGE_CONFIG="$tmp_root/forge.config.toml"

port=$((20000 + RANDOM % 10000))

# ── 3. Boot skeleton from its own project directory ──────────────
# cd into the skeleton dir so forge.project.toml resolves the skill
# paths (they're relative: ../../../skills/github, ../../../skills/slack).
cd "$skeleton_dir"
"$repo_root/target/debug/forge" serve main.forge --host 127.0.0.1 --port "$port" \
  > "$tmp_root/server.log" 2>&1 &
server_pid=$!
for _ in $(seq 1 60); do
  if curl -sf "http://127.0.0.1:$port/__forge/inspect/agents" >/dev/null 2>&1; then
    break
  fi
  sleep 0.25
done

if ! kill -0 "$server_pid" 2>/dev/null; then
  echo "[round-trip] FAIL: server failed to start"
  tail -60 "$tmp_root/server.log"
  exit 1
fi

events_file="$tmp_root/events.txt"
curl -sN "http://127.0.0.1:$port/__forge/events" > "$events_file" &
sse_pid=$!
sleep 1

# ── 4. Drive the outgoing half: POST /cross_project_requested ───
echo "[round-trip] POST /cross_project_requested"
curl -sS -o /dev/null \
  -X POST "http://127.0.0.1:$port/cross_project_requested" \
  --data-urlencode "source_task_id=T7" \
  --data-urlencode "target_repo=ncmlabs/repo-b" \
  --data-urlencode "description=port auth middleware" \
  --data-urlencode "labels=area:auth,priority:p1"
sleep 2

# ── 5. Assert stubbed `gh` received the labeled issue argv ──────
if ! grep -q 'issue|create|' "$GH_CALLS_LOG"; then
  echo "[round-trip] FAIL: stubbed gh was not invoked for 'issue create'"
  echo "--- gh-calls.log ---"; cat "$GH_CALLS_LOG"
  echo "--- server.log ---";   tail -60 "$tmp_root/server.log"
  exit 1
fi
if ! grep -q 'clone-dev,from:T7,blocks:T7,area:auth,priority:p1' "$GH_CALLS_LOG"; then
  echo "[round-trip] FAIL: composite label CSV did not reach gh intact"
  echo "--- gh-calls.log ---"; cat "$GH_CALLS_LOG"
  exit 1
fi
if ! grep -q 'ncmlabs/repo-b' "$GH_CALLS_LOG"; then
  echo "[round-trip] FAIL: target repo missing from gh argv"
  echo "--- gh-calls.log ---"; cat "$GH_CALLS_LOG"
  exit 1
fi
echo "[round-trip] OK — gh invoked with composite label CSV and target repo"

sleep 1
kill "$sse_pid" 2>/dev/null || true

# ── 6. Observer timeline: CrossProjectRequested and TaskBlocked ──
# The bridge's sibling e2e.sh asserts webhook_received / PrMerged / TaskCompleted.
# Here we assert the outgoing-leg events appear in order.
seq_events=(CrossProjectRequested TaskBlocked)
prev=0
for ev in "${seq_events[@]}"; do
  line=$(grep -n "$ev" "$events_file" | head -n1 | cut -d: -f1 || true)
  if [ -z "$line" ]; then
    echo "[round-trip] FAIL: '$ev' never appeared on the Observer feed"
    echo "--- events.txt ---"; cat "$events_file"
    exit 1
  fi
  if [ "$line" -le "$prev" ]; then
    echo "[round-trip] FAIL: '$ev' at line $line precedes previous at line $prev"
    echo "--- events.txt ---"; cat "$events_file"
    exit 1
  fi
  prev=$line
done

cp "$events_file" \
  "$repo_root/examples/agents/cross-project-bridge/tests/round_trip_timeline.ndjson" \
  2>/dev/null || true

echo "[round-trip] PASS — outgoing half drives skill.github.create_labeled_issue and emits TaskBlocked with the opened issue URL"
echo "[round-trip] NOTE: for the full merge→unblock round trip, combine this run with e2e.sh (return half)"
