# shellcheck shell=bash
# The per-user Quire build lock (docs/ARCHITECTURE.md §13.1). Sourced, never run:
#   . "$root/scripts/lib/lock.sh"; quire_lock || exit 1
#
# quire_lock takes an exclusive flock on <home>/.cache/quire/build.lock, where <home> comes from the passwd database
# (getent), so the file is the same whatever $HOME, XDG_RUNTIME_DIR or a private /tmp say, and every worktree shares
# it. It waits (printing "waiting for another Quire build…" to stderr) while another build holds it, then exports
# QUIRE_LOCK_HELD=1. The lock lives on an open file descriptor that every child inherits (an `exec docker run …` keeps
# it), so it is released when the last process of the holder exits. A script started with QUIRE_LOCK_HELD set is a
# descendant of the holder and never waits; it only takes the lock if nobody holds it at all (a stale variable), so
# it never runs unlocked next to a free lock. Never set QUIRE_LOCK_HELD by hand.
# Every failure returns 1 explicitly: callers may use `quire_lock || exit 1` or `if quire_lock`, where `set -e` does
# not act inside the function, and a failed flock must never count as holding the lock.

quire_lock() {
  local home dir
  home="$(getent passwd "$(id -u)" | cut -d: -f6)" || home=""
  if [ -n "${QUIRE_LOCK_HELD:-}" ]; then
    # Nested: the holder is an ancestor, so its flock makes this one fail at once. If it succeeds, nobody held the
    # lock, and this process now does (best effort: a nested call never fails or waits).
    if [ -n "$home" ] && [ -d "$home/.cache/quire" ] && exec {QUIRE_LOCK_FD}>>"$home/.cache/quire/build.lock"; then
      flock -n "$QUIRE_LOCK_FD" || exec {QUIRE_LOCK_FD}>&-
    fi
    return 0
  fi
  if [ -z "$home" ] || [ ! -d "$home" ]; then
    echo "quire_lock: no home directory for uid $(id -u) in the passwd database" >&2
    return 1
  fi
  dir="$home/.cache/quire"
  if ! mkdir -p "$dir"; then
    echo "quire_lock: cannot create $dir" >&2
    return 1
  fi
  # A dynamically allocated descriptor, so it never collides with one the calling script uses.
  if ! exec {QUIRE_LOCK_FD}>>"$dir/build.lock"; then
    echo "quire_lock: cannot open $dir/build.lock" >&2
    return 1
  fi
  if ! flock -n "$QUIRE_LOCK_FD"; then
    echo "waiting for another Quire build…" >&2
    if ! flock "$QUIRE_LOCK_FD"; then
      echo "quire_lock: flock on $dir/build.lock failed" >&2
      exec {QUIRE_LOCK_FD}>&-
      return 1
    fi
  fi
  export QUIRE_LOCK_HELD=1
}
