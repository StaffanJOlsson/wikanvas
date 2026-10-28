# Quire architecture (milestone 1)

Status: binding for M1 implementation. Paths are relative to the repo root. "Inkboard" is `vendor/inkboard`, a submodule
pinned at `v0.2.0-beta.3` (`41e0295`). This document is self-contained: the design-stage UX spec and the reader reports
it was distilled from are not in the repo, and nothing depends on them. The normative UI spec is §9.6. §2.2 records the
alternatives that were rejected. §17 logs how review findings were resolved where a finding was not simply fixed, and
how the owner requirements of 2026-10-27 (public API, CLI, agent testing) were decided (D39–D56, §17.1), how the
review of those amendments was resolved (§17.2), and the owner decisions of 2026-10-28 (published on create, 2 CPUs /
1 GiB, the build slice, HTTPS through Caddy; D23, D25, D38, D57, §17.3). §17.4 records the decisions taken when the
workstreams' deviations were folded into this document (W7, 2026-10-28).

Hard rules (from CLAUDE.md, repeated because they constrain the design):
- No compiler, bundler or package manager runs on the host. Everything goes through `scripts/dev.sh` (quire-toolchain
  image, capped at 6 CPU / 12 GB / no swap, pinned to 6 cores, nice 10). Images are built only with
  `docker buildx build --builder quire-limited`. Git runs on the host; it is not a build tool.
- Caps are kernel-enforced (§13.1, D38): per container, and in total by the systemd slice `quire-build.slice`. One
  capped build runs at a time: `scripts/dev.sh`, `scripts/image.sh`, `scripts/image-smoke.sh`, `scripts/e2e.sh`,
  `scripts/agent-test.sh`, `scripts/cli-dist.sh` and `scripts/toolchain.sh` take one per-user lock.
- No implementing agent runs `sudo` or any root step. The one privileged step (`scripts/install-build-slice.sh`) is
  prepared as a script and run by the coordinator through the owner's askpass dialog (§13.1).
- Never build inside `vendor/inkboard`. Inkboard builds run from the patched copy `.cache/inkboard-src` (§10.5).
- Licensing: Quire is MIT. No code from AGPL or BSL projects (Outline, Lore, Docmost, Wiki.js, Drafft.ink),
  and no design tokens lifted from them. Code borrowed from GetPaper (MIT) gets a header comment plus an entry in
  `THIRD_PARTY.md`. A changed Inkboard file gets a "Changed for Quire" notice line (§10.2).

---

## 1. Scope of M1

Single owner. One workspace. Left sidebar of collections. Inside each collection, a tree of items nested to any depth;
an item is a document (TipTap) or a board (Inkboard), and either kind can have children. M1 also includes autosave,
full-text search, stars, archive, trash, login (one owner account), and Docker hosting with runtime limits behind a
Caddy HTTPS front (D57). Items are published on create; the draft state exists in the data model and the API only, with
no draft UI (D23). Every stored format must allow live multi-user co-editing to be added later without converting data.

M1 also ships, for external programs and AI agents (owner requirement: "an API that can serve external requests, and a
CLI so we can have agents testing the tooling"):
- a **stable, versioned public HTTP API** under `/api/v1` with an OpenAPI 3.1 spec. The web client uses the same API
  with its session cookie; external clients use **personal access tokens** (§7). Documents are readable and writable as
  Markdown and ProseMirror JSON, boards as native Inkboard JSON (§6.4, §7.5);
- the **`quire` CLI** over that API, with `--json` output and stable exit codes (§7.6, §7.7), and server-side admin
  commands in `quire-server admin` (§7.8);
- an **agent test harness**, `scripts/agent-test.sh`, that runs CLI scenarios against the production image (§14.1).

Not built in M1: templates, sharing, invites, avatars or "Viewed by N", comments, the version-history UI, backlinks UI,
imports (including server-side board import from draw.io, Excalidraw, Mermaid or JSON Canvas, D52), Markdown-zip export
of the whole workspace, PNG thumbnails, and board images as attachments. For these, M1 only reserves columns, routes or
layout slots (§15).

---

## 2. Decisions

### 2.1 Decision log

| # | Decision | Rationale |
|---|---|---|
| D1 | Rust 2024, axum 0.8.9, tokio 1.54. One server binary `quire-server` with subcommands `serve`, `healthcheck` and `admin {create-owner, create-token, list-tokens, revoke-token, reset-password, backup, check}` (§7.8); the CLI is a second binary, `quire` (D54, D55) | Single self-hosted server artifact; the distroless image has no shell, so the binary does its own healthcheck. Admin commands need the database, so they live in the server binary. |
| D2 | **rusqlite 0.40.2** (bundled SQLite with FTS5) plus deadpool-sqlite 0.14: one writer pool of size 1 using `BEGIN IMMEDIATE`, one reader pool of size 4 (`query_only`), and an on-demand **maintenance connection** (same PRAGMAs, no `query_only`, never used for app writes) for `VACUUM INTO`. Not sqlx. rusqlite, deadpool-sqlite and rusqlite_migration must resolve to one `libsqlite3-sys` (W0b checks with `cargo tree -i`). If deadpool-sqlite or rusqlite_migration lag rusqlite 0.40, W0b replaces them with a ~100-line pool in `db.rs` or a `user_version` runner in `migrate.rs` and records it in §16. | Matches SQLite's single-writer model. Gives synchronous transactions for tree logic, and the backup and hooks APIs. Compiles lighter under the 6-CPU cap. `VACUUM INTO` fails with SQLITE_READONLY on a `query_only` connection (checked on SQLite 3.46.1). |
| D3 | Migrations use `rusqlite_migration` 2.6 `from_directory`: `crates/quire-store/migrations/NNNN-name/up.sql`, embedded with `include_dir`. Until the first release, `0001-init` holds the whole M1 schema and only its owner (W1) edits it (PLAN rule R5). After the first release, migrations are append-only. | No registry file to edit, consecutive numbering, and nothing shipped that would need a migration chain before M1. |
| D4 | IDs are UUIDv7 TEXT primary keys. Items and collections also get a `url_id` (10 random base62 characters, UNIQUE) used only in URLs | UUIDv7 is standard, sortable and readable in tools. The short id keeps URLs like `/doc/<slug>-<url_id>`. |
| D5 | Time is stored as INTEGER unix milliseconds and sent in the API as RFC 3339 UTC strings | Cheap comparisons in SQL; unambiguous on the wire. |
| D6 | Sibling, collection and star order use fractional-index base62 keys generated **on the server** (`quire-core::rank`, a port of the CC0 `fractional-indexing` algorithm). The client sends `{after_id, before_id}`. | A move writes one row. Clients never compute keys, which avoids divergent implementations. |
| D7 | Document bodies are **Yjs (v1 update encoding) from day one**. The client runs TipTap Collaboration with no provider and POSTs incremental updates. The server (yrs 0.28) validates them, appends them to `doc_updates`, compacts, and derives text, links and image references. ProseMirror JSON is only a cache. A save is a no-op only when both the state vector and the delete set are unchanged (§6.1). | The future WebSocket sync writes the same log, so there is never a conversion. Two tabs already merge in M1. A delete-only update moves no clock, so the state vector alone cannot detect a no-op. |
| D8 | Boards store the **native Inkboard envelope** `{"format":"inkboard","version":2,"document":…}` as received. Image bytes are split out into a sha256 blob store (`board_assets`), and the rest is deflated. | This is the format the engine reads and writes. sha256 is the hash inkboard-session uses for its asset manifest, and the snapshot becomes `Sequencer::new(doc, cap)` for server-owned sync later. |
| D9 | The server never re-serializes a board through the typed model. It validates with `inkboard_core::serial::from_json` and splices assets on the raw JSON with `serde_json::value::RawValue`. | An engine-version mismatch cannot silently drop unknown fields. Peak memory stays about 3× the body size, because no `serde_json::Value` tree is built. |
| D10 | Board previews are **SVG generated by `inkboard_core::svg::export_approx`**, one per page, cached per revision. They serve as thumbnails, the loading placeholder and the read-only fallback. No PNG in M1. | inkboard-core is serde-only. PNG output needs inkboard-formats, which adds 104 crates (D52), including two resvg stacks; that is deferred. |
| D11 | **The board engine runs in one persistent same-origin `<iframe>`** (`/inkboard/{build}/frame.html`) mounted at the app-shell level. It is mounted once and boards are swapped with `setBoard`. Off board routes it is paused (`setActive(false)`). | The iframe isolates Inkboard's page-global listeners (hash stealing, paste stealing), its thread-local singletons, the eframe 0.33 IME `<input>` that scrolls the page, and its keyboard capture. The relative `icons.bin` URL just works. One instance avoids the destroy/remount path. Pausing stops the unconditional 500 ms repaint (`app.rs:1192`). |
| D12 | The Inkboard patch adds a cargo feature **`embed`** with the JS API `globalThis.inkboardEmbed.mount(canvas, opts) → Promise<Handle>` (§10.3), and an optional feature **`net`** that makes iroh optional. `mount` refuses a second live mount: the engine's state is per-window thread-locals (`platform_web.rs:33-40`). Kept as a patch series in `patches/inkboard/`, applied to a copy; the submodule is never modified. | Small and generic enough to upstream; the pinned submodule stays clean and is easy to rebase. |
| D13 | **No eframe upgrade in M1** (stays on 0.33.3). The engine is never started or switched while `document.visibilityState` is `hidden` (§10.4), and a frame test covers a dirty board in a background page (§14). | The iframe makes the IME and text-agent fixes of 0.34–0.37 irrelevant. The background-tab texture panic (fixed in 0.36, egui #8313) is triggered by loading in a hidden tab, which the visibility wait avoids. The upgrade touches about 14k lines of Inkboard tests; leave it to upstream. |
| D14 | The wasm is built **without trunk**: `cargo build` → `wasm-bindgen --target web` → `wasm-opt -Os` (binaryen version_131, installed in the `toolchain-base` stage by W0b), plus a Quire-owned `frame.html`/`frame.js` | Deterministic, with no tool download at build time. The frame page has no inline script, so its CSP needs no hashes. |
| D15 | Static files are embedded with rust-embed 8.13 in a separate `quire-assets` crate. The build makes brotli and gzip variants; the server picks one from `Accept-Encoding` | Single binary. A frontend change recompiles only the small assets crate. |
| D16 | Request origin checks: a **Host allowlist** on every route except `/healthz` (blocks DNS rebinding), SameSite=Lax cookie, a Quire CSRF middleware that checks `Sec-Fetch-Site`/`Origin` on cookie-authenticated mutations and rejects those that carry neither header (§8), and mutating bodies never of a CORS-safelisted type (JSON, or a route's declared type such as `text/markdown`). No CSRF token. Requests authenticated by a Quire bearer token skip the CSRF checks (D43). | Equivalent protection for a same-origin SPA, with no token plumbing. Browsers send `Origin` on every non-GET request, and over plain HTTP (no `Sec-Fetch-*`) that rule carries the check; both paths are tested. Non-browser clients use tokens, so a cookie mutation with neither header has no legitimate sender. |
| D17 | Passwords use argon2id (`argon2` 0.6 defaults: m=19456 KiB, t=2, p=1) inside `spawn_blocking`, behind a `Semaphore(2)` | Current OWASP baseline. The semaphore bounds memory under the container cap. |
| D18 | Sessions are opaque 32-byte tokens; only their sha256 is stored. The cookie is `__Host-quire_session; Secure` on a request that arrived over HTTPS (`X-Forwarded-Proto: https` from the trusted proxy, §8, D57), and `quire_session` on a plain-HTTP request | No JWT, and sessions can be revoked. The cookie follows how the browser actually reached Quire, so the direct plain-HTTP paths (e2e, agent harness) keep working. |
| D19 | First-run setup requires a setup code, printed in the log (or set with `QUIRE_SETUP_TOKEN`). `/api/v1/setup` shares the login per-IP rate limiter. | A fresh container on a LAN cannot be claimed by whoever opens `/setup` first. |
| D20 | Frontend: React 19, TypeScript, Vite 8, React Router (data router), TanStack Query for server state, Zustand for UI state, lucide-react icons (ISC), `@atlaskit/pragmatic-drag-and-drop` (Apache-2.0) | All permissively licensed, small, and well known. |
| D21 | The sidebar loads **a whole collection's live tree as one flat list** (`GET /api/v1/collections/{id}/tree`, with ETag `"t{tree_version}.{viewer_epoch}"` and 304) and builds the tree on the client. Rows are virtualized when there are more than 300 visible. | One request, and no recursive lazy-load states. Enough for a personal wiki: 5k items is about 150 KB gzipped. `viewer_epoch` is 0 in M1 and becomes a per-user permission epoch later. |
| D22 | API DTOs are Rust structs in the crate `quire-api` (D40), re-exported as `quire-core::api::*`. They are exported to TypeScript with ts-rs 12 into `web/src/api/gen/` (one file per type, committed) and to the OpenAPI spec with utoipa (D41). CI fails on a diff of either. | One source of truth for the contract between parallel workstreams, the web client, the CLI and external clients. |
| D23 | **Items are published on create** (`published_at = created_at`; owner decision 2026-10-28, §17.3). The draft state stays in the data model and the API, so a Drafts workflow can be switched on with multi-user: `published_at IS NULL` is a draft, which in M1 only `POST …/unpublish` (API or CLI) makes; `POST …/publish` (`with_ancestors` publishes draft ancestors too), `GET /drafts`, `drafts_count` and `include_drafts` work as specified. Visibility is **derived**: an item is *effectively published* when it and every ancestor are published; unpublish, move, restore and unarchive need no publish rules. The M1 web client has no draft UI: no Publish button or step, no Draft pill, no Drafts entry or page, no publish shortcut; an unpublished item looks like any other (single-user visibility ignores publication, §5). When multi-user arrives, new items may start as drafts again, drafts stay private to their author, the draft UI appears with `config.multi_user`, and the upgrade offers "Publish all drafts". | A single owner gains nothing from a Publish step. Keeping the column, routes and derived rule costs little and turns the multi-user workflow into a default change plus UI, not a migration. A derived rule cannot be broken by moves or restores, so nothing needs enforcing. |
| D24 | Soft states are timestamps (`published_at`, `archived_at`, `deleted_at`) plus a root pointer (`archive_root_id`, `trash_root_id`), so a subtree is restored as one unit. Deleting a collection sets `trash_root_id = <collection id>` on its live items. | Trash, Archive and Drafts are queries. Nothing is copied. |
| D25 | Runtime container: distroless `cc-debian13:nonroot`, read-only rootfs, `cap_drop: ALL`, **2 CPUs / 1 GiB / no swap / 256 pids** by default (`QUIRE_CPUS=2`, `QUIRE_MEM=1g`, overridable; owner decision 2026-10-28, §17.3). Tokio runs 2 worker threads (one per CPU) and at most 16 blocking threads. Work permits: `board_work` (1) for board GET, PUT and previews; `doc_work` (3), one per document read or Yjs POST, and two at once (`acquire_many(2)`, taken once, before the body is read) for a whole-document PUT (§6.4), so one PUT and one document read can run side by side; `upload_work` (2) for attachment uploads, which stream to `/data/tmp`; export and backup are single-flight (D47). Permits are owned and moved into the blocking work they guard, so a request that is dropped (timeout, client gone) keeps its permit until that work ends. Board response bodies of 1 MiB or more (the envelope GET, the `.inkboard` download, page SVGs) also take a share of a 96 MiB **body budget** (their size in MiB) before the body is built; it is always taken before `board_work`, never while holding it, the body is sent in 256 KiB chunks through a 4-chunk channel, a client that reads nothing for 30 s or is not done after 600 s loses it, and a body that waits more than 10 s for its share answers 429 `busy` with `Retry-After: 5`. Apart from that fixed order, no code path acquires a permit while holding one. | The owner requires limits. Budget for 1024 MiB: board work ≤ 150 MiB (one 50 MB save at about 3×); doc work ≤ 480 MiB (three loaded docs at up to 10× their 16 MiB cap, or one PUT's parse result and base, bounded by the 1 MiB / 4 MiB inputs and the nesting pre-scan, beside one loaded doc); uploads ≤ 16 MiB of buffers; export or backup ≤ 16 MiB (the maintenance connection's 8 MiB cache; the database copy and files streamed in 256 KiB chunks through a 4-chunk channel, never a whole file in memory); argon2 2 × 19 MiB; SQLite caches 5 × 8 MiB; the 32 MiB `/tmp` tmpfs, which the kernel charges to the container's memory. That is 772 MiB; the remaining ~250 MiB covers the runtime, thread stacks, response buffers and an in-container `quire` CLI (one body copy at most, §7.6); the body budget bounds what large board responses add to it. The second CPU and the extra 256 MiB buy a third doc permit, not a bigger board: the board limit stays 50 MB, and the headroom grows from about 170 MiB (768 MiB, tmpfs counted) to about 250 MiB. Because a PUT takes its two permits in one step (tokio's semaphore is fair and all-or-nothing), two PUTs cannot deadlock. In e2e and agent runs the 256 MiB `/data` tmpfs is charged to the same limit; their data stays far below it. |
| D26 | Blobs are content-addressed files under `/data/files/aa/bb/<sha256>`, with metadata in a `blobs` table. Write path: `stage` (write tmp in `/data/tmp`, fsync; outside the writer), then `commit` (rename into place, or drop the tmp and refresh the mtime when the file exists, then `INSERT OR IGNORE` the row; inside the writer transaction). GC runs in one writer closure: delete unreferenced rows, commit, unlink the files while still holding the writer. A sweep removes files that have no row and are older than 1 h. | Off-host backups (restic or rsync) dedupe immutable files, and blobs stay out of the WAL. Every rename and every unlink happens under the single writer, so a concurrent save can never end with a row whose file was deleted. |
| D27 | Search: FTS5 external-content table over `item_text(title, body)`, tokenizer `unicode61 remove_diacritics 2`, prefixes 2 and 3, bm25 with the title weighted 8×. The query builder is in §5. | Built in, fast, and ignores accents. Folding also merges å/ä→a and ö→o (§17); the index is derived data, so a tokenizer change later is a rebuild, not a migration. |
| D28 | Board conflicts use `If-Match: "r{rev}"`, answered with **412** `rev_mismatch`. Yjs update POSTs cannot conflict (CRDT); whole-document writes over the API use `If-Match: "d{seq}"` with the same 412 (D45, D50). | HTTP semantics. |
| D29 | No outbound network calls from the server in M1: no reqwest, no TLS stack, no relay. HTTPS and ACME live in the Caddy container in front of it (D57). The CLI (`quire-cli`) is a client and has its own rustls stack (D54); it is never linked into `quire-server` (checked with `cargo tree`, §14). | Self-hosted privacy, and less to compile. |
| D30 | The Inkboard **iroh live sharing is compiled out** of the embedded wasm (`--no-default-features --features web-glow,embed`). If the `net` patch is dropped, the code is still built but its UI is hidden and the frame CSP blocks relay sockets. | A self-hosted wiki must not contact a third-party relay. |
| D31 | The board title is owned by Quire (`items.title`). The server overwrites `document.name` when it returns a board, and ignores it on save. | One source of truth for the tree, breadcrumbs and search. |
| D32 | Doc autosave: 1 s debounce, at most 5 s apart, plus a flush on hide, blur, navigation and Ctrl/Cmd+S. Board autosave: 1.5 s quiet, at most 10 s apart, plus the same flushes. Title autosave: 500 ms debounce, registered in the same pending registry. | A board save sends the whole snapshot, so it is less frequent. |
| D33 | Limits: upload 25 MB (`QUIRE_MAX_UPLOAD_MB`); image 100 megapixels (read from the header only); board 50 MB (`QUIRE_MAX_BOARD_MB`, warning at 15 MB); stored Y document 16 MiB (`QUIRE_MAX_DOC_MB`); a single update POST up to 16 MiB; a Markdown document PUT up to 1 MiB and a ProseMirror JSON PUT up to 4 MiB (nesting depth 100); every other body 1 MiB; per-token body bytes (D47). Writes that stage files (uploads, board PUTs, export, backup) first check free space on `/data` (`statvfs`, rustix): below the smaller of `QUIRE_MIN_FREE_MB` (default 1024) and 10% of the filesystem they answer 507 `insufficient_storage` | A loaded yrs Doc takes 5–10× its encoded size, so 16 MiB (at most 160 MiB loaded, one `doc_work` permit) is the ceiling the 1 GiB budget of D25 is built on. A full disk would stop SQLite, and with it the whole wiki. |
| D34 | Export in M1: `GET /api/v1/export` streams a tar of `manifest.json`, a copy of `quire.db` (made with `VACUUM INTO` under `/data/tmp`, then sessions and API tokens deleted, password hashes cleared, and the `sqlite_stat1`/`sqlite_stat4` rows of the credential tables deleted, because `PRAGMA optimize` samples index keys such as token hashes into them, on a connection with `journal_mode=DELETE` and `secure_delete=ON`, closed before streaming, so no `-wal` file keeps the deleted rows; then a second `VACUUM INTO` from that scrubbed copy writes the file that is streamed, because the unused space of the first copy's pages can still hold older copies of deleted rows, such as stat4 samples of token hashes; the first copy is deleted) and `files/**`. Single-flight (D47). Per item: a document exports as Markdown (server, `GET …/doc/markdown`, §6.4) or HTML (client-side), a board as `.inkboard` or SVG (server). | Exact and cheap, and the download carries no credentials. A whole-workspace Markdown zip is deferred to M2; with server-side Markdown (D49) it is a loop over items. Full backups (§12) keep everything. |
| D35 | Passwords are 12–1024 bytes. Reset is done with `quire admin reset-password --email X` (`quire-server admin …`, §7.8), which sets a random temporary password, prints it, sets `must_change_password`, and revokes all sessions; with `--revoke-tokens` also every API token of that user (the compromise switch) | M1 sends no email. A one-time password needs no reset-link route. |
| D36 | Doc images are referenced through `item_attachments`, rebuilt from the ytext walk in every save transaction and copied by duplicate. Yjs revisions record their attachment ids. Attachments that no live item, revision or setting references are deleted after 24 h. `attachments.item_id` is the upload context only and is never used for authorization. | Images removed from docs are eventually freed, and later multi-user authorization of `GET /api/v1/attachments/{id}` can go through the referencing items. |
| D37 | Open-ended value lists are lookup tables, not CHECKs: `content_formats`, `revision_reasons`, `link_kinds` (like `user_roles` and `item_kinds`). Formats are stored as `(format, format_version)`, e.g. `('inkboard', 2)`, `('yjs', 1)`. | SQLite cannot alter a CHECK without a 12-step table rebuild; `revisions` holds large binary snapshots. A new format or link kind is one `INSERT`. |
| D38 | Build caps are enforced by the kernel at two levels: each build container (`--cpus 6 --memory 12g --memory-swap 12g`, pinned to the host's last 6 cores with `--cpuset-cpus`), and the systemd slice `quire-build.slice` (`CPUQuota=600%`, `MemoryMax=12G`, `MemorySwapMax=0`, `CPUWeight=20`, `IOWeight=20`; the weights also on its parent `quire.slice`), installed once by the coordinator through the owner's askpass dialog (owner decision 2026-10-28, §17.3). Every build container, the BuildKit daemon behind `quire-limited`, and the e2e, agent and smoke containers join it as their cgroup parent (Docker's systemd cgroup driver takes the slice name; checked at run time by `scripts/lib/slice.sh`). Without the slice everything still runs, with the per-container caps and a warning. The flock only queues builds. | The owner's machine has been brought to 100% by Rust builds; caps were requested "asap". The slice caps the total even if several capped containers run at once, or if someone runs `docker buildx build --builder quire-limited` outside the scripts. buildx's `docker-container` driver applies `cgroup-parent` only under the cgroupfs driver (checked in buildx's driver source and docs, 2026-10-28), so `scripts/toolchain.sh` starts the BuildKit container itself and registers it with the `remote` driver (§13.1). Pinning makes `nproc`, Node's `availableParallelism()`, rayon and Go see 6 cores. |
| D39 | **One public API under `/api/v1`, used by the web client and by external clients alike** (§7). Every route the SPA calls is in the OpenAPI spec; there are no private routes. Two authentication methods: the session cookie (browser) and personal access tokens (`Authorization: Bearer`). Within v1, changes are additive only (§7.1); a breaking change ships as `/api/v2` next to v1. | The public API can never lag the UI, and the web client, the CLI and agents exercise the same handlers. A version prefix costs nothing now and avoids a flag day later. |
| D40 | **DTOs live in their own crate `quire-api`** (serde, ts-rs, utoipa; no I/O, no yrs, no Inkboard). `quire-core` re-exports it as `quire_core::api`, so every `quire-core::api::*` path in this document stays valid. It also holds `ErrorCode`, the error envelope, `Page<T>`, `Scope` and the CLI exit-code table. | The CLI shares the request and response types with the server without compiling yrs, inkboard-core or comrak (§4.1). |
| D41 | **OpenAPI 3.1 is generated with utoipa 6.0.0 and utoipa-axum 0.3.0** (checked on crates.io and docs.rs 2026-10-27: utoipa 6 serializes `openapi: "3.1.0"` by default, `OpenApiVersion::Version31`; utoipa-axum 0.3.0 requires axum ^0.8.4). Route modules return `OpenApiRouter<AppState>` and register handlers only through `routes!` on `#[utoipa::path]` handlers; DTOs derive `ToSchema` next to ts-rs `TS`. Served at `GET /api/v1/openapi.json`, committed as `docs/api/openapi.json`, and a test fails on drift (§7.4). No interactive docs UI. Fallback if utoipa 6 does not build: utoipa 5.5.0 + utoipa-axum 0.2.0, recorded in §16. | aide's stable line (0.15.1, 2025-08) depends on schemars 0.9 while schemars is at 1.2.3, its 0.16 is still alpha, and it needs `OperationInput` impls for every custom extractor. With `routes!` the path and method come from the same annotation as the spec, so they cannot diverge. utoipa's path and schema maps are ordered maps (`preserve_order` and `preserve_path_order` stay off), so the committed JSON does not depend on merge order. Swagger UI or Scalar would need a build-time download or CDN scripts (D29, CLAUDE.md). |
| D42 | **Personal access tokens**: `qpat_` + 40 base62 random characters + 6 base62 characters of CRC32 over them (51 characters). Only sha256(token) and a 12-character display prefix are stored (`api_tokens`, §5); the secret is shown once. Scopes `read` ⊂ `write` ⊂ `admin`, stored expanded in `api_token_scopes`. Optional expiry (default 90 days, `null` = never). `last_used_at`/`last_used_ip` are written at most once per minute. Revocation takes effect on the next request (no cache). At most 100 live tokens per user. A token created by a token never has `admin` scope, never outlives its parent, and is revoked with it (`parent_token_id`); a session re-enters the password to create an `admin` or never-expiring token. | High-entropy secrets need a fast hash, not argon2 (same reasoning as D18). The prefix makes leaked tokens findable by secret scanners; the checksum lets the CLI reject a mistyped token without a request. A scope junction table allows finer scopes later with one INSERT (D37). A leaked token cannot outlive or escalate itself, and an XSS in the web session cannot silently mint a permanent admin credential. |
| D43 | **A Quire bearer token wins.** One predicate, `is_quire_bearer` (an `Authorization` header with scheme `Bearer` and credentials starting `qpat_`), is used by both `Auth<S>` and the CSRF layer. A matching request is authenticated by the token alone (a bad token is 401 `invalid_token`, never a cookie fallback) and skips the CSRF/Fetch-Metadata checks (D16). Any other `Authorization` value (HTTP Basic from a proxy, an oauth2-proxy JWT) is ignored: the request takes the cookie path with the full CSRF checks. Session-only routes (password change, logout) answer token requests with 403 `session_required`. | A browser cannot attach a custom `Authorization` value to a cross-site request without a CORS preflight, which Quire refuses by default (D48), so Quire bearer requests carry no ambient authority and may send any `Origin`. Browsers do replay cached Basic credentials and proxies add their own headers, so those must neither disable the cookie nor skip CSRF. |
| D44 | **Scope enforcement is a typed extractor `Auth<S>`** (`S` = `Read`, `Write`, `Admin`, `Session`, `AnyToken`), always the first handler argument, so a missing scope is answered (403 `insufficient_scope`) before any path, query or body parsing. A test calls every operation of the generated spec with a token one scope below the declared one (§7.4), and another pins the exact list of public operations, so a handler annotated `security: []` by mistake fails. | One enforcement point and one generic test instead of one test per route; the spec documents exactly what is enforced. |
| D45 | **Optimistic concurrency is ETag/If-Match with 412 `rev_mismatch` everywhere**: boards `"r{rev}"` (D28), documents `"d{seq}"` (seq of the newest stored update), item metadata `"m{meta_version}"` (optional on PATCH; replaces the earlier `meta_version` body field). Content writes require `If-Match` (428 otherwise); `If-Match: *` is an explicit last-write-wins. A `W/` prefix on these tags is ignored. `GET /items/{id}`, the Markdown GET and the board GET send their tag for `If-Match` but never answer 304, because their bodies hold data the tag does not version (stars, breadcrumbs, board meta, live titles, the board's `document.name`). | One rule for the web client, the CLI and external clients, reusing the board rule that already existed. A proxy that weakens ETags (nginx gzip) must not break writes. |
| D46 | **Cursor pagination**: opaque base64url cursors over a keyset `(sort value, id)`, bound to the normalized query by an 8-byte hash (a cursor reused with other parameters gives 422 `validation`, `details.field = "cursor"`). Relevance-sorted search uses an offset cursor, capped at 1,000 results. Every paginated list returns `{items, next_cursor}`, `next_cursor = null` on the last page. | Stable under inserts and deletes, no server state, one page shape for every list. |
| D47 | **Per-token limits** with governor keyed by token id: `QUIRE_TOKEN_RATE` requests per minute (default 600, burst 120) and `QUIRE_TOKEN_BYTES_PER_HOUR` of request body (default 1 GiB, counted in KiB as bodies are read), answered with 429 `rate_limited` (`details.reason` `requests` or `bytes`) and `Retry-After`. Failed bearer authentications are counted per IP (30 per minute; the client IP of §8, from Caddy's `X-Forwarded-For`); over that, a token that fails lookup gets 429 instead of 401, while valid tokens from the same IP keep working. Export and on-demand backup are single-flight (429, `reason: "busy"`). Cookie sessions keep only the login limits (§8). Limiter keys are pruned hourly (`retain_recent`). | Protects the 2-CPU container and the disk from a runaway agent loop, without locking out every client behind one IP. The CLI retries 429 honouring `Retry-After`. |
| D48 | **CORS is off by default.** `QUIRE_CORS_ORIGINS` (exact origins, comma-separated; `*` refused at startup) enables tower-http `CorsLayer` (feature `cors`) on `/api/v1/*` for bearer clients; `Access-Control-Allow-Credentials` is never sent, so cookies never work cross-origin. | Browser-based tools can opt in to tokens without weakening the cookie model. |
| D49 | **Documents are readable and writable as Markdown and ProseMirror JSON, converted in Rust** (`quire-core::convert`, §6.4). Y `XmlFragment` ↔ ProseMirror JSON mirrors `@tiptap/y-tiptap` 3.0.10 (MIT, the editor's binding) exactly, and the API's JSON is the normalized shape of `editor.getJSON()`; Markdown is parsed with **comrak 0.59** (`default-features = false`; BSD-2-Clause) and written by Quire's own deterministic printer. The web client's "Export Markdown" uses the same endpoint, so `prosemirror-markdown` leaves the web package. | One Markdown dialect, one implementation. comrak gives an AST plus GFM tables, task lists, strikethrough, autolinks, GitHub alerts (callouts) and `==highlight==`; pulldown-cmark 0.13 has no highlight. Quire's own printer keeps golden files stable across comrak upgrades and controls escaping. comrak's default features (`cli`, `syntect-onig`, a C library) are off. |
| D50 | **External document writes are diffs, not replacements.** A Markdown or JSON PUT is diffed against the document at its `If-Match` base: Markdown in Markdown space (the base printed and re-parsed, so a block whose Markdown the input leaves alone is never touched), JSON against the normalized base. Children are anchored by `blockId` (JSON) and by content hash (an order-preserving diff); the runs between anchors are paired by type and similarity and updated in place; text gets a character-level diff (§6.4). The result is one Yjs update whose client id and new `blockId`s derive from a hash of the request, so a repeated request is a no-op; it is saved through the normal path (§6.1). The writer re-checks the base: a stale base gives 412 (also when it went stale during the PUT), unless the target already equals the current state (200 `changed: false`), or `?merge=true` and the base is still reconstructible (`seq ≥ compacted_seq`), when the CRDT merges it with the newer updates. | Unchanged blocks keep their Y identity and `blockId`s, so open editors, outboxes, heading anchors and the later live sync merge cleanly, and nothing outside the Yjs log is written. Strict by default (as D28); the opt-in merge lets an agent edit while the owner types; retries are safe. |
| D51 | **Boards over the API use the web path unchanged** (D8, D9, D31): the native envelope in and out, raw-JSON splicing, unknown fields preserved, `document.name` owned by the title, `If-Match` required (`"r{rev}"` or `*`), SVG page previews. `X-Quire-Engine` is optional for API clients (`engine_version = "api"` when absent). | One pipeline and one set of preservation tests for every client. |
| D52 | **Server-side board import (draw.io, Excalidraw, Mermaid, JSON Canvas) is deferred.** `inkboard-formats` has no cargo features (`vendor/inkboard/crates/inkboard-formats/Cargo.toml:9-23`): over inkboard-core it adds 104 crates (two resvg/usvg/fontdb/tiny-skia stacks, svg2pdf, image codecs, latex-rust; counted from `vendor/inkboard/Cargo.lock`). The importers themselves (`import_any`, `lib.rs:109`) use only quick-xml, regex, flate2, percent-encoding and inkboard-icons (13 crates) plus `inkboard_core::text::ApproxMeasurer` (`inkboard-core/src/text.rs:214`); resvg and svg2pdf are used only by `raster.rs`, latex-rust only by `math.rs`. `POST /api/v1/items/{id}/board/import` and `quire board import` are reserved. | The weight is not acceptable under the 6-CPU build cap and in the 1 GiB container, and Quire compiles Inkboard's crates unpatched (§4.1), so a local feature split is not possible. An upstream patch that makes `raster`, `pdf` and `latex` optional (§15) turns import into a 13-crate addition. Quire's tooling does not build Inkboard's own `ink convert` (`crates/inkboard-cli/src/main.rs:1-7`) in M1 either, because it links the same crates; a user who has it installed can convert files offline before `quire board put`. |
| D53 | **API content writes keep a recovery point**: before a token-authenticated content write (document Markdown/JSON PUT, Yjs POST, board PUT), `revisions::pre_api` stores a `pre-api` revision of the current state, unless the last content write came from the same token (`last_token_id` on `doc_state`/`board_state`) or the newest revision already holds the current state (`revisions.source_seq`). | The owner's latest work is always in a revision before an agent overwrites it, and an agent's own consecutive writes add none. Recoverable once the history UI ships (M2); the data exists from M1. |
| D54 | **The CLI `quire`** is its own crate `quire-cli`: clap 4.6, **ureq 3.5** (blocking; `default-features = false`, features `rustls-no-provider`, `json`, `gzip`) with rustls 0.23's `ring` provider passed in its `TlsConfig`; root certificates come from the system CA bundle, or `--ca-file` for a private CA (`RootCerts::Specific` either way); no OpenSSL and no bundled webpki-roots. etcetera 0.11 (XDG config dir), toml 1.1. It depends on `quire-api` and rustls only. Every command takes `--json`, exit codes are a contract (§7.7), and it never prompts. Shipped as a static `x86_64-unknown-linux-musl` binary, standalone and in the image. | Blocking ureq compiles far lighter than reqwest 0.13, which pulls tokio and hyper, defaults to aws-lc-rs, and depends on tower-http 0.6 next to Quire's 0.7. ureq's `rustls` feature adds webpki-roots, whose CDLA-Permissive-2.0 licence is not on Quire's allow-list (§4.1); every Linux host and the distroless image ship a CA bundle, and Caddy's local CA (D57) is trusted the same way, by adding its root to the system bundle or with `--ca-file` (owner-accepted default, §17.3). A static musl binary runs on any Linux and in the distroless image. |
| D55 | **Admin commands live in the server binary** (`quire-server admin …`, §7.8) because they open the database directly. The CLI's `quire admin …` execs `quire-server admin …`, found through `QUIRE_SERVER_BIN`, then next to the `quire` binary, then on `PATH`, so `docker compose exec quire quire admin …` works. | The CLI stays free of rusqlite and the store; the server stays free of TLS. One spelling in the docs either way. |
| D56 | **Agent testing** has three layers: `crates/quire-cli/tests/cli` runs the CLI binary (assert_cmd 2.2) against an in-process server; `scripts/agent-test.sh` runs the production image with `compose.e2e.yaml` (the compose caps, the build lock, the build slice when installed), talks to Quire directly on the project network (Caddy is not started), refuses an image built from another commit, provisions an owner and tokens with `quire admin`, and runs bash scenarios of `quire --json` calls asserted with jq (§14.1); its `up` keeps the app for exploration for a bounded time (default 15 min) while a background holder keeps the build lock (owner-accepted default, §17.3); `docs/AGENTS.md` tells agents how to use the CLI. | Agents and CI exercise the shipped artifacts end to end with one command, under the owner's resource caps. |
| D57 | **HTTPS is terminated by Caddy in the compose project** (official image `caddy:2.11.7`, pinned by digest; `docker/Caddyfile`, §13.4; owner decision 2026-10-28, §17.3). Quire listens only on the project network (`expose: 8080`, no published port); Caddy publishes 80 and 443 and has its own caps (0.5 CPU, 128 MiB, no swap) and volumes for `/data` and `/config`. Two modes from env: `QUIRE_TLS=acme` with a public `QUIRE_DOMAIN` (automatic certificates), and `QUIRE_TLS=internal` (default, with `QUIRE_DOMAIN=localhost`) for `localhost` or LAN names such as `quire.lan`, using Caddy's local CA, whose root the owner trusts by hand (§13.4). Caddy has a fixed address on a fixed project subnet, outside the subnet's dynamic `ip_range` so no other container can be given it (§13.3), and `QUIRE_TRUSTED_PROXIES` is exactly that address, so only Caddy's `X-Forwarded-For` (rate limits, logs) and `X-Forwarded-Proto` (Secure cookie, HSTS) count (§8). `QUIRE_DOMAIN` joins the Host allowlist. The Caddyfile has no `encode` and proxies with `compression off`, so Quire's precompressed br/gzip bytes pass through untouched. HSTS (`max-age=31536000`, no `includeSubDomains`, no `preload`) is sent by Quire, only on HTTPS requests in `acme` mode. The CSP is unchanged. | Quire stays free of TLS and outbound calls (D29): ACME runs in Caddy. Browsers offer brotli only over HTTPS, send `Sec-Fetch-*` only to secure origins, and accept `__Host-` cookies only over HTTPS, and Inkboard's copy-as-image needs a secure context. An address rule needs no shared secret and no DNS lookup from Quire. HSTS on `localhost` or on names signed by a private CA would pin every local server, or every LAN device without the root, to HTTPS for a year, so it is public-mode only; keeping it in Quire keeps every security header in one tested place (§11). |

### 2.2 Alternatives considered

The source reports are not in the repo; this table is the record. "UX" marks a value from the design-stage UX notes.

| Question | Rejected | Chosen |
|---|---|---|
| Embed | In-page canvas; iframe with a custom `window.inkboardHost` | **Iframe, using the generic `inkboardEmbed.mount` API inside it** (D11, D12). One engine instance per session. |
| Feature name | `quire-host` | `embed` (generic, upstreamable). |
| eframe 0.33→0.37 | Upgrade as a precondition (UX) | Not needed (D13), with the visibility wait. |
| Wasm build | trunk | cargo + wasm-bindgen + wasm-opt (D14). trunk stays in the dev toolchain only for Inkboard's own page. |
| `icons.bin` URL | A `set_icon_pack_url` patch | Dropped. The relative URL resolves to `/inkboard/{build}/icons.bin` inside the iframe. |
| Board storage | Deflated body with blobs in the DB; TEXT envelope with blobs on disk | Deflated `board_state.body` (stripped envelope, as received) plus blobs on disk (D8, D9, D26). |
| Asset stripping | Delete `document.assets` | Keep the entries with `"data":""`, so `sanitize` never drops images when the stripped form is parsed. |
| Debounces | Board 1 s / 5 s, 2 s / 10 s; doc 750 ms | Board 1.5 s / 10 s; doc 1 s / 5 s (D32). |
| Board conflict status | 409 (UX) | 412 `rev_mismatch` (D28). |
| Thumbnails | resvg PNG (UX) | SVG previews from inkboard-core (D10). |
| Tree loading | Lazy per node (UX) | Whole collection, flat (D21). "Loading children…" and "Show 200 more" are not built. |
| Item routes | `/d/`, `/b/`, `/c/` | `/doc/…`, `/board/…`, `/collection/…`. Slug redirects happen on the client (`history.replace`). |
| CSRF | Token (UX) | Host allowlist + Fetch-Metadata/Origin (D16). |
| Minimum password length | 10 | 12. |
| Login rate limit | 5 per 5 min per IP+account (UX) | 5 per minute per IP, plus 10 failures per 15 min per email, answered with 429 and `Retry-After`. |
| Reset | One-time link (UX) | One-time password (D35). |
| Upload / board limits | 50 MiB / 64 MiB | 25 MB / 50 MB by default. |
| Container memory | 512 MiB; 768 MiB and 1 CPU (the default until 2026-10-28) | 2 CPUs / 1 GiB (owner), with board and doc work bounded by permits; the extra memory buys a third doc permit, not a 64 MB board (D25). |
| Sort options | manual, title, updated | `manual`, `title`, `updated`, `created`. |
| Revisions | Endpoints and UI in M1 | M1 writes `revisions` rows (auto and pre-restore). Endpoints and UI come in M2. |
| Seed | No seed | A "Getting started" collection (§9.6.7). The Y document fixture is produced by the web test suite (W4), the board fixture is a JSON file. |
| Export | Markdown zip (UX) | Tar archive (D34). |
| Offline storage | y-indexeddb for the whole doc (UX) | An outbox of unsent merged updates in IndexedDB (`idb-keyval`), cleared when the server acknowledges them. Bounded; the server remains the source of truth. |
| Contents column | TipTap TableOfContents extension | Computed by Quire from `heading` nodes and their `blockId`. The extension stores a second random id per heading in the Y document. |
| Publish rule | Invariant "published ⇒ ancestors published", enforced on each action | Derived effective visibility (D23). |
| New items | Draft first with a Publish step and a Drafts badge (UX); a Drafts entry shown only while `drafts_count > 0` | Published on create; the draft state stays in the model and the API, with no draft UI in M1 (D23, owner 2026-10-28). |
| TLS | Quire's own rustls listener with ACME; Traefik; README advice only | Caddy 2.11.7 in the compose project (D57). |
| Trusting forwarded headers | The whole compose subnet; a shared-secret header from Caddy; a DNS lookup of `caddy` from Quire; a Unix socket between the two | Caddy's fixed address on a fixed project subnet as `QUIRE_TRUSTED_PROXIES` (D57). |
| HSTS | From Caddy; in every mode | From Quire, HTTPS requests in `acme` mode only (D57). |
| e2e and agent runs | Through Caddy with `tls internal` | Directly to Quire on the project network; the HTTPS path has its own smoke check (§14). |
| BuildKit in the slice | buildx `docker-container` driver with `cgroup-parent` (ignored under Docker's systemd cgroup driver); a daemon-wide `cgroup-parent` (would also capture the hosted app) | A BuildKit container started by `scripts/toolchain.sh` with `--cgroup-parent`, registered with the `remote` driver (D38). |
| Public API | A second, separate API next to the web's `/api` | One versioned API, `/api/v1`, for the web client and external clients (D39). |
| OpenAPI generator | aide 0.15 (schemars 0.9), hand-written YAML | utoipa 6.0 + utoipa-axum 0.3 (D41). |
| API docs UI | Swagger UI, Scalar | None in M1: the spec is served and committed (D41). |
| Token storage | argon2 hash, JWT | sha256 of a 238-bit random token, revocable (D42). |
| External doc writes | Replace the whole Y fragment; diff Markdown in ProseMirror space (rewrites attrs Markdown cannot express); pair children left to right (one insertion shifts the identity of every later block) | Diff against the If-Match base, in Markdown space for Markdown, anchored by `blockId` and content hash; optional CRDT merge (D50). |
| Markdown | pulldown-cmark, comrak's own CommonMark formatter, client-side prosemirror-markdown | comrak parser + Quire's printer, on the server (D49). |
| Board import over the API | Link `inkboard-formats` now | Deferred until an upstream feature split (D52). |
| CLI HTTP client | reqwest 0.13 (tokio, aws-lc-rs) | ureq 3.5 with rustls + ring (D54). |
| CLI TLS roots | Bundled webpki-roots (CDLA-Permissive-2.0, not on the allow-list) | The system CA bundle, or `--ca-file` (D54). |
| Admin commands | In the CLI over HTTP, a separate admin binary | `quire-server admin`, reached through `quire admin` (D55). |

---

## 3. Repository layout

```
Quire/
  Cargo.toml  Cargo.lock              # Quire workspace (exclude = ["vendor"]); lockfile committed
  .cargo/config.toml                  # jobs = 6; [env] TS_RS_EXPORT_DIR = web/src/api/gen (relative),
                                      # TS_RS_LARGE_INT = "number" (i64/u64 DTO fields are `number` in TypeScript)
  rust-toolchain.toml                 # channel = "1.99.1", targets = ["wasm32-unknown-unknown", "x86_64-unknown-linux-musl"],
                                      # components = ["clippy", "rustfmt"], profile = "minimal" (= the image)
  LICENSE  THIRD_PARTY.md  CLAUDE.md  README.md
  crates/
    quire-api/       # DTOs (serde + ts-rs + utoipa ToSchema), ErrorCode, error envelope, Page<T>, Scope, exit codes (D40)
    quire-core/      # pure domain: ids, rank, tree rules, cursor, ytext (Y→text/links/images), convert (Y ↔ ProseMirror
                     # JSON ↔ Markdown, §6.4), schema (schema v1 as data), board codec; re-exports quire-api as `api`
    quire-store/     # rusqlite: Db, migrations/, repositories, blob store, tokens, backup/export, seed
    quire-server/    # axum: lib.rs (library target for tests) + main.rs; bin `quire-server`: routes, auth, tokens, csrf,
                     # cors, limits, headers, OpenAPI, jobs, admin commands; feature `testing` (TestServer for quire-cli)
    quire-assets/    # rust-embed of web/dist and web/dist-inkboard only
    quire-cli/       # bin `quire`: the CLI over /api/v1 (§7.6); tests/cli: assert_cmd against an in-process server
  web/               # React + TS + Vite + TipTap (pnpm). Output: web/dist
    inkboard-frame/  # frame.html, frame.js, frame.css, inkboard.d.ts, tsconfig.json: the iframe page (plain ES module)
    dist-inkboard/   # (git-ignored) output of scripts/inkboard.sh build|stub
  e2e/               # Playwright package (@playwright/test 1.64.1) + tests
    agent/           # agent harness: lib.sh, scenarios/NN-*.sh, fixtures/ (§14.1); results/ is git-ignored
  fixtures/          # cross-language golden files: ydoc/*.bin + *.txt + *.links.json + *.y.json + *.pm.json,
                     # ydoc/server/*.bin + *.pm.json, markdown/*.md (+ *.expected.md, *.warnings.json), schema/v1.json,
                     # boards/*.inkboard, tree/title-order.json (the A–Z sibling order, §9.6.2)
  patches/inkboard/  # 000N-*.patch (git format-patch against the base copy of 41e0295), README.md (base, patchset,
                     # sizes), check-baseline.txt (the warning counts of `inkboard.sh check` on the base, §10.5)
  docker/Dockerfile  # stages: node, toolchain-base, toolchain, inkboard-web, web, licenses, server, cli-src, cli,
                     # cli-alone, cli-dist, runtime (§13.2)
  docker/buildkitd.toml       # quire-buildkit daemon behind the quire-limited builder: max-parallelism = 2, GC 30 GB
  docker/quire-build.slice    # systemd slice: the total build cap (D38)
  docker/quire.slice          # its parent: low CPU and IO weight (D38)
  docker/Caddyfile            # the HTTPS front (D57, §13.4)
  compose.yaml  compose.e2e.yaml  compose.agent.yaml  compose.smoke.yaml  .dockerignore
  scripts/  dev.sh toolchain.sh image.sh image-smoke.sh inkboard.sh e2e.sh agent-test.sh cli-dist.sh fmt.sh
            check-pins.sh install-build-slice.sh lib/lock.sh lib/slice.sh
  docs/     ARCHITECTURE.md PLAN.md AGENTS.md api/openapi.json (generated, §7.4) handoff/
  dist/cli/          # (git-ignored) scripts/cli-dist.sh output: static CLI binary, its sha256, LICENSE-*, THIRD_PARTY.md
  vendor/inkboard                     # submodule, untouched; never built in place
```

`.gitignore` additions: `/web/dist-inkboard/`, `/web/.dist-inkboard.*/` (the staging directories of
`inkboard.sh`), `/e2e/test-results/`, `/e2e/playwright-report/`, `/e2e/agent/results/`, `/dist/`.
`docker/toolchain.Dockerfile` is removed in W0b: its content becomes the `toolchain-base` and `toolchain` stages of
`docker/Dockerfile`, and `scripts/toolchain.sh` builds `--target toolchain` from it.

---

## 4. Crates and packages

### 4.1 Rust workspace (`Cargo.toml`)

```toml
[workspace]
resolver = "3"
members  = ["crates/*"]
exclude  = ["vendor"]          # inkboard-core belongs to vendor/inkboard's own workspace
[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.95"
license = "MIT"
[profile.release]
lto = "thin"
codegen-units = 8
strip = "debuginfo"
[profile.dev]
debug = "line-tables-only"
[profile.dev.package.argon2]
opt-level = 3
[profile.dev.package.blake2]
opt-level = 3
```

Workspace dependencies (checked 2026-10-27; any crate not listed must be MIT, Apache, BSD, ISC, Zlib, Unicode or CC0, and
is pinned by W0b):
`inkboard-core = { path = "vendor/inkboard/crates/inkboard-core" }`, rusqlite 0.40.2 (`bundled`, `backup`, `hooks`),
deadpool-sqlite 0.14 (`rt_tokio_1`), rusqlite_migration 2.6 (`from-directory`), include_dir, axum 0.8.9 (`macros`),
tower 0.5.3, tower-http 0.7.1 (`trace`, `timeout`, `limit`, `set-header`, `request-id`, `catch-panic`,
`sensitive-headers`, `compression-gzip`), tokio 1.54.1, tokio-util, yrs 0.28, uuid 1.29 (`v7`, `serde`), argon2 0.6,
governor 0.10.4, rust-embed 8.13 (`mime-guess`; not `interpolate-folder-path`, which pulls option-ext, MPL-2.0: the
folders are relative to `CARGO_MANIFEST_DIR`), ts-rs 12, serde 1.0.229, serde_json 1.0.154 (`raw_value`), base64
0.23.1, sha2 0.11.1, thiserror 2.0.21, bytes 1.12.1, futures-util 0.3.34, flate2 (`rust_backend` only), tar (default
features off: no xattr), getrandom 0.4, clap 4.6.7
(`derive`, `env`), tracing, tracing-subscriber, cookie, imagesize, percent-encoding, utoipa 6.0.0 (`macros`, `uuid`;
never `preserve_order` or `preserve_path_order`), utoipa-axum 0.3.0, comrak 0.59.0 (`default-features = false`),
crc32fast 1.5, rustix (`fs`: `statvfs` for the free-space floor, D33). ts-rs gets `uuid-impl` and `serde-json-impl`;
tower-http also gets `cors`. CLI only (`quire-cli`): ureq 3.5.0 (`default-features = false`, `rustls-no-provider`,
`json`, `gzip`; never its `rustls` feature, which adds webpki-roots, D54), rustls 0.23 (`default-features = false`,
`ring`, `std`, `tls12`), etcetera 0.11.0, toml 1.1. Dev dependencies: axum-test 21.1, tempfile 3.27, proptest,
assert_cmd 2.2.2, oas3 0.22.0 (`default-features = false`; parses the spec as OpenAPI 3.1 in a test; its default
`preserve-order` feature would turn on serde_json's `preserve_order` in every workspace test build, so test and release
builds would order JSON objects differently). Versions and licences of the additions were
checked on crates.io on 2026-10-27 (utoipa, utoipa-axum, ureq, clap, assert_cmd, etcetera, toml: MIT OR Apache-2.0;
rustls: Apache-2.0 OR ISC OR MIT; ring: Apache-2.0 AND ISC; comrak: BSD-2-Clause; oas3: MIT). comrak 0.59.0 and ureq
3.5.0 are days old: W0b re-checks both (§16). Versions W0b picked where this list gives none are in `Cargo.toml` and
§16 item 24; unicode-normalization 0.1.25 (MIT OR Apache-2.0, already in the lock through comrak) is a plain
dependency of `quire-core` for `ids::slugify`.

Crates shared with Inkboard (serde, serde_json, base64, thiserror) must have the same versions in `Cargo.lock` as in
`vendor/inkboard/Cargo.lock`; `scripts/check-pins.sh` enforces this (§15 "exact protocol match").

| Crate | Depends on | Contents |
|---|---|---|
| `quire-api` | serde, serde_json, ts-rs, utoipa, uuid, crc32fast | `{common,tree,page,auth,tokens,files,docs,boards,error,cli,token}.rs`: every DTO of §7, `ErrorCode` with its HTTP status, `ErrorBody` (the envelope), `Page<T>`, `Scope`, `API_VERSION`, `cli::ExitCode` (§7.7), and `token::{encode, check}` (format and CRC32 checksum, D42; no randomness). No I/O. Used by quire-core (re-exported as `quire_core::api`), quire-server and quire-cli (D40). |
| `quire-core` | quire-api, inkboard-core, yrs, comrak, unicode-normalization, serde, serde_json, uuid, sha2, base64, thiserror, getrandom | `ids` (UUIDv7, url_id, slug), `rank`, `tree` (move validation rules), `time`, `cursor` (D46), `ytext` (Y walk → plain text, links, attachment ids, emptiness), `schema` (schema v1 as data), `convert/{pm,ydiff,md_in,md_out}.rs` (§6.4), `board` (envelope split/splice, text, meta, preview). `pub use quire_api as api`. No I/O. |
| `quire-store` | quire-core, inkboard-core, yrs, rusqlite, deadpool-sqlite, rusqlite_migration, include_dir, flate2, sha2, tokio, tar, imagesize, rustix, serde, serde_json, base64, thiserror, uuid, getrandom, tracing, bytes, futures-util | `error`, `db` (pools, PRAGMAs, maintenance connection), `migrate`, `shared` (helpers used across workstreams, `visible_items`, `WriteBy`, the free-space check), `collections`, `items`, `tree`, `search`, `stars`, `views`, `lists`, `users`, `sessions`, `tokens`, `settings`, `blobs`, `attachments`, `docs`, `boards`, `previews`, `revisions`, `gc`, `backup`, `export`, `seed` |
| `quire-server` | quire-core, quire-store, quire-assets, quire-api, inkboard-core, rusqlite (plain-SQL reads of its own), axum, tower, tower-http, tokio, tokio-util, bytes, futures-util, argon2, governor, clap, cookie, percent-encoding, utoipa, utoipa-axum, serde, serde_json, uuid, sha2, base64, thiserror, getrandom, rustix, tracing, tracing-subscriber; tempfile under `testing` | Binary `quire-server`. `lib` (module list, `pub fn app(state) -> Router`), `main`, `cli` (subcommands), `admin` (§7.8), `config`, `app` (router assembly), `state`, `error`, `extract` (`Auth<S>`, Actor, Viewer, ItemRef), `authz`, `csrf` (Host allowlist + origin checks), `cors`, `ratelimit`, `headers`, `static_files`, `jobs`, `board_previews` (preview worker), `openapi` (spec assembly, §7.4), `testing` (feature `testing`: `TestServer`), `routes/{auth,setup,workspace,tokens,collections,items,tree,lists,search,docs,boards,files,export,config,admin}.rs`. No TLS crate (D29). |
| `quire-assets` | rust-embed | `Web` (`web/dist`) and `Inkboard` (`web/dist-inkboard`) embeds with `#[allow_missing = true]`. Debug builds read from disk. |
| `quire-cli` | quire-api, clap, ureq, rustls, serde, serde_json, etcetera, toml, base64, sha2, thiserror | Binary `quire` (§7.6). `main`, `args` (clap tree), `config` (flags, env, profile file), `http` (ureq agent, retries, error mapping), `output` (human and `--json`), `refs` (UUID, url_id or Quire URL), `cmd/{auth,token,workspace,collection,item,tree,list,search,star,trash,doc,board,attachment,export,api,admin,version}.rs`. Dev-dependencies: assert_cmd, tempfile, `quire-server` with feature `testing`. Never depends on quire-core, quire-store, rusqlite, yrs or Inkboard. |

Inkboard enters Quire's graph only through `inkboard-core` (serde-only; later `inkboard-session`, also pure Rust).
Patches never touch these crates, so the server compiles them unpatched from the same commit as the wasm. Cargo reads
`vendor/inkboard/Cargo.toml` only for inherited fields (`version`, `edition`, `license`, `rust-version`, `[lints]`).

Formatting: always `scripts/fmt.sh [--check]`, which runs `cargo fmt -p quire-api -p quire-core -p quire-store
-p quire-server -p quire-assets -p quire-cli`. Never `cargo fmt --all`: it follows the path dependency into Inkboard's workspace and reformats or
fails on `vendor/inkboard`.

### 4.2 Web package (`web/package.json`, pnpm 11.24.0, exact versions, lockfile committed)

Runtime: react and react-dom 19.3.1, react-router, @tanstack/react-query, @tanstack/react-virtual, zustand, lucide-react,
@atlaskit/pragmatic-drag-and-drop (+ `-hitbox`, `-auto-scroll`), @tiptap/{react, pm, starter-kit,
extension-collaboration, extension-placeholder, extension-typography, extension-highlight, extension-underline,
extension-link, extension-image, extension-table (kit), extension-task-list, extension-task-item,
extension-code-block-lowlight, extension-mention, extension-unique-id, extension-file-handler,
extension-drag-handle-react} 3.32.0, @tiptap/suggestion 3.32.0, @tiptap/y-tiptap 3.0.10, yjs 13.6.33, lowlight,
idb-keyval. (No Markdown library: Markdown export comes from the server, D49.)
Dev: vite 8.3.5, @vitejs/plugin-react, typescript 7.0.2, vitest 5.0.5, happy-dom, @testing-library/react,
@testing-library/dom (its required peer), @testing-library/user-event, fake-indexeddb, @types/react and
@types/react-dom. `packageManager` is `pnpm@11.24.0`; the versions W0b picked where this list gives none are in
`web/package.json`. vite 8.3.5 hard-depends on lightningcss 1.33.0 (MPL-2.0, build time only, nothing of it in
`web/dist`), which is not on the licence allow-list: open for the owner (§16 item 24, `THIRD_PARTY.md`).
Scripts: `dev` (Vite on 0.0.0.0:5173, proxying `/api` and `/inkboard` to 127.0.0.1:8080), `build`,
`test` (`vitest run`; config `maxWorkers: 4`), `typecheck` (`tsc --noEmit -p . && tsc --noEmit -p inkboard-frame`),
`fixtures` (`WRITE_FIXTURES=1 vitest run src/editor/fixtures`).
Check the licence in each package's `package.json` before adding it. Never use TipTap Pro or Cloud packages. A new
package is resolved with a temporary `web/pnpm-workspace.yaml` (`minimumReleaseAge: 20160`, excluding the young pins
above) that is deleted afterwards and never committed: the §13.2 `pnpm fetch` cannot see it, and pnpm 11 then fails the
offline install with `ERR_PNPM_NO_OFFLINE_META`.

### 4.3 e2e package (`e2e/package.json`)

@playwright/test 1.64.1 only, matching the image `mcr.microsoft.com/playwright:v1.64.1-noble`. It is installed with pnpm
from the toolchain image and run from the Playwright image (§14). `playwright.config.ts` sets `workers: 2`.

---

## 5. Data model (`crates/quire-store/migrations/0001-init/up.sql`)

PRAGMAs are set per connection in code: `journal_mode=WAL`, `synchronous=NORMAL` (`QUIRE_SYNC=FULL` to override),
`foreign_keys=ON`, `busy_timeout=5000`, `temp_store=MEMORY`, `cache_size=-8000`, `journal_size_limit=67108864`,
`trusted_schema=OFF`, `PRAGMA optimize=0x10002` when a connection opens, and `query_only=ON` on readers (not on the
writer, not on the maintenance connection). `PRAGMA optimize` also runs hourly. On SIGTERM the server drains requests,
runs `wal_checkpoint(TRUNCATE)`, then exits. `/data` must be a local filesystem (WAL does not work on NFS or SMB).

```sql
CREATE TABLE settings (                 -- keys: workspace.name, workspace.logo_attachment_id, instance.id, instance.created_at
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL CHECK (json_valid(value))
) STRICT;

CREATE TABLE blobs (                    -- file: /data/files/<sha[0:2]>/<sha[2:4]>/<sha256>
  sha256     TEXT PRIMARY KEY CHECK (length(sha256) = 64 AND sha256 NOT GLOB '*[^0-9a-f]*'),
  size       INTEGER NOT NULL CHECK (size >= 0),
  mime       TEXT NOT NULL,             -- sniffed from magic bytes, never trusted from the client
  width      INTEGER,                   -- read from the image header only (imagesize)
  height     INTEGER,
  created_at INTEGER NOT NULL
) STRICT, WITHOUT ROWID;

-- ---------- lookup tables: extend with INSERT, never with a table rebuild (D37) ----------
CREATE TABLE user_roles (role TEXT PRIMARY KEY) STRICT;
INSERT INTO user_roles(role) VALUES ('owner'),('admin'),('member'),('viewer');
CREATE TABLE item_kinds (kind TEXT PRIMARY KEY) STRICT;
INSERT INTO item_kinds(kind) VALUES ('doc'),('board');
CREATE TABLE content_formats (format TEXT PRIMARY KEY) STRICT;
INSERT INTO content_formats(format) VALUES ('yjs'),('inkboard');
CREATE TABLE revision_reasons (reason TEXT PRIMARY KEY) STRICT;
INSERT INTO revision_reasons(reason) VALUES ('auto'),('manual'),('pre-restore'),('restore'),('import'),('pre-api');
CREATE TABLE link_kinds (kind TEXT PRIMARY KEY) STRICT;
INSERT INTO link_kinds(kind) VALUES ('mention'),('link'),('embed');
CREATE TABLE token_scopes (scope TEXT PRIMARY KEY) STRICT;       -- hierarchy read ⊂ write ⊂ admin lives in Rust (Scope)
INSERT INTO token_scopes(scope) VALUES ('read'),('write'),('admin');

CREATE TABLE users (
  id                   TEXT PRIMARY KEY,
  email                TEXT NOT NULL UNIQUE COLLATE NOCASE,
  name                 TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
  password_hash        TEXT,            -- PHC argon2id; NULL = SSO-only (later), or cleared in an export copy
  role                 TEXT NOT NULL REFERENCES user_roles(role),
  avatar_sha256        TEXT REFERENCES blobs(sha256),
  theme                TEXT NOT NULL DEFAULT 'system' CHECK (theme IN ('light','dark','system')),
  must_change_password INTEGER NOT NULL DEFAULT 0 CHECK (must_change_password IN (0,1)),
  created_at           INTEGER NOT NULL,
  updated_at           INTEGER NOT NULL,
  disabled_at          INTEGER
) STRICT;
CREATE UNIQUE INDEX users_single_owner ON users(role) WHERE role = 'owner';

CREATE TABLE sessions (
  id           TEXT PRIMARY KEY,
  token_hash   BLOB NOT NULL UNIQUE,    -- sha256(32 random bytes)
  user_id      TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  persistent   INTEGER NOT NULL CHECK (persistent IN (0,1)),  -- "Keep me signed in"
  created_at   INTEGER NOT NULL,
  last_seen_at INTEGER NOT NULL,        -- touched at most every 5 min
  expires_at   INTEGER NOT NULL,        -- window 30 d (persistent, capped at created_at + 180 d) or 24 h; renewal in §8
  user_agent   TEXT,
  ip           TEXT
) STRICT;
CREATE INDEX sessions_user    ON sessions(user_id);
CREATE INDEX sessions_expires ON sessions(expires_at);

CREATE TABLE api_tokens (                -- personal access tokens (D42, §7.3)
  id           TEXT PRIMARY KEY,         -- UUIDv7
  user_id      TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name         TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
  token_hash   BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),  -- sha256 of the whole 51-character token
  prefix       TEXT NOT NULL CHECK (length(prefix) = 12),             -- 'qpat_' + 7 characters, shown in lists
  created_via  TEXT NOT NULL CHECK (created_via IN ('web','api','admin')),
  parent_token_id TEXT REFERENCES api_tokens(id) ON DELETE SET NULL,  -- token that created it; revoked with it (D42)
  created_at   INTEGER NOT NULL,
  expires_at   INTEGER,                  -- NULL = never
  last_used_at INTEGER,                  -- written at most once per minute
  last_used_ip TEXT,
  revoked_at   INTEGER
) STRICT;
CREATE INDEX api_tokens_user ON api_tokens(user_id, created_at DESC);
CREATE TABLE api_token_scopes (          -- expanded: a write token has rows 'read' and 'write'
  token_id TEXT NOT NULL REFERENCES api_tokens(id) ON DELETE CASCADE,
  scope    TEXT NOT NULL REFERENCES token_scopes(scope),
  PRIMARY KEY (token_id, scope)
) STRICT, WITHOUT ROWID;

CREATE TABLE collections (
  id           TEXT PRIMARY KEY,
  url_id       TEXT NOT NULL UNIQUE,
  name         TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
  description  TEXT NOT NULL DEFAULT '' CHECK (length(description) <= 500),
  icon         TEXT,                    -- lucide icon name, or NULL = letter tile
  color        TEXT NOT NULL DEFAULT '#8D8D8D'
               CHECK (color GLOB '#[0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f]'),
  sort_key     TEXT NOT NULL,
  child_order  TEXT NOT NULL DEFAULT 'manual' CHECK (child_order IN ('manual','title','updated','created')),
  tree_version INTEGER NOT NULL DEFAULT 1,  -- bumped on any structural change in the collection → ETag of /tree
  created_by   TEXT REFERENCES users(id) ON DELETE SET NULL,
  updated_by   TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL,
  deleted_at   INTEGER,
  deleted_by   TEXT REFERENCES users(id) ON DELETE SET NULL
) STRICT;
CREATE INDEX collections_order ON collections(sort_key, id) WHERE deleted_at IS NULL;

CREATE TABLE items (
  id              TEXT PRIMARY KEY,
  url_id          TEXT NOT NULL UNIQUE,
  collection_id   TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
  parent_id       TEXT REFERENCES items(id) ON DELETE CASCADE,   -- NULL = top level of the collection
  kind            TEXT NOT NULL REFERENCES item_kinds(kind),
  title           TEXT NOT NULL DEFAULT '' CHECK (length(title) <= 1000),
  icon            TEXT,                 -- reserved; NULL in M1
  sort_key        TEXT NOT NULL,
  is_template     INTEGER NOT NULL DEFAULT 0 CHECK (is_template IN (0,1)),  -- reserved
  full_width      INTEGER NOT NULL DEFAULT 0 CHECK (full_width IN (0,1)),
  created_by      TEXT REFERENCES users(id) ON DELETE SET NULL,
  updated_by      TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at      INTEGER NOT NULL,
  updated_at      INTEGER NOT NULL,     -- metadata or content change
  content_at      INTEGER,              -- last body save; NULL = never edited ("You created …")
  published_at    INTEGER,              -- set to created_at on create; NULL = draft, only via unpublish in M1 (derived visibility, D23)
  archived_at     INTEGER,
  archive_root_id TEXT,                 -- item whose archiving archived this row
  deleted_at      INTEGER,
  deleted_by      TEXT REFERENCES users(id) ON DELETE SET NULL,
  trash_root_id   TEXT,                 -- item (= id for the root) or collection whose deletion trashed this row
  meta_version    INTEGER NOT NULL DEFAULT 1,
  CHECK (parent_id IS NULL OR parent_id <> id),
  CHECK ((deleted_at IS NULL) = (trash_root_id IS NULL)),
  CHECK ((archived_at IS NULL) = (archive_root_id IS NULL))
) STRICT;
CREATE INDEX items_tree     ON items(collection_id, parent_id, sort_key, id);
CREATE INDEX items_children ON items(parent_id, sort_key, id);
CREATE INDEX items_trash    ON items(trash_root_id) WHERE deleted_at IS NOT NULL;
CREATE INDEX items_archive  ON items(archive_root_id) WHERE archived_at IS NOT NULL;
CREATE INDEX items_updated  ON items(updated_at DESC) WHERE deleted_at IS NULL;
CREATE INDEX items_drafts   ON items(created_by, updated_at DESC) WHERE published_at IS NULL AND deleted_at IS NULL;

-- Structural guards (defence in depth; Rust checks the same rules first). Cycles are checked in the writer transaction
-- with a recursive CTE: SQLite documents CTEs inside triggers as unsupported.
CREATE TRIGGER items_ins_guard BEFORE INSERT ON items
BEGIN
  SELECT RAISE(ABORT, 'parent_invalid') WHERE NOT EXISTS (
    SELECT 1 FROM collections c WHERE c.id = NEW.collection_id AND c.deleted_at IS NULL);
  SELECT RAISE(ABORT, 'parent_invalid') WHERE NEW.parent_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM items p WHERE p.id = NEW.parent_id AND p.collection_id = NEW.collection_id
      AND p.deleted_at IS NULL AND p.archived_at IS NULL);
END;
-- A new parent must be live, unarchived and in the row's (new) collection.
CREATE TRIGGER items_parent_upd BEFORE UPDATE OF parent_id ON items
WHEN NEW.parent_id IS NOT NULL AND NEW.parent_id IS NOT OLD.parent_id
BEGIN
  SELECT RAISE(ABORT, 'parent_invalid') WHERE NOT EXISTS (
    SELECT 1 FROM items p WHERE p.id = NEW.parent_id AND p.collection_id = NEW.collection_id
      AND p.deleted_at IS NULL AND p.archived_at IS NULL);
END;
-- A collection change needs a live target collection and, when the parent stays, a parent already in that collection
-- (so subtrees move top-down, §5 rules). Liveness of an unchanged parent is not checked: trashed or archived
-- descendants move with their subtree.
CREATE TRIGGER items_collection_upd BEFORE UPDATE OF collection_id ON items
WHEN NEW.collection_id IS NOT OLD.collection_id
BEGIN
  SELECT RAISE(ABORT, 'parent_invalid') WHERE NOT EXISTS (
    SELECT 1 FROM collections c WHERE c.id = NEW.collection_id AND c.deleted_at IS NULL);
  SELECT RAISE(ABORT, 'parent_invalid') WHERE NEW.parent_id IS NOT NULL AND NEW.parent_id IS OLD.parent_id
    AND NOT EXISTS (SELECT 1 FROM items p WHERE p.id = NEW.parent_id AND p.collection_id = NEW.collection_id);
END;

-- ---------- documents (Yjs) ----------
CREATE TABLE doc_state (
  item_id        TEXT PRIMARY KEY REFERENCES items(id) ON DELETE CASCADE,
  ydoc           BLOB NOT NULL,         -- compacted Yjs v1 update of the whole doc; new doc = encodeStateAsUpdate(empty)
  yfield         TEXT NOT NULL DEFAULT 'default',
  schema_version INTEGER NOT NULL DEFAULT 1,   -- Quire TipTap schema version (§6.1)
  compacted_seq  INTEGER NOT NULL DEFAULT 0,
  next_seq       INTEGER NOT NULL DEFAULT 1,
  pm_json        TEXT CHECK (pm_json IS NULL OR json_valid(pm_json)),  -- cache sent by the client; rebuildable
  pm_json_seq    INTEGER,
  last_token_id  TEXT,                  -- token of the last content write; NULL = session, seed or admin (D53)
  updated_at     INTEGER NOT NULL
) STRICT;
CREATE TABLE doc_updates (               -- append-only; the future WebSocket writes the same log
  id         INTEGER PRIMARY KEY,
  item_id    TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  seq        INTEGER NOT NULL,
  data       BLOB NOT NULL,              -- one validated Yjs v1 update
  user_id    TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL,
  UNIQUE (item_id, seq)
) STRICT;
CREATE TABLE doc_clients (               -- Yjs clientID → user; survives compaction (future attribution)
  item_id       TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  client_id     INTEGER NOT NULL,
  user_id       TEXT REFERENCES users(id) ON DELETE SET NULL,
  first_seen_at INTEGER NOT NULL,
  PRIMARY KEY (item_id, client_id)
) STRICT, WITHOUT ROWID;

-- ---------- boards (Inkboard) ----------
CREATE TABLE board_state (
  item_id        TEXT PRIMARY KEY REFERENCES items(id) ON DELETE CASCADE,
  body           BLOB NOT NULL,          -- deflate(envelope JSON as received, every document.assets[*].data = "")
  body_sha256    BLOB NOT NULL,          -- sha256 of the uncompressed stripped envelope → skip no-op saves
  raw_len        INTEGER NOT NULL,
  format_version INTEGER NOT NULL,       -- envelope "version" (2)
  engine_version TEXT NOT NULL,          -- X-Quire-Engine of the writer, e.g. "0.2.0-beta.3+embed.quire.1"; "server" for seeds
  rev            INTEGER NOT NULL DEFAULT 1,   -- ETag "r{rev}"
  seq            INTEGER NOT NULL DEFAULT 0,   -- Sequencer seq (server-owned live sync, later)
  meta           TEXT NOT NULL CHECK (json_valid(meta)),  -- {"pages":[{"id","name"}],"objects":n,"background":"#rrggbb","bytes":n,"text_excerpt":"…"}
  last_token_id  TEXT,                   -- as in doc_state (D53)
  updated_at     INTEGER NOT NULL
) STRICT;
CREATE TABLE board_assets (              -- the AssetInfo manifest of inkboard-session
  item_id  TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  asset_id TEXT NOT NULL,
  sha256   TEXT NOT NULL REFERENCES blobs(sha256),
  mime     TEXT NOT NULL,
  PRIMARY KEY (item_id, asset_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX board_assets_blob ON board_assets(sha256);
CREATE TABLE board_previews (
  item_id    TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  page_index INTEGER NOT NULL,
  rev        INTEGER NOT NULL,           -- stale when <> board_state.rev
  svg        BLOB,                       -- deflate(svg); NULL = empty page
  svg_len    INTEGER NOT NULL,
  PRIMARY KEY (item_id, page_index)
) STRICT, WITHOUT ROWID;

-- ---------- history (written in M1, UI in M2) ----------
CREATE TABLE revisions (
  id             TEXT PRIMARY KEY,
  item_id        TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  format         TEXT NOT NULL REFERENCES content_formats(format),
  format_version INTEGER NOT NULL,       -- yjs: update encoding (1); inkboard: envelope version (2)
  data           BLOB NOT NULL,          -- yjs: full encoded state; inkboard: deflated stripped envelope
  preview        TEXT,                   -- yjs: pm_json at that time (display only; restore uses data)
  assets         TEXT CHECK (assets IS NULL OR json_valid(assets)),                  -- inkboard: {"asset_id":"sha256"} (blob GC roots)
  attachment_ids TEXT CHECK (attachment_ids IS NULL OR json_valid(attachment_ids)),  -- yjs: ["attachment id",…] (attachment GC roots)
  schema_version INTEGER NOT NULL DEFAULT 1,
  title          TEXT NOT NULL,
  reason         TEXT NOT NULL REFERENCES revision_reasons(reason),
  source_seq     INTEGER,                -- yjs: the seq it captures; inkboard: the rev (D53)
  label          TEXT,
  author_id      TEXT REFERENCES users(id) ON DELETE SET NULL,
  byte_size      INTEGER NOT NULL,
  created_at     INTEGER NOT NULL
) STRICT;
CREATE INDEX revisions_item ON revisions(item_id, created_at DESC);

-- ---------- attachments (doc images, workspace logo) ----------
CREATE TABLE attachments (
  id         TEXT PRIMARY KEY,           -- URL /api/v1/attachments/{id}
  item_id    TEXT REFERENCES items(id) ON DELETE SET NULL,  -- upload context only; never used for authorization (D36)
  sha256     TEXT NOT NULL REFERENCES blobs(sha256),
  filename   TEXT NOT NULL,              -- sanitized at upload (§8)
  mime       TEXT NOT NULL,
  created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL
) STRICT;
CREATE INDEX attachments_blob ON attachments(sha256);
CREATE TABLE item_attachments (          -- rebuilt from the ytext walk in every doc save txn; copied by duplicate
  item_id       TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  attachment_id TEXT NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
  PRIMARY KEY (item_id, attachment_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX item_attachments_att ON item_attachments(attachment_id);

-- ---------- links (maintained on every doc save; backlinks UI later) ----------
CREATE TABLE item_links (
  source_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  target_id TEXT NOT NULL,               -- not an FK: a purged target renders as "Deleted item"
  kind      TEXT NOT NULL REFERENCES link_kinds(kind),
  PRIMARY KEY (source_id, target_id, kind)
) STRICT, WITHOUT ROWID;
CREATE INDEX item_links_target ON item_links(target_id);

-- ---------- per user ----------
CREATE TABLE stars (
  user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  item_id    TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  sort_key   TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (user_id, item_id)
) STRICT, WITHOUT ROWID;
CREATE TABLE item_views (
  user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  item_id         TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  view_count      INTEGER NOT NULL DEFAULT 1,
  first_viewed_at INTEGER NOT NULL,
  last_viewed_at  INTEGER NOT NULL,
  PRIMARY KEY (user_id, item_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX item_views_recent ON item_views(user_id, last_viewed_at DESC);
CREATE INDEX item_views_item   ON item_views(item_id);

-- ---------- full-text search ----------
CREATE TABLE item_text (
  text_id INTEGER PRIMARY KEY,           -- explicit, so VACUUM cannot renumber rowids
  item_id TEXT NOT NULL UNIQUE REFERENCES items(id) ON DELETE CASCADE,
  title   TEXT NOT NULL,
  body    TEXT NOT NULL                  -- doc: text derived from Y; board: page names, labels, table cells, latex
) STRICT;
CREATE VIRTUAL TABLE item_fts USING fts5(
  title, body, content = 'item_text', content_rowid = 'text_id',
  tokenize = 'unicode61 remove_diacritics 2', prefix = '2 3');
CREATE TRIGGER item_text_ai AFTER INSERT ON item_text BEGIN
  INSERT INTO item_fts(rowid, title, body) VALUES (NEW.text_id, NEW.title, NEW.body);
END;
CREATE TRIGGER item_text_ad AFTER DELETE ON item_text BEGIN
  INSERT INTO item_fts(item_fts, rowid, title, body) VALUES ('delete', OLD.text_id, OLD.title, OLD.body);
END;
CREATE TRIGGER item_text_au AFTER UPDATE ON item_text BEGIN
  INSERT INTO item_fts(item_fts, rowid, title, body) VALUES ('delete', OLD.text_id, OLD.title, OLD.body);
  INSERT INTO item_fts(rowid, title, body) VALUES (NEW.text_id, NEW.title, NEW.body);
END;
CREATE TRIGGER items_text_ins AFTER INSERT ON items BEGIN
  INSERT INTO item_text(item_id, title, body) VALUES (NEW.id, NEW.title, '');
END;
CREATE TRIGGER items_text_title AFTER UPDATE OF title ON items WHEN NEW.title IS NOT OLD.title BEGIN
  UPDATE item_text SET title = NEW.title WHERE item_id = NEW.id;
END;
CREATE TRIGGER items_text_del BEFORE DELETE ON items BEGIN
  DELETE FROM item_text WHERE item_id = OLD.id;
END;
```

Rules for the data model:
- **Visibility.** Every list, tree, sidebar, search and backlink query selects items through
  `shared::visible_items(viewer)`, a SQL fragment that in M1 means "live item in a live collection"
  (`c.deleted_at IS NULL AND i.deleted_at IS NULL`). Multi-user later changes only this helper (§15). The tree and
  default search also filter on `i.archived_at IS NULL`.
- **Effective publication.** `shared::effectively_published(item)`: the item and every ancestor have `published_at`.
  M1 uses it only in tests and in the ItemDetail hint "Hidden until the parent is published" (not shown in single-user).
- **Tree queries** are recursive CTEs with `UNION`, not `UNION ALL` (they terminate even on a corrupt cycle):
  - breadcrumbs: walk ancestors, root first;
  - cycle check: inside the writer transaction, reject with 409 `cycle` when the target parent's ancestor chain contains
    the moving item;
  - cross-collection move: (1) update the root's `parent_id`, `collection_id` and `sort_key`; (2) update the
    descendants' `collection_id` **one depth level at a time, top-down** (e.g. `… WHERE id IN (SELECT value FROM
    json_each(?))` per level), so each row's parent already carries the new collection (trigger
    `items_collection_upd`); (3) bump `tree_version` on both collections.
- **Trash.** Set `deleted_at`, `deleted_by` and `trash_root_id = :root` on every descendant that is not already trashed.
  Restore clears the rows `WHERE trash_root_id = :root`. If the old parent is gone (purged, trashed, or archived while
  the root comes back unarchived), the subtree is re-parented to the top of its collection (`moved_to_top: true`). The
  same applies to archive, with `archive_root_id`. A restored root that is still archived as part of an ancestor's
  archive unit becomes the archive root of its own part of that unit when it goes to the top (one `UPDATE … WHERE
  archive_root_id = :old AND id IN subtree(:root)`, which also bumps their `meta_version`): it shows in `/archive`,
  unarchives by itself, and survives a purge of the old root. Every trash or archive root is therefore the row itself
  or an ancestor, with every row between them in the same unit (`quire_core::tree::Model::check_invariants`).
  - Restore and unarchive act on the unit: they clear the item's trash or archive root, which may be an ancestor, and
    return that root (`RestoredItem`, with `moved_to_top`). Restoring a live item or unarchiving an unarchived one is
    409 `conflict`; unarchiving a trashed item is 409 `read_only`.
  - Trashing an archived item is allowed and keeps its archive state; trashing a trashed one is 409 `read_only`.
    Archive skips trashed descendants. Publish and unpublish of a trashed or archived item are 409 `read_only`.
  - Purge (`DELETE /trash/items/{id}`) takes any trashed item, not only a root, with its subtree; a live item is 409
    `conflict`. `DELETE /trash` purges every item root and every deleted collection.
- **Collection delete.** Soft-deletes the collection and sets `deleted_at` and `trash_root_id = <collection id>` on its
  live items. Restoring the collection clears exactly those rows. Items trashed earlier keep their own roots. While a
  collection is deleted, restoring one of its items, or creating or moving items into it, fails with 409
  `parent_invalid` ("Restore the collection first"). A deleted collection exists only in Trash: its `GET`, `PATCH`,
  `move`, `DELETE`, `…/tree` and `…/updated` answer 404, restoring a live collection or purging one is 409
  `conflict`, and its `item_count` is the number of items its deletion trashed (what a restore brings back).
- **Purge** deletes leaves first, one DELETE per depth level, so the FK cascade depth limit (1000) never matters. A
  collection purge deletes its items leaves first, then the collection row. Trash roots (items and collections) older
  than 30 days are purged by the job (§12).
- **Sort keys.** When a sibling's key grows past 64 characters, the whole sibling group is rebalanced in one
  transaction (a collection rebalance bumps `tree_version` of every collection whose key it rewrites). Non-manual sorts
  are computed at read time; `sort_key` still applies to `manual`. Tree order: parents before children, depth first;
  siblings by `child_order`: `manual` by `sort_key`, `title` A–Z by `quire_core::tree::title_key` (case- and
  accent-insensitive, numbers by value; §9.6.2, the web client's order too), `updated` and `created` newest first;
  ties go to `sort_key`, then the id. `…/children` uses the same order.
- **Versions.** `meta_version` goes up on every change to the item row except a content save: title, `full_width`,
  move (the root and, across collections, its descendants), publication, archive, trash and their reverses, collection
  delete and restore. `updated_at`/`updated_by` change on rename, `full_width`, move (the root) and publication; not on
  archive, trash, restore or a rebalance. `tree_version` goes up on every create, move, rename, `full_width` change,
  publication change, archive, unarchive, trash, restore, discard, duplicate and rebalance, on any collection edit or
  reorder, and on collection delete and restore; `touch_content` bumps it only when the collection sorts by `updated`
  (so under the other orders a 304 tree may show an older `updated_at`, which is not structural, D21).
- **Search query** (built in Rust, `quire_store::search::build_match`; quire-core's module list has no `search`):
  - tokens are split on whitespace; `"…"` keeps a phrase; every positive token or phrase is double-quoted (embedded `"`
    doubled), and the last positive token gets `*`;
  - `-word` or `-"phrase"` is a negative term;
  - output: `(<positives joined by AND>)` plus, if there are negatives, ` NOT (<negatives joined by OR>)`. FTS5 accepts
    `a NOT b` but rejects `a AND NOT b` and a leading `NOT` (checked on SQLite 3.46.1);
  - a query with no positive term returns no results (no FTS call); terms without a letter or digit are dropped (FTS5
    would make empty phrases of them), and only a bare last word gets `*`, never a phrase;
  - snippets come from the body (24 tokens, `…`) and use the markers U+E000 and U+E001; the client splits on them and
    renders text nodes, never HTML. `score` is `-bm25`, so higher is better;
  - `/search/titles` uses the column filter `title : (…)`; a blank `q` lists the recently updated items.
- **Body text** is updated inside the same save transaction:
  `UPDATE item_text SET body=?2 WHERE item_id=?1 AND body IS NOT ?2`.
- **Revision policy.** On a content save, if the newest revision of the item is more than 10 minutes old (or there is
  none), write an `auto` revision of the new state (Yjs revisions with `attachment_ids`, board revisions with `assets`).
  A daily prune keeps everything from the last 24 h, then one per hour for 7 days, one per day for 90 days, then one per
  week: the buckets are fixed UTC hours, days and weeks, and the newest revision in each bucket stays. `manual`
  revisions are never pruned. `maybe_auto` compares with the newest revision of any reason.
- **Attachment GC** (in the 6-hourly job, before blob GC) deletes `attachments` rows older than 24 h that are not in
  `item_attachments`, not in any `json_each(revisions.attachment_ids)`, and not the `workspace.logo_attachment_id`
  setting.
- **Blob GC** (after attachment GC) deletes every blob that is not referenced by `attachments`, `board_assets`,
  `users.avatar_sha256` or `json_each(revisions.assets)`, as one writer closure (D26), then sweeps orphan files.
- **`quire admin check`** runs `quick_check`, `foreign_key_check`, a bounded cycle scan, the FTS `integrity-check`, and
  reports blob rows whose file is missing.
- **Previews** (`board_previews`, §12). A render stores one row per rendered page at one rev, in one writer
  transaction, and removes the rows after them. The rendering is bounded (W6): at most `PAGE_SVG_MAX` (16 MiB) per page
  and `BOARD_SVG_MAX` (48 MiB) per board, estimated from the model before rendering and counted with the real lengths.
  When a page does not fit what is left, its largest images are drawn as grey placeholders, largest first; when the
  drawing without images does not fit, the page is `card_svg` ("Too large to preview", under 1 KB). Once the board's
  budget is spent the remaining pages get no row: with a page-0 row at that rev they are *omitted*, and their page
  route serves the card. `BoardMeta.preview_rev` is the newest rev among the stored rows, and `svg_len` each page's
  length at that rev, or `null` (also for an omitted page).
- **Tokens.** A token is live when `revoked_at IS NULL AND (expires_at IS NULL OR expires_at > now)` and its user has no
  `disabled_at`. Creation refuses a 101st live token per user (422 `validation`, `details.field = "tokens"`). Creation
  trims the name, then checks in this order: another user's parent (403 `forbidden`), `admin` from a token (403
  `session_required`), a scope above the parent's (422, `field: "scopes"`), a later expiry than the parent's, where
  never counts as later (422, `field: "expires_in_days"`). `tokens::list` takes every live token, then the newest
  expired ones, up to 100 in all, and returns them newest first, so 100 newer expired tokens never hide a live one.
  Revoked and expired rows are deleted 30 days after `revoked_at`/`expires_at` by the hourly job (§12). Only live tokens authenticate. A lookup reads the row
  by `token_hash` on a reader connection, with no liveness filter and no cache, then classifies it as live, expired,
  revoked or disabled (§7.3). A revoke also revokes, in the same statement, the live tokens whose `parent_token_id` is
  the revoked one (D42).
- **Recovery points** (D53). `revisions::pre_api` runs before a token's content write: it writes a `pre-api` revision of
  the current state (with `source_seq`) unless `last_token_id` of the doc or board state is that token, or the newest
  revision's `source_seq` equals the current seq (doc) or rev (board). Every content write sets `last_token_id` (NULL
  for session, seed and admin writes); `auto` revisions set `source_seq` too.
- **Document ETag.** `"d{next_seq - 1}"` from `doc_state`: 0 for a new document, bumped by every stored update and never by
  compaction (D45).

---

## 6. Content storage formats

### 6.1 Documents: Yjs

- **Client.**
  - Create a `Y.Doc`, apply the server state with origin `'server'`, and only then create the editor.
  - Configure the editor with `Collaboration.configure({ document, field: 'default' })` and StarterKit with
    `{ undoRedo: false }`. Undo is handled by the Y.UndoManager inside Collaboration.
  - `UniqueID.configure({ types: [<every block type>], attributeName: 'blockId', filterTransaction: tr =>
    !isChangeOrigin(tr) })` (`isChangeOrigin` from `@tiptap/extension-collaboration`), so ids arriving from the server
    or another tab are never reassigned.
  - Never pass `content:` to the editor.
  - Fixture mode (`WRITE_FIXTURES=1`): every `Y.Doc` gets a fixed `clientID` and UniqueID's `generateID` is a seeded
    counter, so `fixtures/ydoc/*.bin` are byte-stable.
- **Schema v1.** These node and mark names are a contract, because the server's walker depends on them. Renaming one
  requires `schema_version` 2 and a Y-level migration.
  - Nodes: `paragraph`, `heading{level}`, `bulletList`, `orderedList`, `listItem`, `taskList`, `taskItem{checked}`,
    `blockquote`, `codeBlock{language}`, `horizontalRule`, `hardBreak`, `table`, `tableRow`, `tableHeader`,
    `tableCell`, `image{src,attachmentId,width,alt,caption}`, `callout{variant: info|tip|warning|danger}`,
    `mention{itemId}`, `boardEmbed{itemId}`.
  - Marks: `bold`, `italic`, `underline`, `strike`, `code`, `highlight`, `link{href,itemId?}`.
  - Every block node carries a `blockId` attribute (UniqueID). Heading anchors are `#h-<blockId>`. The Contents column
    is computed from `heading` nodes (no TableOfContents extension).
  - Pasted or dropped HTML images whose `src` is not same-origin keep `src` with `attachmentId: null` and render as a
    placeholder card ("External image, not loaded" + "Open link"); the CSP never loads them (§11).
- **Server, `quire-core::ytext`.**
  - The walk over `XmlFragment("default")` (an explicit stack; more than 5000 levels are refused) produces:
    - plain text, one line per paragraph, heading and code block; a hard break ends a line; an image caption is a
      line; a table row is one line with `\t` between its cells (a cell's own lines joined by a space); every other
      element is a container; empty lines are dropped; mentions contribute no text;
    - links: `mention.itemId` → `mention`, `boardEmbed.itemId` → `embed`, `link.itemId` → `link`;
    - the image `attachmentId`s;
    - `is_empty`: true when the fragment holds only empty paragraphs (y-tiptap writes one empty paragraph into
      an empty fragment when the editor starts, so empty bytes are not the test).
  - Golden fixtures in `fixtures/ydoc/*.bin` are produced by a headless TipTap test in `web/` and checked against
    `*.txt` and `*.links.json` (`{links: [{target_id, kind}], attachment_ids, is_empty}`, computed by the web writer
    independently of Rust) by `cargo test -p quire-core ytext`.
- **Save (`POST …/doc/updates`, under a `doc_work` permit, taken after the header checks and before the body is
  read; the JSON and base64 work runs in `spawn_blocking`).**
  1. `Update::decode_v1`; an update that does not decode gets 422 `invalid_update`.
  2. Load `ydoc` plus the uncompacted updates and apply the new one in one `TransactionMut`.
  3. If yrs reports a pending update or pending delete set (missing structs), answer 409 `missing_deps` with
     `{state_vector}`.
  4. If the transaction left the state vector unchanged **and** its delete set is empty (`before_state == after_state
     && delete_set.is_empty()`), answer 200 with the current seq and write nothing. A delete-only update moves no
     clock, so it is not a no-op; a re-sent duplicate (insert or deletion) adds nothing and is.
  5. Otherwise append the update to `doc_updates`; update `item_text.body`, `item_links` and `item_attachments` (ids
     that exist in `attachments` only); `INSERT OR IGNORE` a `doc_clients` row for every client id in the update's
     state vector with the actor; set `items.content_at`, `updated_at` and `updated_by` and `doc_state.last_token_id`;
     store `pm_json` if it was sent. A token's POST first runs `revisions::pre_api` (D53). `pm_json` is kept as the
     JSON text it was sent as, never parsed into a tree; it must be a JSON object (otherwise 422 `validation`,
     `field: "pm_json"`), `null` counts as absent, and it is not checked against schema v1 (it is the editor's cache
     and may hold a newer client's nodes). A JSON body nested beyond serde_json's 128 levels is 422 `invalid_document`
     at `/doc`.
  6. Compact when there are more than 200 rows or more than 1 MiB: encode the full state into `ydoc`, delete the folded
     rows, and set `compacted_seq`. This happens in the same writer transaction.
  7. If the encoded state is larger than 16 MiB, answer 413 `doc_too_large`.

  API writes in Markdown or ProseMirror JSON (§6.4) produce one Yjs update on the server and enter this path at step 2
  through a variant that takes no permit (the PUT already holds two, D25), so validation, the no-op rule, derived text, `doc_clients`, compaction and the size cap are the same for every
  writer.
- **Title.** `items.title` is plain text and the source of truth. A live title later uses a `Y.Text('title')` that the
  **server** seeds once from `items.title` when it first creates the room (two clients seeding it would duplicate it).
- **Restore** (M2) runs on the client from `revisions.data`: decode into a scratch `Y.Doc`, convert with y-tiptap's
  `yXmlFragmentToProsemirrorJSON`, then `setContent`, which produces normal Y operations. `preview` is display only.

### 6.2 Boards: the Inkboard envelope

`quire-core::board` (every function returns `Result<_, BoardError>`, which maps to 422 `invalid_board` or
`board_too_new`):
- `split(body: &str) -> Split { stripped: String, assets: Vec<SplitAsset{id, mime, bytes}>, format_version }`
  - Parses `{format, version, document: &RawValue}`. `document` is parsed as an ordered map of `&RawValue`; the
    edits are made on the byte spans the raw values borrow from the input, so nothing is re-serialized and no `Value`
    tree is built. A leading BOM is dropped (as `from_json` does), so stored and returned bodies have none.
  - Rejects `format` other than `inkboard` or `tableau-blanc` with 422 `invalid_board`, and
    `version > inkboard_core::serial::VERSION` with 422 `board_too_new`.
  - Base64-decodes every `assets[*].data` and re-emits the JSON byte-for-byte, except that each asset's `data` becomes
    `""`. Asset entries are kept, so `sanitize` never drops their images.
- `splice(stripped: &str, assets: &HashMap<asset_id, bytes>, title: &str) -> Result<String>`
  - Re-inserts base64 data and replaces `document.name` with the title (D31), inserting a `name` when the document has
    none. The stored body keeps `document.name` as it was sent (the no-op test of step 4 therefore also compares the
    sent name).
  - Assets with no stored bytes are left as `""`. Validation never calls `from_json` on a stripped body; previews splice
    first.
- `inspect(full: &str) -> Result<Inspect { text, meta }>`
  - Calls `inkboard_core::serial::from_json` (validation; an error gives 422 `invalid_board`). A table without rows or
    columns, or whose cells are not rows × columns, is refused too (422 `invalid_board`): the engine never writes one,
    and the SVG export would index it out of bounds.
  - Text, one entry per line: the page name, then per object the box label (`to_plain()`), the connector label, the
    table rows (cells separated by `\t`) and the image `latex`; empty entries are skipped.
  - Meta (`Meta`/`MetaPage`, re-exported as `quire_store::boards::{Meta, MetaPage}`): pages `[{id, name}]`, object
    count, background hex, bytes, and `text_excerpt`, the first 4096 bytes of the text cut at a character boundary.
- `preview(full: &str, page: usize) -> Result<Option<String>>`: `from_json`, then `svg::export_approx(&doc,
  &doc.pages[page])`; a missing page is `None`. `previews(full)` renders every page from one parse and stops when the
  board's budget is spent; `Previewer` renders page by page within `PAGE_SVG_MAX` and `BOARD_SVG_MAX`, and `card_svg`
  draws the "Too large to preview" card (§5 "Previews").

Save (`PUT …/board`, If-Match required, otherwise 428 `precondition_required`; `If-Match: *` skips the rev check, D45).
The web client and API clients use this same pipeline (D51):
1. Before the body is read, in this order: `If-Match` (428), `Content-Type: application/json` (415), `X-Quire-Engine`
   when sent (1–128 printable ASCII characters, else 422 `validation`, `field: "X-Quire-Engine"`), `Content-Length`
   against the limit, `QUIRE_MAX_BOARD_MB` × 1 MiB (413 with `details.limit_bytes`), then the item (404; a document is
   422 `validation`, `field: "kind"`; trashed or archived, 409 `read_only`).
2. Acquire the `board_work` permit (1), then read the body within the limit and the 120 s body deadline (413 also for
   a streamed body without `Content-Length`; 408 at the deadline). Taking the permit first keeps one board body in
   memory at a time, about 3× its size (D25); a slow upload delays other board work, at most for its deadline.
3. In `spawn_blocking`: UTF-8 (422 `invalid_board`), a non-recursive pre-scan of the JSON depth (more than 100 levels
   is 422 `invalid_board`, "nests JSON N levels deep"), then `inspect` the full body, `split` it, check free space
   (D33, 507; `boards::prepare_with` takes the check from the route, which passes the server's injectable probe
   `AppState::free_space`, so tests can fake a full disk) and `BlobStore::stage` each asset (outside the writer).
4. If sha256(stripped) equals `body_sha256`, and every asset sent with data has the same sha256 as the board's
   `board_assets` row for that id and that blob's file exists, drop the staged files and return the current `{rev}`;
   this is a no-op. It runs before the `If-Match` check, so resending the stored body with a stale tag answers 200
   with the current rev. New bytes under an old asset id, data for an asset stored without any, and a lost blob file
   sent again are saved (`BlobStore::commit` restores the file).
5. Otherwise, in the writer transaction:
   - check `rev` against If-Match (a tag list; `W/` is ignored, and a tag that names no board revision, such as
     `"d3"`, never matches); a mismatch gives 412 `rev_mismatch` with `{rev, etag}`;
   - for a token-authenticated save, `revisions::pre_api` (D53); set `last_token_id` (NULL for a session);
   - `BlobStore::commit` each staged asset;
   - store `deflate(stripped)` and set `rev + 1`;
   - replace the `board_assets` rows (an asset sent with `data: ""` keeps the blob the board already had under that
     id; one without such a blob gets no row, and GET returns its `""`);
   - update `meta`, `item_text.body`, `items.content_at`, `updated_at` and `updated_by`;
   - apply the revision policy.
6. Send the preview job (§12).

Other operations:
- **Load (`GET`, under the `board_work` permit, after its share of the body budget when it is 1 MiB or more, D25):**
  inflate, splice with the blob bytes and the title, return the envelope with `ETag: "r{rev}"` and `Cache-Control:
  no-store`. It never answers 304 and takes no `If-None-Match`: the body holds the live title (`document.name`, D31),
  which `"r{rev}"` does not version (D45); the tag is for `If-Match`. A `.inkboard` download is the same bytes with
  `Content-Disposition: attachment; filename="<title>.inkboard"` (the §8 filename rule; `board` when the title is
  empty). The preview worker and the page route render under the same permit (§12).
- **Create:** a new board's envelope is `serial::to_json(&Document::new(""), false)`, with `background.preset` set to
  `white`, or to `slate` when the owner's theme is dark. It gets `engine_version = "server"` and rev 1. The server calls
  `inkboard_core::model::seed_ids(<random u64>)` once at startup, so ids it creates never repeat across restarts.
- **Writer version:** the web client sends `X-Quire-Engine: <engineVersion>` (§10.4). The server stores it, so a future
  engine-format bump can be found per board. API clients may omit it; the server then stores `"api"`.

### 6.3 Forward compatibility of formats

- **Docs.** A live WebSocket (y-protocols sync and awareness, `yrs::sync`) persists to the same `doc_updates` table and
  `doc_clients`. No conversion is needed.
- **Boards.** One actor per open board runs `inkboard_session::Sequencer::new(spliced doc, cap)` with
  `set_budgets(…)` (exists, `sequencer.rs:93`). It appends `board_ops(item_id, seq, author_id, ops_json, created_at)` (a
  new table) and compacts back into `board_state.body`, advancing `seq`. Resuming at a stored `seq` needs an upstream
  `Sequencer::with_seq` (`new` starts at 0, `sequencer.rs:78`). `board_assets` already is the hash manifest the session
  protocol uses. The M1 rows stay as they are.
- **API writes and live sync.** An API document write is already a Yjs update from its own client id (§6.4), so a later
  room broadcasts it like any peer's update. An API board PUT stays a whole-board replace guarded by `If-Match`; with a
  server-owned Sequencer it becomes a diff against the room's document (same idea as §6.4), which needs no format change.

### 6.4 Document conversion: ProseMirror JSON and Markdown (`quire-core::convert`, `quire-core::schema`)

Used by `GET`/`PUT /api/v1/items/{id}/doc/{markdown,prosemirror}` (§7.5) and by the web "Export Markdown" (D49).

**Schema mirror.** `quire-core::schema` holds schema v1 as data: every node (name, group, content expression, `inline`,
`atom`, attrs with type and default) and every mark (name, attrs, `excludes`, rank). The web fixture writer dumps the
live TipTap schema (`editor.schema`, with each attr's default) to `fixtures/schema/v1.json`, and
`schema::tests::matches_tiptap_fixture` compares it with the Rust table, so attrs TipTap adds beyond the §6.1 list (for
example `orderedList.start`, table cells' `colspan`, `rowspan`, `colwidth`) are covered.
`schema::tests::every_mark_excludes_itself` asserts that no v1 mark allows overlapping copies of itself (y-tiptap would
then store `<name>--<hash>` keys). `GET /api/v1/schema/doc` serves the same JSON for API clients.

**Y ↔ ProseMirror JSON** (`convert::pm`; lossless for schema v1). The reference is `@tiptap/y-tiptap` 3.0.10 (MIT), the
binding the editor uses; `convert::pm` ports `yXmlFragmentToProsemirrorJSON`, `createTypeFromElementNode` /
`createTypeFromTextNodes` and `yattr2markname` from it, with a header comment and a `THIRD_PARTY.md` entry. Two shapes
exist:
- **raw** (`pm::raw`, what y-tiptap's `yXmlFragmentToProsemirrorJSON` returns): only the stored attrs, marks in schema
  rank order (unknown marks last, by name). yrs reports a text chunk's attributes as a hash map, so the Yjs order is
  not available; rank order is the order y-tiptap writes them in (one insert per text run, with all its marks), so
  every web fixture matches. Yjs itself would list a mark added later to existing text last;
- **normalized** (`schema::normalize(raw)`, what `editor.getJSON()` returns): every attr present with its default, marks
  sorted by schema rank. `DocJson.doc` and the JSON PUT body are always normalized.

Mapping rules:
- The doc's content is `XmlFragment("default")`. Each non-text node is an `XmlElement` whose tag is the node name. Each
  attribute whose value is not `null` is an element attribute holding the JSON value; `null` attrs are not stored and
  read back as the schema default.
- Consecutive text nodes inside a block are one `XmlText`. Its delta chunks carry the marks as attributes
  `{<mark name>: <mark attrs object, {} when the mark has none>}`; a `--<hash>` suffix on a key is stripped on read, as
  y-tiptap's `yattr2markname` does.
- yrs 0.28 has every call this needs: reading with `XmlFragment::children`, `Xml::attributes` and `XmlTextRef::diff`
  (formatted chunks); writing with `XmlFragment::insert`/`remove_range`, `Xml::insert_attribute`/`remove_attribute`,
  `Text::insert_with_attributes`, `Text::format` and `Text::remove_range`.
- Text offsets are UTF-16 code units, as in Yjs: every server-side `Doc` used for conversion is created with
  `OffsetKind::Utf16`.
- Nodes or marks unknown to schema v1 in a stored document (written by a newer client) pass through to JSON GET
  unchanged and are dropped from Markdown GET with a warning. A PUT to such a document is refused (409
  `schema_too_new`), so a v1 server never deletes them.
- No walk can overflow a worker thread's stack: the Y readers and the JSON normalize and validate steps are iterative
  (explicit stack; a Y walk refuses more than 5000 levels); the Markdown printer recurses up to 200 container levels
  and prints deeper content as the text of its blocks; the parser's block conversion recurses only within the 100
  containers `md_in::check_depth` allows, and its inline conversion is iterative.
- Golden files in both directions: the web fixture writer writes `fixtures/ydoc/*.y.json` (raw) and `*.pm.json`
  (normalized, `editor.getJSON()`), which `convert::tests::y_to_raw_matches_web` and `y_to_pm_matches_web` must
  reproduce exactly. In the other direction `convert::tests::write_server_fixtures` (`WRITE_FIXTURES=1`) converts
  `fixtures/markdown/*.md` into `fixtures/ydoc/server/*.bin` plus normalized `*.pm.json` (it rewrites a `.bin` only
  when its document or state vector changed, and its check compares content: see "determinism" below), and the web test
  `editor/serverWritten.test.ts` opens each `.bin` in a headless TipTap editor and checks that `editor.getJSON()` equals
  the `.pm.json` with no schema repair.

**Writing** (`convert::ydiff`, `docs::write_content`; D50). A PUT in either format runs in `spawn_blocking` holding
**two** of the three `doc_work` permits (`acquire_many(2)`, taken once, after the header checks (415, 428) and before
the body is read, so a request that waits for permits holds no body; nothing below acquires another; the JSON, UTF-8
and sha256 work runs in `spawn_blocking`), so at most one parse result and its base exist at a time, beside at most
one other loaded doc (D25). A slow sender can hold the permits, but only for the 30 s request timeout, and a 413 comes
after the wait for permits:
1. **Parse.** Markdown: a per-line pre-scan bounds container nesting (`>` and list markers, by `markers + indentation
   columns / 2` of the line's container prefix) and refuses a bound above 200 before comrak runs; comrak's tree is then
   refused when containers (quotes, alerts, list items) nest deeper than 100, at the `line:column` of the first one too
   deep. No line scan can see comrak's structure exactly (several markers per line, tabs, lazy continuation lines, CR
   endings, items continued under a narrower quote prefix), so the scan only over-counts, and twice 100 still lets
   lists indented by four columns per level reach the exact check. Then comrak (below) and conversion to normalized
   JSON. JSON: validated against the schema mirror (an unknown
   node, mark or attr, a child the content expression forbids, a wrong attr type, a mark combination an `excludes` rule
   forbids, a link outside the scheme allow-list, nesting deeper than 100); missing attrs get their defaults and marks
   are sorted by rank. A failure is 422 `invalid_document` with `details {path: <JSON pointer, or line:column for
   Markdown>, reason}`.
2. **Base.** A stored document with `schema_version > 1`, or with nodes or marks unknown to v1, gives 409
   `schema_too_new`. The base is the current state when `If-Match` equals the current ETag or is `*`; with
   `?merge=true` and an older `d{n}` with `n ≥ compacted_seq`, a scratch doc made of `ydoc` plus the stored updates with
   `seq ≤ n`. With an older `d{n}` and no merge, the target is compared with the current state: equal gives 200
   `changed: false` (a retried PUT; under `?title=from_h1` only when the title matches too), different gives 412
   `rev_mismatch` with `details {etag, mergeable}`. An `If-Match` that is not `*` or `"d{n}"` is stale (412). A
   ProseMirror PUT with a `schema_version` other than 1 is 422 `validation`.
3. **Comparison space.** JSON: the base read as normalized JSON. Markdown: `B = md_in(md_out(base))`, with every B block
   mapped to the base block it was printed from; the input is diffed against B, so a block whose Markdown is unchanged
   is never touched, whatever it holds that Markdown cannot express. Base blocks that print to nothing (empty
   paragraphs) have no B block and stay in place. The block map: `md_out::lower` applies the lossy rules of the table
   below in JSON space and tags each node with the base child it stands for, and the printer renders the lowered tree
   (`print = render ∘ lower`; `md_normalize` is `lower` without the tags). The lowered base is used as B when it equals
   the re-parse of the printed base; where it does not (a lossy step the lowering does not model, such as comrak
   autolinking an e-mail address in text), `convert::comparison_tree` keeps the re-parsed top-level blocks, each mapped
   to the lowered block it equals or standing for the base blocks of its gap. A changed block paired with its base
   keeps the attrs Markdown cannot express: `image.width`, and in a table the cells' `colwidth` and `align`; a changed
   table whose base has spans keeps them when the rows keep their shape and the expanded filler cells stay empty, and
   is otherwise replaced as a whole (warning `table_span_dropped`).
4. **Diff**, in one transaction on the base:
   - children, per level: anchors are `blockId` matches (JSON input only) and children with equal content hashes (name,
     attrs without `blockId`, content), of which the longest order-preserving set is kept (a Myers diff over the
     hashes, O((N+M)·D)). An anchored pair whose content differs is updated in place; a `blockId` match out of order is
     deleted and re-inserted with its `blockId`. Between two anchors, old and new children of the same name pair in
     order while their text similarity (common prefix plus suffix over the longer length) is at least 0.5, and are
     updated in place (changed attrs with `insert_attribute`/`remove_attribute`, children recursively); the rest are
     deleted or inserted. Inserting a block at the top and editing one at the bottom thus touches exactly those two.
     An empty base textblock pairs with any target block of its type, so a first write fills the empty paragraph the
     editor put into a new document instead of leaving it behind. Myers gives up beyond 2000 edits at one level (then
     only the common prefix and suffix anchor), and similarity pairing looks at most 64 base children ahead;
   - text: drop the common prefix and suffix (UTF-16 units), delete the middle and insert the new middle with its marks,
     then `format` the ranges whose marks changed;
   - every new block node without a `blockId` gets a UUID v4, the format of TipTap's UniqueID; a `blockId` sent in JSON
     is kept unless another block already has it;
   - **determinism:** with `h = sha256(credential id ‖ item id ‖ base ETag ‖ normalized query ‖ sha256(body))`, the
     transaction's client id is the first 4 bytes of `h` as a non-zero u32 (re-hashed while it is in the base's state
     vector), and new `blockId`s come from `sha256(h ‖ counter)` as UUID v4. Text is inserted plain and then formatted
     one mark key at a time in name order, attributes in name order. A repeated request (a client retry) produces the
     same client id, clocks and content, which the §6.1 no-op rule absorbs; the bytes are identical too unless the
     update writes an item link, because yrs encodes the keys of a map value (`{href, itemId}`) in hash-map order.
     The content hashes serialize through `schema::canonical` (sorted keys, explicit stack), so serde_json's map order
     never changes them (a dependency that turns on `preserve_order` makes it insertion order; oas3 did in test builds
     until it was built without default features, §4.1). Collision
     odds equal those of Yjs's random ids.
5. **Commit.** Encode the update (`encode_update_v1` against the base's state vector) and drop the base and scratch
   docs. Then, in the writer transaction: re-read `next_seq - 1`; if it moved past the base and neither `merge` nor
   `If-Match: *` was given, 412 `rev_mismatch` (of two strict PUTs with the same ETag, exactly one succeeds); otherwise write the D53 `pre-api`
   revision when it applies, save through §6.1 from step 2 with its permit-free variant (the CRDT merges the update with
   any newer ones; `merged` is true when there were some), set `pm_json = NULL` (the cache no longer matches; the next
   web save refills it), apply `?title=from_h1` with `items::rename`, and record the actor in `doc_clients` as for any
   update. An empty diff writes nothing and returns `changed: false`.

An open web editor sees the write the next time it fetches `?since=` (focus or visibility, §9.4) and never loses its own
unsent edits, which are CRDT updates as well. Live push arrives with the WebSocket (§15).

**Quire Markdown 1** (`convert::md_in`, `convert::md_out`):
- Parser: comrak with the extensions `table`, `strikethrough`, `tasklist`, `autolink`, `alerts` and `highlight` on. Off:
  `underline` (so `__x__` stays bold, as in CommonMark), footnotes, math, wikilinks, front matter, description lists,
  superscript, subscript, spoiler, shortcodes, header ids and smart punctuation. Raw HTML is never rendered: an HTML
  block becomes a paragraph of its literal text (its lines joined by hard breaks), inline HTML becomes literal text,
  except the exact pair `<u>`…`</u>`. A list item that starts with a non-paragraph gets an empty first paragraph
  (TipTap's `paragraph block*`).
- Printer (Quire's own, deterministic): ATX headings; `-` bullets, alternating with `*` for a bullet or task list that
  directly follows another (so the two stay separate lists); `1.`-style ordered lists starting at `start`, alternating
  with `1)` after another ordered list; an empty item prints its marker alone; fenced code with a fence longer than any
  backtick run inside; `---` rules; backslash hard breaks; whitespace at the edge of a marked run printed outside its
  delimiters; one blank line between blocks, LF line endings, one trailing newline, and CommonMark backslash escapes
  for punctuation that would otherwise start a construct. Bare URLs in text are escaped (`https\://`, `&#119;ww.`) so
  they do not autolink. Inline content is written as cells and then passed through `md_out::fix_runs`, a linear pass
  that mirrors comrak's flanking rules (CommonMark's, with `~` and `=` skipped when looking at a run's neighbours): a
  letter (or a non-ASCII character Rust cannot classify) next to a delimiter that would otherwise not open or close is
  written as a decimal character reference (`&#120;*\_y\_*`), and so is whitespace at the inside edge of a mark that
  the lowering does not move (a no-break space); text `=` or `_` that could open, or close an open, highlight or `_`
  emphasis is escaped. Bold and italic use `_`/`__` instead of `*`/`**` right after a `*` closer (so closers and openers
  never merge into one run) and when opened while the other is open from a `***` run; italic inside a lone `**`, and
  the reverse, keep `*`. An `@` with an e-mail local-part character before it and a domain character after it is
  escaped (`a\@b.co`), and a `!` before a mention is escaped as before a link. These rules change the output only
  where it would not parse back. The printer also emits the block map that step 3 of the write path uses.
- Properties (proptest, `convert::tests::markdown_roundtrip`): for every schema-v1 document,
  `parse(print(d)) == md_normalize(d)` ignoring `blockId`, where `md_normalize` applies exactly the lossy rules of the
  table below; and `print(parse(print(d))) == print(d)`.
- Scheme allow-list for `link.href` and external `image.src` on every write path, and in TipTap's Link `protocols` /
  `isAllowedUri`: `http`, `https`, `mailto` (links only), `quire`, same-origin paths `/…` and fragments `#…`. Markdown
  input with another scheme (`javascript:`, `data:`, …) stays literal text, and Markdown output prints such a stored
  link as its text; both with the warning `unsafe_link`. Both lists (`quire_core::convert::link_allowed` /
  `image_allowed` and `web/src/editor/links.ts`) check the URL after dropping every control character and space
  (U+0000–U+0020, U+007F–U+009F), since a browser drops tab and newline anywhere in a URL: `/\t/host` is the
  protocol-relative `//host` and stays text, like `/\host`. The two tests share their list of rejected URLs.
- Item links use the scheme `quire://item/<item uuid>`. On input, a same-instance URL (`/doc/<slug>-<url_id>`,
  `/board/<slug>-<url_id>`, or either under `QUIRE_PUBLIC_URL`) is resolved to the item id as well; attachment URLs
  `/api/v1/attachments/<id>` likewise. Resolution needs the database, so the parser takes a lookup closure from the
  store.

| Schema v1 content | GET Markdown (Y → Markdown) | PUT Markdown (Markdown → Y) |
|---|---|---|
| `paragraph`, `heading{level}` (1–6), `blockquote`, `horizontalRule`, `hardBreak`, `bulletList`, `orderedList{start}`, `listItem` | Lossless, except: an empty paragraph prints nothing (warning `empty_paragraph_dropped`); a `hardBreak` at the end of a block prints nothing and one inside a heading prints a space (warning `hard_break_dropped`) | Lossless. Setext headings, `*`/`+` bullets, `)` ordered markers and indented code are accepted and normalized; a soft line break becomes a space; adjacent lists stay separate only when their markers differ |
| `taskList`, `taskItem{checked}` | `- [ ]` / `- [x]` | Lossless. A list whose first item has a checkbox becomes a task list; its items without one become unchecked (warning `task_list_mixed`); a checkbox after the first item of a plain list stays text |
| `codeBlock{language}` | Fenced; info string = language | First word of the info string; the rest is dropped |
| `callout{variant}` | GitHub alert: info `[!NOTE]`, tip `[!TIP]`, warning `[!WARNING]`, danger `[!CAUTION]` | Those four map back; `[!IMPORTANT]` becomes info (warning `alert_mapped`); an alert's custom title becomes its first paragraph |
| `bold`, `italic`, `strike`, `code`, `highlight` | `**`, `*`, `~~`, backticks, `==`; whitespace at a mark's edge moves outside it | Lossless; `__x__` and `_x_` are accepted. `code` excludes every other mark (TipTap's `excludes: '_'`), so other marks on code text are dropped (warning `marks_in_code_dropped`) |
| `underline` | `<u>…</u>` | The exact `<u>`/`</u>` pair; any other inline HTML is literal text (warning `html_as_text`) |
| `link{href}` | `[text](href)`; outside the scheme allow-list, the text only (`unsafe_link`) | Lossless within the allow-list; autolinks become links; other schemes stay literal text (`unsafe_link`) |
| `link{itemId}` | `[text](quire://item/<itemId>)` | `quire://item/…` or a same-instance item URL → `link{itemId}` (a same-instance URL keeps its `href` beside the `itemId`); an unknown item → a plain link (warning `unknown_item`) |
| `mention{itemId}` | `[@<live title>](quire://item/<itemId>)`; "Deleted item" when the target is unknown | A link whose text starts with `@` and whose target is `quire://item/<uuid>` or an item URL → `mention`, even when the item is unknown (warning `unknown_item`), so a deleted target round-trips; the text is ignored (mentions show the live title) |
| `boardEmbed{itemId}` | A paragraph holding only `![<board title>](quire://item/<itemId>)`; "Board" when the target is unknown | An image whose URL is an item: a board → `boardEmbed`; a document → `mention` (warning `embed_not_board`); an unknown item → `boardEmbed` (warning `unknown_item`) |
| `image{attachmentId, alt, caption, width}` | `![alt](/api/v1/attachments/<id> "caption")`; width dropped (warning `image_width_dropped`) | A same-instance attachment URL that exists → `attachmentId`; any other allowed URL → an external image (`attachmentId: null`, the §6.1 placeholder); title → caption; width default, or the base's for an unchanged image (step 3). TipTap's image is a block: an image inside a paragraph with other content is hoisted into its own block after that paragraph (warning `image_hoisted`) |
| External `image{src}` | `![alt](src)` | As above |
| `table`, `tableRow`, `tableHeader`, `tableCell` | GFM table. The first row is the header (an empty header row is added when the first row has no `tableHeader` cells). A cell holding anything but one paragraph is flattened: its blocks' inline text joined with `<br>` (warning `table_cell_flattened`). `colspan`/`rowspan` are expanded into empty cells (warning `table_span_dropped`); `colwidth` is dropped | A GFM table → `table` with a `tableHeader` first row; `<br>` in a cell → `hardBreak`; column alignment is dropped (warning `table_alignment_dropped`). A table whose Markdown is unchanged keeps its spans, widths and multi-block cells (step 3) |
| `blockId` | Not written | Blocks whose Markdown is unchanged are untouched; changed blocks paired with the base keep their id; new blocks get fresh ids |
| The item title | Not in the body; `?title=include` prepends `# <title>` and a blank line | `?title=from_h1`: a leading level-1 heading is removed and becomes the item title in the same transaction |
| Nodes or marks unknown to schema v1 | Dropped (warning `unknown_node`) | 409 `schema_too_new` (step 2): nothing is deleted |
| Footnotes, math, front matter, definition lists | n/a | Literal text (warning `unsupported_markdown`) |

ProseMirror JSON is lossless in both directions for schema v1: GET returns the normalized shape (`editor.getJSON()`),
and PUT rejects exactly what the schema rejects, plus links outside the scheme allow-list. Warnings are `ConversionWarning{code, path?, count}`: PUT returns them in its
body, GET Markdown lists their codes in the `Quire-Conversion-Warnings` response header. Input limits: Markdown 1 MiB,
JSON 4 MiB, nesting depth 100 (D33).

---

## 7. HTTP API

### 7.1 Conventions

- **One versioned API** (D39).
  - Every API route lives under `/api/v1`: REST JSON with snake_case fields. `/healthz` is the only unversioned route;
    any other path under `/api/` answers a JSON 404 `not_found`.
  - The web client is a client of this API like any other: it authenticates with the session cookie, external clients
    with a personal access token (§7.3). There are no private routes, and every route is in the OpenAPI spec (§7.4).
  - Compatibility within v1: routes, optional request fields, response fields, error codes and enum values may be
    added. Clients must ignore unknown response fields and treat an unknown error `code` by its HTTP status. Removing
    or renaming a field, changing a type or a default, or tightening validation needs `/api/v2`, served next to v1 for
    at least one release. `quire_api::API_VERSION` stays `"1.0"` until the first release; after that the release
    bumps the minor once when it adds anything. It is the spec's `info.version`, `api_version` in
    `GET /api/v1/config`, and the `Quire-Api-Version` header on every `/api/v1` response.
- **IDs and positions.**
  - Path parameters are named `{id}` (or `{n}`) and are UUIDs; `GET /api/v1/items/{id}` and
    `GET /api/v1/collections/{id}` also accept a `url_id`. One name per path position and no static text after a
    parameter: axum 0.8.9 pins matchit 0.8.4, which rejects both at startup.
  - Positions are `{after_id?: string|null, before_id?: string|null}`. If both are omitted, the item goes last.
  - The document routes (`…/doc`, `…/doc/*`) take UUIDs only, and given a board they answer 422 `validation` with
    `field: "kind"`, as a board PUT does for a document; a board GET for a document is 404.
- **Authentication and scopes** (§7.3). The Auth column of §7.2 names what each route needs: *public* (nothing);
  `read`, `write` or `admin` (a token with that scope, or the session cookie, which has every scope); *session* (the
  cookie only); *token* (any valid token). While no owner exists, every non-public route answers 409 `setup_required`;
  an owner made by `quire admin create-owner` while the server runs is seen on the next request (§8).
- **Errors.** Every error has the shape `{"error":{"code","message","details"?},"request_id"}` (`ErrorBody`), and every
  response carries `X-Request-Id` with the same id. `code` is stable and machine-readable; `message` is English for
  people and may change; `details` has a per-code schema in the spec. axum's own rejections (unknown route, wrong
  method, malformed JSON, path or query) are mapped to the same shape: a body, query or path that does not
  deserialize is 422 `validation` with `details.field` `"body"`, `"query"` or `"path"`; 401, 403, 404, 405, 408, 409,
  413 (`limit_bytes` 1048576), 415, 428 and 503 keep their codes; any other 4xx becomes 422 and any other 5xx 500.
  Request ids are always generated by the server (UUIDv7, 32 hex digits); a client's `X-Request-Id` is ignored.

  | Code | Status |
  |---|---|
  | `unauthenticated`, `invalid_credentials`, `invalid_token` | 401. `invalid_token` adds `WWW-Authenticate: Bearer error="invalid_token"` |
  | `forbidden`, `csrf_rejected`, `insufficient_scope`, `session_required` | 403. `insufficient_scope` adds `WWW-Authenticate: Bearer error="insufficient_scope", scope="<required>"` and `details {required, granted}` |
  | `not_found` | 404 |
  | `method_not_allowed` | 405 |
  | `request_timeout` | 408 (the request deadline, §8: 30 s, or 120 s for the body of an upload or board PUT; the request may have been processed) |
  | `conflict`, `cycle`, `parent_invalid`, `missing_deps`, `setup_required`, `already_setup`, `read_only`, `not_empty`, `schema_too_new` | 409 |
  | `rev_mismatch` | 412, `details {etag}` plus `rev` (boards) or `mergeable` (documents) |
  | `payload_too_large`, `doc_too_large` | 413, `details {limit_bytes}` |
  | `unsupported_media_type` | 415 |
  | `host_not_allowed` | 421 |
  | `validation`, `invalid_board`, `board_too_new`, `invalid_update`, `invalid_document` | 422. `validation` has `details {field}`; `invalid_document` has `details {path, reason}` |
  | `precondition_required` | 428 |
  | `rate_limited` | 429 + `Retry-After` (seconds), `details {reason: "requests" \| "bytes" \| "auth_failures" \| "busy"}` |
  | `internal` | 500 |
  | `unavailable` | 503, sent only before a handler runs (shutdown drain): the request was not processed |
  | `insufficient_storage` | 507, below the free-space floor (D33) |

  `read_only` is returned when writing to a trashed or archived item.
- **Pagination** (D46). Paginated lists take `?cursor=&limit=` (default 25, maximum 100) and return
  `Page<T> = {items: [T], next_cursor: string|null}`. Cursors are opaque, stable under concurrent inserts and deletes,
  and valid only with the parameters that produced them (otherwise 422 `validation` with `details.field = "cursor"`).
  Relevance-sorted search pages by offset, up to 1,000 results. A cursor is base64url without padding over version ‖
  kind ‖ the 8-byte hash of the list's name and normalized parameters (never `cursor` or `limit`; not the viewer in
  M1) ‖ the keyset or offset ‖ a 4-byte sha256 check; input over 8192 characters is refused without decoding.
  Unpaginated by design, and bounded: `/collections`,
  `/collections/{id}/tree` (D21), `/items/{id}/children`, `/trash` (roots only), `/tokens` (at most 100),
  `/auth/sessions` and `/search/titles` (`limit` at most 20).
- **Concurrency** (D45). Content writes require `If-Match` (428 `precondition_required` without it); `If-Match: *`
  means "whatever is current", a deliberate last-write-wins. GETs with a matching `If-None-Match` answer 304, except
  `GET /items/{id}`, the Markdown GET and the board GET (D45). A `W/` prefix on a tag is ignored. The JSON `etag`
  fields carry the exact header value.

  | Resource | ETag | Write rule |
  |---|---|---|
  | Board body (`GET`/`PUT …/board`) | `"r{rev}"`; never 304 (the body carries the live title as `document.name`) | `If-Match` required; stale → 412, unless the body equals the stored one (200, the no-op of §6.2) |
  | Document body (`…/doc/markdown`, `…/doc/prosemirror`) | `"d{seq}"`: the newest stored update, 0 for a new document (§5). Markdown: never 304 (it prints live titles) | `If-Match` required; stale → 412 (200 `changed: false` when the target already equals the current state), or a CRDT merge with `?merge=true` (D50) |
  | Document Yjs updates (`POST …/doc/updates`) | none | No precondition: updates commute (CRDT) |
  | Item metadata (`GET`/`PATCH /items/{id}`) | `"m{meta_version}"`; never 304 (the body also carries stars, breadcrumbs and board meta) | `If-Match` optional; stale → 412; without it the last write wins (the web client sends none) |
  | Tree (`GET …/tree`) | `"t{tree_version}.{viewer_epoch}"` | Read-only |
- **Request limits** (D33). Bodies are limited to 1 MiB unless §7.2 says otherwise: Yjs update 16 MiB, ProseMirror
  JSON PUT 4 MiB, Markdown PUT 1 MiB, board PUT `QUIRE_MAX_BOARD_MB`, attachment `QUIRE_MAX_UPLOAD_MB`. Over the limit:
  413 `payload_too_large` with `details.limit_bytes`. `q` is at most 500 characters; JSON nesting at most 100 levels
  (a deeper board body is 422 `invalid_board`, a deeper ProseMirror document 422 `invalid_document`).
  Every limit is listed in `GET /api/v1/config` (`limits`).
- **Media types.** Mutating routes take `application/json`, except the declared types of the attachment upload (raw
  bytes of any type, sniffed) and of the Markdown PUT (`text/markdown`). `application/x-www-form-urlencoded`,
  `multipart/form-data` and `text/plain`, the types a cross-site form can send without a preflight, are never accepted
  (415). The web client and the CLI therefore always upload as `application/octet-stream`.
- **Rate limits.** Per token, in requests and body bytes (D47, §7.3); setup and login per IP and per email (§8).
- **DTO conventions.** Times are RFC 3339 UTC strings with milliseconds (`string` in TypeScript, `date-time` in
  OpenAPI). Ids are UUIDs, except path references and `url_id`s; a path reference is a UUID or exactly 10 ASCII
  letters and digits, and anything else is 404. Response fields without a value are sent as `null`; optional request
  fields are omitted; a PATCH field where `null` and omitted differ is tri-state (`CollectionPatch.icon`,
  `WorkspacePatch.logo_attachment_id`, `NewToken.expires_in_days`); defaulted boolean flags are omitted when false.
  Unit enums are snake_case strings. `i64`/`u64` fields are `number` in TypeScript (`TS_RS_LARGE_INT`, §3).
  `ErrorCode` is a strict enum in Rust, so a client that must tolerate future codes reads `code` as a string.
- **Shared shapes** (`quire-api`, re-exported as `quire-core::api`):
  - `User{id,email,name,role,theme}`
  - `Collection{id,url_id,name,description,icon,color,sort_key,child_order,item_count,updated_at}`
  - `ItemSummary{id,url_id,collection_id,parent_id,kind,title,sort_key,child_count,published,archived,updated_at,content_at,created_at}`
  - `Crumb{id,url_id,kind,title}`
  - `ItemDetail{item:ItemSummary, collection:Collection, breadcrumbs:[Crumb], starred, effectively_published, trashed_at?, purge_at?, archived_at?, full_width, board?:BoardMeta, meta_version}` (`full_width` is additive, §17.4; `board` is `boards::board_meta`, the same `BoardMeta` a board PUT returns, §5 "Previews")
  - `BoardMeta{rev,pages:[{id,name,svg_len}],objects,background,bytes,preview_rev,text_excerpt}`
  - `Page<T>{items:[T], next_cursor}`
  - `SearchHit{item:ItemSummary, path:[Crumb], snippet:[{text,hit}], score}`
  - `Scope = "read"|"write"|"admin"`;
    `ApiToken{id,name,prefix,scopes:[Scope],created_at,created_via,parent_token_id?,expires_at?,last_used_at?,last_used_ip?}`
  - `Me{user, workspace:{name, logo_url}, must_change_password, auth:{method:"session"|"token", token?:ApiToken}}`
  - `DocJson{schema_version, seq, doc}` (`doc` is normalized ProseMirror JSON `{type:"doc", content}`, §6.4)
  - `DocWriteResult{seq, etag, updated_at, changed, merged, warnings:[ConversionWarning]}`;
    `ConversionWarning{code, path?, count}`

### 7.2 Routes

Auth: *public*, *session*, *token*, or the scope a token needs (a session has every scope), §7.1.

| Method | Path | Auth | Request | Response | Owner |
|---|---|---|---|---|---|
| GET | `/healthz` | *public* | none | `{"status":"ok"}` after `SELECT 1`. Exempt from the Host allowlist | W0b |
| GET | `/api/v1/config` | *public* | none | `{version, commit, api_version, openapi_url:"/api/v1/openapi.json", setup_required, multi_user:false, inkboard: null \| {base:"/inkboard/<build>/", engine_version, format_version, wasm_bytes:{raw,br,gz}}, limits:{max_upload_mb, max_board_mb, max_doc_mb, max_update_bytes, max_markdown_bytes, max_json_bytes, max_body_bytes, json_depth, q_max_chars, page_max, token_rate_per_min, token_burst, token_bytes_per_hour}}`. `inkboard` is null when no manifest is embedded, for the stub dist (`engine_version: "stub"`, whose files are still served) and for a dist without a `.wasm`. The `limits` are the ones the routes enforce (`config::MAX_*`) | W2 |
| GET | `/api/v1/openapi.json` | *public* | `If-None-Match` | The OpenAPI 3.1 spec (§7.4), `Cache-Control: no-cache`, ETag | W2 (each route owner documents its own handlers) |
| GET | `/api/v1/setup` | *public* | none | `{needs_setup}` | W2 |
| POST | `/api/v1/setup` | *public* | `{code, workspace_name, name, email, password}` | 201 `{user}` + a browser-session cookie. Seeds "Getting started". Checked in this order: the per-IP limiter (429), 409 `already_setup`, the code (a wrong one is 401 `invalid_credentials`), then the fields (`email`, `name`, `workspace_name`, `password`; 422) | W2 |
| POST | `/api/v1/auth/login` | *public* | `{email, password, remember}` | `{user}` + cookie; deletes the session the browser still presents (rotation). 401 / 429. A malformed email gets the same 401 and is not counted per email (§8) | W2 |
| POST | `/api/v1/auth/logout` | *session* | none | 204, clears the cookie | W2 |
| GET | `/api/v1/auth/me` | `read` | none | `Me` (with a token, `auth.token` describes it) | W2 |
| PATCH | `/api/v1/auth/me` | `admin` | `{name?, email?, theme?}` | `{user}`. Changing `email` needs a session (a token gets 403 `session_required`); it counts as a change only when the address differs ignoring case, and a taken address is 409 `conflict` with `details.field = "email"` | W2 |
| POST | `/api/v1/auth/password` | *session* | `{current, new}` | 204. Revokes other sessions. A wrong `current` is 422 (`field: "current"`, never 401), a bad `new` 422 (`field: "new"`) | W2 |
| GET | `/api/v1/auth/sessions` | `admin` | none | `[{id,created_at,last_seen_at,user_agent,ip,current}]` | W2 |
| DELETE | `/api/v1/auth/sessions/{id}` · `/api/v1/auth/sessions?others=true` | `admin` | none | 204. From a token, `others=true` revokes every session; `DELETE /auth/sessions` without `others=true` is 422 (`field: "others"`) | W2 |
| GET | `/api/v1/tokens` | `admin` | none | `[ApiToken]`, live and expired (not revoked), newest first, at most 100. Never the secret or its hash | W2 |
| POST | `/api/v1/tokens` | `admin` | `{name, scopes:[Scope], expires_in_days?: 1–3650 \| null, password?}` (omitted: 90; `null`: never) | 201 `{token:"qpat_…", api_token:ApiToken}`; the only response that contains the secret. 422 when the expiry is later than a calling token's (`field: "expires_in_days"`) or 100 live tokens exist (`field: "tokens"`); a scope above the caller's (`field: "scopes"`) can only arise in the store, since the route needs `admin`. A calling token gets 403 `session_required` for `admin` scope. A session must send `password` for an `admin` or never-expiring token (422, `field: "password"`, when missing or wrong) | W2 |
| DELETE | `/api/v1/tokens/{id}` | `admin` | none | 204; also revokes the tokens it created | W2 |
| DELETE | `/api/v1/tokens/current` | *token* | none | 204: the calling token revokes itself | W2 |
| GET / PATCH | `/api/v1/workspace` | `read` / `admin` | `{name?, logo_attachment_id?: string\|null}` | `{name, logo_url}`. The logo must be an existing PNG, JPEG, GIF, WebP or AVIF attachment (422, `field: "logo_attachment_id"`) | W2 |
| GET | `/api/v1/sidebar` | `read` | none | `{collections:[Collection], stars:[ItemSummary], drafts_count}` | W1 |
| GET / POST | `/api/v1/collections` | `read` / `write` | POST `{name, icon?, color?, description?, position?}` | `[Collection]` / 201 `Collection` | W1 |
| GET | `/api/v1/collections/{id}` | `read` | none (`id` may be a `url_id`) | `Collection` | W1 |
| PATCH | `/api/v1/collections/{id}` | `write` | `{name?, icon?, color?, description?, child_order?}` | `Collection` | W1 |
| POST | `/api/v1/collections/{id}/move` | `write` | `{position}` | `Collection` | W1 |
| DELETE | `/api/v1/collections/{id}` | `write` | none | `{trashed_items:n}`. Soft delete, appears in Trash | W1 |
| POST | `/api/v1/collections/{id}/restore` | `write` | none | `Collection` | W1 |
| GET | `/api/v1/collections/{id}/tree` | `read` | `If-None-Match` | `{collection, items:[ItemSummary]}`, flat, visible, unarchived, sorted by `child_order`. ETag `"t{tree_version}.{viewer_epoch}"` | W1 |
| GET | `/api/v1/collections/{id}/updated` | `read` | `?cursor&limit` | `Page<ItemSummary + path>`, all depths, newest first | W1 |
| POST | `/api/v1/items` | `write` | `{collection_id, parent_id?, kind:"doc"\|"board", title?, position?}` | 201 `ItemSummary`, published (`published_at = created_at`, D23). Creates `doc_state` or `board_state` in the same transaction | W1 (+ W4/W6 initializers) |
| GET | `/api/v1/items/{id}` | `read` | none (`id` may be a `url_id`) | `ItemDetail`, `ETag: "m{meta_version}"` for a later `If-Match`; never 304 | W1 |
| PATCH | `/api/v1/items/{id}` | `write` | `{title?, full_width?}`, `If-Match: "m{n}"` optional | `ItemSummary` and the new `ETag: "m{n}"`. A stale `If-Match`, or one that is neither `*` nor `"m{n}"`, gives 412 `rev_mismatch`; without one the last write wins (the web client sends none). An unchanged title changes nothing | W1 |
| GET | `/api/v1/items/{id}/children` | `read` | none | `[ItemSummary]` | W1 |
| GET | `/api/v1/items/{id}/text` | `read` | none | `{kind, title, body, updated_at}`: the derived plain text that search indexes (§6.1, §6.2) | W1 |
| POST | `/api/v1/items/{id}/move` | `write` | `{collection_id?, parent_id: string\|null, position?}` | `ItemSummary`. 409 `cycle` / `parent_invalid`. `parent_id` is required (`null` is the top level): a body without it is 422 `validation` (`field: "body"`), so a reorder among the same siblings sends the current parent | W1 |
| POST | `/api/v1/items/{id}/duplicate` | `write` | none | 201 `ItemSummary`. Copies the ydoc or board bytes (boards share their blobs and copy no previews) and `item_attachments`; children are not copied. The copy goes right after the original, titled `<title> (copy)` (an empty title stays empty), with the same `full_width`. The source must be live and unarchived (409 `read_only`) | W1 (+ W4/W6 copy fns) |
| POST | `/api/v1/items/{id}/publish` | `write` | `{with_ancestors: bool}`, or no body (`false`) | `ItemSummary`. With `true`, draft ancestors are published too. Not called by the M1 web client (D23). A non-empty body must be `application/json` (415) | W1 |
| POST | `/api/v1/items/{id}/unpublish` | `write` | none | `ItemSummary`. Always allowed; in M1 the only way an item becomes a draft (D23) | W1 |
| POST | `/api/v1/items/{id}/archive` · `/unarchive` | `write` | none | `{archived:n}` / `RestoredItem`: the unit's root (`ItemSummary` + `moved_to_top`, §5). Archive skips trashed descendants | W1 |
| DELETE | `/api/v1/items/{id}` | `write` | `?discard_empty=true` optional | `{trashed:n}`. With `discard_empty`: permanent delete if the item is new (created less than 24 h ago) and has no children (in any state), a title that is blank after trimming and an empty body (`docs::is_empty` / `boards::is_empty`), published or not; otherwise 409 `not_empty`. Used by the web client when the owner leaves a new empty item, and by `quire item create --md` after a failed PUT | W1 |
| POST | `/api/v1/items/{id}/restore` | `write` | none | `RestoredItem`: the unit's root (`ItemSummary` + `moved_to_top`, §5). 409 `parent_invalid` while its collection is deleted; 409 `conflict` for a live item | W1 |
| GET | `/api/v1/trash` | `read` | none | `{collections:[Collection+deleted_at+purge_at], items:[ItemSummary+deleted_at+purge_at+descendant_count+path]}` (roots only) | W1 |
| DELETE | `/api/v1/trash/items/{id}` · `/api/v1/trash/collections/{id}` · `/api/v1/trash` | `admin` | none | 204 (purge). Any trashed item with its subtree; a live item or collection is 409 `conflict`. `/trash` purges every item root and deleted collection | W1 |
| GET | `/api/v1/archive` · `/api/v1/drafts` · `/api/v1/starred` · `/api/v1/recent` · `/api/v1/updated` | `read` | `?cursor&limit` | `Page<ItemSummary + path + extra>`. `/drafts` lists unpublished items (in M1 only those unpublished through the API or CLI, D23). `/archive` lists the archive roots that are not trashed; every other list leaves archived items out; `/recent` fills only through `POST …/view` | W1 |
| PUT / DELETE | `/api/v1/items/{id}/star` | `write` | `{position?}`, or no body | 204. PUT without a position keeps an existing star's place; starring a trashed item is 409 `read_only`; DELETE is idempotent. Stars list visible, unarchived items in star order | W1 |
| POST | `/api/v1/items/{id}/view` | `write` | none | 204. Upserts `item_views` (the CLI never calls it) | W1 |
| GET | `/api/v1/search` | `read` | `?q&collection_id&kind&updated_after&include_archived&include_drafts=true&sort=relevance\|updated&cursor&limit` | `Page<SearchHit>` | W1 |
| GET | `/api/v1/search/titles` | `read` | `?q&limit=10` | `[{item, path}]` (quick switcher and mentions) | W1 |
| GET | `/api/v1/items/{id}/doc` | `read` | `?since=<b64 state vector>` | `{update, state_vector, seq, field:"default", schema_version}` (b64 Yjs v1) | W4 |
| POST | `/api/v1/items/{id}/doc/updates` | `write` | `{update:b64, pm_json?}`, up to 16 MiB (`pm_json`: an object or `null`, stored as sent, §6.1) | `{seq, state_vector, updated_at}`. 409 `missing_deps{state_vector}` | W4 |
| GET | `/api/v1/items/{id}/doc/markdown` | `read` | `?title=include`, `?download=true` | `text/markdown; charset=utf-8` (§6.4), `ETag: "d{seq}"` (never 304), `Quire-Conversion-Warnings`. `download` adds `Content-Disposition: attachment; filename="<slug>.md"`, the slug being the title in ASCII with every other character as `-` | W4 |
| PUT | `/api/v1/items/{id}/doc/markdown` | `write` | `text/markdown` up to 1 MiB, `If-Match` required, `?merge=true`, `?title=from_h1` | `DocWriteResult` + ETag. 409 `schema_too_new` / 412 / 422 / 428 | W4 |
| GET | `/api/v1/items/{id}/doc/prosemirror` | `read` | `If-None-Match` | `DocJson`, `ETag: "d{seq}"` | W4 |
| PUT | `/api/v1/items/{id}/doc/prosemirror` | `write` | `{schema_version?: 1, doc}` up to 4 MiB, `If-Match` required, `?merge=true` | `DocWriteResult` + ETag. 422 `invalid_document{path, reason}` / 409 `schema_too_new` / 412 / 428 | W4 |
| GET | `/api/v1/schema/doc` | `read` | none | `DocSchema{schema_version, top_node, nodes: [NodeSpec{name, group, content, marks, inline, atom, attrs}], marks: [MarkSpec{name, rank, excludes, inclusive, attrs}]}`, with `AttrSpec{name, type, default}`: schema v1 as data (§6.4). `NodeSpec.marks` is the allowed marks (null for the default), `type` is TypeScript notation (`integer`, `string \| null`, `integer[] \| null`, `"info" \| "tip" \| …`); `fixtures/schema/v1.json` is exactly this JSON | W4 |
| GET | `/api/v1/items/{id}/board` | `read` | none | envelope JSON, `ETag: "r{rev}"` (for `If-Match`), `Cache-Control: no-store`; never 304 (§6.2). 429 `busy` while large bodies hold the body budget (D25) | W6 |
| PUT | `/api/v1/items/{id}/board` | `write` | envelope JSON, `If-Match` (`"r{rev}"` or `*`), `X-Quire-Engine` (optional for API clients), up to `max_board_mb` | `{rev, etag, updated_at, meta}`. 408 / 412 / 413 / 415 / 422 / 428 / 507, in the order of §6.2 | W6 |
| GET | `/api/v1/items/{id}/board.inkboard` | `read` | none | The envelope as a download, `Content-Disposition: attachment; filename="<title>.inkboard"`; 429 `busy` as above | W6 |
| GET | `/api/v1/items/{id}/board/pages/{n}` | `read` | `?rev=N` optional | `image/svg+xml` (no `.svg` suffix: matchit 0.8.4). `n` must be a page of the board, else 404. Without `rev`: the current page, `no-cache`. `rev` current: immutable. Other rev: 307 to the relative `/api/v1/items/{id}/board/pages/{n}?rev={current}`, `no-store`. Every answer has `ETag: "r{rev}"` and answers a matching `If-None-Match` with 304 (the SVG holds no title). Empty page: 204. A page with no stored preview is rendered on demand (§12); a page the preview budget left out is the "Too large to preview" card (§5) | W6 |
| POST | `/api/v1/attachments` | `write` | raw body (clients send `application/octet-stream`), `?item_id&filename`, up to `max_upload_mb`, streamed to `/data/tmp` under `upload_work` | 201 `{id, url, sha256, mime, size, width?, height?}`. 415 for the three form types (after `Auth<S>`); 413 from `Content-Length` and again while streaming; 422 over 100 MP (`field: "body"`); 404 for an unknown `item_id`; 507 below the free-space floor. The filename is trimmed, and `.` or `..` becomes `file` | W2 |
| GET | `/api/v1/attachments/{id}` | `read` | none | bytes (§11) | W2 |
| GET | `/api/v1/export` | `admin` | none | `application/x-tar` stream. Single-flight (429 `busy`); no request deadline (§8). With the cookie it needs `Sec-Fetch-Site` `same-origin` or `none`, or, without fetch metadata, a same-origin `Referer`; otherwise 403 `csrf_rejected` (SameSite=Lax sends the cookie on cross-site navigations) | W2 |
| POST | `/api/v1/admin/backup` | `admin` | none | `{path, size}`. Single-flight, also against `quire-server admin backup` in another process (429 `busy`, §12); no request deadline; counts against `QUIRE_BACKUP_KEEP` (with 0, only the new file is kept) | W2 |

Reserved and not implemented in M1: `/api/v1/items/{id}/revisions*`, `/api/v1/blobs/{sha256}`,
`/api/v1/items/{id}/doc/ws`, `/api/v1/items/{id}/board/ws`, `POST /api/v1/items/{id}/board/import` (D52), and
`/templates`.

### 7.3 Authentication methods and personal access tokens

- **Resolution** (`extract::Auth<S>`, D43, D44), before any other extractor of the handler:
  1. If `is_quire_bearer(headers)` (scheme `Bearer`, credentials starting `qpat_`; the CSRF layer calls the same
     function): the token must pass `quire_api::token::check` (51 characters, base62, CRC32), otherwise 401
     `invalid_token` with no database lookup. Then the row for `sha256(token)` is read on a reader with no liveness
     filter and classified (§5): live → authenticated; expired → 401 `invalid_token` with `details.reason =
     "expired"`; revoked, disabled user or no row → the same 401 with no reason. Nothing is cached. The cookie is
     ignored, and so are the CSRF checks (§8).
  2. Otherwise (no `Authorization`, or any other value, which is ignored): the session cookie (§8). None, or an expired
     session, is 401 `unauthenticated`.
  3. Scope: `Auth<Read|Write|Admin>` needs that scope in the token's set (a session has all), otherwise 403
     `insufficient_scope`. `Auth<Session>` answers any token with 403 `session_required`; `Auth<AnyToken>` answers a
     session with 403 `forbidden`.

  The result is `Actor{user_id, role, method: Session{session_id} | Token{token_id, scopes}}`; `authz::require` is
  unchanged (§8).
- **Format.** `qpat_` + 40 base62 characters from `getrandom` (about 238 bits) + 6 base62 characters encoding the CRC32
  of those 40 (D42).
- **Creation.** `POST /api/v1/tokens` (a session or an admin token), Settings → API tokens (§9.6.6), or
  `quire admin create-token` (§7.8). `expires_in_days` omitted means 90 days, `null` never. Scopes are stored expanded
  (`write` stores read and write) and cannot exceed the caller's (a session may grant any). A calling token cannot
  grant `admin`, and the new token cannot expire later than the caller (`parent_token_id` records the caller). A
  session caller re-enters its password for an `admin` or never-expiring token (argon2 under the same semaphore as
  login). The `201 {token, api_token}` response is the only place the secret ever appears; the server keeps
  `sha256(token)` and the 12-character prefix.
- **Use.** `Authorization: Bearer qpat_…` on any non-public route. `GET /api/v1/auth/me` returns `auth.token`, which is
  what `quire auth whoami` prints.
- **Last use.** An in-memory map remembers when each token's row was last written; a request writes `last_used_at` and
  `last_used_ip` through the writer, in a spawned task off the request path, only when that was more than 60 s ago.
- **Revocation.** `DELETE /api/v1/tokens/{id}`, `DELETE /api/v1/tokens/current` (a token revoking itself, used by
  `quire auth logout --revoke`) or `quire admin revoke-token`; revoking a token also revokes the tokens it created. It
  takes effect on the next request, because lookups are never cached. A password change revokes sessions, not tokens
  (separate credentials); `quire admin reset-password --revoke-tokens` and `quire admin revoke-token --all --yes` are
  the switches for a compromise. Deleting or disabling a user (later) ends their tokens (FK cascade, `disabled_at`
  check).
- **Rate limits** (D47): keyed governor limiters per token, `QUIRE_TOKEN_RATE` requests per minute (default 600, burst
  120) and `QUIRE_TOKEN_BYTES_PER_HOUR` of body (default 1 GiB); over either, 429 `rate_limited` with `Retry-After`.
  Bearer authentication failures count per IP (30 per minute). Over that, a token that fails lookup gets 429
  (`reason: "auth_failures"`) instead of 401, while valid tokens from that IP are served; malformed tokens never reach
  the database, but count against the per-IP limit too. Body bytes: a declared `Content-Length` is counted when the
  request authenticates, so an oversized declared body gets 429 before the handler runs; other bytes are counted as
  the body is read, one KiB each time the running total crosses a KiB boundary, so a body costs `ceil(size / 1024)` KiB
  however it is chunked, and a body that goes over the budget is cut off with 429 `bytes`. Heavy work stays bounded by
  the `board_work`, `doc_work` and `upload_work` permits (D25) whatever the rate.
- **What each scope covers** (the Auth column of §7.2): `read`, every GET except token and session management and
  export; `write`,
  content and structure (create, edit, move, publish, archive, trash, restore, stars, uploads); `admin`, permanent
  deletion (Trash purge), workspace and account settings, sessions, tokens, export and backup. Password change and
  logout are session-only.

### 7.4 OpenAPI spec (`quire-server::openapi`, D41)

- Every route module returns `OpenApiRouter<AppState>` (utoipa-axum) and registers handlers only through
  `.routes(routes!(…))` on functions annotated with `#[utoipa::path(…)]`. `quire_server::app()` nests them under
  `/api/v1` and calls `split_for_parts()`, so the served router and the spec come from the same registrations. Plain
  `Router::route` is never used under `src/routes/`, and no `.route("/api` appears anywhere in the crate (two greps in
  the release gate). `/healthz`, static files and the SPA fallback are mounted outside `/api/v1` and outside the spec.
- Each operation declares its request and response bodies (the `quire-api` DTOs derive `ToSchema`), every error
  response as `ErrorBody` with its possible codes, its ETag and If-Match headers, and its security: `[]` for public
  routes (utoipa writes `security(())` as `[{}]`, and `openapi::spec()` rewrites it to `[]`),
  `[{bearer: [<scope>]}, {session: []}]` otherwise, `[{session: []}]` for session-only routes, and `[{bearer: []}]`
  for token-only routes (`Auth<AnyToken>`).
- `components.securitySchemes`: `bearer` (`type: http`, `scheme: bearer`, `bearerFormat: qpat`) and `session`
  (`type: apiKey`, `in: cookie`, `name: quire_session`; the description notes `__Host-quire_session` behind https).
- `info`: title "Quire API", `version` = `quire_api::API_VERSION`, licence MIT. The spec is built once at
  startup and served from memory at `GET /api/v1/openapi.json` with `Cache-Control: no-cache` and an ETag
  (`"o<the first 16 hex digits of its sha256>"`; `If-None-Match` ignores `W/` and accepts `*`). Each handler is
  registered with its own `routes!()`: utoipa-axum's `routes!` with handlers on several paths gives every path all
  their methods.
- **Committed copy** `docs/api/openapi.json` (pretty-printed, trailing newline). Tests in `tests/api/openapi.rs` and
  `tests/api/security.rs`:
  - `openapi::spec_matches_committed` fails on any difference; with `UPDATE_OPENAPI=1` it rewrites the file instead
    (`scripts/dev.sh env UPDATE_OPENAPI=1 cargo test -p quire-server --locked --test api -- openapi::`);
  - `openapi::spec_parses_as_3_1` parses the file with oas3 0.22 and checks `openapi == "3.1.0"` and that every path
    starts with `/api/v1/`;
  - `security::every_operation_enforces_its_declared_scope` walks the spec and calls every operation with random UUIDs
    as path parameters and `{}` as the body: without credentials it expects 401 (public routes excepted); with a token
    one scope below the declared one, 403 `insufficient_scope`; on session-only routes, any token gets 403
    `session_required`; on token-only routes, a session gets 403 `forbidden`;
  - `security::public_operations_are_exactly` fails unless the operations with security `[]` are exactly
    `GET /config`, `GET /openapi.json`, `GET /setup`, `POST /setup` and `POST /auth/login`.
- utoipa's maps are ordered (`preserve_order` and `preserve_path_order` stay off), so the output does not depend on which route module registers
  first, and a merge conflict in the committed file is resolved by regenerating it (PLAN R4).
- No interactive documentation UI in M1; any OpenAPI viewer can open the file.

### 7.5 Content over the API

Documents have three representations, all `read` to GET and `write` to change:

| Representation | Routes | Fidelity | Typical client |
|---|---|---|---|
| Yjs v1 updates (base64 in JSON) | `GET …/doc` (`?since=`), `POST …/doc/updates` | Exact: the stored form | The web editor; tools with a Yjs library |
| ProseMirror JSON (`DocJson`) | `GET`/`PUT …/doc/prosemirror` | Lossless for schema v1 (§6.4) | Programs that build documents |
| Markdown (Quire Markdown 1) | `GET`/`PUT …/doc/markdown` | Lossy as listed in §6.4 | Agents, the CLI, "Export Markdown" |

A whole-document PUT is a diff against its `If-Match` base and is stored as one more Yjs update (§6.4, D50). Clients
edit with a read-modify-write loop: GET (keep the ETag), change, PUT with `If-Match`. On 412 they either GET again and
redo the change, or resend with `?merge=true` while `details.mergeable` is true. `GET /api/v1/schema/doc` describes the
nodes and marks a ProseMirror PUT may use; `GET /api/v1/items/{id}/text` returns the derived plain text.

Boards are read and written as the native Inkboard envelope with exactly the web path's rules (§6.2, D51): the server
validates with `inkboard_core::serial::from_json` (`inkboard-core/src/serial.rs:80`), splices on the raw JSON so unknown
fields survive, splits inline base64 images into blobs, and owns `document.name`. PUT needs `If-Match: "r{rev}"` (or
`*`). Previews are `GET …/board/pages/{n}` (`image/svg+xml`, `svg::export_approx`, `inkboard-core/src/svg.rs:456`).
Server-side import from draw.io, Excalidraw, Mermaid or JSON Canvas is not in M1 (D52), and Quire's tooling does not
build Inkboard's `ink convert` (`crates/inkboard-cli`) or `inkboard-mcp` (tools such as `create_board`, `add_shape`,
`import_mermaid` and `export`, `crates/inkboard-mcp/src/tools.rs:69-130`) either; neither ships in Quire's image. A
client that builds a board from scratch follows Inkboard's model (`vendor/inkboard/docs/ARCHITECTURE.md`, "Model";
`crates/inkboard-core/src/{model,serial}.rs`) and PUTs the envelope; the server validates it with `from_json`.

Attachments: `POST /api/v1/attachments` (body sent as `application/octet-stream`) returns `{id, url}`; a document
references the file as `![alt](/api/v1/attachments/<id>)` in Markdown or as `image{attachmentId}` in JSON.

### 7.6 CLI (`crates/quire-cli`, binary `quire`, D54)

- **Transport.** One ureq 3.5 agent (`rustls-no-provider`, with rustls's `ring` provider passed in `TlsConfig`). Roots:
  `--ca-file` / `QUIRE_CA_FILE` (PEM) when set, else the system bundle (`SSL_CERT_FILE`, else the first of
  `/etc/ssl/certs/ca-certificates.crt`, `/etc/pki/tls/certs/ca-bundle.crt`, `/etc/ssl/ca-bundle.pem`,
  `/etc/ssl/cert.pem`), parsed with `ureq::tls::parse_pem` into `RootCerts::Specific`; an https URL with no bundle
  exits 3. No flag skips verification. For a LAN install on Caddy's local CA (§13.4), add its root to the system
  bundle (then no flag is needed) or pass `--ca-file quire-local-ca.crt`. Accepts gzip; sends `User-Agent: quire-cli/<version>`. Timeouts: 30 s
  (`--timeout`), 300 s for transfers (export, boards, attachments). Redirects are never followed (`max_redirects(0)`,
  so a token never travels to another URL): a 3xx exits 3 and names the `Location` (usually a proxy's http→https
  redirect: fix the URL). Plain `http://` to a non-loopback host is refused (exit 3) unless `QUIRE_ALLOW_HTTP=1` (or
  `true`) or the profile sets `allow_http = true`. No proxy is ever used (`HTTPS_PROXY` and the like are ignored), so the
  token goes only to the configured origin. A URL is `scheme://host[:port]` with an optional trailing `/`; a path, query
  or user info is exit 3.
- **Retries** (`--retries`, default 2; after `Retry-After`, capped at 60 s, or else 1 s and then 3 s). Every method is
  retried when the server answered 429 or 503 (it did not process the request) or when the request never left (DNS,
  connect, TLS handshake). GET and HEAD are also retried after a timeout or reset, including one while a body is read
  into memory (within the same `--retries` budget); a streamed download (`export`, `attachment get`, `board preview`,
  any `--out`, a non-JSON `api` answer) is not retried once it started, because part of it may already be written.
  POST, PUT, PATCH and DELETE are never retried once sent: the outcome is unknown, and the error says so
  (`details.sent: true`, §7.7). "Not sent" is decided by the stage: every error of the resolver and of the connect
  stage (TCP connect and the TLS handshake, which ureq completes before it writes the request) is `sent: false`, and
  anything later, a TLS error while the response is read included, is `sent: true`; a pooled connection skips the
  connect stage, so a failure on it counts as sent. A repeated document PUT is a no-op on the server anyway (§6.4).
- **Configuration.** The URL and the token are resolved as a pair, so a profile's token is never sent to a URL that came
  from somewhere else:
  - with `--profile P`: URL = `--url`, else P's `url`; token = `--token-stdin` or `--token-file`, else P's token only
    when the URL is P's. `QUIRE_URL`, `QUIRE_TOKEN` and `QUIRE_TOKEN_FILE` are ignored;
  - otherwise: URL = `--url`, `QUIRE_URL`, the profile named by `QUIRE_PROFILE` or `default_profile`, or (inside the
    server container, where `QUIRE_LISTEN` is set) `http://127.0.0.1:<its port>`; token = `--token-stdin`,
    `--token-file`, `QUIRE_TOKEN`, `QUIRE_TOKEN_FILE`, then that profile's token only when the URL came from the same
    profile.

  A URL with the same origin as the profile's `url` (scheme, host and port, with case and default ports normalized)
  counts as the profile's URL. With `--profile P`, `QUIRE_CA_FILE` and `QUIRE_ALLOW_HTTP` still apply. A relative
  `token_file` in a profile is relative to the profile file. A profile token that would pair with a foreign URL is an
  error (exit 3, `cli_config`: "token not bound to this URL").
  `--token-stdin` together with `--token-file` is a usage error. The profile file is `$XDG_CONFIG_HOME/quire/config.toml`
  (etcetera 0.11; `~/.config/quire/config.toml` by default) or `QUIRE_CONFIG`. The CLI creates its directory with mode
  0700 and writes the file as a 0600 temporary file renamed into place. A profile file that holds a `token` and is
  readable by group or others is refused (exit 3), as ssh does with keys; a group-readable token file only warns
  (Docker secrets are 0444).

  ```toml
  default_profile = "home"

  [profiles.home]
  url = "https://wiki.example.org"
  token_file = "/run/secrets/quire-token"   # or: token = "qpat_…"
  ca_file = "/etc/ssl/private-ca.pem"       # optional
  allow_http = false                        # optional; true allows a non-loopback http:// URL
  ```
- **Non-interactive.** Nothing prompts, opens a browser or a pager, or prints colour. Content comes from a file argument,
  or from stdin with `-`. Tokens come from `--token-stdin`, `--token-file`, the environment or the profile, never from
  argv. `trash purge` needs `--yes`. Content writes need `--if-match ETAG` (`d42`, `"d42"` and `W/"d42"` are all
  accepted) or `--force` (`If-Match: *`); without either the command exits 2 instead of guessing. Human output (without
  `--json`) shows control characters (C0, DEL and C1; `\n` too inside a column or a tree line) as escapes such as
  `\u{1b}`, so a server-supplied title or message cannot move the cursor or set colours; `--json` output and raw content
  (`doc get` and `board get` without `--json`, downloads) are unchanged.
- **Memory.** The CLI keeps at most one copy of a body: wrapped content is a `RawValue`, never a value tree, and
  downloads (export, attachments, boards, previews) stream to stdout or `--out`. Inside the container it shares the
  app's 1 GiB (D25).
- **Logging.** No `RUST_LOG` logger is installed (the container's `RUST_LOG` is the server's). `--debug` prints method,
  URL, status, duration and request id per request to stderr, never a header.
- **References.** Wherever a command takes an item or a collection (`<i>`, `<c>`), it is a UUID, a `url_id`, or a Quire
  URL (`…/doc/<slug>-<url_id>`, from which the CLI takes the `url_id`). A `url_id` or URL costs one
  `GET /items/{url_id}` (or `/collections/{url_id}`) before every route but those two GETs, since only they take a
  `url_id` (§7.1); positions, `--parent` and `--collection` resolve the same way.
- **Global flags:** `--json`, `--url`, `--profile`, `--token-file`, `--token-stdin`, `--timeout`, `--retries`,
  `--ca-file`, `--debug`. List commands also take `--limit`, `--cursor` and `--all` (follows `next_cursor`, at most
  10,000 items). "position" below is `--after <ref>` or `--before <ref>`; neither means last.
- **Commands.** This table is the contract for agents (R3 applies to it); `quire help --json` prints the same tree.
  "→" is the `--json` stdout; "ok" is the body-less success object of §7.7; "binary" is raw bytes to stdout or `--out`
  (with `--json`, `--out` is required and stdout gets `{path, bytes, sha256, content_type}`).

  | Command | API | → |
  |---|---|---|
  | `auth login --url U (--token-stdin \| --token-file F) [--profile P]` | `GET /auth/me`, then writes profile P (default `default`) with `token` or `token_file` (the absolute path), keeping its other keys and the other profiles; `default_profile` is set only when the file has none | `Me` |
  | `auth logout [--revoke]` | removes the profile's token; `--revoke` first calls `DELETE /tokens/current` with the profile's own token at the profile's URL, never with a `QUIRE_TOKEN`, token file or stdin token in play (no profile token: exit 3 before any request) | ok, plus `profile` and `removed` |
  | `auth whoami` | `GET /auth/me` | `Me` |
  | `me update [--name N] [--email E] [--theme light\|dark\|system]` | `PATCH /auth/me` (`--email` needs a session: exit 6 with a token) | `{user}` |
  | `session list` · `session revoke (<id> \| --others)` | `/auth/sessions` | the list · ok |
  | `token list` · `token create --name N --scope read\|write\|admin [--expires 30d\|never] [--token-out F]` · `token revoke <id>` | `/tokens` | `[ApiToken]` · `{token, api_token}`, or with `--token-out` (F written 0600) `{token_file, api_token}` · ok |
  | `config show` · `config path` | local. The sources are named by flag or variable: `url_source` is `--url`, `QUIRE_URL`, `profile` or `QUIRE_LISTEN`; `token_source` is `--token-stdin`, `--token-file`, `QUIRE_TOKEN`, `QUIRE_TOKEN_FILE` or `profile` | `{profile, url, url_source, token_source, token_prefix}` · `{path}` |
  | `workspace get` · `workspace set [--name N] [--logo <attachment> \| --no-logo]` · `sidebar` | `/workspace`, `/sidebar` | the body |
  | `collection list` · `get <c>` · `create --name N [--description D] [--icon I] [--color '#RRGGBB'] [position]` · `update <c> [--name N] [--description D] [--icon I] [--color C] [--sort manual\|title\|updated\|created]` · `move <c> position` · `delete <c>` · `restore <c>` · `updated <c>` | `/collections…` | the body |
  | `item get <i>` | `GET /items/{id}` | `ItemDetail` |
  | `item create --collection <c> --kind doc\|board [--parent <i>] [--title T] [position] [--md F\|- [--title-h1]]` | `POST /items`. The item is published on create (D23; `item unpublish` makes it a draft). With `--md` (docs only): the item is created without a title, then `PUT …/doc/markdown` with `If-Match: "d0"`, then `PATCH` of `--title` only once the PUT succeeded. If the PUT fails, the new empty item is discarded (`DELETE ?discard_empty=true`; a failed discard adds `discard_failed`) and the PUT's error is reported; a failed title PATCH after a good PUT is the warning `item_created`. The file is read before the POST (an unreadable one creates nothing), `--kind board` with `--md` is exit 2, and `--title-h1` without `--title` prints the item from `GET /items/{id}` | `ItemSummary`; with `--md` `{item: ItemSummary, doc: DocWriteResult}` |
  | `item update <i> [--title T] [--full-width true\|false] [--if-match E]` | `PATCH /items/{id}` | `ItemSummary` |
  | `item move <i> [--parent <i> \| --top] [--collection <c>] [position]` | `POST …/move`. With neither `--parent` nor `--top` the item keeps its parent (a reorder): the CLI reads the item and sends its current `parent_id`. `--collection` needs `--parent` or `--top` (exit 2 otherwise) | `ItemSummary` |
  | `item duplicate\|unpublish\|archive\|unarchive\|restore <i>` · `item publish <i> [--with-ancestors]` · `item trash <i> [--discard-empty]` · `item children <i>` · `item text <i>` | `/items/{id}…` | the body |
  | `tree <c>` | `GET /collections/{id}/tree` (human output indented) | `{collection, items}` |
  | `list recent\|updated\|drafts\|starred\|archive` · `trash list` · `trash purge (<i> \| --collection <c> \| --all) --yes` | lists, `/trash` | `Page` · the body · ok |
  | `search <query> [--collection <c>] [--kind doc\|board] [--updated-after T] [--include-archived] [--no-drafts] [--sort relevance\|updated]` · `search --titles <q> [--limit N]` | `/search`, `/search/titles`; `--titles` refuses `--all`, `--cursor`, a `--limit` above 20 and the full-text filters (exit 2) | `Page<SearchHit>` · the list |
  | `star add <i> [position]` · `star remove <i>` | `PUT`/`DELETE /items/{id}/star` | ok |
  | `doc get <i> [--format md\|json] [--title-h1] [--out F]` | `GET …/doc/markdown\|prosemirror` | `{item_id, format, etag, content}` (`content`: a string for Markdown, an object for JSON); with `--out` `{item_id, format, etag, path, bytes, sha256}` |
  | `doc put <i> (--md F\|- \| --json F\|-) (--if-match E \| --force) [--merge] [--title-h1] [--attach-images [--base-dir D]]` | `PUT …/doc/markdown\|prosemirror`. The input flag is `--json-input F`: clap cannot hold the global `--json` beside a local `--json FILE`, so inside `doc put` a `--json` that follows the item and is followed by a value (`-` or a word without a leading `-`), and `--json=F`, are rewritten to `--json-input` before parsing; `quire --json doc put …` and a trailing `--json` stay the output mode. A bare `{"type": "doc", …}` is wrapped in `{"schema_version": 1, "doc": …}` | `DocWriteResult` |
  | `doc schema` | `GET /schema/doc` | the body |
  | `board get <i> [--out F]` · `board put <i> F\|- (--if-match E \| --force)` · `board preview <i> [--page N] [--out F]` | `/items/{id}/board…` (preview without `rev`: the current one) | `{item_id, format: "inkboard", etag, content}` · `{rev, etag, updated_at, meta}` · binary |
  | `attachment upload F\|- [--item <i>] [--filename N]` · `attachment get <id> [--out F]` | `/attachments`, sent as `application/octet-stream` | `{id, url, sha256, mime, size, width?, height?}` · binary |
  | `export [--out F]` · `backup` | `GET /export` · `POST /admin/backup` (a file on the server; `quire admin backup` is the local one) | binary · `{path, size}` |
  | `api <METHOD> <path> [--input F\|-] [--header K:V]… [--out F]` | any route, with auth, retries and error mapping, as `gh api` | the response body (non-JSON: binary) |
  | `admin …` | execs `quire-server admin …` (§7.8, D55) | as §7.8 |
  | `version` · `exit-codes` · `help [--json]` | local, plus `GET /config` (sent without a token) for `version`, which never fails because of the server: no URL gives `server: null`, an unreachable server also a `server_unavailable` warning | `{cli: {version, api_version}, server: {version, commit, api_version} \| null}` · the §7.7 table · the command tree (commands, arguments, flags, which are required, exit codes) |
- **Guards.**
  - `api`: `<path>` must start with `/api/` and contain no scheme, no `//`, no `\` and no `.` or `..` segment (also
    percent-encoded); the URL is the configured origin plus the path by concatenation, never `Url::join`. `--header`
    refuses `Authorization`, `Cookie` and `Host`, and checks the header name's syntax; the path guard also refuses
    control characters, spaces, `#` and an encoded `/` or `\`. A refusal exits 2 before any request. Methods are GET,
    POST, PUT, PATCH, DELETE and HEAD; `api` uses the 300 s transfer timeout (any route may be a download), `--out F`
    writes any answer, JSON too, and with no token configured it calls the route without one (public routes).
  - `--attach-images` uploads only images referenced by relative paths that canonicalize (symlinks resolved) to a file
    under the Markdown file's directory or `--base-dir`, whose magic bytes are png, jpeg, gif, webp or avif, and whose
    size is within `max_upload_mb`. It rewrites their URLs before the PUT and reports every skipped reference as an
    `attach_skipped` warning. With stdin input it needs `--base-dir`.
  - `admin` finds `quire-server` through `QUIRE_SERVER_BIN`, then next to the `quire` binary, then on `PATH`; none
    gives exit 3.
- **Coverage.** Every operation of the spec has a command, except this allow-list of web-only operations, which stay
  reachable through `api`: `GET`/`POST /setup`, `POST /auth/login`, `POST /auth/logout`, `POST /auth/password`,
  `POST /items/{id}/view`, `GET …/doc` and `POST …/doc/updates`; and two that need no command of their own,
  `GET /openapi.json` (the spec itself) and `GET /items/{id}/board.inkboard` (the browser download; `board get` reads
  the same content). A test checks the spec against the clap tree and this list.

  `board import` is reserved (D52). The CLI warns once (`api_version_older`, §7.7) when a response's
  `Quire-Api-Version` has a lower minor than its own; a missing route then exits 7 with the server's 404.

### 7.7 CLI output and exit-code contract

- **stdout** carries only the result.
  - Without `--json`: short human text (lists as columns of id, kind, title and updated time). This format is not
    stable.
  - With `--json`: exactly one JSON document, the "→" column of §7.6. For an API call it is the response body unchanged
    (unknown fields kept). A body-less success (204) prints `{"ok": true, "status": 204, "request_id": "…"}`, plus
    `"id"` of the target when there is one. `--all` prints one merged `{"items": […], "next_cursor": …}`: `null` when
    the list ended, the real cursor when the 10,000 cap stopped it (with a `truncated` warning). If any page fails,
    stdout stays empty and only the error is printed.
  - Without `--json`, content commands write the content raw to stdout (or to `--out`) and the ETag to stderr as
    `etag: "d42"`.
- **stderr.** Without `--json`: human lines (errors, warnings, the `etag:` line, one line per conversion warning). With
  `--json`: JSON Lines only, so `jq` can read it: zero or more `{"warning": {"code", "message", "details"?}}` lines (CLI
  warnings: `truncated` with `{count, next_cursor}`, `attach_skipped` with `{path, reason}`, `api_version_older`,
  `token_file_permissions`, `discard_failed` and `item_created` with the new item's `id`, `server_unavailable`),
  `{"debug": {…}}` lines with `--debug`, then at most one error object. The error is the API's `ErrorBody` plus
  `"status"` and `"exit_code"`; errors raised by the CLI itself use the same envelope with `request_id: null`,
  `"status": null` and the codes `cli_usage` (exit 2; also an unreadable input file), `cli_config`, `network`,
  `timeout`, `tls`, `bad_response` (exit 1, an unparseable response) or `cli_io` (exit 1, a local file could not be
  written). An HTTP error whose body is not an envelope (a proxy's page) gets `code: "http_error"` and its status's exit
  code; a 3xx is `cli_config` with `details.location`. Network and timeout errors carry `details.sent`: `false` when
  DNS, connect or TLS failed before the request left, `true` when it may have reached the server. When an argv entry
  is exactly `--json`, clap usage errors use this form too: the message is clap's first paragraph on one line, so it
  names the missing or unexpected arguments (`the following required arguments were not provided: <--if-match
  <ETAG>|--force>`). Conversion warnings of a PUT are in the result
  (`warnings`), and `doc get --format md --json` adds the codes of `Quire-Conversion-Warnings` as `warnings`. API bodies
  are printed byte for byte (plus a newline); objects the CLI builds are pretty-printed.
- **Exit codes** (`quire_api::cli::ExitCode`, shared with `quire-server admin`; `quire exit-codes --json` prints them
  as `[{"code": 0, "name": "success", "meaning": "…"}, …]`). The table is part of the contract: changing a code is a
  new major CLI version. `docs/AGENTS.md` repeats it, and a test compares the two.

  | Code | Meaning | Typical causes |
  |---|---|---|
  | 0 | Success | |
  | 1 | Unexpected error | A CLI bug or an unparseable response, nothing else |
  | 2 | Usage | Bad flags; a missing `--yes`, or `--if-match`/`--force`; a refused `api` path or header |
  | 3 | Configuration | No URL or token; a malformed token (prefix, length or checksum); a token not bound to the URL; an unreadable or group-readable profile; plain HTTP to a non-loopback host; no CA bundle; a 3xx redirect; `quire-server` not found for `admin`; 421 `host_not_allowed` |
  | 4 | Network | DNS, connection refused, TLS failure, timeout or reset; `details.sent` says whether a write may have happened |
  | 5 | Not authenticated | 401: `unauthenticated`, `invalid_token`, `invalid_credentials` |
  | 6 | Forbidden | 403: `insufficient_scope`, `session_required`, `forbidden`, `csrf_rejected` |
  | 7 | Not found | 404 |
  | 8 | Conflict | 409: `cycle`, `parent_invalid`, `read_only`, `not_empty`, `missing_deps`, `setup_required`, `already_setup`, `schema_too_new`, `conflict`; for `admin create-owner`, an owner exists |
  | 9 | Precondition failed | 412 `rev_mismatch`, 428 `precondition_required`: GET again (or use `--merge`) and retry |
  | 10 | Invalid input | 400, 405, 413, 415, 422, and any other 4xx not listed here |
  | 11 | Rate limited | 429 after the retries |
  | 12 | Server error | 408, 500, 503, 507 and any other 5xx, after the retries |

### 7.8 Admin commands (`quire-server admin`, D55)

They open the database in `QUIRE_DATA_DIR` the way `serve` does (pending migrations included, with the pre-migration
backup) and may run while the server is up: SQLite in WAL mode allows a second process, and its writes wait on
`busy_timeout` (5 s); the running server sees a new owner on its next request (§8). In the container they run as uid
65532 (`docker compose exec` uses the image's user). Every command takes `--json` and uses the exit codes of §7.7, and
prints secrets only to stdout or `--token-out`, never to the log. The CLI's `quire admin …` reaches them (D55).

| Command | Effect |
|---|---|
| `create-owner --email E --name N --workspace W (--password-stdin \| --generate-password) [--no-seed]` | The `POST /setup` transaction without the setup code: creates the owner and the workspace settings and seeds "Getting started" (unless `--no-seed`). A generated password is printed once and sets `must_change_password`. Exit 8 when an owner exists |
| `create-token --name N --scope read\|write\|admin [--expires 90d\|never] [--email E] [--token-out F]` | Creates a token for the owner (or for E) with `created_via = 'admin'` and prints it once (`--json`: `{token, api_token}`; with `--token-out` the token goes to F, written 0600, and stdout has `{token_file, api_token}`) |
| `list-tokens` · `revoke-token (<id> \| --all --yes)` | As the API, without HTTP |
| `reset-password --email E [--revoke-tokens]` | D35; `--revoke-tokens` also revokes every API token of that user |
| `backup [--out PATH]` | `VACUUM INTO` on the maintenance connection, then `quick_check` (§12) |
| `check` | §5 |

With `--json` the result is one JSON document on stdout; an error is one stderr line
`{"error": {…}, "request_id": null, "status", "exit_code"}`:
- `create-owner`: `{user, workspace, generated_password}` (`null` with `--password-stdin`). A generated password has 24
  base62 characters and sets `must_change_password`; `--password-stdin` strips one trailing newline and takes 12–1024
  bytes, otherwise exit 10. It also inserts `workspace.name`, `instance.id` and `instance.created_at` when missing.
- `create-token`: `{token, api_token}`, or `{token_file, api_token}` with `--token-out`. `--expires` is `<n>d` (1–3650)
  or `never`, 90 days by default; an unknown `--email` exits 7, no owner exits 8. Without `--json` stdout holds only the
  token.
- `list-tokens`: `[ApiToken]`. `revoke-token`: `{revoked}`; an id that is not a UUID is 404 (exit 7).
- `reset-password`: `{user, password, sessions_revoked, tokens_revoked}`.
- `backup`: `{path, size}` (`BackupResult`). `--out` refuses an existing file (exit 10); a backup already running, in
  this or another process, is 429 `busy` (exit 11).
- `check`: `{ok, quick_check, foreign_key_violations, cycles, fts, missing_blobs}`; when something is wrong it exits 1,
  with the report still on stdout.

`serve` and `healthcheck` read the same environment. A bare `quire-server serve` listens on `QUIRE_LISTEN` (default
`127.0.0.1:8080`) with its data in `QUIRE_DATA_DIR` (default `./data`); the image sets `0.0.0.0:8080` and `/data`.
`healthcheck [--url URL]` probes `/healthz` (a URL without a path gets it appended; `http://` only) with a 3 s timeout
and exits 0 only for 200. Its default URL is `QUIRE_LISTEN`'s port on 127.0.0.1 when the listen address is unspecified
(`0.0.0.0`, `[::]`), and the listen address itself otherwise (`127.0.0.1`, `[::1]`, a LAN address), because a server
bound to one address does not answer on another. `quire-server --version` prints `quire-server 0.1.0 (<QUIRE_COMMIT or
unknown>)`; `quire --version` prints `quire 0.1.0`, without a commit (the cli stages pass no `QUIRE_COMMIT`).

From the host: `docker compose exec quire quire admin create-token --name ci --scope write --expires 1d --json`.

---

## 8. Authentication and security

- **Setup.**
  - While `users` has no owner, the server generates a 12-character code (`XXXX-XXXX-XXXX`, from 31 symbols without 0,
    O, 1, I and L: about 59 bits) unless `QUIRE_SETUP_TOKEN` is set, and logs `First-run setup: <public_url>/setup code
    …` at startup. The code is compared ignoring case and surrounding spaces, in constant time; a wrong one is 401
    `invalid_credentials` (§7.2 has the order of the checks).
  - `POST /api/v1/setup` runs one IMMEDIATE transaction guarded by `NOT EXISTS (SELECT 1 FROM users)`. It creates the owner
    and the settings, seeds "Getting started" (`fixtures/ydoc/getting-started.bin`,
    `fixtures/ydoc/writing-in-quire.bin` and `fixtures/boards/getting-started.inkboard`, all embedded at build time,
    §9.6.7), and signs the owner in. It shares the per-IP
    login limiter.
  - The server caches whether an owner exists. While the cache says no, every request re-checks with one reader query
    and flips it once, so an owner made by `quire admin create-owner` while the server runs is seen on the next
    request, and the setup code stops working.
- **Login.**
  - argon2id verify runs under `spawn_blocking` behind `Semaphore(2)`. When the email is unknown, a dummy hash is
    verified anyway, so the timing is the same.
  - The error is always the generic `invalid_credentials`.
  - Per IP: governor, 5 attempts per minute (burst 5), shared with setup, counting every attempt (429, `reason:
    "requests"`). Per email: 10 **failed** attempts in a sliding 15-minute window (429, `reason: "auth_failures"`; §17 on
    lockout), checked before the password is verified. That one is not governor, which cannot test a key without
    consuming it, while only failures may count; its keys are the sha256 of the trimmed, lowercased address. An address
    that fails `users::check_email` (over 254 bytes, no `@`, whitespace) cannot belong to an account: it gets the same
    401 after the same dummy verify and is neither checked nor recorded, so failed logins cannot fill memory with keys.
  - The limiters run on one injectable clock, so tests need no sleeps. `Retry-After` is in whole seconds, at least 1.
    A request without a peer address (an in-process transport) is keyed as `0.0.0.0`: such requests fail closed and
    share one budget.
- **Client address and scheme** (`quire-server::config` parses `QUIRE_TRUSTED_PROXIES`: comma-separated IPs or CIDRs).
  Quire has no TLS of its own (D29); Caddy terminates it (D57).
  - The client IP is the socket peer, and the request counts as plain HTTP, unless the peer is in
    `QUIRE_TRUSTED_PROXIES`. From a trusted peer, the rightmost `X-Forwarded-For` address is the client IP and
    `X-Forwarded-Proto: https` marks the request as HTTPS. From any other peer both headers, and `Forwarded`, are
    ignored, so a client that reaches Quire directly can neither choose its rate-limit key nor claim HTTPS.
  - In compose the only trusted peer is Caddy's fixed address (`QUIRE_PROXY_IP`, §13.3). Caddy trusts no proxy in front
    of it, so it discards any `X-Forwarded-*` a client sends and sets exactly one `X-Forwarded-For` address.
  - The per-IP limits (login, setup, failed bearer auth), the access log, `sessions.ip` and `last_used_ip` use this
    client IP. A deployment that replaces Caddy with another proxy must set `QUIRE_TRUSTED_PROXIES` to it (README);
    without it every per-IP limit sees one address.
- **Public URL.** `QUIRE_PUBLIC_URL` when set (compose passes unset variables as empty strings; an empty value counts
  as unset here and for every other `QUIRE_*` variable); otherwise, when `QUIRE_DOMAIN` is set, `https://<QUIRE_DOMAIN>`
  plus `:<QUIRE_HTTPS_PORT>` unless that is 443; otherwise `http://localhost:<the QUIRE_LISTEN port>`, which is
  `http://localhost:8080` for a bare `quire-server serve` (§7.8). It names the setup link in the log, the
  same-instance URLs of §6.4 and the CSRF origin below. The public URL and the CORS origins are `scheme://host[:port]`
  with no path; `QUIRE_DOMAIN` has no port (it goes in `QUIRE_HTTPS_PORT`); a malformed value of any variable stops
  `serve` with `VAR: message` and exit 3.
- **Cookie and renewal.**
  - `quire_session=<b64url token>; HttpOnly; SameSite=Lax; Path=/`. Persistent sessions (`remember`) add
    `Max-Age=<seconds until expires_at>`; others are browser-session cookies.
  - On an HTTPS request (above) the name is `__Host-quire_session` and `Secure` is added; on a plain-HTTP request (the
    direct e2e and agent paths, a bare `quire-server serve`) it is `quire_session`. The server reads only the name that
    matches the request's scheme.
  - Login rotates the token (it deletes the session the browser still presents). On an authenticated request, when
    less than half of the window remains, the server sets `expires_at = now + window` (persistent: window 30 d, capped
    at `created_at + 180 d`; otherwise 24 h) and, for persistent sessions, re-sends `Set-Cookie` with the new `Max-Age`.
    A due renewal, or a `last_seen_at` older than 5 minutes, is one small write on the request path, and `sessions.ip`
    follows the latest client address. A password change revokes every other session.
  - Logout clears the cookie of the request's scheme with `Max-Age=0`; a renewal due on the same request is dropped
    (the headers layer never adds a renewed cookie to a response that sets the session cookie itself).
- **Host allowlist** (`quire-server::csrf`, every route except `/healthz`). `Host` must be the host[:port] of
  `QUIRE_PUBLIC_URL`, `QUIRE_DOMAIN` with any port (Caddy forwards the client's `Host`, which carries the port when the
  host remaps 443), an entry of `QUIRE_ALLOWED_HOSTS` (comma-separated), or `localhost`, `127.0.0.1` or `[::1]` with
  any port; otherwise 421 `host_not_allowed`. This blocks DNS rebinding; Caddy also routes only `QUIRE_DOMAIN` to Quire.
  It applies to bearer requests too: an external client must reach Quire under the public URL's host or an allowed host
  (for example `QUIRE_ALLOWED_HOSTS=quire` for another container on the project network).
- **Bearer tokens** (§7.3, D42, D43). A request for which `is_quire_bearer` holds (`Authorization: Bearer qpat_…`) is
  authenticated by the token alone and skips the CSRF checks below; the cookie on such a request is ignored. Any other
  `Authorization` value is ignored (logged once per process at debug level), so a Basic-auth or oauth2-proxy header
  neither breaks the web client nor skips CSRF. Tokens are looked up by sha256 with no cache, so a revocation applies
  to the next request. Session-only routes refuse tokens (403 `session_required`).
- **CSRF** (same module). For POST, PUT, PATCH and DELETE that are not Quire bearer requests:
  - allow `Sec-Fetch-Site` of `same-origin` or `none`;
  - otherwise allow an `Origin` whose host[:port] equals that of `QUIRE_PUBLIC_URL` or the request's `Host` (already
    allowlisted; default ports normalized). The scheme is not compared: host[:port] already pins the origin to Quire,
    and a proxy other than Caddy may not send `X-Forwarded-Proto`. Quire reads `Host` itself, never
    `X-Forwarded-Host`, for this rule and for the allowlist, so the trusted-proxy rule above cannot widen either;
  - otherwise answer 403 `csrf_rejected`, including a request with neither header: browsers send `Origin` on every
    non-GET request, and non-browser clients use tokens.

  Browsers send `Sec-Fetch-*` only to secure origins: through Caddy they always send it, and on the plain-HTTP paths
  (direct access on the project network, a bare `quire-server serve`) the `Origin` rule carries the check (tested). The one expensive cookie GET, `/api/v1/export`, has its own rule (§7.2). Mutating routes take only `application/json`, or the route's declared type (`text/markdown`, raw upload
  bytes), and never a CORS-safelisted form type (415, §7.1).
- **CORS** (D48). Off by default: no CORS headers are sent. With `QUIRE_CORS_ORIGINS` set (exact
  `scheme://host[:port]` origins; `*` or a malformed origin stops startup), tower-http `CorsLayer` on `/api/v1/*`
  answers preflights from those origins, allows the methods GET, POST, PUT, PATCH and DELETE and the request headers
  `Authorization`, `Content-Type`, `If-Match`, `If-None-Match` and `X-Quire-Engine`, exposes `ETag`, `Retry-After`,
  `X-Request-Id`, `Quire-Api-Version` and `Quire-Conversion-Warnings`, uses `max-age` 600, and never sends
  `Access-Control-Allow-Credentials`. An allowed origin is not trusted by the CSRF check: cross-origin cookie requests
  stay rejected.
- **Rate limits.** Login and setup as above; per token (requests and body bytes) and per IP for failed bearer
  authentications (D47, §7.3).
- **Authorization seam.** Every handler takes `Auth<S>` as its first extractor (D44), which yields the `Actor`, and calls
  `authz::require(&actor, Action, Resource)` for single resources: the owner and `admin` may do everything, `member`
  reads and writes, `viewer` reads (M1 has only the owner). List-shaped store functions take a `Viewer` and go through
  `shared::visible_items` (§5). Store functions take `actor_id`; content writes take `WriteBy{user_id, token_id}` (D53).
  `GET /api/v1/attachments/{id}` authorizes through the
  items in `item_attachments` (M1: owner always allowed).
- **Uploads.** After the free-space check (D33), uploads stream to `/data/tmp` under the `upload_work` permit, hashed as
  they arrive (`BlobStore::stage_stream`), and are never buffered whole. The stored filename strips control characters (including CR and LF), `"`, `\` and `/`, is cut to 200
  characters, and defaults to `file`. Downloads send `Content-Disposition: <inline|attachment>; filename="<ASCII
  fallback>"; filename*=UTF-8''<percent-encoded>` (RFC 6266). The type is sniffed from the bytes (images, SVG, PDF,
  zip, gzip, UTF-8 text, the ISO media brands, WAV, WebM, Ogg, FLAC and MP3; the ISO brand decides before imagesize,
  which calls every `ftyp` HEIF), otherwise `application/octet-stream`. Image width and height come from the header only
  (`imagesize`); more than 100 megapixels gives 422. `BlobStore::path` takes a validated `Sha256` (64 lowercase hex).
- **Request limits.**
  - A deadline middleware (`app::deadline`, in place of tower-http's `TimeoutLayer`): 30 s for every request, except
    that uploads and board PUTs (`PUT /api/v1/items/*/board`) get 120 s for their body, and `GET /api/v1/export` and
    `POST /api/v1/admin/backup` have no deadline: their database copy runs before the response starts and can take
    longer than 30 s on a large database. Both are single-flight and the copy keeps the flag (§12), so they cannot pile
    up. Both deadlines answer the 408 envelope.
  - Tokio runs `worker_threads=2` (one per CPU of the 2-CPU cap, D25) and `max_blocking_threads=16` (both from env).
  - `MALLOC_ARENA_MAX=2`.
- **Secrets in logs and exports.** tower-http `SensitiveHeaders` hides `Authorization`, `Cookie` and `Set-Cookie`.
  Passwords and tokens are never logged. The access log is one span per request, `request{method, path, client_ip,
  token_id}`, with an info line for the status and latency and a debug line for the headers (the three above shown as
  `Sensitive`); colours only on a terminal, so `docker compose logs` is plain text. Exports carry no
  sessions, no API tokens and no password hashes (D34). Backups keep token hashes, which are useless without the
  238-bit secrets.

---

## 9. Frontend architecture

### 9.1 Routing (`web/src/app/router.tsx`, React Router data router)

| Path | Component (owner) |
|---|---|
| `/setup`, `/login` | `pages/Setup`, `pages/Login` (W3). These sit outside `AuthGate`. |
| `/` → redirect to `/home` | |
| `/home`, `/search`, `/starred`, `/archive`, `/trash` | `pages/*` (W3). No `/drafts` page in M1 (D23): the path falls through to `NotFound` |
| `/settings/:section` (`account`, `appearance`, `workspace`, `tokens`, `data` or `about`) | `pages/Settings` (W3) |
| `/collection/:slugId` | `pages/Collection` (W3) |
| `/item/:id` (a UUID) | `pages/ItemRedirect`: loads the item and replaces the URL with its `/doc/…` or `/board/…` page (Back skips it); unknown is `NotFound`. The DOM `href` of a document's item links (§9.6.5) |
| `/doc/:slugId` | `pages/DocPage` (W4), lazily imported |
| `/board/:slugId` | `pages/BoardPage` (W6), lazily imported |
| `*` | `pages/NotFound` (W3) |

How the pieces behave:
- `:slugId` is `<slug>-<url_id>`; the `url_id` is the last 10 characters.
- `AuthGate` loads `GET /api/v1/auth/me`:
  - 401 → `/login?next=`;
  - 409 `setup_required` → `/setup`;
  - `must_change_password` → `/settings/account?force=1`.
- `/setup` sends a visitor of a workspace that is already set up to `/login`, but after its own successful
  `POST /setup` (the owner is signed in, and the refetched status now says set up) it goes to `/home`, never through
  `/login`: the router's `navigate` is a transition, which the status re-render would otherwise overtake.
- After loading an item, the page calls `history.replace` to the canonical slug.
- **Updates.** When a lazy route chunk fails to load (W3) or `frame.html` returns 404 (W6), the client refetches
  `/api/v1/config`; if `version`, `commit` or `inkboard.base` differs from the first `/config` the page loaded (dev
  images keep version 0.1.0 between builds, so `commit` matters), it shows one banner "Quire was updated. Reload"
  instead of an error. Both paths call the same `checkForUpdate()` (`app/updateCheck.ts`), so only one banner can
  show. React Router 8.4.0 drops the rejection of a lazy `Component` on a route without a loader, so `lazyPage()`
  resolves a failed import to a component that throws it during render, and the route's `ErrorBoundary` runs the
  check.

### 9.2 Data and state

- **Server state:** TanStack Query. The keys are `['me']`, `['config']`, `['sidebar']`, `['tree', collectionId]`,
  `['item', id]`, `['children', id]`, `['list', 'recent'|'updated'|'starred'|'archive'|'trash']`,
  `['search', params]`, `['titles', q]` and `['tokens']`. A newly created token's secret is never put in the query
  cache: it lives only in the dialog's component state until the dialog closes.
  - Mutations (create, move, rename, star, trash) update `['tree']`, `['sidebar']` and `['item']`
    optimistically and roll back on error. Every mutation response replaces the affected rows with the server's rows,
    which carry the server `sort_key`.
  - `['tree', cid]` uses an ETag through `client.ts` (304 → keep the cache).
- **UI state:** Zustand stores in `web/src/state/`:
  - `ui.ts`: sidebar width and collapsed state, theme; persisted to `localStorage`.
  - `expanded.ts`: `quire.sidebar.expanded`.
  - `saveStatus.ts`: per-item save state.
  - `pending.ts`: per-item `{hasPending(), flush()}` registry, used by `beforeunload`, route changes and Ctrl/Cmd+S.
    The doc session, the board save loop and the title input (500 ms debounce → `PATCH /api/v1/items/{id}`) register here.
  - `toasts.ts`.
- **API client** (`web/src/api/client.ts`): `api<T>(method, path, {json?, body?, headers?, etag?})`, with paths
  relative to the constant `API_BASE = '/api/v1'`: the web client is a client of the public API (D39) and has no
  private routes. It returns `T` or throws `ApiError{status, code, message, details, requestId}`. A 401 sends an
  `auth-lost` event, which opens the sign-in dialog without leaving the page. Requests are always same-origin with
  `credentials: 'same-origin'` and never send `Authorization`. Uploads send `Content-Type: application/octet-stream`
  (the server sniffs the type; a browser's `text/plain` for a `.txt` file would get 415). Types come from
  `web/src/api/gen/*`.
- **Theme:** `web/public/theme-init.js` is a blocking external script in `<head>`. It sets `data-theme` from
  `localStorage['quire.theme']` before the first paint (CSP allows no inline script). The server copy (`users.theme`)
  wins after `/me` loads. Design tokens live in `web/src/styles/tokens.css` (§9.6.1).

### 9.3 Component tree

```
<App> QueryClientProvider > RouterProvider
  AuthGate
    AppShell                                  (W3) CSS grid: sidebar | main; position:fixed; overflow hidden on the shell
      Sidebar                                 (W3) §9.6.2
        WorkspaceHeader (logo, workspace name, user name, ▾ menu: Settings, Theme, Shortcuts, About, Sign out)
        NavList: Home, Search, Starred   (Drafts and Templates hidden, D23)
        StarredSection (drag reorder)
        CollectionsTree  role=tree; flat visible-row model → VirtualRows (>300)
          CollectionRow  [icon/caret][name][hover: sort ⇅, +, ⋯]
          ItemRow        [indent≤10 levels][caret][kind icon][title][hover: +, ⋯]; current = aria-current
          DropIndicator  (pragmatic-dnd tree-item hitbox, §9.6.3)
        NewCollectionRow
        SidebarFooter: Archive, Trash, Settings   (Invite hidden)
        ResizeHandle (216–400, dblclick reset)
      Main  <Outlet/>
        DocPage                               (W4)
          TopBar (W3): Breadcrumbs, SaveStatus, NewDocSplit, ItemMenu   [multiUser slot hidden; no PublishButton, D23]
          DocLayout: ContentsColumn (≥2 headings; ≥1280 docked, else popover) | DocColumn
            ItemHeader (W3): TitleInput, StarToggle, MetaLine
            QuireEditor (W4): TipTap + BubbleMenu + SlashMenu + MentionMenu + BlockHandle + ImageNodeView + BoardEmbedView + CalloutView
            NestedList (W4)
        BoardPage                             (W6)
          TopBar (W3) with editable title crumb + StarToggle + NestedPopover + FocusButton
          BoardSurface: measures its rect, registers with BoardFrameHost; shows BoardLoading / BoardPreview fallback / ConflictBanner
        Home, Search, Starred, Archive, Trash, Settings, Collection, NotFound   (W3)
      BoardFrameHost                          (file owned by W6; W0b's placeholder renders null; AppShell mounts it)
      QuickSwitcher (Ctrl/Cmd+K), ShortcutsDialog (?), MoveDialog, CollectionDialog, ConfirmDialog, SignInDialog, Toasts  (W3)
```

Interfaces that cross workstreams (all from W0b, typed):
- `TopBar` props: `{ item: ItemDetail; titleCrumb?: 'editable' | 'none'; actions?: ReactNode; leading?: ReactNode }`.
- `ItemHeader` props: `{ item: ItemDetail; onEnterBody(): void; readOnly: boolean }`.
- `ItemMenu` props: `{ item: ItemDetail; extraExports: {label, run}[] }`.
- The `saveStatus` API:
  - `setSaveStatus(itemId, s)` and `useSaveStatus(itemId)`;
  - `SaveState` is `'idle'|'saving'|'saved'|'offline'|'error'|'signed_out'`, plus `message?` and `retry?`.
- `registerPending(itemId, {hasPending, flush}) → unregister`.
- `BoardFrameHost` (default export, no props) and `useBoardFrame(): BoardFrameContext | null` from
  `web/src/board/BoardFrameHost.tsx`. `BoardFrameContext` is `{frame, failure, attach(surface)}` plus the additive
  `progress`, `updateAvailable`, `savedAt`, `open`, `release`, `show`, `reload` and `requestFullscreen`, kept in an
  external zustand store because the host is mounted beside the routes.

### 9.4 Document autosave (W4, `web/src/editor/docSession.ts`)

- **Load.** `DocSession.open(itemId)`:
  1. GET doc, then `Y.applyUpdate(ydoc, u, 'server')`.
  2. Read the IndexedDB outbox `quire.outbox/<itemId>`. If present, apply it locally and queue it.
- **Queue and send.**
  - Queue: `ydoc.on('update', (u, origin) => origin !== 'server' && queue(u))`. Except: while the document is blank
    (`isBlankDoc`: nothing but the fragment, empty or one empty paragraph with any attributes) and was blank when it
    loaded, the editor's own updates (its empty paragraph and block id) are held, not queued, so opening a document
    never writes to it (`content_at` stays NULL, the meta line keeps "You created", and leaving never prompts). The
    first update that leaves the document non-blank, local or from the server, queues the held ones ahead of itself.
  - On every queued update, write the merged pending update to the outbox (throttled to one write per 250 ms, and
    always right before a POST).
  - Debounce 1 s, at most 5 s apart. Then POST `Y.mergeUpdates(queue)`; on success, drop those updates and the outbox.
  - 409 `missing_deps`: POST `Y.encodeStateAsUpdate(ydoc, serverSV)`.
  - Network error: status `offline`, back off 1, 2, 5, 10, then 30 s, and retry at once on `online`.
  - 4xx or 5xx: status `error` with a retry action. 401: `signed_out`.
- **Other triggers.**
  - `pm_json` (`editor.getJSON()`) is attached at most every 30 s and when leaving the page.
  - Focus or `visibilitychange` → visible: `GET ?since=<local SV>` and apply with origin `'server'` (keeps two tabs
    converged).
  - Flush on `visibilitychange` → hidden, blur, route change and Ctrl/Cmd+S. On `pagehide`, send with
    `fetch(keepalive)` only when the payload is under 60 KB; otherwise rely on the outbox. `beforeunload` prompts when
    there is pending work.

### 9.5 Board host (W6, `web/src/board/*`)

- **The iframe.**
  - `BoardFrameHost` creates the iframe when a board page first wants the editor, that is when a `BoardSurface`
    attaches (not for the no-engine and phone fallbacks): `src = config.inkboard.base + 'frame.html'`,
    `allow="fullscreen; clipboard-read; clipboard-write"`. With `config.inkboard === null`, boards open in
    `BoardPreview` with the reason "Whiteboard engine not installed". The iframe is visible only once the current
    board is open, so `BoardLoading` shows underneath while the engine loads.
  - The iframe sits in a `position: fixed` layer with `z-index: 1`: above the in-flow canvas, below the menus (10), the
    Nested popover (30), dialogs, toasts and the sidebar drawer, all of which must stay above 1 to hang over the
    canvas. Its state is an external zustand store, read with `useBoardFrame()`.
  - After `load` it calls `iframe.contentWindow.quireBoardFrame.connect(events)` (§10.4). A page without
    `window.quireBoardFrame` counts as `{kind:'download', missing:true}`: the host then GETs `frame.html` (`no-store`)
    and, on a 404, runs the shell's update check `checkForUpdate()` (§9.1), so the one "Quire was updated" banner is
    the shell's; the board host has no banner of its own. A `connect()` rejected with `download` may be retried; other
    rejections are final (`BoardPreview`).
  - Off board routes the iframe gets `visibility:hidden`, `inert` and `setActive(false)`; back on a board route,
    `setActive(true)`. It is removed after 10 minutes away from boards.
  - Hovering a board row for 300 ms warms the cache with `fetch(url, {cache: 'force-cache', priority: 'low'})` for
    `inkboard_bg.wasm` and `inkboard.js` (`<link rel=prefetch>` falls under the SPA CSP's `default-src 'none'`): the
    sidebar's tree row times the 300 ms hover and reports it through `onBoardRowHover` (`state/hover.ts`), and
    `BoardFrameHost`, mounted once beside the routes, registers the warm-up for it with `useBoardRowWarmup()`
    (`board/warmup.ts`). There is one hover timer, the sidebar's.
- **Open.**
  1. Flush the board the frame showed before (await `host.flushPrevious`). A page's session stays active until its last
     flush is done, so reopening a board while its PUT still runs waits for that PUT and never shows an older
     revision.
  2. GET board, then `api.open({itemId, board, name: title, readOnly, theme})`.
  3. While the engine loads, show `BoardLoading`: the page-0 SVG at 50% opacity and a progress card fed by `onProgress`.
     The card appears only after 300 ms when the engine is cached.
- **Save loop.**
  - `onChange({itemId, revision})` schedules a save: 1.5 s quiet, at most 10 s apart.
  - Save: `snapshot()` → `{board, revision}` → PUT with `If-Match: "r{rev}"` and `X-Quire-Engine`.
    - 200: `markSaved(revision)` and store the new `rev`.
    - 412: `ConflictBanner` with "Load latest" (GET + open) and "Keep mine as a copy" (POST a sibling board titled
      `<title> (copy)`, PUT the snapshot to it at `r1`, and navigate to it; the original is not written).
    - 413: an error explaining the limit.
    - Network error: `offline` and back off 1, 2, 5, 10, then 30 s, as the document session does, with a retry at once
      on `online`. A frame that throws (a trapped engine) counts as gone.
  - Before each PUT, the snapshot is written to IndexedDB `quire.board.unsaved/<itemId>` (idb-keyval's default store)
    as `{baseRev, board, at}` and cleared on 200. On open, a newer unsaved snapshot (written after the item's
    `content_at`, else its `updated_at`) offers "Restore unsaved changes from HH:MM?" (a 24-hour clock); Restore opens
    it and PUTs it on its `baseRev`, and a stale base shows the `ConflictBanner`.
  - Flush on `onSave` (Ctrl/Cmd+S in the canvas), visibility hidden, route change and board switch. `beforeunload`
    prompts when the board is dirty. `fetch keepalive` (64 KB) is not used for boards.
  - A board larger than 15 MB shows a warning.
- **Read-only fallback (`BoardPreview`).**
  - Shown for: no WebGL2, a missing WebAssembly, BigInt or `crypto.getRandomValues`, an engine load or start error, an
    `onError{kind:'panic'}`, no engine in `/api/v1/config`, and by default on `(max-width: 767px) and (pointer: coarse)`
    (with an "Open editor" button).
  - It renders `boardPreviewUrl(id, rev, n)` (`/api/v1/items/{id}/board/pages/{n}?rev={rev}`) in an `<img>`, with page
    tabs, drag to pan, wheel or pinch to zoom, and Fit.
  - After a panic: "The whiteboard engine stopped. Changes up to HH:MM were saved", and a reload that recreates the
    iframe.
- **Keyboard.** The frame forwards Mod+K and Mod+. to the parent (`onHostKey`); Mod+Shift+P (publish) is not a Quire
  shortcut in M1 (D23). Pressing Esc twice within
  500 ms moves focus back to the parent's main region. F5 and Mod+R keep the browser's reload (§17).
- **Theme.** A Quire theme change calls `setTheme`. The board background stays board content.
- **Accessibility.** A visually hidden list next to the canvas holds the board's text, taken from
  `BoardMeta.text_excerpt` (the first 4 KB of the server-derived search text).

### 9.6 UI spec (normative)

Clean light design, generous whitespace, system UI font. Every value here is Quire's own; nothing is taken from
Outline's source or theme.

#### 9.6.1 Design tokens (`web/src/styles/tokens.css`)

| Token | Light | Dark |
|---|---|---|
| `--bg` | `#ffffff` | `#16181d` |
| `--bg-sidebar` | `#f6f7f9` | `#1c1f26` |
| `--bg-hover` | `#eceef2` | `#262a33` |
| `--bg-active` (current row, pressed) | `#e2e7f0` | `#2f3542` |
| `--text` | `#1d2430` | `#e6e8ec` |
| `--text-secondary` | `#5f6878` | `#a3aab6` |
| `--text-tertiary` (meta, placeholders) | `#8b93a1` | `#7a8291` |
| `--border` | `#e4e7ec` | `#2c313b` |
| `--accent` / `--accent-text` | `#3a6df0` / `#ffffff` | `#6c95ff` / `#0d1117` |
| `--danger` · `--warning` · `--success` | `#d93b3b` · `#b86e00` · `#1f8f50` | `#f06a6a` · `#e0a040` · `#4cc184` |
| `--mark` (search hit, highlight) | `#fff0a0` | `#5c4f12` |
| `--focus` | `0 0 0 2px color-mix(in srgb, var(--accent) 45%, transparent)` | same |
| `--shadow-pop` (menus, popovers) | `0 6px 20px rgb(0 0 0 / .10), 0 0 0 1px var(--border)` | `0 6px 20px rgb(0 0 0 / .45), 0 0 0 1px var(--border)` |

- Fonts: `--font-ui: system-ui, -apple-system, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif`;
  `--font-mono: ui-monospace, "SF Mono", Menlo, Consolas, monospace`.
- Type scale (px / line-height): UI 14/20, small 12/16, sidebar section label 11/16 uppercase +0.04em, editor body
  16/26, H1 28/36, H2 22/30, H3 18/26, document title 34/42 weight 700.
- Spacing scale: 4, 8, 12, 16, 24, 32, 48, 64. Radii: 4 (rows, pills), 6 (buttons, inputs), 8 (menus, cards),
  12 (dialogs). Motion: 120 ms ease-out; none under `prefers-reduced-motion`.
- Layout: sidebar 260 px default (216–400), row height 30, indent 16 px per level (capped at 10 levels), icons 16 px;
  top bar 52 px; document column max 720 px (full width: viewport minus 2×48 px); Contents column 220 px; below
  1024 px the sidebar becomes a drawer.
- Collection colours (12): `#8D8D8D #E5484D #F76B15 #FFC53D #46A758 #12A594 #00A2C7 #0090FF #3E63DD #6E56CF #AB4ABA
  #D6409F`. Collection icons: 48 lucide names in `web/src/components/collection/icons.ts` (W3 picks them).

#### 9.6.2 Sidebar

- **Workspace header:** logo (workspace logo attachment, else a letter tile in `--accent`), workspace name (14/600) and
  the user's name below it (12, `--text-secondary`), and a ▾ that opens the workspace menu.
- **Nav:** Home, Search, Starred. Drafts (with its count badge) and Templates are hidden in M1: items are published on
  create, and the few drafts an API or CLI unpublish makes appear in the tree, search and lists like any other item
  (D23). The draft UI returns with `config.multi_user`.
- **STARRED** section (only when something is starred), reorderable.
- **COLLECTIONS** section: one row per collection with its coloured icon (lucide icon tinted with the colour, or a
  letter tile). The collection of the current item is expanded, the current item is highlighted (`--bg-active`,
  `aria-current="page"`) and its ancestors are expanded. Rows show expand/collapse carets. On hover a collection row
  shows a sort menu (⇅: Manual, A–Z, Last updated, Created), "+" (Document D, Board B) and "⋯" (Edit, New document,
  New board, Sort, Delete); item rows show "+" and "⋯" (Rename, New child document, New child board, Duplicate, Move…,
  Star, Archive, Delete). "+ New collection…" ends the list. Every menu (`components/ui/Menu.tsx`, `placeMenu`) opens
  below its button when it fits, above it when it fits there (a row near the bottom of the window), and otherwise on the
  roomier side with its height capped to that side, so every item stays reachable with the mouse.
- **Sibling order.** The client sorts every sibling group itself (`buildTree`): Manual by `sort_key` (code-unit
  order), then `id`; A–Z by the title key; Last updated and Created newest first; ties by `sort_key`, then `id`. The
  server sorts the same way (§5), so an optimistic reorder agrees with the next tree fetch. The **title key** is
  defined twice, as `quire_core::tree::title_key` and the web's `titleKey` (`buildTree.ts`), and both are checked
  against `fixtures/tree/title-order.json`:
  1. NFKD, then every `General_Category=Mark` character removed, then lowercased, with `ς` as `σ` (case- and
     accent-insensitive: `Café` and `cafe` have one key);
  2. compared character by character in code point order, except that a run of ASCII digits is one number, compared
     by value (leading zeros ignored) and placed where the digit `0` is (`Chapter 2` before `Chapter 10`).
  It is not a locale collation (`Äpfel` sorts with `apfel`, after `@home` and `_underscore`), so server and browser
  never differ by ICU version.
- **Footer:** Archive, Trash, Settings. "+ Invite people…" is hidden in M1.

#### 9.6.3 Tree drag and drop and keyboard

Drag and drop (pragmatic-dnd):
- Item rows have zones before 25% / inside 50% / after 25% (Quire's own pure functions in `components/tree/`; the
  `-hitbox` package stays declared but unused). Hovering "inside" a collapsed row for 700 ms expands it. The "after"
  zone of an expanded parent means "before its first child". On the last child of an expanded parent, a pointer left
  of the row's indent outdents the drop one level per 16 px, while each node passed is the last child, so it lands
  after the parent or a further ancestor.
- In a sibling group whose sort is not `manual`, before/after collapse to "into the parent" (appended; the sort
  decides the position) and the indicator highlights the parent.
- Dropping on a collection row moves the item to the end of that collection's top level; moves across collections
  are allowed.
- Collection rows reorder among collections (before/after 50/50); a collection being dragged collapses, and over the
  items of an expanded collection it goes before or after that collection by the half of its block it is in.
- Starred rows reorder before/after only.
- The item itself and its descendants are invalid targets (no indicator). The sidebar auto-scrolls near its edges.
- A before/after drop sends both neighbours, the moved row left out; "inside" and collection drops send no position
  (last). A drop that changes nothing sends no request and shows no toast; that includes before or after the moved
  item itself (its first child on the "after" zone of its own expanded parent), where the indicator stays at its
  current place.
- After a drop: optimistic update, with a display-only key between the new neighbours (never sent; the response
  replaces the row), then a toast "Moved to <parent> · Undo" (8 s); Undo moves it back to its old parent and
  position. Move… shows the same toast; keyboard moves (Alt+Shift) show none, only errors. Trash, Archive and
  collection delete get an Undo toast too (restore, unarchive). A 409 rolls back with an error toast.

Tree keyboard (ARIA tree; focus is on rows):

| Key | Action |
|---|---|
| ↑ / ↓ | Previous / next visible row |
| → | Collapsed parent: expand. Expanded: first child. Leaf: nothing |
| ← | Expanded: collapse. Otherwise: focus the parent row |
| Home / End | First / last visible row |
| Enter | Open the item (collection row: open the collection page) |
| Mod+Enter | Open in a new tab |
| F2 | Rename inline (Enter saves, Esc cancels) |
| Alt+Shift+→ | Indent: becomes the last child of the previous sibling |
| Alt+Shift+← | Outdent: moves to right after its parent |
| Alt+Shift+↑ / ↓ | Move up / down among siblings (manual sort only) |
| Delete | Move to Trash (confirmation when it has children) |
| Printable characters | Type-ahead: focus the next row whose title starts with the typed prefix (500 ms buffer). Repeating one letter cycles; a prefix that matches nothing restarts with the last letter. Characters typed with Shift or AltGr count; other Mod or Alt chords and whitespace do not, and `?` always opens the shortcuts dialog |
| Shift+F10, ContextMenu | Open the row's ⋯ menu |
| Esc | Focus the main region |

Global shortcuts: Mod+K quick switcher; Mod+. toggle the sidebar; Mod+S flush saves; `?` (outside text fields)
shortcuts dialog. (Mod+Shift+P, publish or unpublish, comes back with the draft UI, D23.)

#### 9.6.4 Top bar and item header

- Top bar: breadcrumbs (collection icon + name / ancestors / current title; the board page's current crumb is an
  editable title), save status, "+ New doc" split button (Document, Board, Nested document, Nested board; created as a
  sibling, or as a child with Alt-click), "⋯" (Star, Full width (docs, a checkbox item), Duplicate, Move…, Archive,
  Export (Markdown, HTML; boards: .inkboard, SVG), Delete; Duplicate, Move…, Archive and Delete are disabled for a
  trashed item). Every "new item" action navigates with `{state: {created: true}}`, so the page discards exactly the
  items this page session created and left empty (`DELETE ?discard_empty=true`; 409 `not_empty` is ignored). Full width
  is `PATCH /items/{id} {full_width}`, applied to the cached `ItemDetail` at once and rolled back if the PATCH fails;
  the document page reads `ItemDetail.full_width` and sets `data-full-width` on `.doc-page`, which lifts the 720 px cap
  of the column and the item header (there is no per-session copy of the setting). There is no Publish button or step: a new item
  is published when it is created (D23). The avatars, Share and "Viewed by" slots are hidden in M1. Export → Markdown downloads `GET /api/v1/items/{id}/doc/markdown?download=true`
  (server-side, §6.4); HTML is produced on the client from the editor.
- Item header (docs): large title input (placeholder "Untitled"; Enter moves to the body; Backspace at the start of an
  empty body moves back to the title), a star toggle, and a meta line: "You created <rel>" while `content_at` is NULL,
  else "You updated <rel>". Relative times: "just now" (< 1 min), "N minutes ago", "N hours ago", "yesterday", then a
  date. The meta line shows no publication state (no " · Published", no Draft pill; D23).
- Contents column: shown with ≥ 2 headings; docked left of the document at ≥ 1280 px, otherwise a popover button in the
  top bar. Scroll spy: the active heading is the last one whose top is above 30% of the viewport height.

#### 9.6.5 Editor (W4)

- Slash menu (`/`): Text, Heading 1–3, Bulleted list, Numbered list, To-do list, Quote, Code block, Callout (Info, Tip,
  Warning, Danger), Divider, Table (3×3), Image, Mention a page, Embed board…, New board (creates a child board and
  embeds it). Typing filters; the menu closes after 3 characters that match nothing, or on Esc.
- Markdown shortcuts: `# ` to `### `, `- ` / `* `, `1. `, `[] ` / `[ ] `, `> `, ```` ``` ```` (+ language),
  `---`, `**x**`, `*x*` / `_x_`, `` `x` ``, `~~x~~`, `==x==`, `:::info ` / `:::tip ` / `:::warning ` / `:::danger `.
- Bubble menu on a text selection: bold, italic, underline, strike, code, highlight, link, "Turn into" (paragraph,
  headings, lists, quote).
- Mentions: `@` opens a title search (`/api/v1/search/titles`); a mention shows the item's live title from the query
  cache, a trashed target shows strikethrough with "In Trash", a missing target shows "Deleted item" and is not
  clickable. A pasted Quire URL becomes `link{itemId}`.
- Links: TipTap Link's `protocols` and `isAllowedUri` use the §6.4 scheme allow-list, so a pasted `javascript:` or
  `data:` URL stays text. An item link (`link{itemId}`, or an href `quire://item/<uuid>`) renders with the DOM
  `href="/item/<uuid>"` (§9.1) while the stored mark keeps its href, so "open in new tab" and "copy link address" give a
  working URL; HTML read back (a paste) maps `data-item-id` to `quire://item/<uuid>` again. Following a link
  (`LinkOpener`, `linkTarget`): a click when the document is read-only, Ctrl/Cmd+click while it is editable (a plain
  click places the caret, so link text stays editable). Item links and same-origin URLs go through the router, a
  fragment scrolls to its anchor, other allowed URLs open in a new tab (`noopener,noreferrer`); read-only, a click with
  Ctrl, Cmd or Shift is the browser's own new tab or window. Mentions and board cards keep their own links.
- Images: paste, drop or slash → upload to `/api/v1/attachments` with progress; a failure shows Retry; width handles
  (25/50/75/100%), alt text and caption. External images: §6.1.
- Code blocks: language select; lowlight loads lazily. Tables: header row, add/remove row and column from a cell menu,
  Tab moves between cells.
- Block handle on hover: drag to reorder; menu: Turn into, Duplicate, Copy link (headings: `#h-<blockId>`), Delete.
- Board embed card: the page-0 SVG preview `<img>` and the title, linking to the board; it never loads the engine.
  The preview is requested whenever the board has a page, also while `preview_rev` is null (a seeded or duplicated
  board has no stored preview until the page route renders it on demand, §12); an empty page (204) or an error shows
  the placeholder icon.
- Empty-body placeholder: "Type / for commands". Read-only (trashed, archived): a banner with Restore / Unarchive.
- Nested list below the body: the item's children (title, kind icon, updated time), "+ Add a page".

#### 9.6.6 Pages

- Home: Recently viewed and Recently updated (tabs). The document and board pages call `POST /items/{id}/view` once
  per opened item (`useRecordView`, fire and forget), the only source of `/recent`; the quick switcher's empty state
  shows the same list. Search: query box, filters (collection, kind, updated after,
  include archived; no drafts filter, the API default `include_drafts=true` applies, D23), sort, results with path and
  marked snippets. Starred, Archive: lists with path. Trash: roots with "+N nested", purge date, Restore and Delete forever (confirmation shows the count).
  Settings: Account (name, email, password, sessions; `?force=1` puts the password form first), Appearance (theme),
  Workspace (name, logo), API tokens, Data (Export as a plain `<a download>` of `/api/v1/export`, "Back up now", the
  backup note), About (version, commit, API version, engine version, and the licences: the generated `/licenses.txt`,
  shown on demand, §13.2, and links to the engine's `THIRD_PARTY_LICENSES.md` and `ICONS-LICENSES.txt` under
  `config.inkboard.base`). Collection: description, sort, the top-level items.
- Search keeps its parameters in the URL (`q`, `collection`, `kind`, `updated=day|week|month|year` as the start of that
  local day, `archived=1`, `sort=updated`; typing is debounced 250 ms); Home keeps `?tab=updated`. Login defaults "Keep
  me signed in" on, redirects to `/setup` while setup is required, and follows `next` only when it is a same-origin path
  (one leading `/`, no backslash or control character, not `//…`; `/login…` and `/setup…` give `/home`).
- Settings → API tokens: a list of tokens (name, prefix `qpat_ab12cd3…`, scopes, created, last used, expiry; expired
  ones greyed with "Expired") with Revoke (confirmation names the token). "New token" opens a dialog: name, scope (Read,
  Write, Admin, with one line each on what they allow, §7.3), expiry (7, 30, 90 (default) or 365 days, or Never, which
  shows a warning). Admin scope or Never adds a password field (§7.3). After creation the dialog shows the secret once
  in a monospace field with Copy and the line "You won't see this token again"; closing it drops the secret. A short
  note under the list shows how to use a token with the CLI without putting it in argv or shell history:
  `quire auth login --url <public URL> --token-stdin`, then paste the token and press Ctrl-D (§7.6).

#### 9.6.7 "Getting started" seed (W4 writes the text, original English)

Collection "Getting started" (colour `#3E63DD`, icon `rocket`), all items published (on create, like every item, D23):
- "Welcome to Quire": what collections and nesting are, the sidebar, Mod+K, search, stars, Trash and Archive, and
  where backups live (an archived page leaves the sidebar and the search results; Search's "Include archived" finds
  it). It does not mention drafts or publishing. It embeds the board below and mentions "Writing in Quire".
  Its fixture, `fixtures/ydoc/getting-started.bin`, names the placeholder ids `docs::SEED_WRITING_ID` and
  `docs::SEED_BOARD_ID`; `seed::getting_started` stores it with `docs::init_seed`, which maps them to the two seeded
  items' ids, so the mention and the embed open the real items.
  - "Writing in Quire" (`fixtures/ydoc/writing-in-quire.bin`, stored with `docs::init_with`): the slash menu, Markdown
    shortcuts, mentions, images, callouts, tables.
  - "Example board" (board, `fixtures/boards/getting-started.inkboard`): a few labelled shapes, a connector, a sticky
    note.

---

## 10. Inkboard embed design

### 10.1 Layers

```
Quire SPA (parent window)              web/src/board/BoardFrameHost.tsx       (W6)
  │ same-origin JS calls: iframe.contentWindow.quireBoardFrame                 contract: web/src/board/bridge.ts (W5)
  ▼
/inkboard/{build}/frame.html + frame.js  (Quire-owned, web/inkboard-frame/)    (W5)
  │ globalThis.inkboardEmbed.mount(canvas, opts) → Handle
  ▼
inkboard.js + inkboard_bg.wasm  (Inkboard + patches, feature `embed`)          (W5)
```

### 10.2 Patch series (`patches/inkboard/`, against the base copy of `41e0295`)

Every changed file gets one notice line near the top, in the file's comment syntax: `// Changed for Quire (embed
feature): see patches/inkboard/README.md` in `.rs`, `# …` in `.toml`, `<!-- … -->` in `.md`. This satisfies
Apache-2.0 §4(b). `scripts/inkboard.sh export --upstream` writes a notice-free copy of the series for upstream PRs. The
LICENSE files are untouched. The patches add no new `t()` strings: hiding UI is preferred over adding text, because
Inkboard's tests require a French entry for every string. Nothing under `inkboard-core` or `inkboard-session` changes.
The patch file names come from their subjects (`theme: sync egui theme`, `embed: js api`, `optional net`,
`docs: embedding`), the author is `quire <quire@localhost>`, and the series carries 16 notice lines in all (the
`--upstream` export none). `patches/inkboard/README.md` holds the patchset id (`2` since the review fixes changed 0003
and 0004), which makes `engine_version` `0.2.0-beta.3+embed.quire.2`.

| Patch | Files (`crates/inkboard-app/…` unless noted) | Change |
|---|---|---|
| `0001-theme-sync-egui-theme.patch` | `src/theme.rs` | First line of `theme::apply`: `ctx.set_theme(if dark { Theme::Dark } else { Theme::Light })`. Fixes the OS light/dark switch reverting the visuals, with a regression test (`theme::tests::a_system_theme_switch_keeps_the_applied_visuals`, which fails without the fix). Upstreamable on its own. |
| `0002-embed-js-api.patch` | `Cargo.toml` | `[features] embed = []`. Add web-sys features if `embed.rs` needs them. |
| | `src/main.rs` | `#[cfg(all(target_arch="wasm32", feature="embed"))] mod embed;`. The wasm `main` calls `embed::install_global()` when `embed` is on, and `web::start()` otherwise. |
| | `src/web.rs` | `webgl_available` (line 24) becomes `pub(crate)`. (`platform::random_seed` is already `pub`, `platform_web.rs:340`.) |
| | `src/platform_web.rs` | Split `init` into `init_ctx(ctx)` (sets CTX and installs the paste listener once behind a `Cell<bool>`; the listener returns unless `ctx.input(\|i\| i.focused)`, also on Inkboard's own page) and `init_page()` (hashchange/ticket, online/offline, visibility; it needs no context). `web::start` calls both; embed calls only `init_ctx`. Add `clear_ctx()`. |
| | `src/app.rs` | `pub trait HostEvents { fn ready(&self); fn changed(&self, rev: u64); fn save(&self); }`; `pub struct HostMode { read_only, dark: Option<bool>, paused: bool, events: Box<dyn HostEvents>, last_key: (u64, u64), ready_sent }`; field `pub host: Option<HostMode>`; `fn hosted()`. Dark override at the theme site (1143-1147). Skip `platform::set_title` when hosted (1177-1181). In `run_action` (868-888): `New`/`Open` do nothing, `Save`/`SaveAs` call `events.save()`, `Present` does nothing when hosted. `host_frame()` before `self.autosave()` (1183): when `(revision, hash(editing buffer))` changed and `!objects_moving() && drag.is_none()`, call `changed`, at most once per frame. Skip `request_repaint_after(500 ms)` (1192) while `paused`. The change key is `(revision, hash of the pending text)`, the hash being 0 while nothing is pending, so starting to edit a label is not a change. `host_snapshot() -> String` clones `pending_document()` or `doc`, runs `prune_assets` on the clone (never on the live doc), then `to_json(&clone, false)`. `host_load(&str) -> Result<(), LoadError>` parses with `from_json` and hands the `Document` to `host_load_doc(Document)`, which does the resets (`mount` parses the board once, before eframe starts, so it can reject `load`): set `editing`, `drag`, `draft`, `prop_edit`, `dialog` and `present_req` to `None` and `presenting = false` (fields at 128-160; upstream `open_text` relies on the menu's `commit_editing`, `chrome.rs:418`); then `ed.reset`, `saved_rev = revision`, clear `sel` and `textures`, `paste_count`, set `last_key` (a load is never reported as a change) and `fit_pending`. `host_snapshot` and `host_load*` are `#[cfg(all(target_arch = "wasm32", feature = "embed"))]`, and `HostMode` has `allow(dead_code)` outside that cfg, so native builds get no new warning. `host_guard()` runs every frame for a read-only host, as `session_guest` does for a read-only guest (tool Hand or Laser, no drag but panning, no editing); `setReadOnly(true)` commits the text being typed first. `#[cfg(target_arch = "wasm32")] fn as_any_mut` in `impl eframe::App` (the trait method exists only on wasm, `eframe epi.rs:166-169`). |
| | `src/session_guest.rs` (1595) | `view_only()` is also true when `host.read_only`. |
| | `src/chrome.rs` | `if !app.hosted()` around: the title-card name field and the Saved/Unsaved label (59-61); menu New, Open, Join/Leave, Save, Save as (427-453); Animate the agent (494-500); **Present full screen (507-509)**, which sets `present_req` directly; Language (519) and the separator after it; Quit (529-531). "Import Mermaid…" and the exports stay. |
| | `src/embed.rs` (new, about 350 lines) | §10.3. |
| `0003-optional-net.patch` (droppable) | `Cargo.toml` | `inkboard-net` becomes `optional = true`; `net = ["dep:inkboard-net"]`; `default = ["net"]`; `web-test = ["net"]`. |
| | `web/Trunk.toml` | `features = ["web-glow", "net"]`, so the public page keeps today's behaviour; the WebGPU command in its comment becomes `trunk build --release --features web-wgpu,net`. trunk 0.22.1 replaces Trunk.toml's list with a command-line `--features` (`src/cmd/build.rs`), and `no_default_features = true` stays. |
| | `scripts/build_web.sh`, `scripts/build_web.ps1` (repo root) | The WebGPU variant (`build_web.sh wgpu`, `build_web.ps1 -Renderer wgpu`) passes `--features web-wgpu,net`, for the same reason, so it keeps the guest factory and "Join a session…". |
| | `src/main.rs` | `session_link` and `session_net` behind `#[cfg(any(not(target_arch="wasm32"), feature="net"))]` (only `session_net*` use `session_link`); on wasm without `net`, `#[path = "session_net_off.rs"] mod session_net`. Native without `net` gets `compile_error!`. |
| | `src/session_net_off.rs` (new, about 40 lines) | Stubs used by `share_ui`, `join_ui` and `app.rs:1121`: `RelayListError`, `parse_relay_list`, `read_relays_text`, `save_relays_to`, `wait_background_closes`. |
| | `src/web.rs` | `guest_factory` (line 71) only with `net`. |
| | `src/platform_web.rs` (337) | With `net`: re-export `check_relay_url`/`is_known_relay`. Without: `pub struct NoRelayUrl;` with `fn host_str(&self) -> Option<&str> { None }`, `check_relay_url(&str, bool) -> Result<NoRelayUrl, NoNetwork>` that always returns `Err(NoNetwork)` (a type, not `()`: clippy pedantic's `ignored_unit_patterns` fires at the `join_ui.rs` caller otherwise), and `is_known_relay` that returns `false` (callers: `join_ui.rs:53-56` uses `.host_str()`, `session_guest.rs:1125` uses `.is_ok()`). |
| | `src/chrome.rs` | Hide "Join a session…" by a compile-time condition (wasm without `net`), not by `join.factory.is_none()`: the native screenshot and visual tests run without a factory and would lose the entry. |
| `0004-docs-embedding.patch` | `docs/WEB.md`, `docs/fr/WEB.md`, `CHANGELOG.md` (repo root) | An "Embedding (feature `embed`)" section, in English and French: the API, the single-mount rule, and that the host page must serve `icons.bin` next to itself; `[Unreleased]` changelog entries for embed, net and the theme fix. It says that an embedded board never joins a session and that the build leaves the network out, not that the `.wasm` gets much smaller (`size` measured 1.6 %). Kept separate so the code patches review on their own. |

`patches/inkboard/README.md` records the base commit, the patch list, how to regenerate (`scripts/inkboard.sh export`),
the `size` numbers, and the upstream PR status.

### 10.3 Inkboard JS API (feature `embed`; installed on `globalThis` by wasm `main`)

```ts
interface InkboardEmbed {
  readonly version: string;            // CARGO_PKG_VERSION + "+embed", e.g. "0.2.0-beta.3+embed"
  readonly formatVersion: number;      // inkboard_core::serial::VERSION (2)
  mount(canvas: HTMLCanvasElement, opts: MountOptions): Promise<InkboardHandle>;
  // rejects {kind:'webgl'|'load'|'start', message}; 'start' with "already mounted" while another handle is live
}
interface MountOptions {
  board?: string | null;               // native envelope JSON; null → new empty board
  name?: string;                       // doc.name (export file names)
  theme?: 'light' | 'dark' | 'board';  // chrome only; default 'board' (today's behaviour)
  lang?: 'en' | 'fr';                  // i18n::set after App::new; default: browser locale (today's behaviour)
  readOnly?: boolean;
  onReady?(): void;
  onChange?(e: { revision: number }): void;
  onSave?(): void;                     // Ctrl/Cmd+S or menu Save
  onError?(e: { kind: 'panic'; message: string }): void;
}
interface InkboardHandle {
  snapshot(): { board: string; revision: number };  // synchronous; includes text being typed; works in hidden tabs
  setBoard(json: string | null, name?: string): void; // null: a new empty board; resets undo and transient editor state; throws {kind:'load', message}
  revision(): number;
  isDirty(): boolean;
  markSaved(revision: number): void;
  setTheme(t: 'light' | 'dark' | 'board'): void;
  setReadOnly(b: boolean): void;
  setName(name: string): void;                      // assigns ed.doc.name directly: no undo entry, no revision bump
  setActive(b: boolean): void;                      // false: no periodic repaint (hidden host); true: repaint now
  focus(): void;
  fitView(): void;
  destroy(): void;                                  // runner.destroy(), clear_ctx(); handle becomes inert; mount allowed again
}
```

How `embed.rs` works:
- `install_global` builds a `js_sys::Object` of `Closure`s and sets it with `Reflect::set`, like `web_test.rs:63-80`
  does, so no `#[wasm_bindgen]` exports are needed (the workspace forbids unsafe code). `version` and `formatVersion`
  are plain properties of a frozen object.
- Options left open above: an unknown `theme` means `'board'`; a `lang` other than `en` or `fr` means the browser's.
  `setActive(false)` asks for no frame and `setActive(true)` asks for one. An inert handle (destroyed, or the engine
  stopped) throws `{kind:'start'}` from `snapshot` and `setBoard`, and returns 0 and `false` from `revision` and
  `isDirty`.
- `mount` returns a promise from `future_to_promise` and reads options with `Reflect::get`. A thread-local `MOUNTED`
  flag rejects a second live mount.
- Startup order: WebGL check, `seed_ids(platform::random_seed())`, `from_json` (reject `load` on failure), then
  `WebRunner::new().start(canvas, WebOptions{..default}, creator)`.
- The creator runs `init_ctx`, `App::new(cc, None, None)`, sets `host`, `i18n::set`, `host_load`, `ed.set_read_only`,
  and sets `doc.name`.
- The state lives in `Rc<RefCell<Option<(WebRunner, egui::Context)>>>`. Each call goes through
  `runner.app_mut::<App>()` followed by `ctx.request_repaint()`.
- Callbacks to JS always go through `wasm_bindgen_futures::spawn_local`, because eframe holds the runner borrow during
  `update` (`web_runner.rs:143-157`).
- A panic hook, chained after eframe's, reports through `window.setTimeout(onError.bind(null, {kind:'panic',
  message}), 0)`: a plain JS timer still fires after the wasm instance has trapped, which a `spawn_local` task does not.
- Closures are `forget`ed (a few bytes per mount).

### 10.4 Quire frame API (`web/src/board/bridge.ts`, implemented by `web/inkboard-frame/frame.js`)

```ts
export interface QuireBoardFrame { connect(events: FrameEvents): Promise<BoardFrameApi>; }  // window.quireBoardFrame
export interface FrameEvents {
  onProgress?(p: { loaded: number; total: number }): void;   // decoded wasm bytes / manifest raw size
  onReady?(itemId: string): void;
  onChange?(e: { itemId: string; revision: number }): void;
  onSave?(itemId: string): void;
  onError?(e: { itemId: string | null; kind: 'unsupported' | 'webgl' | 'download' | 'start' | 'load' | 'panic'; message: string }): void;
  onHostKey?(e: { key: string; mod: boolean; shift: boolean; alt: boolean }): void;
}
export interface OpenArgs { itemId: string; board: string | null; name: string; readOnly: boolean; theme: 'light' | 'dark' }
export interface BoardFrameApi {
  readonly engineVersion: string;     // inkboardEmbed.version + ".quire." + patchset, e.g. "0.2.0-beta.3+embed.quire.2"
  readonly formatVersion: number;
  open(a: OpenArgs): Promise<void>;   // first call mounts on the frame's canvas; later calls setBoard + setReadOnly + setName
  current(): string | null;           // itemId shown
  snapshot(): { itemId: string; board: string; revision: number };
  isDirty(): boolean;
  markSaved(revision: number): void;
  setTheme(t: 'light' | 'dark'): void;
  setReadOnly(b: boolean): void;
  setName(n: string): void;
  setActive(b: boolean): void;
  focus(): void;
  fitView(): void;
}
```

How the frame page works:
- `frame.html` contains only `<canvas id="board" tabindex="0">`, `frame.css` (`html, body {margin:0; height:100%;
  overflow:hidden}` and the canvas rules from Inkboard's `index.html`) and `<script type="module" src="frame.js">`. It
  has no inline script, no `#loading` and no `#error`.
- `frame.js`:
  1. Feature checks; a failure sends `onError{kind:'unsupported'|'webgl'}`.
  2. `connect` first reads `manifest.json` next to the frame (a same-origin `fetch`, covered by the frame CSP's
     `connect-src 'self'`; §11 serves it): the wasm's raw size is the `onProgress` total, the patchset gives
     `engineVersion`, and its `format_version` must equal the engine's (else `start`, final). A failed fetch rejects
     with `download`, after which the next `connect()` retries; repeated calls otherwise share one attempt (the events
     are replaced). Then `fetch('inkboard_bg.wasm')` wrapped in a counting `ReadableStream` (progress), and
     `init({ module_or_path: new Response(stream, {headers: {'Content-Type': 'application/wasm'}}) })`.
  3. `open()` awaits `document.visibilityState === 'visible'` (a `visibilitychange` promise) before `mount` or
     `setBoard` (D13). `open` calls run one after the other: the first resolves once the engine has drawn its first
     frame and then sends `onReady(itemId)`; later ones call `setBoard`, `setReadOnly`, `setName` and `setTheme`, then
     send `onReady`. A failed `setBoard` rejects with `load`, reports the new itemId in `onError`, and keeps the previous
     board (and its itemId) in the engine. `snapshot()` and `isDirty()` never wait; `snapshot()` throws while no board
     is open. `lang` is never passed (Inkboard's menus follow the browser, English or French).
  4. A window keydown listener in the capture phase forwards host keys (Mod is Cmd on macOS and Ctrl elsewhere, as in
     `app/hotkeys.ts`; Mod+K and Mod+. are stopped before the engine sees them) and handles double-Esc: the first Esc
     still reaches the engine, a second within 500 ms calls `parent.focus()` and sends `onHostKey{key:'Escape'}`. It
     does not block F5 or Mod+R.
  5. `window` `error` and `unhandledrejection` listeners whose error is a `WebAssembly.RuntimeError` send
     `onError{kind:'panic'}` once (a backup for the panic hook).
  6. On `pagehide`, `WEBGL_lose_context`, unless the page goes into the back/forward cache.
  7. Events carry the current `itemId`; the host ignores events whose id is not current.
- `frame.js` starts with `// @ts-check`, imports its types from `../src/board/bridge.ts` through JSDoc, and imports
  `./inkboard.js`, whose types are declared in `web/inkboard-frame/inkboard.d.ts` (`default init`, `InkboardEmbed` on
  `globalThis`); `pnpm --dir web typecheck` checks it.

### 10.5 Build (`scripts/inkboard.sh`, always run through `scripts/dev.sh` or inside the Docker stage)

The script never runs git against the main repository, so it works the same in the main tree, in a git worktree and in
the Docker context.

| Subcommand | Effect |
|---|---|
| `src [--force]` | Rebuild `.cache/inkboard-src`: check that `vendor/inkboard/Cargo.toml` has `version = "0.2.0-beta.3"`; copy the submodule's files with `tar --exclude=./.git --exclude=./target --exclude=./crates/inkboard-app/web/dist`; `git init -q`; commit everything as `base` (fixed dates, tag `base`; `git -c user.name=quire -c user.email=quire@localhost`); `git am --no-3way patches/inkboard/*.patch`. The self-contained repo lives inside `/work`. It refuses to discard uncommitted changes or unexported commits there unless `--force`. `.git/quire-src-stamp` records the inputs (the submodule's file list with sizes and mtimes, and the patch contents) and the `HEAD` after `git am`; `build`, `check` and `size` rerun `src` only when the inputs changed, and `export` refreshes the stamp. (`scripts/check-pins.sh`, on the host, checks that the submodule is at `41e0295` and clean.) |
| `build [--out web/dist-inkboard]` | Runs `src` if stale, then from `.cache/inkboard-src` (so its `.cargo/config.toml` getrandom rustflag applies; never set `RUSTFLAGS`): `CARGO_TARGET_DIR=${INKBOARD_TARGET_DIR:-$root/.cache/inkboard-target} cargo build --locked --release -p inkboard-app --bin inkboard --target wasm32-unknown-unknown --no-default-features --features web-glow,embed`, then `wasm-bindgen --target web --no-typescript --out-name inkboard --out-dir $OUT …/inkboard.wasm`, then `wasm-opt -Os` with the flags trunk 0.22.1 passes: read from trunk's source (`src/pipelines/rust/mod.rs`, `wasm_opt_build`), which for Inkboard's `index.html` (`data-wasm-opt="s"`, no `data-reference-types`, no `data-wasm-opt-params`) is `wasm-opt --output=OUT -Os IN` and nothing else, recorded in the script as `WASM_OPT_FLAGS` (`BINARYEN_CORES` comes from the environment; binaryen 131 validates the output without feature flags). It copies `web/inkboard-frame/*.{html,js,css}` only, `assets/icons.bin`, `assets/ICONS-LICENSES.txt`, Inkboard `LICENSE-*` and `THIRD_PARTY_LICENSES.md`; precompresses `.wasm .js .html .css` with `brotli -q 11 -k` and `gzip -9 -k -n` (never `icons.bin`); writes `manifest.json {build, engine_version, format_version, patchset, files:{name:{raw,br,gz}}}`: `build` is the first 12 hex digits of a sha256 over the sorted `sha256sum` lines of every file but the manifest, and `files` lists every file but the manifest and the `.br`/`.gz` copies, with null for a missing copy. The dist is flat (no wasm-bindgen JS snippets, and no `web/inkboard-frame/inkboard.js`, which would shadow the engine; `build` refuses both), and it is staged in `web/.dist-inkboard.XXXXXX` and renamed into place; a non-empty directory without `manifest.json` is never replaced. `--quick` (frame-test iteration only; the Dockerfile never passes it) builds release code with `lto = false`, 16 codegen units and incremental compilation (through `--config`, files untouched), skips wasm-opt and compresses with brotli 5 and gzip 6. |
| `stub [--out …]` | A placeholder dist: a `frame.html` saying "engine not built", a `frame.js` (loaded as an external module) whose `connect` sends `onError{kind:'unsupported'}` and rejects, so a board host falls back to the preview, their `.br`/`.gz` copies, and a manifest with `engine_version` and `patchset` `"stub"` and `format_version` parsed from `vendor/inkboard/crates/inkboard-core/src/serial.rs` (`pub const VERSION: u32 = N;`). Used by images built with `INKBOARD_BUILD=stub`; the frame test (§14) needs the real engine and fails on purpose, once, against it. |
| `export [--upstream]` | `git -C .cache/inkboard-src format-patch --zero-commit --no-signature base -o patches/inkboard`, after removing the old files; it refuses a commit message that contains the notice text. `--upstream` writes the series without notice lines to `.cache/inkboard-upstream/` and checks its own output for notices. |
| `check [--record-baseline]` | Feature sets follow the patched manifest's `[features]` (`embed`, `net`), not the patch file names. Always: wasm `--features web-glow`, native default with `--all-targets` (so the tests compile too). With `embed`: wasm `web-glow,embed`. With `net` optional: wasm `--no-default-features --features web-glow,embed` and wasm `web-glow,net` (the public page). `cargo check` and `cargo clippy` for each set; no warning may appear more often in one cargo run than in the baseline `patches/inkboard/check-baseline.txt`. A warning is keyed as `<kind> <file>: <message>`, without line and column, so a patch that moves code adds nothing, and each key carries the highest count of one run (the sets compile the same files again). `--record-baseline` writes the file and is allowed only while `HEAD` is `base`; the recorded base has 11 distinct warnings, all clippy pedantic on the wasm target. A baseline without counts is refused. |
| `size` | Prints the raw, br and gz sizes of the `.wasm` of `web-glow,embed` without and with `net` (with 0003 applied, `net` compiles exactly what the unpatched dependency did). |

At startup the server reads the embedded `manifest.json`. If it is missing (a debug build with no dist), it logs a
warning and serves `config.inkboard = null` (boards open in preview mode); the stub dist gives `null` too, but its files
are still served under its build. If `format_version` differs from `inkboard_core::serial::VERSION`, or the manifest
does not parse, or its `build` is empty or not alphanumeric, it **refuses to start**.

---

## 11. Static serving and headers (`quire-server::static_files`, `headers`)

| Path | Cache-Control | Encoding | Headers |
|---|---|---|---|
| `/assets/*` (Vite-hashed) | `public, max-age=31536000, immutable` | br / gzip / identity | |
| `/index.html` and every SPA route (any path without an extension outside `/api`, `/assets`, `/inkboard`) | `no-cache` | br / gzip | SPA CSP, `X-Frame-Options: DENY` |
| Other root files of `web/dist`: `/theme-init.js`, `/favicon.svg`, and `/licenses.txt` (written by the image build, §13.2) | `no-cache` | identity | the type from the extension table (`text/plain; charset=utf-8` for `.txt`) |
| `/inkboard/{build}/…`: `frame.html`, `*.js`, `*.css`, `*.wasm`, and the licence files (`LICENSE-*`, `THIRD_PARTY_LICENSES.md`, `ICONS-LICENSES.txt`) | `public, max-age=31536000, immutable` | br / gzip / identity where the dist has the variant; `.wasm` is `application/wasm` | Frame CSP, `X-Frame-Options: SAMEORIGIN` |
| `/inkboard/{build}/manifest.json` (the embedded build only) | `no-cache` + ETag (304 on `If-None-Match`). Not immutable: `{build}` hashes the dist's other files but not the manifest's own fields (`patchset`), so one build id can come with another manifest | identity only (the dist has no variants of it, and the compression layer never re-encodes a static file) | `application/json`, `nosniff`; no CSP or `X-Frame-Options` (data that `frame.js` fetches, §10.4, never a page) |
| `/inkboard/{build}/icons.bin` | immutable | identity only (already deflated per set) | `application/octet-stream` |
| `/inkboard/{other}/…` | 404 | | |
| `/api/v1/attachments/{id}` | `private, max-age=31536000, immutable` | identity | `nosniff`, `CSP: default-src 'none'; sandbox`. Inline only for png, jpeg, gif, webp and avif; SVG and everything else get `Content-Disposition: attachment` (filename per §8) |
| `/api/v1/items/{id}/board/pages/{n}` | `?rev=` current: `private, max-age=31536000, immutable`; no `rev`: `no-cache` + ETag; a redirect to the current rev: `no-store` | gzip (fast level) when over 1 KiB and the client accepts it, by the compression layer, with `Vary: Accept-Encoding`; the route itself sends identity | `image/svg+xml`, `nosniff`, `CSP: default-src 'none'; img-src data:; style-src 'unsafe-inline'; sandbox` |
| `/api/v1/items/{id}/doc/markdown` | `no-store` | gzip (fast level) when over 1 KiB | `text/markdown; charset=utf-8`, `nosniff`, `CSP: default-src 'none'; sandbox`; `Content-Disposition: attachment` (filename per §8) with `?download=true` |
| `/api/v1/openapi.json` | `no-cache` + ETag | gzip (fast level) | `application/json` |
| `/api/*` JSON | `no-store` (every `/api/` response without its own policy) | gzip (fast level) when over 1 KiB | `X-Request-Id`, `Quire-Api-Version` (every `/api/v1` response); CORS headers only for an origin in `QUIRE_CORS_ORIGINS` (§8) |

Dynamic responses are gzipped at the fastest level by tower-http when the body is over 1 KiB and carries no
`Content-Encoding` yet: every `application/json` and `text/markdown` body (the `/api` JSON, `openapi.json`, the Markdown
GET), and `image/svg+xml` on the board-page route only (the route marks its response). A streamed body (a large
board, a preview rendered on demand, the export) is a fused stream, since the gzip encoder polls it once more after its
end. Static files (the fallback marks
each response, so a JSON one such as the Inkboard `manifest.json` keeps its bytes too) and attachments, SVG attachments
included, are never re-encoded, and a response that is already encoded is skipped.

How a static file is served:
- Pick `name.br` when `Accept-Encoding` offers br with a non-zero q-value, then `name.gz`, then identity. Requests for
  `*.br` or `*.gz` themselves are 404, and a method other than GET or HEAD is 405 (after the CSRF check).
- Send `Vary: Accept-Encoding`. Caddy forwards the client's `Accept-Encoding` and passes the chosen variant's bytes and
  `Content-Encoding` through unchanged: the Caddyfile has no `encode`, and `compression off` stops Caddy from adding
  `Accept-Encoding: gzip` toward Quire and decoding the answer (§13.4). The image smoke checks it byte for byte (§14).
- The ETag is the first 16 bytes of the chosen variant's sha256 (rust-embed's), in hex; `If-None-Match` gets 304.
  Content types come from one extension table, so rust-embed's guess and the in-memory test source agree.
- An unknown `/api/*` path gets a JSON 404. An unknown path with an extension gets 404, and so does any path with a
  `.` or `..` segment or a backslash.

**SPA CSP:**
```
default-src 'none'; script-src 'self'; style-src 'self'; style-src-attr 'unsafe-inline'; img-src 'self' data: blob:;
font-src 'self'; connect-src 'self'; frame-src 'self'; media-src 'self' blob:; manifest-src 'self'; worker-src 'self';
base-uri 'none'; form-action 'self'; frame-ancestors 'none'; object-src 'none'
```
- `style-src-attr` is needed because ProseMirror writes `style` attributes.
- Create the editor with `injectCSS: false` and import TipTap's CSS through Vite: `web/src/editor/editor.css` carries
  ProseMirror's base rules (`white-space: pre-wrap`, the separator images, the hidden selection, the gap cursor).
  TipTap's own inline `<style>` is refused by this CSP; the Playwright smoke fails on any CSP violation.
- If something still breaks, fall back to `style-src 'self' 'unsafe-inline'` and record that in §16.
- `img-src` stays same-origin: remote images pasted from the web become placeholders (§6.1, §17), so reading a page
  never contacts third parties.

**Frame CSP:**
```
default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:;
font-src 'self' data:; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'self'; object-src 'none'
```

**On every response:** `X-Content-Type-Options: nosniff`, and `X-Request-Id`. **On all HTML:** `Referrer-Policy:
same-origin`, `Cross-Origin-Opener-Policy: same-origin`, `Cross-Origin-Resource-Policy: same-origin`,
`Permissions-Policy: camera=(), microphone=(), geolocation=(), payment=()`. COEP is not needed.

**HSTS** (D57): `Strict-Transport-Security: max-age=31536000` (no `includeSubDomains`, no `preload`) on every response
to an HTTPS request (§8), errors and static files included, when `QUIRE_TLS=acme`; never with `QUIRE_TLS=internal`
and never on plain HTTP. `QUIRE_TLS` defaults to `internal`, and any value other than `internal` or `acme` stops startup (Caddy's `import tls_…` would fail
too). Caddy adds no security header of its own (no `header` directive; at most its `Server` header), so every security
header comes from Quire and is covered by its tests.
The CSPs above are the same behind Caddy.

Browsers offer brotli only over HTTPS, and Inkboard's copy-as-image needs a secure context: the compose deployment
serves Quire over HTTPS through Caddy (D57, §13.4), never on a published plain-HTTP port.

---

## 12. Background jobs (`quire-server::jobs`)

One tokio task per job, each using the writer through `Db`:
- **Previews:** an unbounded mpsc queue of item ids, deduplicated. An item renders 5 s after its last enqueue and at
  most 30 s after its first (the cap keeps steady saving from starving it); a board that is already current (page 0
  has a row at its rev) or gone is skipped. The worker keeps only the database, the blob store and the permit. A
  render holds the owned `board_work` permit until its last blocking step, renders page by page within the preview
  budgets (§5 "Previews"), deflates each page in `spawn_blocking`, and stores one revision's pages in one writer
  transaction. A GET for a stale or missing preview renders on demand through the same permit; a render that waited
  for the permit first looks for a render of the same revision made meanwhile and serves that.
- **Purge and GC:** at startup and every 6 h. Purge Trash roots (items and collections) older than 30 days, leaves
  first; then attachment GC, then blob GC with the orphan-file sweep (§5, D26). The GC also removes staging files in
  `/data/tmp` older than 24 h.
- **Revision prune:** daily.
- **Backup:** every `QUIRE_BACKUP_INTERVAL` (default 24h), keeping `QUIRE_BACKUP_KEEP` (default 7) copies of
  `/data/backups/quire-*.db`, on-demand backups (`POST /api/v1/admin/backup`, `quire admin backup` without `--out`)
  included; pre-migration backups are kept separately. `QUIRE_BACKUP_KEEP=0` turns the schedule off, and an
  on-demand backup then keeps only the new file. The first scheduled backup after a start is due one interval after the
  newest `quire-*.db` (by modification time) and at least 60 s after startup, or 60 s after startup when there is none
  or it is overdue, so a server that restarts more often than the interval still gets its backups. Scheduled and
  on-demand backups never overlap (single-flight), and each starts with the free-space check (D33).
  1. `VACUUM INTO '/data/backups/.tmp-…'` on the maintenance connection (D2).
  2. `quick_check` on the copy.
  3. Rename it to `quire-YYYYMMDDTHHMMZ.db` (`-2`, `-3`, … when the minute's name is taken), then prune to the count;
     a file that is already gone counts as deleted.

  `backup::create` holds an exclusive `flock` on `/data/backups/.lock` from the copy to the prune, so
  `quire-server admin backup` and a running server's backup never overlap across processes either: the second gets
  429 `busy` (exit 11 for the command). The single-flight flag lives in the blocking task that copies, so a request
  that was dropped (the client left) keeps the flag until its copy is done.
- **Pre-migration backup:** if migrations are pending at startup, `migrate` calls `backup::pre_migration`, which runs
  `VACUUM INTO '/data/backups/pre-migrate-v{N}-{ts}.db'` on the maintenance connection; then it migrates in one
  transaction and runs `foreign_key_check`.
- **Export** (on request, single-flight, after the free-space check): `VACUUM INTO '/data/tmp/export-….db'` on the
  maintenance connection (never `/tmp`, which is a 32 MB tmpfs). Then open the copy on its own connection, not through
  the §5 PRAGMA path, with `journal_mode=DELETE` and `secure_delete=ON`; run `DELETE FROM sessions; DELETE FROM
  api_token_scopes; DELETE FROM api_tokens; UPDATE users SET password_hash = NULL;`, and delete the `sqlite_stat1` and
  `sqlite_stat4` rows of `sessions`, `api_tokens`, `api_token_scopes` and `users` (`PRAGMA optimize` samples their index
  keys, token and session hashes among them, into those tables); close it, so no `-wal` or `-journal` file remains next
  to it. Then `VACUUM INTO` a second `/data/tmp/export-….db` from that scrubbed copy, delete the first copy and check
  that the new file is in `DELETE` journal mode: `secure_delete` zeroes the deleted cells, but the first `VACUUM INTO`
  builds its b-trees without clearing unused page space, where older copies of deleted rows (stat4 samples of token
  hashes) survived; the second copy holds only the remaining rows. For a moment `/data/tmp` holds two copies of the
  database, so an export needs about twice its size free on `/data` (the D33 floor does not scale with it). Stream the tar (`manifest.json` with `format`, `format_version` 1, `created_at`, `server_version`,
  `api_version`, `schema_version` and `without`; `quire.db`; `files/aa/bb/<sha>`), reading the copy and each file in
  256 KiB chunks through a 4-chunk channel (at most 1 MiB in flight, D25); delete the copy. The free-space check and
  the copy run on the blocking pool, which owns the single-flight guard; the guard then moves into the response body,
  so it is released when the stream ends or the client goes away, and a request dropped during the copy keeps the
  flag until the copy is done and deleted.
- **Optimize:** `PRAGMA optimize` every hour. Expired sessions are deleted at the same time, and so are API tokens 30
  days after they were revoked or expired (§5); the keyed rate limiters and the token last-use map drop idle keys
  (`retain_recent`).

---

## 13. Docker

### 13.1 Build-side resource caps (owner requirement)

- **`scripts/lib/lock.sh`** defines `quire_lock`: `flock` on `<home>/.cache/quire/build.lock`, where `<home>` comes from
  `getent passwd "$(id -u)"`: the same file whatever `$HOME`, `XDG_RUNTIME_DIR` or a private `/tmp` say, and shared by
  every worktree. It waits and prints "waiting for another Quire build…". After acquiring it exports
  `QUIRE_LOCK_HELD=1`. A script started with it set never waits: it tries a non-blocking `flock`, which fails at once
  when an ancestor holds the lock and takes it when nobody does, so nesting cannot deadlock and a stale or hand-set
  value cannot run a build unlocked (never set it by hand). Every failure returns 1 explicitly (`set -e` does not act
  inside a function called from `if` or `||`), and callers write `quire_lock || exit 1`. The lock is a dynamically
  numbered fd inherited by the `exec docker run`, so it is held until the container exits.
- **Who takes the lock.** `dev.sh`, `image.sh`, `image-smoke.sh`, `e2e.sh`, `agent-test.sh`, `cli-dist.sh` and
  `toolchain.sh` source it.
- **`scripts/dev.sh`** runs `docker run --rm --init` with:
  - `--cpus $QUIRE_BUILD_CPUS` (default 6), `--cpuset-cpus $QUIRE_BUILD_CPUSET` (default: the host's last
    `QUIRE_BUILD_CPUS` cores, e.g. `10-15` on 16 cores; omitted when the host has fewer), `--memory` and
    `--memory-swap` `$QUIRE_BUILD_MEM` (default 12g), `--pids-limit 4096`, `--shm-size 1g` (Chromium);
  - `--cgroup-parent "$QUIRE_CGROUP_PARENT"` when `quire_slice` (below) set it; otherwise the container runs with its
    own caps only, after the slice warning;
  - env `CARGO_BUILD_JOBS`, `BINARYEN_CORES`, `RAYON_NUM_THREADS`, `GOMAXPROCS` and `UV_THREADPOOL_SIZE` set to the CPU
    count, plus `CARGO_HOME`, `HOME` as today;
  - `nice -n 10` around the command;
  - optional variables `QUIRE_DEV_IMAGE` (default `quire-toolchain:latest`), `QUIRE_DEV_NETWORK`, `QUIRE_DEV_PORTS`,
    and `QUIRE_SHARED_CARGO_HOME` (an absolute path, mounted over `/work/.cache/cargo`, so worktrees share one registry
    and `CARGO_HOME` stays the same inside). The CPU and memory variables may only be lowered, and the scripts enforce
    it: `quire_caps` (in `scripts/lib/slice.sh`, shared by `dev.sh` and `toolchain.sh`, so the BuildKit container gets
    the same set by construction) refuses with exit 2 a `QUIRE_BUILD_CPUS` that is not a whole number from 1 to 6, a
    `QUIRE_BUILD_MEM` that is not an integer size of at most 12g, and a `QUIRE_BUILD_CPUSET` with more than
    `QUIRE_BUILD_CPUS` CPUs. The default cpuset is the last N ids of `/sys/devices/system/cpu/online` (not `nproc`,
    which follows the caller's own affinity), and the thread variables use the smaller of the CPU count and the
    cpuset's size. The value checked is the value passed to `--memory`. Without a command, `dev.sh` prints usage and
    exits 2.
- **The slices** (owner decision 2026-10-28: installed). `docker/quire-build.slice`:
  `[Slice] CPUQuota=600% MemoryMax=12G MemorySwapMax=0 CPUWeight=20 IOWeight=20 TasksMax=16384`. A dash in a slice name
  means nesting (systemd.slice(5)), so the unit sits at `/sys/fs/cgroup/quire.slice/quire-build.slice/` under a parent
  `quire.slice`. The quota and the memory limits are absolute and hold at any depth; weights only rank siblings, and
  `quire-build.slice` has none, so `docker/quire.slice` (`[Slice] CPUWeight=20 IOWeight=20`) carries the low priority
  against `user.slice` and `system.slice`. `IOWeight` acts only where the disk's scheduler honours `io.weight` (BFQ or
  io.cost); `--check` prints the scheduler.
- **Joining the slice.** Docker on this host uses the systemd cgroup driver on cgroup v2 (`docker info`: `systemd`,
  `2`; checked 2026-10-28). Under that driver `--cgroup-parent` (compose: `cgroup_parent`) takes a slice name, and the
  container's scope is created as `quire.slice/quire-build.slice/docker-<id>.scope`, inside the slice's totals. The
  name must belong to an installed unit: systemd creates a missing slice on the fly, with no limits. Under the cgroupfs
  driver the same flag means a raw cgroup path outside systemd, so it is not used.
- **`scripts/lib/slice.sh`** defines `quire_slice`. It exports `QUIRE_CGROUP_PARENT=quire-build.slice` when
  `docker info --format '{{.CgroupDriver}}'` is `systemd` and `systemctl show -p FragmentPath --value
  quire-build.slice` is `/etc/systemd/system/quire-build.slice`; otherwise it exports it empty and prints once to stderr
  "quire-build.slice not installed: per-container caps only (scripts/install-build-slice.sh --check)" (once per process
  tree; with the unit installed but a cgroupfs Docker it says that instead, since "not installed" would be false).
  `LoadState` is
  no test: systemd reports `LoadState=loaded` for any slice name, installed or not (a slice's unit file is optional;
  checked with `systemctl show quire-missing-test.slice` on systemd 259, 2026-10-28), and would put containers into an
  on-the-fly slice with no limits. `dev.sh`, `toolchain.sh`, `e2e.sh`,
  `agent-test.sh` and `image-smoke.sh` call it; the compose overrides read the variable (§13.3), so every build
  container, the BuildKit daemon and the e2e, agent and smoke containers join the slice when it exists and still run,
  capped per container, when it does not. `QUIRE_BUILD_SLICE` (default `quire-build.slice`) exists only so the
  acceptance test can name a missing slice and see the warning: it changes which unit is tested, never the parent,
  which is `quire-build.slice` or empty.
- **Installing (one privileged step).** `scripts/install-build-slice.sh` must run as root and never calls `sudo`
  itself; started without root it exits 1 and prints the command below. It installs `docker/quire.slice` and
  `docker/quire-build.slice` into `/etc/systemd/system/`, runs `systemctl daemon-reload` and
  `systemctl start quire-build.slice`, then its own `--check`. Re-running it is harmless. Because it runs as root it
  trusts nothing from the caller: `#!/bin/bash`, a fixed `PATH`, `HOME=/root`, no `DOCKER_*`, `CDPATH`, `BASH_ENV`,
  `ENV` or `TMPDIR`. Each unit file is read once (a regular file, not a symlink, at most 16 KiB) and accepted only if
  its non-comment lines are exactly the approved directives (the values below and a plain-text `Description=`; no line
  continuations or `%` specifiers); both are checked before either is written, a refused file's content is never
  printed, and the checked bytes are staged in a root-only `mktemp -d /run/quire-install-slice.XXXXXXXX` and installed
  `-o root -g root -m 0644`. Implementing agents never run
  it, or any `sudo`: they prepare the files and hand over, and the coordinator runs it once, interactively, through the
  owner's askpass dialog (one dialog for the whole step):
  ```sh
  SUDO_ASKPASS=/home/staffan/.local/bin/claude-askpass sudo -A \
    -p "install the Quire build cap (systemd slice: 6 CPUs, 12 GB RAM, no swap, low priority) :: $PWD/scripts/install-build-slice.sh" \
    "$PWD/scripts/install-build-slice.sh"
  ```
  Then `scripts/install-build-slice.sh --check`, and the BuildKit container is recreated so it joins the slice
  (below).
- **Checking.** `scripts/install-build-slice.sh --check` (no root) prints and verifies: the cgroup driver is `systemd`;
  for `quire-build.slice`, `FragmentPath=/etc/systemd/system/quire-build.slice`, `ControlGroup=/quire.slice/quire-build.slice` (while it is active: after
  a reboot it starts with its first member), `CPUQuotaPerSecUSec=6s`, `MemoryMax=12884901888`, `MemorySwapMax=0`, `CPUWeight=20`, `IOWeight=20`; for
  `quire.slice`, `FragmentPath=/etc/systemd/system/quire.slice`, `CPUWeight=20` and `IOWeight=20`; then lists the current members (`systemctl status quire-build.slice
  --no-pager`) and the scheduler of the disk holding `/var/lib/docker`. Exit 0 when every value holds, 1 otherwise
  (each failing line named), 3 when the slice is not installed (no `FragmentPath`).
- **The `quire-limited` builder** (buildx `remote` driver on a BuildKit container Quire manages). buildx's
  `docker-container` driver sets the builder container's cgroup parent only when Docker uses the cgroupfs driver (its
  `driver/docker-container/driver.go` checks `CgroupDriver == "cgroupfs"`, and the driver docs say the same), so on this
  host a builder created with `--driver-opt cgroup-parent=…` would never join the slice. `scripts/toolchain.sh`
  therefore:
  1. creates the container `quire-buildkit` from
     `moby/buildkit:v0.34.0@sha256:b059f8d7226d0b326bc871af489a5dddf65e36af4d20d14251b072974d9ee87c` (v0.34.1 was under
     two weeks old when it was pinned, §16 risk 23) with `docker create --init --privileged --cgroupns private --restart
     unless-stopped --cpus 6 --cpuset-cpus <same set as dev.sh> --memory 12g --memory-swap 12g --pids-limit 4096`,
     `--cgroup-parent "$QUIRE_CGROUP_PARENT"` when set, and the volume `quire-buildkit-state:/var/lib/buildkit`.
     `--cgroupns private` keeps the build steps' cgroups inside the container's caps: with Docker's
     `default-cgroupns-mode: host` the privileged daemon would create them at the host's root. `buildkitd` gets
     `--config /etc/buildkit/buildkitd.toml` explicitly, so a missing config fails loudly; `docker/buildkitd.toml`
     (`max-parallelism = 2`, GC `maxUsedSpace = "30GB"`) goes there with `docker cp` of a tar stream owned by root, mode
     0644 (no bind mount from a worktree that may disappear); then it starts it;
  2. registers it once: `docker buildx create --name quire-limited --driver remote docker-container://quire-buildkit`.
     `--load` and `--output type=local` work with the remote driver.

  If `quire-limited` exists with another driver (the earlier docker-container builder) or another endpoint, or
  `quire-buildkit` exists with another image, other caps, CPU set, pids limit, privilege or cgroup namespace mode,
  another cgroup parent than `quire_slice` now gives (the slice was installed after it was created), or another config
  file content, the script prints `docker buildx rm quire-limited; docker rm -f quire-buildkit; scripts/toolchain.sh`
  and exits 1. It never removes either by itself. A stopped `quire-buildkit` with matching settings is simply started,
  and every run ends with `docker buildx inspect --bootstrap`. The state volume survives a recreate, and with it the
  build cache. `scripts/toolchain.sh --builder-only` sets up or checks the builder without building the toolchain image;
  `image.sh` and `cli-dist.sh` run it first, so a drifted builder stops their build too.
- **`compose.yaml` has no `build:` key**, so the uncapped default builder can never be used by `docker compose up`.

### 13.2 `docker/Dockerfile`

```dockerfile
# syntax=docker/dockerfile:1.12
# Built only through scripts/image.sh (target runtime) and scripts/toolchain.sh (target toolchain), which use the capped
# quire-limited builder. The three base images are pinned by index digest and binaryen by its release checksum (W0b,
# 2026-10-28; docs/ARCHITECTURE.md §13.2).
ARG RUST_IMAGE=rust:1.99.1-trixie@sha256:53766005847cbe02c3f59bfa49c1c4bb16695f2c9e5829c3d576a15406f5e733
ARG NODE_IMAGE=node:24-trixie-slim@sha256:89c5244bc22a9bc3287d89ebc7e49b9cb5bb84cd40e6175ac3421d5a9f68b95a
ARG RUNTIME_IMAGE=gcr.io/distroless/cc-debian13:nonroot@sha256:3ee00291cf386ec52a552e6a858d555debd74776eb4bfa53fa3df2cd6677515e

FROM ${NODE_IMAGE} AS node

# Everything the image build needs. The official rust image installs the minimal profile: add clippy and rustfmt.
FROM ${RUST_IMAGE} AS toolchain-base
ARG BINARYEN_VERSION=131
ARG BINARYEN_SHA256=b5bf1f0eaf17c63ee588ff7a5954dc8f6ce2c26989051c66f24dfe9ece3e46db
# Keep in step with wasm-bindgen in vendor/inkboard/Cargo.lock.
ARG WASM_BINDGEN_VERSION=0.2.129
ENV CARGO_BUILD_JOBS=6 BINARYEN_CORES=6 RUSTUP_AUTO_INSTALL=0 \
    CC_wasm32_unknown_unknown=clang AR_wasm32_unknown_unknown=llvm-ar \
    CC_x86_64_unknown_linux_musl=musl-gcc \
    PATH=/opt/binaryen/bin:$PATH
# musl-tools: ring (under rustls) compiles C for the static CLI (D54).
RUN apt-get update && apt-get install -y --no-install-recommends clang lld llvm brotli musl-tools \
 && rm -rf /var/lib/apt/lists/*
COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
 && ln -s ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx \
 && npm install -g pnpm@11.24.0 && npm cache clean --force
RUN curl -fsSLo /tmp/binaryen.tgz "https://github.com/WebAssembly/binaryen/releases/download/version_${BINARYEN_VERSION}/binaryen-version_${BINARYEN_VERSION}-x86_64-linux.tar.gz" \
 && echo "${BINARYEN_SHA256}  /tmp/binaryen.tgz" | sha256sum -c - \
 && mkdir -p /opt/binaryen && tar -xzf /tmp/binaryen.tgz -C /opt/binaryen --strip-components=1 && rm /tmp/binaryen.tgz
RUN rustup component add clippy rustfmt && rustup target add wasm32-unknown-unknown x86_64-unknown-linux-musl \
 && cargo install --locked "wasm-bindgen-cli@${WASM_BINDGEN_VERSION}" \
 && rm -rf /usr/local/cargo/registry /usr/local/cargo/git \
 && chmod -R a+rwX /usr/local/cargo /usr/local/rustup

# Dev image quire-toolchain:latest (scripts/toolchain.sh). trunk is only for Inkboard's own public page.
FROM toolchain-base AS toolchain
RUN cargo install --locked trunk@0.22.1 \
 && rm -rf /usr/local/cargo/registry /usr/local/cargo/git && chmod -R a+rwX /usr/local/cargo

FROM toolchain-base AS inkboard-web
ARG INKBOARD_BUILD=full
WORKDIR /src
COPY vendor/inkboard vendor/inkboard
COPY patches/inkboard patches/inkboard
COPY web/inkboard-frame web/inkboard-frame
COPY scripts/inkboard.sh scripts/inkboard.sh
RUN --mount=type=cache,id=quire-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=quire-inkboard-target,target=/cache/inkboard-target \
    if [ "$INKBOARD_BUILD" = stub ]; then scripts/inkboard.sh stub --out /out/inkboard; \
    else INKBOARD_TARGET_DIR=/cache/inkboard-target scripts/inkboard.sh build --out /out/inkboard; fi

FROM toolchain-base AS web
WORKDIR /src/web
COPY web/package.json web/pnpm-lock.yaml ./
RUN --mount=type=cache,id=quire-pnpm,target=/pnpm-store pnpm fetch --frozen-lockfile --store-dir /pnpm-store
COPY web/ ./
RUN --mount=type=cache,id=quire-pnpm,target=/pnpm-store \
    pnpm install --offline --frozen-lockfile --store-dir /pnpm-store && pnpm build \
 && find dist -type f \( -name '*.js' -o -name '*.css' -o -name '*.html' -o -name '*.svg' -o -name '*.json' \) \
      -exec brotli -q 11 -k {} \; -exec gzip -9 -k -n {} \;

# licenses.txt for Settings → About (§9.6.6), served at /licenses.txt: the crates the binaries compile for Linux
# (`cargo metadata`, dev-only crates left out), the web client's production npm packages (`pnpm licenses list`, which
# reads the pnpm store), and the licence files they ship. It reads metadata and files only; nothing is compiled.
FROM web AS licenses
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY .cargo .cargo
COPY vendor/inkboard/Cargo.toml vendor/inkboard/LICENSE-MIT vendor/inkboard/LICENSE-APACHE vendor/inkboard/
COPY vendor/inkboard/crates/inkboard-core vendor/inkboard/crates/inkboard-core
COPY crates crates
COPY <<'EOF' /usr/local/lib/quire-licenses.mjs
// … (about 140 lines of JavaScript: walk cargo metadata's resolve graph from the workspace members over
// non-dev edges, read each package's LICENSE*/COPYING*/NOTICE* files, write the lists and each distinct text once)
EOF
RUN --mount=type=cache,id=quire-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=quire-pnpm,target=/pnpm-store \
    mkdir -p /out \
 && cargo metadata --format-version 1 --locked --filter-platform x86_64-unknown-linux-gnu >/tmp/cargo.json \
 && (cd web && pnpm licenses list --json --prod --config.store-dir=/pnpm-store) >/tmp/npm.json \
 && node /usr/local/lib/quire-licenses.mjs /tmp/cargo.json /tmp/npm.json /out/licenses.txt

FROM toolchain-base AS server
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY .cargo .cargo
# inkboard-core inherits version, edition, license, rust-version and [lints] from Inkboard's workspace root, so that
# manifest is needed too. Cargo reads only its [workspace] tables; the other members are never loaded.
COPY vendor/inkboard/Cargo.toml vendor/inkboard/Cargo.toml
COPY vendor/inkboard/crates/inkboard-core vendor/inkboard/crates/inkboard-core
COPY crates crates
COPY fixtures fixtures
# These COPYs order the server's rustc build after the wasm build: two rustc builds never overlap.
COPY --from=web /src/web/dist web/dist
COPY --from=licenses /out/licenses.txt web/dist/licenses.txt
COPY --from=inkboard-web /out/inkboard web/dist-inkboard
ARG QUIRE_COMMIT=unknown
RUN --mount=type=cache,id=quire-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=quire-server-target,target=/cache/target \
    QUIRE_COMMIT="$QUIRE_COMMIT" CARGO_TARGET_DIR=/cache/target cargo build --release --locked -p quire-server \
 && install -D /cache/target/release/quire-server /out/quire-server && mkdir -p /out/data

# The CLI (D54): a static musl binary, in the image and as a standalone download (scripts/cli-dist.sh). Cargo needs
# every workspace manifest, including inkboard-core's, to resolve the lockfile, but compiles only quire-cli's graph.
FROM toolchain-base AS cli-src
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY .cargo .cargo
COPY vendor/inkboard/Cargo.toml vendor/inkboard/Cargo.toml
COPY vendor/inkboard/crates/inkboard-core vendor/inkboard/crates/inkboard-core
COPY crates crates

# For the runtime image: this COPY orders the CLI's rustc build after the server's, so two rustc builds never overlap.
FROM cli-src AS cli
COPY --from=server /out/quire-server /tmp/order-after-server
RUN --mount=type=cache,id=quire-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=quire-cli-target,target=/cache/target \
    CARGO_TARGET_DIR=/cache/target cargo build --release --locked -p quire-cli --target x86_64-unknown-linux-musl \
 && install -D /cache/target/x86_64-unknown-linux-musl/release/quire /out/quire

# For scripts/cli-dist.sh (`--target cli-dist --output type=local`): the same build without the server.
FROM cli-src AS cli-alone
RUN --mount=type=cache,id=quire-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=quire-cli-target,target=/cache/target \
    CARGO_TARGET_DIR=/cache/target cargo build --release --locked -p quire-cli --target x86_64-unknown-linux-musl \
 && install -D /cache/target/x86_64-unknown-linux-musl/release/quire /out/quire
FROM scratch AS cli-dist
COPY --from=cli-alone /out/quire /quire-x86_64-unknown-linux-musl
# THIRD_PARTY.md covers every crate in the workspace lock (W7), so also the CLI's.
COPY LICENSE THIRD_PARTY.md /

FROM ${RUNTIME_IMAGE} AS runtime
COPY --from=server /out/quire-server /usr/local/bin/quire-server
COPY --from=cli /out/quire /usr/local/bin/quire
COPY --from=server --chown=65532:65532 /out/data /data
COPY LICENSE THIRD_PARTY.md /usr/share/doc/quire/
COPY vendor/inkboard/LICENSE-MIT vendor/inkboard/LICENSE-APACHE vendor/inkboard/THIRD_PARTY_LICENSES.md /usr/share/doc/quire/inkboard/
ENV QUIRE_DATA_DIR=/data QUIRE_LISTEN=0.0.0.0:8080 RUST_LOG=info MALLOC_ARENA_MAX=2
USER 65532:65532
VOLUME /data
EXPOSE 8080
# quire-server healthcheck derives its URL from QUIRE_LISTEN (port; 0.0.0.0 → 127.0.0.1). --start-interval lets
# `compose up --wait` see "healthy" within about a second. The CLI derives its default URL the same way (§7.6).
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --start-interval=1s --retries=3 \
  CMD ["/usr/local/bin/quire-server", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/quire-server"]
CMD ["serve"]
```

The `licenses` stage writes `licenses.txt`, which the server stage embeds as `web/dist/licenses.txt` (served at
`/licenses.txt`, §11; Settings → About shows it, §9.6.6). Its Rust part is every crate the workspace's binaries compile
for `x86_64-unknown-linux-gnu`: `cargo metadata --filter-platform`, walked from the workspace members over normal and
build edges, never dev-only ones. Its npm part is `pnpm licenses list --json --prod` over the web client, which reads
the pnpm store, hence the cache mount and `--config.store-dir` (the subcommand has no `--store-dir` flag). After the
lists come the licence files each package ships (`LICENSE*`, `COPYING*`, `NOTICE*` and the like; inkboard-core's at
its workspace root), each distinct text once with the packages that carry it: about 600 KB. The script is a heredoc in
the Dockerfile; `THIRD_PARTY.md` is the hand-kept record of the same lists for the source tree and the standalone CLI.
The stage reads metadata and files only, so it never overlaps a compile.

`.dockerignore` (one pattern per line: Docker's matcher has no brace expansion; `vendor/inkboard/docs` stays in the
context because patch 0004 edits it):
```
**/.git
**/target
**/node_modules
**/dist
web/dist-inkboard
.cache
data
**/*.sqlite*
vendor/inkboard/context
vendor/inkboard/spikes
vendor/inkboard/prototype-web
vendor/inkboard/packaging
e2e/test-results
e2e/playwright-report
e2e/agent/results
docs/handoff
```

`scripts/image.sh`: `tag=$(git rev-parse --short HEAD 2>/dev/null || echo nocommit)`, plus `-dirty` when
`git status --porcelain` is not empty (never on `nocommit`); then, under the lock, `scripts/toolchain.sh
--builder-only` and `docker buildx build --builder quire-limited --load -f
docker/Dockerfile --target runtime --build-arg QUIRE_COMMIT=$tag --label org.opencontainers.image.revision=$tag
-t quire:$tag -t quire:latest "$@" .`. The label lets `agent-test.sh` refuse a stale image (§14.1).

`scripts/cli-dist.sh`: under the lock, `scripts/toolchain.sh --builder-only`, then `docker buildx build --builder
quire-limited -f docker/Dockerfile --target cli-dist --output type=local,dest=dist/.cli.XXXXXX .`; it checks that the
binary and the three text files are there, writes `quire-x86_64-unknown-linux-musl.sha256`, and renames the directory
to `dist/cli`.
`dist/cli/` then holds the binary, its sha256, `LICENSE` and `THIRD_PARTY.md`. The binary is
static and runs on any x86_64 Linux; other targets (aarch64, macOS) are later work (§15).

### 13.3 `compose.yaml` (runtime limits, HTTPS front)

```yaml
name: quire
services:
  quire:
    image: ${QUIRE_IMAGE:-quire:latest}          # built by scripts/image.sh (quire-limited builder only)
    pull_policy: never
    init: true
    restart: unless-stopped
    stop_grace_period: 20s
    expose: ["8080"]                             # the project network only: no published port; Caddy is the way in (D57)
    environment:
      QUIRE_DOMAIN: ${QUIRE_DOMAIN:-localhost}   # allowed Host (any port) and the default public URL (§8)
      QUIRE_TLS: ${QUIRE_TLS:-internal}          # internal | acme; HSTS only with acme (§11)
      QUIRE_HTTPS_PORT: ${QUIRE_HTTPS_PORT:-443} # appended to the derived public URL unless 443
      QUIRE_PUBLIC_URL: ${QUIRE_PUBLIC_URL:-}    # empty: https://$QUIRE_DOMAIN[:$QUIRE_HTTPS_PORT] (§8)
      QUIRE_ALLOWED_HOSTS: ${QUIRE_ALLOWED_HOSTS:-}
      QUIRE_TRUSTED_PROXIES: ${QUIRE_PROXY_IP:-172.31.240.2}   # Caddy only: X-Forwarded-* from anyone else is ignored
      QUIRE_BACKUP_INTERVAL: ${QUIRE_BACKUP_INTERVAL:-24h}
      QUIRE_BACKUP_KEEP: ${QUIRE_BACKUP_KEEP:-7}
      QUIRE_MAX_UPLOAD_MB: ${QUIRE_MAX_UPLOAD_MB:-25}
      QUIRE_MAX_BOARD_MB: ${QUIRE_MAX_BOARD_MB:-50}
      QUIRE_TOKEN_RATE: ${QUIRE_TOKEN_RATE:-600}             # requests per minute per API token (D47)
      QUIRE_TOKEN_BYTES_PER_HOUR: ${QUIRE_TOKEN_BYTES_PER_HOUR:-1073741824}   # request body bytes per API token (D47)
      QUIRE_MIN_FREE_MB: ${QUIRE_MIN_FREE_MB:-1024}         # free-space floor, capped at 10% of /data (D33)
      QUIRE_CORS_ORIGINS: ${QUIRE_CORS_ORIGINS:-}           # empty = no CORS (D48)
      QUIRE_WORKER_THREADS: "2"                  # one per CPU of the cap below (D25)
      QUIRE_MAX_BLOCKING_THREADS: "16"
      RUST_LOG: ${RUST_LOG:-info}
    volumes: ["quire-data:/data"]
    # Runtime caps (owner requirement, D25). Raise only on purpose.
    cpus: "${QUIRE_CPUS:-2}"
    mem_limit: ${QUIRE_MEM:-1g}
    memswap_limit: ${QUIRE_MEM:-1g}              # = mem_limit → no swap
    pids_limit: 256
    ulimits: { nofile: { soft: 4096, hard: 8192 } }
    read_only: true
    tmpfs: ["/tmp:size=32m"]                     # charged to mem_limit; uploads and exports stage in /data/tmp
    cap_drop: [ALL]
    security_opt: ["no-new-privileges:true"]
    logging: { driver: local, options: { max-size: "10m", max-file: "3" } }
  caddy:
    image: caddy:2.11.7@sha256:5e5286cb37d44086d651a5363f05f43a692ce4d649d88fb1bd844579f45fa9e8   # official image, pinned by digest (D57)
    pull_policy: missing
    restart: unless-stopped
    depends_on: { quire: { condition: service_healthy } }
    ports:                                       # TCP only: HTTP/3 is off in the Caddyfile
      - "${QUIRE_BIND:-127.0.0.1}:${QUIRE_HTTP_PORT:-80}:80"     # http → https redirect, ACME HTTP-01
      - "${QUIRE_BIND:-127.0.0.1}:${QUIRE_HTTPS_PORT:-443}:443"
    environment:
      QUIRE_DOMAIN: ${QUIRE_DOMAIN:-localhost}
      QUIRE_TLS: ${QUIRE_TLS:-internal}
      GOMEMLIMIT: 100MiB                         # Go's GC stays under mem_limit
    volumes:
      - ./docker/Caddyfile:/etc/caddy/Caddyfile:ro
      - caddy-data:/data                         # certificates, ACME account, the local CA's root and keys (§13.4)
      - caddy-config:/config
    networks:
      default: { ipv4_address: "${QUIRE_PROXY_IP:-172.31.240.2}" }   # fixed: Quire trusts this address only
    # Caddy's own caps (D57).
    cpus: "${QUIRE_CADDY_CPUS:-0.5}"
    mem_limit: ${QUIRE_CADDY_MEM:-128m}
    memswap_limit: ${QUIRE_CADDY_MEM:-128m}      # no swap
    pids_limit: 128
    read_only: true
    tmpfs: ["/tmp:size=8m"]
    cap_drop: [ALL]
    cap_add: [NET_BIND_SERVICE]                  # ports 80 and 443 inside the container
    security_opt: ["no-new-privileges:true"]
    logging: { driver: local, options: { max-size: "10m", max-file: "3" } }
networks:
  default:
    ipam:
      config:
        # Dynamic addresses come only from ip_range (.8–.14), so `quire`, which starts first, can never take Caddy's
        # fixed address (.2, inside the subnet, outside the range). Without ip_range Docker hands out .2 to `quire`.
        - { subnet: "${QUIRE_NET_SUBNET:-172.31.240.0/28}", ip_range: "${QUIRE_NET_RANGE:-172.31.240.8/29}" }
volumes:
  quire-data:
  caddy-data:
  caddy-config:
```

The variables an owner sets (in `.env` next to `compose.yaml`; the README lists them): `QUIRE_DOMAIN`, `QUIRE_TLS`,
`QUIRE_BIND` (`127.0.0.1`: this machine only; `0.0.0.0`: the LAN, and required for `acme`), `QUIRE_HTTP_PORT` and
`QUIRE_HTTPS_PORT` (only when 80 or 443 are taken), `QUIRE_CPUS`, `QUIRE_MEM`, `QUIRE_CADDY_CPUS`, `QUIRE_CADDY_MEM`,
and `QUIRE_NET_SUBNET` with `QUIRE_NET_RANGE` and `QUIRE_PROXY_IP` (change all three together, only if
`172.31.240.0/28` collides with an existing network; the proxy address must lie inside the subnet and outside the
range, as Docker documents for static addresses).

**Project subnets.** A fixed subnet can exist once per host, so each project the scripts start gets its own /28 next to
the production one, exported as `QUIRE_NET_SUBNET`, `QUIRE_NET_RANGE` and `QUIRE_PROXY_IP` before `docker compose`:
`quire` (production) `172.31.240.0/28`, range `.8/29`, Caddy `.2`; `quire-e2e` `172.31.240.16/28`, `.24/29`, `.18`;
`quire-agent` `172.31.240.32/28`, `.40/29`, `.34`; `quire-smoke` `172.31.240.48/28`, `.56/29`, `.50`. They never
collide with a running production project. The scripts also pass `--env-file /dev/null` and unset the owner-facing
variables listed above before exporting their own, so neither the owner's `.env` nor the shell (a public domain,
`acme`, raised caps) leaks into a test project: their caps are always the defaults of this file.

`compose.e2e.yaml` (Compose ≥ 2.24 merge tags; a plain `tmpfs:` entry would collide with the `/data` volume). The e2e
and agent projects start only the `quire` service (`up -d --wait quire`): Playwright and the CLI talk to Quire
directly over plain HTTP on the project network, and Caddy is not started (§14):
```yaml
services:
  quire:
    volumes: !override
      # mode 1777 lets uid 65532 write: without it the tmpfs takes the image's /data mode (0755) with owner root.
      - { type: tmpfs, target: /data, tmpfs: { size: 268435456, mode: 0o1777 } }   # charged to mem_limit
    environment:
      QUIRE_SETUP_TOKEN: e2e-setup-code
      QUIRE_PUBLIC_URL: http://quire:8080      # direct, plain HTTP: the session cookie is quire_session (§8)
    cgroup_parent: ${QUIRE_CGROUP_PARENT:-}    # quire-build.slice when installed (scripts/lib/slice.sh); else own caps
```
The `/data` tmpfs needs its mode: runc gives a new tmpfs the mode of the image's directory (`/data` is 0755, owner
65532) but owner root, so without `mode` uid 65532 cannot create `/data/files`. Compose reads `mode: 1777` as decimal;
the YAML octal `0o1777` is 1023, the intended value. The Playwright container joins the `quire-e2e_default` network.
The agent harness (§14.1) uses the same override,
under its own project name `quire-agent`, and adds `compose.agent.yaml` only when it must publish a loopback port:
```yaml
services:
  quire:
    ports: ["127.0.0.1:${QUIRE_AGENT_PORT:-18080}:8080"]   # loopback only, straight to Quire; the Host allowlist accepts 127.0.0.1
```

`compose.smoke.yaml` (for `scripts/image-smoke.sh`, project `quire-smoke`): the full project, Caddy included, in
`internal` mode for `localhost`, reachable only on a loopback port, with throwaway volumes. `image-smoke.sh` exports
`QUIRE_DOMAIN=localhost`, `QUIRE_TLS=internal`, `QUIRE_HTTPS_PORT=18443` (so the derived public URL is
`https://localhost:18443`) and the smoke subnet:
```yaml
services:
  quire:
    volumes: !override
      # mode 1777 lets uid 65532 write: without it the tmpfs takes the image's /data mode (0755) with owner root.
      - { type: tmpfs, target: /data, tmpfs: { size: 268435456, mode: 0o1777 } }
    environment:
      QUIRE_SETUP_TOKEN: smoke-setup-code
    cgroup_parent: ${QUIRE_CGROUP_PARENT:-}
  caddy:
    # The same variable as the derived public URL, so the two cannot diverge. No port 80, never 443: a running Quire
    # is untouched.
    ports: !override ["127.0.0.1:${QUIRE_HTTPS_PORT:-18443}:443"]
    volumes: !override
      - ./docker/Caddyfile:/etc/caddy/Caddyfile:ro
      - { type: tmpfs, target: /data, tmpfs: { size: 16777216 } }      # a fresh local CA per run
      - { type: tmpfs, target: /config, tmpfs: { size: 1048576 } }
    cgroup_parent: ${QUIRE_CGROUP_PARENT:-}
```
`docker cp` (and `docker compose cp`) cannot read a tmpfs mount, so the smoke reads the root with
`docker compose -p quire-smoke exec -T caddy cat /data/caddy/pki/authorities/local/root.crt` (the official Caddy image
has a shell and `cat`).

The image holds both binaries: `quire-server` (entrypoint) and the CLI `quire` (D54). Inside the container the CLI
needs no URL (it derives `http://127.0.0.1:8080` from `QUIRE_LISTEN`, §7.6), so
`docker compose exec -e QUIRE_TOKEN quire quire …` drives the API, and `docker compose exec quire quire admin …` runs
the admin commands of §7.8 without any token. The container's own healthcheck (§13.2) also stays inside it.

Backups: `docker compose exec quire quire admin backup [--out PATH]`. To restore: stop the container, replace
`/data/quire.db`, delete the `-wal` and `-shm` files, start it again. An export (D34) restores the same way but has no
password and no API tokens: run `quire admin reset-password` (and `create-token`) afterwards. The README explains that an off-host copy of the volume is
needed, and that a bind-mounted `/data` must be owned by `65532:65532` (named volumes copy the image's ownership).
The `caddy-data` volume holds the local CA's private key and the ACME account: back it up like a secret.

### 13.4 HTTPS front (`docker/Caddyfile`, D57)

```caddyfile
# Quire's HTTPS front (docs/ARCHITECTURE.md §13.4). compose.yaml sets QUIRE_DOMAIN and QUIRE_TLS.
{
	admin off                 # no admin API; a Caddyfile change needs `docker compose restart caddy`
	skip_install_trust        # never touch a trust store; the owner trusts the local root by hand (below)
	servers {
		protocols h1 h2       # no HTTP/3: UDP 443 is not published
	}
}

(tls_internal) {
	tls internal
}
(tls_acme) {
	tls {
		protocols tls1.2 tls1.3   # Caddy's automatic HTTPS: public ACME certificates, default issuers
	}
}

{$QUIRE_DOMAIN} {
	import tls_{$QUIRE_TLS}
	# No `encode` and no `header`: Quire sends its own precompressed br/gzip and every security header.
	reverse_proxy quire:8080 {
		transport http {
			compression off   # do not add Accept-Encoding toward Quire or decode its answer
		}
	}
}
```

Caddy logs "Caddyfile input is not formatted" at every start, because of the spaces before the inline comments; it is
harmless, and `caddy fmt` would only move the comments.

| Mode | Settings | Certificates | Reached by |
|---|---|---|---|
| LAN or local (default) | `QUIRE_TLS=internal`; `QUIRE_DOMAIN=localhost`, or a LAN name such as `quire.lan` that the LAN's DNS or each client's `/etc/hosts` resolves to the host; `QUIRE_BIND=127.0.0.1` (this machine) or `0.0.0.0` (the LAN) | Caddy's local CA in `caddy-data` (the root is valid for 10 years; leaf certificates renew automatically) | Devices that trust the root (below) |
| Public | `QUIRE_TLS=acme`; `QUIRE_DOMAIN=wiki.example.com`; `QUIRE_BIND=0.0.0.0`; a DNS A record to the host; ports 80 and 443 reachable from the Internet | Let's Encrypt through Caddy's automatic HTTPS, kept in `caddy-data` (keep the volume: ACME rate limits) | Anyone; Quire adds HSTS (§11) |

How Caddy handles a request:
- It keeps the client's `Host`, discards any `X-Forwarded-*` the client sent (it trusts no proxy in front of it), and
  sets `X-Forwarded-For` (one address), `X-Forwarded-Proto` and `X-Forwarded-Host` (Quire ignores `-Host`). Quire
  trusts the first two only from Caddy's fixed address (§8).
- It passes request and response bodies through as streams (uploads, the export tar) and Quire's `Content-Encoding`
  and `ETag` unchanged; it neither compresses nor decompresses (§11).
- Port 80 answers ACME challenges and redirects to HTTPS. With remapped host ports (`QUIRE_HTTP_PORT`,
  `QUIRE_HTTPS_PORT`) the redirect still names 443, so use the HTTPS URL with its port; Quire's derived public URL
  includes `QUIRE_HTTPS_PORT` (§8).
- Only `QUIRE_DOMAIN` is served; another name gets no certificate and never reaches Quire.

**Trusting the local CA** on the owner's machine (Ubuntu), once per CA (a new `caddy-data` volume means a new root):
```sh
docker compose cp caddy:/data/caddy/pki/authorities/local/root.crt ./quire-local-ca.crt
# The system bundle (curl, the quire CLI, most tools): one privileged step. The owner runs it, or, in an agent
# session, the coordinator runs it through the askpass dialog; implementing agents never do.
SUDO_ASKPASS=/home/staffan/.local/bin/claude-askpass sudo -A \
  -p "trust Quire's local HTTPS certificate authority :: install quire-local-ca.crt into /usr/local/share/ca-certificates and run update-ca-certificates" \
  sh -c 'install -m 0644 "$1" /usr/local/share/ca-certificates/quire-local-ca.crt && update-ca-certificates' sh "$PWD/quire-local-ca.crt"
# Chrome and Chromium read their own NSS store (per user, no root; certutil is in libnss3-tools):
certutil -d "sql:$HOME/.pki/nssdb" -A -t "C,," -n "Quire local CA" -i quire-local-ca.crt
# Firefox: Settings → Privacy & Security → Certificates → View Certificates → Authorities → Import, and tick
# "Trust this CA to identify websites".
```
Other LAN devices import the same `root.crt` (phones as a CA profile). The CLI either uses the system bundle once the
root is in it, or `--ca-file quire-local-ca.crt` (D54). Only `root.crt` ever leaves the volume: whoever holds
`root.key` can impersonate any site to every device that trusts the root.

---

## 14. Testing (everything through the capped wrappers)

| Layer | What | Command |
|---|---|---|
| quire-core | `rank` proptests (always strictly between; cross-checked with npm `fractional-indexing` vectors); a tree model test (random create, move, trash and restore never cycles and matches an in-memory model); `ytext` against `fixtures/ydoc`; `board` split → splice round-trip equal to the input, `inspect` text, too-new rejection | `scripts/dev.sh cargo test -p quire-core --locked` |
| quire-store | One test binary (`tests/store/main.rs` + one module per workstream) on a temp-file DB with WAL: CTEs, triggers, trash/restore, collection delete/restore, a leaves-first purge at depth 2,000, FTS sync with `integrity-check`, Y compaction equal before and after, delete-only saves, `missing_deps`, board assets dedupe, attachment and blob GC, backup through the real `Db` (maintenance connection) + `quick_check`, migrations validate | `scripts/dev.sh cargo test -p quire-store --locked` |
| quire-server API | One test binary (`tests/api/main.rs` + modules) using axum-test on `quire_server::app()`, with cheap argon2 parameters (`QUIRE_TEST_ARGON2=1`): setup, login, cookie attributes and renewal, 429, Host allowlist (421), CSRF (403 for `Sec-Fetch-Site: cross-site`, for a cross-site `Origin` with no `Sec-Fetch-Site`, and for neither header), 415, 413, 412/428, 409 cycle, error shape (including 408), CSP/cache/encoding headers, SPA fallback against API 404, bearer tokens (format check, classification, revocation with children, scope and expiry caps, `session_required`), the `is_quire_bearer` predicate (cookie ignored for a Quire token; Basic and foreign Bearer values take the cookie path with CSRF), CORS off by default and per allow-list, per-token 429 (requests and bytes) with `Retry-After`, 507 below the free-space floor, cursor binding, ETag/If-Match on items, documents and boards; behind the proxy (peer address injected through `ConnectInfo`): `X-Forwarded-For`/`-Proto` honoured only from `QUIRE_TRUSTED_PROXIES`, the cookie name and `Secure` per request scheme, HSTS only for HTTPS in `acme` mode, `QUIRE_DOMAIN` with any port in the Host allowlist, the derived public URL; items published on create | `scripts/dev.sh cargo test -p quire-server --locked` |
| Integration | `tests/api/integration.rs` (W7): flows that span workstreams, e.g. board blobs freed after purge + GC, search finds a board label over HTTP | `scripts/dev.sh cargo test -p quire-server --locked --test api -- integration::` |
| Contract | ts-rs export tests write `web/src/api/gen`; `git diff --exit-code web/src/api/gen` must be clean | `scripts/dev.sh cargo test -p quire-api --locked export_bindings && git diff --exit-code web/src/api/gen` |
| OpenAPI | `openapi::spec_matches_committed` (`docs/api/openapi.json`, regenerate with `UPDATE_OPENAPI=1`), `openapi::spec_parses_as_3_1` (oas3), `security::every_operation_enforces_its_declared_scope`, `security::public_operations_are_exactly` (§7.4); no plain `.route(` under `src/routes/`, no `.route("/api` in the crate | `scripts/dev.sh cargo test -p quire-server --locked --test api -- openapi:: security:: && git diff --exit-code docs/api/openapi.json`, `! grep -rn '\.route(' crates/quire-server/src/routes`, `! grep -rn '\.route("/api' crates/quire-server/src` |
| Conversion | `quire-core::convert` and `schema`: Rust schema equals `fixtures/schema/v1.json`; Y → raw and normalized JSON equal the web's `fixtures/ydoc/*.y.json` and `*.pm.json`; Markdown goldens `fixtures/markdown/*.md` → `*.expected.md` and `*.warnings.json`; the round-trip and idempotence proptests (§6.4); the diff keeps `blockId`s and Y identity of unchanged blocks, also after an insertion at the top; a Markdown PUT leaves blocks it does not change untouched; a repeated PUT is a byte-identical no-op; a stale base with `merge` converges with a concurrent web update; UTF-16 offsets with non-BMP text; web side: `editor/serverWritten.test.ts` opens `fixtures/ydoc/server/*.bin` | `scripts/dev.sh cargo test -p quire-core --locked -- convert:: schema::`, `scripts/dev.sh pnpm --dir web test src/editor/serverWritten` |
| CLI | One test binary (`crates/quire-cli/tests/cli/main.rs` + modules) running the real `quire` binary with assert_cmd 2.2 against `quire_server::testing::TestServer` (an in-process server on `127.0.0.1:0`, temp data dir, an owner and one token per scope) and, for transport cases, a scripted mock HTTP listener: URL and token resolved as a pair, profile permissions and atomic writes, offline token check, `--json` on every command, JSON Lines on stderr, every exit code, retries only of unprocessed requests, no redirects, the `api` path guard, admin passthrough, spec coverage, AGENTS.md examples parse, and end-to-end command flows | `scripts/dev.sh cargo test -p quire-cli --locked` |
| Dependency graph | The server links no TLS stack and the CLI no OpenSSL, bundled webpki-roots, store or document engine (D29, D40, D54) | `! scripts/dev.sh cargo tree --locked -p quire-server -e normal --prefix none \| grep -qE '^(rustls\|ureq\|reqwest\|native-tls\|openssl) '`, `! scripts/dev.sh cargo tree --locked -p quire-cli -e normal --prefix none \| grep -qE '^(openssl\|openssl-sys\|native-tls\|rusqlite\|yrs\|inkboard-core\|comrak\|webpki-roots) '` |
| Agent harness | CLI scenarios against the production image (§14.1) | `scripts/image.sh && scripts/agent-test.sh` |
| Lint | clippy, rustfmt (never `--all`, §4.1), pins | `scripts/dev.sh cargo clippy --workspace --all-targets --locked -- -D warnings`, `scripts/fmt.sh --check`, `scripts/check-pins.sh`, `scripts/dev.sh cargo tree --locked -e normal -i libsqlite3-sys` (fails if two versions exist) |
| Web unit | vitest + happy-dom + Testing Library (`maxWorkers: 4`): tree builder and visible rows, DnD hit zones → positions, keyboard tree navigation, autosave queue (merge, retry, outbox, flush on hide), board save loop against a fake frame, search snippet rendering, Contents column; the headless TipTap+Yjs fixture writer (`pnpm --dir web fixtures`) | `scripts/dev.sh pnpm --dir web test`, `scripts/dev.sh pnpm --dir web typecheck`, `scripts/dev.sh pnpm --dir web build` |
| Inkboard patch | Build checks plus a frame test (Playwright, frame.html served statically by `e2e/serve-static.mjs`, which sends the §11 frame CSP and `X-Frame-Options` and negotiates the `.br`/`.gz` copies, so the engine runs under production headers): open `fixtures/boards/frame-test.inkboard`; snapshot round-trip (parsed JSON equal, compared without each page's `view`, which the engine refits to the canvas on open); focus the canvas, press `R` (rectangle tool, `keymap.rs:377`) and drag, so `onChange` reports a higher revision and the snapshot has one more object; `markSaved` → `isDirty()==false`; Ctrl+S → `onSave`; read-only blocks the drag; setting `location.hash` changes nothing; `setBoard` swaps the board and resets undo; `setBoard` while a text label is being edited leaves `isDirty()==false` after `markSaved`; a second `mount` rejects; after F5 in the canvas, R + drag still adds an object (Present is inert when hosted); `setActive(false)` stops painting: WebGL draw calls, counted after checking that they do grow while active, stay flat for 2 s without input (eframe 0.33's web runner requests an animation frame on every display frame and paints only when a repaint is due, so a `requestAnimationFrame` counter shows nothing: 121 calls and 0 paints in 2 s while inactive); **background page**: with a dirty board, bring another page to the front for 60 s, then `snapshot()` works and no `onError{panic}` fired, and a board opened while hidden mounts only once visible. Headless Chromium keeps reporting `visibilityState === 'visible'` for a background page, so that test records a skip, a second test emulates the visibility (an init script overrides `document.visibilityState`) to check the frame's own wait, and §16 records the manual browser check. Also: the panic backup listeners (one `onError{panic}` per trap, none for a plain `Error`), an unreadable board on a later `open()` (rejects `load`, keeps the previous board), `connect()` retried after a `manifest.json` 500, a `format_version` mismatch (`start`, final), Mod+K and double-Esc forwarding, no CSP violation under the frame CSP, the wasm served as br, and `setBoard` resetting undo with a positive control (Ctrl+Z first undoes a rectangle). A `beforeAll` fails once against the stub dist ("run scripts/dev.sh scripts/inkboard.sh build first"), so the frame test needs a real engine build | `scripts/dev.sh scripts/inkboard.sh check`, `scripts/dev.sh scripts/inkboard.sh build`, `scripts/e2e.sh frame` |
| E2E smoke | Playwright against the production image (compose + e2e override); PLAN W7 | `scripts/image.sh && scripts/e2e.sh smoke` |
| Image smoke | `compose.yaml` + `compose.smoke.yaml` (project `quire-smoke`, its own subnet, Caddy in `internal` mode for `localhost` on `127.0.0.1:18443` only, throwaway volumes; §13.3), so the caps checked are the compose ones. `QUIRE_SMOKE_LEVEL` is `full` by default; `basic` also passes with an `INKBOARD_BUILD=stub` image, and `full` fails on one with a hint. Exit codes: 0 pass, 1 a check failed, 2 usage, 3 a missing prerequisite (docker, curl, jq or the image), 4 the project not healthy (at most 180 s; log tails are printed). `basic`: both binaries start (`quire-server --version`, `quire --version` inside the container); the `quire` container has no published port; Caddy's root is read from its tmpfs with `docker compose exec -T caddy cat …/root.crt` (bounded poll; `docker cp` cannot read a tmpfs, §13.3) and `curl --cacert <root> --resolve localhost:18443:127.0.0.1 https://localhost:18443/healthz` answers 200 over HTTPS (and fails without `--cacert`); `docker inspect` shows NanoCpus=2000000000 and Memory = MemorySwap = 1073741824 for `quire`, NanoCpus=500000000 and Memory = MemorySwap = 134217728 for `caddy`. `full` adds, all through Caddy: CSP on `/`; `.wasm` with `Accept-Encoding: br` → `Content-Encoding: br`, `application/wasm`, exactly the manifest's `br` byte count, and bytes whose sha256 starts with Quire's ETag (the hash of the embedded `.br`, §11), so Caddy passed them through byte for byte and re-encoded nothing; `frame.html` with the frame CSP and `X-Frame-Options: SAMEORIGIN`; `manifest.json` next to `frame.html` → 200, identity, `application/json`, the build in the path (`frame.js` cannot connect without it); setup with `smoke-setup-code` (sent with `Origin: https://localhost:18443`, since curl sends no fetch metadata and the CSRF rule of §8 covers every non-bearer POST) sets `__Host-quire_session` with `Secure`; no `Strict-Transport-Security` (`internal` mode); `docker stats` stays under both limits during the smoke | `scripts/image-smoke.sh` (host prerequisites: Docker, curl, jq) |

`scripts/e2e.sh <frame|smoke>`:
1. Takes the build lock (exporting `QUIRE_LOCK_HELD=1`, so the nested `dev.sh` calls below do not lock again) and
   calls `quire_slice` (§13.1).
2. `scripts/dev.sh pnpm --dir e2e install --frozen-lockfile`.
3. `smoke` only: `QUIRE_NET_SUBNET=172.31.240.16/28 QUIRE_NET_RANGE=172.31.240.24/29 QUIRE_PROXY_IP=172.31.240.18
   docker compose --env-file /dev/null -p quire-e2e -f compose.yaml -f compose.e2e.yaml up -d --wait quire`, with a
   trap that runs `down -v` on exit. Only `quire` starts:
   Playwright talks to `http://quire:8080` directly on the project network, not through Caddy (the HTTPS path is the
   image smoke's job).
4. `QUIRE_DEV_IMAGE=mcr.microsoft.com/playwright:v1.64.1-noble@sha256:47b510dd…b8ea QUIRE_BUILD_CPUS=4 QUIRE_BUILD_MEM=6g
   scripts/dev.sh node e2e/node_modules/@playwright/test/cli.js test -c e2e/playwright.config.ts --project <p>`
   (lower when `QUIRE_BUILD_CPUS` or `QUIRE_BUILD_MEM` are lower; an explicit `QUIRE_BUILD_CPUSET` is cut to its last N
   CPUs; extra arguments go to Playwright),
   adding `QUIRE_DEV_NETWORK=quire-e2e_default` for `smoke` only (the network exists only then). The app (2 CPUs,
   1 GiB) plus Playwright (4 CPUs, 6 GiB) is exactly the 6-CPU total and 7 of the 12 GiB, and with the slice installed
   both count against it. Caddy (0.5 CPU) would not fit, which is one more reason it stays out of e2e runs.

Chromium is launched with `--use-angle=swiftshader --enable-unsafe-swiftshader` so WebGL2 works headless.
Inkboard's `web-test` feature is never shipped.

### 14.1 Agent test harness (`scripts/agent-test.sh`, D56)

A repeatable end-to-end run of the shipped CLI against the shipped image, for AI agents and CI. Host prerequisites:
Docker with Compose ≥ 2.24, bash and jq. It never builds: it uses `${QUIRE_IMAGE:-quire:latest}` and refuses an image
that is missing ("run scripts/image.sh first") or whose `org.opencontainers.image.revision` label, without `-dirty`,
differs from `git rev-parse --short HEAD` (`--allow-stale` overrides; a `-dirty` image or worktree only warns). Exit
codes: 0 every step passed, 1 a scenario failed, 2 usage, 3 a prerequisite is missing or the image is stale, 4 the app
did not become healthy.

`scripts/agent-test.sh [run] [--scenario GLOB] [--keep] [--allow-stale]`, `up [--ttl MIN]`, `status`, `down`:
1. Before locking, `run` and `up` exit 3 when the `quire-agent` project is already up. Then the script takes the build
   lock (§13.1), like `e2e.sh`, and calls `quire_slice`: the app container counts against the build total.
2. `QUIRE_NET_SUBNET=172.31.240.32/28 QUIRE_NET_RANGE=172.31.240.40/29 QUIRE_PROXY_IP=172.31.240.34 docker compose
   --env-file /dev/null -p quire-agent -f compose.yaml -f compose.e2e.yaml up -d --wait quire` (plus `-f compose.agent.yaml` with `--keep`, `up`, or `QUIRE_AGENT_CLI`), with
   a trap that runs `down -v` unless the app is kept. Only `quire` starts: the CLI talks to Quire directly over plain
   HTTP (inside the container, or on the loopback port), never through Caddy. The app keeps the compose runtime caps
   (2 CPUs, 1 GiB, no swap) and joins `quire-build.slice` through `compose.e2e.yaml` when the slice is installed;
   `/data` is a tmpfs, so nothing persists.
3. Provisioning, inside the container: `quire admin create-owner --email agent@quire.invalid --name Agent --workspace
   "Agent test" --generate-password --json`, then `quire admin create-token` three times (scopes `read`, `write`,
   `admin`, `--expires 2d --json`: the server caps a created token at its creator's `expires_at` to the millisecond, so
   a scenario's `token create … --expires 1d` from a 1d parent made a moment earlier would be refused). The tokens
   stay in shell variables, are passed with `docker exec -e QUIRE_TOKEN` (the value comes from the environment, never
   from argv), and are never printed. The server sees the new owner on
   its next request (§8).
4. Runs `e2e/agent/scenarios/NN-*.sh` in order (filtered by `--scenario`). Each scenario is a bash file that sources
   `e2e/agent/lib.sh` and uses its helpers:
   - `q [--as read|write|admin] <args…>`: runs `quire --json <args…>` (by default `docker exec -i` into the container,
     where the CLI talks to `127.0.0.1:8080`; with `QUIRE_AGENT_CLI=/path/to/quire`, that host binary against the
     published loopback port). stdout goes to `$OUT_FILE`, and to `$OUT` too when it is text (bash variables cannot hold
     NUL bytes); `$ERR` (JSON Lines, §7.7) and `$RC` are set; stdin is forwarded;
   - `q_raw NAME [--as …] <args…>`: the same without `--json`, stdout written byte for byte to `results/NAME` on the
     host (exports, previews, attachments);
   - `expect_rc N`, `expect '<jq filter>'` (`jq -e` on `$OUT_FILE`), `expect_err '<jq filter>'` (on the last line of
     `$ERR`), `ok "<name>"`;
   - `fixture NAME`: a host path under `e2e/agent/fixtures/` or `fixtures/markdown/`, used only as stdin
     (`q doc put … --md - < "$(fixture x.md)"`), because the in-container CLI cannot read host files and the distroless
     image has no shell. `--out` points into the container's 32 MB `/tmp` and serves only the JSON metadata checks;
   - `scratch_collection`: a fresh collection named after the scenario, so scenarios never depend on each other's data
     and purge only their own items;
   - also `q`/`q_raw --token T` (any token, still through the environment), `q_admin ARGS…` (`quire --json admin …`
     inside the container), `unknown_token` (a well-formed token nobody has), `val 'JQ'` and `ok NAME [DETAIL]` (a
     detail for a passing step, masked). TAP lines go to fd 3, so a check inside `$(…)` still prints; a failed check
     ends its scenario and the runner goes on with the next. A host CLI (`QUIRE_AGENT_CLI`) runs with every `QUIRE_*`
     variable of the caller unset and `QUIRE_CONFIG` pointing at a file that never exists, so neither the caller's
     profile nor an exported `QUIRE_PROFILE`, `QUIRE_TOKEN_FILE`, `QUIRE_CA_FILE` or `QUIRE_ALLOW_HTTP` reaches a
     scenario; only `QUIRE_URL` and `QUIRE_TOKEN` are set.
5. Prints TAP (`ok N - 03-docs: markdown round trip`) and writes `e2e/agent/results/report.json` (git-ignored:
   `{image, revision, head, started, finished, passed, failed, failed_scenarios, steps: [{n, scenario, step, status,
   duration_ms}, plus command, rc, detail, out and err for a failure, and the detail of a passing step that gives
   one]}`), with `steps.jsonl` beside it. Every string written to either has `qpat_[0-9A-Za-z]{46}` replaced by
   `qpat_***`; scenario 10 records `token create`'s output as a step detail and checks that the report holds
   `qpat_***` and not the secret, so the masking is exercised on every passing run.

`up` (or `run --keep`) leaves the app on `127.0.0.1:${QUIRE_AGENT_PORT:-18080}` for `--ttl` minutes (default 15, as PLAN
R1 asks of any running app; an owner-accepted default, §17.3): a background holder keeps the build lock for that time
and then runs `down`; `status`
prints the time left. It writes, under `umask 077`, `.cache/agent-test/env` (absolute paths: `QUIRE_URL`, and
`QUIRE_TOKEN_FILE` pointing at `write.token`) next to `read.token`, `write.token` and `admin.token`. Build a host CLI
with `scripts/cli-dist.sh` before `up`, since the lock stays held. `down` stops the holder and removes the project and
the files.

The scenario suite (PLAN, W8) covers authentication and scopes, collections and the tree (including a 409 cycle),
documents in Markdown and JSON (golden round trip, 412 on a stale ETag, `--merge`, conversion warnings), boards
(unknown fields preserved, SVG preview, 412), search, stars and lists, trash, restore and purge of the scenario's own
items, attachments, export, tokens, and the OpenAPI document. A scenario never sleeps for timing; it polls with a bounded loop where the server is
asynchronous (board previews).

---

## 15. Forward-compatibility notes

| Later feature | What M1 already does |
|---|---|
| Live doc co-editing | Yjs log in `doc_updates` with a `user_id` per update, and `doc_clients` for attribution after compaction. Later: `GET /api/v1/items/{id}/doc/ws` speaking y-protocols sync and awareness (yrs `sync`), one room per open doc evicted after 60 s idle (the room is also the doc cache that replaces `doc_work` loads), the same persistence and compaction, the server-seeded `Y.Text('title')`, and an explicit Host and Origin check on the upgrade (the CSRF middleware does not cover GET). Caddy's `reverse_proxy` passes WebSocket upgrades through as they are (D57). Client: y-websocket 3.1.0 and CollaborationCaret. |
| Server-owned board sync | `board_state` + `board_assets` = Sequencer snapshot + asset manifest; `seq` column. Needs upstream patches: a headless session host extracted from `session_host.rs`, `Sequencer::with_seq`, a browser WebSocket `Connection` (`ws-transport` feature), and trusted-server mode without link E2E. Log budgets use the existing `Sequencer::set_budgets`. Server and wasm come from the same submodule pin, and `scripts/check-pins.sh` keeps the shared crate versions equal (exact protocol match). |
| Multi-user, roles, invites | `users.role` lookup table, `created_by`/`updated_by`, per-user stars and views, the draft state with derived visibility and its API (publish, unpublish, `/drafts`, `drafts_count`, `include_drafts`; D23), so draft-first creation and the Drafts UI become a default change plus UI behind `config.multi_user`, the `Actor` + `authz::require` seam, `Viewer` + `shared::visible_items` on every list query, the per-viewer `viewer_epoch` in the tree ETag, attachment authorization through `item_attachments`, and the `multiUser` capability flag hiding avatars, Share, Invite and "Viewed by N". Later tables: `groups`, `collection_members`, `item_permissions`, `invites`, `shares`. |
| OIDC/SSO | `users.password_hash` is nullable. Later: an `identities(user_id, issuer, subject)` table. Sessions are unchanged. |
| Version history UI | `revisions` is written from M1, with format and GC roots; restore is defined in §6.1. |
| Backlinks | `item_links` is maintained from M1. |
| Templates, icons | `is_template` and `icon` columns; the `/templates` route is reserved. |
| Board images as attachments | `/api/v1/blobs/{sha}` is reserved; `board_assets` is already hash-based. Later: manifest-mode saves, without inline base64. |
| New item kinds, formats, link kinds | One `INSERT` into the lookup tables (D37). |
| PNG thumbnails | Behind an `inkboard-formats` upstream patch that adds `raster`, `pdf` and `latex` features. |
| Board import over the API | The same upstream feature split: with `inkboard-formats` at `default-features = false`, `import_any(name, content, &ApproxMeasurer)` costs 13 crates (D52). The route `POST /api/v1/items/{id}/board/import` (raw file body, `?filename=` for format detection, returns the new board's `ItemSummary` and the importer's `Report`) and `quire board import` are reserved. |
| API v2 | Every route is under `/api/v1` and every client goes through the spec; a v2 is served next to v1, and the CLI picks the highest version both sides support from `GET /api/v1/config`. |
| Finer token scopes, multi-user tokens | Scopes are rows in `api_token_scopes` over the `token_scopes` lookup table (one INSERT adds e.g. `boards:write`); tokens belong to a user and pass through the same `Actor`/`authz::require` seam, so multi-user tokens need no schema change. |
| OAuth device flow or OIDC tokens for the CLI | `Auth<S>` resolves an `Actor` from a header; another bearer format is one more branch. The CLI's profile already separates URL and credentials. |
| Live sync with API writes | API document writes are Yjs updates from their own client id, so a room broadcasts them like any peer's; API board writes become diffs against the room's Sequencer document (§6.3). |
| Change feed (webhooks or SSE) for agents | `doc_updates.seq`, `board_state.rev`, `tree_version` and `meta_version` are monotonic per object; a feed can expose them without new state. |
| An MCP server for agents | The CLI's `--json` contract and the OpenAPI spec are enough to wrap; Inkboard's `inkboard-mcp` (stdio, `crates/inkboard-mcp`) is a model for the board half. |
| Whole-workspace Markdown export | `GET …/doc/markdown` exists (D49); the zip is a loop over the tree plus board `.inkboard` files. |
| More CLI targets | `scripts/cli-dist.sh` builds `x86_64-unknown-linux-musl`; aarch64 Linux is one more target and C toolchain in `toolchain-base`, macOS needs a cross SDK (later). |
| Idempotent creates | `Idempotency-Key` on POST creates (stored per actor for 24 h, replaying the first response) is an additive v1 change. Until then the CLI reports `details.sent` and AGENTS.md says to list before retrying a create (§7.7). |
| Board conversion in Quire's tooling | A `scripts/inkboard.sh cli` building `ink` from `.cache/inkboard-src` under the caps, or import over the API after the upstream feature split (D52). |

---

## 16. Open risks

1. **The Inkboard patch builds are unverified.**
   - Unchecked: the `Closure`-object API under `forbid(unsafe_code)` (the pattern itself is proven in `web_test.rs`),
     the optional `inkboard-net` (patch 0003), and whether getrandom 0.2 or 0.3 drops out of the wasm graph.
   - Mitigation: 0003 can be dropped; `scripts/inkboard.sh check` runs first in W5.
2. **WebGL in headless Chromium and SwiftShader in CI.** If the frame test cannot get WebGL2, the frame and smoke board
   steps fall back to checking `onError{kind:'webgl'}` plus the preview path, and a manual browser check is recorded.
3. **Background tabs.** In 0.33 hidden tabs run no frames, so `onChange` stalls; `snapshot()` is synchronous and works
   anyway, and the host flushes on `visibilitychange`. The 0.36 texture panic is avoided by the visibility wait (D13).
   The frame test skips its real background-page case: headless Chromium keeps reporting `visibilityState ===
   'visible'` for a page brought behind another (W5 D15); the emulated-visibility test passes. **The manual check in a
   real browser has not been done yet:** a dirty board in a background tab for 60 s, then `snapshot()` works, no
   `onError{panic}` fired, and a board opened while hidden mounts only once visible. Record its result here.
4. **After a panic** the runner is poisoned (`app_mut` returns `None`), so the last unsaved edits are lost. Frequent
   saves and the IndexedDB snapshot limit the loss.
5. **Large boards.** Inline base64 images have no resize cap. The 50 MB limit and the serialized `board_work` permit
   mitigate this until board images move to attachments.
6. **Format drift upstream.** `VERSION > 2` would be rejected by older builds. The submodule is pinned, `engine_version`
   is stored per board, and the server refuses to start on a mismatch.
7. **Yjs schema renames** need a Y-level migration. `schema_version` is tracked, and the node and mark names in §6.1 are
   frozen.
8. **SVG previews of image-heavy boards** can be several MB. List views show the placeholder when `svg_len` is over 2 MB.
9. **Keyboard.** Global shortcuts are dead while the canvas has focus, apart from the forwarded set. Inkboard captures
   Tab, Backspace and arrow keys.
10. **No Safari, iOS or real-device testing upstream.** Phones default to the preview.
11. **Whole-collection tree loading.** Collections with more than 10k items would need lazy loading (an API addition,
    not a data change).
12. **Pins not checked against registries:** deadpool-sqlite 0.14 and rusqlite_migration 2.6 against rusqlite 0.40
    (one `libsqlite3-sys`; fallbacks in D2), tower-http 0.7 layers, TypeScript 7 tooling, the
    `@tiptap/extension-drag-handle-react` licence, and the days-old comrak 0.59.0 (0.58.0 the same day) and ureq 3.5.0
    (`rustls-no-provider` plus an explicit ring provider, `tls::parse_pem`). W0b checks each one and records any
    fallback here.
13. **The total build cap depends on a one-time root step** (`scripts/install-build-slice.sh`, approved by the owner on
    2026-10-28 and run by the coordinator through the askpass dialog, §13.1). Until it is installed, and on any other
    machine, the per-container caps and the lock hold and every script warns, but two capped containers started
    outside the scripts could together exceed 6 CPUs. `IOWeight` has no effect where the disk scheduler ignores
    `io.weight` (`--check` prints it). If Docker is ever switched to the cgroupfs driver, `quire_slice` stops passing
    the slice and warns.
14. **utoipa 6.0.0 is five weeks old** (released 2026-09-22). If it or utoipa-axum 0.3.0 fails to build or misbehaves
    with axum 0.8.9, W0b falls back to utoipa 5.5.0 + utoipa-axum 0.2.0 (both OpenAPI 3.1) and records it here (D41).
15. **Y ↔ ProseMirror fidelity in Rust is unproven.** The details y-tiptap relies on (null attrs not stored, `{}`
    for marks without attrs, UTF-16 offsets, element and text ordering) are covered only by the cross-language
    fixtures in both directions (§6.4). They are the gate: a mismatch blocks the content routes, not the web editor.
16. **The merge window is bounded by compaction.** `?merge=true` works only while the base is newer than the last
    compaction (at most 200 updates or 1 MiB of log); after that a stale write gets 412 with `mergeable: false`.
    Agents re-read and retry.
17. **musl and ring.** ring compiles C for `x86_64-unknown-linux-musl` with `musl-gcc`. If that fails in the toolchain
    image, the CLI is built for glibc in the server stage instead (it then needs glibc ≥ the trixie version on the
    host) and the standalone download is postponed; record it here.
18. **Markdown parse memory.** comrak builds an arena AST with a node per container marker; the 1 MiB input cap, the
    nesting pre-scan before comrak and the PUT holding two `doc_work` permits bound it. A proptest feeds adversarial
    input (deep nesting, long emphasis runs) and checks the depth limit.
19. **Open editors see API writes late.** Until the WebSocket (§15), an open editor picks up an API write on focus or
    visibility (`GET ?since=`), not immediately. Both sides' edits are kept (CRDT).
20. **The Markdown-space diff** needs the printer's block map (§6.4 step 3) to stay exact as the printer evolves. If it
    proves fragile, W4 falls back to diffing in JSON space while still copying the attrs Markdown cannot express from
    paired base blocks and leaving hash-equal blocks untouched, and records that here.
21. **Deterministic client ids** (§6.4) collide with the same odds as Yjs's random ids (2⁻³² per pair of clients in one
    document); the re-hash against the base's state vector removes the known cases.
22. **Caddy in the compose project** (D57). The fixed subnets (`172.31.240.0/28` and the script /28s after it) can
    collide with an existing Docker or LAN network (172.31.0.0/16 is also the last of Docker's default bridge pools);
    then `QUIRE_NET_SUBNET`, `QUIRE_NET_RANGE` and `QUIRE_PROXY_IP` change together (the scripts' subnets are
    constants in their files). `no-new-privileges` together with the file capability on the
    image's `/usr/bin/caddy` is unverified: W0a checks that Caddy starts with `cap_drop: ALL` plus `NET_BIND_SERVICE`
    and records any change here. The `(tls_acme)` snippet and the `import tls_{$QUIRE_TLS}` substitution are checked by
    `caddy validate` in both modes (W0a). ACME needs public DNS and ports 80 and 443 reachable; remapped host ports break
    only Caddy's http→https redirect.
23. **The BuildKit daemon is Quire's own container** (`quire-buildkit`, remote driver, D38): buildx no longer manages
    its lifecycle or image, so the image is updated by hand, and a recreate is needed after the slice is installed or
    the caps change (`toolchain.sh` detects both, and also a changed image or `docker/buildkitd.toml`). **W0a pinned
    `moby/buildkit:v0.34.0@sha256:b059f8d7226d0b326bc871af489a5dddf65e36af4d20d14251b072974d9ee87c`**, not v0.34.1:
    v0.34.1 was released on 2026-10-20 and so was under two weeks old on 2026-10-28; v0.34.0 (2026-10-07) is the newest
    older patch. §13.1 and PLAN W0a name v0.34.0 now. To move up (v0.34.1 is two weeks old from 2026-11-03), change
    `BUILDKIT_IMAGE` in `scripts/toolchain.sh` and recreate the builder.
24. **W0b pin checks (2026-10-28)**: what W0b could not confirm as written, and the fallback chosen.
    - Base images (§13.2), each pinned by the index digest its tag resolved to on 2026-10-28:
      `rust:1.99.1-trixie@sha256:53766005847cbe02c3f59bfa49c1c4bb16695f2c9e5829c3d576a15406f5e733` (a Docker rebuild
      of that day; rustc 1.99.1 itself dates from 2026-10-13),
      `node:24-trixie-slim@sha256:89c5244bc22a9bc3287d89ebc7e49b9cb5bb84cd40e6175ac3421d5a9f68b95a` (rebuilt that day;
      node 24.22.1, released about 2026-10-20, kept because the toolchain image already had it; the newest digest at
      least two weeks old carries node 24.21.0) and
      `gcr.io/distroless/cc-debian13:nonroot@sha256:3ee00291cf386ec52a552e6a858d555debd74776eb4bfa53fa3df2cd6677515e`
      (uploaded 2026-10-23). `scripts/e2e.sh` pins
      `mcr.microsoft.com/playwright:v1.64.1-noble@sha256:47b510dd9230bf0b9bfc9b20102241e577ea7a99334ab4c3ca19f19770e9b8ea`
      (2026-10-23), which @playwright/test 1.64.1 must match. All four digests were under two weeks old; if the
      two-week rule is meant for digests too, older digests of the same tags are in the registries' history.
      binaryen `version_131` (2026-07-15) is pinned by its release checksum.
    - rust-embed 8.13.0 is used without `interpolate-folder-path`: that feature pulls option-ext (MPL-2.0, not on the
      licence allow-list). quire-assets uses relative `#[folder = "../../web/dist"]` paths, which rust-embed resolves
      against `CARGO_MANIFEST_DIR`.
    - Risk 12: deadpool-sqlite 0.14.0 and rusqlite_migration 2.6.0 both take rusqlite ^0.40, so the graph has one
      libsqlite3-sys (0.38.2) and the D2 fallback is not needed. The tower-http 0.7.1 layers build and run in the API
      tests. TypeScript 7.0.2 checks both web projects, so there is no 6.x fallback.
      `@tiptap/extension-drag-handle-react` 3.32.0 is MIT. comrak 0.59.0 (`default-features = false`) and ureq 3.5.0
      (`rustls-no-provider`, `json`, `gzip`, with rustls 0.23.46 and ring) build; ureq 3.5.0 needs ureq-proto ^0.6.5,
      published 2026-10-17.
    - Risk 14: utoipa 6.0.0 (utoipa-gen 6.0.1) and utoipa-axum 0.3.0 work with axum 0.8.9, and the served spec parses
      as OpenAPI 3.1. No fallback.
    - Risk 17: ring compiles with `musl-gcc` in the image's `cli` stage, so the static musl CLI needs no fallback.
    - ts-rs 12 (12.0.1) exports `i64` and `u64` fields as `bigint` by default, but the API's JSON numbers are
      JavaScript numbers. `.cargo/config.toml` therefore also sets `[env] TS_RS_LARGE_INT = "number"` (besides the
      `TS_RS_EXPORT_DIR` of §3), and every such DTO field is `number` in `web/src/api/gen`.
    - Pins used as written although they were under two weeks old on 2026-10-28: tokio 1.54.1, uuid 1.29.0, ureq
      3.5.0, comrak 0.59.0, react and react-dom 19.3.1, vite 8.3.5 and vitest 5.0.5. Where §4.1 gives only a minor
      version, the newest release at least two weeks old was taken (crc32fast 1.5.2, not 1.5.3 of 2026-10-20; rustix
      1.1.5, not 1.1.6). Younger transitive crates were moved back with `cargo update --precise`, and the npm lockfile
      was resolved with a 14-day `minimumReleaseAge` apart from those pins.
    - **Open, for the owner:** vite 8.3.5 has lightningcss 1.33.0 and lightningcss-linux-x64-gnu as hard
      dependencies, and both are MPL-2.0, which is not on the licence allow-list. They run only at build time, and
      none of their code ships in `web/dist`. The choices are to accept MPL-2.0 for build-only tools, or to re-pin to
      Vite 7 with @vitejs/plugin-react 5. Still open on 2026-10-28; the pins stay, and `THIRD_PARTY.md` lists both
      packages as pending.
25. **Owner decisions still open** (2026-10-28; nothing here was decided for the owner, and the current pins stay):
    - lightningcss (MPL-2.0, build only; item 24);
    - the age of the image digests (item 24): the four of item 24 were under two weeks old when pinned (node 24.22.1
      about 7 days), and so was Caddy's (v2.11.7 dates from 2026-10-03, but its index digest is Docker's rebuild of
      2026-10-21); if the two-week rule covers digests, older builds of the same tags are in the registries' history
      (node 24.21.0);
    - the fonts the Inkboard wasm embeds (egui's default fonts under the Ubuntu Font Licence 1.0 and SIL OFL 1.1; Noto
      Sans, Gelasio, Patrick Hand and STIX Two Math under SIL OFL 1.1): permissive, but not on the licence allow-list;
      Quire ships them as Inkboard ships them (`THIRD_PARTY.md`);
    - the operating-system layer of the runtime base image (libc6 LGPL-2.1+, the GCC runtime libraries GPL-3+ with the
      runtime exception, the Mozilla CA data of ca-certificates MPL-2.0, among others): the allow-list is a rule for
      Quire's own dependencies, and this layer is listed in `THIRD_PARTY.md` for the owner to see;
    - host port 443 on the owner's machine is held by his own Caddy, so the production project cannot bind it as is.
      The README describes `QUIRE_HTTPS_PORT`/`QUIRE_HTTP_PORT` and running behind that Caddy with
      `QUIRE_TRUSTED_PROXIES`; which one to use is his choice;
    - the copyright line of `LICENSE-MIT` ("The Quire contributors"; the owner may want his name, W0b D39);
    - ~~the build slice is not installed yet (item 13)~~: installed 2026-10-28 after the M1 gate, and the builder
      recreated in it (`docs/handoff/W0a.md`, task 8); every build before that ran with the per-container caps only.
26. **Checks that need a real browser** (happy-dom does no layout and fires no native drag events). The W7 gate's
    Playwright smoke (`e2e/tests/smoke.spec.ts`, real engine, SwiftShader WebGL 2) now covers: the board host with the
    real dist (the frame page, `manifest.json` and the engine from `/inkboard/{build}/`, the iframe shown over the
    surface once the board is open, a rectangle drawn and saved, the reload, two pages and the conflict banner); the
    item menu and the Nested popover over the canvas (the topmost element where they overlap the iframe); the slash
    menu and the board picker; and no CSP violation on any page. Still manual: fullscreen (Focus), double-Esc out of the
    canvas, a background tab (headless Chromium reports a background page as visible, so W5's frame test skips that
    case on purpose), the editor's bubble menu, block handle and popover placement, one real drag and drop in the
    sidebar (the smoke moves through the Move dialog), and the About page.
27. **Integration found at the merge.** `docs/handoff/merge.md` ("Integration problems the checks do not catch") lists
    eleven cross-owner gaps that the per-workstream tests could not see: the engine's `manifest.json` answered 404, the
    board warm-up and the update banner were not wired across W3 and W6, Full width and the new-item discard were
    inert, the seed did not use W4's seed helpers, `ItemDetail.board` came from W1's own meta, page SVGs were gzipped
    twice, `quire item move` without `--parent` sent `parent_id: null`, agent scenario 04 resent an unchanged board as
    its stale write, and the client's title order differed from the server's. All eleven are fixed on main by W7's
    code commits (`ws/W7-fixes`, each with a test; `docs/handoff/merge.md` "W7 merge"), and this document states the
    contract each is fixed to (§5, §6.2, §7.2, §7.6, §9.5, §9.6.2, §9.6.4, §9.6.7, §11, §17.4). W4's request to build
    `oas3` without `preserve-order` is done too (§4.1). Confirmed in the real image by the W7 gate
    (`docs/handoff/W7.md`): the manifest by `QUIRE_SMOKE_LEVEL=full scripts/image-smoke.sh` and by the Playwright
    smoke, and agent scenario 04 by `scripts/agent-test.sh`. The gate found three more, fixed with tests: a finished
    setup sent the new owner to the sign-in page (§9.1), TipTap's inline base CSS was refused by the SPA CSP (§11),
    and agent scenario 03 compared the Markdown goldens with a server that had none of their items (PLAN W8, scenario
    table). The security and product reviews after the gate found ten more, each fixed with a test
    (`docs/handoff/W7.md`, "Review findings"): the Markdown GET lacked its sandbox CSP and `no-store` (§11), the scheme
    allow-list let `/\t/host` through (§6.4), document links could not be followed (§9.6.5), Recently viewed never
    filled (§9.6.6), menus near the bottom of the window hung out of it (§9.6.2), the seeded board's embed card had
    no preview (§9.6.5), new empty boards were never discarded (§9.6.4), opening an empty document wrote to it (§9.4),
    a `--json` usage error dropped the argument names (§7.7), and the Welcome text said archived pages stay
    searchable (§9.6.7). The smoke's step 16 checks four of them in the real browser, and found one more on its first
    run: a streamed board body (a preview rendered on demand, any board body over 1 MiB) panicked the connection task
    when the compression layer gzipped it, because `futures::stream::unfold` was polled after its end; the streams of
    the board route and the export are fused now (§11).
    Still open: the 545 kB main web chunk (W3; Vite warns, a `manualChunks` split is optional).
28. **Smaller follow-ups from the notes.** The per-IP limiters key IPv6 clients by the full address, so one client with a
    /64 can rotate addresses (§8 says "per IP"; keying IPv6 by /64 would close it). Two readers of large boards can hold
    the whole body budget for up to 600 s each, and a third large GET then gets 429 `busy` (small boards are not
    affected). Inkboard's `Page::connector_path` looks objects up linearly, so a page with many connectors renders in
    quadratic time under `board_work` (upstream, §15). If a client trickles a board PUT, `board_work` is held while the
    body arrives, bounded by the 120 s deadline and the per-token byte limit. Board-page SVGs are now gzipped by the
    compression layer on an async worker (up to `PAGE_SVG_MAX`, 16 MiB, at the fastest level) instead of on a blocking
    thread; if that shows in latency on the 2-CPU limit, move the page's gzip back to `spawn_blocking` (§17.4).
29. **Licence records.** `THIRD_PARTY.md` is kept by hand: a change to `Cargo.lock` or `web/pnpm-lock.yaml` must update
    its tables (it says how), while the image's `/licenses.txt` is regenerated by every build. The standalone CLI
    (`dist/cli`) ships `THIRD_PARTY.md` and `LICENSE-*`, which name every crate with its version, licence and source,
    but not the crates' own licence texts; a CLI-only `licenses.txt` in the `cli-dist` stage would add them.
30. **Process note (W5 D18).** During W5 one `node --check e2e/serve-static.mjs` ran on the host, a syntax check that
    left nothing behind, against PLAN R1. It was reported, reviewed and not repeated; W7 acknowledges it here.

---

## 17. Decisions log (review findings not fixed as proposed)

Every other finding of the feasibility (F) and product (P) reviews of 2026-10-27 was fixed in place.

| Finding | Resolution |
|---|---|
| F2 / P11: patch 0002 edits files excluded from the Docker context | Fixed by keeping `vendor/inkboard/docs` in the context (2.7 MB) and moving the docs hunks into `0004-docs-embedding.patch`, not by skipping hunks with `git apply --exclude`. |
| F3 / P4: toolchain pin | Pinned to `1.99.1` with components and target, rather than dropping `rust-toolchain.toml`: the file keeps native and Docker builds on the same compiler; `RUSTUP_AUTO_INSTALL=0` turns a mismatch into an error instead of a download. |
| F5 / P10: git in worktrees | Fixed by making `inkboard.sh src` a self-contained copy + `git init` + `git am` inside `/work`, rather than mounting the git common dir into the container. |
| F8: F5 / Mod+R reload the whole tab | Rejected: reload is the browser's normal action on every Quire page, and `beforeunload` plus autosave protect the board. The Present menu item is hidden, `run_action` ignores Present when hosted, and the `keymap.rs` change is dropped. |
| F16: optional icon URL in `MountOptions` | Rejected for M1: Quire serves `icons.bin` next to `frame.html`; the limitation is documented in patch 0004 for upstream. Single-mount guard and the panic backup are done. |
| P5: hard total cap | Done as a systemd slice. It needs one root step on the owner's machine; the owner said yes on 2026-10-28 (§17.3), and the coordinator runs it. The fixed lock path, `QUIRE_LOCK_HELD`, `--cpuset-cpus` and the 4-CPU Playwright cap apply regardless. |
| P6: drafts and "published ⇒ ancestors published" | Replaced the enforced invariant with derived effective visibility (D23), so move, restore and unarchive need no rule and unpublish never fails. Draft-first creation was superseded on 2026-10-28: items are published on create and the draft state stays in the model and API only (§17.3); its multi-user meaning is written down in D23. |
| P8: CHECK lists | Lookup tables for `revisions.format`, `revisions.reason` and `item_links.kind`. `collections.child_order` and `users.theme` keep CHECKs: small tables, cheap to rebuild if ever needed. |
| P9: background-tab panic | Mitigated with the visibility wait and a frame test rather than backporting egui #8313 through `[patch.crates-io]`, which would fork eframe 0.33. |
| P12: å/ä/ö folded by `remove_diacritics 2` | Kept for M1: accent-insensitive search suits mixed-language notes, and the FTS index is derived, so switching to `remove_diacritics 0` or adding an exact column later is `INSERT INTO item_fts(item_fts) VALUES('rebuild')`, not a data migration. The FTS syntax errors are fixed. |
| P15: bounded memory for doc reads | `doc_work` permits instead of an in-memory doc LRU; the LRU arrives as the WebSocket room cache (§15). |
| P18: per-viewer ETag | Added as `viewer_epoch`, constant 0 in M1. |
| P21: per-email limiter can lock the owner out | Partly: it now counts failures only. The remaining lockout risk is accepted: it needs an attacker who can reach the login page, lasts at most 15 minutes, and existing sessions keep working; a 12-character minimum plus argon2id makes guessing impractical. |
| P22: remote images under `img-src 'self'` | CSP not widened: remote images become placeholders with their link, so reading a page never contacts third parties (D29 spirit). |
| F12: fixture determinism | Seeded UniqueID ids and fixed client ids; the TableOfContents extension is removed instead of seeding it, because it stores a second random id per heading in the Y document. |
| F7: cross-workstream tests | Tests that need another workstream's full behaviour moved to `tests/api/integration.rs` (W7); W0b ships minimal working versions of the shared store functions. |
| F28: caps land late | W0 is split: W0a (caps only, own commit and gate) lands before the scaffold (W0b). |

### 17.1 Owner requirements of 2026-10-27: public API (A), CLI (B), agent testing (C)

The owner asked for "an API that can serve external requests, and a CLI so we can have agents testing the tooling".
Each decision below is binding and has a row in §2.1; this table records what it answers and what was rejected.

| Req. | Decision | Rejected alternatives and why |
|---|---|---|
| A | D39: the existing `/api` routes became the public API `/api/v1`; the web client uses it with the cookie, external clients with tokens; additive-only within v1 | A second API for external clients (would lag the UI and double the tests); unversioned routes (a breaking change would be a flag day) |
| A | D40: DTOs moved to `quire-api`, re-exported as `quire-core::api` | Keeping them in quire-core (the CLI would compile yrs, inkboard-core and comrak); generating client types from the OpenAPI spec (two type systems for one contract) |
| A | D41: utoipa 6.0.0 + utoipa-axum 0.3.0, spec committed with a drift test, no docs UI | aide 0.15.1 (schemars 0.9, extractor impls; 0.16 alpha); a hand-written spec (drifts); Swagger UI / Scalar (build-time download or CDN) |
| A | D42: `qpat_` tokens, sha256 storage, 12-character prefix, CRC32 checksum, expanded scopes, 90-day default expiry, last-use throttled to 1/min, 100 per user | JWT (not revocable without state); argon2 (slow for every request, pointless for 238-bit secrets); one scope column (finer scopes later need a migration); no expiry by default |
| A | D43: a Quire bearer token (`is_quire_bearer`) wins over the cookie; other `Authorization` values are ignored; CSRF checks for every non-token mutation; session-only routes | CSRF for every request (blocks non-browser clients that send `Origin`); falling back to the cookie when a token is bad (confusing, and masks a revoked token) |
| A | D44: `Auth<S>` as the first extractor + one generic test over the spec | A middleware table of path → scope (a second place that can drift); per-route tests |
| A | D45: ETag/If-Match/412 for boards, documents and item metadata; `*` allowed; `meta_version` body field replaced | 409 for document conflicts (inconsistent with boards); allowing content writes without a precondition |
| A | D46: opaque keyset cursors bound to the query; one `Page<T>` shape; search renamed `results` → `items` | Offset pagination everywhere (unstable under inserts); signed cursors (nothing secret in them) |
| A | D47: per-token governor limit (600/min, burst 120) and a per-IP limit on failed bearer auth; `Retry-After` only | Draft `RateLimit-*` headers (the IETF draft changed shape); limiting cookie sessions (the UI's own bursts) |
| A | D48: CORS off; `QUIRE_CORS_ORIGINS` exact list, never credentials | `*` (any page could use a token pasted into it); allowing cookies cross-origin |
| A | D33 updated: Markdown PUT 1 MiB, JSON PUT 4 MiB, depth 100, other bodies 1 MiB | One 16 MiB limit for every content format (comrak's arena and a JSON value tree are many times the input under the container's memory cap) |
| A | D49: Markdown and ProseMirror JSON converted in Rust; comrak 0.59 parser, Quire's printer; web Markdown export via the API | pulldown-cmark (no `==highlight==`); comrak's formatter (output churn across upgrades); client-side prosemirror-markdown (a second dialect) |
| A | D50: external writes as diffs against the If-Match base, saved as ordinary Yjs updates; opt-in CRDT merge for stale bases | Replacing the fragment (destroys Y identity and block ids, duplicates concurrent edits); always merging (surprising interleavings without the client asking) |
| A | D51: boards over the API through the web pipeline; `X-Quire-Engine` optional | A separate API board format or a typed re-serialization (would drop unknown fields, D9) |
| A | D52: server-side board import deferred (inkboard-formats adds 104 crates; the importers alone need 13) | Linking inkboard-formats now (two resvg stacks, svg2pdf, image codecs under the build and runtime caps); patching it locally (Quire compiles Inkboard unpatched, §4.1); import in the CLI (the same weight in every CLI binary) |
| A | D53: `pre-api` revision before a token's content write unless the same token wrote last or the newest revision holds the current state | No extra revision (an agent's mistake would be unrecoverable once history ships); a revision on every write (log growth); "newest revision older than 10 min" (up to 9 minutes of the owner's work in no revision) |
| A | D34 updated: exports delete API tokens | Keeping token hashes in exports (an export is meant to carry no credentials) |
| B | D54: crate `quire-cli`, clap 4.6, ureq 3.5 + rustls/ring, etcetera, toml; `--json` everywhere; exit codes as a contract; static musl binary in the image and standalone | reqwest 0.13 (tokio, hyper, aws-lc-rs by default, tower-http 0.6 duplicate); native-tls/OpenSSL (excluded by the requirement); a glibc-only binary (tied to the build distro) |
| B | §7.7: `--json` stdout is the API body unchanged; errors as the JSON envelope on stderr; 13 exit codes | Errors on stdout (breaks `cmd \| jq` pipelines); a single non-zero code (agents could not tell "retry after re-read" from "fix your input") |
| B | §7.6: content writes need `--if-match` or `--force`; destructive commands need `--yes`; profile file refused when group-readable | A silent GET-then-PUT (hides lost updates); prompts (agents hang) |
| B | D55, D1 updated: binaries `quire-server` and `quire`; admin commands in the server binary; `quire admin` passthrough | Admin over HTTP (needs a token to create the first token); a third binary; one binary with both (the server would link TLS, the CLI rusqlite) |
| B | §7.8: `create-owner` and `create-token` added to the admin commands | Provisioning through the setup code over HTTP in scripts (works, but needs a password and a cookie dance) |
| C | D56, §14.1: `scripts/agent-test.sh` on the production image with `compose.e2e.yaml`, the build lock and the slice (when installed); scenarios in bash + jq; CLI run inside the container by default | A Rust scenario runner (agents edit bash scenarios more easily); running the CLI from the toolchain container (tests a dev build, not the shipped one); publishing a port by default (collisions; loopback only with `--keep`) |
| C | D56: Rust CLI tests with assert_cmd against `quire_server::testing::TestServer` (feature `testing`), plus a mock listener for transport cases | Testing against the HTTP API only (misses argv, exit codes and output); a mock-only suite (misses real server behaviour) |
| C | `docs/AGENTS.md` (W8) as the agent-facing guide, with no secrets | Putting agent instructions in CLAUDE.md (that file is about building the project, not using it) |

### 17.2 Review of the amendments (2026-10-27): consistency (C) and security / agent UX (S)

Every finding of the consistency review (C1–C21) and the security and agent-usability review (S1–S35) was fixed in
place (§2.1, §5, §6, §7, §8, §12–§16, PLAN W0b–W8), except where this table says otherwise.

| Finding | Resolution |
|---|---|
| C4: derive the client id from the request, or never retry a PUT after sending it; treat a 404 on a retried DELETE as success | Both of the first two: the server derives the client id and new `blockId`s from the request (§6.4), and the CLI retries only requests the server provably did not process (§7.6). The 404 rule is dropped: a retried DELETE was never processed, so its 404 is real. |
| C7 / S6: `Bearer` without `qpat_` gets 401 (C7), or is ignored (S6) | S6: every `Authorization` value except a Quire token is ignored and the request takes the cookie path with the CSRF checks, because oauth2-proxy and similar proxies forward `Bearer <JWT>`. |
| C9: move the CLI tests to W7's gate, or have W0b ship minimal routes | Minimal routes: W0b ships `GET /auth/me`, the four `/tokens` routes and `POST /collections`, so W8's parallel-phase gate is real. The 415 test moves to W4. Scenario 01 revokes over HTTP, so `admin revoke-token` stays with W2. |
| C11: allow CDLA-Permissive-2.0 for webpki-roots, or use system roots | System CA bundle (D54): no change to the licence policy is needed. |
| C14: no 304, or a composite ETag | No 304 on `GET /items/{id}` and the Markdown GET: the tag is also the `If-Match` value for writes, which a composite tag would break. |
| C15 / S9: refuse `--keep` without the slice, or a TTL lock holder | A TTL lock holder (default 15 min, PLAN R1) for `up` and `run --keep`. It works with and without the slice. |
| C20: add `scripts/inkboard.sh cli`, or state it is unavailable | Unavailable in M1 (D52, §7.5): `ink` links the 104 crates that D52 defers, and W5 already carries the riskiest work. Listed in §15. |
| S8: a busy export as 409 or 429; an exit code for 507 | 429 `rate_limited` with `reason: "busy"` and `Retry-After`, so the CLI's retry handles it. 507 maps to exit 12, since the client cannot fix it. |
| S12: `Idempotency-Key` on POST creates | Deferred as an additive v1 change (§15). M1 reports `details.sent`, and AGENTS.md says to list before retrying a create. A key store needs an in-flight state for concurrent duplicates, for a case (a reset after sending, on a local link) the harness does not hit. |
| S19: copy fixtures into the container, `docker cp` the outputs | Not possible: `docker cp` cannot reach tmpfs mounts and the distroless image has no shell. Fixtures go in on stdin, binary output comes back on stdout (`q_raw`), and `--out` targets the container's `/tmp` for metadata checks only. |
| S22: write `pre-api` unless the newest revision equals the current state | Refined with `last_token_id`: the proposed rule alone would write a revision before every consecutive write of the same agent. |
| S32: re-enter the password for session-created admin or never-expiring tokens | Done, but a missing or wrong password is 422 `validation` (`field: "password"`), not 401, so the web client's sign-in dialog does not open. |
| S34: make export a POST, or check fetch metadata | Fetch metadata, with a same-origin `Referer` fallback over plain HTTP. Downloading a POST response in a browser needs a form post, which the 415 rule forbids, or a blob held in memory. |

### 17.3 Owner decisions of 2026-10-28

The owner settled four open defaults and accepted two more without comment. Each replaces the earlier default
everywhere in this document and in PLAN; the binding rows are in §2.1.

| Decision | What it changed | Rejected alternatives and why |
|---|---|---|
| 1. Drafts: published on create (D23) | `POST /items` sets `published_at = created_at`, for the web client and the CLI (`item create`) alike. The draft state stays in the schema and the API (`published_at IS NULL`, publish with `with_ancestors`, unpublish, `GET /drafts`, `drafts_count`, `include_drafts`, the CLI's `item publish`/`unpublish` and `list drafts`), so a Drafts workflow can be switched on with multi-user. The web client has no draft UI at all: no Publish button or step, no Draft pill or " · Published", no Drafts entry, badge or page, no Mod+Shift+P, no "include drafts" filter. An item unpublished through the API or CLI looks like any other item in the UI. `?discard_empty=true` no longer needs a draft: it deletes a new (< 24 h), childless, untitled, empty item. The seed and the guide text no longer mention publishing; tests, the smoke spec and agent scenario 06 assert published-on-create | A Drafts entry shown only while `drafts_count > 0`: a page the owner could not act on without the CLI, and one more UI state to build and test. Removing the draft columns: multi-user would need a migration and a new API. A create flag `draft: true`: nothing in M1 needs it, and adding it later is additive |
| 2. App limits: 2 CPUs / 1 GiB / no swap (D25) | `QUIRE_CPUS=2`, `QUIRE_MEM=1g` (overridable). The D25 budget was re-derived: `doc_work` 3 permits (a PUT takes 2, so a PUT and a read run together), the board limit stays 50 MB, export and backup buffers bounded at 16 MiB, the `/tmp` tmpfs counted; about 250 MiB of headroom. Tokio keeps 2 workers (now one per CPU). The image smoke checks NanoCpus=2000000000 and 1073741824 bytes. In e2e runs the app (2) and Playwright (4, unchanged) make exactly the 6-CPU total; Caddy is not started there | Raising the board limit to 64 MB (it would eat the headroom the third doc permit needs, and boards are the rarer large writes). Four doc permits (≥ 640 MiB of doc work). Lowering Playwright to 3 CPUs (2 + 4 already fits, and SwiftShader WebGL is CPU-bound) |
| 3. Build slice: installed (D38) | `quire-build.slice` (`CPUQuota=600%`, `MemoryMax=12G`, `MemorySwapMax=0`, `CPUWeight=20`, `IOWeight=20`) and its parent `quire.slice` (the weights, which only work there), installed by `scripts/install-build-slice.sh`, one root step that the coordinator runs through the owner's askpass dialog; implementing agents never run `sudo`. `--check` verifies it. `scripts/lib/slice.sh` detects it by its unit file (`FragmentPath`: systemd reports any slice name as `loaded`) and passes the slice as cgroup parent (systemd driver) to `dev.sh`, the BuildKit container and the e2e, agent and smoke containers, and warns and continues when it is absent. The builder became a `remote`-driver registration of a BuildKit container that `toolchain.sh` starts itself | buildx's `cgroup-parent` driver option (ignored under the systemd cgroup driver: buildx `driver/docker-container/driver.go` and its docs, checked 2026-10-28). A daemon-wide `cgroup-parent` in `daemon.json` (would also capture the hosted app and Caddy). Switching Docker to cgroupfs (outside systemd's accounting). Weights on `quire-build.slice` alone (no siblings, so no effect) |
| 4. HTTPS through Caddy (D57) | `caddy:2.11.7` (pinned by digest) in `compose.yaml` with `docker/Caddyfile`, its own caps (0.5 CPU, 128 MiB, no swap) and `/data` and `/config` volumes; Quire has no published port. `QUIRE_TLS=acme` with a public `QUIRE_DOMAIN`, or `internal` (default, `localhost` or a LAN name) with the local CA trusted by hand (§13.4). A fixed Caddy address is the only `QUIRE_TRUSTED_PROXIES`, so `X-Forwarded-For` and `-Proto` count only from Caddy (rate limits, Secure cookie). The session cookie follows the request's scheme. `QUIRE_DOMAIN` joins the Host allowlist; the public URL is derived from it. HSTS from Quire, HTTPS requests in `acme` mode only. No `encode`, `compression off`: precompressed `.wasm`/`.js` pass through byte for byte (checked by the image smoke). CSP unchanged. The e2e and agent harnesses talk to Quire directly on the project network (HTTP); the image smoke covers the HTTPS path (a curl through Caddy with the local root answers 200) | TLS in Quire (a TLS stack and ACME in the server, against D29). Traefik (label-driven configuration, more to pin and review). README advice only (the owner asked for it in the deployment). Trusting forwarded headers from the whole subnet (any container on it could spoof a client IP or HTTPS), a shared secret (one more secret to provision), a DNS lookup of `caddy` (an outbound call, and stale after a restart). HSTS from Caddy (security headers in two places) or in `internal` mode (pins `localhost` and LAN names for a year). e2e through Caddy (0.5 CPU over the 6-CPU total with Playwright, and CA plumbing in Playwright and the CLI) |
| Accepted default: CLI trust roots (D54) | The CLI verifies TLS against the system CA bundle, or `--ca-file` for a private CA such as Caddy's local root; it never bundles webpki-roots and has no flag that skips verification | Bundled webpki-roots (CDLA-Permissive-2.0, not on the licence allow-list, and blind to a locally trusted root) |
| Accepted default: agent harness lock TTL (D56, §14.1) | `scripts/agent-test.sh up` and `run --keep` keep the app while a background holder holds the build lock for `--ttl` minutes, 15 by default (PLAN R1), then run `down` | An unbounded hold (blocks every build); refusing `--keep` without the slice (the TTL works with and without it) |

### 17.4 The deviations of W0a–W8, folded in (W7, 2026-10-28)

The workstreams recorded every deviation and open choice in their handoff notes (`docs/handoff/W0a.md` … `W8.md`), and
the merge recorded the integration gaps (`docs/handoff/merge.md`). They are now written into the sections they amend.
Where the notes conflicted, or a note left the choice to the docs, this table records what was decided; nobody was
asked, and the owner's open decisions (§16 items 24 and 25) were left open.

| Question (source) | Decision | Rejected alternatives and why |
|---|---|---|
| Headers of `/inkboard/{build}/manifest.json` (merge item 1; W5 requests W2-1, W7-2) | Revised at the W7 merge to match the code: served for the embedded build only, `no-cache` with its ETag, identity only, `application/json`, `nosniff`; no CSP or `X-Frame-Options`, as it is data that `frame.js` fetches (§11). `frame.js` cannot connect without it | Immutable, the first wording of this row (`{build}` hashes the dist's other files but not the manifest's `patchset`, so one build id can come with another manifest and a cached copy would go stale); keeping it private (every board in the real image failed to connect) |
| Board-page SVG gzipped by both the route and the compression layer (merge item 8; W2 D10, W6-D29) | Revised at the W7 merge to match the code: the compression layer alone, at the fastest level and only over 1 KiB, the rule every other dynamic body follows; the route sends identity and `Vary: Accept-Encoding` (§11). W2 withdrew its request for the route to gzip, so the layer is the one that stays | The route's own gzip on a blocking thread, the first wording of this row (a second compression rule, which also gzipped pages under 1 KiB). Its argument, that the layer compresses up to 16 MiB of SVG on an async worker, is kept as a follow-up (§16 item 28) |
| `ItemDetail.board` from W1's or W6's meta (merge item 7; W1 D17, W6 D12) | W6's `boards::board_meta`: `preview_rev` is the newest stored rev, `svg_len` that rev's (§5 "Previews") | W1's oldest rev (it points clients at a render older than the one the page route serves) |
| Title order of the tree (merge item 11; W3-D1, W1 D10) | Revised at the W7 merge to match the code: one title key defined twice, `quire_core::tree::title_key` and the web's `titleKey` (accent- and case-insensitive, numbers by value), both checked against `fixtures/tree/title-order.json`; ties go to `sort_key`, then the id, for the title, updated and created orders (§5, §9.6.2) | Keeping two orders, the first wording of this row (an optimistic reorder in the browser could disagree with the server until the next fetch); `Intl.Collator` on the client (its order depends on the browser's ICU data, which the server cannot reproduce) |
| Full width across reloads (merge item 4; W3 request W1-2) | `ItemDetail.full_width`, an additive field (R3, §7.1). Revised at the W7 merge to match the code: the document page reads `ItemDetail.full_width` directly and sets `data-full-width`; the PATCH updates the cached detail at once and rolls it back on failure (§9.6.4) | Session-only state (the column exists and the PATCH writes it, but nothing could show it after a reload); a per-session store next to the server value (`useFullWidthOf`, the first wording of this row: two sources for one setting) |
| Sort directions, drop details and toasts the UI spec left open (W3-D1, D2, D4, D5; W3 request W7-1) | Adopted as implemented (§9.6.2, §9.6.3) | Leaving them unwritten (the next owner of the sidebar would have to read the code to keep them) |
| What the update check compares (W3-D6) | `version`, `commit` and `inkboard.base`, against the first `/config` the page loaded (§9.1) | `version` alone (dev images keep 0.1.0 between builds) |
| `doc put --json F` (D-W8-2) | The flag is `--json-input F`, and `--json F` after the item is rewritten to it, so §7.6's spelling keeps working (§7.6) | A different public spelling (breaks the contract of §7.6); a second global flag name (clap cannot hold both ids) |
| Operations without a CLI command (D-W8-17) | `GET /openapi.json` and `GET /items/{id}/board.inkboard` join the allow-list (§7.6) | Commands for them (`api` reads the spec; `board get` returns the same envelope) |
| Expiry of the harness tokens (D-W8-18) | `--expires 2d` (§14.1), so a scenario's 1d child token fits under its parent | 1d (every scenario `token create … --expires 1d` would be refused as outliving its parent) |
| Board GET and `If-None-Match` (W6-D24) | Never 304, like the Markdown GET (D45, §7.1, §6.2) | 304 on `"r{rev}"` (a rename changes `document.name` without a new rev) |
| Memory of previews and large board responses (W6-D20, D22, D23) | Per-page and per-board preview budgets, omitted pages, and a 96 MiB body budget taken before `board_work`: the one documented exception to "no permit while holding one", deadlock-free by its fixed order (D25, §5, §12) | Unbounded renders (an image inlined once per object could exhaust 1 GiB); response bodies outside every permit |
| Markdown nesting check (W4 D11b) | A per-line upper bound before comrak (200), then the exact 100 on comrak's tree (§6.4 step 1) | The single line pre-scan of the first wording (it let about 1000 levels through to the recursive conversion) |
| When a document PUT takes its permits (W4 D14a) | Before the body is read (D25, §6.4) | After the read (each waiting request could buffer 16 MiB, so the permits no longer bound memory) |
| Deadlines of export and backup (W2 D9) | None; both are single-flight and hold their flag while the copy runs (§8, §12) | The 30 s deadline (exports of a large database would be impossible over HTTP, and retries stacked copies) |
| The public URL of a bare serve; the healthcheck address (W0b D25, D26) | `http://localhost:<QUIRE_LISTEN port>`; the listen address itself unless it is unspecified (§8, §7.8) | A fixed `http://localhost:8080` and a probe on 127.0.0.1 always (wrong when the port changes, or when the server binds one specific address) |
| The `/data` tmpfs of the e2e and smoke overlays (W0b D1) | `mode: 0o1777` in both (§13.3) | The §13.3 text without a mode (uid 65532 could not write), `mode: 1777` (compose reads it as decimal) |
| BuildKit pin (W0a, §16 item 23) | v0.34.0 in §13.1 and PLAN W0a, as pinned | v0.34.1 (under two weeks old on 2026-10-28) |
| The licence list of Settings → About (PLAN W7) | A `licenses` stage in `docker/Dockerfile` writes `/licenses.txt` from `cargo metadata` and `pnpm licenses list`, with every package's licence files, each distinct text once; About shows it on demand and links the engine's own lists (§13.2, §9.6.6). `THIRD_PARTY.md` is the hand-kept record, with the commands that regenerate its tables | Generating `THIRD_PARTY.md` in the build (a build cannot update the source tree, and the record should be reviewed); names and licences without the texts (MIT and BSD ask for their notices to travel with the binaries); a script under `scripts/` (the stage stays self-contained as a heredoc) |
| Quire behind the owner's own proxy, which holds port 443 (owner's machine; W0a "Environment findings") | README: a `compose.override.yaml` that publishes Quire on a loopback port, keeps the bundled Caddy off through a compose profile, and trusts the project network's `ip_range`, checked against the logged `client_ip`; or `QUIRE_HTTPS_PORT`/`QUIRE_HTTP_PORT` with the bundled Caddy | Trusting `127.0.0.1` or the network gateway (checked on this host with Docker 29: connections through a loopback-published port reach Quire from an address inside the project's `ip_range`); reusing `QUIRE_PROXY_IP` (it also moves the bundled Caddy's address) |
| W5 D18, one host-side `node --check` | Acknowledged (§16 item 30) | — |
| The smoke's step 4, "select all body text and delete it", while step 15 expects the heading "Plan" from step 3 (PLAN W7 smoke spec) | Decided at the W7 gate: the body is the paragraph under the heading, so step 4 deletes the paragraph's text and keeps the heading; step 15 then finds "Plan" | Deleting the heading too (Ctrl+A): step 15 could not pass, since step 4 types only "quokka" again |
| How the smoke shows that the reloaded board still holds its object (PLAN W7 smoke step 11) | The server's board has one object after the reload, and the next save from the reloaded engine (a second rectangle) holds two: the engine had the first | Reading the engine's state from the iframe (the frame API belongs to the host and has no global); comparing canvas pixels (fragile under SwiftShader) |
| The smoke's `GET /api/v1/export` with the session (PLAN W7 smoke step 13) | Fetched by the page itself, with the browser's fetch metadata; the bare GET without it is checked to be `403 csrf_rejected` (§8: a cookie export needs `Sec-Fetch-Site` or a same-origin `Referer`) | Adding `Sec-Fetch-Site` by hand to Playwright's request context: it would check a header the test wrote, not the browser |
| Agent scenario 03 and the Markdown goldens' fixed ids (`fixtures/markdown/*`, made against `GoldenLookup`) | The scenario creates "Doc A", "Doc B", "Board C" and an attachment, and maps every golden id, url_id and the origin to the live ones in the input and the expected Markdown before comparing exactly (PLAN W8, scenario table) | Comparing only lookup-free goldens (it would skip links, mentions, embeds and images); regenerating the goldens per server (they are W4's fixed contract) |
| `quire item move` without `--parent` or `--top` (merge item 9; W1 D23) | Decided at the W7 merge (W7 fixes): the item keeps its parent, so a position alone reorders; the CLI reads the item and sends its current `parent_id`, and `--collection` needs `--parent` or `--top` (§7.6) | Sending `parent_id: null` (a reorder moved the item to the top level, or got 422 when the position named a sibling); making one of the flags required (no way to reorder without restating the parent) |
| Token hashes left in an export's free page space (found by `integration::export_after_api_writes_has_no_tokens_or_sessions`) | Decided at the W7 merge (W7 integration tests): a second `VACUUM INTO` from the scrubbed copy writes the exported file, so every byte comes from the remaining rows; `/data/tmp` briefly holds two copies (D34, §12) | Deleting the stat rows only (older copies of them stayed in the unused space of the first copy's pages) |
| How a document's links are followed (W7 product review: links could not be followed, item links had the DOM `href` `quire://item/…`) | Decided at the W7 review: item links render `href="/item/<uuid>"`, a new redirect route (§9.1), while the stored mark keeps its href; a click follows a link when the document is read-only, Ctrl/Cmd+click while it is editable; item and same-origin links go through the router, others open in a new tab (§9.6.5) | A plain click that follows while editing (link text could no longer be clicked into and edited); a mark view that resolves each item's `/doc/<slug>-<url_id>` (a query per link and a React mark view inside ProseMirror, for a path that changes on every rename); `/doc/<uuid>` (the doc page would remount once the canonical replace changes the query key) |
| Opening an empty document wrote to it (W7 product review: `content_at` set, "You updated", a "Leave site?" prompt) | Decided at the W7 review: the doc session holds the editor's own updates while the document is blank and was blank when it loaded, and queues them ahead of the first update that gives it content (§9.4) | Seeding an empty paragraph when the server creates a document (every API-created document and the Markdown and PM JSON writers would gain a paragraph to account for; the server's `docs::is_empty` and the discard rule would change) |
| Board embed card of a board without a stored preview (W7 product review: the Welcome document's example board showed an empty card) | Decided at the W7 review: the card requests page 0 whenever the board has a page, and the page route renders a missing preview on demand (§9.6.5, §12) | Enqueueing previews for seeded and duplicated boards (still leaves the card empty until the job runs, and adds a write path to the seed) |
| Discarding a new empty board (W7 product review: only documents were discarded) | The board page follows §9.6.4 like the document page: a board this page session created and left untitled and empty is deleted with `?discard_empty=true` after the save loop and the title flush; the server decides. The title input's pending entry now stays registered until its last PATCH ends, so the discard never overtakes it | A client-side emptiness check of the canvas (the engine is in the frame; the server's `boards::is_empty` is the rule) |
