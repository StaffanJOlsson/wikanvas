#!/usr/bin/env bash
# Builds Quire's Inkboard engine dist (docs/ARCHITECTURE.md §10.5). Always run it through scripts/dev.sh or inside the
# inkboard-web stage of docker/Dockerfile. It never runs git against the main repository, so it works the same in the
# main tree, in a git worktree and in the Docker build context.
#   scripts/inkboard.sh stub [--out DIR]       # placeholder dist: frame page "engine not built" + manifest.json
#   scripts/inkboard.sh src                    # (W5) rebuild .cache/inkboard-src: base copy + patches/inkboard/*.patch
#   scripts/inkboard.sh build [--out DIR]      # (W5) the wasm engine, the frame page, precompressed, manifest.json
#   scripts/inkboard.sh export [--upstream]    # (W5) regenerate patches/inkboard/*.patch from .cache/inkboard-src
#   scripts/inkboard.sh check                  # (W5) check + clippy for every feature set of the applied patches
#   scripts/inkboard.sh size                   # (W5) raw, br and gz sizes of the .wasm, with and without patch 0003
# DIR defaults to web/dist-inkboard. The dist is flat; manifest.json describes it (see write_manifest below and
# patches/inkboard/README.md). A previous dist in DIR is replaced; a non-empty DIR without a manifest.json is refused.
# Never builds inside vendor/inkboard and never modifies it.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
inkboard="$root/vendor/inkboard"
serial_rs="$inkboard/crates/inkboard-core/src/serial.rs"
default_out="$root/web/dist-inkboard"

usage() {
  sed -n '2,13s/^# \{0,1\}//p' "$root/scripts/inkboard.sh"
}

die() {
  echo "scripts/inkboard.sh: $*" >&2
  exit 1
}

# The board format version the engine reads and writes: `pub const VERSION: u32 = N;` in inkboard-core's serial.rs,
# exactly once. The server refuses to start when the embedded manifest disagrees with inkboard_core::serial::VERSION.
format_version() {
  local v
  [ -r "$serial_rs" ] || die "cannot read $serial_rs (is the vendor/inkboard submodule checked out?)"
  v="$(sed -n 's/^pub const VERSION: u32 = \([0-9][0-9]*\);.*$/\1/p' "$serial_rs")"
  [[ "$v" =~ ^[0-9]{1,9}$ ]] || die "expected exactly one 'pub const VERSION: u32 = N;' in $serial_rs"
  echo "$((10#$v))"
}

# Parses `--out DIR` (the only option of stub and build) into $out, as an absolute path.
parse_out() {
  out="$default_out"
  while [ $# -gt 0 ]; do
    case "$1" in
      --out)
        [ $# -ge 2 ] && [ -n "$2" ] || die "--out needs a directory"
        out="$2"
        shift 2
        ;;
      --out=*)
        out="${1#--out=}"
        [ -n "$out" ] || die "--out needs a directory"
        shift
        ;;
      *)
        usage >&2
        exit 2
        ;;
    esac
  done
  case "$out" in
    /*) ;;
    *) out="$PWD/$out" ;;
  esac
  out="${out%/}"
  [ -n "$out" ] || die "--out / is not a dist directory"
}

# A fresh staging directory next to $out (same filesystem, so the final swap is a rename). The EXIT trap removes it
# when the command fails before the swap.
stage_dir() {
  mkdir -p "$(dirname "$out")"
  stage="$(mktemp -d "$(dirname "$out")/.$(basename "$out").XXXXXX")"
  trap 'rm -rf "$stage"' EXIT
  chmod 0755 "$stage"
}

# Replaces $out with $stage. Only an empty directory or a previous dist (it has a manifest.json) is replaced, so a
# mistyped --out can never delete anything else.
swap_into_place() {
  if [ -e "$out" ] || [ -L "$out" ]; then
    if [ -L "$out" ] || [ ! -d "$out" ]; then
      die "$out exists and is not a directory"
    fi
    if [ -n "$(ls -A "$out")" ] && [ ! -f "$out/manifest.json" ]; then
      die "$out is not empty and has no manifest.json: not a previous Inkboard dist, refusing to replace it"
    fi
    rm -rf "$out"
  fi
  mv "$stage" "$out"
  trap - EXIT
}

# Precompresses each named file in $stage with brotli -q 11 and gzip -9 -n, keeping the original (name.br, name.gz).
# build applies it to .wasm .js .html .css only (never icons.bin).
precompress() {
  local f
  command -v brotli >/dev/null || die "brotli not found: rebuild the toolchain image (scripts/toolchain.sh)"
  for f in "$@"; do
    brotli -q 11 -k "$stage/$f"
    gzip -9 -k -n "$stage/$f"
  done
}

# Writes $stage/manifest.json for the flat dist in $stage:
#   build           first 12 hex chars of the sha256 over all files of the dist (the sha256sum lines of every file
#                   except manifest.json, .br and .gz variants included, sorted by name); the server serves the dist
#                   under /inkboard/<build>/
#   engine_version  "stub", or 0.2.0-beta.3+embed.quire.<patchset> for a real build
#   format_version  from serial.rs (format_version above), a number
#   patchset        "stub", or the patch series id recorded in patches/inkboard/README.md
#   files           every file except manifest.json and the .br/.gz variants: {raw, br, gz} byte counts, where br and
#                   gz are null when that variant does not exist
# Usage: write_manifest ENGINE_VERSION PATCHSET FORMAT_VERSION
write_manifest() {
  local engine="$1" patchset="$2" fmt="$3" build name sep="" f
  local -a names=()
  [[ "$engine$patchset" =~ ^[A-Za-z0-9.+_-]+$ ]] || die "engine_version or patchset has unexpected characters"
  [[ "$fmt" =~ ^[0-9]+$ ]] || die "format_version '$fmt' is not a number"
  while IFS= read -r name; do
    [[ "$name" =~ ^[A-Za-z0-9._-]+$ ]] || die "dist file name '$name' has unexpected characters"
    case "$name" in
      manifest.json | *.br | *.gz) ;;
      *) names+=("$name") ;;
    esac
  done < <(find "$stage" -mindepth 1 -maxdepth 1 -printf '%P\n' | LC_ALL=C sort)
  if [ -n "$(find "$stage" -mindepth 1 ! -type f -print -quit)" ]; then
    die "the dist must be flat regular files only"
  fi
  build="$(cd "$stage" && find . -maxdepth 1 -type f ! -name manifest.json -printf '%P\n' | LC_ALL=C sort |
    while IFS= read -r f; do sha256sum -- "$f"; done | sha256sum | cut -c1-12)"
  _size_of() { if [ -f "$stage/$1" ]; then stat -c %s -- "$stage/$1"; else echo null; fi; }
  {
    printf '{\n'
    printf '  "build": "%s",\n' "$build"
    printf '  "engine_version": "%s",\n' "$engine"
    printf '  "format_version": %s,\n' "$fmt"
    printf '  "patchset": "%s",\n' "$patchset"
    printf '  "files": {'
    for name in "${names[@]}"; do
      printf '%s\n    "%s": { "raw": %s, "br": %s, "gz": %s }' "$sep" "$name" \
        "$(_size_of "$name")" "$(_size_of "$name.br")" "$(_size_of "$name.gz")"
      sep=","
    done
    printf '\n  }\n}\n'
  } >"$stage/manifest.json"
}

cmd_stub() {
  local fmt
  parse_out "$@"
  fmt="$(format_version)"
  stage_dir
  # The same frame contract as the real page (§10.4): connect() reports `unsupported` and rejects, so a board host
  # falls back to the preview. No inline script, so the frame CSP holds.
  cat >"$stage/frame.html" <<'EOF'
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Inkboard engine not built</title>
<script type="module" src="frame.js"></script>
</head>
<body>
<p>Inkboard engine not built. This is the placeholder from <code>scripts/inkboard.sh stub</code>; build the engine
with <code>scripts/dev.sh scripts/inkboard.sh build</code>.</p>
</body>
</html>
EOF
  cat >"$stage/frame.js" <<'EOF'
// Placeholder written by scripts/inkboard.sh stub: the Inkboard engine is not built into this dist.
const message = 'Inkboard engine not built (scripts/inkboard.sh stub)';
window.quireBoardFrame = {
  connect(events) {
    if (events && typeof events.onError === 'function') {
      events.onError({ itemId: null, kind: 'unsupported', message });
    }
    return Promise.reject({ kind: 'unsupported', message });
  },
};
EOF
  chmod 0644 "$stage/frame.html" "$stage/frame.js"
  precompress frame.html frame.js
  write_manifest stub stub "$fmt"
  swap_into_place
  echo "wrote the Inkboard stub dist to $out (format_version $fmt)" >&2
}

case "${1:-}" in
  stub)
    shift
    cmd_stub "$@"
    ;;
  src | build | export | check | size)
    echo "scripts/inkboard.sh $1: not implemented (W5)" >&2
    exit 1
    ;;
  -h | --help)
    usage
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac
