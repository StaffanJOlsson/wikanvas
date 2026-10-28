#!/usr/bin/env bash
# Checks Quire's version pins (docs/ARCHITECTURE.md §4.1, §13.2, §15; PLAN R6). Host only: text and git, no build
# tools and no container. Run it after every change to a Cargo.toml, Cargo.lock or docker/Dockerfile.
#   scripts/check-pins.sh
# Checks:
#   - vendor/inkboard: the superproject's gitlink and the checked-out commit are 41e0295, and the submodule is clean;
#   - serde, serde_json, base64 and thiserror: inkboard-core resolves each to the same version in Cargo.lock as in
#     vendor/inkboard/Cargo.lock, and every semver-compatible line of the crate present in both lockfiles has the
#     same version in both (the "exact protocol match" of §15);
#   - .dockerignore has no `{` (Docker's matcher has no brace expansion, so such a pattern silently matches nothing);
#   - rust-toolchain.toml's channel equals the tag of RUST_IMAGE in docker/Dockerfile;
#   - docker/Dockerfile: no digest or checksum placeholder is left, and WASM_BINDGEN_VERSION equals wasm-bindgen in
#     vendor/inkboard/Cargo.lock.
# Prints one line per check; exit 0 when all hold, 1 otherwise.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
export LC_ALL=C

INKBOARD_COMMIT=41e0295d50433faa5d05b2ff48f85eac079a981d
SHARED_CRATES=(serde serde_json base64 thiserror)

case "${1:-}" in
  "") ;;
  -h | --help)
    sed -n '2,15s/^# \{0,1\}//p' "$root/scripts/check-pins.sh"
    exit 0
    ;;
  *)
    sed -n '2,15s/^# \{0,1\}//p' "$root/scripts/check-pins.sh" >&2
    exit 2
    ;;
esac

failures=0
pass() { echo "ok      $*"; }
fail() {
  echo "FAILED  $*"
  failures=$((failures + 1))
}

# --- the Inkboard submodule ------------------------------------------------------------------------------------------
sub="$root/vendor/inkboard"
gitlink="$(git -C "$root" ls-files -s -- vendor/inkboard 2>/dev/null | awk '$1 == "160000" { print $2 }')" || gitlink=""
if [ "$gitlink" = "$INKBOARD_COMMIT" ]; then
  pass "vendor/inkboard gitlink is ${INKBOARD_COMMIT:0:7}"
else
  fail "vendor/inkboard gitlink is '${gitlink:-none}' (want $INKBOARD_COMMIT)"
fi
head="$(git -C "$sub" rev-parse HEAD 2>/dev/null || true)"
if [ "$head" = "$INKBOARD_COMMIT" ]; then
  pass "vendor/inkboard is checked out at ${INKBOARD_COMMIT:0:7}"
else
  fail "vendor/inkboard is checked out at '${head:-nothing}' (want $INKBOARD_COMMIT; git submodule update --init)"
fi
if [ -n "$head" ] && dirty="$(git -C "$sub" status --porcelain 2>/dev/null)" && [ -z "$dirty" ]; then
  pass "vendor/inkboard is clean"
else
  fail "vendor/inkboard is not clean (git -C vendor/inkboard status --porcelain): ${dirty:-not a checkout}"
fi

# --- crates shared with Inkboard ---------------------------------------------------------------------------------
# Prints "name version" for every [[package]] of a Cargo.lock.
lock_packages() {
  awk '
    /^\[\[package\]\]$/ { name = ""; next }
    /^name = "/ { name = $3; gsub(/"/, "", name); next }
    /^version = "/ && name != "" { v = $3; gsub(/"/, "", v); print name, v; name = "" }' "$1"
}
# Prints the dependency entries ("serde" or "base64 0.23.1") of one package of a Cargo.lock.
lock_deps_of() {
  awk -v want="$2" '
    /^\[\[package\]\]$/ { inpkg = 0; indeps = 0; next }
    /^name = "/ { n = $3; gsub(/"/, "", n); inpkg = (n == want); next }
    inpkg && /^dependencies = \[/ { indeps = 1; next }
    indeps && /^\]/ { indeps = 0; next }
    indeps { line = $0; sub(/^ *"/, "", line); sub(/",? *$/, "", line); sub(/ \(.*\)$/, "", line); print line }' "$1"
}
# The version of crate $3 that package $2 depends on in lockfile $1 (empty when it does not depend on it).
dep_version() {
  local entry v
  entry="$(lock_deps_of "$1" "$2" | awk -v c="$3" '$1 == c && !done { print; done = 1 }')"
  [ -n "$entry" ] || return 0
  v="${entry#* }"
  if [ "$v" = "$entry" ]; then
    # A bare name: the lockfile has exactly one version of the crate.
    lock_packages "$1" | awk -v c="$3" '$1 == c && !done { print $2; done = 1 }'
  else
    echo "$v"
  fi
}
# The semver-compatibility line of a version: 1.0.229 → 1, 0.23.1 → 0.23, 0.0.3 → 0.0.3.
compat_line() {
  awk -F. '{ if ($1 != "0") print $1; else if ($2 != "0") print $1 "." $2; else print $1 "." $2 "." $3 }' <<<"${1%%[-+]*}"
}

quire_lock="$root/Cargo.lock"
ink_lock="$sub/Cargo.lock"
if [ ! -f "$quire_lock" ] || [ ! -f "$ink_lock" ]; then
  [ -f "$quire_lock" ] || fail "Cargo.lock is missing"
  [ -f "$ink_lock" ] || fail "vendor/inkboard/Cargo.lock is missing"
elif ! lock_packages "$quire_lock" | awk '$1 == "inkboard-core" { found = 1 } END { exit !found }'; then
  fail "Cargo.lock has no inkboard-core package (quire-core must depend on it, §4.1)"
else
  for crate in "${SHARED_CRATES[@]}"; do
    ours="$(dep_version "$quire_lock" inkboard-core "$crate")"
    theirs="$(dep_version "$ink_lock" inkboard-core "$crate")"
    if [ -z "$ours" ] || [ -z "$theirs" ]; then
      fail "$crate: inkboard-core's dependency not found (Cargo.lock '${ours:-none}', Inkboard '${theirs:-none}')"
      continue
    fi
    bad=""
    while read -r v; do
      [ -n "$v" ] || continue
      line="$(compat_line "$v")"
      while read -r w; do
        [ -n "$w" ] || continue
        if [ "$(compat_line "$w")" = "$line" ] && [ "$w" != "$v" ]; then bad="$bad $v≠$w"; fi
      done < <(lock_packages "$ink_lock" | awk -v c="$crate" '$1 == c { print $2 }')
    done < <(lock_packages "$quire_lock" | awk -v c="$crate" '$1 == c { print $2 }')
    if [ "$ours" = "$theirs" ] && [ -z "$bad" ]; then
      pass "$crate $ours in both lockfiles"
    elif [ "$ours" != "$theirs" ]; then
      fail "$crate: inkboard-core resolves to $ours in Cargo.lock but $theirs in vendor/inkboard/Cargo.lock (cargo update -p $crate@$ours --precise $theirs)"
    else
      fail "$crate: semver-compatible versions differ between Cargo.lock and vendor/inkboard/Cargo.lock:$bad"
    fi
  done
fi

# --- .dockerignore -----------------------------------------------------------------------------------------------
if [ ! -f "$root/.dockerignore" ]; then
  fail ".dockerignore is missing"
elif grep -n '{' "$root/.dockerignore" >/dev/null; then
  fail ".dockerignore has a '{' (no brace expansion in Docker's matcher): $(grep -n '{' "$root/.dockerignore" | tr '\n' ' ')"
else
  pass ".dockerignore has no '{'"
fi

# --- rust-toolchain.toml and docker/Dockerfile -------------------------------------------------------------------
dockerfile="$root/docker/Dockerfile"
rust_tag=""
if [ -f "$dockerfile" ]; then
  rust_tag="$(sed -n 's/^ARG RUST_IMAGE=rust:\([^-@]*\)-.*$/\1/p;T;q' "$dockerfile")"
fi
channel=""
if [ -f "$root/rust-toolchain.toml" ]; then
  channel="$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*$/\1/p;T;q' "$root/rust-toolchain.toml")"
fi
if [ -n "$channel" ] && [ "$channel" = "$rust_tag" ]; then
  pass "rust-toolchain.toml channel $channel = RUST_IMAGE tag"
else
  fail "rust-toolchain.toml channel '${channel:-missing}' differs from the RUST_IMAGE tag '${rust_tag:-missing}' in docker/Dockerfile"
fi

if [ ! -f "$dockerfile" ]; then
  fail "docker/Dockerfile is missing"
else
  placeholders="$(grep -nE '^ARG (RUST|NODE|RUNTIME)_IMAGE=|^ARG BINARYEN_SHA256=' "$dockerfile" |
    grep -vE '@sha256:[0-9a-f]{64}$|=[0-9a-f]{64}$' || true)"
  count="$(grep -cE '^ARG (RUST|NODE|RUNTIME)_IMAGE=[^@]+@sha256:[0-9a-f]{64}$|^ARG BINARYEN_SHA256=[0-9a-f]{64}$' "$dockerfile" || true)"
  if [ -z "$placeholders" ] && [ "$count" = 4 ]; then
    pass "docker/Dockerfile pins the three base images by digest and binaryen by checksum"
  else
    fail "docker/Dockerfile has unpinned base images or checksum: ${placeholders:-$count of 4 pinned}"
  fi
  ours="$(sed -n 's/^ARG WASM_BINDGEN_VERSION=\([0-9.]*\)$/\1/p;T;q' "$dockerfile")"
  theirs=""
  if [ -f "$ink_lock" ]; then theirs="$(lock_packages "$ink_lock" | awk '$1 == "wasm-bindgen" && !done { print $2; done = 1 }')"; fi
  if [ -n "$ours" ] && [ "$ours" = "$theirs" ]; then
    pass "WASM_BINDGEN_VERSION $ours = wasm-bindgen in vendor/inkboard/Cargo.lock"
  else
    fail "WASM_BINDGEN_VERSION '${ours:-missing}' differs from wasm-bindgen '${theirs:-missing}' in vendor/inkboard/Cargo.lock"
  fi
fi

if [ "$failures" -gt 0 ]; then
  echo "check-pins: $failures check(s) failed" >&2
  exit 1
fi
echo "check-pins: every pin holds" >&2
