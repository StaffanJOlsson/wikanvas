#!/usr/bin/env bash
# Builds the wikanvas-toolchain image (the toolchain target of docker/Dockerfile) with the capped wikanvas-limited
# builder (docs/ARCHITECTURE.md §13.1, §13.2, D38).
#   scripts/toolchain.sh                  # set up (or check) the builder, then build wikanvas-toolchain:latest
#   scripts/toolchain.sh --builder-only   # set up (or check) the builder only
# The builder is the BuildKit container wikanvas-buildkit (moby/buildkit, pinned by digest) with the dev.sh caps and CPU
# set, the build slice as cgroup parent when it is installed, config docker/buildkitd.toml and the state volume
# wikanvas-buildkit-state, registered as wikanvas-limited with buildx's remote driver. (buildx's docker-container driver
# ignores cgroup-parent under Docker's systemd cgroup driver, so buildx must not create the container itself.)
# If wikanvas-limited uses another driver, or wikanvas-buildkit has other caps, another cgroup parent, image or config
# than this script would give it now (for example after the slice was installed), it prints the commands that recreate
# both and exits 1; it never removes either itself. The build cache lives in the volume and survives a recreate.
# Caps (may only be lowered): WIKANVAS_BUILD_CPUS, WIKANVAS_BUILD_MEM, WIKANVAS_BUILD_CPUSET, as for scripts/dev.sh.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
. "$root/scripts/lib/lock.sh"
. "$root/scripts/lib/slice.sh"

# v0.34.1 was under two weeks old on 2026-10-28, so the newest older patch is pinned (ARCHITECTURE §16, risk 23).
BUILDKIT_IMAGE=moby/buildkit:v0.34.0@sha256:b059f8d7226d0b326bc871af489a5dddf65e36af4d20d14251b072974d9ee87c
container=wikanvas-buildkit
builder=wikanvas-limited
volume=wikanvas-buildkit-state
config=/etc/buildkit/buildkitd.toml

usage() {
  sed -n '2,13s/^# \{0,1\}//p' "$root/scripts/toolchain.sh"
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

wikanvas_caps || exit 2
wikanvas_lock || exit 1
wikanvas_slice

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
want_cpus=$((wikanvas_cpus * 1000000000))
if docker container inspect "$container" >/dev/null 2>&1; then
  IFS='|' read -r image nano mem swap pids cpuset parent priv cgroupns < <(docker container inspect -f \
    '{{.Config.Image}}|{{.HostConfig.NanoCpus}}|{{.HostConfig.Memory}}|{{.HostConfig.MemorySwap}}|{{.HostConfig.PidsLimit}}|{{.HostConfig.CpusetCpus}}|{{.HostConfig.CgroupParent}}|{{.HostConfig.Privileged}}|{{.HostConfig.CgroupnsMode}}' \
    "$container")
  diffs=()
  [ "$image" = "$BUILDKIT_IMAGE" ] || diffs+=("  image $image (want $BUILDKIT_IMAGE)")
  [ "$nano" = "$want_cpus" ] || diffs+=("  NanoCpus $nano (want $want_cpus)")
  [ "$mem" = "$wikanvas_mem_bytes" ] || diffs+=("  Memory $mem (want $wikanvas_mem_bytes)")
  [ "$swap" = "$wikanvas_mem_bytes" ] || diffs+=("  MemorySwap $swap (want $wikanvas_mem_bytes)")
  [ "$pids" = 4096 ] || diffs+=("  PidsLimit $pids (want 4096)")
  [ "$cpuset" = "$wikanvas_cpuset" ] || diffs+=("  CpusetCpus '$cpuset' (want '$wikanvas_cpuset')")
  [ "$parent" = "$WIKANVAS_CGROUP_PARENT" ] || diffs+=("  CgroupParent '$parent' (want '$WIKANVAS_CGROUP_PARENT')")
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
    --cpus "$wikanvas_cpus" --memory "$wikanvas_mem_bytes" --memory-swap "$wikanvas_mem_bytes" --pids-limit 4096
    -v "$volume:/var/lib/buildkit")
  if [ -n "$wikanvas_cpuset" ]; then opts+=(--cpuset-cpus "$wikanvas_cpuset"); fi
  if [ -n "$WIKANVAS_CGROUP_PARENT" ]; then opts+=(--cgroup-parent "$WIKANVAS_CGROUP_PARENT"); fi
  docker create "${opts[@]}" "$BUILDKIT_IMAGE" --config "$config" >/dev/null
  # docker cp, not a bind mount, so a removed worktree cannot take the daemon's config with it. A tar stream makes
  # the file root-owned in the container (a plain file copy keeps the host uid). The image ships /etc/buildkit.
  tar -C "$root/docker" --owner=0 --group=0 --numeric-owner --mode=0644 -cf - buildkitd.toml |
    docker cp - "$container:${config%/*}" >/dev/null
  docker start "$container" >/dev/null
  echo "created $container ($BUILDKIT_IMAGE): $wikanvas_cpus CPUs${wikanvas_cpuset:+ ($wikanvas_cpuset)}, $wikanvas_mem, no swap, cgroup parent ${WIKANVAS_CGROUP_PARENT:-none}" >&2
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

# The toolchain target of docker/Dockerfile, with the repo root as context (.dockerignore keeps .cache, target,
# node_modules, the dists and data out of it; this target copies nothing from the context anyway).
docker buildx build --builder "$builder" --load \
  -t wikanvas-toolchain:latest -f "$root/docker/Dockerfile" --target toolchain "$root"
