#!/usr/bin/env bash
# Test harness for scripts/check-security.sh.
# Stages synthetic fixtures, runs the scanner, asserts BLOCKED/ALLOWED behavior,
# cleans up unconditionally. Run from any working tree state — this script
# stashes/restores any pre-existing staged content so it doesn't pollute it.
#
# Usage: bash scripts/test-check-security.sh
# Exit:  0 = all assertions pass, 1 = at least one regression

set -uo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
SCANNER="$REPO_ROOT/scripts/check-security.sh"
FIXTURE="$REPO_ROOT/.security_test_fixture.txt"

PASS=0
FAIL=0
FAILURES=""

# ── Cleanup trap: always runs, even on Ctrl-C ─────────────────────
cleanup() {
  git rm --cached "$FIXTURE" 2>/dev/null >/dev/null || true
  rm -f "$FIXTURE"
  # Restore any pre-test staged content
  if [ -n "${STASH_REF:-}" ]; then
    git stash pop "$STASH_REF" 2>/dev/null >/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

# ── Stash existing staged changes so they don't interfere ─────────
STASH_REF=""
if ! git diff --cached --quiet 2>/dev/null; then
  if git stash push --staged -m "test-check-security pre-test stash" >/dev/null 2>&1; then
    STASH_REF="stash@{0}"
  fi
fi

# ── Helpers ───────────────────────────────────────────────────────
expect_blocked() {
  local name="$1" content="$2"
  printf '%s\n' "$content" > "$FIXTURE"
  git add -f "$FIXTURE" 2>/dev/null
  local out
  out=$("$SCANNER" 2>&1 || true)
  git rm --cached "$FIXTURE" 2>/dev/null >/dev/null

  if echo "$out" | grep -q '"continue": false'; then
    PASS=$((PASS + 1))
    printf "  \033[32m✓\033[0m BLOCKED  %s\n" "$name"
  else
    FAIL=$((FAIL + 1))
    FAILURES="${FAILURES}    - expected BLOCKED but passed: $name\n      content: $content\n"
    printf "  \033[31m✗\033[0m EXPECTED BLOCK but passed: %s\n" "$name"
  fi
}

expect_allowed() {
  local name="$1" content="$2"
  printf '%s\n' "$content" > "$FIXTURE"
  git add -f "$FIXTURE" 2>/dev/null
  local out
  out=$("$SCANNER" 2>&1 || true)
  git rm --cached "$FIXTURE" 2>/dev/null >/dev/null

  if echo "$out" | grep -q '"continue": false'; then
    FAIL=$((FAIL + 1))
    FAILURES="${FAILURES}    - expected ALLOWED but blocked: $name\n      content: $content\n      reason: $(echo "$out" | head -c 200)\n"
    printf "  \033[31m✗\033[0m EXPECTED ALLOW but blocked: %s\n" "$name"
  else
    PASS=$((PASS + 1))
    printf "  \033[32m✓\033[0m ALLOWED  %s\n" "$name"
  fi
}

# ── Tests: must block ─────────────────────────────────────────────
# Synthetic fixtures: prefixes are split into shell variables so the script
# source itself does not contain a full token literal — that way GitHub Push
# Protection (and other static scanners) don't flag this test harness as
# leaking secrets. At runtime, bash interpolates the variables and the full
# token pattern is reconstructed in memory, written to the fixture file,
# staged, and seen by our scanner. The fixture file is deleted after each test.

# Prefixes (each one alone is harmless and not a valid token)
ANT_PRE='sk-ant'
OPENAI_PRE='sk'
OPENAI_PROJ='sk-proj'
GOOG_PRE='AIza'
STRIPE_PRE='sk_live'
GHP_PRE='ghp'
GHS_PRE='ghs'
GH_PAT_PRE='github_pat'
GROQ_PRE='gsk'
AWS_PRE='AKIA'
SLACK_BOT='xoxb'
SLACK_USR='xoxp'
JWT_HDR='eyJhbGciOiJIUzI1NiJ9'

echo ""
echo "── Secrets that MUST be blocked ──────────────────────────────"
expect_blocked "Anthropic key"        "ANTHROPIC = ${ANT_PRE}-api01-Z9aB3xQ7m_K2pL8nR5tU6vW0xY4zA1bC2dE3fG4hI5jK6lM7nO8pQ9rS-test"
expect_blocked "GitHub PAT (ghp_)"    "TOKEN = ${GHP_PRE}_abcdefghijklmnopqrstuvwxyzABCDEFGHIJ"
expect_blocked "GitHub PAT (newer)"   "TOKEN = ${GH_PAT_PRE}_AAAAAAAAAAAAAAAAAAAAAAAA_BBBBBBBBBBBBBBBBBBBBBBBBBB"
expect_blocked "GitHub server token"  "TOKEN = ${GHS_PRE}_abcdefghijklmnopqrstuvwxyz0123456789"
expect_blocked "Groq key"             "GROQ = ${GROQ_PRE}_abcdefghijklmnopqrstuvwxyz0123456789"
expect_blocked "AWS access key"       "aws_key = ${AWS_PRE}IOSFODNN7EXAMPLE"
expect_blocked "OpenAI project key"   "OPENAI = ${OPENAI_PROJ}-abcdefghijklmnopqrstuvwxyz1234567890"
expect_blocked "OpenAI legacy key"    "OPENAI = ${OPENAI_PRE}-abcdefghijklmnopqrstuvwxyz1234567890ABCDEF"
expect_blocked "Google API key"       "GOOGLE = ${GOOG_PRE}SyA1B2C3D4E5F6G7H8I9J0K1L2M3N4O5P6Q"
expect_blocked "Stripe live key"      "STRIPE = ${STRIPE_PRE}_abcdefghijklmnopqrstuvwx"
expect_blocked "Slack bot token"      "SLACK = ${SLACK_BOT}-1234567890-1234567890123-AbCdEfGhIjKlMnOpQrStUvWx"
expect_blocked "Slack user token"     "SLACK = ${SLACK_USR}-1234567890-1234567890123-AbCdEfGhIjKlMnOpQrStUvWx"
expect_blocked "Slack channel ID"     "channel = C0AS3AQRNN9"
expect_blocked "Slack user ID"        "user = U09BP5NBKUG"
expect_blocked "ngrok URL"            "callback = https://abc123.ngrok-free.app/webhook"
expect_blocked "ngrok URL (paid)"     "callback = https://my-tunnel.ngrok.app/cb"
expect_blocked "Internal hostname"    "host = api.internal.corp"
expect_blocked "Private IP 192.168"   "host = 192.168.1.42"
expect_blocked "Private IP 10.x"      "host = 10.5.5.123"
expect_blocked "Private IP 172.16-31" "host = 172.20.5.10"
expect_blocked "Private RSA key"      "-----BEGIN RSA PRIVATE KEY-----"
expect_blocked "Hardcoded password"   'password = "supersecret123abc"'
expect_blocked "DB URI with creds"    "DATABASE_URL = postgres://user:p4ssw0rd@dbhost:5432/db"
expect_blocked "JWT token"            "auth = ${JWT_HDR}.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c"

# ── Tests: must NOT block (false-positive guards) ─────────────────
echo ""
echo "── Patterns that must be ALLOWED (false-positive guards) ─────"
expect_allowed "Env var ref"          'api_key = "${ANTHROPIC_API_KEY}"'
expect_allowed "Env var no quote"     'api_key = $SLACK_BOT_TOKEN'
expect_allowed "REDACTED placeholder" 'password = "REDACTED"'
expect_allowed "TODO placeholder"     'password = "TODO-set-real-value"'
expect_allowed "Slack placeholder C"  'channel = "C0123456789"'
expect_allowed "Slack placeholder U"  'user = "U0000000000"'
expect_allowed "Rust SCREAM_CONST"    'const TIMEOUT_MS_DEFAULT_VALUE: u64 = 30000;'
expect_allowed "MSRV version line"    'msrv = "10.0.0"'
expect_allowed "Cargo version pin"    'version = "10.0.0"'
expect_allowed "Rustc version line"   'rustc = 10.0.0.1'
expect_allowed "10.0.0.0 doc literal" 'gateway = 10.0.0.0'
expect_allowed "Public domain"        "host = api.example.com"
expect_allowed ".local mDNS"          "host = printer.local"

# ── Summary ───────────────────────────────────────────────────────
echo ""
echo "──────────────────────────────────────────────────────────────"
echo "  Passed: $PASS    Failed: $FAIL"
echo "──────────────────────────────────────────────────────────────"

if [ $FAIL -gt 0 ]; then
  echo ""
  echo "Failures:"
  printf "%b" "$FAILURES"
  exit 1
fi

exit 0
