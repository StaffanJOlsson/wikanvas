# Wikanvas

Self-hosted wiki: an Outline-style sidebar of collections with documents and Inkboard whiteboards nested to any depth.
Rust (axum) server, SQLite, React + TipTap frontend, Inkboard (vendor/inkboard, git submodule) compiled to WebAssembly for boards.

## Builds run in Docker, with resource caps — never on the host

The owner's machine has been brought to 100% by Rust builds before; caps are a hard requirement (docs/ARCHITECTURE.md
§13.1, D38).

- Run every cargo, rustc, rustfmt, trunk, wasm-bindgen, wasm-opt, node, pnpm and npx command through
  `scripts/dev.sh <command>`. It runs the `wikanvas-toolchain` image capped at 6 CPUs and 12 GB RAM (no swap), pinned to
  the host's last 6 cores (`--cpuset-cpus`, so `nproc` says 6), at nice 10. Do not call cargo/pnpm/node directly on the
  host, and do not raise the caps without the owner asking: `WIKANVAS_BUILD_CPUS`, `WIKANVAS_BUILD_MEM` and
  `WIKANVAS_BUILD_CPUSET` may only be lowered (the scripts refuse higher values).
- Build Docker images only with the capped builder `wikanvas-limited`
  (`docker buildx build --builder wikanvas-limited ...`). `scripts/toolchain.sh` creates it: the BuildKit container
  `wikanvas-buildkit` with the same caps and cores, registered with buildx's `remote` driver. If it prints a
  `docker buildx rm wikanvas-limited; docker rm -f wikanvas-buildkit; scripts/toolchain.sh` instruction (caps, slice or
  config changed), run exactly that. The default builder has no caps.
- `scripts/toolchain.sh` rebuilds the toolchain image, the `toolchain` target of `docker/Dockerfile`, after editing its
  `toolchain-base` or `toolchain` stage (`scripts/toolchain.sh --builder-only` only sets up or checks the builder).
- **One build at a time, by lock.** The build scripts take one per-user lock (`scripts/lib/lock.sh`, a flock on
  `~/.cache/wikanvas/build.lock` resolved through the passwd database) and print "waiting for another Wikanvas build…"
  while another holds it. A script started from a lock holder sees `WIKANVAS_LOCK_HELD=1` and never waits for the lock,
  so nested calls do not deadlock (it takes the lock only if nobody holds it). Never set `WIKANVAS_LOCK_HELD` by hand.
- **The total cap is the systemd slice `wikanvas-build.slice`** (6 CPUs, 12 GB, no swap, low CPU and IO weight via its
  parent `wikanvas.slice`). When it is installed, every build container and the BuildKit daemon join it as cgroup
  parent; without it builds still run, capped per container, after the warning "wikanvas-build.slice not installed:
  per-container caps only". `scripts/install-build-slice.sh --check` (no root) shows whether it is installed and holds
  its values.
- **Never run `sudo`** or any other root step: the coordinator runs the one privileged step
  (`scripts/install-build-slice.sh`) through the owner's askpass dialog.
- **Never build inside `vendor/inkboard`** and never modify it (`git -C vendor/inkboard status --porcelain` stays
  empty); Inkboard builds run from the patched copy in `.cache/inkboard-src`.
- **Format only with `scripts/fmt.sh`**, never `cargo fmt --all` (it reaches into `vendor/inkboard`).
- The agent harness runs only through `scripts/agent-test.sh`, and agents use the `wikanvas` CLI as described in
  `docs/AGENTS.md`.
- `scripts/e2e.sh frame` needs the real engine: run `scripts/dev.sh scripts/inkboard.sh build` first (against the stub
  dist its `beforeAll` fails on purpose).

## Git and CI

- Wikanvas is live, so changes reach `main` only through pull requests with CI (`.github/workflows/ci.yml`,
  docs/ARCHITECTURE.md §14.2, D146): branch from `origin/main`, push often, open a pull request. When every job is
  green, land it as a fast-forward of `main` (`git push origin <branch>:main`), which keeps the commits, and the image
  tags built from them, unchanged.
- Pushing branches and landing green pull requests need no approval from the owner. Stable releases (a tag and a
  GitHub release) come now and then, not per change. Deploying an image to the owner's live wiki needs the owner's
  go-ahead in chat.
- `main` is protected by the repository ruleset "main": no deletion, no force push, linear history, and the three CI
  jobs ("Rust and web", "Image, smoke, agents, e2e", "Board frame") must have passed on the exact commit before it can
  be pushed, so a branch head reaches `main` only after its pull request's CI is green. The ruleset "release tags" keeps
  `v*` tags from moving or being deleted, and releases are immutable once published: attach every asset while the
  release is a draft.
- Dependabot opens security-update pull requests (any ecosystem) and a weekly grouped one for the actions. Treat them
  like any pull request: a lockfile change still updates `THIRD_PARTY.md`.
- CI runs on GitHub's 4-CPU runners with `WIKANVAS_BUILD_CPUS=4`. Only GitHub-owned actions are allowed; pin every
  action by commit SHA, with its release tag in a comment.

## Hosting

The app is hosted as a Docker container behind Caddy, its HTTPS front (`docker/Caddyfile`), in one compose project. Both
containers' runtime limits live in `compose.yaml`: the app at 2 CPUs / 1 GiB, Caddy at 0.5 CPU / 128 MiB, neither with
swap. The app has no published port; only Caddy publishes 80 and 443. `compose.yaml` has no `build:` key: images are
built only through the capped builder.

## Licensing

Wikanvas is MIT. Inkboard is MIT OR Apache-2.0: keep its LICENSE files and mark changed files. Do not copy
code from AGPL/BSL projects (Outline, Lore, Docmost, Wiki.js, Drafft.ink); reading them for ideas is fine.

- A change to `Cargo.lock` or `web/pnpm-lock.yaml` also updates the tables in `THIRD_PARTY.md` (its last section says
  how), and every new licence is checked against PLAN rule R6. The image's `/licenses.txt` is generated by the
  `licenses` stage of `docker/Dockerfile` and needs no edit.
- Some questions are the owner's and stay open until the owner answers (docs/ARCHITECTURE.md §16 item 25: the
  engine's fonts and the base image's OS layer): keep the current state, and do not decide them. Decided on
  2026-10-28: MPL-2.0 is allowed for build-only tools (lightningcss), the image digests stay as pinned, and on the
  owner's machine Wikanvas's Caddy serves `127.0.0.1:8443` (a local `.env`, ignored by git). Resolved for
  v0.1.0-alpha.1: the single `LICENSE` reads "Copyright (c) 2026 The Wikanvas contributors".
