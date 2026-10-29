# W0b handoff: scaffold, contracts, Docker image, harnesses

Status: tasks 1–9 done. Every W0b acceptance command (PLAN lines 480–505) and every R10 check passes (2026-10-28,
final run after the review fixes). Nothing was committed and nothing ran as root: the owner was away and asked for the
work to go ahead without sudo. One question is still open for the owner: the MPL-2.0 licence of lightningcss, a
build-time dependency of the pinned vite 8.3.5 (see "Open for the owner").

**The build slice is still not installed (W0a task 8 is still deferred).** Every build in W0b ran with the
per-container caps and printed "quire-build.slice not installed: per-container caps only". `quire-buildkit` still has
no cgroup parent. When the owner is back, run the install and the post-install acceptance from
`docs/handoff/W0a.md` ("Coordinator: the one privileged step" and "Task 8"). After the install,
`scripts/toolchain.sh --builder-only` exits 1 with the recreate instruction until the builder is recreated. The
toolchain build cache now lives in the `quire-buildkit-state` volume, which `docker rm -f quire-buildkit` keeps, so the
recreate should not need a full rebuild.

## What was built

| File | What it does |
|---|---|
| `docker/Dockerfile` | §13.2, with the three base-image index digests and the binaryen `version_131` checksum filled in (pins below). The only other change is the header line that said "W0b fills in…". Stages: toolchain-base, toolchain, inkboard-web, web, server, cli, cli-dist, runtime. |
| `docker/toolchain.Dockerfile` | Deleted. Its content is now the `toolchain-base` and `toolchain` stages. |
| `scripts/toolchain.sh` | Builds `--target toolchain -f docker/Dockerfile` with the repo root as context. The builder logic is unchanged; only the header comment and usage line range changed. `quire-toolchain:latest` was rebuilt (image `8011426db92e`) and now has clippy, rustfmt, wasm-opt 131, brotli 1.1.0 and musl-gcc, plus everything the old image had (rustc 1.99.1, trunk 0.22.1, wasm-bindgen 0.2.129, clang/lld/llvm 19, node 24.22.1, pnpm 11.24.0). |
| `.dockerignore` | §13.2, byte for byte. |
| `compose.e2e.yaml`, `compose.smoke.yaml` | §13.3, plus `mode: 0o1777` on the quire `/data` tmpfs (deviation D1). |
| `scripts/image.sh` | Builds `quire:<short sha>[-dirty]` and `quire:latest` (target `runtime`) under the lock, with the `quire-limited` builder, the `org.opencontainers.image.revision` label and `QUIRE_COMMIT`. It runs `toolchain.sh --builder-only` first, so a drifted builder stops the build. |
| `scripts/image-smoke.sh` | Project `quire-smoke` on `172.31.240.48/28`. `basic` runs the PLAN checks plus a few more; `full` adds the §14 checks. Always ends with `down -v`. |
| `scripts/e2e.sh` | `frame` (Playwright against the stub dist) and `smoke` (only `quire`, plain HTTP on the project network), with the Playwright image pinned by digest and capped through `quire_caps`. |
| `scripts/fmt.sh` | `cargo fmt -p <six crates> [--check]` through `dev.sh`. |
| `scripts/check-pins.sh` | Host only, text and git. It checks the submodule (gitlink and HEAD at `41e0295…`, clean) and that the shared crates have the same versions in both lockfiles. Also: no `{` in `.dockerignore`, the toolchain channel equals the `RUST_IMAGE` tag, no digest or checksum placeholder is left, and `WASM_BINDGEN_VERSION` equals Inkboard's lock. |
| `scripts/inkboard.sh` | `src`, `build`, `export`, `check` and `size` print "not implemented (W5)" and exit 1. `stub` works: it writes `frame.html` and `frame.js` with their `.br` and `.gz` copies, plus `manifest.json` (format_version 2, parsed from `serial.rs`). |
| `patches/inkboard/README.md` | Base, the patch table (empty so far), notice lines, workflow, dist layout and manifest format, and a sizes table for W5. |
| `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.cargo/config.toml` | §4.1 and §3. One libsqlite3-sys (0.38.2). The shared crates match Inkboard's lock without any `--precise` (serde 1.0.229, serde_json 1.0.154, base64 0.23.1, thiserror 2.0.21). `.cargo/config.toml` keeps `jobs = 6` and adds `TS_RS_EXPORT_DIR` and `TS_RS_LARGE_INT` (D9). |
| `crates/quire-api/src/{lib,common,tree,page,auth,tokens,files,docs,boards,error,cli,token}.rs` | Every DTO of §7.1–§7.3 and §7.5, with serde, ts-rs and utoipa derives. Also `ErrorCode` (all 34 codes and their statuses), `ErrorBody`, `Page<T>`, `Scope`, `API_VERSION = "1.0"`, `cli::ExitCode` with `from_status`, and `token::{encode, check}`. |
| `web/src/api/gen/*.ts` | 101 generated files. A regeneration into a scratch directory is identical. |
| `crates/quire-core/src/*` | `cursor::{encode, decode, query_hash}` complete (D46). `pub use quire_api as api;`. The other modules (`ids`, `rank`, `tree`, `time`, `ytext`, `schema`, `board`, `convert/{mod,pm,ydiff,md_in,md_out}`) are doc-only stubs. |
| `crates/quire-store/migrations/0001-init/up.sql` | The §5 DDL, byte for byte. |
| `crates/quire-store/src/*` | Working: `db`, `migrate`, `backup`, `shared`, `revisions`, `blobs`, `docs`, `boards`, `collections`, `items`, `tokens` and `error`. The remaining modules (`tree`, `search`, `stars`, `views`, `lists`, `users`, `sessions`, `settings`, `attachments`, `previews`, `gc`, `export`, `seed`) are doc-only stubs. |
| `crates/quire-store/tests/store/*` | `TestDb` harness, plus 40 integration tests over 10 modules (`auth.rs` is a stub). |
| `crates/quire-server/src/*` | Frozen server types, `Auth<S>` with both branches of §7.3, the error envelope, the Host allowlist and CSRF check, `config.rs`, `serve`, `healthcheck`, and `admin create-owner` / `create-token`. The other admin commands exit 1 with "not implemented (W2)". 15 routers; minimal routes: GET /auth/me, GET and POST /tokens, DELETE /tokens/{id}, DELETE /tokens/current, POST /collections, GET /api/v1/openapi.json. Also `testing::TestServer` and a no-op `board_previews`. |
| `crates/quire-server/tests/api/*` | `TestApp::{new, with_owner, with_token}` plus helpers, and 39 API tests. |
| `docs/api/openapi.json` | Generated. OpenAPI 3.1.0, 7 operations, the bearer and session security schemes. |
| `crates/quire-assets/src/lib.rs` | `Web` and `Inkboard` embeds (relative folders, `allow_missing`). |
| `crates/quire-cli/src/*` | clap tree with the nine §7.6 global flags, `--version`, and `exit-codes [--json]`. The §7.7 usage-error JSON line. Stubs for every `cmd/*.rs` and for `config`, `http` and `refs`. |
| `crates/quire-cli/tests/cli/*` | `Sandbox` and `Harness` (TestServer plus assert_cmd), and 6 tests. `quire --version` runs against a live TestServer. |
| `web/**` (58 files plus `package.json` and `pnpm-lock.yaml`, not counting `src/api/gen`) | PLAN task 7: the §4.2 dependencies at exact versions; vite, tsconfig and Vitest config; every §9.1 route; the contract modules; `bridge.ts` (§10.4); the placeholder frame; and 43 tests in 8 files. |
| `e2e/{package.json,pnpm-lock.yaml,playwright.config.ts,serve-static.mjs,tests/*.spec.ts}` | @playwright/test 1.64.1 only. Two projects, `frame` and `smoke`, with the SwiftShader flags. `serve-static.mjs` serves `.wasm` as `application/wasm` and never serves outside its root. |
| `fixtures/ydoc/getting-started.bin`, `fixtures/boards/getting-started.inkboard` | `[0,0]`, and the empty light board (generated by a store test; `WRITE_FIXTURES=1` rewrites it only when its content differs). |
| `LICENSE`, `THIRD_PARTY.md`, `README.md` | The licences, Inkboard's entry (W7 completes the rest), and a stub README. |
| `.gitignore` | The five §3 lines, plus `/web/.dist-inkboard.*/` (the staging directories of `inkboard.sh`). |
| `CLAUDE.md` | Task 9: the `docker/Dockerfile` toolchain target, and the agent-harness / `docs/AGENTS.md` line. |
| `docs/ARCHITECTURE.md` | §16 item 24 only (W0b pin checks). No other line changed. |

Docker state: `quire-toolchain:latest` rebuilt (`8011426db92e`); `quire:1b59aa0-dirty` = `quire:latest`
(`0285cc1c739b`, built from the final tree); `mcr.microsoft.com/playwright:v1.64.1-noble` pulled. No smoke or e2e
container, network or volume is left. Git-ignored output left on disk: `web/dist-inkboard/` (the stub), the
`node_modules` of `web/` and `e2e/`, `e2e/test-results/` and `e2e/playwright-report/`. `web/dist` was removed.

## Acceptance (final run, 2026-10-28, after the review fixes)

Each command ran verbatim in its own `bash -c` from the repo root, in this order. Every one passed.

| # | Command (abridged) | Result |
|---|---|---|
| 1 | `bash -n scripts/*.sh scripts/lib/*.sh` | PASS. As written this checks only the first file (request W7-2), so I also ran the per-file loop: PASS |
| 2 | `scripts/toolchain.sh` | PASS (cached; builder drift check passes; expected slice warning) |
| 3 | `rustup show active-toolchain` = `1.99.1-…` | PASS |
| 4 | `cargo clippy --version && rustfmt --version` | PASS (clippy 0.1.99, rustfmt 1.10.0-stable) |
| 5 | `wasm-opt --version` has `version 131` | PASS |
| 6 | `brotli --version` | PASS (1.1.0) |
| 7 | `scripts/check-pins.sh` | PASS ("every pin holds", 11 checks) |
| 8 | `cargo build --workspace --locked` | PASS |
| 9 | `cargo tree --locked -e normal -i libsqlite3-sys` | PASS (one version, 0.38.2) |
| 10 | `cargo test --workspace --locked` | PASS: 273 tests (api 121, cli 10 + 6, core 7, server 34 + 39, store 16 + 40) |
| 11 | `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS |
| 11b | the same with `--all-features` (R10) | PASS |
| 12 | `scripts/fmt.sh --check` | PASS |
| 13 | `cargo test -p quire-api export_bindings && git diff --exit-code web/src/api/gen` | PASS. The files are untracked, so `git diff` proves nothing yet. The sha256 over all 101 files was the same before and after the run, and a fresh export into `.cache/gen-check` (deleted afterwards) is identical (`diff -r`). |
| 14 | `cargo test -p quire-server --test api -- openapi:: && git diff --exit-code docs/api/openapi.json` | PASS (3 tests; the file is untracked, its sha256 is unchanged) |
| 15 | `cargo test -p quire-cli --locked --test cli` | PASS (6 tests) |
| 16 | `! cargo tree -p quire-server … \| grep -qE '^(rustls\|ureq\|reqwest\|native-tls\|openssl) '` | PASS. cargo tree printed more than 100 lines, so the check really ran |
| 17 | `! cargo tree -p quire-cli … \| grep -qE '^(openssl\|…\|webpki-roots) '` | PASS. More than 50 lines, so it ran |
| 18 | `cargo run -p quire-cli -- exit-codes --json \| grep -q '"code": *9'` | PASS |
| 19 | `pnpm --dir web install --frozen-lockfile` | PASS |
| 20 | `pnpm --dir web typecheck && … test && … build` | PASS (both tsc projects; 8 files, 43 tests; vite 8.3.5 build). `web/dist` was removed afterwards |
| 21 | `pnpm --dir e2e install --frozen-lockfile` | PASS |
| 22 | `scripts/inkboard.sh stub && grep -q '"format_version": *2' …/manifest.json` | PASS |
| 23 | `docker compose -f compose.yaml -f compose.e2e.yaml config -q` | PASS |
| 24 | `docker compose -f compose.yaml -f compose.smoke.yaml config -q` | PASS |
| 25 | `scripts/image.sh --build-arg INKBOARD_BUILD=stub && QUIRE_SMOKE_LEVEL=basic scripts/image-smoke.sh` | PASS. Every basic check passed: both `--version`s in the image; no published app port; Caddy only on 127.0.0.1:18443; Caddy's root read from its tmpfs; HTTPS `/healthz` 200 `{"status":"ok"}` with the CA, and curl exit 60 without it; the caps on both containers; cgroup parent none (no slice) |
| 26 | `git -C vendor/inkboard status --porcelain \| wc -l \| grep -qx 0` | PASS |
| R10 | `grep -rnE 'todo!\(\|unimplemented!\(\|TODO\(W'` over crates, web/src, web/inkboard-frame, scripts, e2e, docker, fixtures, patches, the compose files, Cargo.toml, rust-toolchain.toml, .cargo and docs/api | PASS (no match) |
| extra | `scripts/e2e.sh smoke` (real image) / `scripts/e2e.sh frame` (stub dist) | PASS (1 passed / 2 passed) |
| extra | no `quire-smoke*` or `quire-e2e*` container or network left | PASS |

Licences:
- Cargo: all 372 locked packages offer an allowed licence option. No webpki-roots, openssl, native-tls, aws-lc or
  option-ext.
- npm: every package is MIT, Apache-2.0, ISC or BSD, except lightningcss (MPL-2.0; see "Open for the owner").
- No TipTap Pro or Cloud package.

## Deviations and choices

Where the docs gave something "exactly" or as a frozen signature and W0b differs, the item says so. Everything else
is a choice the docs left open.

### Docker, compose and scripts

- **D1. `compose.e2e.yaml` and `compose.smoke.yaml` are not verbatim §13.3.** The quire `/data` tmpfs has
  `mode: 0o1777`, and the comment above it says why.
  - Why: the runtime image has `/data` (0755, owner 65532). runc gives the new tmpfs that directory's mode (0755) but
    owner root, so uid 65532 cannot create `/data/files`, and quire fails with "Permission denied".
  - The §13.3 YAML assumes the tmpfs default of 1777; its own comment says "mode 1777 lets uid 65532 write".
  - `mode: 1777` does not work, because compose reads it as decimal. The YAML octal `0o1777` resolves to 1023.
  - The merged config keeps every runtime limit.
  - §13.3 must get the same mode in both blocks (request W7-1). Do not re-sync these files to the current §13.3 text,
    or the e2e and smoke runs break.
- **D2. `docker/Dockerfile` header.** "W0b fills in the three digests and the binaryen checksum." became "The three base
  images are pinned by index digest and binaryen by its release checksum (W0b, 2026-10-28; docs/ARCHITECTURE.md
  §13.2)." Otherwise only the four placeholders changed. The `# syntax=docker/dockerfile:1.12` line stays unpinned,
  as in the doc.
- **D3. `scripts/toolchain.sh`.** The header names the toolchain target, so `usage()` now prints lines 2–13.
- **D4. `scripts/image.sh`.** It runs `toolchain.sh --builder-only` after taking the lock. `-dirty` is added only
  when git is available, never to `nocommit`. Extra arguments go to `docker buildx build`.
- **D5. `scripts/image-smoke.sh`.**
  - **Level and exit codes:** `QUIRE_SMOKE_LEVEL` defaults to `full`. Exit codes: 0 pass, 1 a check failed,
    2 usage, 3 a missing prerequisite (docker, curl, jq or the image), 4 the project not healthy.
  - **Environment:** besides the owner-facing variables, it unsets `QUIRE_PUBLIC_URL`, `QUIRE_ALLOWED_HOSTS`,
    `QUIRE_BACKUP_*`, `QUIRE_MAX_*`, `QUIRE_TOKEN_*`, `QUIRE_MIN_FREE_MB` and `QUIRE_CORS_ORIGINS`. It keeps
    `QUIRE_IMAGE` and `RUST_LOG`.
  - **Run:** it removes a leftover project first, waits at most 180 s, and prints log tails on failure.
  - **Extra basic checks:** Caddy publishes only `127.0.0.1:18443`; `/healthz` returns `{"status":"ok"}`; both
    containers' cgroup parent equals `QUIRE_CGROUP_PARENT`; without `--cacert` curl must exit 60 or 35.
  - **`full` level:**
    - The CSP check requires only `default-src 'none'` and `frame-ancestors 'none'`.
    - The br byte count and the wasm path come from `GET /api/v1/config`. A null `inkboard` fails with a hint that a
      stub image only passes `basic`.
    - Setup posts a fixed workspace, owner and 25-character password.
    - HSTS is checked on every response.
    - "`docker stats` stays under both limits" is background sampling of peak memory. Peak CPU is printed but not
      checked, because the CFS quota already enforces it.
  - Not yet run against a full (non-stub) image: that needs W5's engine build.
- **D6. `scripts/e2e.sh`.**
  - Extra arguments go to Playwright.
  - The Playwright image is pinned by digest.
  - Playwright's caps are min(4, `QUIRE_BUILD_CPUS`) and min(6g, `QUIRE_BUILD_MEM`). An explicit `QUIRE_BUILD_CPUSET`
    is cut down to its last N CPUs.
  - `smoke` removes a leftover `quire-e2e` project, waits at most 120 s, unsets the same variables as
    `image-smoke.sh`, and exits 3 or 4 like it.
- **D7. `scripts/check-pins.sh`.**
  - "Equal versions" is two rules. inkboard-core resolves each shared crate to the same version in both lockfiles.
    And every semver-compatible line that appears in both lockfiles has the same version.
  - Added checks: no placeholder is left in the Dockerfile, and `WASM_BINDGEN_VERSION` equals Inkboard's lock.
- **D8. `scripts/inkboard.sh`.** §10.5 leaves the manifest loose, so these are now defined (in the script and in
  `patches/inkboard/README.md`):
  - **Manifest fields:**
    - `build` is the first 12 hex digits of a sha256 over the sorted sha256sum lines of every file except
      `manifest.json`.
    - `files` lists every file except the manifest and the `.br`/`.gz` copies, as `{raw, br, gz}`, with null for a
      missing copy.
    - The stub's `patchset` and `engine_version` are both `"stub"`.
  - **The stub dist:** it also has a `frame.js` whose `connect` sends `onError{kind:'unsupported'}` and rejects, so a
    board host falls back to the preview. `frame.html` loads it as an external module, with no inline script.
  - **Writing the dist:** it stages in `web/.dist-inkboard.XXXXXX` and renames into place, and it refuses to replace
    a non-empty directory that has no `manifest.json`.
  - The dist is flat (`web/dist-inkboard/{manifest.json, frame.*, …}`), served under `/inkboard/{build}/`.

### Rust workspace and crates

- **D9. `.cargo/config.toml` also sets `TS_RS_LARGE_INT = "number"`.** ts-rs 12 otherwise exports `i64`/`u64` as
  `bigint`, but JSON numbers are JavaScript numbers. This is a contract choice: every `i64`/`u64` DTO field
  (`meta_version`, `rev`, sizes, …) is `number` in `web/src/api/gen`. It is recorded in §16 item 24; §3's layout line
  should mention it (request W7-3).
- **D10. rust-embed without `interpolate-folder-path`.** §4.1 names that feature, but it pulls option-ext (MPL-2.0).
  quire-assets uses `#[folder = "../../web/dist"]`, which rust-embed resolves against `CARGO_MANIFEST_DIR`. Recorded
  in §16 item 24.
- **D11. Dependencies beyond the §4.1 table:**
  - **quire-store:** inkboard-core, yrs, serde, serde_json, base64, thiserror, uuid (v7), getrandom, tracing, bytes,
    futures-util, and tokio with rt, sync, fs and io-util.
  - **quire-server:**
    - quire-api, inkboard-core, tower (util), tokio and tokio-util (io);
    - bytes, futures-util, serde, serde_json, uuid, sha2, base64, thiserror, getrandom, rustix, tracing and
      tracing-subscriber (env-filter);
    - **rusqlite**, for the plain-SQL reads listed under D24;
    - tempfile, optional, under `testing`.
  - **New workspace dependencies:** bytes 1.12.1 and futures-util 0.3.34.
  - **Default features off:** `tar` without its default features (no xattr); flate2 with only `rust_backend`.
  - **Versions picked where §4.1 gives none (each at least two weeks old):** include_dir 0.7.4, flate2 1.1.10,
    tar 0.4.46, imagesize 0.15.0, tokio-util 0.7.20, cookie 0.18.2, percent-encoding 2.3.2, tracing 0.1.44,
    tracing-subscriber 0.3.23, getrandom 0.4.3, toml 1.1.8, proptest 1.12.0, ts-rs 12.0.1. rustls 0.23.46 was exactly
    14 days old.
- **D12. Transitive crates younger than two weeks were moved back** with `cargo update --precise`:
  - arc-swap, async-compression, cc, compression-codecs, compression-core, ctutils, erased-serde, fearless_simd,
    find-msvc-tools;
  - the icu_* 2.4.0 family, back to 2.3.x;
  - libc, memchr-n, rustls-webpki, smallvec, tinystr, unicode-normalization, yoke, zerovec and zerovec-derive.

  ureq-proto stays at 0.6.5 (2026-10-17), because ureq 3.5.0 requires ^0.6.5.

### quire-api and quire-core

- **D13. DTO conventions:**
  - Times are `Timestamp(i64 ms)`: an RFC 3339 UTC string with milliseconds on the wire, `string` in TypeScript,
    `date-time` in OpenAPI.
  - Ids are `Uuid`, except path refs and url_ids, which are strings.
  - Response fields without a value are sent as `null`; optional request fields are omitted.
  - PATCH fields where null and omitted differ are tri-state through `common::double_option`: `CollectionPatch.icon`,
    `WorkspacePatch.logo_attachment_id` and `NewToken.expires_in_days`.
  - Defaulted boolean flags are omitted when false.
  - Unit enums are snake_case strings with `ALL`, `as_str`, `Display` and `FromStr`.
- **D14. Where DTOs live** (the §4.1 file list is fixed):
  - `auth.rs` also holds Config, InkboardInfo, WasmBytes, Limits, Workspace, WorkspacePatch, Health and UserEnvelope.
  - `files.rs` holds BackupResult; `tree.rs` holds sidebar, lists, trash and search.
  - `docs.rs` holds MissingDepsDetails and DocSchema; `error.rs` holds the other `*Details` types.
- **D15. Shapes the docs leave open:**
  - **Lists and trash:** `ListedItem` (ItemSummary flattened, plus `path` and the optional extras), `RestoredItem`
    (ItemSummary plus `moved_to_top`), and `TrashedCollection` and `TrashedItem`.
  - **Defaults and nullable fields:** `SearchQuery.include_drafts` is `Option<bool>`, and the route applies the
    default true. `BoardMeta.preview_rev` and `BoardPageMeta.svg_len` are nullable. `ErrorBody.request_id` is
    `Option<String>`, null only for errors the CLI raises itself.
  - **Enums:** `ConversionWarningCode` has the 16 codes of §6.4.
  - **DocSchema:** `{schema_version, nodes, marks, AttrSpec{name, type, default}}` is a proposal; W4 may refine it.
- **D16. `ErrorCode` is a strict enum.** A client that must tolerate future codes parses `code` as a string, so
  `ExitCode::from_status(status: u16, code: Option<&str>)` takes a string.
  - From the code alone: `cli_usage` → 2; `cli_config` and `host_not_allowed` → 3; `network`, `timeout` and `tls` → 4.
  - The exit-code names are snake_case forms of the Meaning column. `ExitCodeInfo` is `{code, name, meaning}`.
  - `cli.rs` has no ts-rs export, because it is not an HTTP DTO.
- **D17. `token::encode(random: &[u8; 32])`** reads the bytes as one big-endian number and writes 40 base62
  characters (bias about 2⁻¹⁷, so no rejection loop). `check` reports Prefix, Length, Alphabet or Checksum.
  - Added: `display_prefix` (12 characters) and the format constants.
  - Added: the Scope helpers (`covers`, `expand`, `expand_all`, `highest`, `granted`) and `NewToken::expiry_days()`
    (omitted → 90, null → never).
  - Added: DEFAULT_EXPIRY_DAYS, MAX_EXPIRY_DAYS and MAX_LIVE_TOKENS.
- **D18. Cursor format.** base64url without padding over version ‖ kind ‖ 8-byte query hash ‖ payload ‖ 4-byte sha256
  check.
  - Types: `Cursor::{Keyset{sort: SortValue::{Int, Text}, id}, Offset(u32)}`.
  - Errors: `CursorError::{Malformed, QueryMismatch}`, with `FIELD = "cursor"`. Input over 8192 characters is refused
    without decoding.
  - `query_hash(&[(name, value)])` sorts and length-prefixes the pairs. Callers include the list's name and leave out
    `cursor` and `limit`.
  - **Added in review:** `impl From<CursorError> for quire_store::Error` (422 `validation`, field `cursor`), so the
    contract comment "CursorError → 422 validation" holds when a store function uses `?` on `cursor::decode`.

### quire-store (frozen signatures that differ, then choices)

- **D19. `Db::write` and `Db::read` are `async fn`.** This is a change to a frozen signature. The PLAN listing shows
  `pub fn write<T>(&self, f) -> Result<T>`. deadpool-sqlite (D2) is async, so the pools can only be used this way.
  The closure itself is synchronous. The exact signatures:

  ```rust
  impl Db {
      pub async fn write<T, F>(&self, f: F) -> Result<T>
        where F: FnOnce(&rusqlite::Transaction<'_>) -> Result<T> + Send + 'static, T: Send + 'static;
      pub async fn read<T, F>(&self, f: F) -> Result<T>
        where F: FnOnce(&rusqlite::Connection) -> Result<T> + Send + 'static, T: Send + 'static;
      pub fn maintenance(&self) -> Result<rusqlite::Connection>;
  }
  ```

  - `write` runs in `BEGIN IMMEDIATE` on the single writer. `read` runs in one deferred transaction (one snapshot) on
    a `query_only` reader.
  - A panic inside the closure is resumed on the caller.
  - Additions: `Db::open(path, DbOptions{synchronous_full, readers})`, `Db::path()`, and
    `Db::with_writer(f(&mut Connection))`, for D26's GC, which commits and then unlinks while it still holds the
    writer.
- **D20. `TokenRow` has an extra field, `role: Role`** (additive, R3), so `Auth<S>` builds `Actor` without a second
  query. Struct literals need it (`routes/tokens.rs` builds one). Added: `tokens::hash(token) -> [u8; 32]`.
  **Added in review:** `tokens::get(conn, user_id, token_id) -> Result<Option<ApiToken>>` (a token that is not
  revoked, live or expired). GET /auth/me now uses it (see "Review findings").
- **D21. `BlobStore::new(root)` takes the data directory.** Blobs go to `root/files/aa/bb/<sha>` and staging files to
  `root/tmp`.
  - Added: `tmp_dir`, `files_dir`, `root`, and the `Staged` accessors. `Drop for Staged` deletes an uncommitted file.
  - Added: `Sha256::{parse, of, from_digest, as_str}`.
  - `blobs::sniff` is a minimal sniffer: images, SVG, PDF, zip, gzip and UTF-8 text, otherwise octet-stream; sizes
    from imagesize. W2 extends it.
- **D22. `quire_store::Error`** is a struct with public `code`, `message` and `details` fields, plus a private source
  kept for logs.
  - A trigger's `RAISE(ABORT, '<code>')` maps to that ErrorCode, so `parent_invalid` arrives as 409.
  - `QueryReturnedNoRows` maps to `not_found`; everything else is `internal`.
- **D23. Store behaviour the docs leave open:**
  - **tokens:**
    - **create:** trims the name, then checks in this order: another user's parent → 403 `forbidden`; admin scope →
      403 `session_required`; a scope above the parent's → 422 `scopes`; a later expiry (never counts as later) →
      422 `expires_in_days`.
    - **The 101st live token:** 422 `validation`, field `"tokens"` (request W7-6).
    - **revoke:** transitive through one recursive CTE.
    - **lookup:** runs `token::check` first, then rejects a revoked token or a disabled user before it looks at
      expiry.
    - **list:** non-revoked tokens (live and expired), newest first, LIMIT 100.
  - **revisions:**
    - `maybe_auto` compares with the newest revision of any reason.
    - `pre_api` returns false for session, seed and admin writes. Its author is `items.updated_by`, and it does not
      set `last_token_id` (the write that follows does).
    - `RevisionDraft` has `yjs(..)` and `inkboard(..)` constructors.
  - **items:**
    - `create`: a missing or deleted collection gives 409 `parent_invalid`; a title over 1000 characters gives 422
      `title`; a position naming a non-sibling gives 422 `position`.
    - `rename` with an unchanged title changes nothing. A trashed or archived item gives 409 `read_only`.
    - `purge_expired` deletes one depth level per DELETE, deepest first, then deleted collections.
  - **collections:** `create` trims the name to 1–100 characters. `icon` is `[a-z0-9-]{1,64}`. `color` is `#RRGGBB`,
    default `#8D8D8D`.
  - **sort keys and ids:** crate-private ports in `shared`, because `quire_core::rank` and `ids` are still stubs:
    CC0 fractional-indexing `generateKeyBetween`, checked against the upstream vectors; `new_id` (UUIDv7); and
    `new_url_id` (10 base62 characters).
  - **docs:**
    - The `init_*` and `copy` functions replace earlier state.
    - `init_with` validates the update: 422 `invalid_update`.
    - The body text comes from a minimal Y walk, until W4's ytext.
  - **boards:**
    - The body is raw DEFLATE, and `body_sha256` is a 32-byte BLOB.
    - The text is the page names only ("Page 1" for a new board).
    - `init_with` refuses inline asset data (422 `invalid_board`) until W6's split.
    - `is_empty` means `meta.objects == 0`.
  - **Free space:** `check_free_space` reads MiB and compares f_bavail × f_frsize with min(min_free_mb MiB, 10 % of
    the disk).
  - **migrate:**
    - `migrate::run(db, data_dir, now) -> Migrated{from, to, backup}` runs on the maintenance connection.
    - It skips the backup for a new database, and refuses a database newer than the build.
    - The foreign-key check runs after `to_latest`.
  - **backup:** the pre-migration name gets `-2`, `-3`, … when it is taken.
  - **Fixture:** the variable is `WRITE_FIXTURES=1` (the repo convention), not UPDATE_FIXTURES. The comparison
    ignores the random page id.
  - **Test placement:** some tests already carry the names W1 and W6 will need (for example
    `tree::purge_depth_2000_leaves_first`, `items::pre_api_rule`, `boards::copy_shares_blobs`). Those owners may
    rewrite them.

### quire-server

- **D24. Plain SQL until the W2 store modules exist.** These read or write users, settings and sessions directly:
  `load_user`, `setting` and `load_workspace` in `routes/auth.rs`; the owner insert in `admin.rs`; the session lookup
  in `extract.rs`; and the parent `expires_at` read in `routes/tokens.rs`.
- **D25. Configuration** (§8 and §13.3):
  - **Defaults:** `QUIRE_LISTEN` 127.0.0.1:8080 (the image sets 0.0.0.0:8080) and `QUIRE_DATA_DIR` `./data`.
  - **Public URL fallback:** `http://localhost:<QUIRE_LISTEN port>`. §8 says `http://localhost:8080`; that is the same
    URL for a bare serve, and correct when the port is changed. This deviates from the text (request W7-7).
  - **Parsing:**
    - Every empty variable counts as unset.
    - Public URL and CORS origins are `scheme://host[:port]`, with no path.
    - `QUIRE_DOMAIN` has no port.
    - An allowed-hosts entry is `host` or `host:port`.
    - `QUIRE_BACKUP_INTERVAL` is `<n>s|m|h|d`.
    - Limits must be at least 1; `QUIRE_MIN_FREE_MB` and `QUIRE_BACKUP_KEEP` may be 0.
    - A malformed value stops serve with "VAR: message" and exit 3.
- **D26. `healthcheck`.**
  - **Default URL:** `QUIRE_LISTEN`'s port on 127.0.0.1 when the listen address is unspecified (`0.0.0.0` in the
    image, or `[::]`). A specific address (`127.0.0.1`, `[::1]`, a LAN IP) is probed as it is, because a server bound
    to one address does not answer on another. PLAN says "on 127.0.0.1"; this agrees for the image and for a bare
    serve. The §13.2 Dockerfile comment ("0.0.0.0 → 127.0.0.1") already describes this rule.
  - **Changed in review:** before, a loopback listen address such as `[::1]` was also mapped to 127.0.0.1, which
    would fail.
  - **`--url`:** no path or `/` gets `/healthz` appended; only http://.
  - **Probe:** 3 s timeout, exit 0 only for 200; plain-TCP HTTP, since the server links no HTTP client.
- **D27. OpenAPI.**
  - utoipa writes `security(())` as `[{}]`, and `openapi::spec()` rewrites it to `[]`, as §7.4 spells it (request
    W7-8).
  - Each handler has its own `routes!()`: utoipa-axum's `routes!` with handlers on several paths gives every path all
    their methods.
  - The ETag is `"o<16 hex digits of sha256>"`. If-None-Match ignores `W/` and accepts `*`.
- **D28. Error envelope.**
  - Non-envelope 4xx and 5xx under `/api/` are converted: 400 and 422 → 422 `validation`, with `details.field`
    body, query or path; and 401, 403, 404, 405, 408, 409, 413 (`limit_bytes` 1048576), 415, 428 and 503 → their
    codes.
  - Any other 4xx becomes 422 and any other 5xx 500.
  - Request ids are always generated by the server (UUIDv7, 32 hex digits); a client's X-Request-Id is ignored.
  - `X-Request-Id` is on every response; `Quire-Api-Version` only on `/api/v1` paths.
- **D29. Host allowlist and CSRF are already enforced** (`csrf.rs`).
  - CSRF covers every non-bearer POST, PUT, PATCH and DELETE, unauthenticated ones included.
  - Every Sec-Fetch-Site value must be `same-origin` or `none`.
  - `/healthz` is exempt.
  - `ClientInfo` uses the rightmost X-Forwarded-For entry from a trusted peer. Without ConnectInfo (axum-test's mock
    transport), a request counts as untrusted plain HTTP.
- **D30. Other server choices:**
  - **Rate limiters:** wrapper types over governor's DefaultClock, built from the configuration but not enforced
    (W2).
  - **argon2:** runs under a process-wide `Semaphore(2)` in `state.rs`, so the frozen AppState field list stays
    exact. The cheap parameters are m=MIN, t=1, p=1.
  - **`Auth<S>`:** the marker trait is `Require` and the Actor is cached in request extensions. Helpers on `Actor`:
    `has_scope`, `token_id`, `session_id`, `scopes`, `viewer` and `write_by`. `Viewer` (a re-export of
    `quire_store::shared::Viewer`) authenticates without a scope check.
  - **Path extractors:** `ItemRef` accepts a UUID or exactly 10 ASCII alphanumerics, anything else is 404. Added
    `IdPath` (UUID-only `{id}`).
  - **Sessions:** the cookie value is base64url of 32 bytes, and `token_hash` is sha256 of the decoded bytes.
  - **POST /tokens:** the route checks name, scopes and expiry. A token caller cannot get `admin` (403
    `session_required`) and is capped by its own scope and expiry (422). A session needs `password` for an admin or
    never-expiring token. `created_via` is `web` for a session, `api` for a token.
  - **GET /auth/me:** `logo_url` is `/api/v1/attachments/<logo_attachment_id>`.
- **D31. Admin commands.**
  - **`create-owner --json`** prints `{user, workspace, generated_password}`.
    - The generated password has 24 base62 characters and sets `must_change_password`.
    - `--password-stdin` strips one trailing newline and takes 12–1024 bytes, otherwise exit 10.
    - It inserts `workspace.name`, `instance.id` and `instance.created_at`, each only if missing.
    - **No "Getting started" seed yet, and `--no-seed` has no effect** (W2).
  - **`create-token`:**
    - `--expires` takes `<n>d` (1–3650) or `never`; the default is 90d.
    - An unknown `--email` exits 7; no owner exits 8.
    - Without `--json`, stdout has only the token.
    - `--token-out F` writes the token and a newline with mode 0600; `--json` then prints `{token_file, api_token}`.
  - **Output:** `--json` is a global flag of `admin`. Errors are one stderr line
    `{"error":{…},"request_id":null,"status","exit_code"}`.
  - Commands open the database through `AppState::open`, on a current-thread runtime.
- **D32. `serve`.**
  - **Logging:** tracing fmt filtered by RUST_LOG (default info).
  - **Runtime:** `QUIRE_WORKER_THREADS` and `QUIRE_MAX_BLOCKING_THREADS` size it.
  - **Shutdown:** SIGTERM or SIGINT drains, then runs `wal_checkpoint(TRUNCATE)`.
  - **Setup:** no setup code yet, because POST /setup is W2's; it logs a hint to run create-owner.
  - **Version:** `--version` prints `quire-server 0.1.0 (<QUIRE_COMMIT or unknown>)`.
  - **Layer order, outermost first:** SetSensitiveHeaders → Trace → envelope → CatchPanic → Timeout 30 s → 408 →
    body limit 1 MiB → Host/CSRF → routes.
- **D33. Minimal pieces W2 completes:**
  - **`static_files`:** exact-path embeds, `/inkboard/<path>`, and index.html with no-cache for paths without an
    extension. Otherwise, and for any `..` segment, a plain-text 404. No precompressed variants, ETags or CSP yet.
  - **`quire-assets`:** `web(path)`, `inkboard(path)` and the `*_paths()` functions return `Asset{data, mime,
    sha256}`.
  - **`board_previews`:** `PreviewQueue::noop()`; `enqueue(&self, item_id: &str)` does nothing.
  - **`authz::require`:** the owner is always allowed, any other role gets 403.
- **D34. Test harness additions.**
  - **TestServer:** `start_with(configure)`, `token(scope)`, and the owner and workspace constants. Its tokens are
    `created_via` admin, with 90-day expiry.
  - **TestApp:** `with_config`, `client`, `cookie_client`, `token_client`, `insert_owner`, `insert_session`,
    `mint_token`, `exec` and `assert_error`.
  - **Test names:** the tests sit in W2's modules under W2's planned names where they match (for example
    `security::quire_bearer_wins_over_cookie`). `auth.rs` stays a stub.

### quire-cli

- **D35. CLI skeleton choices.**
  - **`--version`:** prints `quire 0.1.0` without a commit. §7.6's `version` JSON has no CLI commit, and the cli
    stages do not pass QUIRE_COMMIT.
  - **Global flags:**
    - They have no clap `env =`; W8's `config.rs` reads the environment, so the `--profile` and URL/token-pair rules
      of §7.6 can hold.
    - `--timeout` is an `Option<u64>` of at least 1, and `--retries` is a u32 defaulting to 2.
    - `--token-stdin` conflicts with `--token-file` (exit 2). There is no `--token` flag.
  - **The §7.7 usage-error JSON line exists now.** It is triggered by an exact `--json` argv entry and carries
    `"status": null` (request W7-9).
  - **Output:**
    - `exit-codes --json` is pretty-printed.
    - A bare `quire` prints help to stderr and exits 2.
    - Colour is never printed.
    - clap's own `help` subcommand stays until W8's `help [--json]` replaces it.
    - A failed stdout write exits 1.
  - **`GlobalArgs`:** carries `#[cfg_attr(not(test), expect(dead_code))]`. W8 must remove it once every field is
    read, or clippy fails.
  - **Tests:**
    - `tests/cli` also has a `version` module.
    - QUIRE_CONFIG is `<tmp>/config/config.toml`, outside HOME.
    - The command drops every inherited `QUIRE_*`, XDG_CONFIG_HOME, SSL_CERT_*, and RUST_LOG.

### Web and e2e

- **D36. Web package.**
  - **Versions picked where §4.2 gives none, each at least two weeks old:**
    - react-router 8.4.0, @tanstack/react-query 5.104.2, @tanstack/react-virtual 3.14.19, zustand 5.0.15;
    - lucide-react 1.56.0, @atlaskit/pragmatic-drag-and-drop 4.0.0 (-hitbox 3.0.0, -auto-scroll 3.2.1);
    - lowlight 3.3.0, idb-keyval 6.3.0, @vitejs/plugin-react 6.1.2;
    - happy-dom 21.0.1, @testing-library/react 16.3.3, user-event 14.6.8, fake-indexeddb 6.2.5.
  - **TypeScript:** 7.0.2, so no 6.x fallback.
  - **Additions beyond §4.2:** @testing-library/dom 10.4.2 (a required peer), @types/react and @types/react-dom
    19.3.0, and `packageManager: pnpm@11.24.0`.
  - **Peers:** required peers come from autoInstallPeers and are pinned in the lockfile. There is one copy each of
    prosemirror, @tiptap/core and yjs.
  - **Lockfile:**
    - It was resolved with `minimumReleaseAge` 20160 minutes, excluding only the young pinned versions, and that
      resolution reproduces byte for byte.
    - No `pnpm-workspace.yaml` is committed: pnpm 11 checks the lockfile against its release-age policy, and a
      setting that the §13.2 `pnpm fetch` cannot see makes the offline install fail with ERR_PNPM_NO_OFFLINE_META.
- **D37. Web choices.**
  - **Config:** the Vitest config is the `test` block of `vite.config.ts` (happy-dom, setup file, maxWorkers 4).
    `build` is `vite build` only. `fixtures` exits 1 until W4 adds `src/editor/fixtures`.
  - **tsconfig:** the frame project uses `allowJs`, `checkJs` and `allowImportingTsExtensions`, with skipLibCheck
    false.
  - **`bridge.ts`:** §10.4 verbatim, plus the aliases FrameError, FrameErrorKind, FrameHostKey and ConnectError, and a
    `Window.quireBoardFrame` declaration.
  - **`client.ts`:**
    - The `api<T>(method, path, opts)` options are `json`, `body`, `headers`, `etag`, `signal`, `keepalive` and
      `authLost`. A 304 resolves to undefined. `apiResponse<T>` also returns `{status, data, etag, notModified,
      headers}`.
    - The client adds two codes of its own: `network` (status 0) and `bad_response`.
    - The `auth-lost` window event is not sent for `invalid_credentials` or with `authLost: false`.
    - Any Authorization header is dropped.
  - **`saveStatus`:** `SaveState` is the union, and `SaveStatus = {state, message?, retry?}`. `setSaveStatus` also
    takes a bare state.
  - **`pending`:** `registerPending` returns the unregister function, and one item may have several entries.
    `flushPending` never rejects.
  - **Hotkeys:**
    - `runHostShortcut(e: HostKey | KeyboardEvent): boolean` handles Mod+K, Mod+., Escape and Mod+S.
    - A Mod+Shift chord maps to nothing (D23).
    - W3 registers handlers with `onHostShortcut`.
  - **`urls.ts`:** slugs are NFKD with accents stripped, lowercase, any other character as `-`, at most 60
    characters, `untitled` when empty.
  - **Queries:** `useItem` uses the key `['item', ref]`. `useConfig` lives in `queries/config.ts` and is re-exported
    from `items.ts`.
  - **Item chrome:**
    - TopBar does not render ItemMenu; pages pass it through `actions`.
    - `ItemMenu`'s `ExtraExport` is `{label, run}`. There is no built-in Markdown export; W4 supplies it.
    - `ItemHeader` exports `TITLE_INPUT_ID`, and its title autosave waits 500 ms and registers in `pending`.
  - **BoardFrameContext:** `{frame, failure, attach(surface)}`. BoardFrameHost is mounted beside the routes, so W6's
    `useBoardFrame` needs an external store.
  - **W3 files filled in minimally:** App, queryClient, AuthGate, `lib/time`, useCanonicalPath, SaveStatus,
    StarToggle, useTitleAutosave, PlaceholderPage, tokens.css, global.css and favicon.svg.
  - **Theme:** `theme-init.js` reads `localStorage['quire.theme']`.
  - **Router:** an unknown settings section renders NotFound; there is no bare `/settings` route.
- **D38. e2e package.**
  - **package.json:** `quire-e2e`, private, `type: module`, with no scripts and no `packageManager`. There is no
    typescript or @types/node, because §4.3 allows only @playwright/test.
  - **webServer:** runs with `cwd: '..'`, checks readiness by `port: 4173`, and also starts for smoke runs.
    `serve-static.mjs` treats a missing root as a warning and answers 404.
  - **serve-static.mjs:** binds 127.0.0.1, GET and HEAD only, sends `no-store` and `nosniff`, has no compression
    negotiation and no CSP, and serves `.inkboard` as JSON.
  - **Specs:** the placeholders make small real checks instead of skipping.

### Licences, fixtures and docs

- **D39.** `LICENSE-MIT` says "Copyright (c) 2026 The Quire contributors"; the owner may want his own name there.
  `LICENSE-APACHE` is the canonical text. The README stub has the usual dual-licence contribution clause. `.gitignore`
  keeps the existing unanchored `dist/`, so the added `/dist/` is redundant but harmless.
- **D40. CLAUDE.md.** The toolchain bullet names the `toolchain` target and the `toolchain-base` / `toolchain` stages.
  The agent-harness line refers to `docs/AGENTS.md`, which W8 writes.

## Unconfirmed pins (all recorded in ARCHITECTURE §16 item 24)

| Pin | Fallback or note |
|---|---|
| `rust:1.99.1-trixie@sha256:5376600…e733` | rustc 1.99.1 dates from 2026-10-13. The index digest is Docker's rebuild of 2026-10-28. |
| `node:24-trixie-slim@sha256:89c5244…b95a` | Rebuilt 2026-10-28, with node 24.22.1 (about 2026-10-20). It was kept because the toolchain already had it. The newest digest that is two weeks old carries 24.21.0. |
| `gcr.io/distroless/cc-debian13:nonroot@sha256:3ee0029…515e` | Uploaded 2026-10-23. Kept, because the tag is the doc's. |
| `mcr.microsoft.com/playwright:v1.64.1-noble@sha256:47b510d…b8ea` (`scripts/e2e.sh`) | Created 2026-10-23. Kept, because @playwright/test 1.64.1 must match it. |
| binaryen `version_131`, sha256 `b5bf1f0e…46db` | Released 2026-07-15. Confirmed. |
| rust-embed 8.13 `interpolate-folder-path` | Dropped (option-ext is MPL-2.0); relative folders instead (D10). |
| ts-rs 12 | It defaults to `bigint`, so `TS_RS_LARGE_INT = "number"` is set (D9). |
| tokio 1.54.1, uuid 1.29.0, ureq 3.5.0 (with ureq-proto 0.6.5), comrak 0.59.0 | Under two weeks old. Used as pinned; all build with the documented features. |
| react and react-dom 19.3.1, vite 8.3.5, vitest 5.0.5 (with @vitest/mocker and @vitest/spy) | Under two weeks old. Used as pinned. |
| crc32fast 1.5, rustix (unpinned minor) | 1.5.2 and 1.1.5 were taken, because 1.5.3 and 1.1.6 are under two weeks old. |
| deadpool-sqlite 0.14 / rusqlite_migration 2.6 with rusqlite 0.40.2 | They fit, so there is one libsqlite3-sys and no D2 fallback. |
| utoipa 6.0.0, utoipa-axum 0.3.0; tower-http 0.7.1; TypeScript 7.0.2; ring with musl-gcc | They work, so no fallback is needed (risks 12, 14, 17). |

The docs do not say whether the two-week rule also covers image digests. If it does, re-pin the four digests to older
builds of the same tags; for node, that means 24.21.0.

## Open for the owner

1. **lightningcss (MPL-2.0).** vite 8.3.5 hard-depends on lightningcss 1.33.0 and its platform packages, so
   `pnpm install --frozen-lockfile` always installs them. MPL-2.0 is not on the R6 allow-list. They run only at build
   time, and none of their code ships in `web/dist`. The choices:
   - **(a) Accept MPL-2.0 for build-only tools.** Write the exception into PLAN R6 and §4.2, and add a THIRD_PARTY.md
     note.
   - **(b) Re-pin to Vite 7 with @vitejs/plugin-react 5,** regenerate `web/pnpm-lock.yaml`, and re-run the web checks.
     This changes the §4.2 pin; W3 owns `web/package.json`.

   Every acceptance check passes either way. W0b did not choose, because each option changes normative text.
2. **The age of the digests** (above).
3. **The build slice** (top of this note).

## Review findings (two reviewers, 2026-10-28)

The finalize step checked every finding against the code and the docs. None was wrong; the fixes taken are below.
Where a reviewer offered two fixes, the one not taken is listed with its reason.

| # | Finding | Outcome |
|---|---|---|
| C1 / S3 | The compose overlays add `mode: 0o1777` to the `/data` tmpfs, and nothing records it | **Holds.** Recorded as D1, with request W7-1 to fix both §13.3 blocks. The files stay as they are. It is not recorded in §16, because finalize may add only pins there (task 9). |
| C2 | `Db::write` / `Db::read` are `async` against the frozen PLAN listing, unrecorded | **Holds.** Recorded as D19, with the exact signatures, and request W7-4 to update PLAN "Produces". The code stays async, because deadpool-sqlite allows no other shape. |
| C3 | `TokenRow.role` is an extra public field, unrecorded | **Holds.** Recorded as D20 (an additive change under R3). |
| C4 | The public-URL fallback uses the listen port; healthcheck probes a non-loopback listen IP | **Holds, recorded** (D25, D26). The option to follow the text was not taken for either: a fixed `http://localhost:8080` names the wrong URL when `QUIRE_LISTEN` changes the port, and a server bound to one address does not answer a probe on 127.0.0.1. Both choices agree with the text for a bare serve and for the image. While checking, I found that `[::1]` (and any loopback address other than 127.0.0.1) was mapped to 127.0.0.1 and would fail; fixed in `config.rs`, with tests. |
| C5 | `TS_RS_LARGE_INT = "number"` changes the generated contract, unrecorded | **Holds.** Added to §16 item 24 as the fallback for the ts-rs 12 pin, and recorded as D9. Request W7-3 updates the §3 layout line, which finalize may not edit. |
| C6 | Nothing maps `CursorError` to 422 `validation` | **Holds, fixed.** Added `impl From<quire_core::cursor::CursorError> for quire_store::Error` (422, `details.field = "cursor"`) in `crates/quire-store/src/error.rs`, with the test `error::tests::cursor_errors_are_422_on_the_cursor_field`. The reviewer's other option, a request to W1, was not needed: W0b still owns the file. `ApiError` gets the mapping through `From<quire_store::Error>`. |
| S1 | lightningcss is MPL-2.0 | **Holds, unresolved.** It is the owner's decision, because both fixes change normative text (above). |
| S2 | `/auth/me` searched the capped `tokens::list`, so 100 newer expired tokens made `auth.token` null | **Holds, fixed.** Added `quire_store::tokens::get(conn, user_id, token_id)`, and `get_me` now uses it. Tests: `tokens::get_finds_one_unrevoked_token_of_the_user` (store) and `tokens::me_describes_the_token_behind_100_newer_ones` (API, with 100 newer expired rows). |

Earlier in W0b, the acceptance step set aside these agent requests as edits to normative text. They are listed here
for W7 (below):
- the docker agent's PLAN line 480 rewording;
- the rust, store, server and cli agents' §4.1, §3, §5/§7.2, §7.4, §7.7, §7.8 and §13.3 notes;
- the docker agent's optional update of the §13.2 listing.

The cli agent's optional `ARG QUIRE_COMMIT` for the cli stages was also not taken: the Dockerfile stays the §13.2
text, and §7.6's `version` JSON has no CLI commit.

## Requests for the W1–W8 owners

**W1** (`quire-store` shared/items/collections/error, `quire-core`):
1. Once `quire_core::rank::key_between` and `quire_core::ids::{new_uuid, new_url_id}` exist, replace the crate-private
   `shared::rank`, `shared::new_id`, `shared::new_url_id` and `shared::place` with them.
2. `quire_store::Error: From<CursorError>` exists, so list functions can `?` on `cursor::decode` (D18).

**W2** (`quire-server`, `quire-assets`, store users/sessions/settings/tokens/seed):
1. Add `users::get`, `users::create_owner` (with the `NOT EXISTS owner` guard), `settings::{get, set}` and
   `sessions::lookup` to the store. Then replace the plain-SQL copies listed in D24.
2. Wire `seed::getting_started` into `admin::create_owner`, and make `--no-seed` work (D31).
3. `GET /api/v1/config`:
   - Answer `inkboard: null` when the manifest's `engine_version` is `"stub"`. The stub lists only `frame.html` and
     `frame.js`, with no wasm entry, so boards fall back to BoardPreview (§9.5).
   - Add the §10.5 startup check: refuse to start on a `format_version` mismatch.
   - The manifest fields are defined in `patches/inkboard/README.md`.
4. Enforce the rate limiters (D30); `ratelimit.rs` may change its clock type.
5. Add the 30-day purge of expired tokens. Until it exists, `GET /tokens` (the 100 newest non-revoked tokens) can hide
   older live tokens behind 100 newer expired ones.
6. Complete `static_files`: precompressed variants, ETags and CSP (§11); `blobs::sniff` (D21); and `authz` beyond the
   owner (D33).
7. Run `scripts/dev.sh env UPDATE_OPENAPI=1 cargo test -p quire-server --locked --test api -- openapi::` with every
   route change (R4).

**W3** (`web/package.json` and the shell):
1. To keep the two-week rule for new npm packages, resolve them with a temporary `web/pnpm-workspace.yaml` (minimum
   release age and exclusions as below), then delete the file. Never commit it (D36).
   - Content: `minimumReleaseAge: 20160` and the exclusions react@19.3.1, react-dom@19.3.1, vite@8.3.5, vitest@5.0.5,
     @vitest/mocker@5.0.5 and @vitest/spy@5.0.5.
   - Making the rule permanent would need the §13.2 web stage to copy that file before `pnpm fetch`.
2. Act on the lightningcss decision once the owner makes it.
3. W0b filled in some W3 files minimally (D37) and did not create `mockApi.ts`.

**W4:**
1. Replace the minimal Y walk in `quire-store/src/docs.rs` with `quire_core::ytext`, so that `init_with` also sets
   `item_links` and `item_attachments` and uses `\t` between table cells.
2. The `DocSchema` DTO in `quire-api/src/docs.rs` is a proposal; refine it before anything depends on it.
3. Markdown export reaches ItemMenu through `extraExports`.
4. `pnpm --dir web fixtures` needs `src/editor/fixtures`.

**W5:**
1. `scripts/inkboard.sh` `src`/`build`/`export`/`check`/`size`. `write_manifest` already computes `build` (D8); fill in
   the sizes table in `patches/inkboard/README.md`.
2. `e2e/serve-static.mjs` is yours from now on. It sends no CSP and does no compression negotiation (D38).

**W6:**
1. In `quire-store/src/boards.rs`:
   - Replace the inline-asset refusal in `init_with` with `quire_core::board::split` and BlobStore staging.
   - Replace the page-names-only text and meta with `board::inspect`.
   - Keep raw DEFLATE for `board_state.body`, or change docs, boards and revisions together.
2. `useBoardFrame` needs an external store, and `BoardFrameContext` is only a proposal (D37).

**W7** (docs and integration). Normative doc fixes that W0b could not make:
1. §13.3: add `mode: 0o1777` to the quire `/data` tmpfs in both the `compose.e2e.yaml` and `compose.smoke.yaml` blocks
   (D1).
2. PLAN line 480: `bash -n scripts/*.sh scripts/lib/*.sh` checks only the first file. Use
   `for f in scripts/*.sh scripts/lib/*.sh; do bash -n "$f" || exit 1; done`.
3. §3 `.cargo/config.toml` line: add `TS_RS_LARGE_INT = "number"` (D9).
4. PLAN W0b "Produces":
   - the async `Db::write`/`Db::read` signatures of D19;
   - `BlobStore::new(<data dir>)`, with files under `<data dir>/files` and staging under `<data dir>/tmp`;
   - the additive `TokenRow.role` and `tokens::get`.
5. §4.1:
   - drop rust-embed's `interpolate-folder-path`;
   - add bytes 1.12.1 and futures-util 0.3.34 to the workspace dependencies;
   - add rusqlite to quire-server's row of the table (D11, D24).
6. §5 or §7.2: name `details.field = "tokens"` for the 101st live token.
7. §8, §7.8 and §13.3:
   - the bare-serve defaults (QUIRE_LISTEN 127.0.0.1:8080, QUIRE_DATA_DIR ./data, the public URL from the listen
     port);
   - the healthcheck address rule (D25, D26);
   - the admin JSON shapes (D31). docs/AGENTS.md (W8) should repeat them.
8. §7.4: `security(())` is written as `[{}]` and rewritten to `[]` (D27).
9. §7.7: a CLI-raised error line carries `"status": null`, and the `--json` usage rule matches the exact argv entry
   (D35).
10. §13.2, optional: show the filled-in digests and checksum in the listing.
11. From W0a, still open: §13.1 and PLAN W0a task 5 name `moby/buildkit:v0.34.1`, but the pin is v0.34.0 (§16 risk 23).
12. THIRD_PARTY.md: complete it (every crate in Cargo.lock, the npm packages bundled into `web/dist`, borrowed code).

**W8** (`quire-cli`):
1. Remove `GlobalArgs`'s `expect(dead_code)` once every field is read, and replace clap's `help` subcommand with
   `help [--json]`.
2. `config.rs` reads the `QUIRE_*` environment; the clap flags have no `env =` (D35).
3. If the owner wants the commit in `quire --version`, the cli stages need `ARG QUIRE_COMMIT`. That is a §13.2 change,
   for W7.
4. Write docs/AGENTS.md, which CLAUDE.md already points to.

## Follow-ups

- Commit `web/src/api/gen/` and `docs/api/openapi.json` with W0b. Until then the PLAN `git diff --exit-code` lines
  cannot catch drift, and the checks above use sha256 comparisons instead.
- The full image smoke (`QUIRE_SMOKE_LEVEL=full`) needs a real Inkboard build (W5). With the stub image it fails by
  design.
- Host port 443 is still taken by the owner's own Caddy (W0a "Environment findings"). The smoke uses 127.0.0.1:18443
  and is not affected.
- The owner may want his name in `LICENSE-MIT` (D39).

## Files for the W0b commit

- **New:** `.dockerignore`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `LICENSE`,
  `THIRD_PARTY.md`, `README.md`, `compose.e2e.yaml`, `compose.smoke.yaml`, `docker/Dockerfile`, `crates/**`,
  `docs/api/openapi.json`, `docs/handoff/W0b.md`, `e2e/**` (not `node_modules`, `test-results` or
  `playwright-report`), `fixtures/**`, `patches/inkboard/README.md`, `scripts/{check-pins,e2e,fmt,image,image-smoke,inkboard}.sh`,
  and `web/**` (not `node_modules` or `dist-inkboard`; `web/src/api/gen/**` included).
- **Changed:** `.cargo/config.toml`, `.gitignore`, `CLAUDE.md`, `scripts/toolchain.sh`, and `docs/ARCHITECTURE.md`
  (§16 item 24 only).
- **Deleted:** `docker/toolchain.Dockerfile`.
- `vendor/inkboard` is unchanged and clean.
