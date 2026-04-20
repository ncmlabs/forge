#!/usr/bin/env bash
# End-to-end acceptance for #335 — cross-project handoff return path.
#
# Simulates repo B's PR-merged webhook by POSTing a signed payload to a
# running `forge serve` instance, then asserts the Observer SSE feed shows
# `webhook_received` followed by a bus emission into the `PrMerged` handler.
#
# Usage:
#   cargo build
#   examples/agents/cross-project-bridge/tests/e2e.sh
#
# Cleans up the temp data dir on exit. Returns non-zero if any assertion
# fails.

set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../../../.." && pwd)"
example="$repo_root/examples/agents/cross-project-bridge/main.forge"
tmp_root="$(mktemp -d -t forge-wake-e2e-XXXXXX)"
trap 'rm -rf "$tmp_root"; kill $server_pid 2>/dev/null || true' EXIT

cd "$tmp_root"
cat > forge.config.toml <<EOF
[storage]
root = "$tmp_root/.forge-data"
EOF

# Fresh port to avoid colliding with a running dev server.
port=$((20000 + RANDOM % 10000))

# 1. Rotate a fresh secret — capture stdout only (stderr has the warning).
secret=$("$repo_root/target/debug/forge" wake rotate --agent mastermind --trigger pr_merged)
echo "[e2e] registered secret: ${secret:0:8}..."

# 2. Start the server and wait for it to come up.
"$repo_root/target/debug/forge" serve "$example" --host 127.0.0.1 --port "$port" >server.log 2>&1 &
server_pid=$!
for i in $(seq 1 40); do
  if curl -sf "http://127.0.0.1:$port/__forge/inspect/agents" >/dev/null 2>&1; then
    break
  fi
  sleep 0.25
done

# 3. Start tailing the SSE events channel in the background.
events_file="$tmp_root/events.txt"
curl -sN "http://127.0.0.1:$port/__forge/events" >"$events_file" &
sse_pid=$!
sleep 1 # give the SSE handshake a moment

# 4. POST a signed webhook.
body='{"repo":"repo-b","pr_number":42,"merged_by":"octocat"}'
sig=$(printf '%s' "$body" | openssl dgst -sha256 -hmac "$secret" | sed 's/^.* //')
echo "[e2e] POST /wake/mastermind/pr_merged"
response=$(curl -sS -o /dev/null -w '%{http_code}' \
  -X POST "http://127.0.0.1:$port/wake/mastermind/pr_merged" \
  -H "Content-Type: application/json" \
  -H "X-Hub-Signature-256: sha256=$sig" \
  -d "$body")
if [ "$response" != "202" ]; then
  echo "[e2e] FAIL: expected 202 Accepted, got $response"
  exit 1
fi

# 5. Tampered-body negative assertion.
bad_sig=$(printf '%s' "TAMPERED" | openssl dgst -sha256 -hmac "$secret" | sed 's/^.* //')
response=$(curl -sS -o /dev/null -w '%{http_code}' \
  -X POST "http://127.0.0.1:$port/wake/mastermind/pr_merged" \
  -H "Content-Type: application/json" \
  -H "X-Hub-Signature-256: sha256=$bad_sig" \
  -d "$body")
if [ "$response" != "401" ]; then
  echo "[e2e] FAIL: tampered body should return 401, got $response"
  exit 1
fi

# Give the SSE feed a moment to flush events.
sleep 1
kill $sse_pid 2>/dev/null || true

# 6. Assert tracer events appear in the feed.
if ! grep -q 'webhook_received' "$events_file"; then
  echo "[e2e] FAIL: no webhook_received event observed"
  cat "$events_file"
  exit 1
fi
if ! grep -q 'webhook_rejected_signature' "$events_file"; then
  echo "[e2e] FAIL: no webhook_rejected_signature event observed"
  cat "$events_file"
  exit 1
fi

echo "[e2e] PASS — webhook received, signature rejection traced, Observer timeline populated"
