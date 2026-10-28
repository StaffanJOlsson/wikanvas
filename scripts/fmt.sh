#!/usr/bin/env bash
# Formats Quire's own crates with rustfmt, in the capped toolchain container (docs/ARCHITECTURE.md §4.1).
#   scripts/fmt.sh           # format
#   scripts/fmt.sh --check   # only check; exit 1 when a file would change
# Always this script, never `cargo fmt --all`: --all follows the inkboard-core path dependency into Inkboard's own
# workspace and reformats (or fails on) vendor/inkboard. The package list is the workspace's six crates.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"

packages=(quire-api quire-core quire-store quire-server quire-assets quire-cli)

check=()
case "$#:${1:-}" in
  0:) ;;
  1:--check) check=(--check) ;;
  1:-h | 1:--help)
    sed -n '2,6s/^# \{0,1\}//p' "$root/scripts/fmt.sh"
    exit 0
    ;;
  *)
    sed -n '2,6s/^# \{0,1\}//p' "$root/scripts/fmt.sh" >&2
    exit 2
    ;;
esac

args=()
for p in "${packages[@]}"; do args+=(-p "$p"); done
cd "$root"
exec "$root/scripts/dev.sh" cargo fmt "${args[@]}" "${check[@]}"
