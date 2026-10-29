#!/usr/bin/env bash
# Runs a command in the wikanvas-toolchain container with CPU and memory caps, at low priority (docs/ARCHITECTURE.md
# §13.1). Every cargo, rustc, trunk, wasm-bindgen, node and pnpm command for Wikanvas goes through this script; nothing
# is compiled on the host.
#   scripts/dev.sh cargo build
#   scripts/dev.sh pnpm --dir web install
#   WIKANVAS_DEV_PORTS=127.0.0.1:8080:8080 scripts/dev.sh cargo run    # publish a port for a dev server
#
# Caps (may only be lowered): WIKANVAS_BUILD_CPUS (default 6), WIKANVAS_BUILD_MEM (default 12g, no swap),
# WIKANVAS_BUILD_CPUSET (default: the host's last WIKANVAS_BUILD_CPUS CPUs). The container joins wikanvas-build.slice
# when it is installed (scripts/lib/slice.sh); without it the per-container caps still hold, after a warning.
# Builds queue on one per-user lock (scripts/lib/lock.sh).
# Optional: WIKANVAS_DEV_IMAGE (default wikanvas-toolchain:latest), WIKANVAS_DEV_NETWORK (a Docker network to join),
# WIKANVAS_DEV_PORTS (a -p value), WIKANVAS_SHARED_CARGO_HOME (an absolute host path used as CARGO_HOME, so worktrees
# share one registry).
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
. "$root/scripts/lib/lock.sh"
. "$root/scripts/lib/slice.sh"

wikanvas_caps || exit 2
if [ $# -eq 0 ]; then
  echo "usage: scripts/dev.sh <command> [args…]" >&2
  exit 2
fi
if [ -n "${WIKANVAS_SHARED_CARGO_HOME:-}" ] && [[ "$WIKANVAS_SHARED_CARGO_HOME" != /* ]]; then
  echo "WIKANVAS_SHARED_CARGO_HOME=$WIKANVAS_SHARED_CARGO_HOME: must be an absolute path" >&2
  exit 2
fi

wikanvas_lock || exit 1
wikanvas_slice

# Caches live in the repo (git-ignored) so they survive container runs and stay owned by the host user.
mkdir -p "$root/.cache/cargo" "$root/.cache/home"

opts=(--rm --init
  --cpus="$wikanvas_cpus" --memory="$wikanvas_mem_bytes" --memory-swap="$wikanvas_mem_bytes" --pids-limit=4096 --shm-size=1g
  --user "$(id -u):$(id -g)"
  -e CARGO_HOME=/work/.cache/cargo -e HOME=/work/.cache/home
  -e CARGO_BUILD_JOBS="$wikanvas_threads" -e BINARYEN_CORES="$wikanvas_threads" -e RAYON_NUM_THREADS="$wikanvas_threads"
  -e GOMAXPROCS="$wikanvas_threads" -e UV_THREADPOOL_SIZE="$wikanvas_threads"
  -v "$root:/work" -w /work)
if [ -n "$wikanvas_cpuset" ]; then opts+=(--cpuset-cpus="$wikanvas_cpuset"); fi
if [ -n "$WIKANVAS_CGROUP_PARENT" ]; then opts+=(--cgroup-parent="$WIKANVAS_CGROUP_PARENT"); fi
if [ -n "${WIKANVAS_SHARED_CARGO_HOME:-}" ]; then
  # Mounted over the worktree's own CARGO_HOME, so the in-container paths (and cargo's fingerprints) stay the same.
  mkdir -p "$WIKANVAS_SHARED_CARGO_HOME"
  opts+=(-v "$WIKANVAS_SHARED_CARGO_HOME:/work/.cache/cargo")
fi
if [ -n "${WIKANVAS_DEV_NETWORK:-}" ]; then opts+=(--network="$WIKANVAS_DEV_NETWORK"); fi
if [ -n "${WIKANVAS_DEV_PORTS:-}" ]; then opts+=(-p "$WIKANVAS_DEV_PORTS"); fi
if [ -t 0 ] && [ -t 1 ]; then opts+=(-it); fi

# exec keeps the lock's descriptor open in the docker client until the container has exited.
exec docker run "${opts[@]}" "${WIKANVAS_DEV_IMAGE:-wikanvas-toolchain:latest}" nice -n 10 "$@"
