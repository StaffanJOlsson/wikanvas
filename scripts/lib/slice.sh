# shellcheck shell=bash disable=SC2034  # quire_caps sets its results for the scripts that source this file
# The build slice and the per-container build caps (docs/ARCHITECTURE.md §13.1, D38). Sourced, never run:
#   . "$root/scripts/lib/slice.sh"; quire_caps || exit 2; quire_slice
#
# quire_slice exports QUIRE_CGROUP_PARENT=quire-build.slice when Docker uses the systemd cgroup driver and the slice's
# unit file is installed (FragmentPath is /etc/systemd/system/quire-build.slice). Otherwise it exports it empty and
# warns once on stderr: the containers still run, capped one by one. LoadState is no test: systemd reports any slice
# name as "loaded", and a container given a missing slice would land in one created on the fly, with no limits.
# QUIRE_BUILD_SLICE (default quire-build.slice) exists only so the acceptance test can name a missing slice: it
# changes which unit is tested, never the cgroup parent, which is quire-build.slice or nothing.
#
# quire_caps sets (unexported) quire_cpus, quire_mem, quire_mem_bytes, quire_cpuset and quire_threads from
# QUIRE_BUILD_CPUS (default 6), QUIRE_BUILD_MEM (default 12g) and QUIRE_BUILD_CPUSET (default: the host's last
# quire_cpus CPUs, e.g. 10-15 on 16; empty when the host has fewer). The variables may only lower the caps: a higher
# value is refused (return 1). dev.sh and toolchain.sh both call it, so the BuildKit container gets the same CPU set.
# Pass quire_mem_bytes (not quire_mem) to Docker, so the value checked is the value applied.

quire_slice() {
  local slice="${QUIRE_BUILD_SLICE:-quire-build.slice}" driver="" fragment="" why=""
  driver="$(docker info --format '{{.CgroupDriver}}' 2>/dev/null)" || driver=""
  case "$slice" in
    *.slice) fragment="$(systemctl show -p FragmentPath --value "$slice" 2>/dev/null)" || fragment="" ;;
  esac
  # Only quire-build.slice is ever passed as cgroup parent: QUIRE_BUILD_SLICE names the unit to test, never another
  # installed slice to put builds into.
  if [ "$slice" = quire-build.slice ] && [ "$driver" = systemd ] && [ "$fragment" = "/etc/systemd/system/$slice" ]; then
    export QUIRE_CGROUP_PARENT="$slice"
    return 0
  fi
  export QUIRE_CGROUP_PARENT=
  if [ "$fragment" != "/etc/systemd/system/$slice" ]; then
    why="$slice not installed"
  elif [ "$slice" != quire-build.slice ]; then
    why="$slice not used: only quire-build.slice is a build cgroup parent"
  else
    why="$slice not used: Docker's cgroup driver is '${driver:-unknown}', not systemd"
  fi
  # Once per process tree: a nested script (e2e.sh → dev.sh) does not repeat it.
  if [ "${_QUIRE_SLICE_WARNED:-}" != "$slice" ]; then
    echo "$why: per-container caps only (scripts/install-build-slice.sh --check)" >&2
    export _QUIRE_SLICE_WARNED="$slice"
  fi
}

# The ceilings. Raise them only when the owner asks (CLAUDE.md).
QUIRE_CAP_MAX_CPUS=6
QUIRE_CAP_MAX_MEM=12884901888   # 12g

# Bytes from a Docker-style size (12g, 6G, 512m, 1024k, 12gb, 12884901888); integers only. At most 15 digits, and a
# product that would overflow 64-bit arithmetic prints 2^62 instead (far above any cap), so a huge value can never
# wrap around to a small one and pass the check.
_quire_bytes() {
  local v="$1" n u m
  if [[ "$v" =~ ^([0-9]{1,15})([kKmMgG]?)([iI]?[bB])?$ ]]; then
    n="$((10#${BASH_REMATCH[1]}))"
    u="${BASH_REMATCH[2]}"
  else
    return 1
  fi
  case "$u" in
    "") m=1 ;;
    k | K) m=1024 ;;
    m | M) m=1048576 ;;
    g | G) m=1073741824 ;;
  esac
  if [ "$n" -gt $(((1 << 62) / m)) ]; then
    echo "$((1 << 62))"
  else
    echo "$((n * m))"
  fi
}

# Expands a cpuset list ("10-15", "0,2,4-5") to one CPU id per line; fails on anything else. CPU ids have at most four
# digits, so a typo cannot make seq print billions of lines.
_quire_cpus_of() {
  local list="$1" part a b
  local -a parts
  [[ "$list" =~ ^[0-9]{1,4}(-[0-9]{1,4})?(,[0-9]{1,4}(-[0-9]{1,4})?)*$ ]] || return 1
  IFS=, read -ra parts <<<"$list"
  for part in "${parts[@]}"; do
    a="${part%-*}"
    b="${part#*-}"
    a=$((10#$a))
    b=$((10#$b))
    [ "$a" -le "$b" ] || return 1
    seq "$a" "$b"
  done
}

# Compresses sorted CPU ids (one per line on stdin) to a cpuset list: 10 11 12 13 → 10-13.
_quire_cpuset_of() {
  awk 'NR == 1 { s = p = $1; next }
       $1 == p + 1 { p = $1; next }
       { out = out (out ? "," : "") (s == p ? s : s "-" p); s = p = $1 }
       END { if (NR) print out (out ? "," : "") (s == p ? s : s "-" p) }'
}

quire_caps() {
  quire_cpus="${QUIRE_BUILD_CPUS:-$QUIRE_CAP_MAX_CPUS}"
  quire_mem="${QUIRE_BUILD_MEM:-12g}"
  # At most three digits before the comparison: `[` fails on a number beyond 64 bits, and a failed test is no refusal.
  if ! [[ "$quire_cpus" =~ ^[1-9][0-9]{0,2}$ ]] || [ "$quire_cpus" -gt "$QUIRE_CAP_MAX_CPUS" ]; then
    echo "QUIRE_BUILD_CPUS=$quire_cpus: must be a whole number from 1 to $QUIRE_CAP_MAX_CPUS (the caps may only be lowered)" >&2
    return 1
  fi
  if ! quire_mem_bytes="$(_quire_bytes "$quire_mem")" || [ "$quire_mem_bytes" -le 0 ]; then
    echo "QUIRE_BUILD_MEM=$quire_mem: not a size such as 12g, 6g or 512m" >&2
    return 1
  fi
  if [ "$quire_mem_bytes" -gt "$QUIRE_CAP_MAX_MEM" ]; then
    echo "QUIRE_BUILD_MEM=$quire_mem: above 12g (the caps may only be lowered)" >&2
    return 1
  fi

  local ids count
  if [ -n "${QUIRE_BUILD_CPUSET:-}" ]; then
    if ! ids="$(_quire_cpus_of "$QUIRE_BUILD_CPUSET")"; then
      echo "QUIRE_BUILD_CPUSET=$QUIRE_BUILD_CPUSET: not a CPU list such as 10-15 or 2,4-7" >&2
      return 1
    fi
    count="$(sort -un <<<"$ids" | wc -l)"
    if [ "$count" -gt "$quire_cpus" ]; then
      echo "QUIRE_BUILD_CPUSET=$QUIRE_BUILD_CPUSET: $count CPUs, more than QUIRE_BUILD_CPUS=$quire_cpus (the caps may only be lowered)" >&2
      return 1
    fi
    quire_cpuset="$QUIRE_BUILD_CPUSET"
  else
    # The online CPUs as the kernel lists them (not nproc, which follows this process's own affinity).
    if [ -r /sys/devices/system/cpu/online ]; then
      ids="$(_quire_cpus_of "$(cat /sys/devices/system/cpu/online)")" || ids=""
    fi
    if [ -z "${ids:-}" ]; then
      ids="$(seq 0 $(($(getconf _NPROCESSORS_ONLN) - 1)))"
    fi
    count="$(wc -l <<<"$ids")"
    if [ "$count" -lt "$quire_cpus" ]; then
      quire_cpuset=""
      count="$quire_cpus"
    else
      quire_cpuset="$(sort -n <<<"$ids" | tail -n "$quire_cpus" | _quire_cpuset_of)"
      count="$quire_cpus"
    fi
  fi
  # Thread pools follow the CPUs the container can actually use.
  quire_threads="$quire_cpus"
  if [ "$count" -lt "$quire_threads" ]; then
    quire_threads="$count"
  fi
}
