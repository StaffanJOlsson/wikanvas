# Wikanvas milestone 1: implementation plan

Read `docs/ARCHITECTURE.md` first; this plan follows it and cites it as "§n". The work is split into workstreams for
parallel agents:

- **W0a** lands first, alone, as its own commit: the build and runtime caps the owner asked for "asap", the build slice
  (installed by the coordinator), and the compose file with its Caddy HTTPS front.
- **W0b** lands second, alone: scaffold, contracts (including the public API's DTO crate, OpenAPI plumbing, token
  auth and the few minimal routes the CLI tests need), Docker image, test harnesses.
- **W1–W6 and W8** then run in parallel: seven workstreams. W8 (the `wikanvas` CLI and the agent harness) is numbered
  after W7 only because it was added later. Their dependency edges are listed under "Overview"; every edge is satisfied
  by a minimal working version from W0b, so nobody waits on a `todo!()`.
- **W7** integrates, owns the cross-workstream tests, and runs the end-to-end gate, including the full CLI suite and the
  agent harness.

Later milestones follow M1's sections: "M1.1: board document embeds" (ARCHITECTURE §18, workstreams E1–E3) and "M2: the
egui client" (ARCHITECTURE §19, D101–D143: the spike S0, the foundations F0, the port C1–C9, the integration I and the
cutover X), a proposal until the owner accepts §19.

---

## Rules for every workstream

- **R1. Builds and caps.**
  - Run every cargo, rustc, rustfmt, wasm-bindgen, wasm-opt, trunk, node, pnpm and npx command as
    `scripts/dev.sh <cmd>`. Format only with `scripts/fmt.sh` (never `cargo fmt --all`: it reaches into
    `vendor/inkboard`). Build images only with `scripts/image.sh` or `scripts/toolchain.sh`, which use the
    `wikanvas-limited` builder. Run Playwright only through `scripts/e2e.sh`, the agent harness only through
    `scripts/agent-test.sh`, and build the standalone CLI only through `scripts/cli-dist.sh`.
  - These scripts take one per-user lock (§13.1), so builds queue instead of overlapping. Nested calls from a script
    that holds the lock skip it automatically (`WIKANVAS_LOCK_HELD`); never set that variable by hand.
  - Never run `cargo`, `pnpm` or `node` on the host. Never raise the caps (`WIKANVAS_BUILD_CPUS`, `WIKANVAS_BUILD_MEM`,
    `WIKANVAS_BUILD_CPUSET` may only be lowered; the runtime caps `WIKANVAS_CPUS`, `WIKANVAS_MEM`, `WIKANVAS_CADDY_CPUS`
    and `WIKANVAS_CADDY_MEM` change only when the owner asks). Git runs on the host.
  - Never run `sudo` or any other root step. A privileged step is prepared as a script and named in your handoff note;
    the coordinator runs it through the owner's askpass dialog (the only one in M1 is
    `scripts/install-build-slice.sh`, ARCHITECTURE §13.1). Every script must also work, with the per-container caps and
    a warning, while the slice is absent.
  - Do not keep dev servers running. If you must look at the UI, stop the server within 15 minutes, because it holds
    the lock. `scripts/agent-test.sh up` enforces this with its TTL (ARCHITECTURE §14.1).
- **R2. File ownership.** You edit only the files your workstream owns (the ownership tables below). If you need a change
  in a file someone else owns, write it as a concrete request in your handoff note `docs/handoff/Wn.md`, which only you
  own. The orchestrator relays it.
- **R3. Contracts.**
  - The signatures W0b creates (listed under W0b "Produces") are frozen.
  - Additive changes (a new optional field, a new function) are allowed by the owner of the file. The owner records
    each one in its handoff note.
  - A breaking change needs the orchestrator's approval.
- **R4. Generated contracts.**
  - Never hand-edit `web/src/api/gen/*` or `docs/api/openapi.json`.
  - If you change a DTO you own (in `crates/wikanvas-api`), run `scripts/dev.sh cargo test -p wikanvas-api --locked
    export_bindings`, then commit the regenerated files for your own types.
  - If you add or change a route, a DTO or an annotation, run `scripts/dev.sh env UPDATE_OPENAPI=1 cargo test -p
    wikanvas-server --locked --test api -- openapi::` and commit `docs/api/openapi.json` with the change. The file is
    one shared JSON document but deterministic (ARCHITECTURE §7.4), so on a merge conflict the orchestrator takes either
    side and re-runs that command, as for lockfiles (R6). Every route is registered through `routes!` on a
    `#[utoipa::path]` handler whose `security` matches its `Auth<S>` extractor (D44); never `Router::route` under
    `crates/wikanvas-server/src/routes/`.
- **R5. Schema.** Only W1 edits `crates/wikanvas-store/migrations/0001-init/up.sql`. If you need a schema change, put
  the exact DDL in your handoff note.
- **R6. Dependencies and lockfiles.**
  - W0b declares every known dependency (§4). A new crate goes into a crate's `Cargo.toml` through that file's owner
    (W1: `wikanvas-api`, `wikanvas-core`, `wikanvas-store`; W2: `wikanvas-server`, `wikanvas-assets`; W8:
    `wikanvas-cli`). A new npm package goes into `web/package.json` through W3.
  - Whoever changes a `Cargo.toml` commits the regenerated `Cargo.lock` in the same commit
    (`scripts/dev.sh cargo update --workspace`, which adds the new entries and keeps other pins), then runs
    `scripts/check-pins.sh`. The same holds for `package.json` and `pnpm-lock.yaml`.
  - The orchestrator merges branches that touch a lockfile one at a time. On a lockfile conflict it takes the target
    branch's lockfile and re-runs the update command.
  - Licences: MIT, Apache-2.0, BSD, ISC, Zlib, Unicode or CC0 only. Nothing from AGPL or BSL projects; nothing from
    TipTap Pro or Cloud. webpki-roots (CDLA-Permissive-2.0) is not allowed, so never enable ureq's `rustls` feature
    (D54). Build-only tools may also be MPL-2.0: a package that runs only during a build, with none of its code in
    `web/dist`, a binary or the image (owner, 2026-10-28, for lightningcss through Vite 8; `THIRD_PARTY.md` lists it).
- **R7. Tests.** Rust integration tests share one binary per crate: `crates/wikanvas-store/tests/store/main.rs` and
  `crates/wikanvas-server/tests/api/main.rs` (both W0b, frozen, owned by W7 afterwards), and
  `crates/wikanvas-cli/tests/cli/main.rs` (W0b, then W8 with the whole crate). Each workstream owns its own module file
  inside them. Tests that need another workstream's full behaviour live in `tests/api/integration.rs` (W7) or, for the
  CLI, in the `flows` modules of `tests/cli` (written by W8, gated by W7). Web
  tests sit next to the code they test (`*.test.ts(x)`). Unit tests inside a source file live in `mod tests`, so their
  names read `rank::tests::…`.
- **R8. Inkboard.** Never modify `vendor/inkboard` and never build inside it; Inkboard builds run from
  `.cache/inkboard-src` (§10.5). `git -C vendor/inkboard status --porcelain` must stay empty. Inkboard changes exist only
  as `patches/inkboard/*.patch`, which W5 owns.
- **R9. Provenance.** Code borrowed from GetPaper (MIT) gets a header comment and a line in your handoff note, which W7
  moves into `THIRD_PARTY.md`. Design values come from §9.6, never from Outline's source.
- **R10. Done** means all of these are true:
  - your acceptance commands pass;
  - `scripts/dev.sh cargo clippy --workspace --all-targets --locked -- -D warnings` and `scripts/fmt.sh --check` pass
    (Rust workstreams);
  - `scripts/dev.sh pnpm --dir web typecheck` passes (web workstreams);
  - `grep -rnE 'todo!\(|unimplemented!\(|TODO\(W' <your owned paths>` finds nothing;
  - your handoff note lists deviations, requests and follow-ups.
- **R11. Git and worktrees.**
  - Work on the branch `ws/Wn-<name>` in its own worktree, created on the host:
    `git worktree add ../Wikanvas-Wn -b ws/Wn-<name>`, then
    `git -C ../Wikanvas-Wn submodule update --init --reference "$PWD/vendor/inkboard" vendor/inkboard`.
  - Nothing inside a build container runs git against the main repository (a worktree's `.git` points outside the
    mount); `scripts/inkboard.sh` uses its own self-contained repo in `.cache/inkboard-src`.
  - Set `WIKANVAS_SHARED_CARGO_HOME=$HOME/.cache/wikanvas-cargo` so the worktrees share the cargo registry.
  - Commit only your owned files. The orchestrator merges the branches in the order they finish (lockfile branches
    serialized, R6); W7 merges last.

---

## Overview

| Id | Workstream | Depends on | Runs |
|---|---|---|---|
| W0a | Build and runtime caps, the build slice, compose with the Caddy front | none (the coordinator runs the slice install) | first, alone, own commit |
| W0b | Scaffold, contracts, Docker image, harnesses, minimal `me`/token/collection-create routes | W0a | second, alone |
| W1 | Tree, collections, items, lists, search (backend), cursor pagination, and schema owner | W0b | parallel |
| W2 | Platform: auth, setup, security (including the trusted-proxy rule, cookies and HSTS behind Caddy), API tokens, OpenAPI assembly, CORS and rate limits, static serving, files, jobs, backup, `wikanvas-server` commands (backend) | W0b | parallel |
| W3 | Web shell: routing, sidebar tree and drag and drop, top bar, list pages, settings (including API tokens) | W0b | parallel |
| W4 | Documents, full stack: Yjs storage, the TipTap editor, and Markdown / ProseMirror JSON conversion | W0b | parallel |
| W5 | Inkboard engine: patches, wasm build, frame page | W0b | parallel |
| W6 | Boards, full stack minus the engine: storage, previews, board host and page, board API rules | W0b (W5 for the real engine; fake frame until then) | parallel |
| W8 | The `wikanvas` CLI, its tests, the agent harness, `docs/AGENTS.md` | W0b (its minimal routes for the parallel-phase tests; W1, W2, W4, W6 routes for the `flows` tests and scenarios 02–11, gated by W7) | parallel |
| W7 | Integration, cross-workstream tests, production image, end-to-end smoke, full CLI suite and agent harness, docs | W1–W6, W8 | last |

```
W0a ── W0b ──┬─ W1 ─┐
             ├─ W2 ─┤
             ├─ W3 ─┤
             ├─ W4 ─┤
             ├─ W5 ─┼─ W7
             ├─ W6 ─┤   W6 ⇠ W5 (bridge.ts contract from W0b; real dist when W5 lands)
             └─ W8 ─┘   W8 ⇠ W1, W2, W4, W6 (wikanvas-api DTOs and TestServer from W0b; full routes on merge)
```

Declared edges during the parallel phase (each consumer builds against the W0b minimal version, and the full version
arrives on merge):

| Consumer | Needs | From | W0b provides |
|---|---|---|---|
| W2 seed, W2 setup test | `collections::create`, `items::create` | W1 | working minimal versions |
| W2 purge job | `items::purge_expired`, `revisions::prune` | W1 | working minimal / no-op returning 0 |
| W2 jobs | `board_previews::spawn` | W6 | no-op queue |
| W1 `migrate` | `backup::pre_migration` | W2 | working minimal (`VACUUM INTO` on the maintenance connection) |
| W1, W2 create/duplicate/discard/seed | `docs::*`, `boards::*` init, copy, is_empty | W4, W6 | working minimal versions |
| W4, W6 pages | `TopBar`, `ItemHeader`, `ItemMenu`, `saveStatus`, `pending`, `useItem`, `useCreateItem`, `urls` | W3 | typed, minimal working |
| W3 `AppShell` | `<BoardFrameHost/>` mounted once | W6 | placeholder that renders null, already mounted |
| W6 | `bridge.ts` types, then the real dist | W5 | full types; `stub` dist |
| Every route owner | `OpenApiRouter` registration, `Auth<S>`, `ErrorBody`, `Page<T>`, `wikanvas_core::cursor` | W0b, then W1/W2 | working plumbing; the spec assembled from every module's router |
| W8 CLI tests (`config`, `auth`, `tokens`, `admin`, `coverage`) | Bearer auth with scopes; `GET /auth/me`; `GET`/`POST /tokens`, `DELETE /tokens/{id}`, `DELETE /tokens/current`; `wikanvas_server::testing::TestServer` | W2 | working minimal routes and store functions (`is_wikanvas_bearer`, lookup with classification, scope check, create with the caps, list, revoke with children; one token per scope in `TestServer`) |
| W8 agent harness, scenario 01 | `wikanvas-server admin create-owner` and `admin create-token`; `POST /collections`; both binaries in the image | W2, W1, W0b | working minimal commands with their final flags; a minimal create route; the Dockerfile `cli` stages |
| W4 `?title=from_h1`; W4, W6 recovery points | `items::rename`, `revisions::pre_api`, `WriteBy` | W1 | working minimal versions |
| W8 `flows` tests and scenarios 02–11 | Collections, items, tree, search and lists; documents in Markdown/JSON; boards; attachments and export | W1, W4, W6, W2 | DTOs frozen in `wikanvas-api`; the rest runs at W7 |
| W3 Settings → API tokens | `/api/v1/tokens` | W2 | DTOs; mocked in tests |
| W4 web "Export Markdown" | `GET …/doc/markdown?download=true` | W4 (server half) | nothing: the same workstream; mocked in web tests until the route exists |

Lock contention: W5's release wasm build (10–20 min) and the image builds are the long jobs; a full agent-harness run
holds the lock for a few minutes. The lock queues them. Keep test runs filtered to your own modules.

---

## W0a: build and runtime caps (lands first)

**Goal.** Kernel-enforced limits on every build and on the hosted app, committed on their own before any scaffold work,
because the owner asked for them as soon as possible (§13.1, D38, §13.3). That includes the total cap
(`wikanvas-build.slice`, which the owner approved on 2026-10-28) and the compose project with its Caddy HTTPS front and
caps (D57, §13.4).

### Tasks, in order

1. **Wait for running builds.** Check `docker ps` and `docker buildx ls`; the toolchain image may still be building.
   Do not start anything until it is done.
2. **Lock.** Add `scripts/lib/lock.sh` (§13.1): per-user lock file from `getent passwd`, "waiting for another Wikanvas
   build…", exports `WIKANVAS_LOCK_HELD=1`, skips when it is already set. Source it from `scripts/dev.sh` and
   `scripts/toolchain.sh`.
3. **Slice files.** Add `docker/wikanvas-build.slice`, `docker/wikanvas.slice`, `scripts/install-build-slice.sh`
   (install as root, never calling `sudo` itself; `--check` without root; exit codes of §13.1) and
   `scripts/lib/slice.sh` (`wikanvas_slice`: `WIKANVAS_CGROUP_PARENT` only under Docker's systemd cgroup driver with the
   unit file installed, tested through `FragmentPath`, never `LoadState`, which reads `loaded` for any slice name;
   otherwise the warning; `WIKANVAS_BUILD_SLICE` for the warning test). Do not run the installer and never run `sudo`:
   write in your handoff note that the slice is ready, and the coordinator runs, once, through the owner's askpass
   dialog: `SUDO_ASKPASS=/home/staffan/.local/bin/claude-askpass sudo -A -p "install the Wikanvas build cap (systemd
   slice: 6 CPUs, 12 GB RAM, no swap, low priority) :: $PWD/scripts/install-build-slice.sh"
   "$PWD/scripts/install-build-slice.sh"`. Continue with tasks 4–7 meanwhile: everything must work without the slice.
4. **`scripts/dev.sh`** (§13.1): keep `--cpus`, `--memory`, `--memory-swap`, `--pids-limit 4096` and `nice -n 10`;
   add `--cpuset-cpus` (last N cores), `--shm-size 1g`, `wikanvas_slice` and `--cgroup-parent "$WIKANVAS_CGROUP_PARENT"`
   when set, the thread-count environment variables, `WIKANVAS_DEV_IMAGE`, `WIKANVAS_DEV_NETWORK` and
   `WIKANVAS_SHARED_CARGO_HOME`.
5. **Builder.** Add `docker/buildkitd.toml` (`max-parallelism = 2`, GC `maxUsedSpace = "30GB"`). In
   `scripts/toolchain.sh`: create the BuildKit container `wikanvas-buildkit` from `moby/buildkit:v0.34.0` pinned by
   digest (`docker buildx imagetools inspect`; v0.34.1, released 2026-10-20, was under two weeks old on 2026-10-28, so
   W0a took the newest older patch, ARCHITECTURE §16 risk 23), with the caps, cpuset, cgroup parent (from
   `wikanvas_slice`), state volume and `docker cp` of the config of §13.1, and register it as `wikanvas-limited` with
   the `remote` driver (`docker-container://wikanvas-buildkit`): buildx's docker-container driver ignores
   `cgroup-parent` under the systemd cgroup driver. If the existing `wikanvas-limited` uses another driver (it does
   today), or `wikanvas-buildkit` has other caps or another cgroup parent than `wikanvas_slice` gives, print the `docker
   buildx rm wikanvas-limited; docker rm -f wikanvas-buildkit;
   scripts/toolchain.sh` instruction and exit 1. It keeps building `docker/toolchain.Dockerfile` until W0b replaces it.
6. **Runtime caps and HTTPS front.** Write `compose.yaml` exactly as in §13.3 (the app at 2 CPUs / 1 GiB / no swap,
   no published port; Caddy at 0.5 CPU / 128 MiB with its fixed address outside the network's `ip_range`, ports and
   volumes) and `docker/Caddyfile` exactly as in §13.4. Pin `caddy:2.11.7` by digest (`docker buildx imagetools inspect caddy:2.11.7`; if Docker Hub
   has no 2.11.7 yet, take the newest 2.11 patch older than two weeks and record it in ARCHITECTURE §16). Check that
   Caddy starts and binds 80 and 443 with `cap_drop: ALL`, `cap_add: [NET_BIND_SERVICE]`, `no-new-privileges` and a
   read-only rootfs (§16, risk 22): the app image does not exist yet, so check it with a `docker run` of the
   pinned image with the same options and the Caddyfile, publishing nothing (the acceptance block below).
7. **Stale artefacts.** `rm -rf vendor/inkboard/target vendor/inkboard/crates/inkboard-app/web/dist` (977 MB of
   git-ignored output from an earlier in-place build; R8 stays clean).
8. **After the coordinator installed the slice:** `scripts/install-build-slice.sh --check`, then recreate the BuildKit
   container as `toolchain.sh` instructs, so it joins the slice.
9. **CLAUDE.md.** Add: the lock and `WIKANVAS_LOCK_HELD`; the cpuset and the slice (`scripts/install-build-slice.sh
   --check`; builds still run, with a warning, without it); "never run `sudo`: the coordinator runs the one privileged
   step through the askpass dialog"; "never build inside `vendor/inkboard`"; "format only with `scripts/fmt.sh`, never
   `cargo fmt --all`"; and that the hosted app runs behind Caddy, with both containers' limits in `compose.yaml`
   (app 2 CPUs / 1 GiB, Caddy 0.5 CPU / 128 MiB).
10. Commit W0a on its own (this is the repository's first commit, together with the already staged submodule and
    `docs/`).

### Owns

During W0a: `scripts/lib/lock.sh`, `scripts/lib/slice.sh`, `scripts/dev.sh`, `scripts/toolchain.sh`,
`scripts/install-build-slice.sh`, `docker/buildkitd.toml`, `docker/wikanvas-build.slice`, `docker/wikanvas.slice`,
`docker/Caddyfile`, `compose.yaml`, `CLAUDE.md`. Ownership then passes to W0b, and to W7 after W0b.

### Acceptance

```sh
for f in scripts/*.sh scripts/lib/*.sh; do bash -n "$f" || exit 1; done   # bash -n a b checks only a
( scripts/dev.sh sleep 20 & sleep 2; scripts/dev.sh true 2>&1 | grep -q 'waiting for another Wikanvas build' ); rc=$?; wait; test $rc -eq 0
bash -c '. scripts/lib/lock.sh && wikanvas_lock && timeout 120 scripts/dev.sh true'    # nested call does not deadlock
WIKANVAS_BUILD_SLICE=wikanvas-missing-test.slice scripts/dev.sh true 2>&1 | grep -q 'not installed: per-container caps only'
scripts/dev.sh nproc | grep -qx 6
scripts/dev.sh cat /sys/fs/cgroup/cpu.max | grep -qx '600000 100000'
scripts/dev.sh cat /sys/fs/cgroup/memory.max | grep -qx 12884901888
scripts/dev.sh cat /sys/fs/cgroup/memory.swap.max | grep -qx 0
docker buildx inspect wikanvas-limited | grep -qE 'Driver: +remote'
docker inspect -f '{{.HostConfig.NanoCpus}} {{.HostConfig.Memory}} {{.HostConfig.MemorySwap}}' wikanvas-buildkit \
  | grep -qx '6000000000 12884901888 12884901888'
docker inspect -f '{{.HostConfig.CpusetCpus}}' wikanvas-buildkit | grep -q .
docker exec wikanvas-buildkit cat /etc/buildkit/buildkitd.toml | grep -q 'max-parallelism = 2'
docker compose -f compose.yaml config -q && ! grep -q '^\s*build:' compose.yaml
docker compose --env-file /dev/null -f compose.yaml config --format json | jq -e '(.services.wikanvas.ports // []) == []
  and .services.wikanvas.cpus == 2 and .services.caddy.cpus == 0.5 and (.services.caddy.ports | length) == 2
  and .networks.default.ipam.config[0].ip_range == "172.31.240.8/29"
  and .services.caddy.networks.default.ipv4_address == "172.31.240.2"'
! grep -qE '^\s*(encode|header)\b' docker/Caddyfile
img=$(docker compose --env-file /dev/null -f compose.yaml config --format json | jq -r .services.caddy.image) && for m in 'localhost internal' \
  'wiki.example.com acme'; do set -- $m; docker run --rm -e WIKANVAS_DOMAIN=$1 -e WIKANVAS_TLS=$2 \
  -v "$PWD/docker/Caddyfile:/etc/caddy/Caddyfile:ro" "$img" caddy validate --config /etc/caddy/Caddyfile \
  --adapter caddyfile || exit 1; done
# Caddy starts and binds 80/443 with the compose hardening (task 6, risk 22); nothing published, bounded wait
docker run -d --name wikanvas-caddy-check --read-only --tmpfs /tmp --tmpfs /data --tmpfs /config --cap-drop ALL \
  --cap-add NET_BIND_SERVICE --security-opt no-new-privileges:true -e WIKANVAS_DOMAIN=localhost -e WIKANVAS_TLS=internal \
  -v "$PWD/docker/Caddyfile:/etc/caddy/Caddyfile:ro" "$img" >/dev/null \
  && for i in $(seq 20); do docker logs wikanvas-caddy-check 2>&1 | grep -q 'serving initial configuration' && break; sleep 0.5; done; \
  docker logs wikanvas-caddy-check 2>&1 | grep -q 'serving initial configuration'; rc=$?; docker rm -f wikanvas-caddy-check >/dev/null; test $rc -eq 0
test ! -e vendor/inkboard/target
git -C vendor/inkboard status --porcelain | wc -l | grep -qx 0
# after the coordinator installed the slice (task 8):
scripts/install-build-slice.sh --check
( scripts/dev.sh sleep 15 & sleep 5; systemctl status wikanvas-build.slice --no-pager | grep -q 'docker-init -- nice -n 10 sleep 15' ); rc=$?; wait; test $rc -eq 0
docker inspect -f '{{.HostConfig.CgroupParent}}' wikanvas-buildkit | grep -qx wikanvas-build.slice
```

---

## W0b: scaffold, contracts, Docker image (lands second)

**Goal.** Make every later workstream able to build, test and run in parallel without touching shared files.

### Tasks, in order

1. **Docker and scripts.**
   - Write `docker/Dockerfile` exactly as §13.2, filling in the three image digests (`docker buildx imagetools
     inspect`) and the binaryen `version_131` checksum (from the release's `.sha256` file).
   - Delete `docker/toolchain.Dockerfile`; point `scripts/toolchain.sh` at `--target toolchain -f docker/Dockerfile`
     with the repo root as context. Rebuild the toolchain (it now has clippy, rustfmt, binaryen and brotli).
   - Write `.dockerignore`, `compose.e2e.yaml` and `compose.smoke.yaml` exactly as in §13.2 and §13.3.
   - Add `scripts/image.sh` (§13.2, under the lock, with the `org.opencontainers.image.revision` label),
     `scripts/image-smoke.sh` (under the lock, `wikanvas_slice`, project `wikanvas-smoke` on `172.31.240.48/28` with
     range `172.31.240.56/29` and Caddy at `.50`, `--env-file /dev/null` and the owner-facing variables unset (§13.3),
     `compose.yaml` + `compose.smoke.yaml`, `WIKANVAS_DOMAIN=localhost`, `WIKANVAS_TLS=internal`,
     `WIKANVAS_HTTPS_PORT=18443`; `WIKANVAS_SMOKE_LEVEL=basic|full`; `basic` reads Caddy's root from its tmpfs with
     `docker compose exec -T caddy cat /data/caddy/pki/authorities/local/root.crt` (`docker cp` cannot read a tmpfs) and
     checks that `curl --cacert <root> --resolve localhost:18443:127.0.0.1 https://localhost:18443/healthz` answers 200
     (and fails without `--cacert`), that the `wikanvas` container has no published port, that `docker inspect` shows
     `NanoCpus=2000000000`, `Memory=1073741824` and `MemorySwap=1073741824` for `wikanvas` and `NanoCpus=500000000`,
     `Memory=134217728` and `MemorySwap=134217728` for `caddy`, and that `wikanvas-server --version` and
     `wikanvas --version` run inside the image; `full` adds the §14 checks), a skeleton `scripts/e2e.sh frame|smoke`
     (§14; the smoke project starts only `wikanvas`), `scripts/fmt.sh` (§4.1), and `scripts/check-pins.sh` (host only,
     text and git: submodule at `41e0295` and clean; serde, serde_json, base64 and thiserror at equal versions in
     `Cargo.lock` and `vendor/inkboard/Cargo.lock`; no `{` in `.dockerignore`; `rust-toolchain.toml` channel equal to
     the `RUST_IMAGE` tag).
2. **Rust workspace.**
   - Write `Cargo.toml` (§4.1) with every crate's dependencies declared, `Cargo.lock` (align the shared crates with
     Inkboard's lock using `cargo update -p <crate> --precise <version>`), `rust-toolchain.toml` (§3) and
     `.cargo/config.toml` (keep `jobs = 6`; add `[env] TS_RS_EXPORT_DIR = { value = "web/src/api/gen", relative = true }`).
   - Confirm a single `libsqlite3-sys`; if deadpool-sqlite or rusqlite_migration do not fit rusqlite 0.40, apply the D2
     fallback and record it in §16.
   - Create the six crates (`wikanvas-api`, `wikanvas-core`, `wikanvas-store`, `wikanvas-server`, `wikanvas-assets`,
     `wikanvas-cli`) with **every module file listed in the ownership tables**: empty, stubbed, or minimally implemented
     as stated below. `wikanvas-server` has `src/lib.rs` (module list and `pub fn app(state) -> Router`) and a thin
     `src/main.rs`, and builds the binary `wikanvas-server`; `wikanvas-server` has a `testing` feature; `wikanvas-cli`
     builds the binary `wikanvas`. `wikanvas-core` re-exports `pub use wikanvas_api as api;` (D40).
   - Write `migrations/0001-init/up.sql`: the §5 DDL verbatim (including `token_scopes`, `api_tokens` with
     `parent_token_id`, `api_token_scopes`, `last_token_id` on `doc_state` and `board_state`, `revisions.source_seq`,
     and the `pre-api` revision reason).
3. **DTOs** (`crates/wikanvas-api`).
   - Write all DTOs from §7.1–§7.3 and §7.5, with `#[derive(TS, ToSchema)] #[ts(export)]`.
   - Write `ErrorCode` (every §7.1 code with its HTTP status), `ErrorBody`, `Page<T>`, `Scope` (with the hierarchy),
     `API_VERSION = "1.0"` (frozen until the first release, §7.1), `cli::ExitCode` (the §7.7 table, with
     `from_status(status, code)`: unlisted 4xx → 10, unlisted 5xx and 408 → 12, 3xx → 3), and `token::{encode, check}`
     (D42).
   - Generate `web/src/api/gen/*.ts` and commit them.
4. **Server skeleton.**
   - `wikanvas-server serve` (config from env, §13.3: `config.rs` parses every variable, including `WIKANVAS_DOMAIN`,
     `WIKANVAS_TLS`, `WIKANVAS_HTTPS_PORT` and `WIKANVAS_TRUSTED_PROXIES`, and derives the public URL of §8; W2 adds the
     behaviour behind them) and `wikanvas-server healthcheck [--url URL]` (default URL from `WIKANVAS_LISTEN`'s port on
     127.0.0.1; exit 0 if healthy, 1 otherwise).
   - `wikanvas-server admin create-owner` and `admin create-token` with their final §7.8 flags, `--json` and the §7.7
     exit codes, minimally implemented (owner row + settings; token row + scopes). The other admin subcommands exist and
     exit 1 with "not implemented (W2)" until W2 replaces them.
   - `GET /healthz`. Everything else under `/api/` that is not a `/api/v1` route answers the JSON 404.
   - Every `routes::<x>::router()` returns an `OpenApiRouter<AppState>`, empty except for these minimal working routes
     (final paths, DTOs, `#[utoipa::path]` security and `Auth<S>`), which their owners complete: `GET /auth/me`,
     `GET`/`POST /tokens`, `DELETE /tokens/{id}` and `DELETE /tokens/current` (W2), and `POST /collections` (W1).
     `openapi.rs` nests them under `/api/v1`, calls `split_for_parts()`, adds the security schemes and `info` (§7.4),
     and serves `GET /api/v1/openapi.json`; `docs/api/openapi.json` is generated and committed, and
     `tests/api/openapi.rs` has `spec_matches_committed` and `spec_parses_as_3_1`.
   - `app.rs` merges them; `state.rs`, `error.rs` (axum rejections mapped to `ErrorBody`, `X-Request-Id`,
     `Wikanvas-Api-Version`) and `extract.rs` are in place. `extract.rs` has `Auth<S>` with both branches of §7.3
     working (`is_wikanvas_bearer`, the cookie, and bearer lookup through `tokens::lookup` with classification, the
     scope check and `session_required`), plus `Viewer` and `ItemRef`. The owner-exists cache re-checks while false
     (§8).
   - `crates/wikanvas-cli/src/main.rs`: clap skeleton with the global flags of §7.6, `wikanvas --version`, and
     `wikanvas exit-codes [--json]` printing `wikanvas_api::cli::ExitCode`.
5. **Minimal working shared code** (so that no parallel workstream is blocked by a `todo!()`):
   - store: `Db` (writer, readers, `maintenance()`, PRAGMAs, `write` and `read`); `migrate` (calls
     `backup::pre_migration`); `backup::pre_migration`; `shared::*` including `visible_items` and
     `effectively_published`; `revisions::maybe_auto`; `revisions::prune` (no-op, returns 0); `blobs::BlobStore`
     (`stage`, `commit`, `put`, `read`, `path`);
   - store: `docs::{init_empty,init_with,copy,is_empty}`, where an empty doc is `Y.encodeStateAsUpdate(new Doc)`
     (`[0,0]`) and `is_empty` follows §6.1; `boards::{init_empty,init_with,copy,is_empty}`, where an empty board is
     `serial::to_json(&Document::new(""), false)` with the background chosen by `dark`;
   - store: `collections::create`, `items::create` (sets `published_at = created_at`, D23, and calls the docs/boards
     initializers), `items::rename` (bumps
     `meta_version` and `tree_version`), `items::purge_expired` (leaves first), `revisions::pre_api` (the §5 rule);
   - server: `board_previews::{PreviewQueue, spawn}` with a no-op `enqueue` (file
     `wikanvas-server/src/board_previews.rs`);
   - store: `tokens::{create, lookup, list, revoke}` (generate with getrandom, `wikanvas_api::token::encode`, store
     sha256, prefix, expanded scopes and `parent_token_id`; look up by hash and classify; revoke with children);
   - core: `cursor::{encode, decode}` complete (D46): base64url, keyset or offset, 8-byte hash of the normalized
     query.
6. **Test harnesses.**
   - `crates/wikanvas-store/tests/store/{main,common}.rs`: a temp-file DB with migrations applied, opened through `Db`.
   - `crates/wikanvas-server/tests/api/{main,common,integration}.rs`: `TestApp::new()` (`wikanvas_server::app()`, temp
     data dir, cheap argon2), `TestApp::with_owner()` (inserts the owner and a session with raw SQL; returns a client
     with the cookie, an allowed `Host` and `Sec-Fetch-Site: same-origin`), and `TestApp::with_token(scope)` (a client
     with `Authorization: Bearer` and no cookie).
   - `crates/wikanvas-server/src/testing.rs` (feature `testing`): `TestServer::start() -> TestServer{url, read_token,
     write_token, admin_token, data_dir}`, which binds `127.0.0.1:0`, runs `app()` on its own tokio runtime thread with
     a temp data dir and cheap argon2, creates the owner, and stops on drop.
   - `crates/wikanvas-cli/tests/cli/{main,common}.rs`: `common` starts a `TestServer` and builds an assert_cmd `Command`
     for the `wikanvas` binary with `WIKANVAS_URL`, `WIKANVAS_TOKEN`, `WIKANVAS_CONFIG` and `HOME` pointing into a temp
     dir.
   - One test per crate that passes (for the CLI: `wikanvas --version`).
7. **Web skeleton.**
   - `web/package.json` with every §4.2 dependency at exact versions, plus the scripts `dev`, `build`, `test`,
     `typecheck` and `fixtures`; `vitest` config with `maxWorkers: 4`.
   - `pnpm-lock.yaml`, `vite.config.ts` (proxy `/api` and `/inkboard` to `127.0.0.1:8080`), `tsconfig.json`,
     `web/inkboard-frame/tsconfig.json` (`allowJs`, `checkJs`), `web/inkboard-frame/inkboard.d.ts`, `index.html` (no
     inline script), `public/theme-init.js`.
   - `src/main.tsx` and `src/app/router.tsx` with every §9.1 route, pointing at placeholder pages that include
     `pages/DocPage.tsx` and `pages/BoardPage.tsx`.
   - `src/app/AppShell.tsx` mounting `<BoardFrameHost/>` from `src/board/BoardFrameHost.tsx` (a placeholder that renders
     null; `useBoardFrame()` returns null).
   - Contract modules with their final exported types and minimal working bodies:
     - `src/api/client.ts` (`API_BASE = '/api/v1'`, §9.2);
     - `src/api/queries/items.ts`: `useItem(ref)`, `useCreateItem()`, `useConfig()`;
     - `src/state/saveStatus.ts` and `src/state/pending.ts`;
     - `src/components/topbar/TopBar.tsx`, `src/components/item/ItemHeader.tsx` and `src/components/item/ItemMenu.tsx`
       (props as in §9.3);
     - `src/app/hotkeys.ts`: `runHostShortcut(e)`;
     - `src/lib/urls.ts`: `itemPath`, `collectionPath`, `boardPreviewUrl(id, rev, page)`, `attachmentUrl(id)`,
       `inkboardBase(config)`;
     - `src/test/{setup.ts,render.tsx}`.
   - `src/board/bridge.ts`: the full §10.4 types.
   - `web/inkboard-frame/{frame.html,frame.js,frame.css}`: a placeholder whose `wikanvasBoardFrame.connect` rejects
     `{kind:'unsupported'}`, and which passes typecheck.
8. **Inkboard and e2e scaffolding.**
   - `scripts/inkboard.sh` with a working `stub` (manifest per §10.5, `format_version` parsed from `serial.rs`); the
     other subcommands print "not implemented (W5)".
   - `patches/inkboard/README.md`.
   - `e2e/package.json` (@playwright/test 1.64.1) and its lockfile.
   - `e2e/playwright.config.ts` with `workers: 2`, projects `frame` (`baseURL` `http://127.0.0.1:4173`) and `smoke`
     (`baseURL` `http://wikanvas:8080`: Wikanvas directly on the project network, plain HTTP; Caddy is not part of e2e runs), and Chromium launch args `--use-angle=swiftshader --enable-unsafe-swiftshader`.
     It also declares `webServer: node e2e/serve-static.mjs web/dist-inkboard 4173`.
   - A minimal `e2e/serve-static.mjs`, owned by W5 afterwards. It must serve `.wasm` as `application/wasm`.
   - Placeholder specs `e2e/tests/frame.spec.ts` and `e2e/tests/smoke.spec.ts`.
9. **Placeholders, licences and docs.**
   - Fixture placeholders: `fixtures/ydoc/getting-started.bin` (`[0,0]`) and
     `fixtures/boards/getting-started.inkboard` (an empty `Document`).
   - `LICENSE`, `THIRD_PARTY.md`, a stub `README.md`.
   - `.gitignore` additions (§3).
   - In `CLAUDE.md`: the `docker/Dockerfile` toolchain target in place of `docker/toolchain.Dockerfile`, and one line:
     the agent harness runs only through `scripts/agent-test.sh`, and agents use the `wikanvas` CLI as described in
     `docs/AGENTS.md`.
   - In `docs/ARCHITECTURE.md` §16: any pin W0b could not confirm and the fallback chosen.

### Owns

During W0b, W0b owns everything it creates plus the W0a files. When W0b ends, ownership passes as listed in the W1–W8
tables.

### Produces (frozen contracts)

- **Rust store functions** (`tx: &rusqlite::Transaction`, `now: i64` ms; errors are `wikanvas_store::Error`, which wraps
  `ErrorCode`):

  ```rust
  // wikanvas-store::db
  impl Db {                                       // async: deadpool-sqlite's pools; the closure itself is synchronous
    pub async fn write<T, F>(&self, f: F) -> Result<T>   // BEGIN IMMEDIATE on the single writer
      where F: FnOnce(&rusqlite::Transaction<'_>) -> Result<T> + Send + 'static, T: Send + 'static;
    pub async fn read<T, F>(&self, f: F) -> Result<T>    // one deferred transaction on a query_only reader
      where F: FnOnce(&rusqlite::Connection) -> Result<T> + Send + 'static, T: Send + 'static;
    pub fn maintenance(&self) -> Result<rusqlite::Connection>; }                  // no query_only; VACUUM INTO only
  // wikanvas-store::shared
  pub fn touch_content(tx, item_id: &str, actor_id: &str, now) -> Result<()>;         // content_at, updated_at, updated_by
  pub fn set_body_text(tx, item_id: &str, body: &str) -> Result<()>;                   // item_text.body (no-op if equal)
  pub fn ensure_writable(tx, item_id: &str, kind: ItemKind) -> Result<ItemRow>;        // not_found / validation (wrong kind) / read_only
  pub fn item_row(conn: &rusqlite::Connection, item_id: &str) -> Result<ItemRow>;
  pub fn visible_items(viewer: &Viewer) -> &'static str;                               // SQL fragment, §5
  pub fn effectively_published(conn: &rusqlite::Connection, item_id: &str) -> Result<bool>;
  pub struct WriteBy<'a> { pub user_id: &'a str, pub token_id: Option<&'a str> }      // content writes (D53)
  pub fn check_free_space(data_dir: &Path, min_free_mb: u64) -> Result<()>;            // D33: 507 insufficient_storage
  // wikanvas-store::revisions
  pub fn maybe_auto(tx, item_id: &str, draft: RevisionDraft, now) -> Result<bool>;    // §5 policy (10 min)
  pub fn pre_api(tx, item_id: &str, kind: ItemKind, by: &WriteBy, now) -> Result<bool>; // D53: last_token_id, source_seq
  pub fn prune(tx, now) -> Result<u64>;
  // wikanvas-store::blobs (D26)
  impl BlobStore { pub fn new(root: PathBuf) -> Self;          // root = the data dir: files in root/files, staging in root/tmp
    pub fn stage(&self, bytes: &[u8]) -> std::io::Result<Staged>;                    // outside the writer
    pub async fn stage_stream(&self, body: impl Stream<Item = std::io::Result<Bytes>> + Unpin, limit: u64)
      -> Result<Staged>;                                                               // uploads: hashed while streamed
    pub fn commit(&self, tx, staged: Staged, now) -> Result<BlobRef /* sha256, size, mime (sniffed), w, h */>;
    pub fn put(&self, tx, bytes: &[u8], now) -> Result<BlobRef>;                       // stage + commit
    pub fn read(&self, sha256: &Sha256) -> std::io::Result<Vec<u8>>;
    pub fn path(&self, sha256: &Sha256) -> PathBuf; }                                 // Sha256: validated 64 lowercase hex
  // wikanvas-store::backup
  pub fn pre_migration(db: &Db, data_dir: &Path, version: u32, now) -> Result<PathBuf>;
  // wikanvas-store::docs / boards  (called by W1 item create/duplicate/discard, and by W2 seed)
  pub fn docs::init_empty(tx, item_id, now) -> Result<()>;
  pub fn docs::init_with(tx, item_id, ydoc_v1: &[u8], now) -> Result<()>;
  pub fn docs::copy(tx, from_id, to_id, now) -> Result<()>;                            // also copies item_attachments
  pub fn docs::is_empty(tx, item_id) -> Result<bool>;                                  // §6.1: only empty paragraphs
  pub fn boards::init_empty(tx, item_id, dark: bool, now) -> Result<()>;
  pub fn boards::init_with(tx, blobs: &BlobStore, item_id, envelope: &str, now) -> Result<()>;
  pub fn boards::copy(tx, from_id, to_id, now) -> Result<()>;
  pub fn boards::is_empty(tx, item_id) -> Result<bool>;
  // wikanvas-store::collections / items (W1 completes; W2's seed and purge job call them)
  pub fn collections::create(tx, actor_id: &str, new: &NewCollection /* api DTO */, now) -> Result<Collection>;
  pub fn items::create(tx, actor_id: &str, new: &NewItem /* api DTO */, dark: bool, now) -> Result<ItemSummary>;
  pub fn items::rename(tx, item_id: &str, title: &str, actor_id: &str, now) -> Result<ItemSummary>; // meta + tree version
  pub fn items::purge_expired(tx, older_than: i64) -> Result<u64>;
  // wikanvas-store::tokens (W2 completes: caps, touch, expiry purge)
  pub fn tokens::create(tx, user_id: &str, new: &NewToken /* api DTO */, via: CreatedVia,
    parent: Option<&TokenRow>, now) -> Result<CreatedToken>;                          // parent caps expiry, forbids admin
  pub fn tokens::lookup(conn: &rusqlite::Connection, token: &str, now)
    -> Result<TokenLookup /* Live(TokenRow{id, user_id, role, scopes, expires_at}) | Expired | Rejected */>;
  pub fn tokens::get(conn: &rusqlite::Connection, user_id: &str, token_id: &str) -> Result<Option<ApiToken>>; // not revoked
  pub fn tokens::list(conn: &rusqlite::Connection, user_id: &str, now) -> Result<Vec<ApiToken>>;
  pub fn tokens::revoke(tx, user_id: &str, token_id: &str, now) -> Result<u64>;      // also its children; 0 = not found
  // wikanvas-core::cursor (W1 owns afterwards)
  pub fn encode(c: &Cursor, query_hash: [u8; 8]) -> String;
  pub fn decode(s: &str, query_hash: [u8; 8]) -> Result<Cursor, CursorError>;          // CursorError → 422 validation
  ```

- **Rust server types:** `AppState{db: Db, blobs: Arc<BlobStore>, config: Arc<Config>, previews: PreviewQueue,
  board_work: Arc<Semaphore> /* 1 */, doc_work: Arc<Semaphore> /* 3; a PUT takes 2 at once */, upload_work: Arc<Semaphore>
  /* 2 */, token_limiter, token_bytes_limiter, auth_fail_limiter, owner_exists: Arc<AtomicBool>, openapi: Arc<str>}`,
  `Actor{user_id, role, method: AuthMethod::Session{session_id} | AuthMethod::Token{token_id, scopes}}`,
  `Auth<S>` with `S ∈ {Read, Write, Admin, Session, AnyToken}` (first handler argument; derefs to `Actor`),
  `Viewer{user_id, role}`, `ApiError` (implements `From<wikanvas_store::Error>` and `IntoResponse` with the §7.1 shape),
  `wikanvas_server::app(state) -> Router`, and `routes::<x>::router() -> OpenApiRouter<AppState>` for `auth`, `setup`,
  `workspace`, `tokens`, `files`, `export`, `config`, `admin`, `collections`, `items`, `tree`, `lists`, `search`, `docs`
  and `boards`. Also `board_previews::{PreviewQueue::enqueue(&self, item_id), spawn(state) -> PreviewQueue}`,
  `openapi::spec() -> utoipa::openapi::OpenApi`, and `testing::TestServer` (feature `testing`).
- **DTOs:** all of §7 in `wikanvas-api`, plus `ErrorCode`, `ErrorBody`, `Page<T>`, `Scope`, `API_VERSION`,
  `cli::ExitCode` and `token::{encode, check}`.
- **Command lines:** the `wikanvas-server` subcommands and flags of §7.8, and the `wikanvas` global flags and command
  table of §7.6 (W8 implements the table; a change to it goes through R3).
- **TypeScript:** the modules listed in task 7 (signatures frozen), `BoardFrameHost`/`useBoardFrame` (§9.3), and
  `bridge.ts` (owned by W5 after W0b).
- **Scripts:** `scripts/inkboard.sh` subcommand names and the dist layout (`web/dist-inkboard/` plus `manifest.json`,
  §10.5); `scripts/fmt.sh`, `scripts/check-pins.sh`.

### Acceptance

```sh
for f in scripts/*.sh scripts/lib/*.sh; do bash -n "$f" || exit 1; done   # bash -n a b checks only a
scripts/toolchain.sh
scripts/dev.sh rustup show active-toolchain | grep -q '^1\.99\.1-'
scripts/dev.sh cargo clippy --version && scripts/dev.sh rustfmt --version
scripts/dev.sh wasm-opt --version | grep -q 'version 131'
scripts/dev.sh brotli --version
scripts/check-pins.sh
scripts/dev.sh cargo build --workspace --locked
scripts/dev.sh cargo tree --locked -e normal -i libsqlite3-sys
scripts/dev.sh cargo test --workspace --locked
scripts/dev.sh cargo clippy --workspace --all-targets --locked -- -D warnings
scripts/fmt.sh --check
scripts/dev.sh cargo test -p wikanvas-api --locked export_bindings && git diff --exit-code web/src/api/gen
scripts/dev.sh cargo test -p wikanvas-server --locked --test api -- openapi:: && git diff --exit-code docs/api/openapi.json
scripts/dev.sh cargo test -p wikanvas-cli --locked --test cli
! scripts/dev.sh cargo tree --locked -p wikanvas-server -e normal --prefix none | grep -qE '^(rustls|ureq|reqwest|native-tls|openssl) '
! scripts/dev.sh cargo tree --locked -p wikanvas-cli -e normal --prefix none | grep -qE '^(openssl|openssl-sys|native-tls|rusqlite|yrs|inkboard-core|comrak|webpki-roots) '
scripts/dev.sh cargo run -p wikanvas-cli --locked -- exit-codes --json | grep -q '"code": *9'
scripts/dev.sh pnpm --dir web install --frozen-lockfile
scripts/dev.sh pnpm --dir web typecheck && scripts/dev.sh pnpm --dir web test && scripts/dev.sh pnpm --dir web build
scripts/dev.sh pnpm --dir e2e install --frozen-lockfile
scripts/dev.sh scripts/inkboard.sh stub && grep -q '"format_version": *2' web/dist-inkboard/manifest.json
docker compose -f compose.yaml -f compose.e2e.yaml config -q
docker compose -f compose.yaml -f compose.smoke.yaml config -q
scripts/image.sh --build-arg INKBOARD_BUILD=stub && WIKANVAS_SMOKE_LEVEL=basic scripts/image-smoke.sh   # incl. curl through Caddy over HTTPS with the local CA
git -C vendor/inkboard status --porcelain | wc -l | grep -qx 0
```

---

## W1: tree, collections, items, lists, search (backend)

**Goal.** The whole hierarchy:
- collections: create, edit, reorder, soft-delete (with their items, §5), restore;
- items: create (published on create, D23), rename, move (within and across collections, with cycle checks and
  top-down collection updates), duplicate, publish (optionally with ancestors) and unpublish (API and CLI only; the
  draft state stays in the model, D23), archive and unarchive (whole subtree), trash, restore, purge, discard of a new
  empty item (`?discard_empty=true`, §7.2);
- the flat tree with an ETag, children and breadcrumbs, all through `shared::visible_items`;
- stars with ordering, views, and the lists recent, updated, drafts, starred, archive and trash;
- FTS search with the §5 query builder, and title search;
- the rank rebalance and the revision prune;
- cursor pagination (D46) on every paginated list and on search (`Page<T>`, `Page<SearchHit>`), with `cursor.rs`
  completed from W0b's version;
- item metadata concurrency: `ETag: "m{meta_version}"` on `GET /items/{id}` (never 304) and an optional `If-Match`
  on `PATCH /items/{id}` (412 `rev_mismatch`, D45; a `W/` prefix ignored); `GET /items/{id}/text` (§7.2);
- `items::rename` (bumps `meta_version` and `tree_version`; used by PATCH and by W4's `?title=from_h1`) and
  `revisions::pre_api` (D53, §5), both completed from W0b's versions; `POST /collections` completed from W0b's;
- `#[utoipa::path]` annotations with the §7.2 scopes and `Auth<S>` on every W1 route (R4);
- ownership of the schema and of the shared DTOs (`wikanvas-api` `common`, `tree`, `page`).

### Owns

| Area | Files |
|---|---|
| `crates/wikanvas-api` | `Cargo.toml`, `src/{common,tree,page}.rs` |
| `crates/wikanvas-core` | `Cargo.toml`, `src/{ids,rank,tree,time,cursor}.rs` |
| `crates/wikanvas-store` | `Cargo.toml`, `src/{error,db,migrate,shared,collections,items,tree,search,stars,views,lists,revisions}.rs`, `migrations/**` |
| `crates/wikanvas-store/tests/store` | `{common,tree,items,search,lists}.rs` |
| `crates/wikanvas-server` | `src/routes/{collections,items,tree,lists,search}.rs` |
| `crates/wikanvas-server/tests/api` | `{tree,items,search,lists}.rs` |
| `web/src/api/gen` | the generated files for the types in `wikanvas-api` `common.rs`, `tree.rs` and `page.rs` |

### Produces

- The W1 routes of §7.2, and the `ItemSummary`, `ItemDetail` (with `effectively_published`), `Collection`, `Crumb` and
  position semantics.
- `collections::create`, `items::create`, `items::rename` and `items::purge_expired` completed;
  `revisions::{prune, pre_api}`.
- `wikanvas-core::rank::{key_between, rebalance}`, `ids::{new_uuid, new_url_id, slugify}` and
  `cursor::{encode, decode}`; `wikanvas_store::search::build_match` (wikanvas-core's frozen module list has no
  `search`); and a store helper `lists::page(…)` that every paginated query uses.

### Consumes

- `Db`, `ApiError`, `Actor`, `Viewer` (from W0b, owned by W2); `backup::pre_migration` (W0b minimal, owned by W2).
- `docs::*` and `boards::*` init, copy and is_empty (W0b minimal versions, owned by W4 and W6).
- `ItemDetail.board` is filled from `board_state.meta` by a store read in `items.rs`; the meta JSON shape is §5.

### Key behaviours

- Item create, move, trash, restore and archive bump `tree_version` on every affected collection.
- Every create (route, seed, duplicate) sets `published_at = created_at` (D23); a draft exists only after an unpublish.
  Publication is derived: publish with `with_ancestors` publishes every draft ancestor; unpublish always works;
  `drafts_count`, `/drafts` and `include_drafts` count and filter those unpublished items.
- `?discard_empty=true` deletes permanently only an item created less than 24 h ago with no children, an empty title
  and an empty body, whether or not it is published; otherwise 409 `not_empty`.
- Restore or unarchive whose parent is gone moves the subtree to the end of the top level and returns
  `moved_to_top: true`. Restore, create or move into a deleted collection gives 409 `parent_invalid`.
- A non-manual `child_order` affects tree output order only.
- Moves of a whole subtree never touch the descendants' `sort_key`; cross-collection moves update `collection_id`
  top-down, one depth level per statement.

### Acceptance

```sh
scripts/dev.sh cargo test -p wikanvas-core   --locked -- ids:: rank:: tree:: cursor::
scripts/dev.sh cargo test -p wikanvas-store  --locked --test store -- tree:: items:: search:: lists::
scripts/dev.sh cargo test -p wikanvas-server --locked --test api   -- tree:: items:: search:: lists::
scripts/dev.sh cargo test -p wikanvas-server --locked --test api   -- openapi:: && git diff --exit-code docs/api/openapi.json
```

These tests must exist and pass:

| Crate | Test |
|---|---|
| wikanvas-core | `rank::tests::between_is_strictly_between` (proptest) |
| wikanvas-core | `rank::tests::matches_fractional_indexing_vectors` |
| wikanvas-core | `tree::tests::random_ops_never_cycle_and_match_model` (proptest, 500 cases: create, move, trash, restore, archive) |
| wikanvas-store | `tree::purge_depth_2000_leaves_first` |
| wikanvas-store | `tree::move_into_descendant_is_cycle` |
| wikanvas-store | `tree::cross_collection_move_updates_descendants` (including trashed and archived descendants) |
| wikanvas-store | `tree::parent_trigger_rejects_trashed_or_archived_parent` |
| wikanvas-store | `tree::collection_trigger_rejects_foreign_parent` (`UPDATE items SET collection_id` alone) |
| wikanvas-store | `items::trash_restore_subtree_roundtrip` |
| wikanvas-store | `items::restore_with_trashed_parent_goes_top` |
| wikanvas-store | `items::collection_delete_restore_roundtrip` (items trashed earlier stay in Trash) |
| wikanvas-store | `items::create_restore_or_move_into_deleted_collection_409` |
| wikanvas-store | `items::archive_hides_subtree_from_tree_and_default_search` |
| wikanvas-store | `items::effectively_published_needs_published_ancestors` |
| wikanvas-store | `items::rebalance_when_key_over_64` |
| wikanvas-store | `search::fts_integrity_after_rename_save_purge` |
| wikanvas-store | `search::query_builder_treats_fts_syntax_as_literal` (input `a OR b* NEAR( "x`) |
| wikanvas-store | `search::negative_terms` (`plan -x` finds "plan quokka", not "plan x") |
| wikanvas-store | `search::only_negative_terms_return_nothing` (`-x` alone: no FTS error, no results) |
| wikanvas-store | `search::remove_diacritics` (`café` finds `cafe`) |
| wikanvas-server | `tree::etag_304_until_structure_changes` (ETag `"t{n}.0"`) |
| wikanvas-server | `items::create_is_published_and_initializes_content` (`published: true`, `effectively_published: true`, `drafts_count` unchanged) |
| wikanvas-server | `items::discard_empty_refuses_nonempty_409` (also an empty item older than 24 h; a published new empty item is discarded) |
| wikanvas-server | `items::unpublish_makes_draft_listed_in_drafts` (`/drafts` and `drafts_count` see it; tree and default search still show it) |
| wikanvas-server | `items::publish_with_ancestors` (after unpublishing the item and its parent) |
| wikanvas-server | `items::unpublish_always_allowed` |
| wikanvas-server | `items::get_by_url_id` |
| wikanvas-server | `lists::trash_lists_roots_with_purge_at` (item roots and collections) |
| wikanvas-server | `search::snippet_markers_and_draft_filter` |
| wikanvas-core | `cursor::tests::roundtrip_and_query_binding` (a cursor decoded with another query hash fails; a tampered cursor fails) |
| wikanvas-store | `lists::cursor_stable_under_concurrent_insert` (insert between two pages: no item repeated or skipped) |
| wikanvas-server | `lists::cursor_from_other_query_422` (`details.field == "cursor"`) |
| wikanvas-server | `lists::every_paginated_list_has_page_shape` (`items` and `next_cursor`; `null` on the last page) |
| wikanvas-server | `search::relevance_offset_capped_at_1000` |
| wikanvas-server | `items::get_sets_meta_etag_and_patch_stale_if_match_412` (and `GET /items/{id}` never answers 304, also after a star) |
| wikanvas-server | `items::rename_bumps_tree_version` (the tree ETag changes after a PATCH of the title) |
| wikanvas-store | `items::pre_api_rule` (an owner edit then a token write: one `pre-api`; the same token's next write: none; newest revision equal to the current state: none) |
| wikanvas-server | `items::text_returns_derived_body` (doc and board) |
| wikanvas-server | `items::purge_needs_admin_scope` (write token → 403 `insufficient_scope`) |

---

## W2: platform (backend)

**Goal.**
- Setup with a code (rate-limited) and the seeded "Getting started" collection.
- Login, logout, sessions with half-window renewal and cookie refresh, password change, the `reset-password` CLI,
  `must_change_password`.
- The Host allowlist and CSRF middleware, rate limits, security headers, static serving with precompressed variants,
  `/api/v1/config` (with `inkboard: null` when no manifest).
- **Behind Caddy** (D57, §8, §11): `WIKANVAS_TRUSTED_PROXIES` parsed as IPs and CIDRs; `X-Forwarded-For` (rightmost
  address) and `X-Forwarded-Proto` honoured only from a trusted peer, `Forwarded` ignored; the client IP feeding every
  per-IP limiter, the access log, `sessions.ip` and `last_used_ip`; the request scheme choosing the cookie
  (`__Host-wikanvas_session; Secure` on HTTPS, `wikanvas_session` on plain HTTP, read by that name only);
  `WIKANVAS_DOMAIN` with any port in the Host allowlist; the public URL derived from `WIKANVAS_DOMAIN` and
  `WIKANVAS_HTTPS_PORT`; HSTS (`max-age=31536000`) on HTTPS requests when `WIKANVAS_TLS=acme` only; the CSP unchanged.
- Attachments (filename sanitizing, RFC 6266 headers, header-only image size and the 100 MP limit) and blobs;
  attachment GC, blob GC and the orphan sweep (D26, D36).
- Jobs: purge, GC, backup, optimize, session expiry, and the scheduling of the preview worker.
- Backup on the maintenance connection; export without sessions, API tokens or password hashes;
  `backup::pre_migration`.
- The `wikanvas-server` commands (§7.8): `serve`, `healthcheck` (URL from `WIKANVAS_LISTEN`), `admin create-owner`,
  `create-token`, `list-tokens`, `revoke-token`, `reset-password`, `backup`, `check`, all with `--json` and the §7.7
  exit codes.
- Graceful shutdown with a WAL checkpoint.
- **Public API platform** (§7.1, §7.3, §7.4, D39–D48):
  - personal access tokens end to end: `tokens::*` (create with the 100 limit, the scope ceiling, the expiry cap for
    token callers, no `admin` from a token, `parent_token_id`, and the password for a session's `admin` or
    never-expiring token; lookup with classification; list; revoke with children; revoke-current; the once-a-minute
    `last_used` write; deletion 30 days after revocation or expiry), the `/api/v1/tokens` routes completed from W0b's,
    `Me.auth`, and `Auth<S>` completed (expiry reason, disabled users);
  - `is_wikanvas_bearer` shared by `Auth<S>` and the CSRF layer; other `Authorization` values ignored; CSRF for every
    non-token mutation, including those with neither `Sec-Fetch-Site` nor `Origin`; the cookie rule for
    `GET /export`; `Authorization` in `SensitiveHeaders`, `token_id` in the access log;
  - CORS from `WIKANVAS_CORS_ORIGINS` (`cors.rs`); the per-token request and byte limiters and the per-IP failed-bearer
    limiter that still serves valid tokens, pruned hourly (`ratelimit.rs`);
  - the free-space floor (507), the `upload_work` permit with streamed uploads, single-flight export and backup
    (429 `busy`), the export copy on `journal_mode=DELETE`, on-demand backups under `WIKANVAS_BACKUP_KEEP`;
  - the owner-exists cache that re-checks while false (`admin create-owner` while serving), `reset-password
    --revoke-tokens`, the session-only email change, `others=true` for token callers, the full `limits` in
    `/api/v1/config`;
  - the error envelope for every rejection (404, 405, 408, malformed JSON, path and query), `X-Request-Id`,
    `Wikanvas-Api-Version`, `WWW-Authenticate` on 401 `invalid_token` and 403 `insufficient_scope`;
  - the OpenAPI assembly and `GET /api/v1/openapi.json` (`openapi.rs`), and the generic scope test
    `security::every_operation_enforces_its_declared_scope`;
  - `wikanvas_server::testing::TestServer` completed (W8's CLI tests run against it).

### Owns

| Area | Files |
|---|---|
| `crates/wikanvas-api` | `src/{auth,tokens,files,error,token}.rs` |
| `crates/wikanvas-store` | `src/{users,sessions,tokens,settings,blobs,attachments,gc,backup,export,seed}.rs` |
| `crates/wikanvas-store/tests/store` | `{auth,tokens,blobs,backup}.rs` |
| `crates/wikanvas-server` | `Cargo.toml`, `src/{main,cli,admin,config,app,state,error,extract,authz,csrf,cors,ratelimit,headers,static_files,jobs,openapi,testing}.rs`, `src/routes/{auth,setup,workspace,tokens,files,export,config,admin}.rs` |
| `crates/wikanvas-server/tests/api` | `{common,auth,tokens,security,openapi,static_files,files,export,admin}.rs` |
| `crates/wikanvas-assets` | everything |
| `web/src/api/gen` | the generated files for the types in `wikanvas-api` `auth.rs`, `tokens.rs`, `files.rs` and `error.rs` |

### Produces

- Session and token auth (`Auth<S>`, `Actor`, `Viewer`), the `ApiError` mapping, the client address and scheme from
  the trusted proxy (§8), the Host/CSRF, CORS and limits layers, the §11 headers (HSTS included), `/api/v1/config` (with `api_version` and `openapi_url`), `/api/v1/openapi.json`, the token
  routes, the admin commands, and `TestServer`.
- `static_files` is generic over an `AssetSource` trait: rust-embed in production, an in-memory map in tests.
- The seed: `seed::getting_started(tx, blobs, owner_id, now)` uses W1's create functions and `docs::init_with` /
  `boards::init_with` with `fixtures/ydoc/getting-started.bin` and `fixtures/boards/getting-started.inkboard`; the
  seeded items are published on create like every item (D23).
- At startup: the manifest check (§10.5: missing → warn and `inkboard: null`; `format_version` mismatch → refuse to
  start), and `inkboard_core::model::seed_ids(<random>)`.

### Consumes

- W1 `items::purge_expired`, `revisions::prune`, and the collection and item create functions (W0b minimal versions
  until W1 merges).
- W6 `board_previews::spawn`.
- The W4 and W6 fixture files.

### Acceptance

```sh
scripts/dev.sh cargo test -p wikanvas-store  --locked --test store -- auth:: tokens:: blobs:: backup::
scripts/dev.sh cargo test -p wikanvas-server --locked --test api   -- auth:: tokens:: security:: openapi:: static_files:: files:: export:: admin::
scripts/dev.sh cargo test -p wikanvas-server --locked --test api   -- openapi:: && git diff --exit-code docs/api/openapi.json
scripts/dev.sh env WIKANVAS_LISTEN=127.0.0.1:9 cargo run -p wikanvas-server --locked -- healthcheck; test $? -eq 1
scripts/dev.sh bash -c 'd=$(mktemp -d) && export WIKANVAS_DATA_DIR=$d \
  && cargo run -q -p wikanvas-server --locked -- admin create-owner --email a@wikanvas.invalid --name A --workspace W --generate-password --json >/dev/null \
  && cargo run -q -p wikanvas-server --locked -- admin create-token --name t --scope read --expires 1d --json | grep -q "\"token\": *\"qpat_" \
  && { cargo run -q -p wikanvas-server --locked -- admin create-owner --email b@wikanvas.invalid --name B --workspace W --generate-password; test $? -eq 8; }'
! grep -rn '\.route(' crates/wikanvas-server/src/routes
! grep -rn '\.route("/api' crates/wikanvas-server/src
```

These tests must exist and pass:

| Area | Test |
|---|---|
| Setup | `auth::setup_requires_code` |
| Setup | `auth::setup_only_once_under_concurrency` |
| Setup | `auth::setup_required_409_before_setup` |
| Setup | `auth::setup_rate_limited_per_ip` |
| Setup | `auth::setup_seeds_getting_started` |
| Login | `auth::login_cookie_attributes` (HttpOnly, SameSite=Lax, Path=/; `__Host-` and Secure only for `X-Forwarded-Proto: https` from a trusted peer, plain `wikanvas_session` otherwise; Max-Age only with `remember`) |
| Login | `auth::cookie_read_by_request_scheme` (a `__Host-wikanvas_session` sent on a plain-HTTP request is ignored, and `wikanvas_session` on an HTTPS one) |
| Login | `auth::login_rate_limited_429_retry_after` (`FakeRelativeClock`) |
| Login | `auth::email_limiter_counts_failures_only` |
| Login | `auth::unknown_email_same_code_as_bad_password` |
| Sessions | `auth::renewal_after_half_window_resends_cookie` (persistent: new Max-Age; 180 d cap) |
| Sessions | `auth::password_change_revokes_other_sessions` |
| Sessions | `auth::reset_password_cli_sets_must_change` |
| Security | `security::host_not_allowlisted_421` (and `/healthz` exempt) |
| Security | `security::host_allowlist_accepts_wikanvas_domain_any_port` (`WIKANVAS_DOMAIN=wikanvas.lan`: `wikanvas.lan` and `wikanvas.lan:8443` pass, `evil.lan` gets 421) |
| Security | `security::forwarded_headers_only_from_trusted_proxy` (peer injected through `ConnectInfo`: from an untrusted peer, `X-Forwarded-For` does not change the login limiter's key and `X-Forwarded-Proto: https` gives no `Secure`; from `WIKANVAS_TRUSTED_PROXIES` both apply and the rightmost address counts) |
| Security | `security::hsts_only_on_https_in_acme_mode` (present with `WIKANVAS_TLS=acme` and a trusted `https`; absent on plain HTTP, absent with `WIKANVAS_TLS=internal`) |
| Security | `security::public_url_derived_from_domain` (`WIKANVAS_DOMAIN` + `WIKANVAS_HTTPS_PORT=8443` → `https://<domain>:8443` in the setup log and the CSRF origin; an explicit `WIKANVAS_PUBLIC_URL` wins) |
| Security | `security::csrf_cross_site_403` |
| Security | `security::csrf_origin_only_cross_site_403` (no `Sec-Fetch-Site`, foreign `Origin`) |
| Security | `security::csrf_origin_only_same_host_ok` |
| Security | `security::csrf_same_origin_ok` |
| Security | `security::csrf_no_fetch_metadata_no_origin_403` (a cookie mutation with neither header) |
| Security | `security::basic_auth_header_takes_cookie_path_with_csrf` (cookie + `Authorization: Basic …`: cross-site → 403 `csrf_rejected`, same-origin → 2xx) |
| Security | `security::foreign_bearer_ignored` (cookie + `Bearer <JWT>` → served through the cookie) |
| Security | `security::form_body_415` |
| Security | `security::default_body_limit_413` |
| Static files | `static_files::spa_fallback_vs_api_json_404` |
| Static files | `static_files::extension_404` |
| Static files | `static_files::br_then_gzip_then_identity_with_vary` (the `br` body is byte-identical to the embedded `.br` file, which the image smoke then checks through Caddy) |
| Static files | `static_files::wasm_content_type_and_immutable` |
| Static files | `static_files::icons_bin_identity_only` |
| Static files | `static_files::spa_csp_and_frame_csp` |
| Static files | `static_files::old_inkboard_build_404` |
| Static files | `static_files::missing_manifest_gives_null_inkboard_config` |
| Files | `files::upload_mime_sniffed_not_trusted` |
| Files | `files::svg_attachment_is_download_with_sandbox_csp` |
| Files | `files::content_disposition_rfc6266_sanitized` (CR, LF and `"` in the name) |
| Files | `files::image_over_100_megapixels_422` (a tiny file whose header claims huge dimensions) |
| Files | `files::upload_over_limit_413` |
| Files | `files::low_disk_507` (an injected free-space probe; also for board PUT staging and export) |
| Files | `files::upload_streams_under_upload_work` (a third concurrent upload waits; peak memory independent of size) |
| Files | `files::octet_stream_txt_upload_accepted` (a `.txt` name sent as `application/octet-stream`; `text/plain` → 415) |
| Export | `export::tar_has_manifest_db_and_files` |
| Export | `export::copy_has_no_sessions_or_password_hashes` |
| Export | `export::extracted_db_has_no_credentials_without_sidecars` (tar extracted into an empty dir, `wikanvas.db` opened read-only with no `-wal`: the tables are empty; the raw bytes contain no `$argon2id$` and no known token or session sha256) |
| Export | `export::single_flight_429_busy` |
| Export | `export::cookie_cross_site_navigation_403` (`Sec-Fetch-Site: cross-site`; and, without fetch metadata, no or a foreign `Referer`) |
| Backup | `backup::on_demand_counts_against_keep` |
| Backup | `backup::vacuum_into_through_db_passes_quick_check` (the real `Db`; a `query_only` reader would fail) |
| Blobs | `blobs::gc_keeps_referenced_blobs_including_revision_assets` |
| Blobs | `blobs::gc_between_stage_and_commit_never_leaves_row_without_file` |
| Blobs | `blobs::attachment_gc_respects_refs_and_24h_grace` |
| Tokens | `tokens::create_shows_secret_once_list_never` (list and `Me` never contain the secret or hash) |
| Tokens | `tokens::scopes_expanded_and_capped_by_caller` (a write token cannot reach `POST /tokens`, which needs `admin`: 403; a token asking for `admin` is 403 `session_required`; a token's own cap is 422 `scopes`, reachable in the store: a read parent, a write child) |
| Tokens | `tokens::malformed_token_401_without_lookup` (bad checksum; a counting reader sees no query) |
| Tokens | `tokens::expired_token_401_reason_expired` |
| Tokens | `tokens::lookup_classifies_expired_revoked_disabled` (revoked, disabled and unknown give the same body) |
| Tokens | `tokens::token_cannot_mint_longer_lived_token` (422, `field: "expires_in_days"`) |
| Tokens | `tokens::token_cannot_mint_admin_403` (`session_required`) |
| Tokens | `tokens::revoking_parent_revokes_children` |
| Tokens | `tokens::session_admin_or_never_needs_password` (missing or wrong: 422 `field: "password"`, never 401) |
| Tokens | `tokens::revoked_token_rejected_on_next_request` |
| Tokens | `tokens::current_revokes_itself` |
| Tokens | `tokens::limit_100_live_tokens_422` |
| Tokens | `tokens::last_used_written_at_most_once_per_minute` (injected clock) |
| Tokens | `tokens::password_change_keeps_tokens` |
| Tokens | `tokens::purged_30_days_after_revocation` (store) |
| Auth | `auth::me_with_token_reports_auth_token` |
| Auth | `auth::email_change_needs_session` · `auth::sessions_others_with_token_revokes_all` |
| Auth | `auth::config_reports_limits_and_api_version` (every `limits` field of §7.2) |
| Security | `security::wikanvas_bearer_wins_over_cookie` (valid cookie + bad `qpat_` token → 401 `invalid_token`) |
| Security | `security::bearer_skips_csrf` (token + `Sec-Fetch-Site: cross-site` → 2xx) |
| Security | `security::session_only_routes_refuse_tokens_403` (`auth/password`, `auth/logout`) |
| Security | `security::token_only_route_refuses_session_403` (`DELETE /tokens/current`, `forbidden`) |
| Security | `security::every_operation_enforces_its_declared_scope` (walks the generated spec, §7.4) |
| Security | `security::public_operations_are_exactly` (§7.4) |
| Security | `security::failed_bearer_limit_spares_valid_tokens` (over the per-IP limit: unknown token → 429, valid token → 2xx) |
| Security | `security::token_byte_budget_429_reason_bytes` (`FakeRelativeClock`) |
| Security | `security::api_version_header_on_every_response` · `security::timeout_408_envelope` |
| Security | `security::cors_off_by_default` · `security::cors_allow_list_never_credentials` · `security::cors_star_refused_at_startup` |
| Security | `security::cookie_from_cors_origin_still_csrf_rejected` |
| Security | `security::token_rate_limit_429_retry_after` (`FakeRelativeClock`) |
| Security | `security::failed_bearer_limited_per_ip` |
| Security | `security::authorization_redacted_in_logs` (captured tracing output) |
| Security | `security::unversioned_api_path_404_json` · `security::method_not_allowed_405_envelope` · `security::malformed_json_422_envelope` |
| OpenAPI | `openapi::spec_matches_committed` · `openapi::spec_parses_as_3_1` · `openapi::served_no_cache_with_etag` |
| Export | `export::copy_has_no_api_tokens` |
| Admin | `admin::create_owner_once_then_exit_8` |
| Admin | `admin::create_token_prints_once_and_authenticates_over_http` |
| Admin | `admin::runs_while_server_is_up` (WAL, second process) |
| Admin | `admin::create_owner_while_serving_clears_setup_required` (the next request is no longer 409) |
| Admin | `admin::reset_password_revoke_tokens` · `admin::create_token_token_out_0600` |

---

## W3: web shell and navigation (frontend)

**Goal.** Everything a user sees except the document editor body and the board canvas, as specified in §9.6:
- **App structure:** `AuthGate`; the app shell with the resizable, collapsible sidebar and the drawer below 1024 px; the
  "Wikanvas was updated. Reload" banner when a route chunk fails to load (§9.1).
- **Sidebar (§9.6.2):** the workspace header (logo, workspace name, user name, menu), nav (Home, Search, Starred; no
  Drafts entry, D23), the STARRED section, and the collections tree: virtualized visible-row model, carets, persisted expand state, the current
  item highlighted with its ancestors expanded, hover menus, and the ARIA tree keyboard (§9.6.3). Drag and drop
  (§9.6.3): before/inside/after zones, non-manual groups, drops on collection rows, cross-collection moves, collection
  reorder, star reorder, auto-expand after 700 ms, auto-scroll, outdent by x position, and the Undo toast. The "+"
  popover with Document (D) and Board (B), rename, and the row menus.
- **Dialogs and overlays:** the collection dialog (48 icons, 12 colours, §9.6.1), the Move dialog, the quick switcher,
  the shortcuts dialog, the sign-in dialog, toasts.
- **Shared page components (§9.6.4):** `TopBar` and `ItemHeader`, including title autosave (500 ms debounce, registered
  in `pending`).
- **Pages (§9.6.6):** setup, login, Home, Search (no drafts filter), Starred, Archive, Trash, Settings (including the
  API tokens section: list, create with the show-once secret, revoke), Collection, NotFound.
- **API client:** every request goes to `API_BASE = '/api/v1'` with the cookie and never sends `Authorization` (D39);
  uploads (the workspace logo) send `application/octet-stream`; list hooks read `Page<T>` (`items`, `next_cursor`),
  including search. `lib/urls.ts` `boardPreviewUrl(id, rev, page)` is `/api/v1/items/{id}/board/pages/{page}?rev={rev}`
  (no `.svg`, §7.2).
- **API tokens dialog:** the password field for Admin scope or Never (§9.6.6); a 422 `field: "password"` shows inline.
- **Styles:** the tokens and themes of §9.6.1.
- Hidden in M1: Templates, Invite, Share, avatars, "Viewed by". Not built in M1 (D23: items are published on create,
  and the draft state lives only in the API): the Drafts nav entry, badge and page, the Draft pill, the Publish button
  and its "Also publish N parent pages?" prompt, " · Published" in the meta line, Mod+Shift+P and the search page's
  "include drafts" filter.

### Owns

- `web/package.json`, `web/pnpm-lock.yaml`, `web/vite.config.ts`, `web/tsconfig.json`, `web/index.html`, `web/public/**`
- `web/src/main.tsx`, `web/src/app/**` (keep the `<BoardFrameHost/>` mount W0b placed in `AppShell`),
  `web/src/api/client.ts`
- `web/src/api/queries/{auth,tree,items,lists,search,workspace,config,tokens}.ts`
- `web/src/state/**`, `web/src/styles/**`, `web/src/components/**`, `web/src/lib/**`, `web/src/test/**`
- `web/src/pages/**`, except `DocPage.tsx` and `BoardPage.tsx`

### Produces

- The frozen W0b TypeScript modules, now fully implemented.
- The query hooks `useTree`, `useSidebar`, `useMoveItem`, `useMoveCollection`, `useCreateItem`, `useStar`,
  `useTrash`, `useSearch`, `useTitleSearch` and `useMe` (no `usePublish` in M1, D23).
- `runHostShortcut`, which handles Mod+K, Mod+. and Escape from the board frame.

### Consumes

- The W1 and W2 routes, mocked in tests with `src/test/mockApi.ts`.
- `DocPage` (W4) and `BoardPage` (W6) through lazy routes; `BoardFrameHost` (W6) through the existing mount.

### Acceptance

```sh
scripts/dev.sh pnpm --dir web typecheck
scripts/dev.sh pnpm --dir web test src/app src/api src/components src/pages src/state src/lib
scripts/dev.sh pnpm --dir web build
```

These tests must exist and pass:

| Area | Test |
|---|---|
| Tree | `tree/buildTree.test.ts`: flat list to nested; all four sorts |
| Tree | `tree/visibleRows.test.ts`: expanded set; indent capped at level 10 |
| Tree | `tree/hitZones.test.ts`: 25/50/25 zones; outdent on the last child by pointer x |
| Tree | `tree/positions.test.ts`: drop maps to `{after_id, before_id}` |
| Tree | `tree/nonManualDrop.test.ts`: before/after in a non-manual group becomes "into the parent" |
| Tree | `tree/invalidTargets.test.ts`: self and descendants refused |
| Tree | `tree/keyboard.test.tsx`: every key of the §9.6.3 table, including Alt+Shift indent, outdent and reorder |
| Sidebar | `sidebar/optimisticMove.test.tsx`: rollback and error toast on 409 `cycle`; Undo moves back |
| Sidebar | `sidebar/collectionDrop.test.tsx`: drop on a collection row moves to its top level (cross-collection) |
| Sidebar | `sidebar/collectionReorder.test.tsx` |
| Sidebar | `sidebar/currentItem.test.tsx`: current item highlighted, ancestors expanded |
| Sidebar | `sidebar/expandedPersistence.test.ts` |
| Sidebar | `sidebar/noDraftUi.test.tsx`: no Drafts entry or badge even with `drafts_count > 0`; an item with `published: false` renders like any other row; the top bar has no Publish button; Mod+Shift+P does nothing (D23) |
| Quick switcher | `quickSwitcher.test.tsx`: empty query shows recent; typing shows titles; Enter opens; Mod+Enter opens a new tab |
| Search | `search/snippet.test.tsx`: an `<img onerror>` snippet renders as text; U+E000/E001 become `<mark>` |
| Item header | `item/titleAutosave.test.tsx`: 500 ms debounce; registered in `pending`; flushed on route change |
| App | `AuthGate.test.tsx`: 401 → `/login?next=`; 409 → `/setup`; `must_change_password` → the account settings |
| App | `updateBanner.test.tsx`: a failed chunk load plus a changed `/api/v1/config` version shows the banner |
| App | `saveStatus.test.ts` |
| App | `theme-init.test.ts` |
| Pages | `trash.test.tsx`: restore, and delete forever with the count in the confirmation |
| Pages | `settings/apiTokens.test.tsx`: create shows the secret once with Copy; closing the dialog drops it (not in the query cache, not in `localStorage`); the list shows prefix, scopes, last used and expiry; revoke asks for confirmation; "Never" shows the warning; Admin or Never asks for the password |
| Lib | `lib/urls.test.ts`: `boardPreviewUrl` is `/api/v1/items/<id>/board/pages/<n>?rev=<r>` |
| API | `api/client.test.ts`: every path is prefixed with `/api/v1`; no `Authorization` header; `ApiError` carries `code` and `requestId` from the envelope |
| API | `api/pages.test.ts`: `useSearch` and the list hooks follow `next_cursor` and stop at `null` |

---

## W4: documents (full stack)

**Goal.** Documents work end to end.
- **Server:**
  - Yjs storage under the `doc_work` permit: GET with `since`, POST updates with validation, `missing_deps`, no-op
    detection by state vector **and** delete set, compaction, the 16 MiB cap, text, links, `item_attachments` and
    `doc_clients` maintenance, `pm_json` cache, revisions with `attachment_ids`.
  - `wikanvas-core::ytext` (§6.1), including `is_empty`.
  - **Conversion** (§6.4, D49, D50): `wikanvas-core::schema` (schema v1 as data, checked against
    `fixtures/schema/v1.json`; `normalize`) and `wikanvas-core::convert` (Y ↔ raw and normalized ProseMirror JSON after
    `@tiptap/y-tiptap`, the nesting pre-scan, the Markdown parser on comrak and Wikanvas's printer with its block map,
    the scheme allow-list, the diff writer: Markdown-space comparison, `blockId` and hash anchors, similarity pairing,
    deterministic client id and `blockId`s, the writer re-check, `schema_too_new`, `?merge=true`, conversion
    warnings). A PUT holds two of the three `doc_work` permits from parsing to commit, in `spawn_blocking`, and saves
    through the permit-free variant of the §6.1 path.
  - The content routes `GET`/`PUT …/doc/markdown`, `GET`/`PUT …/doc/prosemirror` and `GET /api/v1/schema/doc` (§7.2),
    with `"d{seq}"` ETags (Markdown GET never 304), `If-Match`/428/412 (200 `changed: false` when a stale target
    already equals the current state), `?title=include|from_h1` (`items::rename`), `?download=true`, `pm_json = NULL`
    after an API write, and the `pre-api` revision through `revisions::pre_api` for token PUTs and token Yjs POSTs
    (D53); OpenAPI annotations for every W4 route.
- **Client:**
  - `DocSession` autosave with the IndexedDB outbox written on every update (throttled, §9.4).
  - The TipTap editor with schema v1 and every §9.6.5 feature: slash menu, Markdown shortcuts, bubble menu, mentions
    with live titles and dead-target states, callouts, code blocks with lazy lowlight, tables, images uploaded through
    `/api/v1/attachments` as `application/octet-stream` with progress and retry, links limited to the §6.4 scheme
    allow-list, external-image placeholders, the `boardEmbed` card (SVG preview, never
    the wasm), the block handle, UniqueID with `filterTransaction`.
  - The Contents column computed from headings (≥2 headings, scroll spy, popover below 1280 px), the nested-items list,
    Markdown export (a download of `GET …/doc/markdown?download=true`) and HTML export (client-side), print CSS, and
    read-only modes for trashed and archived items.
  - The new-empty-item discard on leave (`DELETE ?discard_empty=true` for an item this page session created; no draft
    check, D23).
- **Fixtures:** the headless fixture writer for `fixtures/ydoc/*` (fixed Y `clientID`s and seeded UniqueID ids, so
  output is byte-stable), including `getting-started.bin` with the guide content of §9.6.7 (no drafts or Publish in
  the text), plus `fixtures/ydoc/*.y.json`
  (raw), `*.pm.json` (normalized) and `fixtures/schema/v1.json`; the Markdown goldens `fixtures/markdown/*.md` with
  `*.expected.md` and `*.warnings.json` (every row of the §6.4 table, including the lossy ones; the agent harness reads
  them too); and the server-written fixtures `fixtures/ydoc/server/*` (Rust writes them,
  the web test reads them).

### Owns

| Area | Files |
|---|---|
| `crates/wikanvas-api` | `src/docs.rs` |
| `crates/wikanvas-core` | `src/{ytext,schema}.rs`, `src/convert/**` |
| `crates/wikanvas-store` | `src/docs.rs`, `tests/store/docs.rs` |
| `crates/wikanvas-server` | `src/routes/docs.rs`, `tests/api/docs.rs` |
| `web` | `src/pages/DocPage.tsx`, `src/editor/**`, `src/api/queries/docs.ts` |
| `fixtures` | `fixtures/ydoc/**`, `fixtures/markdown/**`, `fixtures/schema/**` |
| `web/src/api/gen` | the generated files for the types in `wikanvas-api` `docs.rs` |

### Produces

- The doc routes and schema v1 (§6.1). Renaming a node is a breaking change.
- Wikanvas Markdown 1 and the ProseMirror JSON mapping (§6.4): the printer's output is a contract (golden files); a
  change to it is an additive API change only when old output still parses to the same document.
- `docs::*` functions, fully implemented, plus `docs::write_content(…, by: &WriteBy, …)` (the §6.4 write path) and
  `docs::etag(…)`.
- The fixtures.

### Consumes

- W3 `TopBar`, `ItemHeader`, `ItemMenu`, `saveStatus`, `pending`, `useItem`, `useCreateItem` and `urls`.
- W1 `/api/v1/search/titles` and `/api/v1/items/{id}/children`; W1's item lookups by `url_id` for same-instance
  links in Markdown.
- W2 `Auth<S>` (W0b version until W2 merges) and `Actor.method`, turned into `WriteBy` for the `pre-api` rule.
- W1 `items::rename` and `revisions::pre_api` (W0b minimal versions until W1 merges).
- W2 `/api/v1/attachments`.
- W6 `boardPreviewUrl` (from `lib/urls.ts`) and `ItemDetail.board`.

### Acceptance

```sh
scripts/dev.sh pnpm --dir web fixtures && git diff --exit-code fixtures/ydoc fixtures/schema
scripts/dev.sh pnpm --dir web fixtures && git diff --exit-code fixtures/ydoc fixtures/schema   # second run: still byte-identical
scripts/dev.sh env WRITE_FIXTURES=1 cargo test -p wikanvas-core --locked -- convert::tests::write_server_fixtures \
  && git diff --exit-code fixtures/ydoc/server
scripts/dev.sh cargo test -p wikanvas-core   --locked -- ytext:: schema:: convert::
scripts/dev.sh cargo test -p wikanvas-store  --locked --test store -- docs::
scripts/dev.sh cargo test -p wikanvas-server --locked --test api   -- docs::
scripts/dev.sh cargo test -p wikanvas-server --locked --test api   -- openapi:: && git diff --exit-code docs/api/openapi.json
scripts/dev.sh pnpm --dir web test src/editor
```

These tests must exist and pass:

| Area | Test |
|---|---|
| ytext | `ytext::tests::golden_fixtures`: every `fixtures/ydoc/*.bin` gives exactly its `.txt` and `.links.json` |
| ytext | `ytext::tests::is_empty_with_editor_paragraph` (one empty paragraph is empty; an image alone is not) |
| Store | `docs::compaction_preserves_state` (250 updates, state vectors and text equal before and after) |
| Store | `docs::missing_deps_409_with_state_vector` |
| Store | `docs::duplicate_update_writes_nothing` (a re-sent insert and a re-sent deletion) |
| Store | `docs::delete_only_update_persists` (select-all delete: stored, text gone after reload) |
| Store | `docs::over_16mib_413` |
| Store | `docs::body_text_links_and_attachments_in_same_txn` |
| Store | `docs::doc_clients_survive_compaction` |
| Store | `docs::read_only_when_trashed_409` |
| API | `docs::two_clients_converge_over_http` |
| API | `docs::since_returns_diff_only` |
| schema | `schema::tests::matches_tiptap_fixture` (`fixtures/schema/v1.json`) |
| schema | `schema::tests::every_mark_excludes_itself` |
| convert | `convert::tests::y_to_raw_matches_web` (every `fixtures/ydoc/*.bin` → its `.y.json`) · `y_to_pm_matches_web` (→ its normalized `.pm.json`) |
| convert | `convert::tests::pm_to_y_to_pm_identity` (proptest over schema-valid normalized documents) |
| convert | `convert::tests::markdown_goldens` (`fixtures/markdown/*.md` → `*.expected.md` and `*.warnings.json`; covers `image_hoisted`, `marks_in_code_dropped`, `hard_break_dropped`, `empty_paragraph_dropped`, adjacent lists, edge whitespace, `unsafe_link`) |
| convert | `convert::tests::markdown_roundtrip` (`parse(print(d)) == md_normalize(d)`) and `markdown_print_idempotent` (proptests, §6.4) |
| convert | `convert::tests::utf16_offsets_with_emoji_and_combining_marks` (a diff inside "a😀é" lands where Yjs puts it) |
| convert | `convert::tests::diff_keeps_unchanged_blocks_identity` (Y item ids and `blockId`s of untouched blocks unchanged; only the edited paragraph's text changes) |
| convert | `convert::tests::diff_insert_top_edit_bottom_keeps_middle_identity` (P1..P12, insert before P2, edit P11: only those two touched) |
| convert | `convert::tests::json_reorder_moves_by_block_id` |
| convert | `convert::tests::markdown_put_leaves_unchanged_blocks_untouched` (an image with width 50 and a table with `colspan` survive a PUT that edits another paragraph) |
| convert | `convert::tests::repeated_put_is_byte_identical_update` (same request twice → same client id, same update bytes) |
| convert | `convert::tests::new_blocks_get_uuid_block_ids` |
| convert | `convert::tests::invalid_json_422_with_pointer` (unknown node, wrong child, wrong attr type, excluded mark pair, `javascript:` link, depth 101) |
| convert | `convert::tests::markdown_prescan_rejects_depth_101` (a 1 MiB line of `>` is rejected before comrak) · `adversarial_markdown_bounded` (long emphasis runs) |
| Store | `docs::markdown_put_is_one_update_through_save_path` (one `doc_updates` row, `item_text` updated, `doc_clients` has the actor) |
| Store | `docs::noop_put_writes_nothing` (`changed: false`, seq unchanged) |
| Store | `docs::merge_true_converges_with_concurrent_update` (base `d{n}`, a web update at `n+1`, a PUT with merge: both edits present, `merged: true`) |
| Store | `docs::merge_after_compaction_412_not_mergeable` |
| Store | `docs::concurrent_strict_puts_one_412` (two PUTs with the same `If-Match`: one 200, one 412) |
| Store | `docs::stale_if_match_equal_target_200_unchanged` (a retried strict PUT) · `docs::retried_merge_put_is_noop` |
| Store | `docs::put_to_unknown_nodes_409_schema_too_new` (nothing deleted) · `docs::api_write_clears_pm_json` |
| Store | `docs::token_write_creates_pre_api_revision` (after a session edit: one; the same token's next PUT: none) · `docs::token_yjs_post_creates_pre_api_revision` |
| API | `docs::markdown_get_etag_and_warnings_header_never_304` · `docs::json_get_304` |
| API | `docs::markdown_put_accepts_text_markdown_but_never_text_plain` (415; moved here from W2) |
| API | `docs::put_holds_two_permits` (during a PUT one Yjs POST proceeds and a second waits; two concurrent PUTs finish, no deadlock) |
| API | `docs::put_without_if_match_428` · `docs::put_stale_if_match_412_with_mergeable` · `docs::put_if_match_star_overwrites` |
| API | `docs::title_from_h1_sets_item_title` |
| API | `docs::open_session_sees_api_write_on_since` (a Yjs client at seq n fetches `?since` and gets the API's update) |
| API | `docs::schema_doc_route_matches_fixture` |
| Web | `editor/docSession.test.ts`: 1 s debounce and 5 s max wait (fake timers); outbox written on every update (throttled) and replayed (fake-indexeddb); `missing_deps` resend; flush on `visibilitychange` hidden; offline backoff sequence |
| Web | `editor/schema.test.ts`: node and mark names equal the §6.1 list |
| Web | `editor/uniqueId.test.ts`: block ids applied with origin `'server'` are never reassigned |
| Web | `editor/exportMarkdown.test.ts`: Export → Markdown downloads `GET …/doc/markdown?download=true` (mocked); no client-side Markdown code |
| Web | `editor/serverWritten.test.ts`: every `fixtures/ydoc/server/*.bin` opens in a headless TipTap editor and `editor.getJSON()` equals its `.pm.json` with no schema repair |
| Web | `editor/contents.test.ts`: ≥2 headings rule, `#h-<blockId>` anchors, scroll-spy threshold at 30% |
| Web | `editor/slash.test.tsx`: filter and the 3-character close |
| Web | `editor/inputRules.test.ts`: `:::info `, `[] `, ```` ``` ````, `==x==` |
| Web | `editor/mention.test.tsx`: live title; trashed shows strikethrough; deleted is not clickable |
| Web | `editor/externalImage.test.tsx`: a pasted remote `<img>` becomes a placeholder and is never fetched |
| Web | `editor/boardEmbed.test.tsx`: renders an `<img>` preview and never loads `/inkboard/` |
| Web | `editor/link.test.ts`: a pasted or typed `javascript:` / `data:` link stays text |
| Web | `editor/upload.test.ts`: images upload as `application/octet-stream`, whatever the file's type |

---

## W5: Inkboard engine (patches, wasm, frame)

**Goal.**
- The patch series of §10.2: 0001 theme, 0002 the `embed` JS API (host mode, `host_load` resets, pause, single mount,
  Present hidden), 0003 optional `net` (droppable), 0004 docs.
- The build pipeline `scripts/inkboard.sh` (§10.5): self-contained `src`, `build`, `stub`, `export [--upstream]`,
  patch-aware `check`, `size`.
- The frame page implementing `bridge.ts` (§10.4): visibility wait, panic backup listeners, `setActive`,
  `inkboard.d.ts`.
- A Playwright frame test.
- Patches that are ready to upstream: notices in each file's comment syntax, a notice-free `--upstream` export, and no
  new `t()` strings.

### Order

1. `src` and `check` on the unpatched base (sets: wasm `web-glow`, native default); record the existing warnings as the
   baseline.
2. Patch 0001.
3. Patch 0002.
4. The frame page.
5. `build`.
6. The frame test.
7. Patch 0003, with `size` run with and without it. If 0003 does not build cleanly within one day, drop it, record why,
   and leave the Join UI hidden through host mode.
8. Patch 0004 (docs).

### Owns

- `patches/inkboard/**`, `scripts/inkboard.sh`
- `web/inkboard-frame/**` (including `inkboard.d.ts` and `tsconfig.json`), `web/src/board/bridge.ts`
- `fixtures/boards/frame-test.inkboard`
- `e2e/tests/frame.spec.ts`, `e2e/serve-static.mjs`

### Produces

- `web/dist-inkboard/` with `frame.html`, `frame.js`, `frame.css`, `inkboard.js`, `inkboard_bg.wasm`, `icons.bin`,
  `ICONS-LICENSES.txt`, `LICENSE-MIT`, `LICENSE-APACHE`, `THIRD_PARTY_LICENSES.md`, `manifest.json`, and `.br`/`.gz`
  variants (never for `icons.bin`). No `tsconfig.json` or `.d.ts` in the dist.
- `window.wikanvasBoardFrame` and `globalThis.inkboardEmbed` (§10.3).
- The `engine_version` string, `0.2.0-beta.3+embed.wikanvas.<patchset>`.

### Consumes

The toolchain from W0b (binaryen, brotli, clang) and nothing else.

### Acceptance

```sh
scripts/dev.sh scripts/inkboard.sh src            # copy + git init + git am: all patches apply without fuzz
scripts/dev.sh scripts/inkboard.sh check          # every feature set of §10.5 for the applied patches: check + clippy, no new warnings
scripts/dev.sh bash -c 'cd .cache/inkboard-src && CARGO_TARGET_DIR=/work/.cache/inkboard-target cargo test --locked -p inkboard-app i18n'
                                                  # upstream i18n tests still pass; if native test deps cannot link in the toolchain image, record it in the handoff note (W7 decides)
scripts/dev.sh scripts/inkboard.sh build && test -s web/dist-inkboard/inkboard_bg.wasm.br && test ! -e web/dist-inkboard/icons.bin.gz && test ! -e web/dist-inkboard/tsconfig.json
scripts/dev.sh scripts/inkboard.sh size           # numbers recorded in patches/inkboard/README.md
scripts/dev.sh scripts/inkboard.sh export --upstream && ! grep -rq 'Changed for Wikanvas' .cache/inkboard-upstream
scripts/dev.sh pnpm --dir web typecheck           # frame.js (// @ts-check) against bridge.ts and inkboard.d.ts
scripts/e2e.sh frame
git -C vendor/inkboard status --porcelain | wc -l | grep -qx 0
```

`e2e/tests/frame.spec.ts` must cover every item in the §14 "Inkboard patch" row, plus two more checks:
- `onProgress` reaches `loaded == total`;
- after `open()`, `document.title` of the frame is unchanged.

---

## W6: boards (full stack, without the engine)

**Goal.** Boards work end to end.
- **Server:**
  - `wikanvas-core::board` split, splice, inspect and preview (§6.2): raw-JSON splicing, no `Value` trees.
  - The board store: the PUT pipeline with the `board_work` permit, `BlobStore::stage` outside and `commit` inside the
    writer, the no-op hash, If-Match/412/428, meta, text, revisions with `assets`; GET with splice and title under the
    same permit; the `.inkboard` download.
  - The preview worker and the per-page route `…/board/pages/{n}` (`image/svg+xml`, no `.svg` suffix; §11 headers;
    no `rev`: the current page with `no-cache`; 307 for a stale rev; 204 for an empty page).
  - The board API rules (D51, D53): `If-Match: *` and `W/` tags, `X-Wikanvas-Engine` optional (stored as `"api"`),
    `{rev, etag, updated_at, meta}` responses, 412 `details {rev, etag}`, the free-space check before staging,
    `revisions::pre_api` and `last_token_id` for token-authenticated PUTs, and OpenAPI annotations for every W6 route.
  - The real `fixtures/boards/getting-started.inkboard` (§9.6.7).
- **Client:**
  - `BoardFrameHost` (replacing W0b's placeholder): one persistent iframe, rect tracking, hidden + inert +
    `setActive(false)` off board routes, removed after 10 minutes, cache warm-up with `fetch(…, {cache:'force-cache'})`
    on hover, the update banner when `frame.html` returns 404, preview mode when `config.inkboard` is null.
  - `BoardPage`: the title crumb, star, Nested popover, Focus (fullscreen) button.
  - `BoardLoading`: the dimmed SVG and the progress card.
  - The save loop (§9.5), `ConflictBanner`, the IndexedDB unsaved snapshot and restore prompt, the >15 MB warning.
  - `BoardPreview`, the read-only fallback, which is also the phone default.
  - Theme sync and host-key forwarding to `runHostShortcut`.
  - Until W5 lands, a fake frame (`src/board/fakeFrame.ts`) implements `BoardFrameApi`, and tests run against it.

### Owns

| Area | Files |
|---|---|
| `crates/wikanvas-api` | `src/boards.rs` |
| `crates/wikanvas-core` | `src/board.rs` |
| `crates/wikanvas-store` | `src/{boards,previews}.rs`, `tests/store/boards.rs` |
| `crates/wikanvas-server` | `src/routes/boards.rs`, `src/board_previews.rs`, `tests/api/boards.rs` |
| `web` | `src/pages/BoardPage.tsx`, `src/board/**` except `bridge.ts`, `src/api/queries/boards.ts` |
| `fixtures` | `fixtures/boards/**` except `frame-test.inkboard` |
| `web/src/api/gen` | the generated files for the types in `wikanvas-api` `boards.rs` |

### Produces

- The board routes, `BoardMeta`, `boards::*`, and `board_previews::spawn`.
- The `BoardFrameHost` component and its context `useBoardFrame()`.

### Consumes

- `bridge.ts` and the dist from W5.
- `BlobStore` from W0b/W2.
- W3 `TopBar`, `ItemHeader`/`StarToggle`, `saveStatus`, `pending`, `runHostShortcut`, `useItem`, `useCreateItem`
  (for "Keep mine as a copy"), and `urls`.

### Acceptance

```sh
scripts/dev.sh cargo test -p wikanvas-core   --locked -- board::
scripts/dev.sh cargo test -p wikanvas-store  --locked --test store -- boards::
scripts/dev.sh cargo test -p wikanvas-server --locked --test api   -- boards::
scripts/dev.sh cargo test -p wikanvas-server --locked --test api   -- openapi:: && git diff --exit-code docs/api/openapi.json
scripts/dev.sh pnpm --dir web test src/board
```

These tests must exist and pass:

| Area | Test |
|---|---|
| board | `board::tests::split_splice_roundtrip_byte_identical` (a fixture with two images and a shared asset) |
| board | `board::tests::stripped_keeps_asset_entries_with_empty_data` |
| board | `board::tests::unknown_fields_survive_put_get` (an extra `"x_future":1` on an object and on the document) |
| board | `board::tests::version_3_rejected_board_too_new` |
| board | `board::tests::tableau_blanc_accepted` |
| board | `board::tests::inspect_text_includes_labels_tables_pages_latex` |
| Store | `boards::assets_deduped_across_boards` |
| Store | `boards::noop_save_keeps_rev_and_drops_staged_files` |
| Store | `boards::copy_shares_blobs` |
| Store | `boards::save_sets_body_text_with_box_label` |
| API | `boards::missing_if_match_428` |
| API | `boards::stale_if_match_412_with_rev` |
| API | `boards::get_sets_document_name_to_title` |
| API | `boards::over_limit_413` |
| API | `boards::preview_route_headers_immutable_307_and_current_without_rev` |
| API | `boards::inkboard_download_disposition` |
| API | `boards::if_match_star_overwrites` |
| API | `boards::put_without_engine_header_stores_api` |
| API | `boards::token_put_writes_pre_api_revision` (after a session save: one; the same token's next PUT: none) |
| API | `boards::token_put_get_preserves_unknown_fields` (the `x_future` fixture through a bearer client) |
| Web | `board/saveLoop.test.ts`: 1.5 s quiet and 10 s max (fake timers); `markSaved` with the snapshot revision; flush before switching boards; flush on hidden and on `onSave` |
| Web | `board/conflict.test.tsx`: 412 shows the banner; both actions work |
| Web | `board/unsaved.test.ts`: snapshot written before PUT, cleared on 200, restore prompt when newer (fake-indexeddb) |
| Web | `board/fallback.test.tsx`: webgl error, panic, `config.inkboard === null`, and coarse pointer under 768 px |
| Web | `board/staleEvents.test.ts`: events for a non-current `itemId` are ignored |
| Web | `board/frameLifecycle.test.tsx`: one iframe across two boards; hidden, inert and `setActive(false)` off route; removed after 10 minutes |
| Web | `board/warmup.test.ts`: hover warms with `fetch(…, {cache:'force-cache'})`; no `<link rel=prefetch>` |

---

## W8: CLI and agent harness (parallel phase)

**Goal.**
- The `wikanvas` CLI of §7.6 and §7.7, complete: every command of the §7.6 table with exactly its flags and `--json`
  output, the exit-code table, the URL and token resolved as a pair, the profile file (0700 directory, 0600 atomic
  write, refusal when group-readable with a token), the offline token check, references (UUID, `url_id` or URL),
  `--all` with the truncation rule, the retry rule (only unprocessed requests), no redirects, timeouts, TLS roots from
  the system bundle or `--ca-file`, plain HTTP refused for non-loopback hosts, `--if-match` (`d42`, `"d42"`, `W/"d42"`)
  or `--force` on content writes, `--yes` on `trash purge`, `item create --md` (title only after the PUT, discard on
  failure), `doc put --attach-images` with its containment rules, the `api` path and header guards, `--token-out`,
  `--debug`, the 204 ok object, JSON Lines on stderr with `details.sent`, the `admin` passthrough through
  `WIKANVAS_SERVER_BIN`, `version`, `exit-codes` and `help --json`.
- `scripts/cli-dist.sh` (§13.2): the static musl binary in `dist/cli/` with its sha256 file, `LICENSE` and
  `THIRD_PARTY.md`.
- The CLI test binary `crates/wikanvas-cli/tests/cli/` (modules below), against `wikanvas_server::testing::TestServer`
  and a scripted mock HTTP listener written in the test (std `TcpListener`, no extra crate).
- The agent harness of §14.1: `scripts/agent-test.sh` (`run`, `up --ttl` with the 15-minute default, `status`, `down`,
  the lock holder, `wikanvas_slice`, the project subnet `172.31.240.32/28` with range `172.31.240.40/29`,
  `--env-file /dev/null` with the owner-facing variables unset (§13.3), `up -d --wait wikanvas` so that Caddy never
  starts and the CLI talks to Wikanvas directly, the stale-image check, its exit codes), `e2e/agent/lib.sh`,
  `e2e/agent/scenarios/*.sh`, `e2e/agent/fixtures/**`, `compose.agent.yaml` (§13.3; a plain `ports:` list, since the
  base file publishes nothing for `wikanvas`).
- `docs/AGENTS.md`, short, with no secrets and no real hostnames, in this order:
  1. what the CLI is and where it runs (in the image, or the standalone binary);
  2. a cheat sheet: one line per command group of §7.6 with its main flags and what `--json` prints (items are
     published on create; `item unpublish` makes a draft, D23);
  3. getting a token (Settings → API tokens, or `docker compose exec wikanvas wikanvas admin create-token --name …
     --scope write --expires 1d --token-out /tmp/agent.token`, or `umask 077; wikanvas token create … --json | jq -r
     .token > file`) and using it through `WIKANVAS_TOKEN_FILE` or `wikanvas auth login --token-stdin`, never argv or
     shell history; `WIKANVAS_URL`;
  4. always `--json`: stdout is one JSON value, stderr is JSON Lines whose last line is the error;
  5. the §7.7 exit-code table with the action for every code 0–12, including exit 4 with `details.sent: true` after a
     create (list before retrying) and exit 3 after a redirect (fix the URL);
  6. one complete transcript: scratch collection → doc from Markdown → `doc get` (keep the ETag) → edit → a stale PUT
     exits 9 → `--merge` → search → star → trash → restore;
  7. the Markdown dialect in five lines with a pointer to ARCHITECTURE §6.4 for the lossy constructs, and `wikanvas doc
     schema` for JSON;
  8. boards as native JSON: keep unknown fields; Inkboard's model is in `vendor/inkboard/docs/ARCHITECTURE.md`
     ("Model") and `crates/inkboard-core/src/{model,serial}.rs`; no import from other formats in M1 (D52);
  9. running `scripts/agent-test.sh` (`--scenario`, `up --ttl`, `status`, `down`, the report file, its exit codes);
  10. the rules for agents: builds only through `scripts/dev.sh`, never print or commit a token, work in a scratch
      collection and trash what you create, never purge outside the harness.

### Owns

| Area | Files |
|---|---|
| `crates/wikanvas-cli` | everything: `Cargo.toml`, `src/**`, `tests/cli/**` |
| `crates/wikanvas-api` | `src/cli.rs` (the exit-code table) |
| Scripts | `scripts/agent-test.sh`, `scripts/cli-dist.sh` |
| Harness | `e2e/agent/**`, `compose.agent.yaml` |
| Docs | `docs/AGENTS.md` |

### Produces

- The CLI command tree of §7.6, its JSON output and its exit codes (§7.7): a contract for agents, CI and W7's gate.
- The harness interface: `scripts/agent-test.sh [run|up|status|down] [--scenario GLOB] [--keep] [--ttl MIN]
  [--allow-stale]` with exit codes 0, 1, 3 and 4, the variables `WIKANVAS_IMAGE`, `WIKANVAS_AGENT_CLI` and
  `WIKANVAS_AGENT_PORT`, the helpers of `e2e/agent/lib.sh` (`q`, `q_raw`, `expect_rc`, `expect`, `expect_err`, `ok`,
  `fixture`, `scratch_collection`; `$OUT_FILE`, `$OUT`, `$ERR`, `$RC`), and `e2e/agent/results/report.json` (tokens
  masked).

### Consumes

- `wikanvas-api` DTOs, `ErrorBody`, `Page<T>` and `token::check` (W0b; files owned by W1, W2, W4 and W6).
- `wikanvas_server::testing::TestServer`, bearer authentication and the W0b minimal routes `GET /auth/me`, `/tokens*`
  and `POST /collections` (W0b minimal, W1 and W2 full).
- `wikanvas-server admin create-owner` and `admin create-token` (W0b minimal with the final flags, W2 full).
- The W1, W2, W4 and W6 routes, over HTTP only. The `flows::` tests and scenarios 02–11 pass once those workstreams
  have merged; W7 runs them in the gate.
- The image from `scripts/image.sh` (the Dockerfile `cli` stages and the revision label from W0b).

### Order

1. Configuration, transport, output and exit codes, against the mock listener.
2. `auth`, `token`, `version`, `exit-codes`, `help`, `api` and `admin`, against `TestServer`.
3. The remaining commands with their `flows::` tests, and the `coverage::` tests.
4. `scripts/cli-dist.sh`.
5. The harness with scenario 01, then 02–11.
6. `docs/AGENTS.md`.

### Acceptance (during the parallel phase)

```sh
scripts/dev.sh cargo test -p wikanvas-cli --locked --test cli -- config:: transport:: output:: exit_codes:: api:: auth:: tokens:: admin:: coverage::
scripts/dev.sh cargo clippy -p wikanvas-cli --all-targets --locked -- -D warnings
! scripts/dev.sh cargo tree --locked -p wikanvas-cli -e normal --prefix none | grep -qE '^(openssl|openssl-sys|native-tls|rusqlite|yrs|inkboard-core|comrak|webpki-roots) '
for f in scripts/agent-test.sh scripts/cli-dist.sh e2e/agent/lib.sh e2e/agent/scenarios/*.sh; do
  bash -n "$f" || exit 1
done
scripts/cli-dist.sh && scripts/dev.sh bash -c 'f=dist/cli/wikanvas-x86_64-unknown-linux-musl \
  && { ldd "$f" 2>&1 | grep -qE "not a dynamic executable|statically linked"; } && "$f" --version \
  && (cd dist/cli && sha256sum -c wikanvas-x86_64-unknown-linux-musl.sha256) && test -s dist/cli/THIRD_PARTY.md'
scripts/image.sh --build-arg INKBOARD_BUILD=stub && scripts/agent-test.sh --scenario '01-*'
scripts/agent-test.sh up --ttl 5 && test -s .cache/agent-test/env && scripts/agent-test.sh status \
  && scripts/agent-test.sh down && test ! -e .cache/agent-test/env
```

`coverage::every_spec_operation_has_command_or_api_only_allowlist` reads the committed `docs/api/openapi.json`, so in
the parallel phase it covers the operations merged so far; W7's gate runs it on the full spec.

These tests must exist and pass (the `flows::` ones in W7's gate):

| Module | Test |
|---|---|
| config | `config::url_and_token_resolve_as_a_pair` (every row of §7.6 "Configuration"; an explicit `--profile` beats `WIKANVAS_URL`/`WIKANVAS_TOKEN`) |
| config | `config::profile_token_never_sent_to_overridden_url` (`WIKANVAS_URL` elsewhere: exit 3, the mock listener sees no `Authorization`) |
| config | `config::group_readable_profile_with_token_exit_3` · `config::group_readable_token_file_warns` |
| config | `config::malformed_token_exit_3_without_request` (the mock listener records no connection) |
| config | `config::login_writes_profile_0600_in_0700_dir_atomically` |
| config | `config::token_stdin_with_token_file_exit_2` |
| transport | `transport::retries_only_unprocessed_requests` (answered 429/503 and connect failures for every method; a timeout retried for GET only; a PUT and a POST whose response is lost are not retried and exit 4 with `details.sent: true`; `Retry-After` honoured) |
| transport | `transport::connection_refused_exit_4_sent_false` · `transport::timeout_exit_4` |
| transport | `transport::plain_http_refused_for_non_loopback` (`WIKANVAS_ALLOW_HTTP=1` and `allow_http` allow it; loopback always allowed) |
| transport | `transport::redirect_never_followed_exit_3` (the mock's 302 `Location` is in the error; no second request) |
| transport | `transport::https_without_ca_bundle_exit_3` (`SSL_CERT_FILE` pointing at a missing file, no `--ca-file`) |
| output | `output::every_command_accepts_json` (walks the clap command tree; stdout parses as exactly one JSON value) |
| output | `output::json_stdout_is_the_body_unchanged` (an unknown field from the mock survives) |
| output | `output::no_content_prints_ok_object` (a 204 prints `{"ok": true, "status": 204, …}`) |
| output | `output::json_error_on_stderr_and_empty_stdout` · `output::json_stderr_is_json_lines` (a warning line and an error both parse) |
| output | `output::usage_error_is_json_when_flag_present` (exit 2, `cli_usage`) |
| output | `output::content_write_without_if_match_or_force_exit_2` · `output::if_match_accepts_bare_quoted_and_weak` |
| output | `output::never_prompts_with_closed_stdin` |
| output | `output::debug_never_prints_authorization` · `output::help_json_lists_every_command` |
| output | `output::all_truncation_keeps_next_cursor` (mock pages past 10,000 items: real `next_cursor`, a `truncated` warning) |
| exit_codes | `exit_codes::every_status_maps_to_the_table` (mock: 302, 400, 401, 403, 404, 405, 408, 409, 412, 413, 415, 418, 421, 422, 428, 429, 500, 503, 507, 599) |
| exit_codes | `exit_codes::table_matches_agents_md` (parses the table in `docs/AGENTS.md`) |
| api | `api::path_and_header_guards` (`//host/x`, `https://host/x`, `/api/../x`, `/api/%2e%2e/x`, `--header Authorization:…`, `Cookie`, `Host`: exit 2, the mock sees nothing) |
| auth | `auth::whoami_shows_token_scopes` |
| auth | `auth::unknown_token_exit_5` · `auth::read_token_write_command_exit_6` |
| tokens | `tokens::create_prints_secret_once_and_revoke_works` · `tokens::token_out_writes_0600_and_keeps_secret_off_stdout` |
| tokens | `tokens::logout_revoke_revokes_current_token` |
| admin | `admin::passthrough_execs_wikanvas_server_with_args_and_exit_code` (a stub through `WIKANVAS_SERVER_BIN`) |
| admin | `admin::missing_wikanvas_server_exit_3` (`WIKANVAS_SERVER_BIN` unset, no sibling binary, empty `PATH`) |
| coverage | `coverage::every_spec_operation_has_command_or_api_only_allowlist` (the §7.6 allow-list) |
| coverage | `coverage::agents_md_examples_parse` (every `wikanvas …` line in `docs/AGENTS.md` parses with the clap tree) |
| flows | `flows::collection_tree_move_and_cycle_exit_8` |
| flows | `flows::doc_markdown_roundtrip_and_stale_etag_exit_9` |
| flows | `flows::doc_put_merge_after_concurrent_update` |
| flows | `flows::item_create_md_discards_item_when_put_fails` · `flows::item_create_md_sets_title_only_after_put` |
| flows | `flows::attach_images_refuses_paths_outside_base` (`../`, an absolute path, a symlink out, a non-image: skipped with `attach_skipped`) |
| flows | `flows::board_put_get_preserves_unknown_fields` |
| flows | `flows::search_all_merges_pages` |
| flows | `flows::attachment_upload_get_same_sha256` |
| flows | `flows::trash_purge_needs_yes_and_admin` |
| flows | `flows::export_to_stdout_is_a_tar` |

Agent scenarios (`e2e/agent/scenarios/`; each step is one `q` or `q_raw` call plus assertions; fixtures go in on
stdin):

| Scenario | Steps |
|---|---|
| `01-auth.sh` | `auth whoami` with each token shows its scopes; a read token creating a collection exits 6 with `insufficient_scope`; a well-formed unknown token exits 5; a malformed token exits 3; a token revoked with `token revoke` exits 5 |
| `02-tree.sh` | Create a collection, then doc A, B under A, C under B; `tree --json` has three items with the right parents; move B to the top level and back; moving A under C exits 8 with `cycle`; duplicate C |
| `03-docs.sh` | For every `fixtures/markdown/*.md` (W4's goldens): `item create --md -`, then `doc get --format md` equals its `.expected.md` and the PUT's warning codes equal its `.warnings.json`, after the goldens' items ("Doc A", "Doc B", "Board C", one attachment) are created and their ids and origin mapped to the live ones; `doc get --format json` has `schema_version` 1; a PUT with an ETag from before a second PUT exits 9; the same PUT with `--merge` succeeds with `merged: true` and both edits present; repeating that command gives `changed: false`; `--title-h1` sets the title |
| `04-boards.sh` | `board put -` of `fixtures/future.inkboard` (`x_future` fields) with `--if-match r1`; `board get` keeps both `x_future` values and `document.name` equals the title; `board preview --page 0 --out /tmp/agent-p0.svg` gives `content_type` `image/svg+xml` (bounded poll for the preview); a stale `--if-match` exits 9; `--force` succeeds |
| `05-search.sh` | After a PUT containing "quokka", `search quok` lists that doc first; `search "quok -zebra"` still finds it; `search --titles` finds it; `--all` merges pages |
| `06-stars-lists.sh` | `star add` prints the ok object; `list starred`; a new doc is published on create (`item get` has `effectively_published: true`; `sidebar`'s `drafts_count` is unchanged and `list drafts` does not list it); `item unpublish` on it and on its parent puts both in `list drafts` and raises `drafts_count` by 2; `item publish --with-ancestors` on the child empties `list drafts` again |
| `07-trash.sh` | Trash B: `tree` lacks B and C; `trash list` shows B with one descendant; `item restore` brings both back; `trash purge B` without `--yes` exits 2, with the write token exits 6, with the admin token and `--yes` succeeds (its own item only, never `--all`) |
| `08-attachments.sh` | `attachment upload -` of `fixtures/pixel.png`; a Markdown PUT that references `/api/v1/attachments/<id>` round-trips; `q_raw` of `attachment get` has the same sha256 |
| `09-export.sh` | `q_raw export.tar export` is a tar with `manifest.json` and `wikanvas.db`; with the write token it exits 6 |
| `10-tokens.sh` | `token create --scope read --expires 1d` prints `qpat_…` once and the report shows it masked; `--token-out /tmp/agent-t.token` prints no secret; `token list` shows the prefix and never the secret; `token revoke`, then reuse exits 5; the admin token minting an `admin` token exits 6; a child of a second admin token (from `wikanvas admin create-token`) exits 5 once that parent is revoked |
| `11-openapi.sh` | `api GET /api/v1/openapi.json`: `openapi == "3.1.0"`, every path starts with `/api/v1/`, `components.securitySchemes.bearer` exists; `api GET /api/v1/nope` exits 7; `api GET //example.invalid/x` exits 2 |

---

## W7: integration, production image, end-to-end smoke (last)

**Goal.**
- Merge W1–W6 and W8 (lockfile branches one at a time, R6; regenerate `docs/api/openapi.json` after each merge that
  touches routes, R4) and apply the handoff requests.
- Remove all stubs, and confirm the startup format check.
- Write and pass the cross-workstream tests in `tests/api/integration.rs`.
- Build the full production image and pass the full image smoke, the Playwright smoke, the full CLI suite (including
  `flows::`) and the full agent harness.
- Finish `README.md`:
  - install with `scripts/image.sh` and `docker compose up -d`, then open `https://localhost` (or the configured
    domain) after trusting the local CA;
  - the setup code (`docker compose logs wikanvas`);
  - the runtime limits and how to change them (app `WIKANVAS_CPUS=2`, `WIKANVAS_MEM=1g`; Caddy
    `WIKANVAS_CADDY_CPUS=0.5`, `WIKANVAS_CADDY_MEM=128m`; no swap); `WIKANVAS_ALLOWED_HOSTS`;
  - HTTPS through the bundled Caddy (ARCHITECTURE §13.4): LAN or local mode (`WIKANVAS_TLS=internal`, `WIKANVAS_DOMAIN`,
    `WIKANVAS_BIND`) with the steps to trust the local CA (system bundle, Chrome/Chromium NSS, Firefox, other devices,
    the CLI's `--ca-file`) and to keep `root.key` secret; public mode (`WIKANVAS_TLS=acme`, a public `WIKANVAS_DOMAIN`,
    `WIKANVAS_BIND=0.0.0.0`, DNS and ports 80/443 reachable); `WIKANVAS_HTTP_PORT`/`WIKANVAS_HTTPS_PORT` when 80 or 443
    are taken; `WIKANVAS_NET_SUBNET`, `WIKANVAS_NET_RANGE` and `WIKANVAS_PROXY_IP` together if the subnet collides (the
    proxy address inside the subnet, outside the range); `docker compose down -v` deletes `caddy-data`, and with it the
    local CA every device trusts; HSTS only in public mode; back up
    `caddy-data`;
  - backups and restore, export restore needing `reset-password`, the need for an off-host copy, bind-mount ownership;
  - `wikanvas admin reset-password`, `create-owner` and `create-token`; the compromise steps
    (`wikanvas admin reset-password --email … --revoke-tokens`, or `revoke-token --all --yes`);
  - compose sets `WIKANVAS_TRUSTED_PROXIES` to Caddy's fixed address, so only Caddy's `X-Forwarded-For` and
    `X-Forwarded-Proto` count; a deployment that swaps Caddy for another proxy must set it (every per-IP limit uses
    it), and a proxy that adds Basic auth or its own `Bearer` header keeps working (D43);
  - the public API: `/api/v1`, personal access tokens and scopes, the OpenAPI file (`/api/v1/openapi.json`,
    `docs/api/openapi.json`), `WIKANVAS_TOKEN_RATE`, `WIKANVAS_TOKEN_BYTES_PER_HOUR`, `WIKANVAS_MIN_FREE_MB`,
    `WIKANVAS_CORS_ORIGINS`, and `WIKANVAS_ALLOWED_HOSTS` for clients on a Docker network;
  - the CLI: `docker compose exec wikanvas wikanvas …` and the standalone binary from `scripts/cli-dist.sh`, with a
    pointer to `docs/AGENTS.md`;
  - a note that builds never run on the host, and the build slice: installed once (`scripts/install-build-slice.sh`,
    a root step), checked with `--check`, and optional for anyone else building Wikanvas (scripts warn and keep the
    per-container caps).
- Build `THIRD_PARTY.md` and the About page's licence data: a build-time `licenses.txt` from `cargo metadata` and
  `pnpm licenses list --json`, plus Inkboard's `THIRD_PARTY_LICENSES.md` and `ICONS-LICENSES.txt`. comrak is
  BSD-2-Clause; any code W4 ported from `@tiptap/y-tiptap` (MIT) gets its entry. `THIRD_PARTY.md` covers every crate
  in the workspace lock, so the `cli-dist` stage ships it, with `LICENSE`, next to the standalone binary (§13.2).
- Update `docs/ARCHITECTURE.md` with every deviation recorded in the handoff notes.

### Owns (from the start of the parallel phase; W7 may draft early, but runs its gate last)

| Area | Files |
|---|---|
| Rust workspace | `Cargo.toml`, `rust-toolchain.toml`, `.cargo/config.toml`; `Cargo.lock` at merge time (R6) |
| Build and deploy | `docker/**` (including `Caddyfile`, `wikanvas.slice`, `wikanvas-build.slice`), `compose.yaml`, `compose.e2e.yaml`, `compose.smoke.yaml`, `.dockerignore`, `.gitignore` |
| Scripts | `scripts/{dev,toolchain,image,image-smoke,e2e,fmt,check-pins,install-build-slice}.sh`, `scripts/lib/**` |
| e2e | `e2e/package.json`, `e2e/pnpm-lock.yaml`, `e2e/playwright.config.ts`, `e2e/tests/smoke.spec.ts` |
| Docs and licences | `README.md`, `THIRD_PARTY.md`, `LICENSE`, `CLAUDE.md`, `docs/ARCHITECTURE.md`, `docs/PLAN.md`; `docs/api/openapi.json` (generated: every route owner regenerates it, R4) |
| Frozen module lists | `crates/wikanvas-api/src/lib.rs`, `crates/wikanvas-core/src/lib.rs`, `crates/wikanvas-store/src/lib.rs`, `crates/wikanvas-server/src/lib.rs`, `crates/wikanvas-server/src/routes/mod.rs` |
| Test binaries | `crates/wikanvas-store/tests/store/main.rs`, `crates/wikanvas-server/tests/api/main.rs`, `crates/wikanvas-server/tests/api/integration.rs` |

### Integration tests (`tests/api/integration.rs`)

| Test | Spans |
|---|---|
| `integration::board_blobs_freed_after_purge_and_gc` | W1 purge, W2 GC, W6 board assets |
| `integration::search_finds_board_label_over_http` | W1 search, W6 board text |
| `integration::seeded_getting_started_is_published_and_searchable` | W2 seed, W1 search, W4/W6 fixtures |
| `integration::doc_image_attachment_freed_after_removal` | W4 `item_attachments`, W2 attachment GC (24 h grace via injected clock) |
| `integration::markdown_put_with_token_is_searchable` | W2 tokens, W4 conversion, W1 search |
| `integration::markdown_attachment_reference_keeps_attachment_alive` | W4 Markdown image resolution, W2 attachment GC |
| `integration::board_token_put_then_text_and_preview` | W2 tokens, W6 board PUT and previews, W1 `/items/{id}/text` |
| `integration::export_after_api_writes_has_no_tokens_or_sessions` | W2 export and tokens, W4/W6 writes |

### Smoke spec (`e2e/tests/smoke.spec.ts`, against `http://wikanvas:8080` in the compose e2e project)

Playwright reaches Wikanvas directly over plain HTTP on the project network; Caddy is not started (ARCHITECTURE §14).
The HTTPS path through Caddy is covered by `WIKANVAS_SMOKE_LEVEL=full scripts/image-smoke.sh`.

1. `/` redirects to `/setup`. Submit the code `e2e-setup-code` with the workspace and owner. Home shows
   "Getting started".
2. Sign out, then sign in with "Keep me signed in". The cookie has a Max-Age.
3. Create the collection "Eng". Create the document A, then B as a child of A, then C as a child of B (depth 3). In C,
   type a heading "Plan" and the body "quokka". The status shows "Saved". Reload: the text is still there, the sidebar
   tree is expanded to C with C highlighted, and the breadcrumbs read `Eng / A / B`.
4. In C, select all body text and delete it; the status shows "Saved". Reload: the body is empty. Undo is not
   required; type "quokka" again and wait for "Saved".
5. Search for "quok": C is the first result, and its snippet marks the hit. `quok -zebra` still finds C. Mod+K, then
   "C", then Enter opens C.
6. Star C; it appears in STARRED. A, B and C were published on create: `GET /api/v1/items/{C}` has
   `effectively_published: true`, the top bar has no Publish button and the sidebar has no Drafts entry.
7. Move B, with its subtree, to "Getting started" through the Move dialog. C's breadcrumbs update.
8. Trash B: B and C leave the tree, and Trash shows "+1 nested". Restore: both are back in the same position.
9. Create a board under A. The iframe engine reports ready (or, if WebGL is unavailable, the test asserts that the
   fallback banner appears and skips steps 10–11 with a recorded reason).
10. Click the canvas, press `R` and drag. The status shows "Saved", and `GET /api/v1/items/{id}` has `board.objects == 1`.
11. Reload: the object is still there. `GET …/board/pages/0?rev=…` returns `image/svg+xml`. The board embed card in
    A (inserted through slash → "Embed board…") shows the preview `<img>`, and no `/inkboard/` request is made from the
    document page.
12. Open the same board in a second page, change it in both, and save. The second save shows the conflict banner.
13. `GET /api/v1/export` returns a tar containing `manifest.json`.
14. Settings → API tokens: create a read token; the secret is shown once. `GET /api/v1/auth/me` with
    `Authorization: Bearer <token>` (Playwright `request`, no cookie) returns `auth.method == "token"`; a POST with it
    gets 403 `insufficient_scope`. Revoke it in the UI; the next request gets 401.
15. Document ⋯ → Export → Markdown downloads a `.md` file whose first heading is "Plan" (from C, step 3).
16. (Added at the W7 review, `docs/handoff/W7.md`.) Home's Recently viewed lists C. A document holding an item link to
    C (written as an agent would, `wikanvas://item/<uuid>`) renders it with `href="/item/<uuid>"`; a plain click keeps
    the page, Ctrl+click opens C, and Back returns. The Welcome document's board embed card loads its preview. A row
    menu opened near the bottom of the window opens above the row, with Delete inside the window.

### Acceptance (the M1 release gate)

```sh
scripts/install-build-slice.sh --check            # the owner's machine: the slice is installed and holds its values
scripts/check-pins.sh
docker compose -f compose.yaml config --format json | jq -e '(.services.wikanvas.ports // []) == []'
! grep -qE '^\s*(encode|header)\b' docker/Caddyfile
scripts/fmt.sh --check
scripts/dev.sh cargo clippy --workspace --all-targets --locked -- -D warnings
scripts/dev.sh cargo tree --locked -e normal -i libsqlite3-sys
scripts/dev.sh cargo test --workspace --locked
scripts/dev.sh cargo test -p wikanvas-api --locked export_bindings && git diff --exit-code web/src/api/gen
scripts/dev.sh cargo test -p wikanvas-server --locked --test api -- openapi:: && git diff --exit-code docs/api/openapi.json
scripts/dev.sh cargo test -p wikanvas-cli --locked                     # every module, including flows::
! scripts/dev.sh cargo tree --locked -p wikanvas-server -e normal --prefix none | grep -qE '^(rustls|ureq|reqwest|native-tls|openssl) '
! scripts/dev.sh cargo tree --locked -p wikanvas-cli -e normal --prefix none | grep -qE '^(openssl|openssl-sys|native-tls|rusqlite|yrs|inkboard-core|comrak|webpki-roots) '
! grep -rn '\.route(' crates/wikanvas-server/src/routes
! grep -rn '\.route("/api' crates/wikanvas-server/src
scripts/dev.sh pnpm --dir web install --frozen-lockfile
scripts/dev.sh pnpm --dir web fixtures && git diff --exit-code fixtures
scripts/dev.sh env WRITE_FIXTURES=1 cargo test -p wikanvas-core --locked -- convert::tests::write_server_fixtures && git diff --exit-code fixtures/ydoc/server
scripts/dev.sh pnpm --dir web typecheck && scripts/dev.sh pnpm --dir web test && scripts/dev.sh pnpm --dir web build
scripts/dev.sh scripts/inkboard.sh check && scripts/dev.sh scripts/inkboard.sh build && scripts/e2e.sh frame
scripts/image.sh
WIKANVAS_SMOKE_LEVEL=full scripts/image-smoke.sh     # app 2 CPUs / 1 GiB, Caddy 0.5 CPU / 128 MiB, no swap; no published app port;
                                                  # HTTPS via Caddy with the local CA; br wasm passed through byte for byte;
                                                  # __Host- Secure cookie; no HSTS in internal mode; CSP; peak memory < limits
scripts/e2e.sh smoke
scripts/agent-test.sh                              # every scenario, against the image just built (no --allow-stale)
scripts/cli-dist.sh && scripts/dev.sh dist/cli/wikanvas-x86_64-unknown-linux-musl --version
! grep -rnE 'todo!\(|unimplemented!\(|TODO\(W|not implemented \(W' crates web/src web/inkboard-frame scripts e2e/agent
git -C vendor/inkboard status --porcelain | wc -l | grep -qx 0
```

---

## M1.1: board document embeds

**Goal.** The owner's request of 2026-10-28, "I want text docs to be able to be embedded and editable inside a board",
as designed in `docs/ARCHITECTURE.md` §18 (normative; D58–D75). A card on a board shows a real document and edits it
in place: a click selects, moves and resizes it, a double-click puts the caret in its text, Esc or a click outside
leaves editing; a card is added with "Text doc" (a new child of the board), "Embed page…" (a picker), or by dragging a
document from the sidebar onto the canvas.

**Frozen contracts**, committed on main with this section (R3: additive changes only, by the file's owner, recorded
in its handoff note; anything else through the orchestrator):
- the stored representation of a card, ARCHITECTURE §18.2, and the fixture `fixtures/boards/embed.inkboard` (no
  workstream edits it; tests build their variants from it in code);
- the bridge types, `web/src/board/bridge.ts` "board document embeds" (§18.4: the `FrameEvents` and `BoardFrameApi`
  additions, optional, and `DragItemType`, `DraggedItem`, `FramePoint`, `FrameRect`, `EmbedBox`, `EmbedLayout`,
  `EmbedOpenEvent`, `EmbedEditEndReason`, `EmbedEditEndEvent`, `DropItemEvent`, `InsertEmbedArgs`);
- the engine types, `web/inkboard-frame/inkboard.d.ts` "Patch 0005" (§18.3).

**Rules.** R1–R11 apply as written, with these M1.1 specifics:
- R2 ownership is the "Owns" lists below; they do not overlap, so E1, E2 and E3 run in parallel from this commit.
  Handoff notes: `docs/handoff/E1.md`, `E2.md`, `E3.md` (each owned by its workstream), and `docs/handoff/E-merge.md`.
- R11 worktrees and branches, created on the host from main:
  ```sh
  git worktree add ../Wikanvas-E1-engine -b ws/E1-engine main
  git worktree add ../Wikanvas-E2-server -b ws/E2-server main
  git worktree add ../Wikanvas-E3-web    -b ws/E3-web    main
  for w in ../Wikanvas-E1-engine ../Wikanvas-E2-server ../Wikanvas-E3-web; do
    git -C "$w" submodule update --init --reference "$PWD/vendor/inkboard" vendor/inkboard
  done
  ```
  Every `scripts/dev.sh` command sets `WIKANVAS_SHARED_CARGO_HOME=$HOME/.cache/wikanvas-cargo` in the same command.
- No new Rust or npm dependency is expected. If one is unavoidable, its owner adds it (E2: the `wikanvas-*` manifests it
  touches; E3: `web/package.json`), commits the lockfile and the `THIRD_PARTY.md` row in the same commit (R6), checks
  the licence against R6 and names it in the handoff note; the merge stage then merges that branch alone.
- No schema change (R5): `link_kinds` already has `embed`. No route, DTO or error code changes, so
  `docs/api/openapi.json` and `web/src/api/gen` stay byte-identical.
- The owner's live wiki runs as compose project `wikanvas`: never `docker compose up/down/restart/rm` or a writing
  `exec` against it, never its volumes; use only the scripts (`e2e`, `agent-test`, `image-smoke`), which use their own
  project names. Retagging `wikanvas:latest` with `scripts/image.sh` is fine.
- R10 "done" uses `grep -rnE 'todo!\(|unimplemented!\(|TODO\(E' <owned paths>`.

| Id | Workstream | Worktree, branch | Depends on |
|---|---|---|---|
| E1 | Engine: patches 0005 and 0006, `frame.js`, the frame tests | `/home/staffan/Projects/Wikanvas-E1-engine`, `ws/E1-engine` | the frozen contracts |
| E2 | Server, core, store, `docs/AGENTS.md`, agent scenario 12 | `/home/staffan/Projects/Wikanvas-E2-server`, `ws/E2-server` | §18.2 and the fixture |
| E3 | Web: the host overlay, "Text doc", "Embed page…", the sidebar drag and the drop, the fake frame, unit tests | `/home/staffan/Projects/Wikanvas-E3-web`, `ws/E3-web` | the bridge types (fake frame until E1 merges) |
| M | Merge, smoke step 17, docs, the M1.1 gate | main, the main checkout | E1, E2, E3 |

```
main (contracts) ──┬─ E1 ─┐
                   ├─ E2 ─┼─ M (merge, smoke step 17) ─ M1.1 gate (+ the whole M1 release gate)
                   └─ E3 ─┘   E3 ⇠ E1 through bridge.ts only; E1, E3 ⇠ E2 through §18.2 only
```

### E1: engine (patches 0005 and 0006, frame page, frame tests)

**Goal.** ARCHITECTURE §18.3 and §18.4: the `BoxObj.embed` field in `inkboard-core`, the host events and methods in
`inkboard-app`, and `frame.js`'s side of the bridge, including the drop of a sidebar document.

**Order.**
1. `scripts/dev.sh scripts/inkboard.sh src` in the fresh worktree (base + 0001–0004).
2. 0005, part 1 (`inkboard-core`): `Embed`, `BoxObj.embed` last, lenient deserialization, `is_embed`, `embed: None` at the
   construction sites (`inkboard-core/src/model.rs`, `inkboard-core/tests/properties.rs`,
   `inkboard-formats/src/mermaid.rs`), the model tests; `cargo test -p inkboard-core`.
3. 0005, part 2 (`inkboard-app`): `HostEvents` and `HostMode` additions, the layout report in `host_frame`, the
   `start_editing` interception, the editing state with the swallowed press, `insertEmbed`, titles in place, no
   rotation handle for a single card, the JS side in `embed.rs`, the native unit tests (§18.3 "Native unit tests").
4. `export`, then `check` (no warning above the baseline) and the native tests.
5. `frame.js` (§18.4) against the frozen types.
6. `build --quick` and the new frame tests; then the full `build`.
7. 0006 (docs) and `patches/inkboard/README.md`: patchset `3`, the two rows, and the `inkboard-core` sentence of the
   series description amended as §10.2 now says.
8. `export --upstream`, the full acceptance below.

**Owns.**
- `patches/inkboard/**` (new `0005-embed-documents.patch`, `0006-docs-embedded-documents.patch`; `README.md`;
  `check-baseline.txt` stays as recorded on `base`), `scripts/inkboard.sh` (only if a change is needed)
- `web/inkboard-frame/**` (`frame.js`; `inkboard.d.ts` additive only), `web/src/board/bridge.ts` (additive only)
- `e2e/tests/frame.spec.ts`, `e2e/serve-static.mjs`, `fixtures/boards/frame-test.inkboard`
- `docs/handoff/E1.md`

**Produces.** A dist whose `manifest.json` has `patchset` `"3"` (`engine_version` `0.2.0-beta.3+embed.wikanvas.3`) and whose
`frame.js` implements every §18.4 member: the four events and `insertEmbed`, `setEmbedEditing`, `setEmbedTitles`,
`requestEmbedLayout`.

**Consumes.** The frozen contracts; nothing from E2 or E3.

**Key behaviours** (§18.3, §18.4): a card is a `rect` box with `embed {item}` and round-trips byte for byte; a malformed
`embed` loads as a plain box; the layout is reported at most once per frame and only on change, in canvas px turned into
frame px by `frame.js`, with the occluders of every visible egui layer above `Order::Background`; every path into
label editing of a card sends `onEmbedOpen` instead (with the double-click point), and nothing at all in a read-only
host (a right-click selects there, so Enter and the menu's "Edit text" still reach `start_editing`);
`setEmbedEditing` hides the card's handles, turns the engine's shortcuts off while it lasts, and the first press or
focus gain in the frame ends it, a canvas press swallowed; `insertEmbed` validates a label being typed first, uses
§18.2's defaults exactly (480 × 360, the style and label, the first visible unlocked layer), one undo step, `null`
when read-only (checked before `ed.insert`, which returns an id even then); `setEmbedTitles` changes labels
in place with no revision, again after every revision change, cleared by `host_load_doc`; capture-phase `dragenter`,
`dragover` and `drop` for `application/x-wikanvas-item` only, refused while read-only; no `t()` string; every changed
file has its notice line.

**Acceptance.**

```sh
scripts/dev.sh scripts/inkboard.sh src
scripts/dev.sh scripts/inkboard.sh check
scripts/dev.sh bash -c 'cd .cache/inkboard-src && CARGO_TARGET_DIR=/work/.cache/inkboard-target cargo test --locked -p inkboard-core'
scripts/dev.sh bash -c 'cd .cache/inkboard-src && CARGO_TARGET_DIR=/work/.cache/inkboard-target cargo test --locked -p inkboard-app embed'
scripts/dev.sh bash -c 'cd .cache/inkboard-src && CARGO_TARGET_DIR=/work/.cache/inkboard-target cargo test --locked -p inkboard-app i18n'
scripts/dev.sh bash -c 'cd .cache/inkboard-src && CARGO_TARGET_DIR=/work/.cache/inkboard-target cargo test --locked -p inkboard-formats mermaid'
scripts/dev.sh scripts/inkboard.sh build && jq -e '.patchset == "3" and .engine_version == "0.2.0-beta.3+embed.wikanvas.3"' web/dist-inkboard/manifest.json
scripts/dev.sh scripts/inkboard.sh export --upstream && ! grep -rq 'Changed for Wikanvas' .cache/inkboard-upstream
test "$(grep -c 'Changed for Wikanvas' patches/inkboard/0005-embed-documents.patch)" -ge 1
scripts/dev.sh pnpm --dir web typecheck
scripts/dev.sh pnpm --dir web test src/board          # the fake frame still satisfies the bridge
scripts/e2e.sh frame                                  # every M1 frame test and the new ones below
scripts/dev.sh cargo test -p wikanvas-core --locked -- board::   # the server's unpatched core still reads the boards
git -C vendor/inkboard status --porcelain | wc -l | grep -qx 0
```

New frame tests (`e2e/tests/frame.spec.ts`, real engine, `fixtures/boards/embed.inkboard`):

| Test | Checks |
|---|---|
| `embeds: the layout of the fixture` | (`clipboard` `[]`, and in `embeds: the editing state` `[ITEM1]` after Ctrl+C of card 1: the M1.1 review, D95) One `onEmbedLayout` after `open()` (as built: the engine sends the forced first frame's report and the settled one a frame later, E1-D12, so the test waits until the reports settle and checks the last, and that at least one came): two embeds with the fixture's object ids, items, frames and labels, the plain box left out; `items` sorted; `editing` null; `canvas` the whole canvas; `occluders` non-empty and inside `canvas`; a screenshot of the 80 × 24 px region centred at `origin + centre × zoom` of each card holds pixels darker than `#808080` (the engine's centred title snapshot; the fixture's background and the card's fill are both white, so a fill probe would prove nothing), while the same region 120 world units above a card's centre (inside the card, its white fill) holds none |
| `embeds: pan and zoom report at most once per frame` | A wheel zoom and a Hand-tool drag change `zoom` and `origin`; reports ≤ the animation frames during the gesture plus 2 (eframe also runs an engine frame inside `pointerdown` and `pointerup`, §18.3); no report in 1 s of idling; none for a pan on a board without cards (`frame-test.inkboard`) |
| `embeds: open instead of label editing` | A double-click on a card gives `onEmbedOpen` with its `objectId`, `item` and `point` within 2 px of the click, the card selected, no `onChange` and the snapshot unchanged; Enter on the selected card gives `point: null`; the plain box still edits its label (positive control) |
| `embeds: the editing state` | `setEmbedEditing(id)` → `layout.editing === id`; with the canvas still focused, `r`, `Delete` and Ctrl+V change neither the tool nor the board (no `onChange`, the card still there); a click on empty canvas → `onEmbedEditEnd {reason: 'press'}`, no marquee and the selection unchanged; again, then a click on an engine toolbar button → `press` and the tool changes; focusing the canvas after focusing another element → `focus`; `open()` of another board while editing → `gone` |
| `embeds: insertEmbed` | `'center'` returns an id, sends `onChange`, and the snapshot holds the card with exactly §18.2's defaults centred on the view centre; a point gives a card centred there (±1 world unit); Ctrl+Z removes it; read-only returns `null` and the snapshot gains no object; with the first layer hidden (a variant of the fixture built in the test) the card goes on the second, visible layer |
| `embeds: titles in place` | `setEmbedTitles` changes the layout's label with no `onChange`, the same revision and `isDirty()` false; after drawing a rectangle the snapshot holds the new label; after `open()` of the same board again the map is cleared |
| `embeds: round trip and lenient model` | The fixture round-trips byte for byte (without views); a box with `"embed": 5` opens without a `load` error and its snapshot has no `embed`; an ellipse with an `embed` keeps it in the snapshot and is not in the layout |
| `embeds: drop of a sidebar document` | A synthetic `dragover` with `application/x-wikanvas-item` is accepted (`defaultPrevented`); the `drop` gives `onDropItem` with the `DraggedItem` and the drop point; a drop without the type, with JSON that does not parse, with `v: 2` or `kind: 'board'` gives nothing; read-only: the `dragover` is not accepted and no event |
| `embeds: no rotation handle for a single card` | Dragging from where the rotation handle of a selected card would be leaves its `angle` 0; the same gesture on the plain box rotates it (positive control) |
| `embeds: read-only` | A double-click on a card gives no `onEmbedOpen`; neither do Enter nor the context menu's "Edit text" on a card selected with a right-click (as built: after a positive control on a writable board, where the same right-click and menu row give `onEmbedOpen` with `point: null`, so the row is known to be "Edit text"; the menu is found as the new occluder whose left edge is at the click, E1-D12) |
| `embeds: while the host edits, Shift and Alt with the wheel pan and the middle button pans, keeping the focus` (added by the M1.1 review, ARCHITECTURE D94) | With the editing set and the keyboard focus in another element of the page (as in the host's editor, so the board hears no key): Shift + wheel changes `originX` only and Alt + wheel `originY` only, `zoom` unchanged; a middle-button drag pans by the pointer's movement, the other element keeps the focus during the drag, `onEmbedFocusBack` follows the release (after eframe focused its canvas), and no `onEmbedEditEnd` comes; a right-click then ends the editing with `press` |
| `embeds: frame pixels when the canvas is not at the frame origin` (added by E1's review, R1) | With the canvas moved to 100, 80 before `open()`: `canvas` is `{100, 80, 1180, 640}`, the occluders lie inside it, the title's dark pixels are at `origin + centre × zoom`, the double-click point is within 2 px of the mouse, and a 40 × 40 card inserted at a frame point is the card a double-click there opens (each direction of the conversion checked against something absolute) |

The M1 frame tests keep passing, and every new test also checks that no CSP violation was recorded.

### E2: server, core, store, AGENTS.md, agent scenario 12

**Goal.** ARCHITECTURE §18.8–§18.10: cards become `item_links`, no value of `embed` is refused (only a repeated
`embed` key), an embedded item is never discarded, agents can build cards.

**Owns.**

| Area | Files |
|---|---|
| `crates/wikanvas-core` | `src/board.rs` |
| `crates/wikanvas-store` | `src/boards.rs`, `tests/store/boards.rs`; in `src/items.rs` only `discard_empty` (the `embed` link check, §18.8) |
| `crates/wikanvas-server` | `src/routes/boards.rs` (only if a change is needed), `tests/api/boards.rs` |
| Docs | `docs/AGENTS.md`, `docs/handoff/E2.md` |
| Agent harness | `e2e/agent/scenarios/12-board-embeds.sh`, any new `e2e/agent/fixtures/embed-*.inkboard` |

**Produces.** `wikanvas_core::board::Inspect::embeds`, `EMBED_LINKS_MAX` (1000), the embed scan (§18.8); `boards::save`,
`boards::init_with` and `boards::copy` maintaining `item_links` kind `embed` (board → target), the board's own id skipped;
the 422 for a repeated `embed` key; `items::discard_empty` refusing an item that any `embed` link targets; AGENTS.md
§8 "Embedded pages" with the §18.2 JSON, its rules and the `jq` recipe; scenario 12. No route, DTO or OpenAPI change
(the `discard_empty` parameter's description in `wikanvas-api` stays as it is).

**Consumes.** §18.2 and `fixtures/boards/embed.inkboard`; nothing from E1 or E3.

**Key behaviours.** The scan is a second, borrowed, typed pass after `from_json` (no `Value` tree, `Cow<str>` for
`type` and `geom`); it fails a PUT only for a box kind with the key `embed` twice (422 `invalid_board`, which the M1.1
engine could not load), and any other failure gives no embeds; only `type: "box"`, `geom: "rect"`, an `embed` object
with a canonical-UUID string `item` counts; targets are distinct in first-appearance order and capped at 1000; the
links are replaced in the PUT's writer transaction after the body text, untouched by a no-op save; nothing checks that
a target exists; `discard_empty` answers 409 `not_empty` for an item that is the `target_id` of an `embed` row.

**Acceptance.**

```sh
scripts/dev.sh cargo test -p wikanvas-core   --locked -- board::
scripts/dev.sh cargo test -p wikanvas-store  --locked --test store -- boards::
scripts/dev.sh cargo test -p wikanvas-server --locked --test api   -- boards::
scripts/dev.sh cargo test -p wikanvas-server --locked --test api   -- openapi:: && git diff --exit-code docs/api/openapi.json
scripts/dev.sh cargo test -p wikanvas-api --locked export_bindings && git diff --exit-code web/src/api/gen
scripts/dev.sh cargo clippy --workspace --all-targets --locked -- -D warnings
scripts/fmt.sh --check
scripts/image.sh --build-arg INKBOARD_BUILD=stub      # after the last commit: the harness refuses a stale image
scripts/agent-test.sh --scenario '12-*'
scripts/agent-test.sh                                 # every scenario, 01–12
```

These tests must exist and pass:

| Area | Test |
|---|---|
| core | `board::tests::embeds_found_in_rect_cards_only` (the fixture: both targets in order, the plain box none) |
| core | `board::tests::embeds_ignore_malformed_values` (`"embed": 5`, `{}`, `{"item": 3}`, `{"item": "abc"}`, an uppercase UUID, an ellipse with a valid `embed`: no target, and `inspect` succeeds) |
| core | `board::tests::embeds_distinct_and_capped` (one target twice gives it once; 1001 distinct give the first 1000) |
| core | `board::tests::embed_fixture_round_trips_byte_identical` (`split` then `splice` gives the input with the title) |
| core | `board::tests::embed_fixture_text_and_preview` (the text has both labels; page 0's SVG holds "First page") |
| core | `board::tests::embeds_repeated_key_is_refused` (a box kind with `"embed"` twice: 422 `invalid_board`; the same key once in each of two boxes, and a kind whose `"type"` is written `"bo\u0078"`, scan without an error) |
| Store | `boards::save_records_embed_links` (rows `(board, target, 'embed')`; a second save with one card leaves one row; a no-op save keeps them; the board's own id is never linked) |
| Store | `boards::init_with_records_embed_links` |
| Store | `boards::copy_copies_embed_links` |
| Store | `boards::purging_a_board_drops_its_links_not_a_targets` (purging the board removes its rows; purging a target leaves the board's row) |
| Store | `boards::discard_empty_keeps_embedded_items` (a new empty untitled document that a board's card targets: `items::discard_empty` is 409 `not_empty`; after a save without the card it is deleted; a new empty board that a document embeds is kept the same way) |
| API | `boards::put_with_embeds_keeps_them_byte_for_byte` (session and bearer clients) |
| API | `boards::malformed_embeds_are_accepted_and_kept` |
| API | `boards::repeated_embed_key_is_refused` (422 `invalid_board`, the stored board unchanged) |
| API | `boards::put_with_embeds_records_links` |
| API | `boards::duplicate_board_copies_embed_links` (through `POST /items/{id}/duplicate`) |

Agent scenario `12-board-embeds.sh` (CLI only, against the image, like 04):
1. A scratch collection; a board "Embed board"; a child document of the board "Embedded notes" (`item create --parent`,
   `--md` with "kangaroo"); a document "Shared page" at the top level.
2. The fixture with its two placeholder ids replaced (`jq`) by the child's and the shared page's ids: `board put
   --if-match r1` exits 0.
3. `board get`: both cards keep `kind.embed.item`, the plain box has no `embed`, `document.name` is the title, and the
   content equals the sent board apart from the name (`jq -S`).
4. `item text` of the board holds "First page" and "Second page"; `search "Second page"` finds the board.
5. A board with `"embed": 5`, `{"item": "not-a-uuid"}` and an ellipse with a valid `embed` added: `board put` exits 0 and
   `board get` keeps all three exactly.
6. `board preview --page 0` is `image/svg+xml` and holds "First page".
7. `item duplicate` of the board: the copy's `board get` has the same two targets.
8. `item trash` of the board: the child shows `trashed_at`, the shared page does not; `item restore`: the child is back.
9. `item move` of the child to the top level: `board get` is unchanged.
10. AGENTS.md's `jq` recipe, verbatim with the scenario's ids, appends a card for the shared page: `board put` exits 0,
    and `board get`'s page 0 ends with that card (`kind.embed.item` the shared page's id) while every earlier object is
    unchanged.
    10b. (Added by E2's review, R-3.) The recipe for a page titled `"  Padded  title  "` gives the label
    `"Padded  title"` (trimmed like the web client's snapshot). The scenario has 13 TAP steps (step 11 has two).
11. A new empty, untitled document `$SPARE` (`item create --kind doc` with neither `--title` nor `--md`), then a card
    for it appended to the board (`board put`): `item trash "$SPARE" --discard-empty` exits 8 with `not_empty`; after a
    `board put` without that card it exits 0 and `item get "$SPARE"` exits 7.

### E3: web (host overlay, Text doc, Embed page…, sidebar drag and drop)

**Goal.** ARCHITECTURE §18.5–§18.7: cards drawn and edited over the board, the three ways to add one, titles; all tested
against the fake frame, since E1's engine arrives at the merge.

**Owns.**

| Area | Files |
|---|---|
| Board host | `web/src/board/embed/**` (new), `web/src/board/{BoardFrameHost.tsx,host.ts,fakeFrame.ts,testkit.tsx,board.css,BoardSurface.tsx}`, the tests in `web/src/board/` |
| Board page | `web/src/pages/BoardPage.tsx` |
| Editor | `web/src/editor/{WikanvasEditor.tsx,menus.tsx,extensions.ts,slash.ts}` (`slash.ts`: leaving out an item whose callback is missing), new tests in `web/src/editor/` |
| Sidebar | `web/src/components/sidebar/TreeRow.tsx`, a new test next to it |
| Queries | `web/src/api/queries/docs.ts` |
| Docs | `docs/handoff/E3.md` |

Not owned: `web/src/board/bridge.ts` (E1; frozen) and `web/inkboard-frame/**` (E1). `web/package.json`,
`web/pnpm-lock.yaml` and `THIRD_PARTY.md` only if a dependency is unavoidable (Rules).

**Produces.** The overlay (`.board-embeds`, the §18.5 test contract), the "Text doc" and "Embed page…" buttons and the
picker, the drag source on document rows, the drop handling, `WikanvasEditor`'s `variant`/`popupRoot` props, the
`['docJson', id]` query, the fake frame's embed members and helpers (`layout(l)`, `openEmbed(objectId, point)`,
`endEdit(objectId, reason)`, `dropItem(item, point)`, the inserted cards), and additive `BoardFrameContext` members if
the board page needs them.

**Consumes.** The bridge types; the existing routes (`POST /items`, `GET …/doc/prosemirror`, `GET /items/{id}`,
`GET /search/titles`, the doc session's routes).

**Key behaviours** (§18.5–§18.7): the overlay shares the iframe's rect, applies each layout synchronously to the DOM, and
re-renders only when the set of drawn cards changes (as built, also on a card's label or selection change: ARCHITECTURE
D86; the merge folded E3-D1 to D15 into §18.5–§18.6 and D86–D92); `overflow: clip` (never `hidden`) on the overlay and the card
boxes, only the editing body scrolls, and no `will-change`; at most 24 static cards at zoom ≥ 0.5 and ≥ 120 × 80 px
with hysteresis, inset 12 world units, inner sizes clamped at 0; clipped to the canvas minus the occluders (disjoint
holes); static cards are read-only TipTap viewers of `DocJson` (200 blocks, `If-None-Match`, `injectCSS: false`), with
"Empty page", "Double-click to edit" on a selected card, and a title link on a read-only board; one editing card on a
`DocSession`, entered on `onEmbedOpen` with `setEmbedEditing` and focus moved to the card at once, the caret at the
double-click point (through world units), left on Esc (then `frame.focus()`), `onEmbedEditEnd`, a pointerdown outside
the card and `popupRoot`, a later layout without the card or no longer editing it, and every board or target state
change; `popupRoot` in `document.body`, moved into `.board-frame-layer` while it is fullscreen; no board items in a
card's slash menu; Ctrl+wheel over the editing card prevented; the target states; paper cards with `color-scheme:
light`; the three ways to add through one `insertCard` (a drop titled from the cached `ItemDetail`); the discard of
empty unreferenced "Text doc" documents, before the board's own discard; titles to `setEmbedTitles`; Focus over the
layer; the buttons only when the frame has `insertEmbed`.

**Acceptance.**

```sh
scripts/dev.sh pnpm --dir web typecheck
scripts/dev.sh pnpm --dir web test src/board src/editor src/components/sidebar src/pages
scripts/dev.sh pnpm --dir web test
scripts/dev.sh pnpm --dir web build
! grep -rnE 'TODO\(E' web/src
```

These tests must exist and pass:

| File | Checks |
|---|---|
| `board/embed/layout.test.ts` | world → frame → viewport with an iframe offset; `chooseCards`: the 0.5 zoom and 120 × 80 px thresholds, the hysteresis (a drawn card stays down to 0.45 and 108 × 72 px, a new one needs 0.5), the cap of 24 largest first, the editing card always, off-screen and rotated bounds; inner boxes of a 20 × 20 card clamp at 0; `clipPath` with overlapping and edge-touching occluders gives disjoint holes whose area is the union's |
| `board/embed/embedLayer.test.tsx` | a fake layout sets the world transform (and no `will-change`) and the card boxes (inset 12); a camera-only layout renders no card component again (a render counter); hidden and `inert` with the iframe; emptied on a frame reset; another board's events ignored; `requestEmbedLayout` after `open()`; `.board-embed-popups` moves into `.board-frame-layer` on a `fullscreenchange` that makes the layer the fullscreen element, and back to `document.body` on the next |
| `board/embed/staticCard.test.tsx` | `DocJson` fetched once, revalidated with `If-None-Match` (304 keeps it); only the first 200 blocks; no POST ever; no `<style>` element added to `document.head` (`injectCSS: false`); "Empty page" for an empty document; "Double-click to edit" only while selected on a writable board; on a read-only board the header title is a link to the page; the states `loading`, `static`, `trashed`, `archived`, `missing` (404, a non-UUID `item`, a board target) and `error` |
| `board/embed/editing.test.tsx` | (after the M1.1 review: every Esc is sent with `keyCode` 27, as a browser does, so ProseMirror's own handling runs; Esc in the body leaves through the editor's `onEscape` and the event is `defaultPrevented`, Esc in the title input leaves too, the slash menu's Esc does not; a save refused with 409 `read_only` ends the editing, the card becomes `trashed` after the refetch, shows the server's text and "Not saved" without a Retry, then, restored, a Retry that GETs the document and POSTs the outbox; the frame's `onEmbedFocusBack` puts the focus back in the editor, with its selection, or the title input, and does nothing for another card or after the end) `onEmbedOpen` → `setEmbedEditing(id)` and the card focused before the session has opened (a pending `GET …/doc`), then a `DocSession` for the target, the editor focused with the caret from the converted point (the camera moved in between), typing POSTs only the document's updates; Esc while the session still opens ends it; Esc → `setEmbedEditing(null)`, flush, `frame.focus()`; `onEmbedEditEnd` ends without moving focus; the first layout after `onEmbedOpen` with `editing: null` does not end it, a later layout without the card or with `editing` no longer this card does; a pointerdown outside ends it, one in `popupRoot` does not; an `onEmbedOpen` for the editing card changes nothing, a second one for another card ends and flushes the first before opening; never opened on a read-only board or for a trashed, archived or missing target; ends when the board turns read-only; an open failure → the toast and `setEmbedEditing(null)`; a Ctrl+wheel over the editing card is `defaultPrevented` |
| `board/embed/textDoc.test.tsx` | "Text doc": `POST /items` with `parent_id` = the board and `kind: "doc"`, then `insertEmbed({at: 'center', title: 'Untitled'})`, then editing; disabled while read-only or without `insertEmbed`; leaving the board discards created documents absent from `items` (`DELETE ?discard_empty=true`, a 409 ignored) and keeps the referenced ones, before a created board's own discard; an insert failure discards at once; (M1.1 review, D95) a document whose card is on the layout's `clipboard` is kept at the leave and at a later leave while the clipboard still holds it, then discarded once it does not, its cached detail reset so a card of it shows "Page not found"; one pasted on the next board stays |
| `board/embed/picker.test.tsx` | "Embed page…" lists documents only (a board hit filtered out), Enter takes the first, Esc closes, `insertEmbed` at the centre, then `frame.focus()` |
| `board/embed/drop.test.tsx` | `onDropItem` with a document → `insertEmbed` at its point, titled from the cached `ItemDetail` when there is one, otherwise from the detail fetched first (M1.1 review, D98: never from the drag data); a non-UUID id, another board's event, a board, trashed or missing target (cached or fetched) or a read-only board → nothing; a detail that does not load → the error toast; a title over 200 characters cut to 200; a drop on the editing card is taken in the capture phase and its viewport point converted to frame px |
| `board/embed/titles.test.ts` | `setEmbedTitles` gets `title.trim() \|\| 'Untitled'` for every cached target in `items`, debounced 250 ms, only when the map changed; a rename in the cache updates it |
| `board/boardPage.test.tsx` (updated) | Focus fullscreens `.board-frame-layer`; the two buttons exist only when the frame has `insertEmbed` |
| `editor/embedVariant.test.tsx` | `variant="embed"`: the slash and mention popups render in `popupRoot`, the bubble and table menus are appended to it with `strategy: 'fixed'`, no block handle, no heading anchors, and without `pickBoard`/`newBoard` the slash menu has no "Embed board…" and no "New board"; `variant="page"` (the default) renders as before, with both items |
| `components/sidebar/dragExternal.test.tsx` | a document row's `draggable` has `getInitialDataForExternal` giving `{'application/x-wikanvas-item': JSON.stringify({v: 1, id, kind: 'doc', title})}`; board and collection rows have none |

### Merge stage (M)

On main, by the orchestrator or one integrating agent, after E1, E2 and E3 are done (R10):
1. Merge the branches in the order they finish (`git merge --no-ff`; no file is shared, so a conflict means a
   workstream edited outside its "Owns" and goes back to it). After each merge, rerun that workstream's acceptance on
   main.
2. Apply the handoff notes' requests and fold their deviations into ARCHITECTURE §18 (with a row per decision in
   §18.14) and this section.
3. Write smoke step 17 (below) in `e2e/tests/smoke.spec.ts`, after step 16.
4. Run the M1.1 gate.

**Owns.** `e2e/tests/smoke.spec.ts`, `docs/ARCHITECTURE.md`, `docs/PLAN.md`, `README.md` (a sentence on cards in the
feature list, if it has one), `THIRD_PARTY.md`, `CLAUDE.md`, `docs/handoff/E-merge.md`, and fixes found by the gate in any
file, each as its own commit with a test.

**Smoke step 17** (real engine; skipped with a recorded reason when step 9 fell back to the preview, like steps 10–12):
"17. Board document embeds: Text doc, Embed page…, a sidebar drop, double-click editing, autosave, reload".
1. Create a new board "Embed board" under A and wait for the engine.
2. "Text doc": a `POST /items` with `parent_id` = the board; the sidebar shows an "Untitled" child under the board; a
   card `[data-state="editing"]` appears with its centre within 40 px of the canvas's; the focus is in its
   `.board-embed-editor .ProseMirror`. Type "wombat": the card's `.board-embed-status` reads "Saved", and
   `GET /api/v1/items/{doc}/text` holds "wombat". Click the card's title input and type "Wombat notes": the sidebar
   row is renamed.
3. Esc (in the title input): the card is `static` and still shows "wombat".
4. "Embed page…": search "C" and choose C: a second card appears near the centre, static, showing "quokka".
5. Drag the sidebar row of A (visible: the tree is expanded to the board) onto an empty part of the canvas at a point
   P with real mouse events: a third card appears centred on P (±40 px). If Playwright cannot carry the drag into the frame, dispatch `dragenter`,
   `dragover` and `drop` at P in the frame with a `DataTransfer` holding exactly §18.6's data, record the reason in the
   test annotations and in `docs/handoff/E-merge.md`, and add the real drag to ARCHITECTURE §16 item 26's manual list.
6. After the board's save: `GET /api/v1/items/{board}/board` holds three cards whose `kind.embed.item` are the new
   document, C and A, and the first card's label is "Wombat notes" (the title map was saved with the later saves).
7. Double-click the word "quokka" in C's card with `page.mouse` at the word's box (a static card takes no pointer
   events, so the double-click reaches the engine): the card is `editing` and the focus is in its editor; press End and
   type " numbat": "Saved", and `GET /api/v1/items/{C}/text` holds "quokka numbat". Click an empty part of the canvas
   with `page.mouse`: the card is `static` again, and the next board save (if any) holds no new object. Then zoom in
   with the wheel over an empty part of the canvas until C's card is taller than the canvas, double-click its body near
   the top, press Ctrl+End and Enter ten times: `.board-embeds` keeps `scrollTop` and `scrollLeft` 0 (ARCHITECTURE
   §18.5 `overflow: clip`) and the card's `getBoundingClientRect()` is the one it had before the keys (±2 px); Ctrl+Z
   ten times and a click on empty canvas leave the text as it was (step 8's reload refits the view).
8. Reload: once the engine is ready, the three cards are `static` and show "wombat", "quokka numbat" and A's text;
   open C: its body holds "quokka numbat".

As written at the merge (`e2e/tests/smoke.spec.ts`, `docs/handoff/E-merge.md`): the step runs in a 1600 × 1000 window
(three cards at zoom 1 need empty canvas around them) and restores the window after; the sidebar drag of item 5 is a
real mouse drag, which Playwright carries into the frame, so the synthetic fallback is not used; item 2's "Untitled"
child shows because "Text doc" expands the board's row (ARCHITECTURE D92); item 7 starts with a single click on C's card,
which selects it ("Double-click to edit") without editing; "leave the text as it was" compares C's stored text with the
text saved before the ten Enters; "A's text" is the title of the board A embeds ("Sketch", step 11).
Added after the M1.1 review (2026-10-29, `docs/handoff/E.md`): in item 7, while C is edited at zoom 1, Shift + wheel
and Alt + wheel over empty canvas above C pan sideways and up and down (C's box moves, its size does not), a
middle-button drag pans by the pointer's movement, and C stays editing with the focus back in its editor (" wallaby"
typed then is saved); at the end of item 7, a double-click in C's body and Esc pressed there (not in the title input)
make the card static, the frame focused and the card still selected ("Double-click to edit").

### Acceptance (the M1.1 gate)

On main after the merge stage, in this order, with `WIKANVAS_SHARED_CARGO_HOME=$HOME/.cache/wikanvas-cargo` in every
`scripts/dev.sh` command:
1. E1's, E2's and E3's acceptance blocks, verbatim, on main.
2. `scripts/dev.sh scripts/inkboard.sh build && scripts/e2e.sh frame` (the M1 frame tests and the new ones).
3. `scripts/image.sh` (the full engine), then `scripts/e2e.sh smoke` with step 17, and `scripts/agent-test.sh`
   (scenarios 01–12, no `--allow-stale`).
4. The whole M1 release gate, "Acceptance (the M1 release gate)" above, verbatim (it rebuilds and reruns the image,
   the image smoke, the smoke and the harness against the merged main).
5. `! grep -rnE 'todo!\(|unimplemented!\(|TODO\(W|TODO\(E|not implemented \(W' crates web/src web/inkboard-frame scripts e2e/agent`.

A failure is fixed at its cause or reported as a blocker; nothing is skipped, ignored or loosened (§16 item 31 lists
what only a real browser shows).

---

## M2: the egui client

**Goal.** The owner's decisions of 2026-10-29, as designed in `docs/ARCHITECTURE.md` §19 (normative once accepted;
D101-D143): one egui app for desktop and browser with boards and documents in one UI, Wikanvas Desktop and Wikanvas
Server from one binary, and a server-rendered HTML read view. Parity with today's web client is tracked item by item in
`docs/parity.md` (seeded from `docs/parity-seed.md`: 202 items, P-001-P-202, plus the vitest files, the new M2
behaviours and the manual checks). Written against main `d5d3c94`, updated after its review against `9c6e60a` (the M1.1
review's D93-D99) and after the rename to Wikanvas (`484905c`, D100).

Phases, in order:
- **S0**, the spike: time-boxed, with numbered go/no-go criteria. Nothing of the port starts before its go.
- **F0**, foundations: lands alone, as its own commits: the binary names (D127; the rename itself came with
  v0.1.0-alpha.1, D100), the Inkboard patches from S0, the crate scaffold and every frozen contract.
- **C1-C9**, the port: nine workstreams in parallel worktrees.
- **I**, integration: merges, and the M2 gate run twice with both clients (dual run).
- **X**, the cutover: the React client, the frame page and the DOM tests are deleted. Stored data needs no migration.
- **The M2 release gate.**

A read-view workstream (C7's first half) does not depend on the spike's outcome and may run during S0 (see C7).

---

### Rules for every M2 workstream

R1-R11 (top of this file) apply as written. In particular, and repeated because the port multiplies builds:
- **Builds only through the capped scripts, one at a time.** Every cargo, rustc, rustfmt, wasm-bindgen, wasm-opt, node,
  pnpm and npx command runs as `scripts/dev.sh <cmd>`; images only through the buildx scripts, all on the
  `wikanvas-limited` builder: `scripts/image.sh`, `scripts/toolchain.sh`, `scripts/cli-dist.sh`, the new
  `scripts/dist.sh` and the S0-only `scripts/spike/image.sh`; browsers only through `scripts/e2e.sh` (and, during S0,
  `scripts/spike/browser.sh`). The per-user lock lets exactly one build run at a time across every worktree
  (`scripts/lib/lock.sh`): at most four workstreams are active at once and their builds queue on that one lock (the
  orchestrator staggers the others); test runs stay filtered to the workstream's own modules, and
  `WIKANVAS_SHARED_CARGO_HOME=$HOME/.cache/wikanvas-cargo` is set in every `scripts/dev.sh` command. The caps may only
  be lowered. Never `sudo`; never build in or modify `vendor/inkboard`.
- **Environment for a container command** goes inside it: `scripts/dev.sh env NAME=value cmd …`. A variable set in front
  of `scripts/dev.sh` stays on the host, because `scripts/dev.sh` passes only its fixed `-e` list into the container
  (`scripts/dev.sh:37-43`). Host scripts (`scripts/image-smoke.sh`, `scripts/e2e.sh`) read their own variables on the
  host as today.
- **New scripts.** Host scripts take the lock and call `wikanvas_slice` exactly as `scripts/e2e.sh` does (§14) and run
  their builds through `scripts/dev.sh` or the capped builder: `scripts/desktop-smoke.sh`, `scripts/ui-peek.sh`,
  `scripts/parity.sh`, `scripts/dist.sh`, `scripts/spike/browser.sh`, `scripts/spike/image.sh`,
  `scripts/spike/desktop-smoke.sh`, `scripts/spike/collect.sh`. Container scripts run inside `scripts/dev.sh` and take no
  lock, like `scripts/inkboard.sh`: `scripts/wasm-app.sh` and `scripts/spike/wasm-app.sh` (called as
  `scripts/dev.sh scripts/wasm-app.sh …`).
- **A skipped check is a failed check.** A gate line whose tests can skip (Inkboard's visual tests print `SKIPPED` and
  pass without `INKBOARD_SNAPSHOTS=1`, `crates/inkboard-app/src/tests_visual.rs:48-50`) also fails when its output
  holds `SKIPPED`. A negated check (`! … | grep …`) first captures its producer's output and fails when the producer
  failed, and a check over a glob first asserts that the glob matches a file.

M2 specifics:
- **R12. One egui.** egui, eframe, epaint and egui_kittest are 0.37.0 everywhere (D102); `scripts/check-pins.sh` fails on a
  second egui version in `Cargo.lock` and on any GUI crate (egui, eframe, epaint, wgpu, resvg, image) whose version
  differs from `.cache/inkboard-src/Cargo.lock` (D124). That file is a generated copy that only `scripts/inkboard.sh src`
  writes, so check-pins also fails when `.cache/inkboard-src` is missing or its stamp (`.git/wikanvas-src-stamp`,
  `scripts/inkboard.sh:26`) does not match the current patch files; the gate runs `scripts/dev.sh scripts/inkboard.sh
  src` before it.
- **R13. AccessKit contract.** Every interactive widget has an AccessKit role and a label unique within its container;
  custom widgets write their own nodes (§19.11). A widget without one is a bug, also because desktop screen readers need
  it. Tests select by role and label, never by AccessKit node id.
- **R14. Clock.** No `Instant::now`, `SystemTime::now`, `Date::now`, `performance().now`, `Performance::now`,
  `Local::now` or `Utc::now` in `wikanvas-client`, `wikanvas-app`, `wikanvas-editor` or `wikanvas-board` outside
  `wikanvas-client`'s `clock.rs`; every timer that saves runs on the injected `Clock` (D106, §19.4.2), and so does the
  undo manager's timestamp. No test sleeps for timing: `run_until_idle`, `ManualClock::advance` or a bounded poll.
- **R15. No hand-written JS or TS** under `crates/`, except the one boot loader `crates/wikanvas-web/boot/boot.js` of at
  most 80 lines (D139); browser access goes through web-sys; `js_sys::eval` and `js_sys::Function::new` are not used;
  no `<script` in Rust source except the shell template's one `<script type="module" src="/app/{build}/boot.js">`. The
  only other JS that ships is wasm-bindgen's output. The browser smoke's CDP expressions are test code and never ship.
- **R16. The live wiki.** The owner's wiki runs as compose project `wikanvas` from `compose.yaml` on main: never
  `docker compose up/down/restart/rm` or a writing `exec` against it, never its volumes; only the scripts with their own
  project names. `compose.yaml`'s project, service, volume, variable and environment-key names do not change in M2
  (D142). The wiki's image is pinned with `WIKANVAS_IMAGE` in `.env` since the v0.1.0-alpha.1 cut-over of 2026-10-29,
  which set it to `wikanvas:484905c` (§19.15, owner steps), so the `scripts/image.sh` runs of any worktree, which retag
  `wikanvas:latest` (`scripts/image.sh:33-35`), do not change it. From F0 on, `scripts/image.sh` tags `wikanvas:latest`
  only for a clean tree on main; the spike image uses its own tag `wikanvas-spike:<commit>`. The owner switches the live
  wiki to a new image by its exact tag.
- **R17. Inkboard.** Only S0-a, F0 (task 2), C6 and X (step 4) edit `patches/inkboard/**`, in that order and never two at
  once; any other workstream's need is a request in its handoff note. Every rebase reruns `scripts/inkboard.sh check`
  and Inkboard's full native test suite.
- **R18. The parity ledger.** `docs/parity.md` is owned by C9. A workstream that ports an item names its new test(s) in
  its handoff note; C9 sets the row to `ported` only when that test is in the tree and passes. A changed behaviour cites
  its D-number. Where a row names a test in a file another workstream owns (for example a C2 item tested by a C3 flow),
  the file's owner writes that test on the item owner's request, and the request and its test name go in both handoff
  notes.
- **R19. Names.** New code uses the Wikanvas names, as existing code does since the rename (D100). The build
  infrastructure keeps its `wikanvas-*` and `WIKANVAS_*` names (the lock, `WIKANVAS_BUILD_*`, `WIKANVAS_LOCK_HELD`,
  `WIKANVAS_SHARED_CARGO_HOME`, `WIKANVAS_DEV_*`, `wikanvas-toolchain`, `wikanvas-limited`, `wikanvas-buildkit`, the
  slices), and so does `compose.yaml` (D142); F0 renames only the CLI's binary (D127).
- **R20. Borrowed and ported code.** Every file that borrows or ports code carries a header naming the source, its
  version or commit and its licence, gets a row in `THIRD_PARTY.md` "Code ported into Wikanvas" in the same commit, and
  a line in the handoff note. The known sources: egui's `TextEdit` builder and `accesskit_text` (MIT OR Apache-2.0),
  Inkboard's `rich_edit.rs` format bar and paste listener and its test input helpers (MIT OR Apache-2.0),
  prosemirror-schema-list, prosemirror-tables and ProseMirror's `Fitter` (MIT). GPL, AGPL and BSL sources (Zed, Outline,
  Docmost, Wiki.js) are ideas only. I checks that every such header has its row.
- **R21. The toolchain image is built only from main.** `scripts/toolchain.sh` always tags `wikanvas-toolchain:latest`,
  which every worktree's `scripts/dev.sh` uses (`scripts/toolchain.sh:126`, `scripts/dev.sh:56`). So a change to the
  `toolchain-base` or `toolchain` stage lands on main as its own small commit, the coordinator runs
  `scripts/toolchain.sh` once from main, and each worktree rebases. Two such commits are planned: S0's (Mesa lavapipe,
  `libvulkan1`, Xvfb, `libgl1-mesa-dri`, `libxkbcommon-x11-0`, `LP_NUM_THREADS`) before S0 starts, and F0's (zig and
  cargo-zigbuild) at F0's start.
- **R10 "done"** additionally needs: wasm clippy for every wasm32 crate the workstream owns (the commands in the gate),
  the label audit for every screen it owns, and `grep -rnE 'todo!\(|unimplemented!\(|TODO\((S0|F0|C[1-9])' <owned paths>`
  finding nothing. Handoff notes: `docs/handoff/S0.md`, `F0.md`, `C1.md` … `C9.md`, `M2-merge.md`.
- **Worktrees** (R11), created on the host from main; the directory names follow the repository's name at the time:
  `git worktree add ../Wikanvas-C4-editor -b ws/C4-editor main`, then the submodule with `--reference`, as in M1.1.

### Overview

| Id | Workstream | Depends on | Effort, person-days (estimate) |
|---|---|---|---|
| S0 | The spike: go/no-go | the owner's acceptance of §19, R21's toolchain commit | time box: 15 working days (G1 alone up to 20), 4 tracks with 5 agents (at most 85) |
| F0 | Foundations: binary names, Inkboard series 4, scaffold, frozen contracts, scripts, the licence list, ledger | S0 go | 13 |
| C1 | Data layer: `wikanvas-client`, `LocalTransport` | F0 | 26 parity + 10 new = 36 |
| C2 | Shell and sidebar: routes, shell, tree, dialogs, menus, toasts, hotkeys, theme, icons, accessibility | F0 | 63 + 5 = 68 |
| C3 | Pages and settings: auth gate, item header and actions, collections, lists, search, switcher, settings, trash | F0 (C2's shell contract) | 42.5 |
| C4 | Editor core: model, commands, layout, selection, input, IME, keymap, clipboard, undo, input rules | F0 | 53 parity (P-115 not built; P-084 split, `docs/parity-seed.md`); 71-137 by lines, likely 99 |
| C5 | Editor blocks and menus: code, links, tasks, tables, images, callouts, mentions, embeds, menus, block handle, Contents, find | F0, C4's block contract (frozen in F0) | 54 parity (P-094 split); 30-59 by lines, likely 41 |
| C6 | Boards and cards: Inkboard 0008/0009 final, `wikanvas-board` | F0, C4 read-only mode (for static cards), C1 `board_sync` | 45.5 + 10 = 55 |
| C7 | Server: read view, HTML forms, `doc/html`, standalone `serve`, desktop identity, agents, app shell and `/app/{build}/` | none for the read view; F0 for the rest | 7 + 15 = 22 |
| C8 | Desktop and web entries, packaging: the `wikanvas` binary's desktop mode, `wikanvas-web` with the boot loader, Docker stages, the licence list, `dist` | F0 | 0.5 + 20 = 20.5 |
| C9 | Tests and parity: testkit, flows of smoke steps 1-17, browser smoke B1-B16, snapshots, digest, ledger | F0 | 22 |
| I | Integration, the dual-run gate twice | C1-C9 | 10 |
| X | Cutover | I, the owner's sign-off | 3 |

"Parity" is the weight of the items each workstream owns in `docs/parity-seed.md` (S 0.5, M 2.5, L 10 days, the
inventory's weights; P-084 and P-094 are weighed by their sized sub-items, P-115 counts 0 because M2 builds nothing for
it); "new" is work no parity item describes (transports, patches, packaging, the read view), estimated from reading,
UNVERIFIED. "By lines" turns the editor research's line ranges (§19.5: C4's components 10,700 / 14,800 / 20,500
production lines, C5's 4,500 / 6,200 / 8,800) into days at an assumed rate of 300 lines of production code plus its
tests (about as many again) per person-day: days = lines × 2 ÷ 300. That rate is UNVERIFIED; S0 measures it (lines
written and passing per track-day in S0-b and S0-c), and F0's handoff note re-derives C4 and C5 from the measured rate.
The plan uses the larger of the two estimates per workstream: C4 99 (71-137), C5 54. Total after S0: about 420-490
person-days (likely 445). The critical path is C4 → C5 → C6's editing cards → I → X. With two agents on C4 (the model
and commands; layout, input and clipboard) and C5 overlapping C4 once F0 freezes the block contract, it is about 90-150
working days after F0 (UNVERIFIED until the measured rate). These are planning estimates.

```
S0 (spike) ── go ── F0 ──┬─ C1 data layer ──────────────────────┐
     │                    ├─ C2 shell and sidebar ───────────────┤
     │                    ├─ C3 pages and settings ──────────────┤   C3 ⇠ C2 (shell contract from F0)
     │                    ├─ C4 editor core ─┬───────────────────┤
     │                    │                  └─ C5 blocks, menus ─┤   C5 ⇠ C4 (block contract from F0)
     │                    ├─ C6 boards and cards ────────────────┤   C6 ⇠ C4 (read-only mode), C1 (board_sync)
     │                    ├─ C7 server (rest) ───────────────────┤
     │                    ├─ C8 desktop, web, packaging ─────────┤
     │                    └─ C9 tests and parity ────────────────┴─ I (dual-run gate ×2) ─ X (cutover) ─ M2 gate
     └─ C7a read view and HTML forms (may run during S0, on today's crates)
```

Declared edges (each consumer builds against F0's minimal working version; the full version arrives on merge):

| Consumer | Needs | From | F0 provides |
|---|---|---|---|
| C2, C3, C4, C5, C6 | `Transport`, `Pending<T>`, `Clock`, `ApiError`, the typed calls they use | C1 | the traits, `FakeBackend`, the typed calls for `me`, `config`, `sidebar`, `tree`, `items/{id}` |
| C3 | the shell: `Route`, page slots, `Platform`, toasts, dialogs, menus | C2 | working minimal shell, routing and one dialog |
| C5 | `DocView`, the commands, the `BlockView` trait, `EditorHost` | C4 | the S0 model and primitives, the traits, plain paragraphs editable |
| C6 | the editor in read-only and editing modes inside a rect at a zoom | C4 | `DocEditor::show_in(rect, zoom, mode)` working for paragraphs |
| C6 | `board_sync` | C1 | the module with the save loop's public API, a no-backoff minimal version |
| C6 | `BoardView` | F0 (patch 0008 draft) | the S0 draft of 0008 with the frozen API of §19.6.2 |
| C8 | the app, the server as a library | C2, C7 | `WikanvasApp`, `Server::start` |
| C9 | every screen's AccessKit labels | C2-C6 | the label naming rules of R13 |
| C1, C3 | `doc/html`, the forms, the revision preview from Y | C7 | the routes answering 501 until C7 lands (the client hides the entries) |

---

### S0: the spike

**Goal.** Prove with numbers, before the port, that the hard parts work: a board as a widget in a host egui app, the
editor's hard cases (typing, IME, selection across blocks, undo, tables, HTML paste, live collaboration over yrs
against the real server), a long document's frame time, the WASM size and startup, and the egui upgrade.

**Before S0.** The owner accepts §19 (the live wiki's image is pinned since the v0.1.0-alpha.1 cut-over, R16); the
toolchain commit of R21 (lavapipe, Xvfb) lands on main and `scripts/toolchain.sh` runs once from main.

**Time box.** 15 working days, then a hard stop; only G1 may take up to 5 more days, and then only S0-a's G1, G2 and G15
run on (the other tracks stop at day 15). The result is `docs/handoff/S0.md` with each criterion's measured value, its
verdict, for every no-go the action taken or the owner's decision, and the measured coding rate (production and test
lines written and passing per track-day, from `git diff --stat` of each track) that F0 uses to re-derive C4 and C5.
Four tracks, five agents, each track in its own worktree:

| Track | Worktree, branch | Criteria | Starts | Needs from other tracks |
|---|---|---|---|---|
| S0-a engine (two agents) | `../Wikanvas-S0-engine`, `spike/S0-engine` | G1, G2, G15 | day 1: one agent on G1 (the upgrade); the other drafts 0008 on today's 0.33 tree from day 1 and rebases it onto 0007 when G1 passes | G13's `LocalTransport` (S0-d) by day 8 for G2; S0-b's `show_in(rect, zoom, mode)` by day 8 and G12's lavapipe (S0-d) by day 4 for G15 |
| S0-b editor model | `../Wikanvas-S0-editor`, `spike/S0-editor` | G3, G4, G6, G7, G9, G16 | day 1, on egui 0.37 without Inkboard | G12's kittest setup by day 4 |
| S0-c input | `../Wikanvas-S0-input`, `spike/S0-input` | G5, G8 | day 1, on egui 0.37 without Inkboard | G12's browser driver (S0-d) by day 5 |
| S0-d platform | `../Wikanvas-S0-platform`, `spike/S0-platform` | G10, G11, G12, G13, G14 | day 1 (G12 first: G5, G8, G10 and G11 use its browser driver) | G1's 0007 for G11's wasm with the engine; S0-b's editor for G10 |

If G1 needs its extension, G2 and G15 move into it (they need 0007); G11 is measured once without the engine on day 15
and again with it in the extension.

**Owns.** The four spike branches; throwaway crates `crates/spike-*` on them (never merged); `.cache/inkboard-src` commits
for 0007 and a draft 0008 (exported to the spike branch's `patches/inkboard/`); spike-only scripts `scripts/spike/*.sh`;
spike-only test files under `web/src/editor/spike*.test.ts` (never merged): `spikeStream.test.ts` (G9) and
`spikeWritten.test.ts` (G3 (3)); `docs/handoff/S0.md` (track S0-d writes it, the others send their sections). What
carries into F0: 0007, the 0008 draft, the measured numbers and rate, and the spike tests turned into goldens.

**How numbers are taken.** Every measurement is printed by a bench binary or a test as one JSON line,
`{"criterion":"G10","metric":"open_5000_native_ms","value":118.4,"runs":20}`, and `scripts/spike/collect.sh` gathers them
into the report. Native numbers come from release builds inside the toolchain container under its caps (6 CPUs, 12 GB);
browser numbers from headless Chromium in the pinned Playwright image (`mcr.microsoft.com/playwright:v1.64.1-noble`, as
`scripts/e2e.sh` uses) at 4 CPUs and 6 GiB, with WebGL2 on SwiftShader. Since SwiftShader rasterises on the CPU, browser
frame budgets use egui's CPU time per frame (`Frame::info().cpu_usage`, reported through the test bridge), and the
painted-frame rate is recorded beside it, not gated. `scripts/spike/browser.sh <scenario>` runs a static musl build of
the spike server (the spike dist plus `wikanvas_server::testing::TestServer`'s router, under the app CSP of §19.9) and
the static musl chromiumoxide driver inside that image, through the lock, exactly as `scripts/e2e.sh` runs Playwright.

**Go/no-go criteria.** "Blocking" criteria must pass for a go. A non-blocking criterion may fail if its no-go action is
taken and recorded.

| # | Proves | Pass when | Command or test | On no-go |
|---|---|---|---|---|
| G1 (blocking) | Inkboard on egui 0.37 | 0007 applies after 0001-0006 with `git am --no-3way`; `inkboard.sh check` shows no warning above the baseline in every feature set; `cargo test` of `inkboard-core`, `inkboard-formats` and `inkboard-app` passes (392 `#[test]` in `inkboard-app` at `9c6e60a`: the 388 counted on the patched tree at `d5d3c94` plus the 4 the M1.1 review added to 0005; the 12 ignored ones still ignored, font widths re-pinned); the 11 visual references regenerate on lavapipe (needs G12); `inkboard.sh build` succeeds and `scripts/e2e.sh frame` gives 25 passed, 1 skipped (the 26 tests at `9c6e60a`) with today's React client; over a generated corpus of at least 500 labels (1-40 words each, in every font family, size and box width that `fixtures/boards/*.inkboard` uses), at most 5 % per font family change their line count against 0.33.3 (a spike test printing both layouts; the 11 labels of the 3 fixture boards are reported as well, 9 of them 1-3 words, and `vendor/inkboard` holds no `.inkboard` file); 0007 changes at most 300 non-test lines | `scripts/dev.sh scripts/inkboard.sh src && scripts/dev.sh scripts/inkboard.sh check`; `scripts/dev.sh bash -c 'cd .cache/inkboard-src && CARGO_TARGET_DIR=/work/.cache/inkboard-target cargo test --locked -p inkboard-core -p inkboard-formats -p inkboard-app'`; `scripts/dev.sh scripts/inkboard.sh build && scripts/e2e.sh frame`; `scripts/dev.sh cargo test -p spike-engine --locked metrics::label_line_counts` | Up to 5 more days for G1 alone. If it still fails, stop: the owner chooses between waiting for upstream and M2 on egui 0.33.3 with the editor's IME limited to 0.33's `ImeEvent` (no active preedit range, no `DeleteSurrounding` from the web text agent, no `interrupt_ime`; 0.33's mobile web keyboards; shaping without harfrust). Every other track pauses its Inkboard-dependent parts |
| G2 (blocking) | A board as a widget beside other egui UI | Draft 0008 and the changed 0003 pass `inkboard.sh check`, including a native build without `net`; `tests_widget` passes: two boards in one context keep separate text fields; every chrome `Area`'s AccessKit bounds lie inside the board rect at 1024 × 700 and 640 × 480; typing "r" into a host `TextEdit` leaves the tool unchanged while a board is on screen; `CardPainter::paint` runs once per visible card in z-order (an arrow above a card is painted after its content); `edit`'s child `Ui` receives typed text; a primary press outside the card ends editing with `EditEnd::Press` and starts no marquee, and a secondary press there ends it with `Press` and opens the board's context menu; a primary press, a drag and a right-click inside the editing card leave the board's selection, the objects' frames, the drag state and the context menu unchanged; a middle-button drag over the card pans and keeps the editing; after a press gives the canvas egui focus, "r" selects the rectangle tool; `snapshot_json` → `load_json` round-trips equal. In `spike-host` (native and wasm), a sidebar, a board and a `TextEdit` share one context; a kittest draws a rectangle (R and a drag) and the board saves through `LocalTransport` (G13): `GET …/board` then holds one more object. 0008 changes at most 1,500 lines without the rename | `scripts/dev.sh bash -c 'cd .cache/inkboard-src && CARGO_TARGET_DIR=/work/.cache/inkboard-target cargo test --locked -p inkboard-app tests_widget'`; `scripts/dev.sh cargo test -p spike-host --locked board::` | Chrome cannot be bounded: `Chrome::Tools` plus Wikanvas-drawn menus. Pastes cannot be isolated on the web: the board's paste listener is installed only while the board has focus. A lib target impossible without the crate split: the owner decides between an upstream refactor request and boards later in M2 |
| G3 (blocking) | The editor model over yrs (structure edits, exact y-tiptap shape) | (1) `DocView` of all 19 fixtures (`fixtures/ydoc/*.bin`, 9 web-written; `fixtures/ydoc/server/*.bin`, 10 server-written), normalized, equals each `.pm.json`. (2) Every command (typing, Backspace and Delete across blocks, Enter in every block type, Tab and Shift+Tab in lists, mark toggles, set heading, wrap in quote and callout, paste of a JSON slice) on every fixture leaves `schema::validate(pm::normalized(doc))` passing and equals a reviewed golden. (3) Every spike-written `.bin` opens in the headless TipTap writer (`web/src/editor/fixtures/writer`) with no schema repair and the same PM JSON, through the spike test `web/src/editor/spikeWritten.test.ts`, which reads `SPIKE_YDOC_DIR` and fails when the directory is missing or holds a number of `.bin`/`.pm.json` pairs other than the count the spike wrote into `count.txt` beside them (`serverWritten.test.ts` cannot be used: its directory is fixed, `:8`). (4) A proptest of 10,000 random command sequences on random v1 documents never yields an invalid document. (5) Two replicas exchanging updates under random concurrent commands, 1,000 runs, converge; every invalid merged tree is displayed through read-time repair with zero writes, and the next local edit in that region writes a valid tree. (6) One keystroke in a 5,000-block document (transaction, observer, mirror update), release, native: median < 0.2 ms, p99 < 1 ms | `scripts/dev.sh cargo test -p spike-editor --locked model::`; `scripts/dev.sh env SPIKE_YDOC_DIR=/work/.cache/spike/ydoc pnpm --dir web test src/editor/spikeWritten`; `scripts/dev.sh cargo run --release -p spike-editor --bin bench -- keystroke` | (3) fails: copy y-tiptap's exact shape for that structure and rerun (a change, not a kill). (4) or (5) fails without a local fix: structural commands fall back to editing an own subtree and writing it through a `ydiff`-style diff (direct text edits stay), and C4 is re-estimated (+20 days); if that fails too, the design is killed and the owner is asked. (6) > 1 ms: profile; > 5 ms: change the mirror's update granularity |
| G4 (blocking) | Typing and selection across blocks | 1,000 typed characters equal the stored text; Enter in the middle and at the end of a paragraph, a heading, a list item, a task item, a quote and a code block (a newline there) gives the reviewed goldens; Backspace at a block start joins or lifts as P-084 describes; Shift+arrows and a mouse drag select across three blocks of three kinds, and deleting that selection is one transaction and one undo step; double- and triple-click select a word and a block; ↑ and ↓ keep the goal x across blocks within 1 px | `scripts/dev.sh cargo test -p spike-editor --locked ui::typing ui::selection` | Fixed inside the box. Unreliable hit-testing across block kinds: leave out the gap cursor in M2 and record it |
| G5 | IME | Native kittest with synthetic `Event::Ime` sequences: preedit then commit, preedit then focus loss, `DeleteSurrounding`, a Korean jamo sequence: the committed text is right; no yrs update exists while composing (the sync queue stays empty); one undo removes the composed word; a remote update applied every 300 ms (simulated) during a composition neither cancels nor corrupts it. In headless Chromium, three `Input.imeSetComposition` calls and an `Input.insertText` into the wasm spike store the committed text, and the published `cursor_rect` (read through the bridge) is within 2 px of the caret. By hand on the owner's Linux desktop: ibus or fcitx5 with Japanese or Pinyin, and a dead key (´ then e), in the native spike and in Chromium and Firefox on the wasm build. Other platforms (macOS, Windows, Android GBoard, iOS Safari) recorded, not gated | `scripts/dev.sh cargo test -p spike-editor --locked ui::ime`; `scripts/spike/browser.sh ime` | Native failure: fixed in the box. CDP IME fails in eframe's text agent: an eframe patch offered upstream, and the case recorded. The manual Linux check fails: the owner decides whether that is acceptable for M2 |
| G6 (blocking) | Undo | After a scripted session (typing, a split, a mark, a list indent, a table cell edit, a paste), ten Mod+Z restore the stored text to each earlier state in order and Mod+Shift+Z redoes; an update applied with origin `remote` between local edits survives ten undos; after each undo the caret is in the block it was in before the undone step; with the undo manager's timestamp on the `ManualClock`, two typing bursts 600 ms apart in simulated time are two undo steps and two 300 ms apart are one | `scripts/dev.sh cargo test -p spike-editor --locked ui::undo` | Bookmarks through `StackItem` meta fail: keep the selections in a side map keyed by the stack's length, and record it |
| G7 | Tables | Insert a 3 × 3 table with a header row; Tab through every cell, and past the last one adds a row; Shift+Tab goes back; add and delete a row and a column; toggle the header row; drag across a 2 × 2 rectangle (`Selection::Cells`) and Backspace clears it; every cell selected and Backspace deletes the table; a table with colspan and rowspan displays as its `.pm.json` says, and editing a spanned cell keeps its spans (`fixtures/ydoc/table.pm.json` has a colspan of 2 and no fixture a rowspan above 1, so the spike adds a fixture with a rowspan of 2 and a 2 × 2 merged cell); every result passes `schema::validate` and the Markdown GET prints the expected table | `scripts/dev.sh cargo test -p spike-editor --locked ui::tables` | Spans cannot be edited safely in time: tables with spans edit cell text only (structure commands disabled) in M2, recorded, the owner told |
| G8 | HTML paste and the rich clipboard | A corpus of at least 8 sources (Wikipedia, a GitHub README, Google Docs, Word Online, Notion, Stack Overflow, MDN, LibreOffice Writer) saved as HTML files parses to golden PM JSON; a Wikanvas-to-Wikanvas copy and paste through `data-wikanvas-slice` round-trips exactly (marks, task states, table spans, `itemId`s); a hostile corpus (`javascript:` and `data:` URLs, `on*` attributes, `<script>`, `<style>`, 10,000-deep nesting, 5 MB of HTML) gives only allow-listed nodes, never panics, and takes < 200 ms per MB natively; in headless Chromium with eframe's own listener present, the HTML our `copy` listener writes is what a `paste` in the same page reads back (`text/html` with `data-wikanvas-slice`), and pasting the corpus HTML into the wasm editor gives the same goldens; arboard writes and reads HTML under Xvfb (X11). Wayland, macOS and Windows recorded by hand. html5ever's added wasm size goes to G11 | `scripts/dev.sh cargo test -p spike-editor --locked clipboard::`; `scripts/spike/browser.sh clipboard` | html5ever adds > 400 KB brotli: a smaller tolerant parser measured against the same corpus. Our listener's write is overwritten: HTML copy only from the menu command (`navigator.clipboard.write`, secure contexts only), recorded. arboard HTML fails on X11: plain text plus an in-process slice cache |
| G9 (blocking) | Live collaboration over yrs against the real server | `wikanvas_server::testing::TestServer`, two spike editors each with a minimal Rust `doc_sync` (the M1 pull protocol), and a stream of at least 200 TipTap-written updates on the same document (typing, splits, list indents, marks), made once by a spike-only vitest with the headless writer and replayed through `POST …/doc/updates` with a token; 1,000 random edits per editor, pulls every 300 ms (simulated focus) and random POST delays; at the end `GET …/doc/prosemirror` equals both editors' mirrors and passes `schema::validate`; at every remote apply, no caret leaves its block unless that block was deleted; opening a document and closing it without an edit sends no POST (the server's `seq` unchanged); one dropped POST forces a 409 `missing_deps`, which recovers; two editors rewrapping the same block at the same time (both set it as a heading; both wrap it in a quote) converge to a valid tree whose outcome is recorded (the duplicated block of §19.5.2 is expected), and the read view of the result has unique heading ids. 20 seeds, 20 of 20 pass | `scripts/dev.sh pnpm --dir web test src/editor/spikeStream` (once, writes the stream); `scripts/dev.sh cargo test -p spike-editor --locked --test collab` | Divergence is a bug and is fixed; at the CRDT level it kills the design (the owner is asked). A caret jump: fix the sticky-index mapping. The TipTap stream needs schema repair on our side: extend the read-time repair rules and record them |
| G10 (native part blocking) | A long document's frame time | Generated documents of 1,000, 5,000 and 20,000 blocks (the largest about 1 MiB of Markdown, the API's input limit, with one 2,000-item list, 20 code blocks and 5 tables). Open to first painted frame of 5,000 blocks: < 150 ms native, < 400 ms wasm. Keystroke to presented frame in its middle, CPU time per frame p95: < 4 ms native, < 12 ms wasm. Scrolling the 20,000-block document at 3,000 px/s, CPU time p95: < 8 ms native, < 16 ms wasm, and the scrollbar thumb never jumps more than 2 % of its track in one frame once estimates are replaced. A width change relays out the visible blocks in < 16 ms. Memory for 20,000 blocks < 150 MB native resident and < 150 MB wasm linear memory. A 10,000-row sidebar scrolls at CPU time p95 < 4 ms native | `scripts/dev.sh cargo run --release -p spike-editor --bin bench -- long`; `scripts/spike/browser.sh long` | Within 2× of a budget: optimise in the box (incremental mirror, job reuse). Beyond 2×: measure blocks ahead in idle frames (2 ms a frame), then the own galley cache with atlas invalidation (D108's fallback). Wasm still beyond 2×: the owner decides whether the wasm budgets may be relaxed |
| G11 | WASM size and startup | `spike-host` with the egui 0.37 shell, the spike editor, the patched Inkboard widget, yrs, `wikanvas-core` (comrak), html5ever and a minimal client, release with `wasm-opt -Os`: ≤ 5.0 MB brotli and ≤ 18 MB raw, of which at most 1.5 MB brotli above the engine's 4,034,465 bytes; cold start (empty cache) from navigation start to the first painted frame ≤ 3.0 s and warm (cached) ≤ 1.5 s, median of 5, in headless Chromium on loopback over HTTP/1.1; the same cold start also under CDP `Network.emulateNetworkConditions` (20 Mbit/s down, 40 ms RTT), reported beside it, not gated (5.0 MB at 20 Mbit/s is 2.0 s of download alone); no CSP violation under §19.9's app CSP | `scripts/dev.sh scripts/spike/wasm-app.sh` (build and sizes, as `inkboard.sh size` does); `scripts/spike/browser.sh startup` | Over 5.0 MB: apply the savings of §19.9 (STIX loaded lazily, emoji fonts out of the web build, `-Oz`, image decoders reviewed) and measure again in the box. Still over 6.0 MB: the owner decides (§19.15 question 1). Cold start over 3 s: a streaming-instantiation check and a loading bar; the owner told |
| G12 (kittest and flow parts blocking) | The test infrastructure | With `mesa-vulkan-drivers` in the toolchain image (R21's commit on main), kittest 0.37's `.wgpu()` renders 1024 × 700 on lavapipe (`MESA_VK_DEVICE_SELECT=list` shows it; with and without Xvfb recorded); the test bridge's `tree()` returns the AccessKit tree in Chromium for a 500-row sidebar in < 20 ms and < 1 MB of JSON; chromiumoxide starts the image's Chromium with SwiftShader WebGL2 (renderer string recorded) and drives `Input.imeSetComposition`; one kittest flow against `TestServer` with `run_until_idle` passes 50 of 50 runs | `scripts/toolchain.sh` (once, from main, R21); `scripts/dev.sh cargo test -p spike-host --locked snapshot::renders_on_lavapipe`; `scripts/spike/browser.sh bridge`; `scripts/dev.sh bash -c 'for i in $(seq 50); do cargo test -p spike-host --locked flows::one_flow -q \|\| exit 1; done'` | Lavapipe fails: PNG snapshots leave the gate (AccessKit text snapshots stay) and a manual visual check is recorded. The bridge fails: Inkboard's `mark()` pattern. chromiumoxide fails: Playwright with a bridge-only spec (Node stays for tests). A flaky flow: fix the kit, never add retries |
| G13 (blocking) | The one boundary, in-process | `LocalTransport` over `Router::oneshot` with a desktop session created in-process: `GET /auth/me`, `GET /sidebar`, `POST …/doc/updates`, `PUT …/board` of two 20 MiB envelopes, one image-heavy and one object-heavy (at least 50,000 labelled boxes and arrows, no images: `Page.objects` is a deep-cloned `Vec<Object>`, `crates/inkboard-core/src/model.rs:558-563`, while assets and strokes are `Arc`) (200, through the CSRF layer, the Host allowlist and the body limits), a 412 on a stale rev, an export streamed as a download; the same cookie mutation without `Sec-Fetch-Site` is refused with 403, which shows the CSRF layer is in the path; a 401 after the session is deleted mints a new one and the retried request succeeds; each 20 MiB PUT completes in < 1.5 s native while the UI thread is blocked < 16 ms (the snapshot clone only). Recorded, not gated: the object-heavy clone's time on wasm, and on G10's 20,000-block document at a 250 ms POST cadence the median `POST …/doc/updates` latency and the SQLite writer's hold time (§19.4.4) | `scripts/dev.sh cargo test -p spike-host --locked local_transport::` | A middleware needs a socket: inject what it reads (`ConnectInfo` and the like). Streaming does not work through `oneshot`: collect bodies in M2, recorded. The object-heavy clone blocks > 16 ms: the snapshot is taken only after the 1.5 s quiet (as now) and the cost recorded; structural sharing in `Page` becomes an upstream request |
| G14 | One binary for desktop and `serve` | A spike `wikanvas` binary that links eframe 0.37 and the server: `readelf -d` lists only `libc.so.6`, `libm.so.6`, `libgcc_s.so.1` and `ld-linux-x86-64.so.2` as `NEEDED`; its `serve` answers `/healthz` 200 inside `gcr.io/distroless/cc-debian13` (a spike image target, built through the capped builder); its desktop mode starts under Xvfb with an in-process server and SQLite in a temporary data dir and writes a screenshot ≥ 800 × 600 with ≥ 50 colours within 120 s; binary size recorded | `scripts/dev.sh bash -c 'readelf -d target/release/wikanvas \| grep NEEDED'`; `scripts/spike/image.sh` (a spike variant of `scripts/image.sh`, tag `wikanvas-spike:<commit>`, never `:latest`); `scripts/spike/desktop-smoke.sh` | A display library in `NEEDED`: the image builds `--no-default-features --features server` (D115's fallback), the owner told. The Xvfb launch fails: record the driver error and fix in F0 |
| G15 | Zoomed board cards stay sharp and stable | On `fixtures/boards/embed.inkboard` plus generated cards, one editing card and 24 static cards at the bucket zooms 0.25, 0.5, 1, 2 and 4 and at the off-bucket zooms 0.53, 1.37, 2.97 and 3.9 (D109's buckets are powers of 2^(1/16)): at each zoom a card's text differs from the same text laid out natively at the bucket's font size by < 0.5 % of pixels (lavapipe), and its bounding box lies inside the card's inner rect; line breaks equal zoom 1's for ≥ 99 % of lines over the fixture documents at every tested zoom; a pinch (zoom changing every frame) costs CPU time p95 < 8 ms native and < 16 ms wasm with 24 static cards, and every card reaches the resting zoom's bucket within 4 frames; at zoom 3 and at 2.97 a click on the centre of each of 100 random characters puts the caret before that character, 100 of 100 | `scripts/dev.sh cargo test -p spike-host --locked cards::`; `scripts/dev.sh cargo run --release -p spike-host --bin bench -- pinch` | Line breaks differ: static cards use Inkboard's per-line method (`render.rs:671-718`); the editing card relays out per bucket. A pinch too slow: fewer static cards move to the new bucket per frame (down to 2), recorded; no scaled drawing (rejected in D109) |
| G16 (blocking part) | The editor is visible to tests and screen readers | Blocking: kittest finds paragraphs, headings with their level, list items, table cells, images with their alt text and links in the spike editor by role and label, and the label audit passes on `spike-host`. Recorded, not gated: what NVDA, Orca and VoiceOver read | `scripts/dev.sh cargo test -p spike-editor --locked a11y::` | Without these nodes no L2 or L3 test can see the editor: fix before go |

**Go rule.** S0 is a go when G1, G2, G3, G4, G6, G9, G10 (native), G12 (kittest and flows), G13 and G16 (blocking part)
pass, and every other criterion passes or has its no-go action taken and recorded in `docs/handoff/S0.md`, with the
owner's decision where the table says so. The whole design is killed only by G3 (4) or (5) failing together with its
fallback, by a CRDT-level divergence in G9, or by a G1 failure after which the owner chooses to wait for upstream.

---

### F0: foundations (lands first, alone)

**Goal.** Everything the parallel workstreams build on, working and frozen, so nobody waits on a `todo!()`.

**Tasks, in order.**
1. **Binary names** (§19.8.10, D127, D142; the rename to Wikanvas itself was done for v0.1.0-alpha.1, D100, with no
   fallback): the server's binary `wikanvas-server` → `wikanvas` (the new crate's) and the CLI's binary `wikanvas` →
   `wikanvas-cli`, with no alias; `scripts/image.sh` tags `wikanvas:latest` only for a clean tree on main (today every
   run tags `wikanvas:<commit>` and `wikanvas:latest`), and the test scripts default to `wikanvas:latest`; the port in
   the plain-HTTP cookie name (§19.8.3); docs, AGENTS.md and the agent harness. Not renamed (D142, R19): `compose.yaml`
   (project, service, volumes, variables, environment keys), the export's `wikanvas.db` entry, and the build
   infrastructure (the lock, `WIKANVAS_BUILD_*`, `WIKANVAS_LOCK_HELD`, `WIKANVAS_SHARED_CARGO_HOME`, `WIKANVAS_DEV_*`,
   `wikanvas-toolchain`, `wikanvas-limited`, `wikanvas-buildkit`, the slices).
2. **Inkboard series 4** from S0-a: the changed 0003, 0007, the 0008 draft with the frozen `widget` API of §19.6.2, 0009;
   `patches/inkboard/README.md` (patchset 4, rows, sizes); the root manifest takes `inkboard-core` from
   `.cache/inkboard-src/crates/inkboard-core`; `scripts/dev.sh` runs `scripts/inkboard.sh src` when stale;
   `scripts/check-pins.sh` gets R12's checks; D59 reworded in ARCHITECTURE (D124).
3. **Server as a library**: `Server::{start, router, listen, shutdown}` (§19.8.2); `TestServer::start_unprovisioned(code)`
   and `seed: true`; `wikanvas-api` with features `openapi` and `ts`; `wikanvas-core` building for wasm32 (getrandom's
   wasm backend).
4. **Scaffold**, each crate compiling natively and, where §19.3 says so, for wasm32, with one test: `wikanvas-client`
   (the traits, `Pending`, `Clock`, `ApiError`, `FetchTransport`, `HttpTransport` under `http`, the calls listed under
   "Declared edges", `FakeBackend`), `wikanvas-editor` (S0's `DocView` and primitives, `EditorHost`, `BlockView`, plain
   paragraphs editable, `show_in(rect, zoom, mode)`), `wikanvas-board` (a `BoardSurface` showing a board), `wikanvas-app`
   (`WikanvasApp`, `Route`, `Platform`, the shell skeleton, theme tokens, the i18n module, fonts installed once),
   `wikanvas-web` (`WebRunner` start under `/next/`), `wikanvas` (desktop and `serve` modes, `LocalTransport`, the data-dir
   lock), `wikanvas-testkit` (the harness wrapper, `run_until_idle`, the label audit), `wikanvas-browser-test` (B1
   only).
5. **Docker and scripts**: zig and cargo-zigbuild (pinned by checksum) join the toolchain stage through R21's commit on
   main at F0's start (the Mesa packages came before S0); the `licenses` stage is replaced by one that does not depend on
   `web`: `cargo metadata` for `x86_64-unknown-linux-gnu` and `wasm32-unknown-unknown`, then `wikanvas-tools licenses`
   with Inkboard's lists from `.cache/inkboard-src`, the font licences, lucide's LICENSE and the `web` stage's npm list
   as a file (§19.13); a `web-app` stage builds `wikanvas-web`'s dist, ordered after the Inkboard wasm stage so two
   rustc builds never overlap; the runtime image runs `wikanvas serve` and embeds both clients (React at `/`, egui at
   `/next/`); the `NEEDED` gate; the new scripts of the Rules with the lock pattern; `scripts/e2e.sh` gains
   `smoke-react` (today's Playwright smoke, renamed), `smoke` (the Rust browser smoke, run with the static musl driver
   inside the same pinned Playwright image and caps), `--only B1,B5,…` and `--startup` (G11's start-up scenario against
   the image, cold and warm); `scripts/image-smoke.sh` gains the app's checks (the test bridge absent from the served
   wasm and glue, `/app/{other build}` 404, the licence list); `THIRD_PARTY.md` rows for every new crate and build
   tool.
6. **Ledger**: `docs/parity.md` from `docs/parity-seed.md`, every row `missing` except those S0's goldens already cover;
   the vitest section expanded from one row per file to one row per `it`/`test` call site (350 at `9c6e60a`), each
   naming a new test `path::name` or `gone: <reason>`; `scripts/parity.sh` checks every named Rust test exists
   (`scripts/dev.sh cargo test --workspace --locked -- --list`) and fails on a `missing` row in any section.
7. **Tests of the new M2 behaviours** that F0 itself builds (`docs/parity-seed.md`, "New behaviour"): the
   second-instance refusal and `PRAGMA user_version` staying 1.

**Owns.** Everything the tasks touch, until F0 merges; afterwards each file belongs to the workstream the C-sections name.

**Produces (frozen contracts, R3).** `Transport`, `Pending<T>`, `Clock`, `ApiError` and the typed-call naming
(`wikanvas-client`); `EditorHost`, `BlockView`, `DocEditor::show_in` and the command API (`wikanvas-editor`); the
`widget` API of §19.6.2 (patch 0008); `Route`, `Platform`, the page-slot API and the toast and dialog API
(`wikanvas-app`); the testkit API (`App::test`, `run_until_idle`, `by_label`, `by_role`, `tree_rows`, `restart`, input
helpers, `FakeBackend` scripting); the flows binary `crates/wikanvas/tests/flows/main.rs` with one module per workstream
(R7); the AccessKit naming rules (R13).

**Acceptance.**

```sh
scripts/dev.sh scripts/inkboard.sh src && scripts/check-pins.sh       # check-pins needs the fresh patched lock (R12)
scripts/fmt.sh --check
scripts/dev.sh cargo clippy --workspace --all-targets --locked -- -D warnings
scripts/dev.sh cargo clippy --locked --target wasm32-unknown-unknown -p wikanvas-api -p wikanvas-core -p wikanvas-client -p wikanvas-editor -p wikanvas-board -p wikanvas-app -p wikanvas-web -- -D warnings
scripts/dev.sh cargo test --workspace --locked                 # the 706 server-side tests plus the scaffold's
scripts/dev.sh cargo test -p wikanvas-store --locked -- schema::user_version_stays_1
scripts/dev.sh cargo test -p wikanvas-server --locked --test api -- openapi:: && git diff --exit-code docs/api/openapi.json
scripts/dev.sh scripts/inkboard.sh check
scripts/dev.sh bash -c 'cd .cache/inkboard-src && CARGO_TARGET_DIR=/work/.cache/inkboard-target cargo test --locked -p inkboard-core -p inkboard-formats -p inkboard-app'
scripts/dev.sh pnpm --dir web typecheck && scripts/dev.sh pnpm --dir web test   # the React client still works
scripts/dev.sh scripts/inkboard.sh build && scripts/e2e.sh frame
scripts/image.sh && WIKANVAS_SMOKE_LEVEL=full scripts/image-smoke.sh && scripts/e2e.sh smoke-react && scripts/agent-test.sh   # image-smoke now also requires web-sys and wasm-bindgen in /app/{build}/licenses.txt (§19.13)
scripts/e2e.sh smoke --only B1                                 # the egui app boots at /next/ under its CSP
scripts/desktop-smoke.sh
scripts/dev.sh cargo test -p wikanvas --locked --test flows -- desktop::second_instance_refused
git -C vendor/inkboard status --porcelain | wc -l | grep -qx 0
```

---

### C1: the data layer (`wikanvas-client`, `LocalTransport`)

**Goal.** §19.4: the three transports, the typed API for every operation, the query cache with optimistic updates,
`doc_sync`, `board_sync`, the outbox, the pending registry.

**Owns.** `crates/wikanvas-client/**`; `crates/wikanvas/src/local.rs`; `crates/wikanvas/tests/flows/client.rs`;
`docs/handoff/C1.md`.

**Parity items** (`docs/parity-seed.md`): P-016, P-018, P-116-P-124, P-128, P-129, P-131, P-169-P-171.

**Key behaviours.** Debounces and back-offs exactly as §9.4 and D32, the desktop included (§19.4.4); every timer on the
injected `Clock` (monotonic, wall with the local offset, sleep on the client's executor), so saves fire while the window
is minimised or the tab hidden; an immediate retry on `online`; the desktop flushes when its window becomes invisible;
no `pm_json`; the React outbox adopted and its entry deleted after the POST that carried it, the React board snapshots
read from `keyval-store`/`keyval` and deleted after Restore or Discard (§19.4.4, §19.4.5); the board snapshot also on
hide, `pagehide`, 401 and every 2 s for boards up to 2 MiB; `upload` with progress (§19.4.1); `LocalTransport` mints a
new desktop session on a 401 and retries once (D105); retries only for network and 5xx, at most twice; ETag 304 keeps
the cache; optimistic updates roll back on error.

**Acceptance.**

```sh
scripts/dev.sh cargo test -p wikanvas-client --locked                     # L1: every case of docSession, saveLoop, unsaved, pending, client and the cache tests, ported
scripts/dev.sh cargo test -p wikanvas --locked --test flows -- client::   # L3 on LocalTransport, and over HttpTransport to TestServer; client::minimised_window_still_saves, client::desktop_session_survives_password_change
scripts/dev.sh cargo clippy --locked --target wasm32-unknown-unknown -p wikanvas-client -- -D warnings
out="$(scripts/dev.sh cargo tree --locked -p wikanvas -e normal --prefix none)" && ! grep -qE '^(rustls|ureq|native-tls|openssl) ' <<<"$out"
```

### C2: shell and sidebar (`wikanvas-app`: shell, routes, tree, dialogs, theme)

**Goal.** The app's frame: routing (web History through C8's `Platform`, desktop in-app history), the shell and its
breakpoints, the sidebar tree with drag and drop and keyboard, dialogs, menus, toasts, global keys, theme tokens,
icons, fonts, i18n, zoom, and the AccessKit roles of all of them.

**Owns.** `crates/wikanvas-app/src/{app,route,shell,sidebar,tree,dialogs,menus,toasts,hotkeys,theme,icons,i18n,zoom,a11y,prefs}*`,
`crates/wikanvas-app/assets/icons/**`, `crates/wikanvas-app/tests/ui/{routing,shell,sidebar,dialogs,menus,toasts,hotkeys,theme,a11y,zoom}.rs`,
`crates/wikanvas-app/tests/snapshots/{sidebar,focus}*`, `docs/handoff/C2.md`.

**Parity items:** groups A, C, D, Q, R, S, T, U, V (except P-193), and P-197, P-198.

**Key behaviours.** The tree model ports test for test (buildTree 7 cases, hitZones 10, positions 10, keyboard 11,
visibleRows 6, invalidTargets 4, nonManualDrop 4) with `fixtures/tree/title-order.json` shared; the sort key from
`wikanvas_core::tree::title_key`; menus and dialogs paint in `Order::Foreground` over the board (P-139); the hidden
sidebar is absent from the AccessKit tree; `localStorage` keys become eframe storage keys, read once from the React
client's `wikanvas.ui`, `wikanvas.theme` and `wikanvas.sidebar.expanded` (§19.4.4); `hotkeys::TABLE` is the one source
of the global shortcuts and of the chords whose browser action `wikanvas-web` suppresses (D140); Mod+P opens the read
view (web) or "Print" (desktop).

**Acceptance.**

```sh
scripts/dev.sh cargo test -p wikanvas-app --locked -- route:: tree:: theme:: icons:: i18n:: prefs::   # L1
scripts/dev.sh cargo test -p wikanvas-app --locked --test ui -- routing:: shell:: sidebar:: dialogs:: menus:: toasts:: hotkeys:: theme:: a11y:: zoom::
scripts/dev.sh env WIKANVAS_SNAPSHOTS=require cargo test -p wikanvas-app --locked snapshot::sidebar snapshot::focus
scripts/dev.sh cargo clippy --locked --target wasm32-unknown-unknown -p wikanvas-app -- -D warnings
```

### C3: pages and settings (`wikanvas-app`: pages)

**Goal.** Every page except the document body and the board surface: the auth gate (web 401 → `/login?next`, D114),
the item header and title autosave, item actions and the item menu (with "Open reading view" and "Copy reading link"),
collections, Home, Starred, Archive, Search, the quick switcher, Settings (account, appearance, workspace, tokens, data,
about), Trash, the document page around the editor (read-only banner, nested list, exports).

**Owns.** `crates/wikanvas-app/src/pages/**`, `crates/wikanvas-app/tests/ui/{auth,items,topbar,collections,lists,search,switcher,settings,trash,docs,pages}.rs`,
`crates/wikanvas/tests/flows/{auth,items,lists,search,settings,trash}.rs`, `docs/handoff/C3.md`.

**Parity items:** groups B (except P-013, P-014, P-016, P-018, P-019), E, F, G, H, M, N, O, and P-110-P-112.

**Acceptance.**

```sh
scripts/dev.sh cargo test -p wikanvas-app --locked -- time:: search::
scripts/dev.sh cargo test -p wikanvas-app --locked --test ui -- auth:: items:: topbar:: collections:: lists:: search:: switcher:: settings:: trash:: docs:: pages::
scripts/dev.sh cargo test -p wikanvas --locked --test flows -- auth:: items:: lists:: search:: settings:: trash::
```

### C4: the editor core (`wikanvas-editor`)

**Goal.** §19.5.1-§19.5.6 and §19.5.8: the mirror, positions, primitives, the text and structure commands, layout,
virtualisation, painting, selection, input and IME, the keymap, the clipboard with HTML import and export, undo, the
input rules, the placeholder, the title link, read-only mode, the AccessKit nodes; the editor inside a rect at a zoom
(for C6).

**Owns.** `crates/wikanvas-editor/src/{model,pos,prim,commands,layout,paint,select,input,ime,keymap,clipboard,html_import,undo,input_rules,a11y,host}*`,
`crates/wikanvas-editor/tests/{corpus,typing,selection,delete,ime,undo,clipboard,keymap,a11y}.rs`,
`crates/wikanvas-editor/examples/bench.rs`, `crates/wikanvas-core/src/convert/html_dom.rs` (the html5ever
tree C7's tests also use), `fixtures/ydoc/egui/**` (the corpus the egui editor writes, with its `count.txt`),
`fixtures/html/**` (the paste corpus), one additive change to `web/src/editor/serverWritten.test.ts` (also read
`fixtures/ydoc/egui/*.bin`, and fail when that directory holds fewer pairs than its `count.txt`), `docs/handoff/C4.md`.

**Parity items:** P-080-P-086, P-088, P-103, P-104, P-107, P-108, P-113, P-115 (owner; nothing built in M2, weight 0),
P-200.

**Key behaviours.** Only the nine primitives write yrs (§19.5.2); opening never writes; no galley kept across frames;
an empty fragment is a virtual paragraph; preedit never enters yrs; undo tracks only `wikanvas.local` and groups on the
injected `Clock`; the later of two blocks sharing a `blockId` gets a new one when edited; a paste of
`__inkboard_objects__` is ignored. The gate checks deterministic proxies for G10's budgets (opening a 5,000-block
document lays out at most 120 blocks; a keystroke in it re-lays out at most 3 blocks; a scroll frame lays out at most
the blocks of one screen plus one), and the bench with a guard band of 2× S0's budgets.

**Acceptance.**

```sh
scripts/dev.sh cargo test -p wikanvas-editor --locked                    # L1 (proptests: 10,000 sequences, 1,000 convergence runs) and L2
scripts/dev.sh cargo test -p wikanvas-editor --locked --test corpus      # every fixtures/ydoc .bin read; open-and-close writes nothing; goldens written
scripts/dev.sh pnpm --dir web test src/editor/serverWritten              # TipTap opens every egui-written .bin without repair (during the port); fails on a short corpus
scripts/dev.sh cargo clippy --locked --target wasm32-unknown-unknown -p wikanvas-editor -- -D warnings
scripts/dev.sh cargo run --release -p wikanvas-editor --example bench -- long --json /work/.cache/bench/long.json && jq -e '.open_5000_native_ms < 300 and .keystroke_p95_native_ms < 8 and .scroll_p95_native_ms < 16' .cache/bench/long.json   # 2× G10's native budgets; numbers into docs/handoff/C4.md
```

### C5: editor blocks and menus (`wikanvas-editor`)

**Goal.** §19.5.7: code blocks with the language picker and Inkboard's highlighter (D136), links (allow-list, follow,
popup, autolink, item URL paste, the context menu), task lists, tables, images (load, upload with progress, width, alt,
caption, external card), callouts, mentions, board embeds (the resvg preview, D123), the slash and mention menus, the
bubble and table menus, the block handle with drag reorder, Contents and scroll spy, find (D135).

**Owns.** `crates/wikanvas-editor/src/{blocks,links,tables,images,mentions,embeds,menus,slash,contents,find}/**`,
`crates/wikanvas-editor/tests/{blocks,links,tables,images,mentions,menus,find}.rs`,
`crates/wikanvas/tests/flows/{docs,links}.rs` (`links` holds smoke step 16's item links, Back and the low row menu, and
the desktop's own-URL paste), `docs/handoff/C5.md`.

**Parity items:** P-087, P-089-P-102, P-105, P-106, P-109, P-195, P-199.

**Acceptance.**

```sh
scripts/dev.sh cargo test -p wikanvas-editor --locked --test blocks --test links --test tables --test images --test mentions --test menus --test find
scripts/dev.sh cargo test -p wikanvas --locked --test flows -- docs:: links::
```

### C6: boards and cards (Inkboard 0008/0009 final, `wikanvas-board`)

**Goal.** §19.6: the finished widget patch and its tests, the board page (top bar, exports, nested pages, Focus,
read-only, theme), board switching with the previous save awaited, the conflict banner and its two choices, the unsaved
restore prompt, the large-board warning, the card painter (static cards with the read-only editor, the editing card),
Text doc, Embed page…, the sidebar drop, the title map, the Text doc clean-up, board previews for documents (resvg),
including the on-demand preview of a board whose `preview_rev` is null (the seeded Welcome board, smoke step 16).
0008's final form keeps main's card rules (§18.14's D93-D98: the middle button pans while a card is edited, a secondary
press ends the editing, the cards on the clipboard are kept, a refused save ends the editing, no style panel while
editing, drops titled from the item's own detail) in their in-process form (§19.6.3).

**Owns.** `patches/inkboard/**` (R17), `scripts/inkboard.sh` (if a change is needed), `crates/wikanvas-board/**`,
`crates/wikanvas/tests/flows/{boards,board_embeds}.rs`, `fixtures/boards/**` (additive), `fixtures/snapshots/inkboard/**`,
`docs/handoff/C6.md`.

**Parity items:** P-125-P-127, P-130, P-132-P-154, P-193.

**Acceptance.**

```sh
scripts/dev.sh scripts/inkboard.sh src && scripts/dev.sh scripts/inkboard.sh check
scripts/dev.sh bash -c 'cd .cache/inkboard-src && CARGO_TARGET_DIR=/work/.cache/inkboard-target cargo test --locked -p inkboard-core -p inkboard-formats -p inkboard-app'
scripts/dev.sh env INKBOARD_SNAPSHOTS=1 INKBOARD_SNAPSHOT_DIR=/work/fixtures/snapshots/inkboard bash -c 'set -o pipefail; cd .cache/inkboard-src && CARGO_TARGET_DIR=/work/.cache/inkboard-target cargo test --locked -p inkboard-app tests_visual -- --nocapture 2>&1 | tee /work/.cache/tests_visual.log' && ! grep -q SKIPPED .cache/tests_visual.log
scripts/dev.sh scripts/inkboard.sh export --upstream && ! grep -rq 'Changed for' .cache/inkboard-upstream
scripts/dev.sh cargo test -p wikanvas-board --locked
scripts/dev.sh cargo test -p wikanvas --locked --test flows -- boards:: board_embeds::
scripts/dev.sh cargo clippy --locked --target wasm32-unknown-unknown -p wikanvas-board -- -D warnings
```

### C7: the server (read view, forms, `serve`, desktop identity, agents)

**Goal.** §19.7 and the server half of §19.8: `convert::html_out` and the `/read/*` pages with `read.css`; `/login`,
`/setup`, `POST /logout`, `/login/once`; `GET …/doc/html`; `item_ref` for `/read/` paths; the revision preview from Y;
`serve`'s flags, defaults, data-dir lock, cookie names and LAN refusal and the setup link; the owner without a
password; no schema change (D143); the server half of the desktop's agent
access (the token with `created_via = 'admin'` and the `settings` key, its revocation; C8 writes the discovery file);
the CLI's `desktop` URL source and `doc get --format html`; the app shell per path (`/next/` during the
port) with the app CSP and the assets under `/app/{build}/` (D141).

**C7a may start during S0** on today's crates (`crates/wikanvas-core`, `crates/wikanvas-server`), limited to the read
view, the forms and `doc/html`, because they serve the React client too and do not depend on the spike. It then rebases
onto F0. The rest of C7 starts after F0.

**Owns.** `crates/wikanvas-core/src/convert/html_out.rs` and its tests; `crates/wikanvas-server/src/{read,pages,serve,desktop}*`,
the `routes/docs.rs` additions, `config.rs`, `extract.rs`'s cookie function, `static_files.rs`, `headers.rs`;
`crates/wikanvas-store/src/{docs,users,tokens,revisions,db}.rs` additions (no new migration, D143);
`crates/wikanvas-cli/**` (the two additions and the discovery-file tests); `fixtures/ydoc/**/*.html` (goldens);
`crates/wikanvas-server/tests/api/{read_view,pages,serve,desktop}.rs`;
`docs/AGENTS.md`; `e2e/agent/scenarios/13-*.sh`; `docs/handoff/C7.md`.

**Parity items:** P-013, P-014, P-019, P-114, P-196, P-202.

**Acceptance.**

```sh
scripts/dev.sh cargo test -p wikanvas-core --locked -- convert::html_out::      # goldens for 19 fixtures, the well-formedness proptest
scripts/dev.sh cargo test -p wikanvas-server --locked --test api -- read_view:: pages:: serve::   # 303s, CSP exactly, no "<script", ETag/304, trashed 404, html_dom structure and unique ids, forms and CSRF, cookie names, LAN refusal, the lock, /app/{other build} 404
scripts/dev.sh cargo test -p wikanvas-server --locked --test api -- pages::login_once_single_use_and_expiry desktop::agent_token_revoked_at_exit_and_next_start
scripts/dev.sh cargo test -p wikanvas-store --locked                             # an M1 database used by M2 keeps user_version 1; owner without password
scripts/dev.sh cargo test -p wikanvas-server --locked --test api -- openapi:: && git diff --exit-code docs/api/openapi.json
scripts/dev.sh cargo test -p wikanvas-cli --locked                              # discovery_file_refusals (not owned, not 0600, dead pid)
scripts/image.sh && scripts/agent-test.sh                                        # scenarios 01-12 unchanged, 13: doc get --format html
```

### C8: desktop and web entries, packaging

**Goal.** The `wikanvas` binary's desktop mode (first-run dialog, window close flush, Ctrl+Q through the flush, rfd save
dialogs, "Open in browser" through `/login/once`, the opt-in agent listener and discovery file, the second-instance
refusal, the minimise flush, the log file, the panic hook and the restore dialog, the stable listener port, the
desktop's link rules, `--screenshot`); `wikanvas-web` (the boot loader, History router under the base path, the
shortcut listener, `beforeunload` and `online`, clipboard listeners, file picks and drops streamed as `web_sys::File`,
XHR uploads with progress, the IndexedDB store glue, visibility and `pagehide` flushes with keepalive, the update check,
`test-bridge`); the Docker stages; `scripts/dist.sh` with cargo-zigbuild for x86_64 and aarch64 (glibc 2.35), `.deb`
and `.tar.gz`, each with its licence files (§19.8.8); `wikanvas-tools licenses` and the licence list for About (P-167,
§19.13); `scripts/desktop-smoke.sh`.

**Owns.** `crates/wikanvas/src/{main,desktop,single,agents,dialogs,logs}*`, `crates/wikanvas/tests/flows/{setup,desktop}.rs`
(the desktop first run, smoke step 1's desktop half, the second instance, the crash note), `crates/wikanvas-web/**`,
`crates/wikanvas-tools/**`, `docker/Dockerfile`'s `web-app`, `runtime`, `licenses` and `dist` stages,
`scripts/{dist,desktop-smoke,wasm-app}.sh`, `docs/handoff/C8.md`.

**Parity items:** P-201 (and the platform halves of P-008, P-024, P-122, P-123, P-166, P-167 that their owners request).

**Acceptance.**

```sh
scripts/dev.sh cargo clippy --locked --target wasm32-unknown-unknown -p wikanvas-web --features test-bridge -- -D warnings
scripts/dev.sh scripts/wasm-app.sh size && jq -e '.br <= 5000000' .cache/wasm-app/sizes.json   # the budget agreed at S0 (G11)
scripts/image.sh && WIKANVAS_SMOKE_LEVEL=full scripts/image-smoke.sh
scripts/desktop-smoke.sh
scripts/dev.sh cargo test -p wikanvas --locked --test flows -- setup:: desktop::
test "$(wc -l < crates/wikanvas-web/boot/boot.js)" -le 80                       # D139's budget
scripts/dist.sh && scripts/dev.sh bash -c 'set -o pipefail; readelf -d dist/wikanvas-x86_64-unknown-linux-gnu | sed -n "s/.*Shared library: \[\(.*\)\]/\1/p" | sort > /work/.cache/needed.txt' && test -s .cache/needed.txt && test -z "$(comm -23 <(sort -u .cache/needed.txt) <(printf '%s\n' ld-linux-x86-64.so.2 libc.so.6 libgcc_s.so.1 libm.so.6 | sort))" && scripts/dev.sh dist/wikanvas-x86_64-unknown-linux-gnu --version
scripts/e2e.sh smoke --only B1,B5,B6,B8,B12,B13,B14,B15,B16
```

### C9: tests and parity

**Goal.** §19.11 and the parity proof: `wikanvas-testkit`; `flows::smoke_sequence` and one flow per smoke step
(`docs/parity-seed.md`, "Old tests"); the browser smoke B1-B16 in `wikanvas-browser-test` run by `scripts/e2e.sh smoke`;
the snapshot set; `scripts/ui-peek.sh`; the server-state digest (`wikanvas-testkit`'s `digest` binary over the CLI's
JSON) and `scripts/parity.sh` (every section: items, vitest call sites, new behaviours, manual checks; each named Rust
test must exist in `cargo test -- --list`); the React smoke's digest hook (`scripts/e2e.sh smoke-react --digest`); the
ledger.

**Owns.** `crates/wikanvas-testkit/**`, `crates/wikanvas-browser-test/**`, `crates/wikanvas/tests/flows/{main,smoke}.rs`,
`crates/wikanvas-app/tests/snapshots/` (the harness; screens' references belong to their owners), `scripts/{e2e,ui-peek,parity}.sh`,
`e2e/tests/smoke.spec.ts` (the digest hook only), `docs/parity.md`, `docs/handoff/C9.md`.

The browser smoke's checks (each reads the bridge, never sleeps; CDP `Runtime.evaluate` expressions are test code):

| Id | Check |
|---|---|
| B1 | `/` (and `/next/` during the port) boots the wasm under the app CSP with no violation; a WebGL2 context, renderer string recorded; `/setup` is the HTML form; setup typed with real key events → the app's Home; the session cookie is HttpOnly |
| B2 | Sign out through the form; sign in with "Keep me signed in": the cookie's `Max-Age` holds more than 24 h |
| B3 | A new document; "# Plan", Enter, "quokka" typed by CDP keys and one IME composition committed; `idle()`; a real reload; the text is still there (bridge tree and `GET …/text`) |
| B4 | A new board; R and a real mouse drag; `PUT …/board` 200; idle for 2 s with `frames()` flat |
| B5 | A deep-link reload on `/doc/<slug>-<id>`; Ctrl+click on an item link opens it; browser Back returns |
| B6 | Emulated `visibilityState = hidden` flushes the pending save; a board opened while hidden paints once visible; a board edited 0.5 s before a navigation away fires the leave prompt (CDP `Page.javascriptDialogOpening`, type `beforeunload`); after accepting it, the next open of the board offers "Restore unsaved changes" with that edit |
| B7 | Export Markdown is a real download whose first heading is `# Plan`; the page's own `fetch('/api/v1/export')` is a tar with `manifest.json`; the same GET without fetch metadata is `csrf_rejected` |
| B8 | The bridge's `panic()` shows the "stopped" screen, the outbox is kept, a reload restores the edit; a React `wikanvas.outbox` entry put into IndexedDB before boot is adopted and deleted after its POST; a React snapshot `wikanvas.board.unsaved/<id>` put into `keyval-store`/`keyval` leads to the restore prompt and is deleted after it |
| B9 | CDP offline: status "Offline", an edit queued; online: flushed at once (before the next back-off step) |
| B10 | The read view of the document: `h1` "Plan" and "quokka" in `document.body.innerText`, no script element, its CSP, print CSS present |
| B11 | Copy in the editor puts `text/html` with `data-wikanvas-slice` on the clipboard; pasting HTML from the corpus gives its golden |
| B12 | A phone viewport (coarse pointer, 390 × 844) opens the read view with "Open editor", and no request for the wasm is made |
| B13 | Chromium started with WebGL disabled (`--disable-webgl`), and once with WebAssembly disabled (`--js-flags=--noexpose_wasm`): the error panel and the reading link show, and no request for the wasm is made |
| B14 | The first request for the wasm answers 500 (CDP `Fetch.fulfillRequest`): the loader retries once and boots; when both answer 500, the error panel shows the reading link and "Try again" |
| B15 | In the editor and on a board: Mod+K opens the switcher, Mod+F opens find, Mod+B makes the selection bold, Mod+U and Mod+E and Mod+Shift+B run their app actions; for each, the `keydown` is `defaultPrevented` (a bubble-phase probe added through CDP) and `document.hasFocus()` stays true |
| B16 | An image upload through a CDP file drop shows the progress placeholder with a value between 0 and 1 before the image appears (the upload throttled with `Network.emulateNetworkConditions`) |

**Acceptance.**

```sh
scripts/dev.sh cargo test -p wikanvas-testkit --locked
scripts/dev.sh cargo test -p wikanvas --locked --test flows                    # every module, smoke_sequence included
scripts/image.sh && scripts/e2e.sh smoke                                       # B1-B16
scripts/e2e.sh smoke-react --digest out/react.json && scripts/parity.sh       # digests equal, no `missing` row
scripts/dev.sh bash -c 'for i in $(seq 20); do cargo test -p wikanvas --locked --test flows -q || exit 1; done'   # a new suite passes 20 times in a row when it lands
```

---

### I: integration (dual run)

On main, by the orchestrator or one integrating agent, after C1-C9 are done (R10):
1. Merge the branches in the order they finish (`git merge --no-ff`), lockfile branches one at a time (R6); after each
   merge, rerun that workstream's acceptance on main.
2. Apply the handoff notes' requests; fold their deviations into ARCHITECTURE §19 (a decision row each) and this
   section.
3. Run the M2 release gate (below) with the dual-run lines, twice in a row, both clean. A failure is fixed at its
   cause or reported as a blocker; nothing is skipped, ignored or loosened.
4. Hand the owner an image to run on the live wiki, named by its exact tag `wikanvas:<commit>` (R16: the owner sets
   `WIKANVAS_IMAGE` to it in `.env` and switches, after the backup of §19.13's rollback steps), with the egui client at
   `/next/` and the desktop binary.
5. Check R20: every file with a "ported from" or "borrowed from" header has its `THIRD_PARTY.md` row.

**Owns.** `docs/ARCHITECTURE.md`, `docs/PLAN.md`, `README.md`, `THIRD_PARTY.md`, `CLAUDE.md`, `docs/handoff/M2-merge.md`,
and fixes found by the gate in any file, each as its own commit with a test.

### X: the cutover

**When.** All of: two consecutive clean dual-run gates; `docs/parity.md` without a `missing` row; equal server-state
digests; every manual check of `docs/parity.md` recorded in `docs/handoff/M2-merge.md`; and the owner's sign-off after using the egui client on the live wiki (default: 5 working days, §19.15
question 25).

**Steps, in one branch, merged alone.**
1. Run `scripts/dev.sh pnpm --dir web fixtures` one last time; its outputs in `fixtures/ydoc/` stay committed as frozen,
   read-only goldens (the corpus test reads them for ever).
2. Delete what §19.13 lists: `web/`, `web/inkboard-frame/`, `e2e/tests/frame.spec.ts`, the DOM smoke spec, `e2e/package.json`
   and Node in the toolchain (if B1-B16 run on chromiumoxide), ts-rs and its feature and env lines, the `node` and `web`
   Docker stages (the `licenses` stage loses its npm input), the `/inkboard/{build}/` route, the frame and SPA CSPs, the
   two old embeds of `wikanvas-assets`.
3. Move the egui app from `/next/` to `/`; `/next/*` answers 308 to the path without the prefix.
4. Re-cut the Inkboard series (§19.6.6, C6's owner or the cutover agent): patchset 5, the JS API and its docs gone.
5. Remove the dual-run gate lines.
6. Docs: ARCHITECTURE §9, §10, §11, §14 and §18 rewritten or marked as replaced by §19; AGENTS.md; README.
7. The live wiki: no data migration and no schema change (D143); the database file keeps its name `wikanvas.db`
   (§19.8.10); `compose.yaml` keeps its names and environment keys (D142); browsers on HTTPS stay signed in (UNVERIFIED,
   inferred from the cookie names and not tested: the name does not change there, §19.8.3); the web client adopts the
   React outbox and board snapshots (§19.4.4, §19.4.5). The owner keeps the rollback steps of §19.13 at hand.

**Acceptance.** The M2 release gate below without the dual-run lines, on the cutover branch and again on main after the
merge.

### Acceptance (the M2 release gate)

On main, in this order, every `scripts/dev.sh` command with `WIKANVAS_SHARED_CARGO_HOME` set (the name stays, R19). Each
negated check captures its producer first or asserts its paths exist, so a failing producer fails the line (Rules, "A
skipped check is a failed check"):

```sh
scripts/install-build-slice.sh --check
scripts/dev.sh scripts/inkboard.sh src && scripts/check-pins.sh     # R12: one egui; GUI crates equal to the fresh patched lock
docker compose -f compose.yaml config --format json | jq -e '.name == "wikanvas" and (.services | has("wikanvas")) and ((.services.wikanvas.ports // []) == [])'   # D142: names unchanged, no published app port
! grep -qE '^\s*(encode|header)\b' docker/Caddyfile
scripts/fmt.sh --check
scripts/dev.sh cargo clippy --workspace --all-targets --locked -- -D warnings
scripts/dev.sh cargo clippy --locked --target wasm32-unknown-unknown -p wikanvas-api -p wikanvas-core -p wikanvas-client -p wikanvas-editor -p wikanvas-board -p wikanvas-app -p wikanvas-web -- -D warnings
scripts/dev.sh cargo clippy --locked --target wasm32-unknown-unknown -p wikanvas-web --features test-bridge -- -D warnings
scripts/dev.sh cargo tree --locked -e normal -i libsqlite3-sys
scripts/dev.sh cargo test --workspace --locked                      # L1, L2, L3, L7, the server, store, core and CLI tests
scripts/dev.sh env WIKANVAS_SNAPSHOTS=require cargo test -p wikanvas-app --locked snapshot::   # with `require`, a snapshot that cannot render fails instead of skipping
scripts/dev.sh cargo test -p wikanvas-editor --locked --test corpus
scripts/dev.sh env WRITE_FIXTURES=1 cargo test -p wikanvas-core --locked -- convert::tests::write_server_fixtures && git diff --exit-code fixtures/ydoc/server
scripts/dev.sh cargo test -p wikanvas-server --locked --test api -- openapi:: && git diff --exit-code docs/api/openapi.json
scripts/dev.sh cargo test -p wikanvas-cli --locked
out="$(scripts/dev.sh cargo tree --locked -p wikanvas -e normal --prefix none)" && ! grep -qE '^(rustls|ureq|reqwest|native-tls|openssl|iroh) ' <<<"$out"      # D29, D129, D138
out="$(scripts/dev.sh cargo tree --locked -p wikanvas-cli -e normal --prefix none)" && ! grep -qE '^(openssl|openssl-sys|native-tls|rusqlite|yrs|inkboard-core|comrak|webpki-roots|egui) ' <<<"$out"
out="$(scripts/dev.sh cargo tree --locked -p wikanvas-app -e normal --prefix none)" && ! grep -qE '^egui_kittest ' <<<"$out"
test -d crates/wikanvas-server/src/routes && ! grep -rn '\.route(' crates/wikanvas-server/src/routes
test -d crates/wikanvas-server/src && ! grep -rn '\.route("/api' crates/wikanvas-server/src
d="crates/wikanvas-client/src crates/wikanvas-app/src crates/wikanvas-editor/src crates/wikanvas-board/src"; for x in $d; do test -d "$x" || exit 1; done; ! grep -rnE 'Instant::now|SystemTime::now|Date::now|performance\(\)\.now|Performance::now|Local::now|Utc::now' $d | grep -v '/clock.rs:'   # R14
test -d crates && ! find crates \( -name '*.js' -o -name '*.ts' -o -name '*.mjs' \) -not -path crates/wikanvas-web/boot/boot.js | grep -q .   # R15
test "$(wc -l < crates/wikanvas-web/boot/boot.js)" -le 80                                                                             # D139
test -d crates && ! grep -rn '<script' crates --include='*.rs' | grep -v 'type="module" src="/app/{build}/boot.js"'                                    # R15: no other script in Rust strings
scripts/dev.sh scripts/inkboard.sh check
scripts/dev.sh bash -c 'cd .cache/inkboard-src && CARGO_TARGET_DIR=/work/.cache/inkboard-target cargo test --locked -p inkboard-core -p inkboard-formats -p inkboard-app'   # all of them, not only embed and i18n
scripts/dev.sh env INKBOARD_SNAPSHOTS=1 INKBOARD_SNAPSHOT_DIR=/work/fixtures/snapshots/inkboard bash -c 'set -o pipefail; cd .cache/inkboard-src && CARGO_TARGET_DIR=/work/.cache/inkboard-target cargo test --locked -p inkboard-app tests_visual -- --nocapture 2>&1 | tee /work/.cache/tests_visual.log' && ! grep -q SKIPPED .cache/tests_visual.log
scripts/dev.sh scripts/inkboard.sh export --upstream && ! grep -rq 'Changed for' .cache/inkboard-upstream
scripts/dev.sh cargo run --release -p wikanvas-editor --example bench -- long --json /work/.cache/bench/long.json && jq -e '.open_5000_native_ms < 300 and .keystroke_p95_native_ms < 8 and .scroll_p95_native_ms < 16' .cache/bench/long.json   # 2× G10's native budgets
scripts/dev.sh scripts/wasm-app.sh size && jq -e '.br <= 5000000' .cache/wasm-app/sizes.json
compgen -G '.cache/wasm-app/dist/*.wasm' >/dev/null && compgen -G '.cache/wasm-app/dist/*.js' >/dev/null && ! grep -lq __wikanvas_test .cache/wasm-app/dist/*.wasm .cache/wasm-app/dist/*.js   # the test bridge never ships (local dist)
scripts/image.sh
WIKANVAS_SMOKE_LEVEL=full scripts/image-smoke.sh                       # app caps, Caddy, the app wasm as br byte for byte, the app CSP, the read view's CSP, /app/{other build} 404; the identity wasm and glue served by the image contain no __wikanvas_test; /app/{build}/licenses.txt lists web-sys and wasm-bindgen
scripts/e2e.sh smoke                                                # B1-B16 against the image
scripts/e2e.sh smoke --startup                                      # G11's cold and warm start, ≤ 2× S0's budgets (6.0 s, 3.0 s)
scripts/desktop-smoke.sh                                            # L6
scripts/agent-test.sh                                               # scenarios 01-13, against the image just built
scripts/cli-dist.sh && scripts/dev.sh dist/cli/wikanvas-cli-x86_64-unknown-linux-musl --version
scripts/dist.sh && scripts/dev.sh bash -c 'set -o pipefail; readelf -d dist/wikanvas-x86_64-unknown-linux-gnu | sed -n "s/.*Shared library: \[\(.*\)\]/\1/p" | sort > /work/.cache/needed.txt' && test -s .cache/needed.txt && test -z "$(comm -23 <(sort -u .cache/needed.txt) <(printf '%s\n' ld-linux-x86-64.so.2 libc.so.6 libgcc_s.so.1 libm.so.6 | sort))" && scripts/dev.sh dist/wikanvas-x86_64-unknown-linux-gnu --version   # NEEDED ⊆ the four (D115)
test -d crates && test -d scripts && test -d e2e/agent && ! grep -rnE 'todo!\(|unimplemented!\(|TODO\((W|E|S0|F0|C[1-9])|not implemented' crates scripts e2e/agent
git -C vendor/inkboard status --porcelain | wc -l | grep -qx 0
```

During the dual run only (removed by X):

```sh
scripts/dev.sh pnpm --dir web install --frozen-lockfile
scripts/dev.sh pnpm --dir web fixtures && git diff --exit-code fixtures
scripts/dev.sh pnpm --dir web typecheck && scripts/dev.sh pnpm --dir web test && scripts/dev.sh pnpm --dir web build
scripts/dev.sh pnpm --dir web test src/editor/serverWritten        # TipTap opens every egui-written .bin
scripts/dev.sh cargo test -p wikanvas-api --locked --features ts export_bindings && git diff --exit-code web/src/api/gen
scripts/dev.sh scripts/inkboard.sh build && scripts/e2e.sh frame
scripts/e2e.sh smoke-react --digest out/react.json
scripts/parity.sh                                                   # no `missing` row; out/react.json equals out/egui.json
```

Today's whole gate took 14 minutes (`docs/handoff/E.md:7-8`); S0's G12 timings (spike S5 of the testing research) set
the budget for the new suites. A budget of 20 minutes for the M2 gate is the target; a slower gate is reported, not
trimmed.

---

### Review log (2026-10-29)

Each finding of the review of §19, this plan and `docs/parity-seed.md` was checked against the code, the crate sources
(egui, eframe, yrs, arboard, axum, idb-keyval) and crates.io before it was fixed or rejected. While the review ran, the
other workflow landed the M1.1 review on main (`d5d3c94` → `9c6e60a`: §18.14's D93-D99, 0005 at 2,014 lines, 26 frame
tests), so the three drafts were also re-cited against `9c6e60a`. 51 findings: 51 fixed (one of them with a wrong piece
of evidence corrected), 0 rejected.

| # | Finding | Lens | Severity | Disposition | Why, and where |
|---|---|---|---|---|---|
| 1 | The editing card's child `Ui` shares the canvas's layer, so presses inside it reach the board | feasibility | high | fixed | Holds: egui hides a covered widget only across layers (0.37 `src/hit_test.rs:142-152`) and `canvas_pointer` uses `contains_pointer` (`tools.rs:750-766`). 0008 adds `pointer_in_editing_card` for presses, drags, the context menu and the laser (§19.6.3, §19.6.5); G2 and `tests_widget` test a press, a drag and a right-click inside the card |
| 2 | Timers on egui's input time stall while the window is minimised or the tab hidden | feasibility | medium | fixed | Holds (egui 0.37 `src/context.rs:1148-1174`). `Clock` now has a monotonic time, the wall time and a sleep on the client's executor (D106, §19.4.2); the desktop flushes when its viewport becomes invisible (§19.4.4, §19.8.4); flow `client::minimised_window_still_saves` |
| 3 | G3 (3) re-runs the server-fixture test and never opens a spike `.bin` | feasibility | medium | fixed | Holds (`serverWritten.test.ts:8`). New spike test `spikeWritten.test.ts` reads `SPIKE_YDOC_DIR` and checks the exact count; G3's command changed; S0 owns it; C4's change to `serverWritten` also fails on a short corpus |
| 4 | G15 tests only zooms that are multiples of 1/16; the pinch rule contradicts D108 and D109 | feasibility | medium | fixed | Holds. Buckets are now powers of 2^(1/16), floor, painted unscaled, never wider than the frame; during a pinch at most 6 static cards per frame move bucket and the rest reuse egui's cache, so no galley is kept (D109, §19.6.3); G15 adds zooms 0.53, 1.37, 2.97, 3.9 |
| 5 | The editor estimate implies 270-425 tested lines a day and hides the hardest work in two L items | feasibility | medium | fixed | Holds. The rate (300 lines plus tests per person-day, UNVERIFIED) is stated; C4 71-137 (likely 99) and C5 30-59 by lines; P-115 counts 0; P-084 and P-094 split into sized sub-items (13.5 and 16.5 days); S0 measures the real rate and F0 re-derives C4 and C5 (Overview, `docs/parity-seed.md` Totals) |
| 6 | yrs's undo timestamp is the wall clock natively and missing on wasm | feasibility | low | fixed | Holds (`yrs-0.28.0/src/undo.rs:239`, `:788-813`). `timestamp` is a closure over `Clock` (§19.5.6); G6 adds the 600 ms and 300 ms cases; R14 covers it |
| 7 | Inkboard's shortcut guard is true whenever any widget has focus, the canvas included | feasibility | low | fixed | Holds (`app.rs:809`; egui 0.37 `src/context.rs:3485-3487`). 0008 changes the guard to "a focused widget that is not the canvas" and the canvas requests focus on a press (§19.6.2, §19.6.5); G2 checks "r" after the canvas has focus |
| 8 | G1's label corpus is 11 short labels; Inkboard has no `.inkboard` fixtures | feasibility | low | fixed | Holds (3 boards, 11 labels; `find` gives 0). G1 uses a generated corpus of at least 500 labels with a 5 % limit per font family |
| 9 | The 0007 estimate ignores what 0001-0006 add, and 0005 changed on main | feasibility | low | fixed (evidence corrected) | Holds in substance: 6 sites in the patches' added lines need 0.37 changes (0001's two `ctx.style()`, 0005's `wants_keyboard_input`, `raw_scroll_delta` and the test's `MouseWheel` literal). The claim that 0.37 removed `MouseWheel::modifiers` is wrong: the field stays (egui 0.37 `src/data/input/event.rs:196-197`); the literal breaks on the new `phase` and `source`. 0007's size and contents updated (§19.6.5); main's secondary-press and middle-button rules folded into 0008 (§19.6.3); 0005 citations re-cut to `9c6e60a` |
| 10 | D102 overstates what egui 0.33 lacks | feasibility | low | fixed | Holds: `owns_ime_events` is `has_focus` (0.37 `src/memory/mod.rs:1068-1074`), which 0.33.3 has. D102 and G1's no-go now name the preedit range, `DeleteSurrounding`, `interrupt_ime`, the mobile fixes and harfrust |
| 11 | The desktop's 250 ms debounce assumes a save is cheap, but each POST rewrites derived data | feasibility | low | fixed | Holds (`crates/wikanvas-store/src/docs.rs:339-420`, `:269-284`). The desktop keeps D32's 1 s / 5 s; G13 records POST latency and writer hold time on a 20,000-block document (§19.4.4, P-118) |
| 12 | G13's envelope is unspecified (an image-heavy one passes trivially); G11 measures loopback only | feasibility | low | fixed | Holds (`model.rs:558-563`: `objects` is a deep-cloned `Vec`). G13 has an image-heavy and an object-heavy envelope with a no-go action; G11 adds a 20 Mbit/s, 40 ms run, reported |
| 13 | React board snapshots live in idb-keyval's default store, not `wikanvas.outbox` | feasibility | low | fixed | Holds (`unsaved.ts:5`; idb-keyval `dist/index.js:31`). §19.4.5 names `keyval-store`/`keyval`; B8 seeds a snapshot |
| 14 | Two concurrent rewraps of one block duplicate it with one `blockId` | feasibility | low | fixed | Accepted as an inference (not run). Named in §19.5.2; G9 records the case; the read view de-duplicates heading ids (§19.7.2); the editor renews the later id on edit (§19.5.1, N-19, N-20) |
| 15 | Four UNVERIFIED items can be settled from source | feasibility | low | fixed | All four hold: tower `util` (`axum-0.8.9/Cargo.toml:268-271`), arboard HTML read on three platforms, only a colspan in the fixtures, the text agent's only `set_value("")`. Marked verified (§19.4.1, §19.5.4, §19.5.5, §19.16); G7 adds a rowspan fixture |
| 16 | S0-a is one long chain with unlisted cross-track inputs | feasibility | low | fixed | Holds. The S0 table lists each track's inputs and days; S0-a gets two agents (one drafts 0008 from day 1); what moves if G1 takes its extension is stated |
| 17 | The web boot needs a hand-written loader that the design forbids | completeness | high | fixed | Holds. D139: one `boot.js` of at most 80 lines with feature checks, the phone check, progress, one retry and the error panel; goal 6 and R15 allow exactly it; B12-B14 test it (§19.7.6, §19.9) |
| 18 | App shortcuts also trigger the browser's own action (Mod+K, Mod+F, Mod+U, Mod+B) | completeness | high | fixed | Holds (eframe 0.37 `src/web/events.rs:237-279`). D140: a capture-phase listener from the one shortcut table; Mod+P bound; B15 (§19.5.4, §19.9, P-172, P-195) |
| 19 | Unsaved web work can be lost: no leave prompt, no `online` retry, the board snapshot only before a PUT | completeness | high | fixed | Holds (`pending.ts:56-67`, `saveLoop.ts:85-89`). `beforeunload` while pending, an immediate retry on `online`, the board snapshot also on hide, `pagehide`, 401 and every 2 s up to 2 MiB (§19.4.4, §19.4.5, §19.9); B6 and B9 extended |
| 20 | No rollback plan; the migration and a rename of the database file make going back harmful | completeness | high | fixed | Holds (`migrate.rs:53-60`, `db.rs:107`). No schema change in M2 (D143: the desktop token uses `created_via = 'admin'` and a `settings` key); M2 renames no database file (D127, §19.8.10: D100 renamed it once, at v0.1.0-alpha.1); the export entry stays `wikanvas.db` (D142); a rollback subsection with owner steps (§19.13, X step 7) |
| 21 | A revoked or expired desktop session cannot recover | completeness | medium | fixed | Holds (`routes/auth.rs:346`, `:416`). The session is minted at every start, in memory only, and again on a 401 (D105, §19.8.5); flow `client::desktop_session_survives_password_change` |
| 22 | Immutable app assets without a build id break the update banner | completeness | medium | fixed | Holds (§11 today relies on hashed paths). D141: `/app/{build}/`, `no-cache` shells, another build 404 (§19.9, P-010, N-14) |
| 23 | Upload progress cannot work through `fetch` | completeness | medium | fixed | Holds (`upload.ts:3`). `Transport::upload` with progress, `XmlHttpRequest` on the web (§19.4.1); B16 |
| 24 | No source of wall-clock time or the local time zone | completeness | medium | fixed | Holds. `Clock::wall` with the local offset from chrono 0.4.45 (`wasmbind` on wasm32) and `web-time`, both in a lock already; tests in a fixed zone (§19.4.2, §19.11, §19.12, P-075) |
| 25 | `flows::setup` and `flows::links` have no owner; four L2 tests sit in another owner's files | completeness | medium | fixed | Holds. `setup.rs` to C8, `links.rs` to C5, with their acceptance filters; P-007, P-180, P-181 and P-185 moved into C2's files; R18 says who writes a cross-owned test |
| 26 | The vitest mapping is folder-level wildcards | completeness | medium | fixed | Holds. `docs/parity-seed.md` has one row per vitest file (79, 350 call sites at `9c6e60a`); F0 expands it per call site; `scripts/parity.sh` checks every named test exists through `cargo test -- --list` |
| 27 | The new M2 behaviours have no named tests | completeness | medium | fixed | Holds. `docs/parity-seed.md` "New behaviour" (N-01-N-21) with named tests, in the F0, C1, C7 and C8 acceptance; `parity.sh` fails on a `missing` row there |
| 28 | The rename list misses the CLI profile path and says nothing of what new code writes | completeness | medium | fixed | Holds (`crates/wikanvas-cli/src/config.rs:12`, `:243`). The profile path moved with the rename (D100, no fallback) and stays in M2; M2 keeps writing `wikanvas://item/` and `qpat_` (§19.8.10, D127) |
| 29 | §16's manual checks are not carried over; no supported-browser statement | completeness | low | fixed | Holds. `docs/parity-seed.md` "Manual checks" maps each §16 item 26 and 31 check to a kittest or a recorded manual check before X, and states the supported browsers |
| 30 | The desktop has no logging or crash policy | completeness | low | fixed | Holds (`cli.rs:174-183`: stdout only). Per-start log files, a panic hook with a crash note, the restore dialog on the next start (§19.8.4, N-15) |
| 31 | Desktop links and ports are undefined | completeness | low | fixed | Holds. One listener with a port kept across restarts; reading links only while it runs, "Copy item link" otherwise; same-instance paste defined (§19.7.6, §19.8.6, P-091, P-106, N-16, N-17) |
| 32 | Adoption of the React client's local state is incomplete | completeness | low | fixed | Holds. The three `localStorage` keys are read once; the adopted outbox entry is deleted after its POST and the adopted snapshot after Restore or Discard (§19.4.4, §19.4.5, P-020, P-032, P-116, P-131); B8 extended |
| 33 | Smoke step 16's on-demand preview of the seeded board has no test | completeness | low | fixed | Holds (`smoke.spec.ts:770-782`). `flows::boards::seeded_board_preview_on_demand` in P-100 and the step-16 row |
| 34 | P-196 narrows autofill to sign-in and setup | completeness | low | fixed | Holds (`Account.tsx:85-93`, `ApiTokens.tsx:155`). Recorded as `changed: D114` for the password change and the admin token's prompt |
| 35 | Variables set in front of `scripts/dev.sh` never reach the container, so the visual tests skip | rules | high | fixed | Holds (`scripts/dev.sh:37-43`; `tests_visual.rs:48-50`). Every such line now uses `scripts/dev.sh env …`, runs with `--nocapture` and fails on `SKIPPED`; a rule says so |
| 36 | The `licenses` stage is `FROM web`; the list misses wasm32 crates and points at a deleted route | rules | high | fixed | Holds (`docker/Dockerfile:67`, `:198`, `:224`). F0 replaces it with `wikanvas-tools licenses` over both targets plus Inkboard's lists, fonts and icons from the patched tree; shipped in the image and every dist archive; the image smoke checks web-sys and wasm-bindgen (§19.13, §19.8.8) |
| 37 | scraper pulls in MPL-2.0 crates and is too new | rules | medium | fixed | Holds (crates.io: cssparser 0.38 and selectors 0.41 MPL-2.0; scraper 0.29.0 of 2026-10-21). Dropped; the read view's structure tests use `wikanvas_core::convert::html_dom` over html5ever (D125, §19.11, §19.12) |
| 38 | "No hand-written JS" cannot hold, and no inventory of non-Rust code exists | rules | medium | fixed | Holds. D139's loader is the one exception with a line budget; R15 and the gate allow exactly its path and grep Rust strings for `<script`; §19.12.1 lists every non-Rust part that ships |
| 39 | The font licences are missing from §19.12, and M2 widens their use | rules | medium | fixed | Holds (crates.io: `epaint_default_fonts` is `(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0`). Font rows added; question 13 asks about the wider scope; dist archives ship the licence texts and `dist.sh` checks them; the current state kept as CLAUDE.md asks |
| 40 | Negated pipelines pass when the producer fails or a glob matches nothing | rules | medium | fixed | Holds. Every such gate line now captures the producer or asserts its paths; the test-bridge check also runs on the image's served wasm (Rules, the M2 gate, C1) |
| 41 | Renaming compose names would move the live wiki to empty volumes; the jq check can pass vacuously | rules | medium | fixed | Holds (`compose.yaml:1`, `:3`, `:30`). `compose.yaml` keeps every name and environment key in M2 (D142, R16); the jq check asserts the project and the `wikanvas` service exist |
| 42 | Every image build retags `wikanvas:latest`, which the live wiki runs | rules | medium | fixed | Held until the v0.1.0-alpha.1 cut-over (`image.sh:33-35`), which pinned the live wiki's image with `WIKANVAS_IMAGE` in `.env`; from F0 `:latest` comes only from main; the spike image has its own tag; I names the exact tag (R16, §19.15 owner steps) |
| 43 | The spike's toolchain build replaces everyone's toolchain image | rules | medium | fixed | Holds (`toolchain.sh:126`, `dev.sh:56`). R21: toolchain changes land on main first and the image is built only from main (before S0 and at F0's start) |
| 44 | The build identifiers' rename is unspecified and could split the lock | rules | medium | fixed | Holds (`lock.sh:21`, `install-build-slice.sh`). The lock, build variables, images, builder and slices keep their names in M2 (R19, D142); the rename (D100) renamed them once, before M2, with the slice re-install through the owner's askpass |
| 45 | D93-D99 already exist on main | rules | medium | fixed | Holds (`docs/ARCHITECTURE.md:4371-4377` at `9c6e60a`). §19's decisions renumbered D101-D138 and every reference updated in all three files; new decisions D139-D143 |
| 46 | Only one port gets attribution; other borrowed code has no rule | rules | low | fixed | Holds (R9 covers GetPaper only). R20 requires a header, a `THIRD_PARTY.md` row and a handoff line for each, names the sources, and I checks it (§19.5.7) |
| 47 | Some gate lines measure nothing; R14's grep misses web clocks | rules | low | fixed | Holds. The bench runs in the gate at 2× G10's budgets beside deterministic proxies; start-up runs at 2× G11's; NEEDED is compared with the allowed set; R14's pattern covers `Date::now`, `performance().now`, `Local::now`, `Utc::now` |
| 48 | Rule texts contradict each other and CLAUDE.md | rules | low | fixed | Holds. One build at a time on the lock; every buildx script listed; host and container scripts told apart (`wasm-app.sh` is a container script); R17 adds F0 and X |
| 49 | The notice-line rule cannot cover PNGs and `Cargo.lock` | rules | low | fixed | Holds. The README's patch table is their notice; 0007 adds that exception (§19.6.5) |
| 50 | Lucide also carries Feather's MIT licence; poll-promise comes in anyway | rules | low | fixed | Holds (lucide's LICENSE; `egui_system_font_provider` 0.37.0 depends on poll-promise ^0.3.0). D133 and §19.12 corrected; lucide's LICENSE ships |
| 51 | check-pins reads a generated, possibly stale copy of Inkboard's lock | rules | low | fixed | Holds (`check-pins.sh:2-3`). check-pins fails when the copy is missing or its stamp does not match the patches, and the gate runs `inkboard.sh src` first (R12) |
