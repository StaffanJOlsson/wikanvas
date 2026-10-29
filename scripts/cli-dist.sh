#!/usr/bin/env bash
# Builds the standalone wikanvas CLI (docs/ARCHITECTURE.md §13.2, D54) with the capped wikanvas-limited builder, under
# the build lock: the cli-dist target of docker/Dockerfile, exported with --output type=local.
#   scripts/cli-dist.sh            # dist/cli/wikanvas-x86_64-unknown-linux-musl (+ .sha256, LICENSE, THIRD_PARTY.md)
# Extra arguments go to `docker buildx build` unchanged. The binary is static (musl) and runs on any x86_64 Linux.
# The output is staged in dist/.cli.XXXXXX and renamed to dist/cli only once complete. The builder is set up or
# checked first (scripts/toolchain.sh --builder-only), as scripts/image.sh does.
# Exit codes: 0 built, 1 the build failed, 2 usage.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
. "$root/scripts/lib/lock.sh"

name=wikanvas-x86_64-unknown-linux-musl
case "${1:-}" in
  -h | --help)
    sed -n '2,8s/^# \{0,1\}//p' "$root/scripts/cli-dist.sh"
    exit 0
    ;;
esac

wikanvas_lock || exit 1
# Nested under the lock (WIKANVAS_LOCK_HELD): it never waits for it again.
"$root/scripts/toolchain.sh" --builder-only

mkdir -p "$root/dist"
stage="$(mktemp -d "$root/dist/.cli.XXXXXX")"
trap 'rm -rf "$stage"' EXIT

cd "$root"
docker buildx build --builder wikanvas-limited -f docker/Dockerfile --target cli-dist \
  --output "type=local,dest=$stage" "$@" .

for f in "$name" LICENSE THIRD_PARTY.md; do
  if [ ! -s "$stage/$f" ]; then
    echo "cli-dist: the build did not produce $f" >&2
    exit 1
  fi
done
chmod 0755 "$stage" "$stage/$name"
(cd "$stage" && sha256sum "$name" >"$name.sha256")

rm -rf "$root/dist/cli"
mv "$stage" "$root/dist/cli"
trap - EXIT
echo "built dist/cli/$name ($(cut -c1-12 "$root/dist/cli/$name.sha256")…)" >&2
