# W0a handoff: build and runtime caps

Status: tasks 1–7 and 9 done; every acceptance command above "after the coordinator installed the slice" passes
(2026-10-28). Tasks 8 and 10 are the coordinator's: install the slice, recreate the BuildKit container, commit.
Nothing was committed, nothing ran as root, and `scripts/install-build-slice.sh` was run only as `--check` (and once
without root, to see it refuse).

**Task 8 is deferred (2026-10-28):** the owner was away from the machine and could not answer the askpass dialog, so
the install was cancelled and W0a was committed without it. Until the install runs, builds use the per-container caps
and print "wikanvas-build.slice not installed: per-container caps only", and `wikanvas-buildkit` has no cgroup parent.
When the owner is back, run the install below, then task 8. After the install, `scripts/toolchain.sh` and
`scripts/toolchain.sh --builder-only` exit 1 with the recreate instruction until the builder is recreated.

**Task 8 done (2026-10-28, evening, after the M1 gate):** the owner was back, and the coordinator ran the install through
the askpass dialog. Every value holds (`--check` exits 0). `scripts/toolchain.sh --builder-only` then printed the
recreate instruction (`CgroupParent '' (want 'wikanvas-build.slice')`); the builder was recreated with exactly that, the
build cache survived (every toolchain step `CACHED`), and both post-checks pass: `wikanvas-buildkit` has cgroup parent
`wikanvas-build.slice`, and a `scripts/dev.sh` container shows up in the slice as `docker-init -- nice -n 10 …`. The
install's `--check` printed `printf: --: invalid option` for its info lines (a format string starting with `--`);
fixed in `info()`.

## Coordinator: the one privileged step

The slice is ready to install. Run it once, interactively, through the owner's askpass dialog (one dialog for the
whole step), from the repo root:

```sh
SUDO_ASKPASS=/home/staffan/.local/bin/claude-askpass sudo -A \
  -p "install the Wikanvas build cap (systemd slice: 6 CPUs, 12 GB RAM, no swap, low priority) :: $PWD/scripts/install-build-slice.sh" \
  "$PWD/scripts/install-build-slice.sh"
```

It installs `docker/wikanvas.slice` and `docker/wikanvas-build.slice` into `/etc/systemd/system/` (`install -o root -g
root -m 0644`), runs `systemctl daemon-reload` and `systemctl start wikanvas-build.slice`, then its own `--check`. It
never calls `sudo` itself. Exit 0 means every value holds. It refuses (exit 1, nothing written) a unit file that is a
symlink or not a regular file, or whose directives differ from the approved ones (see "Independent review").

## Task 8 (after the install)

```sh
scripts/install-build-slice.sh --check          # exit 0; prints the values, the members, the disk scheduler
scripts/toolchain.sh --builder-only             # now exits 1: CgroupParent '' (want 'wikanvas-build.slice'), and
                                                # prints the recreate instruction below
docker buildx rm wikanvas-limited; docker rm -f wikanvas-buildkit; scripts/toolchain.sh --builder-only
```

`--builder-only` recreates and registers the builder without rebuilding the toolchain image. The instruction the
script prints ends in plain `scripts/toolchain.sh` (as §13.1 says), which also rebuilds `wikanvas-toolchain:latest`.
The new `wikanvas-buildkit-state` volume holds no toolchain cache yet (the image was not rebuilt in W0a), so that would
be a full rebuild: about ten minutes at 6 CPUs, and the floating `rust:1.99-trixie` and `node:24-trixie-slim` tags
may resolve to newer bases. W0b rebuilds the toolchain anyway (it moves it into `docker/Dockerfile`).

Then the post-install acceptance:

```sh
scripts/install-build-slice.sh --check
( scripts/dev.sh sleep 15 & sleep 5; systemctl status wikanvas-build.slice --no-pager | grep -q 'docker-init -- nice -n 10 sleep 15' ); rc=$?; wait; test $rc -eq 0
docker inspect -f '{{.HostConfig.CgroupParent}}' wikanvas-buildkit | grep -qx wikanvas-build.slice
```

The second line is PLAN's, made strict in two ways (see "Documentation issues" 1 and 5):
- `rc=$?; … test $rc -eq 0` is added. As written in PLAN it ends in `; wait`, whose status is always 0.
- The grep names the dev.sh container's own process. PLAN's `'docker-.*\.scope'` also matches the recreated
  `wikanvas-buildkit` scope, so it would pass even if `dev.sh` never joined the slice. `systemctl status` prints full
  command lines when its output is not a terminal (checked: `/sbin/docker-init -- nice -n 10 sleep 15`).

## What was built

| File | What it does |
|---|---|
| `scripts/lib/lock.sh` | `wikanvas_lock`: flock on `<home>/.cache/wikanvas/build.lock`, `<home>` from `getent passwd "$(id -u)"`; prints "waiting for another Wikanvas build…" to stderr; exports `WIKANVAS_LOCK_HELD=1`. With it set, it never waits, and takes the lock only if nobody holds it. Every failure returns 1 explicitly. The lock is a dynamically numbered fd, inherited by the `exec docker run`, so it is held until the container exits. |
| `scripts/lib/slice.sh` | `wikanvas_slice` (§13.1: systemd driver and `FragmentPath=/etc/systemd/system/<slice>`, never `LoadState`; else `WIKANVAS_CGROUP_PARENT=` and a once-per-process-tree warning; `WIKANVAS_BUILD_SLICE` for the test) and `wikanvas_caps` (see deviation 1). |
| `scripts/dev.sh` | §13.1: `--rm --init`, `--cpus`, `--cpuset-cpus` (last N online CPUs: `10-15` here), `--memory` = `--memory-swap`, `--pids-limit 4096`, `--shm-size 1g`, `--cgroup-parent` when the slice is installed, `CARGO_BUILD_JOBS`/`BINARYEN_CORES`/`RAYON_NUM_THREADS`/`GOMAXPROCS`/`UV_THREADPOOL_SIZE`, `CARGO_HOME` and `HOME` as before, `nice -n 10`, `WIKANVAS_DEV_IMAGE`, `WIKANVAS_DEV_NETWORK`, `WIKANVAS_DEV_PORTS`, `WIKANVAS_SHARED_CARGO_HOME`. Takes the lock. |
| `scripts/toolchain.sh` | Takes the lock, calls `wikanvas_slice`, checks or creates `wikanvas-buildkit` (with `--cgroupns private`) and the `wikanvas-limited` remote-driver registration (§13.1), then builds `docker/toolchain.Dockerfile` as before. `--builder-only` stops after the builder. |
| `scripts/install-build-slice.sh` | Install as root (never calls `sudo`; without root it prints the askpass command and exits 1) or `--check` without root. Exit codes 0 / 1 / 2 (usage) / 3 (not installed), per §13.1. Hardened for root: `/bin/bash`, a fixed PATH, approved-content check of both unit files before writing either. |
| `docker/wikanvas-build.slice` | `[Slice] CPUQuota=600% MemoryMax=12G MemorySwapMax=0 CPUWeight=20 IOWeight=20 TasksMax=16384`. |
| `docker/wikanvas.slice` | `[Slice] CPUWeight=20 IOWeight=20`. |
| `docker/buildkitd.toml` | `[worker.oci] max-parallelism = 2`, `gc = true`, `maxUsedSpace = "30GB"`. The live daemon reports four GC rules capped at 30 GiB. |
| `compose.yaml` | §13.3 verbatim, with the Caddy digest filled in. |
| `docker/Caddyfile` | §13.4 verbatim. |
| `CLAUDE.md` | Task 9: the lock and `WIKANVAS_LOCK_HELD`, the cpuset, the slice and `--check` (and the warning without it), never `sudo`, never build in `vendor/inkboard`, `scripts/fmt.sh` only, the remote-driver builder and its recreate instruction, and the Caddy-fronted hosting with both containers' limits. |
| `docs/ARCHITECTURE.md` | §16 risk 23 only: the BuildKit pin (below). |

Removed (task 7): `vendor/inkboard/target` (977 MB) and `vendor/inkboard/crates/inkboard-app/web/dist` (27 MB), both
git-ignored build output; `git -C vendor/inkboard status --porcelain` is empty.

Docker state changed: the old docker-container builder `wikanvas-limited` (container `buildx_buildkit_wikanvas-limited0`
and its state volume) was removed with `docker buildx rm wikanvas-limited`. The new container `wikanvas-buildkit` runs
(6 CPUs on `10-15`, 12 GiB, no swap, pids 4096, privileged, `--init`, restart `unless-stopped`, no cgroup parent
yet), with volume `wikanvas-buildkit-state`, registered as `wikanvas-limited` (driver `remote`,
`docker-container://wikanvas-buildkit`, BuildKit v0.34.0). A throwaway `FROM scratch` build with `--load` through it
worked. `wikanvas-toolchain:latest` was not touched. Pulled: the pinned `moby/buildkit:v0.34.0` and `caddy:2.11.7`
images. `moby/buildkit:buildx-stable-1` (the old builder's image) is now unused and can be removed with
`docker rmi moby/buildkit:buildx-stable-1` if nothing else needs it.

## Pins

| Image | Pin | Why |
|---|---|---|
| BuildKit | `moby/buildkit:v0.34.0@sha256:b059f8d7226d0b326bc871af489a5dddf65e36af4d20d14251b072974d9ee87c` (index) | v0.34.1 was released 2026-10-20 (GitHub release and Docker Hub push): 8 days old, under two weeks. v0.34.0 (2026-10-07, 21 days) is the newest older patch. Recorded in ARCHITECTURE §16, risk 23. |
| Caddy | `caddy:2.11.7@sha256:5e5286cb37d44086d651a5363f05f43a692ce4d649d88fb1bd844579f45fa9e8` (index) | Caddy v2.11.7 was released 2026-10-03 (25 days), so the doc's version stands. The official image index behind the tag was rebuilt on 2026-10-21 (alpine 3.23 base); the digest pins that build. |

## Acceptance (pre-slice part), 2026-10-28

Each command ran verbatim in its own `bash -c` from the repo root. Every one passed:

| # | Command (abridged) | Result |
|---|---|---|
| 1 | `bash -n scripts/*.sh scripts/lib/*.sh` | PASS |
| 2 | `( scripts/dev.sh sleep 20 & sleep 2; scripts/dev.sh true 2>&1 \| grep -q 'waiting for another Wikanvas build' ); wait` | PASS, and PASS with the subshell's own status checked |
| 3 | `bash -c '. scripts/lib/lock.sh && wikanvas_lock && timeout 120 scripts/dev.sh true'` | PASS (returns at once) |
| 4 | `WIKANVAS_BUILD_SLICE=wikanvas-missing-test.slice scripts/dev.sh true 2>&1 \| grep -q 'not installed: per-container caps only'` | PASS |
| 5–8 | `nproc` = 6, `cpu.max` = `600000 100000`, `memory.max` = 12884901888, `memory.swap.max` = 0 | PASS |
| 9 | `docker buildx inspect wikanvas-limited \| grep -qE 'Driver: +remote'` | PASS |
| 10 | `wikanvas-buildkit` NanoCpus/Memory/MemorySwap = `6000000000 12884901888 12884901888` | PASS |
| 11 | `wikanvas-buildkit` CpusetCpus non-empty (`10-15`) | PASS |
| 12 | `max-parallelism = 2` in the container's `/etc/buildkit/buildkitd.toml` | PASS |
| 13 | `docker compose -f compose.yaml config -q && ! grep -q '^\s*build:' compose.yaml` | PASS |
| 14 | compose JSON: no `wikanvas` ports, cpus 2 / 0.5, two Caddy ports, `ip_range` `172.31.240.8/29`, Caddy at `172.31.240.2` | PASS |
| 15 | `! grep -qE '^\s*(encode\|header)\b' docker/Caddyfile` | PASS |
| 16 | `caddy validate` for `localhost internal` and `wiki.example.com acme` | PASS |
| 17 | Caddy starts read-only with `--cap-drop ALL --cap-add NET_BIND_SERVICE --security-opt no-new-privileges:true` | PASS |
| 18 | `test ! -e vendor/inkboard/target` | PASS |
| 19 | `git -C vendor/inkboard status --porcelain \| wc -l \| grep -qx 0` | PASS |

Other checks:
- `scripts/install-build-slice.sh --check` exits 3 now (not installed). It prints the install command.
- Without root, the install exits 1 and prints the command.
- The two unit files pass `systemd-analyze verify`. Loaded into the *user* manager for a moment (no root; removed
  right after), systemd reported exactly the values `--check` expects: `CPUQuotaPerSecUSec=6s`,
  `MemoryMax=12884901888`, `MemorySwapMax=0`, weights 20, `TasksMax=16384`.
- **Risk 22, verified, no change needed.** The image's `/usr/bin/caddy` carries `cap_net_bind_service=ep`.
  - With `no-new-privileges`, `cap_drop: ALL` and `NET_BIND_SERVICE`, Caddy listens on TCP 80 and 443 and has no UDP
    socket (HTTP/3 off). It also issues its local-CA certificate onto the tmpfs `/data`.
  - Negative control: without `NET_BIND_SERVICE` it never reaches "serving initial configuration", so check 17 can
    fail.
- `dev.sh` inside the container:
  - uid 1000, nice 10, `Cpus_allowed_list` 10-15, pids.max 4096, `/dev/shm` 1 GiB, all five thread variables 6.
  - `WIKANVAS_BUILD_CPUS=4 WIKANVAS_BUILD_MEM=6g` gives 4 CPUs on 12-15, `400000 100000` and 6442450944.
  - `WIKANVAS_SHARED_CARGO_HOME`, `WIKANVAS_DEV_IMAGE`, `WIKANVAS_DEV_NETWORK` and `WIKANVAS_DEV_PORTS` work.
- `toolchain.sh` drift detection is tested:
  - The old docker-container builder prints the instruction and exits 1.
  - `WIKANVAS_BUILD_CPUS=4` names NanoCpus and CpusetCpus; an edited `buildkitd.toml` names the config. Both exit 1.
  - A rerun with nothing changed exits 0 and does nothing.
- ShellCheck 0.11 (run once in a capped, network-less container; nothing added to the repo) is clean on all five
  scripts.

## Deviations and choices (where the docs were silent or ambiguous; the safer option taken)

1. **Caps are enforced, not only documented.** "May only be lowered" is a rule in the docs; `wikanvas_caps` makes it a
   check, and `dev.sh` and `toolchain.sh` refuse anything higher with exit 2:
   - `WIKANVAS_BUILD_CPUS` must be a whole number from 1 to 6 (Docker would accept `4.5`; the default cpuset needs a
     whole count).
   - `WIKANVAS_BUILD_MEM` must be an integer size of at most 12g (`12g`, `6G`, `512m`, plain bytes).
   - `WIKANVAS_BUILD_CPUSET` may name at most `WIKANVAS_BUILD_CPUS` CPUs.

   The ceilings are two constants in `scripts/lib/slice.sh`; raise them only when the owner asks.
2. **`wikanvas_caps` lives in `scripts/lib/slice.sh`.** §13.1 gives that file only `wikanvas_slice`. Putting the cap
   computation there lets `dev.sh` and `toolchain.sh` share it, so "the same set as dev.sh" for the BuildKit
   container holds by construction.
   - The default cpuset is the last N ids of `/sys/devices/system/cpu/online`, not `nproc`, which follows the calling
     process's own affinity. A non-contiguous list is compressed to a list such as `2,4-7`.
   - The thread variables use min(`WIKANVAS_BUILD_CPUS`, the cpuset's size), so a smaller explicit cpuset also lowers
     them.
3. **Slice warning text.**
   - The warning names `$WIKANVAS_BUILD_SLICE`, for example "wikanvas-missing-test.slice not installed: per-container
     caps only (scripts/install-build-slice.sh --check)". With the default it is exactly §13.1's text.
   - When the unit file is installed but Docker's cgroup driver is not systemd, it says so instead: "… not used:
     Docker's cgroup driver is 'cgroupfs', not systemd: per-container caps only …", because "not installed" would be
     false there.
   - "Prints once" is per process tree, through an exported `_WIKANVAS_SLICE_WARNED`. Setting it by hand only hides the
     warning; detection always reruns.
   - A `WIKANVAS_BUILD_SLICE` without `.slice` counts as not installed.
4. **The BuildKit container gets `--init`**, as buildx's own docker-container builder does. The image entrypoint is
   `buildkitd-entrypoint`, the same one that builder ran under `docker-init`.
   - `buildkitd` gets `--config /etc/buildkit/buildkitd.toml` explicitly, so a missing config fails loudly instead of
     falling back silently to defaults.
   - The config goes in through `docker cp` of a tar stream with `--owner=0 --group=0 --mode=0644`. A plain file copy
     left it owned by the host uid 1000 inside the container. The pinned image ships `/etc/buildkit`.
5. **`toolchain.sh` checks more than caps and cgroup parent.** It also checks the image reference, `PidsLimit`,
   `Privileged`, the CPU set and the config file's content, against what it would create now. Any difference prints
   the §13.1 instruction and exits 1. This covers the hand updates of risk 23 (a new pin, a changed
   `buildkitd.toml`). It never removes anything itself.
   - A `wikanvas-limited` with the `remote` driver but another endpoint counts as "another driver".
   - A stopped `wikanvas-buildkit` with matching settings is simply started.
   - After every run it calls `docker buildx inspect --bootstrap`, so a fresh daemon is reachable before the build.
6. **`scripts/toolchain.sh --builder-only`** (new flag): sets up or checks the builder without building the image.
   Without it, setting up the builder for the acceptance checks, and task 8's recreate, would each force a full
   toolchain rebuild.
7. **`install-build-slice.sh --check`** also:
   - verifies `TasksMax=16384`, which the unit file sets;
   - checks `ControlGroup` only while the slice is active (an inactive slice reports none), and prints a `--` line
     otherwise;
   - finds the disk through `docker info`'s `DockerRootDir` (falling back to `/var/lib/docker`) and walks partitions,
     LVM or dm-crypt to the whole disk;
   - says whether `io.weight` is honoured (BFQ, or `io.cost` enabled). Here it is `nvme0n1`, `[none] mq-deadline`,
     io.cost off: `IOWeight` has no effect on this disk, as §13.1 anticipates;
   - prints the install command when it exits 3.

   `-h`/`--help` prints usage; any other argument exits 2.
8. **The unit files have a `[Unit] Description=` and a comment header.** These are additive and change no
   `[Slice]` value.
9. **`WIKANVAS_SHARED_CARGO_HOME`** must be an absolute path (Docker would read a relative one as a named volume). It is
   mounted over the worktree's `/work/.cache/cargo`, so `CARGO_HOME` inside the container, and with it cargo's
   fingerprints, stay the same with and without it.
10. **`dev.sh` without a command** prints usage and exits 2. The header's port example is now
    `127.0.0.1:8080:8080` (loopback) instead of `8080:8080`.
11. **`compose.yaml`** differs from §13.3 only on the Caddy `image:` line: the digest is filled in, and the comment
    reads "official image, pinned by digest (D57)" instead of "(W0a fills the digest)".

## Documentation issues (requests to the orchestrator; I may not edit these)

1. **PLAN W0a acceptance, lock line** (and the post-slice `systemctl status` line):
   - `( … | grep -q … ); wait` always exits 0, because the list's status is that of `wait`. The background job
     belongs to the subshell, so the outer `wait` has nothing to wait for and returns 0.
   - Suggested form: `( scripts/dev.sh sleep 20 & sleep 2; scripts/dev.sh true 2>&1 | grep -q 'waiting for another
     Wikanvas build' ); rc=$?; wait; test $rc -eq 0`.
   - The lock check passes in that strict form too.
2. **ARCHITECTURE §13.1** (step 1 of "The wikanvas-limited builder") and PLAN W0a task 5 still name
   `moby/buildkit:v0.34.1`. The pin is v0.34.0 (§16 risk 23 records why); update the two mentions, or move to v0.34.1
   after 2026-11-03 by changing `BUILDKIT_IMAGE` in `scripts/toolchain.sh` and recreating.
3. **The §13.4 Caddyfile is not `caddy fmt`-clean.** The only difference is the extra spaces before the inline
   comments. Caddy logs a warning at every start ("Caddyfile input is not formatted"). It is harmless; I kept the
   text verbatim. If the doc is reflowed, run `caddy fmt` on it.
4. **§13.1 "Who takes the lock"** lists `image.sh`, `image-smoke.sh`, `e2e.sh`, `agent-test.sh` and `cli-dist.sh`. Those
   are W0b's and later. They should start with `. scripts/lib/lock.sh; wikanvas_lock` and, where §13.1 says so,
   `. scripts/lib/slice.sh; wikanvas_slice`. Anything that runs its own capped container should also use `wikanvas_caps`
   rather than re-deriving the cpuset.

## Environment findings (for the owner and W0b/W7)

- **Host port 443 is taken.** The owner's own Caddy container (`caddy`, image `caddy-duckdns:local`, host network,
  restart `unless-stopped`) listens on `*:443` (TCP and UDP). `docker compose up` of the production project would
  fail to bind `127.0.0.1:443` until the owner sets `WIKANVAS_HTTPS_PORT` (for example `8443`) in `.env`, which also
  goes into the derived public URL (§8), or fronts Wikanvas with that existing Caddy instead. Port 80 is free. The W0a
  checks publish nothing and were not affected. The image smoke (`127.0.0.1:18443`) and the e2e and agent projects
  (no Caddy) are not affected either.
- Host: 16 CPUs (i7-10700, `0-15` online), so the build cpuset is `10-15`. Docker 29.8.2 uses the systemd cgroup
  driver on cgroup v2; systemd 259; Compose v5.6.0; buildx v0.37.1. No existing Docker network overlaps
  `172.31.240.0/28`.
- `/var/lib/docker` is on `nvme0n1` with scheduler `none`: `IOWeight=20` will not act there (risk 13).

## Files for the W0a commit (task 10)

New: `scripts/lib/lock.sh`, `scripts/lib/slice.sh`, `scripts/install-build-slice.sh`, `docker/buildkitd.toml`,
`docker/wikanvas-build.slice`, `docker/wikanvas.slice`, `docker/Caddyfile`, `compose.yaml`, `docs/handoff/W0a.md`.
Changed: `scripts/dev.sh`, `scripts/toolchain.sh`, `CLAUDE.md`, `docs/ARCHITECTURE.md` (§16 risk 23 only).
PLAN task 10 also puts the pre-existing untracked files into this first commit: `docs/`, `.cargo/config.toml`,
`.gitignore` and `docker/toolchain.Dockerfile` (unchanged by W0a). The submodule and `.gitmodules` are already
staged.

## Independent review (verifier, 2026-10-28)

A second agent re-ran all 19 pre-slice acceptance commands, all of which passed before and after the fixes below. It
then reviewed the W0a files against §13.1, §13.3 and §13.4.
- `compose.yaml` differs from the §13.3 block only on the Caddy `image:` line.
- `docker/Caddyfile` is byte-identical to §13.4.
- Both pins resolve to the recorded index digests with `docker buildx imagetools inspect`.
- ShellCheck 0.11 (capped, `--network none`, image removed afterwards) is clean on all five scripts.

Problems found and fixed:

1. **Lock bypass when `flock` fails** (`scripts/lib/lock.sh`).
   - `wikanvas_lock` relied on `set -e` to stop after a failed `flock`. `set -e` does not act inside a function called
     from `if`, `&&` or `||`.
   - With a failing `flock`, `wikanvas_lock || exit 1` and `if wikanvas_lock` both returned 0 and exported
     `WIKANVAS_LOCK_HELD=1` without holding the lock (reproduced with a stub `flock`).
   - Every step now returns 1 explicitly. `dev.sh` and `toolchain.sh` call `wikanvas_lock || exit 1`.
2. **A stale or hand-set `WIKANVAS_LOCK_HELD` ran unlocked** (`scripts/lib/lock.sh`).
   - A nested call still never waits, so nesting cannot deadlock.
   - It now tries a non-blocking flock. That fails at once when an ancestor holds the lock, and takes the lock when
     nobody does.
   - Checked: a `WIKANVAS_LOCK_HELD=1 scripts/dev.sh sleep 12` now makes a second `dev.sh` print "waiting…". The nested
     acceptance line still returns in 0.2 s.
3. **Cap checks that could pass a value they did not understand** (`scripts/lib/slice.sh`).
   - `WIKANVAS_BUILD_CPUS=99999999999999999999` passed `wikanvas_caps`: `[` fails on a number beyond 64 bits, and a
     failed test is no refusal.
   - `WIKANVAS_BUILD_MEM=17179869185g` wrapped around to 1 GiB in the check, while Docker got the raw string. Docker
     happened to refuse both, so nothing escaped.
   - A `WIKANVAS_BUILD_CPUSET` such as `0-99999999999` would have made `seq` print 10^11 lines before the count check.
   - Now: at most 3 digits for CPUs, 15 for sizes, and a product that would overflow counts as 2^62 (refused as above
     12g); 4 digits per CPU id. `12bb` and the like are refused.
   - `dev.sh` and `toolchain.sh` now pass `wikanvas_mem_bytes` to `--memory`/`--memory-swap`, so the value checked is
     the value applied.
4. **BuildKit build steps relied on the daemon's default cgroup namespace** (`scripts/toolchain.sh`).
   - Checked on the host: build-step processes sit in `docker-<id>.scope/buildkit/<step>`, inside the container's
     caps (`600000 100000`, 12884901888, cpuset `10-15`).
   - That nesting holds only because the container has a private cgroup namespace, which is merely Docker's default on
     cgroup v2. With `default-cgroupns-mode: host`, the privileged buildkitd would create the step cgroups at the
     host's cgroup root, outside every cap.
   - The container is now created with `--cgroupns private`, and the drift check requires `CgroupnsMode=private`.
   - `wikanvas-buildkit` was recreated with the new script (volume kept). Steps still nest, and a rerun exits 0.
5. **`WIKANVAS_BUILD_SLICE` could choose another installed slice as cgroup parent** (`scripts/lib/slice.sh`).
   - It is meant only for the warning test. Now it changes which unit is tested, never the parent.
   - `WIKANVAS_CGROUP_PARENT` is `wikanvas-build.slice` or empty. Another installed slice gets "… not used: only
     wikanvas-build.slice is a build cgroup parent".
6. **`install-build-slice.sh`, which runs as root, followed caller-controlled paths and environment.**
   - *Interpreter and PATH.* `#!/usr/bin/env bash` and every command were looked up through PATH. The caller's PATH
     here starts with user-writable directories (`~/.grok/bin`, linuxbrew, `~/.local/bin`). Whether sudo-rs replaces
     it (`secure_path`) cannot be checked without root: sudoers is unreadable. The script now uses `#!/bin/bash` and
     a fixed `PATH=/usr/sbin:/usr/bin:/sbin:/bin`, and unsets `CDPATH`, `BASH_ENV`, `ENV` and `TMPDIR`.
   - *Environment as root.* In install mode it also sets `HOME=/root` and unsets `DOCKER_*`, because the final
     `--check` runs `docker info`, which runs the CLI plugins under `$HOME/.docker`. `systemctl show` gets
     `--no-pager`.
   - *Symlinked unit sources.* The sources were installed with `install`, which follows a symlink. A
     `docker/wikanvas.slice -> /etc/shadow` would have been copied world-readable into `/etc/systemd/system`. Each
     source is now read once:
     - It must be a regular file, not a symlink.
     - The read is bounded (`timeout 10 head -c 16385`, at most 16 KiB).
     - It is accepted only if its non-comment lines are exactly the approved directives (`UNIT_DIRECTIVES`: the §13.1
       values; any plain-text `Description=`). Line continuations and `%` specifiers are refused.
     - A refused file's content is never printed, so a symlink to a secret leaks nothing.
     - Both files are checked before either is written. The checked bytes are what gets installed.
   - *Staging.* The checked bytes are staged in `mktemp -d /run/wikanvas-install-slice.XXXXXXXX`, which only root can
     write, and the directory is removed on exit. Staging under `$TMPDIR` would let a caller's directory be swapped
     under root.
   - *Re-running.* A first attempt with `install … /dev/stdin` failed on the second run: uutils coreutils 0.10's
     `install` gives "No such file or directory" when the target exists. That would have broken §13.1's "re-running
     it is harmless". The staging file avoids it.
   - *How it was tested, without root.* A scratch copy pointed at scratch directories, with the root check removed
     and `systemctl` stubbed, installed byte-identical files twice. It refused a raised `MemoryMax` without touching
     the existing unit. The real script was not run beyond `--check` and the no-root refusal.

Documentation issues, continuing the numbered list above (requests to the orchestrator):

5. **PLAN post-slice acceptance**: `systemctl status wikanvas-build.slice --no-pager | grep -q 'docker-.*\.scope'`
   is satisfied by the recreated `wikanvas-buildkit` alone, so it proves nothing about `dev.sh`. Use
   `grep -q 'docker-init -- nice -n 10 sleep 15'` together with the strict `rc=$?` form (Task 8 above).
6. **Caddy pin age**: the version (2.11.7, released 2026-10-03) is older than two weeks. The pinned index digest,
   however, is Docker's rebuild of 2026-10-21 (7 days old, alpine 3.23 base), and older builds of the tag cannot be
   listed. Kept as pinned; recorded here in case the two-week rule is meant for image builds too.

Residual risks (not fixable in W0a's files):
- The default buildx builder (`default`, docker driver) is still the user's global default and has no caps.
  `docker build` or `docker buildx build` without `--builder wikanvas-limited` escapes every cap. Every Wikanvas script
  names the builder explicitly, and `compose.yaml` has no `build:`. Making `wikanvas-limited` the global default
  (`docker buildx use --default`) would also change the owner's other projects, so it was not done.
- If the `docker run` client of `dev.sh` is killed with SIGKILL, the lock is released while the container keeps
  running (still capped). SIGTERM and SIGINT are proxied to the container.

## Follow-ups

- W0b: `docker/toolchain.Dockerfile` moves into `docker/Dockerfile` (`--target toolchain`). `toolchain.sh` keeps its
  builder logic; only its final `docker buildx build` line changes.
- `TODO`-free: `grep -rnE 'todo!\(|unimplemented!\(|TODO\(W' scripts docker compose.yaml CLAUDE.md` finds nothing.
