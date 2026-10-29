#!/usr/bin/env bash
# Builds the production image wikanvas:<commit> and wikanvas:latest (the runtime target of docker/Dockerfile) with the
# capped wikanvas-limited builder, under the build lock (docs/ARCHITECTURE.md §13.1, §13.2).
#   scripts/image.sh                                   # full build: web app, Inkboard engine, server, CLI
#   scripts/image.sh --build-arg INKBOARD_BUILD=stub   # with the placeholder engine dist (scripts/inkboard.sh stub)
# Extra arguments go to `docker buildx build` unchanged. The tag is `git rev-parse --short HEAD` (nocommit outside
# git), plus -dirty when `git status --porcelain` is not empty; it is also the image's
# org.opencontainers.image.revision label and WIKANVAS_COMMIT, so scripts/agent-test.sh can refuse a stale image.
# The builder is set up or checked first (scripts/toolchain.sh --builder-only): a builder whose caps or cgroup parent
# drifted stops the build with the recreate instruction.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
. "$root/scripts/lib/lock.sh"

case "${1:-}" in
  -h | --help)
    sed -n '2,10s/^# \{0,1\}//p' "$root/scripts/image.sh"
    exit 0
    ;;
esac

# Git runs on the host (PLAN R11), never in a build container.
tag="$(git -C "$root" rev-parse --short HEAD 2>/dev/null || echo nocommit)"
if [ "$tag" != nocommit ] && [ -n "$(git -C "$root" status --porcelain 2>/dev/null)" ]; then
  tag="$tag-dirty"
fi

wikanvas_lock || exit 1
# Nested under the lock (WIKANVAS_LOCK_HELD): it never waits for it again.
"$root/scripts/toolchain.sh" --builder-only

cd "$root"
docker buildx build --builder wikanvas-limited --load -f docker/Dockerfile --target runtime \
  --build-arg WIKANVAS_COMMIT="$tag" --label org.opencontainers.image.revision="$tag" \
  -t "wikanvas:$tag" -t wikanvas:latest "$@" .
echo "built wikanvas:$tag and wikanvas:latest" >&2
