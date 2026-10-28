# Quire milestone 1: implementation plan

Read `docs/ARCHITECTURE.md` first; this plan follows it and cites it as "§n". The work is split into workstreams for
parallel agents:

- **W0a** lands first, alone, as its own commit: the build and runtime caps the owner asked for "asap", the build slice
  (installed by the coordinator), and the compose file with its Caddy HTTPS front.
- **W0b** lands second, alone: scaffold, contracts (including the public API's DTO crate, OpenAPI plumbing, token
  auth and the few minimal routes the CLI tests need), Docker image, test harnesses.
- **W1–W6 and W8** then run in parallel: seven workstreams. W8 (the `quire` CLI and the agent harness) is numbered after
  W7 only because it was added later. Their dependency edges are listed under "Overview"; every edge is satisfied by a
  minimal working version from W0b, so nobody waits on a `todo!()`.
- **W7** integrates, owns the cross-workstream tests, and runs the end-to-end gate, including the full CLI suite and the
  agent harness.

---

## Rules for every workstream

- **R1. Builds and caps.**
  - Run every cargo, rustc, rustfmt, wasm-bindgen, wasm-opt, trunk, node, pnpm and npx command as
    `scripts/dev.sh <cmd>`. Format only with `scripts/fmt.sh` (never `cargo fmt --all`: it reaches into
    `vendor/inkboard`). Build images only with `scripts/image.sh` or `scripts/toolchain.sh`, which use the
    `quire-limited` builder. Run Playwright only through `scripts/e2e.sh`, the agent harness only through
    `scripts/agent-test.sh`, and build the standalone CLI only through `scripts/cli-dist.sh`.
  - These scripts take one per-user lock (§13.1), so builds queue instead of overlapping. Nested calls from a script
    that holds the lock skip it automatically (`QUIRE_LOCK_HELD`); never set that variable by hand.
  - Never run `cargo`, `pnpm` or `node` on the host. Never raise the caps (`QUIRE_BUILD_CPUS`, `QUIRE_BUILD_MEM`,
    `QUIRE_BUILD_CPUSET` may only be lowered; the runtime caps `QUIRE_CPUS`, `QUIRE_MEM`, `QUIRE_CADDY_CPUS` and
    `QUIRE_CADDY_MEM` change only when the owner asks). Git runs on the host.
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
  - If you change a DTO you own (in `crates/quire-api`), run `scripts/dev.sh cargo test -p quire-api --locked
    export_bindings`, then commit the regenerated files for your own types.
  - If you add or change a route, a DTO or an annotation, run `scripts/dev.sh env UPDATE_OPENAPI=1 cargo test -p
    quire-server --locked --test api -- openapi::` and commit `docs/api/openapi.json` with the change. The file is one
    shared JSON document but deterministic (ARCHITECTURE §7.4), so on a merge conflict the orchestrator takes either
    side and re-runs that command, as for lockfiles (R6). Every route is registered through `routes!` on a
    `#[utoipa::path]` handler whose `security` matches its `Auth<S>` extractor (D44); never `Router::route` under
    `crates/quire-server/src/routes/`.
- **R5. Schema.** Only W1 edits `crates/quire-store/migrations/0001-init/up.sql`. If you need a schema change, put the
  exact DDL in your handoff note.
- **R6. Dependencies and lockfiles.**
  - W0b declares every known dependency (§4). A new crate goes into a crate's `Cargo.toml` through that file's owner
    (W1: `quire-api`, `quire-core`, `quire-store`; W2: `quire-server`, `quire-assets`; W8: `quire-cli`). A new npm
    package goes into `web/package.json` through W3.
  - Whoever changes a `Cargo.toml` commits the regenerated `Cargo.lock` in the same commit
    (`scripts/dev.sh cargo update --workspace`, which adds the new entries and keeps other pins), then runs
    `scripts/check-pins.sh`. The same holds for `package.json` and `pnpm-lock.yaml`.
  - The orchestrator merges branches that touch a lockfile one at a time. On a lockfile conflict it takes the target
    branch's lockfile and re-runs the update command.
  - Licences: MIT, Apache-2.0, BSD, ISC, Zlib, Unicode or CC0 only. Nothing from AGPL or BSL projects; nothing from
    TipTap Pro or Cloud. webpki-roots (CDLA-Permissive-2.0) is not allowed, so never enable ureq's `rustls` feature
    (D54).
- **R7. Tests.** Rust integration tests share one binary per crate: `crates/quire-store/tests/store/main.rs` and
  `crates/quire-server/tests/api/main.rs` (both W0b, frozen, owned by W7 afterwards), and
  `crates/quire-cli/tests/cli/main.rs` (W0b, then W8 with the whole crate). Each workstream owns its own module file
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
    `git worktree add ../Quire-Wn -b ws/Wn-<name>`, then
    `git -C ../Quire-Wn submodule update --init --reference "$PWD/vendor/inkboard" vendor/inkboard`.
  - Nothing inside a build container runs git against the main repository (a worktree's `.git` points outside the
    mount); `scripts/inkboard.sh` uses its own self-contained repo in `.cache/inkboard-src`.
  - Set `QUIRE_SHARED_CARGO_HOME=$HOME/.cache/quire-cargo` so the worktrees share the cargo registry.
  - Commit only your owned files. The orchestrator merges the branches in the order they finish (lockfile branches
    serialized, R6); W7 merges last.

---

## Overview

| Id | Workstream | Depends on | Runs |
|---|---|---|---|
| W0a | Build and runtime caps, the build slice, compose with the Caddy front | none (the coordinator runs the slice install) | first, alone, own commit |
| W0b | Scaffold, contracts, Docker image, harnesses, minimal `me`/token/collection-create routes | W0a | second, alone |
| W1 | Tree, collections, items, lists, search (backend), cursor pagination, and schema owner | W0b | parallel |
| W2 | Platform: auth, setup, security (including the trusted-proxy rule, cookies and HSTS behind Caddy), API tokens, OpenAPI assembly, CORS and rate limits, static serving, files, jobs, backup, `quire-server` commands (backend) | W0b | parallel |
| W3 | Web shell: routing, sidebar tree and drag and drop, top bar, list pages, settings (including API tokens) | W0b | parallel |
| W4 | Documents, full stack: Yjs storage, the TipTap editor, and Markdown / ProseMirror JSON conversion | W0b | parallel |
| W5 | Inkboard engine: patches, wasm build, frame page | W0b | parallel |
| W6 | Boards, full stack minus the engine: storage, previews, board host and page, board API rules | W0b (W5 for the real engine; fake frame until then) | parallel |
| W8 | The `quire` CLI, its tests, the agent harness, `docs/AGENTS.md` | W0b (its minimal routes for the parallel-phase tests; W1, W2, W4, W6 routes for the `flows` tests and scenarios 02–11, gated by W7) | parallel |
| W7 | Integration, cross-workstream tests, production image, end-to-end smoke, full CLI suite and agent harness, docs | W1–W6, W8 | last |

```
W0a ── W0b ──┬─ W1 ─┐
             ├─ W2 ─┤
             ├─ W3 ─┤
             ├─ W4 ─┤
             ├─ W5 ─┼─ W7
             ├─ W6 ─┤   W6 ⇠ W5 (bridge.ts contract from W0b; real dist when W5 lands)
             └─ W8 ─┘   W8 ⇠ W1, W2, W4, W6 (quire-api DTOs and TestServer from W0b; full routes on merge)
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
| Every route owner | `OpenApiRouter` registration, `Auth<S>`, `ErrorBody`, `Page<T>`, `quire_core::cursor` | W0b, then W1/W2 | working plumbing; the spec assembled from every module's router |
| W8 CLI tests (`config`, `auth`, `tokens`, `admin`, `coverage`) | Bearer auth with scopes; `GET /auth/me`; `GET`/`POST /tokens`, `DELETE /tokens/{id}`, `DELETE /tokens/current`; `quire_server::testing::TestServer` | W2 | working minimal routes and store functions (`is_quire_bearer`, lookup with classification, scope check, create with the caps, list, revoke with children; one token per scope in `TestServer`) |
| W8 agent harness, scenario 01 | `quire-server admin create-owner` and `admin create-token`; `POST /collections`; both binaries in the image | W2, W1, W0b | working minimal commands with their final flags; a minimal create route; the Dockerfile `cli` stages |
| W4 `?title=from_h1`; W4, W6 recovery points | `items::rename`, `revisions::pre_api`, `WriteBy` | W1 | working minimal versions |
| W8 `flows` tests and scenarios 02–11 | Collections, items, tree, search and lists; documents in Markdown/JSON; boards; attachments and export | W1, W4, W6, W2 | DTOs frozen in `quire-api`; the rest runs at W7 |
| W3 Settings → API tokens | `/api/v1/tokens` | W2 | DTOs; mocked in tests |
| W4 web "Export Markdown" | `GET …/doc/markdown?download=true` | W4 (server half) | nothing: the same workstream; mocked in web tests until the route exists |

Lock contention: W5's release wasm build (10–20 min) and the image builds are the long jobs; a full agent-harness run
holds the lock for a few minutes. The lock queues them. Keep test runs filtered to your own modules.

---

## W0a: build and runtime caps (lands first)

**Goal.** Kernel-enforced limits on every build and on the hosted app, committed on their own before any scaffold work,
because the owner asked for them as soon as possible (§13.1, D38, §13.3). That includes the total cap
(`quire-build.slice`, which the owner approved on 2026-10-28) and the compose project with its Caddy HTTPS front and
caps (D57, §13.4).

### Tasks, in order

1. **Wait for running builds.** Check `docker ps` and `docker buildx ls`; the toolchain image may still be building.
   Do not start anything until it is done.
2. **Lock.** Add `scripts/lib/lock.sh` (§13.1): per-user lock file from `getent passwd`, "waiting for another Quire
   build…", exports `QUIRE_LOCK_HELD=1`, skips when it is already set. Source it from `scripts/dev.sh` and
   `scripts/toolchain.sh`.
3. **Slice files.** Add `docker/quire-build.slice`, `docker/quire.slice`, `scripts/install-build-slice.sh` (install as
   root, never calling `sudo` itself; `--check` without root; exit codes of §13.1) and `scripts/lib/slice.sh`
   (`quire_slice`: `QUIRE_CGROUP_PARENT` only under Docker's systemd cgroup driver with the unit file installed, tested
   through `FragmentPath`, never `LoadState`, which reads `loaded` for any slice name; otherwise the warning;
   `QUIRE_BUILD_SLICE` for the warning test). Do not run the installer and never run `sudo`: write in your
   handoff note that the slice is ready, and the coordinator runs, once, through the owner's askpass dialog:
   `SUDO_ASKPASS=/home/staffan/.local/bin/claude-askpass sudo -A -p "install the Quire build cap (systemd slice: 6 CPUs,
   12 GB RAM, no swap, low priority) :: $PWD/scripts/install-build-slice.sh" "$PWD/scripts/install-build-slice.sh"`.
   Continue with tasks 4–7 meanwhile: everything must work without the slice.
4. **`scripts/dev.sh`** (§13.1): keep `--cpus`, `--memory`, `--memory-swap`, `--pids-limit 4096` and `nice -n 10`;
   add `--cpuset-cpus` (last N cores), `--shm-size 1g`, `quire_slice` and `--cgroup-parent "$QUIRE_CGROUP_PARENT"` when
   set, the thread-count environment variables, `QUIRE_DEV_IMAGE`, `QUIRE_DEV_NETWORK` and `QUIRE_SHARED_CARGO_HOME`.
5. **Builder.** Add `docker/buildkitd.toml` (`max-parallelism = 2`, GC `maxUsedSpace = "30GB"`). In
   `scripts/toolchain.sh`: create the BuildKit container `quire-buildkit` from `moby/buildkit:v0.34.1` pinned by digest
   (`docker buildx imagetools inspect`; v0.34.1 was days old on 2026-10-28, so as for Caddy below, if it is still
   under two weeks old take the newest older patch and record it in ARCHITECTURE §16), with the caps, cpuset, cgroup parent (from `quire_slice`), state volume and
   `docker cp` of the config of §13.1, and register it as `quire-limited` with the `remote` driver
   (`docker-container://quire-buildkit`): buildx's docker-container driver ignores `cgroup-parent` under the systemd
   cgroup driver. If the existing `quire-limited` uses another driver (it does today), or `quire-buildkit` has other
   caps or another cgroup parent than `quire_slice` gives, print the `docker buildx rm quire-limited; docker rm -f
   quire-buildkit; scripts/toolchain.sh` instruction and exit 1. It keeps building `docker/toolchain.Dockerfile` until
   W0b replaces it.
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
9. **CLAUDE.md.** Add: the lock and `QUIRE_LOCK_HELD`; the cpuset and the slice (`scripts/install-build-slice.sh
   --check`; builds still run, with a warning, without it); "never run `sudo`: the coordinator runs the one privileged
   step through the askpass dialog"; "never build inside `vendor/inkboard`"; "format only with `scripts/fmt.sh`, never
   `cargo fmt --all`"; and that the hosted app runs behind Caddy, with both containers' limits in `compose.yaml`
   (app 2 CPUs / 1 GiB, Caddy 0.5 CPU / 128 MiB).
10. Commit W0a on its own (this is the repository's first commit, together with the already staged submodule and
    `docs/`).

### Owns

During W0a: `scripts/lib/lock.sh`, `scripts/lib/slice.sh`, `scripts/dev.sh`, `scripts/toolchain.sh`,
`scripts/install-build-slice.sh`, `docker/buildkitd.toml`, `docker/quire-build.slice`, `docker/quire.slice`,
`docker/Caddyfile`, `compose.yaml`, `CLAUDE.md`. Ownership then passes to W0b, and to W7 after W0b.

### Acceptance

```sh
bash -n scripts/*.sh scripts/lib/*.sh
( scripts/dev.sh sleep 20 & sleep 2; scripts/dev.sh true 2>&1 | grep -q 'waiting for another Quire build' ); wait
bash -c '. scripts/lib/lock.sh && quire_lock && timeout 120 scripts/dev.sh true'    # nested call does not deadlock
QUIRE_BUILD_SLICE=quire-missing-test.slice scripts/dev.sh true 2>&1 | grep -q 'not installed: per-container caps only'
scripts/dev.sh nproc | grep -qx 6
scripts/dev.sh cat /sys/fs/cgroup/cpu.max | grep -qx '600000 100000'
scripts/dev.sh cat /sys/fs/cgroup/memory.max | grep -qx 12884901888
scripts/dev.sh cat /sys/fs/cgroup/memory.swap.max | grep -qx 0
docker buildx inspect quire-limited | grep -qE 'Driver: +remote'
docker inspect -f '{{.HostConfig.NanoCpus}} {{.HostConfig.Memory}} {{.HostConfig.MemorySwap}}' quire-buildkit \
  | grep -qx '6000000000 12884901888 12884901888'
docker inspect -f '{{.HostConfig.CpusetCpus}}' quire-buildkit | grep -q .
docker exec quire-buildkit cat /etc/buildkit/buildkitd.toml | grep -q 'max-parallelism = 2'
docker compose -f compose.yaml config -q && ! grep -q '^\s*build:' compose.yaml
docker compose --env-file /dev/null -f compose.yaml config --format json | jq -e '(.services.quire.ports // []) == []
  and .services.quire.cpus == 2 and .services.caddy.cpus == 0.5 and (.services.caddy.ports | length) == 2
  and .networks.default.ipam.config[0].ip_range == "172.31.240.8/29"
  and .services.caddy.networks.default.ipv4_address == "172.31.240.2"'
! grep -qE '^\s*(encode|header)\b' docker/Caddyfile
img=$(docker compose --env-file /dev/null -f compose.yaml config --format json | jq -r .services.caddy.image) && for m in 'localhost internal' \
  'wiki.example.com acme'; do set -- $m; docker run --rm -e QUIRE_DOMAIN=$1 -e QUIRE_TLS=$2 \
  -v "$PWD/docker/Caddyfile:/etc/caddy/Caddyfile:ro" "$img" caddy validate --config /etc/caddy/Caddyfile \
  --adapter caddyfile || exit 1; done
# Caddy starts and binds 80/443 with the compose hardening (task 6, risk 22); nothing published, bounded wait
docker run -d --name quire-caddy-check --read-only --tmpfs /tmp --tmpfs /data --tmpfs /config --cap-drop ALL \
  --cap-add NET_BIND_SERVICE --security-opt no-new-privileges:true -e QUIRE_DOMAIN=localhost -e QUIRE_TLS=internal \
  -v "$PWD/docker/Caddyfile:/etc/caddy/Caddyfile:ro" "$img" >/dev/null \
  && for i in $(seq 20); do docker logs quire-caddy-check 2>&1 | grep -q 'serving initial configuration' && break; sleep 0.5; done; \
  docker logs quire-caddy-check 2>&1 | grep -q 'serving initial configuration'; rc=$?; docker rm -f quire-caddy-check >/dev/null; test $rc -eq 0
test ! -e vendor/inkboard/target
git -C vendor/inkboard status --porcelain | wc -l | grep -qx 0
# after the coordinator installed the slice (task 8):
scripts/install-build-slice.sh --check
( scripts/dev.sh sleep 15 & sleep 5; systemctl status quire-build.slice --no-pager | grep -q 'docker-.*\.scope' ); wait
docker inspect -f '{{.HostConfig.CgroupParent}}' quire-buildkit | grep -qx quire-build.slice
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
     `scripts/image-smoke.sh` (under the lock, `quire_slice`, project `quire-smoke` on `172.31.240.48/28` with range
     `172.31.240.56/29` and Caddy at `.50`, `--env-file /dev/null` and the owner-facing variables unset (§13.3),
     `compose.yaml` + `compose.smoke.yaml`, `QUIRE_DOMAIN=localhost`, `QUIRE_TLS=internal`, `QUIRE_HTTPS_PORT=18443`;
     `QUIRE_SMOKE_LEVEL=basic|full`; `basic` reads Caddy's root from its tmpfs with `docker compose exec -T caddy cat
     /data/caddy/pki/authorities/local/root.crt` (`docker cp` cannot read a tmpfs) and checks that
     `curl --cacert <root> --resolve localhost:18443:127.0.0.1 https://localhost:18443/healthz` answers 200 (and fails
     without `--cacert`), that the `quire` container has no published port, that `docker inspect` shows
     `NanoCpus=2000000000`, `Memory=1073741824` and `MemorySwap=1073741824` for `quire` and `NanoCpus=500000000`,
     `Memory=134217728` and `MemorySwap=134217728` for `caddy`, and that `quire-server --version` and `quire --version`
     run inside the image; `full` adds the §14 checks), a skeleton `scripts/e2e.sh frame|smoke` (§14; the smoke
     project starts only `quire`), `scripts/fmt.sh` (§4.1), and `scripts/check-pins.sh` (host only, text and git:
     submodule at `41e0295` and clean; serde, serde_json, base64 and thiserror at equal versions in `Cargo.lock` and
     `vendor/inkboard/Cargo.lock`; no `{` in `.dockerignore`; `rust-toolchain.toml` channel equal to the `RUST_IMAGE`
     tag).
2. **Rust workspace.**
   - Write `Cargo.toml` (§4.1) with every crate's dependencies declared, `Cargo.lock` (align the shared crates with
     Inkboard's lock using `cargo update -p <crate> --precise <version>`), `rust-toolchain.toml` (§3) and
     `.cargo/config.toml` (keep `jobs = 6`; add `[env] TS_RS_EXPORT_DIR = { value = "web/src/api/gen", relative = true }`).
   - Confirm a single `libsqlite3-sys`; if deadpool-sqlite or rusqlite_migration do not fit rusqlite 0.40, apply the D2
     fallback and record it in §16.
   - Create the six crates (`quire-api`, `quire-core`, `quire-store`, `quire-server`, `quire-assets`, `quire-cli`) with
     **every module file listed in the ownership tables**: empty, stubbed, or minimally implemented as stated below.
     `quire-server` has `src/lib.rs` (module list and `pub fn app(state) -> Router`) and a thin `src/main.rs`, and builds
     the binary `quire-server`; `quire-server` has a `testing` feature; `quire-cli` builds the binary `quire`.
     `quire-core` re-exports `pub use quire_api as api;` (D40).
   - Write `migrations/0001-init/up.sql`: the §5 DDL verbatim (including `token_scopes`, `api_tokens` with
     `parent_token_id`, `api_token_scopes`, `last_token_id` on `doc_state` and `board_state`, `revisions.source_seq`,
     and the `pre-api` revision reason).
3. **DTOs** (`crates/quire-api`).
   - Write all DTOs from §7.1–§7.3 and §7.5, with `#[derive(TS, ToSchema)] #[ts(export)]`.
   - Write `ErrorCode` (every §7.1 code with its HTTP status), `ErrorBody`, `Page<T>`, `Scope` (with the hierarchy),
     `API_VERSION = "1.0"` (frozen until the first release, §7.1), `cli::ExitCode` (the §7.7 table, with
     `from_status(status, code)`: unlisted 4xx → 10, unlisted 5xx and 408 → 12, 3xx → 3), and `token::{encode, check}`
     (D42).
   - Generate `web/src/api/gen/*.ts` and commit them.
4. **Server skeleton.**
   - `quire-server serve` (config from env, §13.3: `config.rs` parses every variable, including `QUIRE_DOMAIN`,
     `QUIRE_TLS`, `QUIRE_HTTPS_PORT` and `QUIRE_TRUSTED_PROXIES`, and derives the public URL of §8; W2 adds the
     behaviour behind them) and `quire-server healthcheck [--url URL]` (default URL from `QUIRE_LISTEN`'s port on
     127.0.0.1; exit 0 if healthy, 1 otherwise).
   - `quire-server admin create-owner` and `admin create-token` with their final §7.8 flags, `--json` and the §7.7 exit
     codes, minimally implemented (owner row + settings; token row + scopes). The other admin subcommands exist and
     exit 1 with "not implemented (W2)" until W2 replaces them.
   - `GET /healthz`. Everything else under `/api/` that is not a `/api/v1` route answers the JSON 404.
   - Every `routes::<x>::router()` returns an `OpenApiRouter<AppState>`, empty except for these minimal working routes
     (final paths, DTOs, `#[utoipa::path]` security and `Auth<S>`), which their owners complete: `GET /auth/me`,
     `GET`/`POST /tokens`, `DELETE /tokens/{id}` and `DELETE /tokens/current` (W2), and `POST /collections` (W1).
     `openapi.rs` nests them under `/api/v1`, calls `split_for_parts()`, adds the security schemes and `info` (§7.4),
     and serves `GET /api/v1/openapi.json`; `docs/api/openapi.json` is generated and committed, and
     `tests/api/openapi.rs` has `spec_matches_committed` and `spec_parses_as_3_1`.
   - `app.rs` merges them; `state.rs`, `error.rs` (axum rejections mapped to `ErrorBody`, `X-Request-Id`,
     `Quire-Api-Version`) and `extract.rs` are in place. `extract.rs` has `Auth<S>` with both branches of §7.3 working
     (`is_quire_bearer`, the cookie, and bearer lookup through `tokens::lookup` with classification, the scope check
     and `session_required`), plus `Viewer` and `ItemRef`. The owner-exists cache re-checks while false (§8).
   - `crates/quire-cli/src/main.rs`: clap skeleton with the global flags of §7.6, `quire --version`, and
     `quire exit-codes [--json]` printing `quire_api::cli::ExitCode`.
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
   - server: `board_previews::{PreviewQueue, spawn}` with a no-op `enqueue` (file `quire-server/src/board_previews.rs`);
   - store: `tokens::{create, lookup, list, revoke}` (generate with getrandom, `quire_api::token::encode`, store sha256,
     prefix, expanded scopes and `parent_token_id`; look up by hash and classify; revoke with children);
   - core: `cursor::{encode, decode}` complete (D46): base64url, keyset or offset, 8-byte hash of the normalized
     query.
6. **Test harnesses.**
   - `crates/quire-store/tests/store/{main,common}.rs`: a temp-file DB with migrations applied, opened through `Db`.
   - `crates/quire-server/tests/api/{main,common,integration}.rs`: `TestApp::new()` (`quire_server::app()`, temp data
     dir, cheap argon2), `TestApp::with_owner()` (inserts the owner and a session with raw SQL; returns a client with
     the cookie, an allowed `Host` and `Sec-Fetch-Site: same-origin`), and `TestApp::with_token(scope)` (a client with
     `Authorization: Bearer` and no cookie).
   - `crates/quire-server/src/testing.rs` (feature `testing`): `TestServer::start() -> TestServer{url, read_token,
     write_token, admin_token, data_dir}`, which binds `127.0.0.1:0`, runs `app()` on its own tokio runtime thread with
     a temp data dir and cheap argon2, creates the owner, and stops on drop.
   - `crates/quire-cli/tests/cli/{main,common}.rs`: `common` starts a `TestServer` and builds an assert_cmd `Command`
     for the `quire` binary with `QUIRE_URL`, `QUIRE_TOKEN`, `QUIRE_CONFIG` and `HOME` pointing into a temp dir.
   - One test per crate that passes (for the CLI: `quire --version`).
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
   - `web/inkboard-frame/{frame.html,frame.js,frame.css}`: a placeholder whose `quireBoardFrame.connect` rejects
     `{kind:'unsupported'}`, and which passes typecheck.
8. **Inkboard and e2e scaffolding.**
   - `scripts/inkboard.sh` with a working `stub` (manifest per §10.5, `format_version` parsed from `serial.rs`); the
     other subcommands print "not implemented (W5)".
   - `patches/inkboard/README.md`.
   - `e2e/package.json` (@playwright/test 1.64.1) and its lockfile.
   - `e2e/playwright.config.ts` with `workers: 2`, projects `frame` (`baseURL` `http://127.0.0.1:4173`) and `smoke`
     (`baseURL` `http://quire:8080`: Quire directly on the project network, plain HTTP; Caddy is not part of e2e runs), and Chromium launch args `--use-angle=swiftshader --enable-unsafe-swiftshader`.
     It also declares `webServer: node e2e/serve-static.mjs web/dist-inkboard 4173`.
   - A minimal `e2e/serve-static.mjs`, owned by W5 afterwards. It must serve `.wasm` as `application/wasm`.
   - Placeholder specs `e2e/tests/frame.spec.ts` and `e2e/tests/smoke.spec.ts`.
9. **Placeholders, licences and docs.**
   - Fixture placeholders: `fixtures/ydoc/getting-started.bin` (`[0,0]`) and
     `fixtures/boards/getting-started.inkboard` (an empty `Document`).
   - `LICENSE`, `THIRD_PARTY.md`, a stub `README.md`.
   - `.gitignore` additions (§3).
   - In `CLAUDE.md`: the `docker/Dockerfile` toolchain target in place of `docker/toolchain.Dockerfile`, and one line:
     the agent harness runs only through `scripts/agent-test.sh`, and agents use the `quire` CLI as described in
     `docs/AGENTS.md`.
   - In `docs/ARCHITECTURE.md` §16: any pin W0b could not confirm and the fallback chosen.

### Owns

During W0b, W0b owns everything it creates plus the W0a files. When W0b ends, ownership passes as listed in the W1–W8
tables.

### Produces (frozen contracts)

- **Rust store functions** (`tx: &rusqlite::Transaction`, `now: i64` ms; errors are `quire_store::Error`, which wraps
  `ErrorCode`):

  ```rust
  // quire-store::db
  impl Db { pub fn write<T>(&self, f) -> Result<T>; pub fn read<T>(&self, f) -> Result<T>;
    pub fn maintenance(&self) -> Result<rusqlite::Connection>; }                  // no query_only; VACUUM INTO only
  // quire-store::shared
  pub fn touch_content(tx, item_id: &str, actor_id: &str, now) -> Result<()>;         // content_at, updated_at, updated_by
  pub fn set_body_text(tx, item_id: &str, body: &str) -> Result<()>;                   // item_text.body (no-op if equal)
  pub fn ensure_writable(tx, item_id: &str, kind: ItemKind) -> Result<ItemRow>;        // not_found / validation (wrong kind) / read_only
  pub fn item_row(conn: &rusqlite::Connection, item_id: &str) -> Result<ItemRow>;
  pub fn visible_items(viewer: &Viewer) -> &'static str;                               // SQL fragment, §5
  pub fn effectively_published(conn: &rusqlite::Connection, item_id: &str) -> Result<bool>;
  pub struct WriteBy<'a> { pub user_id: &'a str, pub token_id: Option<&'a str> }      // content writes (D53)
  pub fn check_free_space(data_dir: &Path, min_free_mb: u64) -> Result<()>;            // D33: 507 insufficient_storage
  // quire-store::revisions
  pub fn maybe_auto(tx, item_id: &str, draft: RevisionDraft, now) -> Result<bool>;    // §5 policy (10 min)
  pub fn pre_api(tx, item_id: &str, kind: ItemKind, by: &WriteBy, now) -> Result<bool>; // D53: last_token_id, source_seq
  pub fn prune(tx, now) -> Result<u64>;
  // quire-store::blobs (D26)
  impl BlobStore { pub fn new(root: PathBuf) -> Self;
    pub fn stage(&self, bytes: &[u8]) -> std::io::Result<Staged>;                    // outside the writer
    pub async fn stage_stream(&self, body: impl Stream<Item = std::io::Result<Bytes>> + Unpin, limit: u64)
      -> Result<Staged>;                                                               // uploads: hashed while streamed
    pub fn commit(&self, tx, staged: Staged, now) -> Result<BlobRef /* sha256, size, mime (sniffed), w, h */>;
    pub fn put(&self, tx, bytes: &[u8], now) -> Result<BlobRef>;                       // stage + commit
    pub fn read(&self, sha256: &Sha256) -> std::io::Result<Vec<u8>>;
    pub fn path(&self, sha256: &Sha256) -> PathBuf; }                                 // Sha256: validated 64 lowercase hex
  // quire-store::backup
  pub fn pre_migration(db: &Db, data_dir: &Path, version: u32, now) -> Result<PathBuf>;
  // quire-store::docs / boards  (called by W1 item create/duplicate/discard, and by W2 seed)
  pub fn docs::init_empty(tx, item_id, now) -> Result<()>;
  pub fn docs::init_with(tx, item_id, ydoc_v1: &[u8], now) -> Result<()>;
  pub fn docs::copy(tx, from_id, to_id, now) -> Result<()>;                            // also copies item_attachments
  pub fn docs::is_empty(tx, item_id) -> Result<bool>;                                  // §6.1: only empty paragraphs
  pub fn boards::init_empty(tx, item_id, dark: bool, now) -> Result<()>;
  pub fn boards::init_with(tx, blobs: &BlobStore, item_id, envelope: &str, now) -> Result<()>;
  pub fn boards::copy(tx, from_id, to_id, now) -> Result<()>;
  pub fn boards::is_empty(tx, item_id) -> Result<bool>;
  // quire-store::collections / items (W1 completes; W2's seed and purge job call them)
  pub fn collections::create(tx, actor_id: &str, new: &NewCollection /* api DTO */, now) -> Result<Collection>;
  pub fn items::create(tx, actor_id: &str, new: &NewItem /* api DTO */, dark: bool, now) -> Result<ItemSummary>;
  pub fn items::rename(tx, item_id: &str, title: &str, actor_id: &str, now) -> Result<ItemSummary>; // meta + tree version
  pub fn items::purge_expired(tx, older_than: i64) -> Result<u64>;
  // quire-store::tokens (W2 completes: caps, touch, expiry purge)
  pub fn tokens::create(tx, user_id: &str, new: &NewToken /* api DTO */, via: CreatedVia,
    parent: Option<&TokenRow>, now) -> Result<CreatedToken>;                          // parent caps expiry, forbids admin
  pub fn tokens::lookup(conn: &rusqlite::Connection, token: &str, now)
    -> Result<TokenLookup /* Live(TokenRow{id, user_id, scopes, expires_at}) | Expired | Rejected */>;
  pub fn tokens::list(conn: &rusqlite::Connection, user_id: &str, now) -> Result<Vec<ApiToken>>;
  pub fn tokens::revoke(tx, user_id: &str, token_id: &str, now) -> Result<u64>;      // also its children; 0 = not found
  // quire-core::cursor (W1 owns afterwards)
  pub fn encode(c: &Cursor, query_hash: [u8; 8]) -> String;
  pub fn decode(s: &str, query_hash: [u8; 8]) -> Result<Cursor, CursorError>;          // CursorError → 422 validation
  ```

- **Rust server types:** `AppState{db: Db, blobs: Arc<BlobStore>, config: Arc<Config>, previews: PreviewQueue,
  board_work: Arc<Semaphore> /* 1 */, doc_work: Arc<Semaphore> /* 3; a PUT takes 2 at once */, upload_work: Arc<Semaphore>
  /* 2 */, token_limiter, token_bytes_limiter, auth_fail_limiter, owner_exists: Arc<AtomicBool>, openapi: Arc<str>}`,
  `Actor{user_id, role, method: AuthMethod::Session{session_id} | AuthMethod::Token{token_id, scopes}}`,
  `Auth<S>` with `S ∈ {Read, Write, Admin, Session, AnyToken}` (first handler argument; derefs to `Actor`),
  `Viewer{user_id, role}`, `ApiError` (implements `From<quire_store::Error>` and `IntoResponse` with the §7.1 shape),
  `quire_server::app(state) -> Router`, and `routes::<x>::router() -> OpenApiRouter<AppState>` for `auth`, `setup`,
  `workspace`, `tokens`, `files`, `export`, `config`, `admin`, `collections`, `items`, `tree`, `lists`, `search`, `docs`
  and `boards`. Also `board_previews::{PreviewQueue::enqueue(&self, item_id), spawn(state) -> PreviewQueue}`,
  `openapi::spec() -> utoipa::openapi::OpenApi`, and `testing::TestServer` (feature `testing`).
- **DTOs:** all of §7 in `quire-api`, plus `ErrorCode`, `ErrorBody`, `Page<T>`, `Scope`, `API_VERSION`,
  `cli::ExitCode` and `token::{encode, check}`.
- **Command lines:** the `quire-server` subcommands and flags of §7.8, and the `quire` global flags and command table
  of §7.6 (W8 implements the table; a change to it goes through R3).
- **TypeScript:** the modules listed in task 7 (signatures frozen), `BoardFrameHost`/`useBoardFrame` (§9.3), and
  `bridge.ts` (owned by W5 after W0b).
- **Scripts:** `scripts/inkboard.sh` subcommand names and the dist layout (`web/dist-inkboard/` plus `manifest.json`,
  §10.5); `scripts/fmt.sh`, `scripts/check-pins.sh`.

### Acceptance

```sh
bash -n scripts/*.sh scripts/lib/*.sh
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
scripts/dev.sh cargo test -p quire-api --locked export_bindings && git diff --exit-code web/src/api/gen
scripts/dev.sh cargo test -p quire-server --locked --test api -- openapi:: && git diff --exit-code docs/api/openapi.json
scripts/dev.sh cargo test -p quire-cli --locked --test cli
! scripts/dev.sh cargo tree --locked -p quire-server -e normal --prefix none | grep -qE '^(rustls|ureq|reqwest|native-tls|openssl) '
! scripts/dev.sh cargo tree --locked -p quire-cli -e normal --prefix none | grep -qE '^(openssl|openssl-sys|native-tls|rusqlite|yrs|inkboard-core|comrak|webpki-roots) '
scripts/dev.sh cargo run -p quire-cli --locked -- exit-codes --json | grep -q '"code": *9'
scripts/dev.sh pnpm --dir web install --frozen-lockfile
scripts/dev.sh pnpm --dir web typecheck && scripts/dev.sh pnpm --dir web test && scripts/dev.sh pnpm --dir web build
scripts/dev.sh pnpm --dir e2e install --frozen-lockfile
scripts/dev.sh scripts/inkboard.sh stub && grep -q '"format_version": *2' web/dist-inkboard/manifest.json
docker compose -f compose.yaml -f compose.e2e.yaml config -q
docker compose -f compose.yaml -f compose.smoke.yaml config -q
scripts/image.sh --build-arg INKBOARD_BUILD=stub && QUIRE_SMOKE_LEVEL=basic scripts/image-smoke.sh   # incl. curl through Caddy over HTTPS with the local CA
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
- ownership of the schema and of the shared DTOs (`quire-api` `common`, `tree`, `page`).

### Owns

| Area | Files |
|---|---|
| `crates/quire-api` | `Cargo.toml`, `src/{common,tree,page}.rs` |
| `crates/quire-core` | `Cargo.toml`, `src/{ids,rank,tree,time,cursor}.rs` |
| `crates/quire-store` | `Cargo.toml`, `src/{error,db,migrate,shared,collections,items,tree,search,stars,views,lists,revisions}.rs`, `migrations/**` |
| `crates/quire-store/tests/store` | `{common,tree,items,search,lists}.rs` |
| `crates/quire-server` | `src/routes/{collections,items,tree,lists,search}.rs` |
| `crates/quire-server/tests/api` | `{tree,items,search,lists}.rs` |
| `web/src/api/gen` | the generated files for the types in `quire-api` `common.rs`, `tree.rs` and `page.rs` |

### Produces

- The W1 routes of §7.2, and the `ItemSummary`, `ItemDetail` (with `effectively_published`), `Collection`, `Crumb` and
  position semantics.
- `collections::create`, `items::create`, `items::rename` and `items::purge_expired` completed;
  `revisions::{prune, pre_api}`.
- `quire-core::rank::{key_between, rebalance}`, `ids::{new_uuid, new_url_id, slugify}`, `search::build_match`,
  `cursor::{encode, decode}`, and a store helper `lists::page(…)` that every paginated query uses.

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
scripts/dev.sh cargo test -p quire-core   --locked -- ids:: rank:: tree:: cursor::
scripts/dev.sh cargo test -p quire-store  --locked --test store -- tree:: items:: search:: lists::
scripts/dev.sh cargo test -p quire-server --locked --test api   -- tree:: items:: search:: lists::
scripts/dev.sh cargo test -p quire-server --locked --test api   -- openapi:: && git diff --exit-code docs/api/openapi.json
```

These tests must exist and pass:

| Crate | Test |
|---|---|
| quire-core | `rank::tests::between_is_strictly_between` (proptest) |
| quire-core | `rank::tests::matches_fractional_indexing_vectors` |
| quire-core | `tree::tests::random_ops_never_cycle_and_match_model` (proptest, 500 cases: create, move, trash, restore, archive) |
| quire-store | `tree::purge_depth_2000_leaves_first` |
| quire-store | `tree::move_into_descendant_is_cycle` |
| quire-store | `tree::cross_collection_move_updates_descendants` (including trashed and archived descendants) |
| quire-store | `tree::parent_trigger_rejects_trashed_or_archived_parent` |
| quire-store | `tree::collection_trigger_rejects_foreign_parent` (`UPDATE items SET collection_id` alone) |
| quire-store | `items::trash_restore_subtree_roundtrip` |
| quire-store | `items::restore_with_trashed_parent_goes_top` |
| quire-store | `items::collection_delete_restore_roundtrip` (items trashed earlier stay in Trash) |
| quire-store | `items::create_restore_or_move_into_deleted_collection_409` |
| quire-store | `items::archive_hides_subtree_from_tree_and_default_search` |
| quire-store | `items::effectively_published_needs_published_ancestors` |
| quire-store | `items::rebalance_when_key_over_64` |
| quire-store | `search::fts_integrity_after_rename_save_purge` |
| quire-store | `search::query_builder_treats_fts_syntax_as_literal` (input `a OR b* NEAR( "x`) |
| quire-store | `search::negative_terms` (`plan -x` finds "plan quokka", not "plan x") |
| quire-store | `search::only_negative_terms_return_nothing` (`-x` alone: no FTS error, no results) |
| quire-store | `search::remove_diacritics` (`café` finds `cafe`) |
| quire-server | `tree::etag_304_until_structure_changes` (ETag `"t{n}.0"`) |
| quire-server | `items::create_is_published_and_initializes_content` (`published: true`, `effectively_published: true`, `drafts_count` unchanged) |
| quire-server | `items::discard_empty_refuses_nonempty_409` (also an empty item older than 24 h; a published new empty item is discarded) |
| quire-server | `items::unpublish_makes_draft_listed_in_drafts` (`/drafts` and `drafts_count` see it; tree and default search still show it) |
| quire-server | `items::publish_with_ancestors` (after unpublishing the item and its parent) |
| quire-server | `items::unpublish_always_allowed` |
| quire-server | `items::get_by_url_id` |
| quire-server | `lists::trash_lists_roots_with_purge_at` (item roots and collections) |
| quire-server | `search::snippet_markers_and_draft_filter` |
| quire-core | `cursor::tests::roundtrip_and_query_binding` (a cursor decoded with another query hash fails; a tampered cursor fails) |
| quire-store | `lists::cursor_stable_under_concurrent_insert` (insert between two pages: no item repeated or skipped) |
| quire-server | `lists::cursor_from_other_query_422` (`details.field == "cursor"`) |
| quire-server | `lists::every_paginated_list_has_page_shape` (`items` and `next_cursor`; `null` on the last page) |
| quire-server | `search::relevance_offset_capped_at_1000` |
| quire-server | `items::get_sets_meta_etag_and_patch_stale_if_match_412` (and `GET /items/{id}` never answers 304, also after a star) |
| quire-server | `items::rename_bumps_tree_version` (the tree ETag changes after a PATCH of the title) |
| quire-store | `items::pre_api_rule` (an owner edit then a token write: one `pre-api`; the same token's next write: none; newest revision equal to the current state: none) |
| quire-server | `items::text_returns_derived_body` (doc and board) |
| quire-server | `items::purge_needs_admin_scope` (write token → 403 `insufficient_scope`) |

---

## W2: platform (backend)

**Goal.**
- Setup with a code (rate-limited) and the seeded "Getting started" collection.
- Login, logout, sessions with half-window renewal and cookie refresh, password change, the `reset-password` CLI,
  `must_change_password`.
- The Host allowlist and CSRF middleware, rate limits, security headers, static serving with precompressed variants,
  `/api/v1/config` (with `inkboard: null` when no manifest).
- **Behind Caddy** (D57, §8, §11): `QUIRE_TRUSTED_PROXIES` parsed as IPs and CIDRs; `X-Forwarded-For` (rightmost
  address) and `X-Forwarded-Proto` honoured only from a trusted peer, `Forwarded` ignored; the client IP feeding every
  per-IP limiter, the access log, `sessions.ip` and `last_used_ip`; the request scheme choosing the cookie
  (`__Host-quire_session; Secure` on HTTPS, `quire_session` on plain HTTP, read by that name only); `QUIRE_DOMAIN`
  with any port in the Host allowlist; the public URL derived from `QUIRE_DOMAIN` and `QUIRE_HTTPS_PORT`; HSTS
  (`max-age=31536000`) on HTTPS requests when `QUIRE_TLS=acme` only; the CSP unchanged.
- Attachments (filename sanitizing, RFC 6266 headers, header-only image size and the 100 MP limit) and blobs;
  attachment GC, blob GC and the orphan sweep (D26, D36).
- Jobs: purge, GC, backup, optimize, session expiry, and the scheduling of the preview worker.
- Backup on the maintenance connection; export without sessions, API tokens or password hashes;
  `backup::pre_migration`.
- The `quire-server` commands (§7.8): `serve`, `healthcheck` (URL from `QUIRE_LISTEN`), `admin create-owner`,
  `create-token`, `list-tokens`, `revoke-token`, `reset-password`, `backup`, `check`, all with `--json` and the §7.7
  exit codes.
- Graceful shutdown with a WAL checkpoint.
- **Public API platform** (§7.1, §7.3, §7.4, D39–D48):
  - personal access tokens end to end: `tokens::*` (create with the 100 limit, the scope ceiling, the expiry cap for
    token callers, no `admin` from a token, `parent_token_id`, and the password for a session's `admin` or
    never-expiring token; lookup with classification; list; revoke with children; revoke-current; the once-a-minute
    `last_used` write; deletion 30 days after revocation or expiry), the `/api/v1/tokens` routes completed from W0b's,
    `Me.auth`, and `Auth<S>` completed (expiry reason, disabled users);
  - `is_quire_bearer` shared by `Auth<S>` and the CSRF layer; other `Authorization` values ignored; CSRF for every
    non-token mutation, including those with neither `Sec-Fetch-Site` nor `Origin`; the cookie rule for
    `GET /export`; `Authorization` in `SensitiveHeaders`, `token_id` in the access log;
  - CORS from `QUIRE_CORS_ORIGINS` (`cors.rs`); the per-token request and byte limiters and the per-IP failed-bearer
    limiter that still serves valid tokens, pruned hourly (`ratelimit.rs`);
  - the free-space floor (507), the `upload_work` permit with streamed uploads, single-flight export and backup
    (429 `busy`), the export copy on `journal_mode=DELETE`, on-demand backups under `QUIRE_BACKUP_KEEP`;
  - the owner-exists cache that re-checks while false (`admin create-owner` while serving), `reset-password
    --revoke-tokens`, the session-only email change, `others=true` for token callers, the full `limits` in
    `/api/v1/config`;
  - the error envelope for every rejection (404, 405, 408, malformed JSON, path and query), `X-Request-Id`,
    `Quire-Api-Version`, `WWW-Authenticate` on 401 `invalid_token` and 403 `insufficient_scope`;
  - the OpenAPI assembly and `GET /api/v1/openapi.json` (`openapi.rs`), and the generic scope test
    `security::every_operation_enforces_its_declared_scope`;
  - `quire_server::testing::TestServer` completed (W8's CLI tests run against it).

### Owns

| Area | Files |
|---|---|
| `crates/quire-api` | `src/{auth,tokens,files,error,token}.rs` |
| `crates/quire-store` | `src/{users,sessions,tokens,settings,blobs,attachments,gc,backup,export,seed}.rs` |
| `crates/quire-store/tests/store` | `{auth,tokens,blobs,backup}.rs` |
| `crates/quire-server` | `Cargo.toml`, `src/{main,cli,admin,config,app,state,error,extract,authz,csrf,cors,ratelimit,headers,static_files,jobs,openapi,testing}.rs`, `src/routes/{auth,setup,workspace,tokens,files,export,config,admin}.rs` |
| `crates/quire-server/tests/api` | `{common,auth,tokens,security,openapi,static_files,files,export,admin}.rs` |
| `crates/quire-assets` | everything |
| `web/src/api/gen` | the generated files for the types in `quire-api` `auth.rs`, `tokens.rs`, `files.rs` and `error.rs` |

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
scripts/dev.sh cargo test -p quire-store  --locked --test store -- auth:: tokens:: blobs:: backup::
scripts/dev.sh cargo test -p quire-server --locked --test api   -- auth:: tokens:: security:: openapi:: static_files:: files:: export:: admin::
scripts/dev.sh cargo test -p quire-server --locked --test api   -- openapi:: && git diff --exit-code docs/api/openapi.json
scripts/dev.sh env QUIRE_LISTEN=127.0.0.1:9 cargo run -p quire-server --locked -- healthcheck; test $? -eq 1
scripts/dev.sh bash -c 'd=$(mktemp -d) && export QUIRE_DATA_DIR=$d \
  && cargo run -q -p quire-server --locked -- admin create-owner --email a@quire.invalid --name A --workspace W --generate-password --json >/dev/null \
  && cargo run -q -p quire-server --locked -- admin create-token --name t --scope read --expires 1d --json | grep -q "\"token\": *\"qpat_" \
  && { cargo run -q -p quire-server --locked -- admin create-owner --email b@quire.invalid --name B --workspace W --generate-password; test $? -eq 8; }'
! grep -rn '\.route(' crates/quire-server/src/routes
! grep -rn '\.route("/api' crates/quire-server/src
```

These tests must exist and pass:

| Area | Test |
|---|---|
| Setup | `auth::setup_requires_code` |
| Setup | `auth::setup_only_once_under_concurrency` |
| Setup | `auth::setup_required_409_before_setup` |
| Setup | `auth::setup_rate_limited_per_ip` |
| Setup | `auth::setup_seeds_getting_started` |
| Login | `auth::login_cookie_attributes` (HttpOnly, SameSite=Lax, Path=/; `__Host-` and Secure only for `X-Forwarded-Proto: https` from a trusted peer, plain `quire_session` otherwise; Max-Age only with `remember`) |
| Login | `auth::cookie_read_by_request_scheme` (a `__Host-quire_session` sent on a plain-HTTP request is ignored, and `quire_session` on an HTTPS one) |
| Login | `auth::login_rate_limited_429_retry_after` (`FakeRelativeClock`) |
| Login | `auth::email_limiter_counts_failures_only` |
| Login | `auth::unknown_email_same_code_as_bad_password` |
| Sessions | `auth::renewal_after_half_window_resends_cookie` (persistent: new Max-Age; 180 d cap) |
| Sessions | `auth::password_change_revokes_other_sessions` |
| Sessions | `auth::reset_password_cli_sets_must_change` |
| Security | `security::host_not_allowlisted_421` (and `/healthz` exempt) |
| Security | `security::host_allowlist_accepts_quire_domain_any_port` (`QUIRE_DOMAIN=quire.lan`: `quire.lan` and `quire.lan:8443` pass, `evil.lan` gets 421) |
| Security | `security::forwarded_headers_only_from_trusted_proxy` (peer injected through `ConnectInfo`: from an untrusted peer, `X-Forwarded-For` does not change the login limiter's key and `X-Forwarded-Proto: https` gives no `Secure`; from `QUIRE_TRUSTED_PROXIES` both apply and the rightmost address counts) |
| Security | `security::hsts_only_on_https_in_acme_mode` (present with `QUIRE_TLS=acme` and a trusted `https`; absent on plain HTTP, absent with `QUIRE_TLS=internal`) |
| Security | `security::public_url_derived_from_domain` (`QUIRE_DOMAIN` + `QUIRE_HTTPS_PORT=8443` → `https://<domain>:8443` in the setup log and the CSRF origin; an explicit `QUIRE_PUBLIC_URL` wins) |
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
| Export | `export::extracted_db_has_no_credentials_without_sidecars` (tar extracted into an empty dir, `quire.db` opened read-only with no `-wal`: the tables are empty; the raw bytes contain no `$argon2id$` and no known token or session sha256) |
| Export | `export::single_flight_429_busy` |
| Export | `export::cookie_cross_site_navigation_403` (`Sec-Fetch-Site: cross-site`; and, without fetch metadata, no or a foreign `Referer`) |
| Backup | `backup::on_demand_counts_against_keep` |
| Backup | `backup::vacuum_into_through_db_passes_quick_check` (the real `Db`; a `query_only` reader would fail) |
| Blobs | `blobs::gc_keeps_referenced_blobs_including_revision_assets` |
| Blobs | `blobs::gc_between_stage_and_commit_never_leaves_row_without_file` |
| Blobs | `blobs::attachment_gc_respects_refs_and_24h_grace` |
| Tokens | `tokens::create_shows_secret_once_list_never` (list and `Me` never contain the secret or hash) |
| Tokens | `tokens::scopes_expanded_and_capped_by_caller` (a write token cannot mint admin: 422) |
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
| Security | `security::quire_bearer_wins_over_cookie` (valid cookie + bad `qpat_` token → 401 `invalid_token`) |
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
  "Quire was updated. Reload" banner when a route chunk fails to load (§9.1).
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
  - `quire-core::ytext` (§6.1), including `is_empty`.
  - **Conversion** (§6.4, D49, D50): `quire-core::schema` (schema v1 as data, checked against
    `fixtures/schema/v1.json`; `normalize`) and `quire-core::convert` (Y ↔ raw and normalized ProseMirror JSON after
    `@tiptap/y-tiptap`, the nesting pre-scan, the Markdown parser on comrak and Quire's printer with its block map, the
    scheme allow-list, the diff writer: Markdown-space comparison, `blockId` and hash anchors, similarity pairing,
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
| `crates/quire-api` | `src/docs.rs` |
| `crates/quire-core` | `src/{ytext,schema}.rs`, `src/convert/**` |
| `crates/quire-store` | `src/docs.rs`, `tests/store/docs.rs` |
| `crates/quire-server` | `src/routes/docs.rs`, `tests/api/docs.rs` |
| `web` | `src/pages/DocPage.tsx`, `src/editor/**`, `src/api/queries/docs.ts` |
| `fixtures` | `fixtures/ydoc/**`, `fixtures/markdown/**`, `fixtures/schema/**` |
| `web/src/api/gen` | the generated files for the types in `quire-api` `docs.rs` |

### Produces

- The doc routes and schema v1 (§6.1). Renaming a node is a breaking change.
- Quire Markdown 1 and the ProseMirror JSON mapping (§6.4): the printer's output is a contract (golden files); a change
  to it is an additive API change only when old output still parses to the same document.
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
scripts/dev.sh env WRITE_FIXTURES=1 cargo test -p quire-core --locked -- convert::tests::write_server_fixtures \
  && git diff --exit-code fixtures/ydoc/server
scripts/dev.sh cargo test -p quire-core   --locked -- ytext:: schema:: convert::
scripts/dev.sh cargo test -p quire-store  --locked --test store -- docs::
scripts/dev.sh cargo test -p quire-server --locked --test api   -- docs::
scripts/dev.sh cargo test -p quire-server --locked --test api   -- openapi:: && git diff --exit-code docs/api/openapi.json
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
- `window.quireBoardFrame` and `globalThis.inkboardEmbed` (§10.3).
- The `engine_version` string, `0.2.0-beta.3+embed.quire.<patchset>`.

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
scripts/dev.sh scripts/inkboard.sh export --upstream && ! grep -rq 'Changed for Quire' .cache/inkboard-upstream
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
  - `quire-core::board` split, splice, inspect and preview (§6.2): raw-JSON splicing, no `Value` trees.
  - The board store: the PUT pipeline with the `board_work` permit, `BlobStore::stage` outside and `commit` inside the
    writer, the no-op hash, If-Match/412/428, meta, text, revisions with `assets`; GET with splice and title under the
    same permit; the `.inkboard` download.
  - The preview worker and the per-page route `…/board/pages/{n}` (`image/svg+xml`, no `.svg` suffix; §11 headers;
    no `rev`: the current page with `no-cache`; 307 for a stale rev; 204 for an empty page).
  - The board API rules (D51, D53): `If-Match: *` and `W/` tags, `X-Quire-Engine` optional (stored as `"api"`),
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
| `crates/quire-api` | `src/boards.rs` |
| `crates/quire-core` | `src/board.rs` |
| `crates/quire-store` | `src/{boards,previews}.rs`, `tests/store/boards.rs` |
| `crates/quire-server` | `src/routes/boards.rs`, `src/board_previews.rs`, `tests/api/boards.rs` |
| `web` | `src/pages/BoardPage.tsx`, `src/board/**` except `bridge.ts`, `src/api/queries/boards.ts` |
| `fixtures` | `fixtures/boards/**` except `frame-test.inkboard` |
| `web/src/api/gen` | the generated files for the types in `quire-api` `boards.rs` |

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
scripts/dev.sh cargo test -p quire-core   --locked -- board::
scripts/dev.sh cargo test -p quire-store  --locked --test store -- boards::
scripts/dev.sh cargo test -p quire-server --locked --test api   -- boards::
scripts/dev.sh cargo test -p quire-server --locked --test api   -- openapi:: && git diff --exit-code docs/api/openapi.json
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
- The `quire` CLI of §7.6 and §7.7, complete: every command of the §7.6 table with exactly its flags and `--json`
  output, the exit-code table, the URL and token resolved as a pair, the profile file (0700 directory, 0600 atomic
  write, refusal when group-readable with a token), the offline token check, references (UUID, `url_id` or URL),
  `--all` with the truncation rule, the retry rule (only unprocessed requests), no redirects, timeouts, TLS roots from
  the system bundle or `--ca-file`, plain HTTP refused for non-loopback hosts, `--if-match` (`d42`, `"d42"`, `W/"d42"`)
  or `--force` on content writes, `--yes` on `trash purge`, `item create --md` (title only after the PUT, discard on
  failure), `doc put --attach-images` with its containment rules, the `api` path and header guards, `--token-out`,
  `--debug`, the 204 ok object, JSON Lines on stderr with `details.sent`, the `admin` passthrough through
  `QUIRE_SERVER_BIN`, `version`, `exit-codes` and `help --json`.
- `scripts/cli-dist.sh` (§13.2): the static musl binary in `dist/cli/` with its sha256 file, `LICENSE-*` and
  `THIRD_PARTY.md`.
- The CLI test binary `crates/quire-cli/tests/cli/` (modules below), against `quire_server::testing::TestServer` and a
  scripted mock HTTP listener written in the test (std `TcpListener`, no extra crate).
- The agent harness of §14.1: `scripts/agent-test.sh` (`run`, `up --ttl` with the 15-minute default, `status`, `down`,
  the lock holder, `quire_slice`, the project subnet `172.31.240.32/28` with range `172.31.240.40/29`,
  `--env-file /dev/null` with the owner-facing variables unset (§13.3), `up -d --wait quire` so that Caddy never
  starts and the CLI talks to Quire directly, the stale-image check, its exit codes), `e2e/agent/lib.sh`,
  `e2e/agent/scenarios/*.sh`, `e2e/agent/fixtures/**`, `compose.agent.yaml` (§13.3; a plain `ports:` list, since the
  base file publishes nothing for `quire`).
- `docs/AGENTS.md`, short, with no secrets and no real hostnames, in this order:
  1. what the CLI is and where it runs (in the image, or the standalone binary);
  2. a cheat sheet: one line per command group of §7.6 with its main flags and what `--json` prints (items are
     published on create; `item unpublish` makes a draft, D23);
  3. getting a token (Settings → API tokens, or `docker compose exec quire quire admin create-token --name … --scope
     write --expires 1d --token-out /tmp/agent.token`, or `umask 077; quire token create … --json | jq -r .token >
     file`) and using it through `QUIRE_TOKEN_FILE` or `quire auth login --token-stdin`, never argv or shell history;
     `QUIRE_URL`;
  4. always `--json`: stdout is one JSON value, stderr is JSON Lines whose last line is the error;
  5. the §7.7 exit-code table with the action for every code 0–12, including exit 4 with `details.sent: true` after a
     create (list before retrying) and exit 3 after a redirect (fix the URL);
  6. one complete transcript: scratch collection → doc from Markdown → `doc get` (keep the ETag) → edit → a stale PUT
     exits 9 → `--merge` → search → star → trash → restore;
  7. the Markdown dialect in five lines with a pointer to ARCHITECTURE §6.4 for the lossy constructs, and `quire doc
     schema` for JSON;
  8. boards as native JSON: keep unknown fields; Inkboard's model is in `vendor/inkboard/docs/ARCHITECTURE.md`
     ("Model") and `crates/inkboard-core/src/{model,serial}.rs`; no import from other formats in M1 (D52);
  9. running `scripts/agent-test.sh` (`--scenario`, `up --ttl`, `status`, `down`, the report file, its exit codes);
  10. the rules for agents: builds only through `scripts/dev.sh`, never print or commit a token, work in a scratch
      collection and trash what you create, never purge outside the harness.

### Owns

| Area | Files |
|---|---|
| `crates/quire-cli` | everything: `Cargo.toml`, `src/**`, `tests/cli/**` |
| `crates/quire-api` | `src/cli.rs` (the exit-code table) |
| Scripts | `scripts/agent-test.sh`, `scripts/cli-dist.sh` |
| Harness | `e2e/agent/**`, `compose.agent.yaml` |
| Docs | `docs/AGENTS.md` |

### Produces

- The CLI command tree of §7.6, its JSON output and its exit codes (§7.7): a contract for agents, CI and W7's gate.
- The harness interface: `scripts/agent-test.sh [run|up|status|down] [--scenario GLOB] [--keep] [--ttl MIN]
  [--allow-stale]` with exit codes 0, 1, 3 and 4, the variables `QUIRE_IMAGE`, `QUIRE_AGENT_CLI` and
  `QUIRE_AGENT_PORT`, the helpers of `e2e/agent/lib.sh` (`q`, `q_raw`, `expect_rc`, `expect`, `expect_err`, `ok`,
  `fixture`, `scratch_collection`; `$OUT_FILE`, `$OUT`, `$ERR`, `$RC`), and `e2e/agent/results/report.json` (tokens
  masked).

### Consumes

- `quire-api` DTOs, `ErrorBody`, `Page<T>` and `token::check` (W0b; files owned by W1, W2, W4 and W6).
- `quire_server::testing::TestServer`, bearer authentication and the W0b minimal routes `GET /auth/me`, `/tokens*` and
  `POST /collections` (W0b minimal, W1 and W2 full).
- `quire-server admin create-owner` and `admin create-token` (W0b minimal with the final flags, W2 full).
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
scripts/dev.sh cargo test -p quire-cli --locked --test cli -- config:: transport:: output:: exit_codes:: api:: auth:: tokens:: admin:: coverage::
scripts/dev.sh cargo clippy -p quire-cli --all-targets --locked -- -D warnings
! scripts/dev.sh cargo tree --locked -p quire-cli -e normal --prefix none | grep -qE '^(openssl|openssl-sys|native-tls|rusqlite|yrs|inkboard-core|comrak|webpki-roots) '
bash -n scripts/agent-test.sh scripts/cli-dist.sh e2e/agent/lib.sh e2e/agent/scenarios/*.sh
scripts/cli-dist.sh && scripts/dev.sh bash -c 'f=dist/cli/quire-x86_64-unknown-linux-musl \
  && { ldd "$f" 2>&1 | grep -qE "not a dynamic executable|statically linked"; } && "$f" --version \
  && (cd dist/cli && sha256sum -c quire-x86_64-unknown-linux-musl.sha256) && test -s dist/cli/THIRD_PARTY.md'
scripts/image.sh --build-arg INKBOARD_BUILD=stub && scripts/agent-test.sh --scenario '01-*'
scripts/agent-test.sh up --ttl 5 && test -s .cache/agent-test/env && scripts/agent-test.sh status \
  && scripts/agent-test.sh down && test ! -e .cache/agent-test/env
```

`coverage::every_spec_operation_has_command_or_api_only_allowlist` reads the committed `docs/api/openapi.json`, so in
the parallel phase it covers the operations merged so far; W7's gate runs it on the full spec.

These tests must exist and pass (the `flows::` ones in W7's gate):

| Module | Test |
|---|---|
| config | `config::url_and_token_resolve_as_a_pair` (every row of §7.6 "Configuration"; an explicit `--profile` beats `QUIRE_URL`/`QUIRE_TOKEN`) |
| config | `config::profile_token_never_sent_to_overridden_url` (`QUIRE_URL` elsewhere: exit 3, the mock listener sees no `Authorization`) |
| config | `config::group_readable_profile_with_token_exit_3` · `config::group_readable_token_file_warns` |
| config | `config::malformed_token_exit_3_without_request` (the mock listener records no connection) |
| config | `config::login_writes_profile_0600_in_0700_dir_atomically` |
| config | `config::token_stdin_with_token_file_exit_2` |
| transport | `transport::retries_only_unprocessed_requests` (answered 429/503 and connect failures for every method; a timeout retried for GET only; a PUT and a POST whose response is lost are not retried and exit 4 with `details.sent: true`; `Retry-After` honoured) |
| transport | `transport::connection_refused_exit_4_sent_false` · `transport::timeout_exit_4` |
| transport | `transport::plain_http_refused_for_non_loopback` (`QUIRE_ALLOW_HTTP=1` and `allow_http` allow it; loopback always allowed) |
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
| admin | `admin::passthrough_execs_quire_server_with_args_and_exit_code` (a stub through `QUIRE_SERVER_BIN`) |
| admin | `admin::missing_quire_server_exit_3` (`QUIRE_SERVER_BIN` unset, no sibling binary, empty `PATH`) |
| coverage | `coverage::every_spec_operation_has_command_or_api_only_allowlist` (the §7.6 allow-list) |
| coverage | `coverage::agents_md_examples_parse` (every `quire …` line in `docs/AGENTS.md` parses with the clap tree) |
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
| `03-docs.sh` | For every `fixtures/markdown/*.md` (W4's goldens): `item create --md -`, then `doc get --format md` equals its `.expected.md` and the PUT's warning codes equal its `.warnings.json`; `doc get --format json` has `schema_version` 1; a PUT with an ETag from before a second PUT exits 9; the same PUT with `--merge` succeeds with `merged: true` and both edits present; repeating that command gives `changed: false`; `--title-h1` sets the title |
| `04-boards.sh` | `board put -` of `fixtures/future.inkboard` (`x_future` fields) with `--if-match r1`; `board get` keeps both `x_future` values and `document.name` equals the title; `board preview --page 0 --out /tmp/agent-p0.svg` gives `content_type` `image/svg+xml` (bounded poll for the preview); a stale `--if-match` exits 9; `--force` succeeds |
| `05-search.sh` | After a PUT containing "quokka", `search quok` lists that doc first; `search "quok -zebra"` still finds it; `search --titles` finds it; `--all` merges pages |
| `06-stars-lists.sh` | `star add` prints the ok object; `list starred`; a new doc is published on create (`item get` has `effectively_published: true`; `sidebar`'s `drafts_count` is unchanged and `list drafts` does not list it); `item unpublish` on it and on its parent puts both in `list drafts` and raises `drafts_count` by 2; `item publish --with-ancestors` on the child empties `list drafts` again |
| `07-trash.sh` | Trash B: `tree` lacks B and C; `trash list` shows B with one descendant; `item restore` brings both back; `trash purge B` without `--yes` exits 2, with the write token exits 6, with the admin token and `--yes` succeeds (its own item only, never `--all`) |
| `08-attachments.sh` | `attachment upload -` of `fixtures/pixel.png`; a Markdown PUT that references `/api/v1/attachments/<id>` round-trips; `q_raw` of `attachment get` has the same sha256 |
| `09-export.sh` | `q_raw export.tar export` is a tar with `manifest.json` and `quire.db`; with the write token it exits 6 |
| `10-tokens.sh` | `token create --scope read --expires 1d` prints `qpat_…` once and the report shows it masked; `--token-out /tmp/agent-t.token` prints no secret; `token list` shows the prefix and never the secret; `token revoke`, then reuse exits 5; the admin token minting an `admin` token exits 6; a child of a second admin token (from `quire admin create-token`) exits 5 once that parent is revoked |
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
  - the setup code (`docker compose logs quire`);
  - the runtime limits and how to change them (app `QUIRE_CPUS=2`, `QUIRE_MEM=1g`; Caddy `QUIRE_CADDY_CPUS=0.5`,
    `QUIRE_CADDY_MEM=128m`; no swap); `QUIRE_ALLOWED_HOSTS`;
  - HTTPS through the bundled Caddy (ARCHITECTURE §13.4): LAN or local mode (`QUIRE_TLS=internal`, `QUIRE_DOMAIN`,
    `QUIRE_BIND`) with the steps to trust the local CA (system bundle, Chrome/Chromium NSS, Firefox, other devices,
    the CLI's `--ca-file`) and to keep `root.key` secret; public mode (`QUIRE_TLS=acme`, a public `QUIRE_DOMAIN`,
    `QUIRE_BIND=0.0.0.0`, DNS and ports 80/443 reachable); `QUIRE_HTTP_PORT`/`QUIRE_HTTPS_PORT` when 80 or 443 are
    taken; `QUIRE_NET_SUBNET`, `QUIRE_NET_RANGE` and `QUIRE_PROXY_IP` together if the subnet collides (the proxy
    address inside the subnet, outside the range); `docker compose down -v` deletes `caddy-data`, and with it the
    local CA every device trusts; HSTS only in public mode; back up
    `caddy-data`;
  - backups and restore, export restore needing `reset-password`, the need for an off-host copy, bind-mount ownership;
  - `quire admin reset-password`, `create-owner` and `create-token`; the compromise steps
    (`quire admin reset-password --email … --revoke-tokens`, or `revoke-token --all --yes`);
  - compose sets `QUIRE_TRUSTED_PROXIES` to Caddy's fixed address, so only Caddy's `X-Forwarded-For` and
    `X-Forwarded-Proto` count; a deployment that swaps Caddy for another proxy must set it (every per-IP limit uses
    it), and a proxy that adds Basic auth or its own `Bearer` header keeps working (D43);
  - the public API: `/api/v1`, personal access tokens and scopes, the OpenAPI file (`/api/v1/openapi.json`,
    `docs/api/openapi.json`), `QUIRE_TOKEN_RATE`, `QUIRE_TOKEN_BYTES_PER_HOUR`, `QUIRE_MIN_FREE_MB`,
    `QUIRE_CORS_ORIGINS`, and `QUIRE_ALLOWED_HOSTS` for clients on a Docker network;
  - the CLI: `docker compose exec quire quire …` and the standalone binary from `scripts/cli-dist.sh`, with a pointer to
    `docs/AGENTS.md`;
  - a note that builds never run on the host, and the build slice: installed once (`scripts/install-build-slice.sh`,
    a root step), checked with `--check`, and optional for anyone else building Quire (scripts warn and keep the
    per-container caps).
- Build `THIRD_PARTY.md` and the About page's licence data: a build-time `licenses.txt` from `cargo metadata` and
  `pnpm licenses list --json`, plus Inkboard's `THIRD_PARTY_LICENSES.md` and `ICONS-LICENSES.txt`. comrak is
  BSD-2-Clause; any code W4 ported from `@tiptap/y-tiptap` (MIT) gets its entry. `THIRD_PARTY.md` covers every crate
  in the workspace lock, so the `cli-dist` stage ships it, with `LICENSE-*`, next to the standalone binary (§13.2).
- Update `docs/ARCHITECTURE.md` with every deviation recorded in the handoff notes.

### Owns (from the start of the parallel phase; W7 may draft early, but runs its gate last)

| Area | Files |
|---|---|
| Rust workspace | `Cargo.toml`, `rust-toolchain.toml`, `.cargo/config.toml`; `Cargo.lock` at merge time (R6) |
| Build and deploy | `docker/**` (including `Caddyfile`, `quire.slice`, `quire-build.slice`), `compose.yaml`, `compose.e2e.yaml`, `compose.smoke.yaml`, `.dockerignore`, `.gitignore` |
| Scripts | `scripts/{dev,toolchain,image,image-smoke,e2e,fmt,check-pins,install-build-slice}.sh`, `scripts/lib/**` |
| e2e | `e2e/package.json`, `e2e/pnpm-lock.yaml`, `e2e/playwright.config.ts`, `e2e/tests/smoke.spec.ts` |
| Docs and licences | `README.md`, `THIRD_PARTY.md`, `LICENSE-*`, `CLAUDE.md`, `docs/ARCHITECTURE.md`, `docs/PLAN.md`; `docs/api/openapi.json` (generated: every route owner regenerates it, R4) |
| Frozen module lists | `crates/quire-api/src/lib.rs`, `crates/quire-core/src/lib.rs`, `crates/quire-store/src/lib.rs`, `crates/quire-server/src/lib.rs`, `crates/quire-server/src/routes/mod.rs` |
| Test binaries | `crates/quire-store/tests/store/main.rs`, `crates/quire-server/tests/api/main.rs`, `crates/quire-server/tests/api/integration.rs` |

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

### Smoke spec (`e2e/tests/smoke.spec.ts`, against `http://quire:8080` in the compose e2e project)

Playwright reaches Quire directly over plain HTTP on the project network; Caddy is not started (ARCHITECTURE §14).
The HTTPS path through Caddy is covered by `QUIRE_SMOKE_LEVEL=full scripts/image-smoke.sh`.

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

### Acceptance (the M1 release gate)

```sh
scripts/install-build-slice.sh --check            # the owner's machine: the slice is installed and holds its values
scripts/check-pins.sh
docker compose -f compose.yaml config --format json | jq -e '(.services.quire.ports // []) == []'
! grep -qE '^\s*(encode|header)\b' docker/Caddyfile
scripts/fmt.sh --check
scripts/dev.sh cargo clippy --workspace --all-targets --locked -- -D warnings
scripts/dev.sh cargo tree --locked -e normal -i libsqlite3-sys
scripts/dev.sh cargo test --workspace --locked
scripts/dev.sh cargo test -p quire-api --locked export_bindings && git diff --exit-code web/src/api/gen
scripts/dev.sh cargo test -p quire-server --locked --test api -- openapi:: && git diff --exit-code docs/api/openapi.json
scripts/dev.sh cargo test -p quire-cli --locked                     # every module, including flows::
! scripts/dev.sh cargo tree --locked -p quire-server -e normal --prefix none | grep -qE '^(rustls|ureq|reqwest|native-tls|openssl) '
! scripts/dev.sh cargo tree --locked -p quire-cli -e normal --prefix none | grep -qE '^(openssl|openssl-sys|native-tls|rusqlite|yrs|inkboard-core|comrak|webpki-roots) '
! grep -rn '\.route(' crates/quire-server/src/routes
! grep -rn '\.route("/api' crates/quire-server/src
scripts/dev.sh pnpm --dir web install --frozen-lockfile
scripts/dev.sh pnpm --dir web fixtures && git diff --exit-code fixtures
scripts/dev.sh env WRITE_FIXTURES=1 cargo test -p quire-core --locked -- convert::tests::write_server_fixtures && git diff --exit-code fixtures/ydoc/server
scripts/dev.sh pnpm --dir web typecheck && scripts/dev.sh pnpm --dir web test && scripts/dev.sh pnpm --dir web build
scripts/dev.sh scripts/inkboard.sh check && scripts/dev.sh scripts/inkboard.sh build && scripts/e2e.sh frame
scripts/image.sh
QUIRE_SMOKE_LEVEL=full scripts/image-smoke.sh     # app 2 CPUs / 1 GiB, Caddy 0.5 CPU / 128 MiB, no swap; no published app port;
                                                  # HTTPS via Caddy with the local CA; br wasm passed through byte for byte;
                                                  # __Host- Secure cookie; no HSTS in internal mode; CSP; peak memory < limits
scripts/e2e.sh smoke
scripts/agent-test.sh                              # every scenario, against the image just built (no --allow-stale)
scripts/cli-dist.sh && scripts/dev.sh dist/cli/quire-x86_64-unknown-linux-musl --version
! grep -rnE 'todo!\(|unimplemented!\(|TODO\(W|not implemented \(W' crates web/src web/inkboard-frame scripts e2e/agent
git -C vendor/inkboard status --porcelain | wc -l | grep -qx 0
```
