# shellcheck shell=bash
# Helpers of the agent scenarios (docs/ARCHITECTURE.md §14.1, D56). Sourced by each e2e/agent/scenarios/NN-*.sh,
# which scripts/agent-test.sh runs one by one with stdin from /dev/null. Never run it directly.
#
# The runner exports: AGENT_SCENARIO (the scenario's name), AGENT_ROOT (the repo), AGENT_RESULTS (results dir),
# AGENT_STEPS (the JSON Lines step log), AGENT_COUNTER (the TAP counter file), AGENT_CID (the wikanvas container) and,
# with a host CLI, WIKANVAS_AGENT_CLI and AGENT_URL; and the three tokens AGENT_TOKEN_READ, AGENT_TOKEN_WRITE and
# AGENT_TOKEN_ADMIN, which reach the CLI only through its environment (WIKANVAS_TOKEN), never through argv.
#
#   q [--as read|write|admin] [--token T] ARGS…   wikanvas --json ARGS…; sets $OUT_FILE, $OUT (text only), $ERR, $RC
#   q_raw NAME [--as …] [--token T] ARGS…        wikanvas ARGS… (no --json), stdout byte for byte to results/NAME
#   q_admin ARGS…                                 wikanvas --json admin ARGS… inside the container (no token)
#   expect_rc N · expect 'JQ' · expect_err 'JQ'   checks on $RC, on $OUT_FILE (jq -e), on the last line of $ERR
#   ok "name" [DETAIL]                            records the step as passed (TAP: ok N - scenario: name); a DETAIL
#                                                 goes into the report too, masked like everything else
#   fixture NAME                                  the host path of e2e/agent/fixtures/NAME or fixtures/markdown/NAME
#   scratch_collection                            a fresh collection named after the scenario; sets $SCRATCH
# A failed check prints `not ok`, records the step with the failing command and excerpts, and ends the scenario
# (exit 1). Every string written to the TAP output or the report has its tokens masked (qpat_***).
#
# A host CLI (WIKANVAS_AGENT_CLI) runs with no WIKANVAS_* variable of the caller's shell and with WIKANVAS_CONFIG
# pointing at a file that never exists, so neither an exported WIKANVAS_PROFILE nor the caller's own profile file
# reaches a scenario.

set -u

# TAP lines go to fd 3 (the runner's stdout), so a check inside a command substitution still prints them.
exec 3>&1

OUT_FILE="$AGENT_RESULTS/.out"
OUT=""
ERR=""
RC=0
LAST_CMD=""
SCRATCH=""
_step_started="$(date +%s%N)"

# Masks every token: qpat_ + 46 base62 characters.
_mask() {
  sed -E 's/qpat_[0-9A-Za-z]{46}/qpat_***/g'
}

_next() {
  local n
  n=$(($(cat "$AGENT_COUNTER" 2>/dev/null || echo 0) + 1))
  echo "$n" >"$AGENT_COUNTER"
  echo "$n"
}

# _record STATUS NAME [DETAIL]: one step line of the report (masked), and the TAP line.
_record() {
  local status="$1" name="$2" detail="${3:-}" n now ms
  n="$(_next)"
  now="$(date +%s%N)"
  ms=$(((now - _step_started) / 1000000))
  _step_started="$now"
  jq -cn \
    --arg scenario "$AGENT_SCENARIO" --arg step "$name" --arg status "$status" --argjson n "$n" \
    --argjson duration_ms "$ms" --arg command "$LAST_CMD" --arg detail "$detail" \
    --arg out "${OUT:0:2000}" --arg err "${ERR:0:2000}" --argjson rc "$RC" \
    '{n: $n, scenario: $scenario, step: $step, status: $status, duration_ms: $duration_ms}
     + (if $status == "fail" then {command: $command, rc: $rc, detail: $detail, out: $out, err: $err}
        elif $detail != "" then {detail: $detail} else {} end)' \
    | _mask >>"$AGENT_STEPS"
  if [ "$status" = pass ]; then
    echo "ok $n - $AGENT_SCENARIO: $name" | _mask >&3
  else
    {
      echo "not ok $n - $AGENT_SCENARIO: $name"
      echo "  # $detail"
      echo "  # command: $LAST_CMD (exit $RC)"
      [ -n "$OUT" ] && echo "${OUT:0:600}" | sed 's/^/  # stdout: /'
      [ -n "$ERR" ] && echo "${ERR:0:600}" | sed 's/^/  # stderr: /'
    } | _mask >&3
  fi
}

_fail() {
  _record fail "${_last_ok:+after \"$_last_ok\": }$1" "$1"
  exit 1
}

# _token_for read|write|admin
_token_for() {
  case "$1" in
    read) printf '%s' "$AGENT_TOKEN_READ" ;;
    write) printf '%s' "$AGENT_TOKEN_WRITE" ;;
    admin) printf '%s' "$AGENT_TOKEN_ADMIN" ;;
    *)
      echo "lib.sh: --as $1: use read, write or admin" >&2
      exit 2
      ;;
  esac
}

# _run JSON(0|1) STDOUT_FILE ARGS…: runs wikanvas with the token of --as/--token, stdin forwarded.
_run() {
  local json="$1" outf="$2" as=write token="" errf="$AGENT_RESULTS/.err"
  shift 2
  while [ $# -gt 0 ]; do
    case "$1" in
      --as)
        as="$2"
        shift 2
        ;;
      --token)
        token="$2"
        shift 2
        ;;
      *) break ;;
    esac
  done
  [ -n "$token" ] || token="$(_token_for "$as")"
  local -a flags=()
  [ "$json" = 1 ] && flags=(--json)
  LAST_CMD="wikanvas ${flags[*]} $* (as ${as})"
  if [ -n "${WIKANVAS_AGENT_CLI:-}" ]; then
    # Only the harness's URL and token: the caller's WIKANVAS_* variables and profile file stay out.
    local -a unset=()
    local v
    for v in $(compgen -e); do
      case "$v" in WIKANVAS_*) unset+=(-u "$v") ;; esac
    done
    env "${unset[@]}" WIKANVAS_CONFIG="$AGENT_RESULTS/.no-config.toml" WIKANVAS_URL="$AGENT_URL" WIKANVAS_TOKEN="$token" \
      "$WIKANVAS_AGENT_CLI" "${flags[@]}" "$@" >"$outf" 2>"$errf"
  else
    WIKANVAS_TOKEN="$token" docker exec -i -e WIKANVAS_TOKEN "$AGENT_CID" wikanvas "${flags[@]}" "$@" >"$outf" 2>"$errf"
  fi
  RC=$?
  ERR="$(cat "$errf")"
  OUT_FILE="$outf"
  # Bash variables cannot hold NUL bytes: $OUT is set only for text output.
  if [ "$(tr -d '\000' <"$outf" | wc -c)" = "$(wc -c <"$outf")" ]; then
    OUT="$(cat "$outf")"
  else
    OUT=""
  fi
  return 0
}

q() {
  _run 1 "$AGENT_RESULTS/.out" "$@"
}

q_raw() {
  local name="$1"
  shift
  case "$name" in
    */* | .* | "")
      echo "lib.sh: q_raw NAME: a plain file name" >&2
      exit 2
      ;;
  esac
  _run 0 "$AGENT_RESULTS/$name" "$@"
}

# The admin commands run inside the container (wikanvas-server is there), with no token.
q_admin() {
  local errf="$AGENT_RESULTS/.err"
  LAST_CMD="wikanvas --json admin $*"
  docker exec -i "$AGENT_CID" wikanvas --json admin "$@" >"$AGENT_RESULTS/.out" 2>"$errf"
  RC=$?
  ERR="$(cat "$errf")"
  OUT_FILE="$AGENT_RESULTS/.out"
  OUT="$(cat "$OUT_FILE")"
  return 0
}

expect_rc() {
  [ "$RC" = "$1" ] || _fail "expected exit $1, got $RC"
}

expect() {
  jq -e "$1" "$OUT_FILE" >/dev/null 2>&1 || _fail "stdout does not satisfy: $1"
}

expect_err() {
  local last
  last="$(printf '%s\n' "$ERR" | sed '/^[[:space:]]*$/d' | tail -n 1)"
  printf '%s' "$last" | jq -e "$1" >/dev/null 2>&1 || _fail "the last stderr line does not satisfy: $1"
}

_last_ok=""
ok() {
  _last_ok="$1"
  _record pass "$1" "${2:-}"
}

fixture() {
  local f
  for f in "$AGENT_ROOT/e2e/agent/fixtures/$1" "$AGENT_ROOT/fixtures/markdown/$1"; do
    if [ -f "$f" ]; then
      printf '%s' "$f"
      return 0
    fi
  done
  _fail "no fixture $1"
}

scratch_collection() {
  q collection create --name "agent $AGENT_SCENARIO $$"
  expect_rc 0
  SCRATCH="$(jq -r .id "$OUT_FILE")"
  [ -n "$SCRATCH" ] && [ "$SCRATCH" != null ] || _fail "the scratch collection has no id"
}

# A well-formed token that no server knows: 40 random base62 characters and their CRC32 in 6 base62 digits (D42);
# gzip's trailer holds the CRC32, little-endian like od on x86_64.
unknown_token() {
  local secret crc sum="" alphabet=0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz
  secret="$(LC_ALL=C tr -dc '0-9A-Za-z' </dev/urandom 2>/dev/null | head -c 40)"
  crc="$(printf '%s' "$secret" | gzip -c | tail -c 8 | head -c 4 | od -An -tu4 | tr -d ' ')"
  for _ in 1 2 3 4 5 6; do
    sum="${alphabet:$((crc % 62)):1}$sum"
    crc=$((crc / 62))
  done
  printf 'qpat_%s%s' "$secret" "$sum"
}

# The value of a jq filter on $OUT_FILE (a small convenience for the scenarios).
val() {
  jq -r "$1" "$OUT_FILE"
}
