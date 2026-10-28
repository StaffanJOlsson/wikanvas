#!/usr/bin/env bash
# Runs Quire's Playwright tests, capped and under the build lock (docs/ARCHITECTURE.md §14).
#   scripts/e2e.sh frame [playwright args…]   # e2e/tests/frame.spec.ts: the Inkboard frame page from web/dist-inkboard,
#                                             # served by e2e/serve-static.mjs inside the Playwright container
#   scripts/e2e.sh smoke [playwright args…]   # e2e/tests/smoke.spec.ts: the production image ${QUIRE_IMAGE:-quire:latest}
#                                             # in project quire-e2e (compose.yaml + compose.e2e.yaml, quire only)
# Steps: take the build lock and call quire_slice; install e2e's packages (scripts/dev.sh pnpm --dir e2e install
# --frozen-lockfile); for smoke, start only the quire service on the e2e subnet (172.31.240.16/28, range .24/29, proxy
# address .18) with --env-file /dev/null and the owner-facing variables unset, removed again on exit (down -v); then
# Playwright in mcr.microsoft.com/playwright:v1.64.1-noble through scripts/dev.sh with 4 CPUs and 6 GiB (lower when
# QUIRE_BUILD_CPUS, QUIRE_BUILD_MEM or QUIRE_BUILD_CPUSET are lower), joining quire-e2e_default for smoke. Playwright
# reaches Quire directly at http://quire:8080 (plain HTTP); Caddy is never started (the HTTPS path is the image
# smoke's, scripts/image-smoke.sh). The app (2 CPUs, 1 GiB) plus Playwright (4 CPUs, 6 GiB) fit the 6-CPU build total.
# Exit codes: Playwright's (0 passed, 1 failed), 2 usage or caps refused, 3 the image is missing, 4 the app did not
# become healthy.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
. "$root/scripts/lib/lock.sh"
. "$root/scripts/lib/slice.sh"

# The image matches @playwright/test 1.64.1 in e2e/package.json (§4.3); pinned by index digest (2026-10-28).
PLAYWRIGHT_IMAGE=mcr.microsoft.com/playwright:v1.64.1-noble@sha256:47b510dd9230bf0b9bfc9b20102241e577ea7a99334ab4c3ca19f19770e9b8ea
PW_MAX_CPUS=4
PW_MAX_MEM=6442450944 # 6g

usage() {
  sed -n '2,16s/^# \{0,1\}//p' "$root/scripts/e2e.sh"
}
case "${1:-}" in
  frame | smoke)
    suite="$1"
    shift
    ;;
  -h | --help)
    usage
    exit 0
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac

# Playwright's caps: 4 CPUs and 6 GiB, or the caller's lower values. quire_caps validates QUIRE_BUILD_* (it refuses
# anything above the build caps) and derives the CPU set.
quire_caps || exit 2
pw_cpus="$quire_cpus"
[ "$pw_cpus" -le "$PW_MAX_CPUS" ] || pw_cpus="$PW_MAX_CPUS"
pw_mem="$quire_mem_bytes"
[ "$pw_mem" -le "$PW_MAX_MEM" ] || pw_mem="$PW_MAX_MEM"
pw_env=(QUIRE_DEV_IMAGE="$PLAYWRIGHT_IMAGE" QUIRE_BUILD_CPUS="$pw_cpus" QUIRE_BUILD_MEM="$pw_mem")
if [ -n "${QUIRE_BUILD_CPUSET:-}" ]; then
  # An explicit CPU set wider than Playwright's CPUs keeps its last ones (dev.sh refuses a set wider than the count).
  pw_env+=(QUIRE_BUILD_CPUSET="$(_quire_cpus_of "$QUIRE_BUILD_CPUSET" | sort -un | tail -n "$pw_cpus" | _quire_cpuset_of)")
fi

quire_lock || exit 1
quire_slice

cd "$root"
"$root/scripts/dev.sh" pnpm --dir e2e install --frozen-lockfile

if [ "$suite" = smoke ]; then
  # Neither the owner's .env nor the shell's owner-facing variables reach the e2e project (§13.3): its caps and
  # settings are compose.yaml's defaults. QUIRE_IMAGE (the image under test) and RUST_LOG are kept.
  unset QUIRE_DOMAIN QUIRE_TLS QUIRE_BIND QUIRE_HTTP_PORT QUIRE_HTTPS_PORT QUIRE_CPUS QUIRE_MEM QUIRE_CADDY_CPUS \
    QUIRE_CADDY_MEM QUIRE_NET_SUBNET QUIRE_NET_RANGE QUIRE_PROXY_IP
  unset QUIRE_PUBLIC_URL QUIRE_ALLOWED_HOSTS QUIRE_BACKUP_INTERVAL QUIRE_BACKUP_KEEP QUIRE_MAX_UPLOAD_MB \
    QUIRE_MAX_BOARD_MB QUIRE_TOKEN_RATE QUIRE_TOKEN_BYTES_PER_HOUR QUIRE_MIN_FREE_MB QUIRE_CORS_ORIGINS
  export QUIRE_NET_SUBNET=172.31.240.16/28 QUIRE_NET_RANGE=172.31.240.24/29 QUIRE_PROXY_IP=172.31.240.18
  image="${QUIRE_IMAGE:-quire:latest}"
  if ! docker image inspect "$image" >/dev/null 2>&1; then
    echo "e2e: $image not found: run scripts/image.sh first" >&2
    exit 3
  fi
  compose() {
    docker compose --env-file /dev/null -p quire-e2e -f "$root/compose.yaml" -f "$root/compose.e2e.yaml" "$@"
  }
  cleanup() {
    local rc=$?
    if [ "$rc" -ne 0 ]; then
      echo "--- last log lines of quire-e2e ---" >&2
      compose logs --no-color --tail 40 quire >&2 2>/dev/null || true
    fi
    compose down -v --remove-orphans >/dev/null 2>&1 || true
    exit "$rc"
  }
  trap cleanup EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
  # A leftover project from an interrupted run would keep its old container and data.
  compose down -v --remove-orphans >/dev/null 2>&1 || true
  if ! compose up -d --wait --wait-timeout 120 quire; then
    echo "e2e: quire did not become healthy in project quire-e2e" >&2
    exit 4
  fi
  pw_env+=(QUIRE_DEV_NETWORK=quire-e2e_default)
fi

rc=0
env "${pw_env[@]}" "$root/scripts/dev.sh" node e2e/node_modules/@playwright/test/cli.js test \
  -c e2e/playwright.config.ts --project "$suite" "$@" || rc=$?
exit "$rc"
