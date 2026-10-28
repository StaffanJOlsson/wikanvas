# Quire

Self-hosted wiki: an Outline-style sidebar of collections with documents and Inkboard whiteboards nested to any depth.
Rust (axum) server, SQLite, React + TipTap frontend, Inkboard (vendor/inkboard, git submodule) compiled to WebAssembly for boards.

## Builds run in Docker, with resource caps — never on the host

The owner's machine has been brought to 100% by Rust builds before; caps are a hard requirement (docs/ARCHITECTURE.md
§13.1, D38).

- Run every cargo, rustc, rustfmt, trunk, wasm-bindgen, wasm-opt, node, pnpm and npx command through
  `scripts/dev.sh <command>`. It runs the `quire-toolchain` image capped at 6 CPUs and 12 GB RAM (no swap), pinned to
  the host's last 6 cores (`--cpuset-cpus`, so `nproc` says 6), at nice 10. Do not call cargo/pnpm/node directly on the
  host, and do not raise the caps without the owner asking: `QUIRE_BUILD_CPUS`, `QUIRE_BUILD_MEM` and
  `QUIRE_BUILD_CPUSET` may only be lowered (the scripts refuse higher values).
- Build Docker images only with the capped builder `quire-limited` (`docker buildx build --builder quire-limited ...`).
  `scripts/toolchain.sh` creates it: the BuildKit container `quire-buildkit` with the same caps and cores, registered
  with buildx's `remote` driver. If it prints a `docker buildx rm quire-limited; docker rm -f quire-buildkit;
  scripts/toolchain.sh` instruction (caps, slice or config changed), run exactly that. The default builder has no caps.
- `scripts/toolchain.sh` rebuilds the toolchain image after editing docker/toolchain.Dockerfile
  (`scripts/toolchain.sh --builder-only` only sets up or checks the builder).
- **One build at a time, by lock.** The build scripts take one per-user lock (`scripts/lib/lock.sh`, a flock on
  `~/.cache/quire/build.lock` resolved through the passwd database) and print "waiting for another Quire build…" while
  another holds it. A script started from a lock holder sees `QUIRE_LOCK_HELD=1` and never waits for the lock, so nested
  calls do not deadlock (it takes the lock only if nobody holds it). Never set `QUIRE_LOCK_HELD` by hand.
- **The total cap is the systemd slice `quire-build.slice`** (6 CPUs, 12 GB, no swap, low CPU and IO weight via its
  parent `quire.slice`). When it is installed, every build container and the BuildKit daemon join it as cgroup parent;
  without it builds still run, capped per container, after the warning "quire-build.slice not installed: per-container
  caps only". `scripts/install-build-slice.sh --check` (no root) shows whether it is installed and holds its values.
- **Never run `sudo`** or any other root step: the coordinator runs the one privileged step
  (`scripts/install-build-slice.sh`) through the owner's askpass dialog.
- **Never build inside `vendor/inkboard`** and never modify it (`git -C vendor/inkboard status --porcelain` stays
  empty); Inkboard builds run from the patched copy in `.cache/inkboard-src`.
- **Format only with `scripts/fmt.sh`**, never `cargo fmt --all` (it reaches into `vendor/inkboard`).

## Hosting

The app is hosted as a Docker container behind Caddy, its HTTPS front (`docker/Caddyfile`), in one compose project. Both
containers' runtime limits live in `compose.yaml`: the app at 2 CPUs / 1 GiB, Caddy at 0.5 CPU / 128 MiB, neither with
swap. The app has no published port; only Caddy publishes 80 and 443. `compose.yaml` has no `build:` key: images are
built only through the capped builder.

## Licensing

Quire is MIT. Inkboard is MIT OR Apache-2.0: keep its LICENSE files and mark changed files. Do not copy
code from AGPL/BSL projects (Outline, Lore, Docmost, Wiki.js, Drafft.ink); reading them for ideas is fine.
