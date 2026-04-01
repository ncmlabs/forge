#!/usr/bin/env bash
# FORGE Security Scanner
# Called by Claude Code PreToolUse hook before git commit.
# Scans staged files for leaked secrets, credentials, and internal IPs.
# Outputs JSON to block commit on violation, exits 0 silently on pass.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
ALLOWLIST_FILE="$REPO_ROOT/.forge-security-allow"
VIOLATIONS=""

# ── Load allowlist ─────────────────────────────────────────────────
allowed_files=""
if [ -f "$ALLOWLIST_FILE" ]; then
  allowed_files=$(grep -v '^\s*#' "$ALLOWLIST_FILE" | grep -v '^\s*$' | cut -d: -f1 || true)
fi

is_allowed() {
  local file="$1"
  if [ -n "$allowed_files" ]; then
    echo "$allowed_files" | grep -qx "$file" && return 0
  fi
  return 1
}

# ── Check 1: Blocked file patterns ────────────────────────────────
# These files should never be committed regardless of content.

STAGED_FILES=$(git diff --cached --name-only --diff-filter=ACM 2>/dev/null || true)

if [ -z "$STAGED_FILES" ]; then
  exit 0
fi

for f in $STAGED_FILES; do
  case "$f" in
    *.pem|*.key|*.p12|*.pfx|*.jks)
      VIOLATIONS="${VIOLATIONS}[SECRET FILE] $f\nBlocked: Private key / certificate file should not be committed\n\n"
      ;;
    .env|.env.*|*.env)
      VIOLATIONS="${VIOLATIONS}[SECRET FILE] $f\nBlocked: Environment file may contain secrets\n\n"
      ;;
    id_rsa*|id_ed25519*|id_ecdsa*|id_dsa*)
      VIOLATIONS="${VIOLATIONS}[SECRET FILE] $f\nBlocked: SSH private key should not be committed\n\n"
      ;;
    credentials.json|service-account*.json)
      VIOLATIONS="${VIOLATIONS}[SECRET FILE] $f\nBlocked: Credentials file should not be committed\n\n"
      ;;
  esac
done

# ── Check 2: Scan staged diffs for secrets ─────────────────────────
# Only scan added lines (^+) excluding diff headers (^+++)

STAGED_DIFF=$(git diff --cached 2>/dev/null || true)

if [ -z "$STAGED_DIFF" ]; then
  # Only new files with no diff — check file contents directly
  for f in $STAGED_FILES; do
    filepath="$REPO_ROOT/$f"
    [ -f "$filepath" ] || continue
    is_allowed "$f" && continue
    STAGED_DIFF="${STAGED_DIFF}
+++ b/$f
$(sed 's/^/+/' "$filepath")"
  done
fi

# Process diff file by file
current_file=""
while IFS= read -r line; do
  # Track current file
  if [[ "$line" == "+++ b/"* ]]; then
    current_file="${line#+++ b/}"
    continue
  fi

  # Only scan added lines (not removals or context)
  [[ "$line" == "+"* ]] || continue
  [[ "$line" == "+++"* ]] && continue

  # Skip allowed files
  if [ -n "$current_file" ] && is_allowed "$current_file"; then
    continue
  fi

  content="${line#+}"

  # ── API Keys ───────────────────────────────────────────────────
  if echo "$content" | grep -qE 'sk-ant-[a-zA-Z0-9_-]{20,}'; then
    VIOLATIONS="${VIOLATIONS}[API KEY] Anthropic key in $current_file\nLine: ${content:0:80}...\n\n"
  fi

  if echo "$content" | grep -qE 'ghp_[a-zA-Z0-9]{36}'; then
    VIOLATIONS="${VIOLATIONS}[API KEY] GitHub personal access token in $current_file\nLine: ${content:0:80}...\n\n"
  fi

  if echo "$content" | grep -qE 'gsk_[a-zA-Z0-9]{20,}'; then
    VIOLATIONS="${VIOLATIONS}[API KEY] Groq API key in $current_file\nLine: ${content:0:80}...\n\n"
  fi

  if echo "$content" | grep -qE 'AKIA[0-9A-Z]{16}'; then
    VIOLATIONS="${VIOLATIONS}[API KEY] AWS access key in $current_file\nLine: ${content:0:80}...\n\n"
  fi

  # ── Private IPs ────────────────────────────────────────────────
  if echo "$content" | grep -qE '192\.168\.[0-9]{1,3}\.[0-9]{1,3}'; then
    VIOLATIONS="${VIOLATIONS}[PRIVATE IP] Internal IP (192.168.x.x) in $current_file\nLine: ${content:0:80}...\n\n"
  fi

  if echo "$content" | grep -qE '(^|[^0-9])10\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}'; then
    # Exclude common non-IP patterns like version numbers
    if ! echo "$content" | grep -qE '10\.(0\.0\.0|255\.|x\.)'; then
      VIOLATIONS="${VIOLATIONS}[PRIVATE IP] Internal IP (10.x.x.x) in $current_file\nLine: ${content:0:80}...\n\n"
    fi
  fi

  if echo "$content" | grep -qE '172\.(1[6-9]|2[0-9]|3[01])\.[0-9]{1,3}\.[0-9]{1,3}'; then
    VIOLATIONS="${VIOLATIONS}[PRIVATE IP] Internal IP (172.16-31.x.x) in $current_file\nLine: ${content:0:80}...\n\n"
  fi

  # ── Private Keys ───────────────────────────────────────────────
  if echo "$content" | grep -qE 'BEGIN (RSA |EC |DSA |OPENSSH )?PRIVATE KEY'; then
    VIOLATIONS="${VIOLATIONS}[PRIVATE KEY] Private key material in $current_file\nLine: ${content:0:80}...\n\n"
  fi

  # ── Secrets in config ──────────────────────────────────────────
  # Match password/secret/token assignments, exclude env var references
  if echo "$content" | grep -iqE '(password|secret|auth_token|access_key|private_key)\s*[=:]\s*["\x27]?[a-zA-Z0-9/+=_-]{8,}'; then
    # Exclude env var references like ${VAR} or $VAR or "not-required"
    if ! echo "$content" | grep -qE '\$\{|\$[A-Z]|not-required|REDACTED|placeholder|example|TODO'; then
      VIOLATIONS="${VIOLATIONS}[SECRET] Possible hardcoded secret in $current_file\nLine: ${content:0:80}...\n\n"
    fi
  fi

  # ── Database URIs with credentials ─────────────────────────────
  if echo "$content" | grep -qE '(mysql|postgres|postgresql|mongodb|redis|amqp)://[^@:]+:[^@]+@'; then
    VIOLATIONS="${VIOLATIONS}[DATABASE] Connection string with credentials in $current_file\nLine: ${content:0:80}...\n\n"
  fi

  # ── JWT Tokens ─────────────────────────────────────────────────
  if echo "$content" | grep -qE 'eyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}'; then
    VIOLATIONS="${VIOLATIONS}[JWT] JSON Web Token in $current_file\nLine: ${content:0:80}...\n\n"
  fi

done <<< "$STAGED_DIFF"

# ── Output ──────────────────────────────────────────────────────

if [ -n "$VIOLATIONS" ]; then
  reason=$(printf '%b' "$VIOLATIONS" | head -c 2000)
  reason_escaped=$(echo "$reason" | python3 -c 'import sys,json; print(json.dumps(sys.stdin.read()))' 2>/dev/null || echo '"Security violation detected — check staged files for secrets"')
  printf '{"continue": false, "stopReason": %s}\n' "$reason_escaped"
  exit 0
fi

exit 0
