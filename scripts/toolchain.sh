#!/usr/bin/env bash
# Builds the quire-toolchain image with the capped quire-limited builder (docs/ARCHITECTURE.md §13.1, D38).
#   scripts/toolchain.sh                  # set up (or check) the builder, then build quire-toolchain:latest
#   scripts/toolchain.sh --builder-only   # set up (or check) the builder only
# The builder is the BuildKit container quire-buildkit (moby/buildkit, pinned by digest) with the dev.sh caps and CPU
# set, the build slice as cgroup parent when it is installed, config docker/buildkitd.toml and the state volume
# quire-buildkit-state, registered as quire-limited with buildx's remote driver. (buildx's docker-container driver
# ignores cgroup-parent under Docker's systemd cgroup driver, so buildx must not create the container itself.)
# If quire-limited uses another driver, or quire-buildkit has other caps, another cgroup parent, image or config than
# this script would give it now (for example after the slice was installed), it prints the commands that recreate
# both and exits 1; it never removes either itself. The build cache lives in the volume and survives a recreate.
# Caps (may only be lowered): QUIRE_BUILD_CPUS, QUIRE_BUILD_MEM, QUIRE_BUILD_CPUSET, as for scripts/dev.sh.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
. "$root/scripts/lib/lock.sh"
. "$root/scripts/lib/slice.sh"

# v0.34.1 was under two weeks old on 2026-10-28, so the newest older patch is pinned (ARCHITECTURE §16, risk 23).
BUILDKIT_IMAGE=moby/buildkit:v0.34.0@sha256:b059f8d7226d0b326bc871af489a5dddf65e36af4d20d14251b072974d9ee87c
container=quire-buildkit
builder=quire-limited
volume=quire-buildkit-state
config=/etc/buildkit/buildkitd.toml

usage() {
  sed -n '2,12s/^# \{0,1\}//p' "$root/scripts/toolchain.sh"
}

builder_only=0
case "$#:${1:-}" in
  0:) ;;
  1:--builder-only) builder_only=1 ;;
  1:-h | 1:--help)
    usage
    exit 0
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac

quire_caps || exit 2
quire_lock || exit 1
quire_slice

recreate() {
  {
    printf '%s\n' "$@"
    echo "Recreate the builder (the build cache in the $volume volume survives):"
    echo "  docker buildx rm $builder; docker rm -f $container; scripts/toolchain.sh"
  } >&2
  exit 1
}

# 1. The buildx registration, if any, must be the remote driver on our container.
registered=0
if info="$(docker buildx inspect "$builder" 2>/dev/null)"; then
  driver="$(awk '$1 == "Driver:" { print $2; exit }' <<<"$info")"
  endpoint="$(awk '$1 == "Endpoint:" { print $2; exit }' <<<"$info")"
  if [ "$driver" != remote ] || [ "$endpoint" != "docker-container://$container" ]; then
    recreate "The $builder builder uses the '${driver:-unknown}' driver (endpoint ${endpoint:-unknown}), not 'remote' on docker-container://$container."
  fi
  registered=1
fi

# 2. The BuildKit container: check an existing one against what it would be given now, or create it.
want_cpus=$((quire_cpus * 1000000000))
if docker container inspect "$container" >/dev/null 2>&1; then
  IFS='|' read -r image nano mem swap pids cpuset parent priv cgroupns < <(docker container inspect -f \
    '{{.Config.Image}}|{{.HostConfig.NanoCpus}}|{{.HostConfig.Memory}}|{{.HostConfig.MemorySwap}}|{{.HostConfig.PidsLimit}}|{{.HostConfig.CpusetCpus}}|{{.HostConfig.CgroupParent}}|{{.HostConfig.Privileged}}|{{.HostConfig.CgroupnsMode}}' \
    "$container")
  diffs=()
  [ "$image" = "$BUILDKIT_IMAGE" ] || diffs+=("  image $image (want $BUILDKIT_IMAGE)")
  [ "$nano" = "$want_cpus" ] || diffs+=("  NanoCpus $nano (want $want_cpus)")
  [ "$mem" = "$quire_mem_bytes" ] || diffs+=("  Memory $mem (want $quire_mem_bytes)")
  [ "$swap" = "$quire_mem_bytes" ] || diffs+=("  MemorySwap $swap (want $quire_mem_bytes)")
  [ "$pids" = 4096 ] || diffs+=("  PidsLimit $pids (want 4096)")
  [ "$cpuset" = "$quire_cpuset" ] || diffs+=("  CpusetCpus '$cpuset' (want '$quire_cpuset')")
  [ "$parent" = "$QUIRE_CGROUP_PARENT" ] || diffs+=("  CgroupParent '$parent' (want '$QUIRE_CGROUP_PARENT')")
  [ "$priv" = true ] || diffs+=("  Privileged $priv (want true)")
  [ "$cgroupns" = private ] || diffs+=("  CgroupnsMode '$cgroupns' (want 'private')")
  if ! docker cp "$container:$config" - 2>/dev/null | tar -xOf - 2>/dev/null | cmp -s - "$root/docker/buildkitd.toml"; then
    diffs+=("  $config differs from docker/buildkitd.toml")
  fi
  if [ ${#diffs[@]} -gt 0 ]; then
    recreate "The $container container differs from what this script would create now:" "${diffs[@]}"
  fi
  if [ "$(docker container inspect -f '{{.State.Running}}' "$container")" != true ]; then
    docker start "$container" >/dev/null
  fi
else
  # --cgroupns private: buildkitd puts every build step in a child cgroup of its own root (seen on the host as
  # docker-<id>.scope/buildkit/<step>), so the steps stay inside these caps. In the host cgroup namespace (a daemon's
  # default-cgroupns-mode), a privileged buildkitd would create them at the host's cgroup root, outside every cap.
  opts=(--name "$container" --privileged --cgroupns private --restart unless-stopped --init
    --cpus "$quire_cpus" --memory "$quire_mem_bytes" --memory-swap "$quire_mem_bytes" --pids-limit 4096
    -v "$volume:/var/lib/buildkit")
  if [ -n "$quire_cpuset" ]; then opts+=(--cpuset-cpus "$quire_cpuset"); fi
  if [ -n "$QUIRE_CGROUP_PARENT" ]; then opts+=(--cgroup-parent "$QUIRE_CGROUP_PARENT"); fi
  docker create "${opts[@]}" "$BUILDKIT_IMAGE" --config "$config" >/dev/null
  # docker cp, not a bind mount, so a removed worktree cannot take the daemon's config with it. A tar stream makes
  # the file root-owned in the container (a plain file copy keeps the host uid). The image ships /etc/buildkit.
  tar -C "$root/docker" --owner=0 --group=0 --numeric-owner --mode=0644 -cf - buildkitd.toml |
    docker cp - "$container:${config%/*}" >/dev/null
  docker start "$container" >/dev/null
  echo "created $container ($BUILDKIT_IMAGE): $quire_cpus CPUs${quire_cpuset:+ ($quire_cpuset)}, $quire_mem, no swap, cgroup parent ${QUIRE_CGROUP_PARENT:-none}" >&2
fi

# 3. Register it once.
if [ "$registered" = 0 ]; then
  docker buildx create --name "$builder" --driver remote "docker-container://$container" >/dev/null
  echo "registered $builder (remote driver, docker-container://$container)" >&2
fi
# Waits until buildkitd answers (it needs a moment after a start).
docker buildx inspect --bootstrap "$builder" >/dev/null

if [ "$builder_only" = 1 ]; then
  exit 0
fi

docker buildx build --builder "$builder" --load \
  -t quire-toolchain:latest -f "$root/docker/toolchain.Dockerfile" "$root/docker"
