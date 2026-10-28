#!/usr/bin/env bash
# Builds Quire's Inkboard engine dist (docs/ARCHITECTURE.md §10.5). Always run it through scripts/dev.sh or inside the
# inkboard-web stage of docker/Dockerfile. It never runs git against the main repository, so it works the same in the
# main tree, in a git worktree and in the Docker build context.
#   scripts/inkboard.sh src [--force]              # rebuild .cache/inkboard-src: base copy + patches/inkboard/*.patch
#   scripts/inkboard.sh build [--out DIR] [--quick]  # the wasm engine, the frame page, precompressed, manifest.json
#   scripts/inkboard.sh stub [--out DIR]           # placeholder dist: frame page "engine not built" + manifest.json
#   scripts/inkboard.sh export [--upstream]        # regenerate patches/inkboard/*.patch from .cache/inkboard-src
#   scripts/inkboard.sh check [--record-baseline]  # check + clippy for every feature set of the applied patches
#   scripts/inkboard.sh size                       # raw, br and gz sizes of the .wasm, with and without patch 0003
# DIR defaults to web/dist-inkboard. The dist is flat; manifest.json describes it (see write_manifest below and
# patches/inkboard/README.md). A previous dist in DIR is replaced; a non-empty DIR without a manifest.json is refused.
# --quick (iteration only, never for an image): release code without LTO, incremental, no wasm-opt, fast compression.
# Never builds inside vendor/inkboard and never modifies it. INKBOARD_TARGET_DIR overrides .cache/inkboard-target.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
inkboard="$root/vendor/inkboard"
serial_rs="$inkboard/crates/inkboard-core/src/serial.rs"
default_out="$root/web/dist-inkboard"
patch_dir="$root/patches/inkboard"
src_dir="$root/.cache/inkboard-src"
upstream_dir="$root/.cache/inkboard-upstream"
target_dir="${INKBOARD_TARGET_DIR:-$root/.cache/inkboard-target}"
frame_dir="$root/web/inkboard-frame"
baseline_file="$patch_dir/check-baseline.txt"
stamp_file="$src_dir/.git/quire-src-stamp"
# The only Inkboard version this series is written against (§10.2); src refuses any other.
INKBOARD_VERSION=0.2.0-beta.3
# Every changed file carries one of these lines (§10.2); export --upstream removes them.
NOTICE='Changed for Quire (embed feature): see patches/inkboard/README.md'
NOTICE_RE='^(// |# |<!-- )Changed for Quire \(embed feature\): see patches/inkboard/README\.md( -->)?$'
# wasm-opt as trunk 0.22.1 runs it for Inkboard's web/index.html (`data-wasm-opt="s"`, no data-reference-types and no
# data-wasm-opt-params): `wasm-opt --output=OUT -Os IN` and nothing else, read from trunk's src/pipelines/rust/mod.rs
# (wasm_opt_build). wasm-bindgen as trunk runs it: --target web --no-typescript (plus --out-dir/--out-name).
WASM_OPT_FLAGS=(-Os)
# The feature set of the engine Quire ships (§10.5).
ENGINE_FEATURES=(--no-default-features --features web-glow,embed)

usage() {
  sed -n '2,15s/^# \{0,1\}//p' "$root/scripts/inkboard.sh"
}

die() {
  echo "scripts/inkboard.sh: $*" >&2
  exit 1
}

# git on the self-contained copy only (never on Quire's repository), with a fixed identity and no user hooks.
git_src() {
  git -C "$src_dir" -c user.name=quire -c user.email=quire@localhost -c commit.gpgsign=false -c core.hooksPath=/dev/null \
    -c core.autocrlf=false -c advice.detachedHead=false "$@"
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

# The patch series id: the `| Patchset | \`N\` …` row of patches/inkboard/README.md, exactly once.
patchset() {
  local v
  v="$(sed -n 's/^| Patchset | `\([0-9A-Za-z.]*\)`.*$/\1/p' "$patch_dir/README.md")"
  [[ "$v" =~ ^[0-9A-Za-z.]+$ ]] || die "expected exactly one '| Patchset | \`N\` …' row in $patch_dir/README.md"
  echo "$v"
}

check_version() {
  grep -qx "version = \"$INKBOARD_VERSION\"" "$inkboard/Cargo.toml" ||
    die "vendor/inkboard/Cargo.toml is not version $INKBOARD_VERSION: the patch series is written against that release"
}

# Parses `--out DIR` (the only option of stub, and of build besides --quick) into $out, as an absolute path.
# Unknown arguments are left in $rest.
parse_out() {
  out="$default_out"
  rest=()
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
        rest+=("$1")
        shift
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

# Precompresses each named file in $stage with brotli and gzip (-q 11 and -9 -n, or faster levels with --quick),
# keeping the original (name.br, name.gz). build applies it to .wasm .js .html .css only (never icons.bin).
precompress() {
  local f
  command -v brotli >/dev/null || die "brotli not found: rebuild the toolchain image (scripts/toolchain.sh)"
  for f in "$@"; do
    brotli -q "${brotli_level:-11}" -k "$stage/$f"
    gzip "-${gzip_level:-9}" -k -n "$stage/$f"
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
  [ ${#rest[@]} -eq 0 ] || {
    usage >&2
    exit 2
  }
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

# ---------------------------------------------------------------------------------------------- the patched copy

# What .cache/inkboard-src is made of: the submodule's files (path, size, mtime) and the patch files (content). A
# change to either makes the copy stale.
src_inputs() {
  {
    (cd "$inkboard" && find . \( -path ./.git -o -path ./target -o -path ./crates/inkboard-app/web/dist \) -prune -o \
      -type f -printf '%P %s %T@\n' | LC_ALL=C sort)
    if compgen -G "$patch_dir/*.patch" >/dev/null; then
      (cd "$patch_dir" && sha256sum -- *.patch)
    fi
  } | sha256sum | cut -c1-64
}

write_stamp() {
  printf 'inputs %s\nhead %s\n' "$(src_inputs)" "$(git_src rev-parse HEAD)" >"$stamp_file"
}

stamp_field() {
  [ -f "$stamp_file" ] && sed -n "s/^$1 //p" "$stamp_file"
}

# Refuses to throw away work in .cache/inkboard-src: uncommitted changes, or commits not exported to patches/.
guard_src_work() {
  [ -d "$src_dir/.git" ] || return 0
  if [ -n "$(git_src status --porcelain)" ]; then
    die "$src_dir has uncommitted changes: commit and export them, or pass src --force to discard them"
  fi
  if [ -f "$stamp_file" ] && [ "$(git_src rev-parse HEAD)" != "$(stamp_field head)" ]; then
    die "$src_dir has commits that are not exported: run export first, or pass src --force to discard them"
  fi
}

cmd_src() {
  local force=0 patches=()
  while [ $# -gt 0 ]; do
    case "$1" in
      --force) force=1 ;;
      *)
        usage >&2
        exit 2
        ;;
    esac
    shift
  done
  check_version
  command -v git >/dev/null || die "git not found"
  [ "$force" -eq 1 ] || guard_src_work
  rm -rf "$src_dir"
  mkdir -p "$src_dir"
  # The submodule's .git is a gitlink file pointing outside the container: never copied, never used.
  tar -C "$inkboard" --exclude=./.git --exclude=./target --exclude=./crates/inkboard-app/web/dist -cf - . |
    tar -C "$src_dir" -xf -
  git -C "$src_dir" -c init.defaultBranch=main init -q
  git_src add -A
  GIT_AUTHOR_DATE=2026-10-27T00:00:00Z GIT_COMMITTER_DATE=2026-10-27T00:00:00Z \
    git_src commit -q -m "base: Inkboard $INKBOARD_VERSION (vendor/inkboard)"
  git_src tag base
  if compgen -G "$patch_dir/*.patch" >/dev/null; then
    patches=("$patch_dir"/*.patch)
    if ! git_src am --no-3way --quiet "${patches[@]}"; then
      git_src am --abort >/dev/null 2>&1 || true
      die "the patch series does not apply to the base copy without fuzz"
    fi
  fi
  write_stamp
  echo "rebuilt $src_dir: base + ${#patches[@]} patch(es)" >&2
}

# Runs src when the copy is missing or older than the submodule or the patches (build, check and size call it).
ensure_src() {
  if [ -d "$src_dir/.git" ] && [ "$(stamp_field inputs)" = "$(src_inputs)" ]; then
    return 0
  fi
  echo "$src_dir is missing or stale: running src" >&2
  cmd_src
}

# Is `[features] NAME = …` declared in the patched inkboard-app manifest?
has_feature() {
  [ -n "$(sed -n "/^\[features\]/,/^\[/{/^$1 = /p}" "$src_dir/crates/inkboard-app/Cargo.toml")" ]
}

# cargo in the patched copy, so its .cargo/config.toml (the getrandom rustflag of the wasm target) applies. Never
# set RUSTFLAGS: it would replace that flag.
cargo_src() {
  (cd "$src_dir" && CARGO_TARGET_DIR="$target_dir" cargo "$@")
}

# ---------------------------------------------------------------------------------------------- build and size

# Builds the release wasm with the given features and writes inkboard.js + inkboard_bg.wasm (optimized) into DIR.
# Usage: engine_into DIR QUICK FEATURE_ARGS…
engine_into() {
  local dir="$1" quick="$2" profile=() wasm
  shift 2
  if [ "$quick" -eq 1 ]; then
    profile=(--config profile.release.lto=false --config profile.release.codegen-units=16
      --config profile.release.incremental=true)
  fi
  cargo_src build --locked --release "${profile[@]}" -p inkboard-app --bin inkboard --target wasm32-unknown-unknown "$@"
  wasm="$target_dir/wasm32-unknown-unknown/release/inkboard.wasm"
  [ -f "$wasm" ] || die "cargo did not produce $wasm"
  wasm-bindgen --target web --no-typescript --out-name inkboard --out-dir "$dir" "$wasm"
  [ -f "$dir/inkboard.js" ] && [ -f "$dir/inkboard_bg.wasm" ] || die "wasm-bindgen did not write inkboard.js and inkboard_bg.wasm"
  if [ -e "$dir/snippets" ]; then
    die "wasm-bindgen wrote JS snippets: the dist must stay flat (a dependency uses inline JS)"
  fi
  if [ "$quick" -eq 0 ]; then
    wasm-opt "--output=$dir/inkboard_bg.opt.wasm" "${WASM_OPT_FLAGS[@]}" "$dir/inkboard_bg.wasm"
    mv "$dir/inkboard_bg.opt.wasm" "$dir/inkboard_bg.wasm"
  fi
}

cmd_build() {
  local quick=0 fmt set engine f
  parse_out "$@"
  for f in "${rest[@]}"; do
    case "$f" in
      --quick) quick=1 ;;
      *)
        usage >&2
        exit 2
        ;;
    esac
  done
  fmt="$(format_version)"
  set="$(patchset)"
  engine="$INKBOARD_VERSION+embed.quire.$set"
  ensure_src
  has_feature embed || die "the patched copy has no 'embed' feature: is patch 0002 in patches/inkboard?"
  [ -d "$frame_dir" ] || die "$frame_dir is missing"
  stage_dir
  engine_into "$stage" "$quick" "${ENGINE_FEATURES[@]}"
  # The frame page (§10.4): only its .html, .js and .css; never inkboard.d.ts or tsconfig.json.
  shopt -s nullglob
  for f in "$frame_dir"/*.html "$frame_dir"/*.js "$frame_dir"/*.css; do
    [ "$(basename "$f")" != inkboard.js ] || die "$frame_dir/inkboard.js would overwrite the engine's module"
    cp -- "$f" "$stage/"
  done
  shopt -u nullglob
  [ -f "$stage/frame.html" ] && [ -f "$stage/frame.js" ] || die "$frame_dir has no frame.html or frame.js"
  cp -- "$src_dir/crates/inkboard-app/assets/icons.bin" "$src_dir/crates/inkboard-app/assets/ICONS-LICENSES.txt" \
    "$src_dir/LICENSE-MIT" "$src_dir/LICENSE-APACHE" "$src_dir/THIRD_PARTY_LICENSES.md" "$stage/"
  chmod 0644 "$stage"/*
  if [ "$quick" -eq 1 ]; then
    brotli_level=5 gzip_level=6
  fi
  (cd "$stage" && find . -maxdepth 1 -type f \( -name '*.wasm' -o -name '*.js' -o -name '*.html' -o -name '*.css' \) \
    -printf '%P\n') | while IFS= read -r f; do precompress "$f"; done
  write_manifest "$engine" "$set" "$fmt"
  swap_into_place
  echo "wrote the Inkboard dist to $out ($engine, format_version $fmt$([ "$quick" -eq 0 ] || echo ', --quick'))" >&2
}

cmd_size() {
  local label line raw br gz
  local -a sets=()
  [ $# -eq 0 ] || {
    usage >&2
    exit 2
  }
  ensure_src
  has_feature embed || die "the patched copy has no 'embed' feature: is patch 0002 in patches/inkboard?"
  if has_feature net; then
    sets=("with patch 0003 (no net)|--no-default-features --features web-glow,embed"
      "without patch 0003 (net)|--no-default-features --features web-glow,embed,net")
  else
    sets=("without patch 0003 (not applied)|--no-default-features --features web-glow,embed")
  fi
  # Global, not local: the EXIT trap runs after this function has returned.
  tmp="$(mktemp -d "$root/.cache/inkboard-size.XXXXXX")"
  trap 'rm -rf "$tmp"' EXIT
  printf '%-34s %10s %10s %10s\n' ".wasm (bytes)" raw br gz >"$tmp/table"
  for line in "${sets[@]}"; do
    label="${line%%|*}"
    rm -rf "$tmp/out" && mkdir "$tmp/out"
    # shellcheck disable=SC2086 # the feature arguments are words
    engine_into "$tmp/out" 0 ${line#*|}
    raw="$(stat -c %s "$tmp/out/inkboard_bg.wasm")"
    br="$(brotli -q 11 -c "$tmp/out/inkboard_bg.wasm" | wc -c)"
    gz="$(gzip -9 -n -c "$tmp/out/inkboard_bg.wasm" | wc -c)"
    printf '%-34s %10s %10s %10s\n' "$label" "$raw" "$br" "$gz" >>"$tmp/table"
  done
  cat "$tmp/table"
}

# ---------------------------------------------------------------------------------------------- export

cmd_export() {
  local upstream=0
  case "$#:${1:-}" in
    0:) ;;
    1:--upstream) upstream=1 ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
  [ -d "$src_dir/.git" ] || die "$src_dir does not exist: run src first"
  git_src rev-parse -q --verify refs/tags/base >/dev/null || die "$src_dir has no 'base' tag: run src first"
  if [ -n "$(git_src status --porcelain)" ]; then
    die "$src_dir has uncommitted changes: commit them first"
  fi
  if git_src log --format=%B base..HEAD | grep -c 'Changed for Quire' >/dev/null; then
    die "a commit message in $src_dir mentions the notice line; keep it out of the messages"
  fi
  if [ "$upstream" -eq 1 ]; then
    export_upstream
    return
  fi
  rm -f "$patch_dir"/*.patch
  git_src format-patch --zero-commit --no-signature --quiet base -o "$patch_dir"
  write_stamp
  echo "exported $(git_src rev-list --count base..HEAD) patch(es) to $patch_dir" >&2
}

# The same series without the notice lines, for upstream pull requests: every commit is replayed in a scratch clone
# with its tree as in .cache/inkboard-src minus the notice lines, then format-patch writes .cache/inkboard-upstream/.
export_upstream() {
  local c f
  # Global, not local: the EXIT trap runs after this function has returned.
  work="$(mktemp -d "$root/.cache/inkboard-upstream-work.XXXXXX")"
  trap 'rm -rf "$work"' EXIT
  git clone -q --no-local "$src_dir" "$work/repo"
  _git_up() {
    git -C "$work/repo" -c user.name=quire -c user.email=quire@localhost -c commit.gpgsign=false \
      -c core.hooksPath=/dev/null -c advice.detachedHead=false "$@"
  }
  _git_up checkout -q --detach base
  for c in $(git_src rev-list --reverse base..HEAD); do
    _git_up read-tree -u --reset "$c"
    _git_up grep -l -z -E "$NOTICE_RE" -- . 2>/dev/null | while IFS= read -r -d '' f; do
      sed -E -i "/$(printf '%s' "$NOTICE_RE" | sed 's/\//\\\//g')/d" "$work/repo/$f"
    done || true
    _git_up add -A
    _git_up commit -q --allow-empty -C "$c"
  done
  rm -rf "$upstream_dir"
  mkdir -p "$upstream_dir"
  _git_up format-patch --zero-commit --no-signature --quiet base -o "$upstream_dir"
  if grep -rq "$NOTICE" "$upstream_dir" || grep -rq 'Changed for Quire' "$upstream_dir"; then
    die "a notice line survived in $upstream_dir"
  fi
  echo "wrote $(find "$upstream_dir" -name '*.patch' | wc -l) notice-free patch(es) to $upstream_dir" >&2
}

# ---------------------------------------------------------------------------------------------- check

# The feature sets of §10.5 for the patches applied in .cache/inkboard-src, one per line: "<kind> <cargo args…>".
check_sets() {
  echo "native --all-targets"
  echo "wasm --target wasm32-unknown-unknown --features web-glow"
  if has_feature embed; then
    echo "wasm --target wasm32-unknown-unknown --features web-glow,embed"
  fi
  if has_feature net; then
    echo "wasm --target wasm32-unknown-unknown --no-default-features --features web-glow,embed"
    echo "wasm --target wasm32-unknown-unknown --no-default-features --features web-glow,net"
  fi
}

# The warnings of one cargo --message-format=short log, counted: "<count>\t<kind>\t<file>: <message>" per distinct
# warning. Line and column are dropped, so a patch that moves code adds nothing; the count keeps apart two warnings with
# the same message in the same file, so a patch that adds one more of them is still caught.
warnings_of() {
  sed -n -E 's/^(crates\/[^:]+):[0-9]+:[0-9]+: warning: (.*)$/\1: \2/p' "$2" |
    awk -v kind="$1" '{ n[kind "\t" $0]++ } END { for (k in n) print n[k] "\t" k }'
}

# Merges counted warning lists (warnings_of): per warning, the highest count of any one cargo run, sorted by warning.
# The highest, not the sum: the feature sets the patches add compile mostly the same files again, which must not count
# as new warnings, while a new warning still raises the count of the runs that compile it.
max_counts() {
  awk -F'\t' '{ k = $0; sub(/^[0-9]+\t/, "", k); if (!(k in m) || $1 + 0 > m[k]) m[k] = $1 + 0 }
    END { for (k in m) print m[k] "\t" k }' "$@" | LC_ALL=C sort -t "$(printf '\t')" -k2
}

cmd_check() {
  local record=0 kind args tool new count=0
  case "$#:${1:-}" in
    0:) ;;
    1:--record-baseline) record=1 ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
  ensure_src
  if [ "$record" -eq 1 ] && [ "$(git_src rev-parse HEAD)" != "$(git_src rev-parse base)" ]; then
    die "--record-baseline needs the unpatched base: run it with no patches applied (HEAD = base)"
  fi
  # Global, not local: the EXIT trap runs after this function has returned.
  log="$(mktemp -d "$root/.cache/inkboard-check.XXXXXX")"
  trap 'rm -rf "$log"' EXIT
  while IFS= read -r line; do
    kind="${line%% *}"
    args="${line#* }"
    for tool in check clippy; do
      count=$((count + 1))
      echo "== cargo $tool -p inkboard-app $args" >&2
      # shellcheck disable=SC2086 # the feature arguments are words
      if ! cargo_src "$tool" --locked -p inkboard-app $args --message-format=short </dev/null >"$log/$count" 2>&1; then
        cat "$log/$count" >&2
        die "cargo $tool -p inkboard-app $args failed"
      fi
      grep -E '^(crates/[^:]+:[0-9]+:[0-9]+: )?warning' "$log/$count" | grep -v '^warning: .* generated [0-9]* warning' >&2 || true
      warnings_of "$kind" "$log/$count" >"$log/$count.warnings"
    done
  done < <(check_sets)
  max_counts "$log"/*.warnings >"$log/all"
  if [ "$record" -eq 1 ]; then
    {
      echo "# Warnings of scripts/inkboard.sh check on the unpatched base ($INKBOARD_VERSION, rustc $(rustc --version | cut -d' ' -f2))."
      echo "# One line per distinct warning: <count><TAB><kind><TAB><file>: <message>, line and column dropped; <count> is the"
      echo "# most times it appears in one cargo run. Rewrite only on the unpatched base:"
      echo "#   scripts/dev.sh scripts/inkboard.sh check --record-baseline   (with no patches applied)"
      cat "$log/all"
    } >"$baseline_file"
    echo "recorded $(grep -vc '^#' "$baseline_file" || true) distinct baseline warning(s) in $baseline_file" >&2
    return
  fi
  [ -f "$baseline_file" ] || die "no baseline: run check --record-baseline on the unpatched base first"
  if grep -v '^#' "$baseline_file" | grep -qvE "^[0-9]+$(printf '\t')(native|wasm)$(printf '\t')crates/"; then
    die "$baseline_file has a line that is not '<count><TAB><kind><TAB><file>: <message>': record it again (check --record-baseline)"
  fi
  # Every warning whose count in one cargo run is above the baseline's (0 for a warning the baseline does not have).
  new="$(awk -F'\t' '
    FNR == NR { if ($0 !~ /^#/) { k = $0; sub(/^[0-9]+\t/, "", k); base[k] = $1 + 0 }; next }
    { k = $0; sub(/^[0-9]+\t/, "", k); b = (k in base) ? base[k] : 0
      if ($1 + 0 > b) print k " (" $1 " in one cargo run, baseline " b ")" }' "$baseline_file" "$log/all")"
  if [ -n "$new" ]; then
    echo "new warnings against $baseline_file:" >&2
    printf '%s\n' "$new" >&2
    exit 1
  fi
  echo "check: $count cargo runs, no new warnings ($(wc -l <"$log/all") distinct known)" >&2
}

case "${1:-}" in
  stub)
    shift
    cmd_stub "$@"
    ;;
  src)
    shift
    cmd_src "$@"
    ;;
  build)
    shift
    cmd_build "$@"
    ;;
  export)
    shift
    cmd_export "$@"
    ;;
  check)
    shift
    cmd_check "$@"
    ;;
  size)
    shift
    cmd_size "$@"
    ;;
  -h | --help)
    usage
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac
