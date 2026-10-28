#!/bin/bash
# Installs Quire's build slice (docs/ARCHITECTURE.md §13.1, D38), the one privileged step of M1, or checks it.
#   scripts/install-build-slice.sh           # as root: install docker/quire.slice and docker/quire-build.slice into
#                                            # /etc/systemd/system, daemon-reload, start the slice, then --check
#   scripts/install-build-slice.sh --check   # no root needed: print and verify the installed slices
# It never calls sudo itself. Implementing agents never run the install; the coordinator runs it once, through the
# owner's askpass dialog (the command is printed when it is started without root). Re-running it is harmless.
# Exit codes: 0 every value holds; 1 a value does not hold (each failing line is named), the install was started
# without root, or a unit file was refused; 2 usage; 3 the slice is not installed (no FragmentPath).
#
# Run as root, it trusts nothing the caller controls beyond the two unit files' approved content: /bin/bash and a
# fixed PATH (never the caller's, which starts with user-writable directories), no sourced file, and it writes only
# /etc/systemd/system/quire.slice and quire-build.slice (staged in a root-only /run directory that it removes). Each
# source is read once (a regular file, not a symlink, at most 16 KiB) and installed byte for byte as checked, and only
# if its directives are exactly the approved ones (UNIT_DIRECTIVES below), so a swapped or edited source can neither
# raise a cap nor copy another file (a symlink to /etc/shadow, say) into a world-readable unit. A refused file's
# content is never printed.
set -euo pipefail
PATH=/usr/sbin:/usr/bin:/sbin:/bin
export PATH LC_ALL=C
unset CDPATH BASH_ENV ENV TMPDIR
root="$(cd "$(dirname "$0")/.." && pwd)"
self="$root/scripts/install-build-slice.sh"
units=/etc/systemd/system

usage() {
  sed -n '2,9s/^# \{0,1\}//p' "$self"
}

# The approved directives of each unit, in order (comments and blank lines aside; any Description text of letters,
# digits, spaces and ,.:()/- counts as "Description="). Changing a value means changing it here too, in the open.
declare -A UNIT_DIRECTIVES=(
  [quire.slice]=$'[Unit]\nDescription=\n[Slice]\nCPUWeight=20\nIOWeight=20\n'
  [quire-build.slice]=$'[Unit]\nDescription=\n[Slice]\nCPUQuota=600%\nMemoryMax=12G\nMemorySwapMax=0\nCPUWeight=20\nIOWeight=20\nTasksMax=16384\n'
)

# read_unit NAME: prints docker/NAME as it will be installed, or names the reason and returns 1 (never the content).
read_unit() {
  local name="$1" src="$root/docker/$1" text line n=0 directives="" desc_re='^Description=[A-Za-z0-9 ,.:()/-]*$'
  if [ -L "$src" ] || [ ! -f "$src" ]; then
    echo "refused $src: not a regular file (or a symlink)" >&2
    return 1
  fi
  # One bounded read (a FIFO or /dev/zero swapped in cannot hang or flood it); what is checked is what is installed.
  if ! text="$(timeout 10 head -c 16385 -- "$src")"; then
    echo "refused $src: cannot read it" >&2
    return 1
  fi
  if [ "${#text}" -gt 16384 ]; then
    echo "refused $src: larger than 16 KiB" >&2
    return 1
  fi
  while IFS= read -r line; do
    n=$((n + 1))
    case "$line" in
      *\\)
        echo "refused $src: line $n ends in a backslash (a systemd line continuation)" >&2
        return 1
        ;;
      "" | "#"* | ";"*) continue ;;
      Description=*)
        if ! [[ "$line" =~ $desc_re ]]; then
          echo "refused $src: line $n: Description has characters other than letters, digits, spaces and ,.:()/-" >&2
          return 1
        fi
        line=Description=
        ;;
    esac
    directives+="$line"$'\n'
  done <<<"$text"
  if [ "$directives" != "${UNIT_DIRECTIVES[$name]}" ]; then
    {
      echo "refused $src: its directives are not exactly the approved ones:"
      while IFS= read -r line; do echo "  $line"; done <<<"${UNIT_DIRECTIVES[$name]%$'\n'}"
    } >&2
    return 1
  fi
  printf '%s\n' "$text"
}

print_install_command() {
  cat <<EOF
SUDO_ASKPASS=/home/staffan/.local/bin/claude-askpass sudo -A \\
  -p "install the Quire build cap (systemd slice: 6 CPUs, 12 GB RAM, no swap, low priority) :: $self" \\
  "$self"
EOF
}

failures=0
ok() { printf 'ok    %s\n' "$*"; }
info() { printf -- '--    %s\n' "$*"; }
bad() {
  printf 'FAIL  %s\n' "$*"
  failures=$((failures + 1))
}

# expect UNIT PROPERTY WANT, against the "KEY=VALUE" lines of `systemctl show` in $props.
expect() {
  local unit="$1" key="$2" want="$3" got
  got="$(sed -n "s/^$key=//p" <<<"$props" | head -n 1)"
  if [ "$got" = "$want" ]; then
    ok "$unit $key=$got"
  else
    bad "$unit $key=${got:-(empty)} (want $want)"
  fi
}

disk_scheduler() {
  local dir src disk majmin sched
  dir="$(docker info --format '{{.DockerRootDir}}' 2>/dev/null)" || dir=""
  dir="${dir:-/var/lib/docker}"
  src="$(findmnt -n -o SOURCE -T "$dir" 2>/dev/null | head -n 1)" || src=""
  src="${src%%\[*}" # btrfs: /dev/sda2[/@] → /dev/sda2
  if [ -z "$src" ] || [ ! -b "$src" ]; then
    info "the disk holding $dir: unknown (${src:-no mount source}); IOWeight may have no effect"
    return 0
  fi
  # The whole disks under the mount source (through partitions, LVM, dm-crypt).
  for disk in $(lsblk -s -n -r -o NAME,TYPE "$src" 2>/dev/null | awk '$2 == "disk" { print $1 }' | sort -u); do
    { read -r sched <"/sys/block/$disk/queue/scheduler"; } 2>/dev/null || sched=unknown
    majmin="$(lsblk -d -n -r -o MAJ:MIN "/dev/$disk" 2>/dev/null || true)"
    if [[ "$sched" == *"[bfq]"* ]]; then
      info "$dir is on $disk ($src), scheduler: $sched: io.weight is honoured (BFQ)"
    elif [ -n "$majmin" ] && grep -qs "^$majmin enable=1" /sys/fs/cgroup/io.cost.qos; then
      info "$dir is on $disk ($src), scheduler: $sched, io.cost on: io.weight is honoured"
    else
      info "$dir is on $disk ($src), scheduler: $sched, io.cost off: IOWeight has no effect on this disk"
    fi
  done
}

check() {
  local driver
  driver="$(docker info --format '{{.CgroupDriver}}' 2>/dev/null)" || driver=""
  if [ "$driver" = systemd ]; then
    ok "docker CgroupDriver=$driver"
  else
    bad "docker CgroupDriver=${driver:-(docker info failed)} (want systemd: under cgroupfs the slice is not used)"
  fi

  props="$(systemctl show --no-pager -p FragmentPath -p ActiveState -p ControlGroup -p CPUQuotaPerSecUSec -p MemoryMax \
    -p MemorySwapMax -p CPUWeight -p IOWeight -p TasksMax quire-build.slice 2>/dev/null)" || props=""
  if [ -z "$(sed -n 's/^FragmentPath=//p' <<<"$props")" ]; then
    printf 'FAIL  quire-build.slice is not installed (no FragmentPath). The coordinator installs it once, through the owner'"'"'s askpass dialog:\n'
    print_install_command
    return 3
  fi
  expect quire-build.slice FragmentPath "$units/quire-build.slice"
  if [ "$(sed -n 's/^ActiveState=//p' <<<"$props")" = active ]; then
    expect quire-build.slice ControlGroup /quire.slice/quire-build.slice
  else
    info "quire-build.slice is inactive (it starts with its first member): ControlGroup not checked"
  fi
  expect quire-build.slice CPUQuotaPerSecUSec 6s
  expect quire-build.slice MemoryMax 12884901888
  expect quire-build.slice MemorySwapMax 0
  expect quire-build.slice CPUWeight 20
  expect quire-build.slice IOWeight 20
  expect quire-build.slice TasksMax 16384

  props="$(systemctl show --no-pager -p FragmentPath -p CPUWeight -p IOWeight quire.slice 2>/dev/null)" || props=""
  expect quire.slice FragmentPath "$units/quire.slice"
  expect quire.slice CPUWeight 20
  expect quire.slice IOWeight 20

  info "members of quire-build.slice:"
  systemctl status quire-build.slice --no-pager || true
  disk_scheduler

  if [ "$failures" -gt 0 ]; then
    echo "$failures value(s) do not hold (the FAIL lines above)"
    return 1
  fi
  echo "quire-build.slice holds every value"
}

install_slices() {
  if [ "$(id -u)" -ne 0 ]; then
    {
      echo "scripts/install-build-slice.sh must run as root, and it never calls sudo itself. The coordinator runs it"
      echo "once, through the owner's askpass dialog:"
      print_install_command
    } >&2
    exit 1
  fi
  # root's own home, and no Docker client settings from the caller (the --check below runs docker info as root).
  export HOME=/root
  unset DOCKER_HOST DOCKER_CONTEXT DOCKER_CONFIG DOCKER_CERT_PATH DOCKER_TLS_VERIFY
  # Both files are checked before either is written, so a refusal installs nothing.
  local u parent build
  parent="$(read_unit quire.slice)" || exit 1
  build="$(read_unit quire-build.slice)" || exit 1
  # The checked bytes go through a root-only mktemp directory: `install` from /dev/stdin fails once the target exists
  # (uutils coreutils 0.10), which would break re-running. It lives in /run, never under $TMPDIR, which could be a
  # directory of the caller's, where the caller could swap the directory and plant symlinks for root to write through.
  umask 077
  staging="$(mktemp -d /run/quire-install-slice.XXXXXXXX)"
  trap 'rm -rf -- "$staging"' EXIT
  printf '%s\n' "$parent" >"$staging/quire.slice"
  printf '%s\n' "$build" >"$staging/quire-build.slice"
  umask 022
  for u in quire.slice quire-build.slice; do
    install -o root -g root -m 0644 "$staging/$u" "$units/$u"
    echo "installed $units/$u"
  done
  systemctl daemon-reload
  systemctl start quire-build.slice
  local rc=0
  check || rc=$?
  exit "$rc"
}

case "${1:-}" in
  "")
    [ $# -le 1 ] || {
      usage >&2
      exit 2
    }
    install_slices
    ;;
  --check)
    [ $# -eq 1 ] || {
      usage >&2
      exit 2
    }
    rc=0
    check || rc=$?
    exit "$rc"
    ;;
  -h | --help)
    usage
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac
