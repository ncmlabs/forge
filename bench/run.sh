#!/usr/bin/env bash
# FORGE authoring benchmark harness — issue #483 part 2.
#
# Per spec: docs/forge-card.md plus spec.md go to the model, the reply's first
# fenced block is checked with `forge check --json`, a failed check is repaired
# at most twice from the previous attempt's diagnostics, and the surviving
# program is graded by `forge test --expect` against the spec's recorded
# fixture. Every spec is reported — no best-of (Principle I). Usage lives in
# `bench/README.md` § Running the harness.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CARD="docs/forge-card.md"
SPECS_DIR="bench/specs"
MOCK_CONFIG="bench/mock.config.toml"
# The one instruction appended to every prompt that asks for a program.
INSTRUCTION='Reply with one complete .forge program in a single ```forge block.'
# The draft plus two repairs (the #471 loop shape).
MAX_ATTEMPTS=3

BENCH_BASE_URL="${BENCH_BASE_URL:-}"
BENCH_API_KEY="${BENCH_API_KEY:-}"
BENCH_MODEL="${BENCH_MODEL:-}"
BENCH_FAKE="${BENCH_FAKE:-}"
FORGE_BIN="${FORGE_BIN:-cargo run -q --}"
export FORGE_BIN
read -r -a FORGE_CMD <<<"$FORGE_BIN"

log() { echo "bench: $*" >&2; }
die() { echo "bench: $*" >&2; exit 1; }

usage() {
  cat <<'USAGE'
Usage: bench/run.sh --model <name> [--specs 01,02,...] [--out <file>]
       bench/run.sh --self-test

Runs the selected bench/specs/NN-* specs against one model, writes the results
JSON, and prints one markdown table on stdout.

  --model <name>  label for this run, and the default results file name
  --specs <ids>   comma-separated two-digit spec ids (default: every spec)
  --out <file>    results JSON (default: bench/results/<date>-<name>.json)
  --self-test     offline fake-model run of specs 01 and 02, self-checking

Environment:
  BENCH_BASE_URL  OpenAI-compatible /v1 base URL (required unless BENCH_FAKE)
  BENCH_API_KEY   bearer token for that endpoint (optional)
  BENCH_MODEL     provider model id (required unless BENCH_FAKE)
  BENCH_FAKE      reference | broken — answer locally instead of curl
  FORGE_BIN       forge command (default: cargo run -q --)
USAGE
}

# ---------------------------------------------------------------- the model

# `reference`: every reply is the spec's own reference. `broken`: the first
# reply is the reference minus its first block-opening `do` (a parse error —
# `gives` is optional, so deleting that line still checks clean), later replies
# are the reference. Both are wrapped in a fence and returned as the same
# OpenAI-compatible JSON a /v1 endpoint returns, so everything downstream —
# usage accounting included — runs offline with no key and no cost.
fake_response() {  # $1 = request JSON, $2 = spec dir, $3 = attempt number
  local request="$1" dir="$2" attempt="$3" content fenced
  if [[ "$BENCH_FAKE" == broken && "$attempt" == 1 ]]; then
    content="$(awk 'BEGIN{done=0} /^[[:space:]]*do[[:space:]]*$/ && !done {done=1; next} {print}' \
      "$dir/reference.forge")"
  else
    content="$(cat "$dir/reference.forge")"
  fi
  # The backticks are markdown fences sent to the model, not command substitution.
  # shellcheck disable=SC2016
  printf -v fenced '```forge\n%s\n```' "$content"
  # No tokenizer here: counts are a length estimate, enough to exercise the
  # accounting path. A live run reports the endpoint's own usage numbers.
  jq -cn --arg request "$request" --arg content "$fenced" \
    '{choices:[{message:{role:"assistant",content:$content}}],
      usage:{prompt_tokens:((($request | length) / 4) | floor),
             completion_tokens:((($content | length) / 4) | floor)}}'
}

model_response() {  # $1 = request JSON, $2 = spec dir, $3 = attempt number
  [[ -n "$BENCH_FAKE" ]] || die "no BENCH_FAKE: the live /v1 path lands in the next commit"
  fake_response "$1" "$2" "$3"
}

# The first fenced block of a reply, else the whole reply: a program sent
# without fences is still graded, and pure prose falls through to a parse error
# instead of to an empty program, which `forge check` accepts as clean.
extract_program() {
  local text
  text="$(printf '%s\n' "$1" | awk '
    /^[[:space:]]*```/ { if (inside) exit; inside = 1; next }
    inside { print }
  ')"
  if [[ -z "${text//[[:space:]]/}" ]]; then
    printf '%s\n' "$1"
  else
    printf '%s\n' "$text"
  fi
}

# ------------------------------------------------------------- the grading

# The diagnostics array `forge check --json` reports for a program. A missing
# envelope is a harness bug, not a model failure, so it is fatal.
check_diagnostics() {  # $1 = program file
  local out
  out="$("${FORGE_CMD[@]}" check --json "$1" 2>/dev/null)" || true
  if ! jq -e 'has("data") and (.data | has("diagnostics"))' >/dev/null 2>&1 <<<"$out"; then
    die "forge check --json reported no diagnostics envelope for $1: $out"
  fi
  jq -c '.data.diagnostics' <<<"$out"
}

GRADE_CORRECT=false
GRADE_REASON=""

# `forge test --expect` replays the spec's recorded fixture beside the model's
# program, offline and at zero cost: exit 0 is correct. A program that makes
# calls the fixture lacks is a replay miss, recorded as incorrect with its
# reason rather than retried.
grade_correct() {  # $1 = spec dir, $2 = program file
  local dir="$1" program="$2" out status
  GRADE_CORRECT=false
  GRADE_REASON=""
  cp "$dir/reference.forge.fixtures.json" "$program.fixtures.json"
  out="$(FORGE_CONFIG="$MOCK_CONFIG" "${FORGE_CMD[@]}" test "$program" \
    --expect "$dir/expected.txt" --json 2>/dev/null)" && status=0 || status=$?
  if [[ $status -eq 0 ]]; then
    GRADE_CORRECT=true
    return 0
  fi
  GRADE_REASON="$(jq -r '(.error.message // "") | split("\n")[0]' <<<"$out" 2>/dev/null || true)"
  if [[ -z "$GRADE_REASON" ]]; then
    GRADE_REASON="forge test exited $status"
  fi
  if [[ "$GRADE_REASON" == *"no recorded response"* ]]; then
    GRADE_REASON="replay miss: $GRADE_REASON"
  fi
  GRADE_REASON="${GRADE_REASON:0:300}"
}

# --------------------------------------------------------------- one spec

run_spec() {  # $1 = spec directory; appends one result object to $SPECS_JSONL
  local dir="$1" name id difficulty program
  name="$(basename "$dir")"
  id="${name%%-*}"
  difficulty="$(sed -n 's/^difficulty *= *"\([^"]*\)".*/\1/p' "$dir/meta.toml")"
  program="$WORK/$id.forge"

  local messages request response content text diags turn
  local attempts=0 tokens_in=0 tokens_out=0
  local clean=false pass1=false note="" final_diags='[]'

  messages="$(jq -cn --rawfile card "$CARD" --rawfile spec "$dir/spec.md" \
    --arg instruction "$INSTRUCTION" \
    '[{role:"system",content:$card},
      {role:"user",content:($spec + "\n" + $instruction)}]')"

  while [[ $attempts -lt $MAX_ATTEMPTS ]]; do
    note=""
    request="$(jq -cn --argjson messages "$messages" --arg model "${BENCH_MODEL:-$MODEL_NAME}" \
      '{model:$model,messages:$messages}')"
    response="$(model_response "$request" "$dir" "$((attempts + 1))")" \
      || die "$name: the model call failed"
    if ! jq -e '(.choices | type == "array") and (.choices | length > 0)' >/dev/null 2>&1 <<<"$response"; then
      die "$name: no OpenAI-compatible reply — check BENCH_BASE_URL and BENCH_MODEL: $response"
    fi
    content="$(jq -r '.choices[0].message.content // ""' <<<"$response")"
    tokens_in=$((tokens_in + $(jq -r '(.usage.prompt_tokens // 0) | floor' <<<"$response")))
    tokens_out=$((tokens_out + $(jq -r '(.usage.completion_tokens // 0) | floor' <<<"$response")))
    attempts=$((attempts + 1))

    text="$(extract_program "$content")"
    printf '%s\n' "$text" >"$program"
    if [[ -z "${text//[[:space:]]/}" ]]; then
      # An empty program checks clean; a reply with no program is not a pass.
      note="no .forge program in the reply"
      diags='[]'
    else
      diags="$(check_diagnostics "$program")"
    fi
    final_diags="$diags"

    if [[ -z "$note" && "$diags" == '[]' ]]; then
      clean=true
      if [[ $attempts -eq 1 ]]; then pass1=true; fi
      break
    fi
    if [[ $attempts -ge $MAX_ATTEMPTS ]]; then break; fi

    # The #471 repair turn: the previous program, then the diagnostics array
    # from `forge check --json` unchanged.
    turn="$text"
    if [[ -z "${turn//[[:space:]]/}" ]]; then turn="$content"; fi
    if [[ -z "${turn//[[:space:]]/}" ]]; then turn="(no program was returned)"; fi
    messages="$(jq -cn --argjson messages "$messages" --arg turn "$turn" \
      --argjson diags "$diags" --arg instruction "$INSTRUCTION" \
      '$messages + [{role:"assistant",content:$turn},
        {role:"user",content:("forge check --json reported these diagnostics:\n\n"
          + ($diags | tojson) + "\n\n" + $instruction)}]')"
  done

  grade_correct "$dir" "$program"
  if [[ "$clean" != true && -z "$note" ]]; then
    note="diagnostics remain after $attempts attempt(s)"
  fi

  jq -cn --arg spec "$name" --arg difficulty "$difficulty" \
    --argjson pass1 "$pass1" --argjson pass3 "$clean" --argjson attempts "$attempts" \
    --argjson correct "$GRADE_CORRECT" --arg note "$note" --arg reason "$GRADE_REASON" \
    --argjson tokens_in "$tokens_in" --argjson tokens_out "$tokens_out" \
    --argjson final_codes "$(jq -c '[.[].code]' <<<"$final_diags")" \
    --argjson final_diagnostics "$final_diags" \
    '{spec:$spec, difficulty:$difficulty, pass1:$pass1, pass3:$pass3, attempts:$attempts,
      correct:$correct, note:$note, reason:$reason, tokens_in:$tokens_in,
      tokens_out:$tokens_out, final_codes:$final_codes,
      final_diagnostics:$final_diagnostics}' >>"$SPECS_JSONL"
  log "$name: pass@1=$pass1 pass@3=$clean attempts=$attempts correct=$GRADE_CORRECT tokens=${tokens_in}/${tokens_out}"
}

print_table() {  # $1 = results JSON
  jq -r '
    "| spec | difficulty | pass@1 | pass@3 | attempts | correct | tokens in | tokens out | final codes |",
    "| --- | --- | --- | --- | --- | --- | --- | --- | --- |",
    (.specs[] | "| \(.spec) | \(.difficulty) | \(.pass1) | \(.pass3) | \(.attempts) | \(.correct) | \(.tokens_in) | \(.tokens_out) | \(.final_codes | join(" ")) |"),
    "",
    "**\(.model)** — \(.date), forge \(.forge_rev): pass@1 \(.totals.pass1_rate), pass@3 \(.totals.pass3_rate), correct \(.totals.correct_rate) over \(.specs | length) spec(s); \(.totals.tokens_in) tokens in / \(.totals.tokens_out) out."
  ' "$1"
}

# --------------------------------------------------------------- self-test

# Offline proof that the loop works end to end: with `reference` a model that
# always answers correctly must score pass@1 = pass@3 = correct = 2/2 in one
# attempt per spec; with `broken` the first attempt is a parse error and the
# repair fixes it, so pass@1 = 0, pass@3 = 2/2 and attempts = 2.
expect_fake() {  # $1 = fake mode, $2 = pass@1 rate, $3 = pass@3 rate, $4 = attempts
  local mode="$1" p1="$2" p3="$3" want="$4" json="$WORK/self-$1.json"
  log "self-test: BENCH_FAKE=$mode"
  BENCH_FAKE="$mode" bash "$ROOT/bench/run.sh" --model "fake-$mode" --specs 01,02 --out "$json"
  if jq -e --argjson p1 "$p1" --argjson p3 "$p3" --argjson n "$want" '
        (.specs | length) == 2
        and .totals.pass1_rate == $p1
        and .totals.pass3_rate == $p3
        and .totals.correct_rate == 1
        and ([.specs[].attempts] | all(. == $n))' "$json" >/dev/null; then
    log "self-test $mode: PASS — pass@1 $p1, pass@3 $p3, correct 2/2, $want attempt(s) each"
  else
    echo "bench self-test: FAIL for BENCH_FAKE=$mode (want pass@1=$p1, pass@3=$p3, correct=1, attempts=$want)" >&2
    print_table "$json" >&2
    exit 1
  fi
}

self_test() {
  expect_fake reference 1 1 1
  expect_fake broken 0 1 2
}

# -------------------------------------------------------------------- main

SELF_TEST=0
MODEL_NAME=""
SPECS=""
OUT=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --model) MODEL_NAME="${2:?--model needs a name}"; shift 2 ;;
    --specs) SPECS="${2:?--specs needs a comma-separated id list}"; shift 2 ;;
    --out) OUT="${2:?--out needs a path}"; shift 2 ;;
    --self-test) SELF_TEST=1; shift ;;
    -h | --help) usage; exit 0 ;;
    *) die "unknown argument: $1 (try --help)" ;;
  esac
done

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

if [[ "$SELF_TEST" == 1 ]]; then
  self_test
  exit 0
fi

[[ -n "$MODEL_NAME" ]] || die "--model <name> is required (or run --self-test)"
if [[ -z "$BENCH_FAKE" ]]; then
  [[ -n "$BENCH_BASE_URL" ]] || die "BENCH_BASE_URL is required: an OpenAI-compatible /v1 base URL"
  [[ -n "$BENCH_MODEL" ]] || die "BENCH_MODEL is required: the provider's model id"
fi
BENCH_BASE_URL="${BENCH_BASE_URL%/}"

dirs=()
if [[ -z "$SPECS" ]]; then
  for dir in "$SPECS_DIR"/*/; do dirs+=("$dir"); done
else
  IFS=',' read -r -a ids <<<"$SPECS"
  for id in "${ids[@]}"; do
    [[ "$id" =~ ^[0-9]{2}$ ]] || die "--specs takes two-digit ids; got '$id'"
    matches=("$SPECS_DIR/$id"-*/)
    [[ -d "${matches[0]:-}" ]] || die "no spec directory matching $SPECS_DIR/$id-*"
    dirs+=("${matches[0]}")
  done
fi
[[ ${#dirs[@]} -gt 0 ]] || die "no specs selected"

OUT="${OUT:-bench/results/$(date -u +%F)-$MODEL_NAME.json}"
mkdir -p "$(dirname "$OUT")"
SPECS_JSONL="$WORK/specs.jsonl"
: >"$SPECS_JSONL"
FORGE_REV="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
log "model=$MODEL_NAME specs=${#dirs[@]} out=$OUT fake=${BENCH_FAKE:-none}"

for dir in "${dirs[@]}"; do
  run_spec "$dir"
done

jq -n --slurpfile specs "$SPECS_JSONL" --arg model "$MODEL_NAME" \
  --arg date "$(date -u +%F)" --arg rev "$FORGE_REV" '
  $specs as $s |
  {model: $model, date: $date, forge_rev: $rev,
   totals: {
     pass1_rate: (([$s[] | select(.pass1)] | length) / ($s | length) * 1000 | round / 1000),
     pass3_rate: (([$s[] | select(.pass3)] | length) / ($s | length) * 1000 | round / 1000),
     correct_rate: (([$s[] | select(.correct)] | length) / ($s | length) * 1000 | round / 1000),
     tokens_in: ([$s[].tokens_in] | add // 0),
     tokens_out: ([$s[].tokens_out] | add // 0)},
   specs: $s}' >"$OUT"

print_table "$OUT"
