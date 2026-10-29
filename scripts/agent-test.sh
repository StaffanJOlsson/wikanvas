#!/usr/bin/env bash
# The agent test harness (docs/ARCHITECTURE.md §14.1, D56): the shipped CLI against the shipped image, for AI agents
# and CI. It never builds: build the image with scripts/image.sh first (and a host CLI with scripts/cli-dist.sh).
#   scripts/agent-test.sh [run] [--scenario GLOB] [--keep] [--ttl MIN] [--allow-stale]
#   scripts/agent-test.sh up [--ttl MIN] [--allow-stale]    # leave the app on 127.0.0.1:$WIKANVAS_AGENT_PORT
#   scripts/agent-test.sh status                             # what is up, and the time left
#   scripts/agent-test.sh down                               # stop the holder, remove the project and its files
# run: starts project wikanvas-agent (compose.yaml + compose.e2e.yaml, only the wikanvas service, subnet
# 172.31.240.32/28, range .40/29, proxy address .34; --env-file /dev/null and the owner-facing variables unset),
# provisions an owner and a read, a write and an admin token inside the container (wikanvas admin …), runs
# e2e/agent/scenarios/NN-*.sh in order (--scenario filters by file name), prints TAP and writes
# e2e/agent/results/report.json (tokens masked). The project is removed on exit (down -v) unless --keep.
# up and run --keep add compose.agent.yaml (127.0.0.1:${WIKANVAS_AGENT_PORT:-18080} → wikanvas:8080) and leave the app
# for --ttl minutes (default 15, at most 15: PLAN R1): a background holder keeps the build lock, then runs down. They
# write, under umask 077, .cache/agent-test/env (WIKANVAS_URL, WIKANVAS_TOKEN_FILE=…/write.token) next to read.token,
# write.token and admin.token.
# Variables: WIKANVAS_IMAGE (default wikanvas:latest), WIKANVAS_AGENT_CLI (a host wikanvas binary: the scenarios then
# use it against the loopback port instead of the CLI in the container), WIKANVAS_AGENT_PORT (default 18080).
# Exit codes: 0 every step passed, 1 a scenario failed, 2 usage, 3 a prerequisite is missing (docker, compose >= 2.24,
# jq, the image, a free port), the image is stale (its revision label is not HEAD; --allow-stale overrides) or the
# project is already up, 4 the app did not become healthy. status: 0 up, 3 not up. down: 0.
# Host prerequisites: Docker with Compose >= 2.24, bash and jq.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
. "$root/scripts/lib/lock.sh"
. "$root/scripts/lib/slice.sh"

project=wikanvas-agent
state="$root/.cache/agent-test"
results="$root/e2e/agent/results"
image="${WIKANVAS_IMAGE:-wikanvas:latest}"
port="${WIKANVAS_AGENT_PORT:-18080}"
max_ttl=15

usage() {
  sed -n '2,25s/^# \{0,1\}//p' "$root/scripts/agent-test.sh"
}

die() {
  local code="$1"
  shift
  echo "agent-test: $*" >&2
  exit "$code"
}

# --- arguments ----------------------------------------------------------------------------------------------------
cmd=run
case "${1:-}" in
  run | up | status | down)
    cmd="$1"
    shift
    ;;
  -h | --help)
    usage
    exit 0
    ;;
esac
scenario_glob="*"
keep=0
ttl=$max_ttl
allow_stale=0
from_holder=0
while [ $# -gt 0 ]; do
  case "$1:$cmd" in
    --scenario:run)
      [ $# -ge 2 ] || die 2 "--scenario needs a GLOB"
      scenario_glob="$2"
      shift 2
      ;;
    --keep:run)
      keep=1
      shift
      ;;
    --ttl:run | --ttl:up)
      [ $# -ge 2 ] || die 2 "--ttl needs minutes"
      ttl="$2"
      shift 2
      ;;
    --allow-stale:run | --allow-stale:up)
      allow_stale=1
      shift
      ;;
    --from-holder:down)
      from_holder=1
      shift
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
done
if ! [[ "$ttl" =~ ^[1-9][0-9]?$ ]] || [ "$ttl" -gt "$max_ttl" ]; then
  die 2 "--ttl $ttl: whole minutes from 1 to $max_ttl (PLAN R1: a running app stops within 15 minutes)"
fi
[ "$cmd" = up ] && keep=1
if ! [[ "$port" =~ ^[0-9]{1,5}$ ]] || [ "$port" -lt 1024 ] || [ "$port" -gt 65535 ]; then
  die 2 "WIKANVAS_AGENT_PORT=$port: a port from 1024 to 65535"
fi

project_up() {
  [ -n "$(docker ps -q --filter "label=com.docker.compose.project=$project" 2>/dev/null)" ]
}

compose_files() {
  local -a f=(-f "$root/compose.yaml" -f "$root/compose.e2e.yaml")
  if [ "${1:-0}" = 1 ]; then f+=(-f "$root/compose.agent.yaml"); fi
  printf '%s\n' "${f[@]}"
}

compose() {
  local with_port="$1"
  shift
  local -a files
  mapfile -t files < <(compose_files "$with_port")
  docker compose --env-file /dev/null -p "$project" "${files[@]}" "$@"
}

# --- status and down ------------------------------------------------------------------------------------------------
if [ "$cmd" = status ]; then
  if ! project_up; then
    echo "$project is not running"
    exit 3
  fi
  left=""
  if [ -f "$state/deadline" ]; then
    left=$(($(cat "$state/deadline") - $(date +%s)))
    [ "$left" -lt 0 ] && left=0
  fi
  holder="$(cat "$state/holder.pid" 2>/dev/null || true)"
  url="$(sed -n 's/^WIKANVAS_URL=//p' "$state/env" 2>/dev/null || true)"
  echo "$project is up${url:+ at $url}"
  if [ -n "$left" ]; then
    echo "time left: $((left / 60)) min $((left % 60)) s (holder pid ${holder:-?}, then down)"
  fi
  [ -f "$state/env" ] && echo "env: $state/env (WIKANVAS_URL, WIKANVAS_TOKEN_FILE)"
  exit 0
fi

if [ "$cmd" = down ]; then
  if [ "$from_holder" = 0 ] && [ -f "$state/holder.pid" ]; then
    holder="$(cat "$state/holder.pid")"
    # The holder leads its own process group (setsid): stop it and its sleep, which release the build lock.
    if [[ "$holder" =~ ^[0-9]+$ ]] && kill -0 "$holder" 2>/dev/null; then
      kill -TERM -- "-$holder" 2>/dev/null || kill -TERM "$holder" 2>/dev/null || true
    fi
  fi
  compose 1 down -v --remove-orphans >/dev/null 2>&1 || true
  rm -f "$state/env" "$state/read.token" "$state/write.token" "$state/admin.token" "$state/holder.pid" \
    "$state/deadline"
  echo "$project is down" >&2
  exit 0
fi

# --- run and up: prerequisites -----------------------------------------------------------------------------------
for tool in docker jq; do
  command -v "$tool" >/dev/null || die 3 "missing host prerequisite: $tool"
done
compose_version="$(docker compose version --short 2>/dev/null || true)"
if ! [[ "$compose_version" =~ ^v?([0-9]+)\.([0-9]+) ]] ||
  { [ "${BASH_REMATCH[1]}" -eq 2 ] && [ "${BASH_REMATCH[2]}" -lt 24 ]; } || [ "${BASH_REMATCH[1]}" -lt 2 ]; then
  die 3 "Docker Compose >= 2.24 is needed (found '${compose_version:-none}')"
fi
# Before locking: a project that is already up belongs to someone else's run (or an `up`).
if project_up; then
  die 3 "$project is already up: scripts/agent-test.sh status, or scripts/agent-test.sh down"
fi
if ! docker image inspect "$image" >/dev/null 2>&1; then
  die 3 "$image not found: run scripts/image.sh first"
fi
label="$(docker image inspect --format '{{ index .Config.Labels "org.opencontainers.image.revision" }}' "$image" 2>/dev/null || true)"
head="$(git -C "$root" rev-parse --short HEAD 2>/dev/null || echo nocommit)"
dirty_tree=""
[ -n "$(git -C "$root" status --porcelain 2>/dev/null)" ] && dirty_tree=1
if [ "${label%-dirty}" != "$head" ]; then
  if [ "$allow_stale" = 1 ]; then
    echo "agent-test: warning: $image is from ${label:-an unknown revision}, HEAD is $head (--allow-stale)" >&2
  else
    die 3 "$image is from ${label:-an unknown revision}, HEAD is $head: run scripts/image.sh (or pass --allow-stale)"
  fi
elif [ "$label" != "${label%-dirty}" ] || [ -n "$dirty_tree" ]; then
  echo "agent-test: warning: $image ($label) or the worktree has uncommitted changes" >&2
fi
with_port=0
if [ "$keep" = 1 ] || [ -n "${WIKANVAS_AGENT_CLI:-}" ]; then
  with_port=1
fi
if [ -n "${WIKANVAS_AGENT_CLI:-}" ] && [ ! -x "$WIKANVAS_AGENT_CLI" ]; then
  die 3 "WIKANVAS_AGENT_CLI=$WIKANVAS_AGENT_CLI is not an executable (build one with scripts/cli-dist.sh)"
fi
if [ "$with_port" = 1 ] && (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then
  die 3 "127.0.0.1:$port is taken: set WIKANVAS_AGENT_PORT to a free port"
fi

wikanvas_lock || exit 1
wikanvas_slice

# The owner-facing variables (§13.3) and the other settings compose.yaml reads never reach the test project: its
# caps and settings are always the defaults of compose.yaml.
unset WIKANVAS_DOMAIN WIKANVAS_TLS WIKANVAS_BIND WIKANVAS_HTTP_PORT WIKANVAS_HTTPS_PORT WIKANVAS_CPUS WIKANVAS_MEM WIKANVAS_CADDY_CPUS \
  WIKANVAS_CADDY_MEM WIKANVAS_NET_SUBNET WIKANVAS_NET_RANGE WIKANVAS_PROXY_IP
unset WIKANVAS_PUBLIC_URL WIKANVAS_ALLOWED_HOSTS WIKANVAS_BACKUP_INTERVAL WIKANVAS_BACKUP_KEEP WIKANVAS_MAX_UPLOAD_MB \
  WIKANVAS_MAX_BOARD_MB WIKANVAS_TOKEN_RATE WIKANVAS_TOKEN_BYTES_PER_HOUR WIKANVAS_MIN_FREE_MB WIKANVAS_CORS_ORIGINS
export WIKANVAS_NET_SUBNET=172.31.240.32/28 WIKANVAS_NET_RANGE=172.31.240.40/29 WIKANVAS_PROXY_IP=172.31.240.34
export WIKANVAS_IMAGE="$image" WIKANVAS_AGENT_PORT="$port"

kept=0
cleanup() {
  local rc=$?
  if [ "$kept" = 0 ]; then
    if [ "$rc" -ne 0 ] && [ "$rc" -ne 1 ]; then
      echo "--- last log lines of $project ---" >&2
      compose "$with_port" logs --no-color --tail 40 >&2 2>/dev/null || true
    fi
    compose "$with_port" down -v --remove-orphans >/dev/null 2>&1 || true
    rm -f "$state/env" "$state/read.token" "$state/write.token" "$state/admin.token" "$state/holder.pid" \
      "$state/deadline"
  fi
  exit "$rc"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# --- start and provision -----------------------------------------------------------------------------------------
compose "$with_port" down -v --remove-orphans >/dev/null 2>&1 || true
echo "starting $project ($image${label:+, $label})" >&2
if ! compose "$with_port" up -d --wait --wait-timeout 120 wikanvas >&2; then
  die 4 "$project did not come up healthy"
fi
cid="$(compose "$with_port" ps -q wikanvas)"
[ -n "$cid" ] || die 4 "$project has no wikanvas container"

# Inside the container: the CLI execs wikanvas-server admin. The generated password and the tokens stay in variables
# and are never printed.
if ! owner_json="$(docker exec "$cid" wikanvas --json admin create-owner --email agent@wikanvas.invalid --name Agent \
  --workspace "Agent test" --generate-password)"; then
  die 4 "wikanvas admin create-owner failed"
fi
unset owner_json
declare -A token=()
for scope in read write admin; do
  # 2d, not §14.1's 1d: a token cannot outlive the token that creates it (to the millisecond), and the scenarios
  # create 1d tokens with these.
  if ! out="$(docker exec "$cid" wikanvas --json admin create-token --name "agent-$scope" --scope "$scope" --expires 2d)"; then
    die 4 "wikanvas admin create-token --scope $scope failed"
  fi
  token[$scope]="$(jq -r .token <<<"$out")"
  unset out
  [[ "${token[$scope]}" == qpat_* ]] || die 4 "create-token --scope $scope printed no token"
done
url="http://127.0.0.1:$port"

# Leaves the app up for the TTL: the env file and the token files, then a holder that keeps the build lock (an
# inherited descriptor) and runs down at the end.
keep_up() {
  (
    umask 077
    mkdir -p "$state"
    for scope in read write admin; do printf '%s\n' "${token[$scope]}" >"$state/$scope.token"; done
    printf 'WIKANVAS_URL=%s\nWIKANVAS_TOKEN_FILE=%s\n' "$url" "$state/write.token" >"$state/env"
    echo $(($(date +%s) + ttl * 60)) >"$state/deadline"
  )
  setsid bash -c 'trap "exit 0" TERM; sleep "$1" & wait $!; exec "$2" down --from-holder' agent-test-holder \
    "$((ttl * 60))" "$root/scripts/agent-test.sh" </dev/null >/dev/null 2>&1 &
  echo "$!" >"$state/holder.pid"
  kept=1
  echo "$project is up at $url for $ttl min (scripts/agent-test.sh status | down); env: $state/env" >&2
}

if [ "$cmd" = up ]; then
  keep_up
  exit 0
fi

# --- run the scenarios -------------------------------------------------------------------------------------------
rm -rf "$results"
mkdir -p "$results"
steps="$results/steps.jsonl"
counter="$results/.counter"
: >"$steps"
echo 0 >"$counter"
started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
mask() { sed -E 's/qpat_[0-9A-Za-z]{46}/qpat_***/g'; }

mapfile -t scenarios < <(find "$root/e2e/agent/scenarios" -maxdepth 1 -name '[0-9][0-9]-*.sh' -name "$scenario_glob" | sort)
if [ "${#scenarios[@]}" -eq 0 ]; then
  die 2 "no scenario matches --scenario $scenario_glob"
fi
echo "TAP version 14"
failed=0
for file in "${scenarios[@]}"; do
  name="$(basename "$file" .sh)"
  echo "# $name"
  set +e
  AGENT_SCENARIO="$name" AGENT_ROOT="$root" AGENT_RESULTS="$results" AGENT_STEPS="$steps" AGENT_COUNTER="$counter" \
    AGENT_CID="$cid" AGENT_URL="$url" AGENT_TOKEN_READ="${token[read]}" AGENT_TOKEN_WRITE="${token[write]}" \
    AGENT_TOKEN_ADMIN="${token[admin]}" bash "$file" </dev/null 2>&1 | mask
  rc="${PIPESTATUS[0]}"
  set -e
  if [ "$rc" -ne 0 ]; then
    failed=$((failed + 1))
    if ! jq -e --arg s "$name" 'select(.scenario == $s and .status == "fail")' "$steps" >/dev/null 2>&1; then
      n=$(($(cat "$counter") + 1))
      echo "$n" >"$counter"
      echo "not ok $n - $name: the scenario exited $rc"
      jq -cn --arg s "$name" --argjson n "$n" --argjson rc "$rc" \
        '{n: $n, scenario: $s, step: "the scenario exited \($rc)", status: "fail", duration_ms: 0}' >>"$steps"
    fi
  fi
done
total="$(cat "$counter")"
echo "1..$total"
finished="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
jq -s --arg image "$image" --arg revision "$label" --arg head "$head" --arg started "$started" \
  --arg finished "$finished" --argjson failed_scenarios "$failed" \
  '{image: $image, revision: $revision, head: $head, started: $started, finished: $finished,
    passed: map(select(.status == "pass")) | length, failed: map(select(.status == "fail")) | length,
    failed_scenarios: $failed_scenarios, steps: .}' "$steps" | mask >"$results/report.json"
rm -f "$results/.out" "$results/.err" "$counter"
echo "# report: $results/report.json ($failed failed scenario(s))" >&2

if [ "$keep" = 1 ]; then
  keep_up
fi
[ "$failed" -eq 0 ]
