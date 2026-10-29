# R: the rename to Wikanvas: rename, regeneration, review and gate (2026-10-29)

## Status

The owner asked to rename every use of the project's old name, source code included, with no legacy alias, because
of existing trademarks on that name. The decision, its reasons and the list of what changed are row D100 of
ARCHITECTURE §2.1; README's NOTE (line 13) says it to readers. The owner also chose a new mark for the new name
(`cf549ae`, a W drawn as one rounded stroke replaces the Q of the favicon).

**The rename is done on branch `ws/R-rename`, every generated file is rebuilt from it, and the M1 release gate passes
at `bcdd931`, every line but the slice check, which is owner-pending** (it needs the owner's root step, and the owner
cannot approve the askpass dialog at the moment). Three independent reviews then reported 18 findings: eleven held
(findings 1 and 5 are the same defect) and seven were rejected (six of them already handled by the runbook the
rehearsal had corrected). None of them is in the code: the fixes are in the coordinator's cut-over runbook, and the
one repo fix is `42ce845` (README), after which the image was rebuilt and its checks passed again. The migration of
the live wiki was rehearsed end to end on a copy of its database with no problem left.

Nothing was skipped, ignored or loosened: no `#[ignore]`, no skipped test, no `--allow-stale`, no check weakened. The
one skip in the frame suite is W5's designed "background page for 60 s" case (headless Chromium reports a background
page as visible; ARCHITECTURE §16), as in every run since W5. Nothing ran as root, nothing was pushed, no commit was
amended (R1's squash of its own first commits is described below), `git stash` was not used, and `vendor/inkboard` is
clean. No new dependency.

**The owner's live wiki was never touched.** Until the cut-over it still runs under the old names: its compose project,
two containers, network and three volumes, and the `.env` of the main checkout, on the rollback image of commit
`95b452a` (`33ea96a36093`, patchset 2). The old builder, its container and state volume, the old toolchain tag and the
old images are as they were. The renamed docs describe the state after the cut-over (D100), which has not happened
yet. Every build, server, browser and compose project of this stage ran in the scripts' own `wikanvas-*` projects or
in rehearsal-only names, and was removed afterwards; no host port 80, 443 or 8443 was used.

This note is committed after the gate, so the gate's image `wikanvas:42ce845` is one commit behind the branch tip.
`scripts/image.sh` is run again on the commit that adds the note (only `docs/handoff/` changes, which `.dockerignore`
keeps out of the image), so that `wikanvas:latest` carries the branch tip's revision, and
`WIKANVAS_SMOKE_LEVEL=full scripts/image-smoke.sh` runs on that image.

## What was renamed

Every name the project owns, as D100 lists them: the crates, binaries and Rust paths, every setting, the cookie, the
headers, the `wikanvas://item/` link scheme, the database file with its backups and exports, the browser storage keys,
the frame global and the drag type, the images, builder, test projects and slices, the patches' notices and author,
every user-visible string, and the docs, handoff notes included. The slices' values are byte for byte unchanged. The
engine version is `0.2.0-beta.3+embed.wikanvas.3` (patchset 3 had still not shipped, D99). The version is
`0.1.0-alpha.1`. The LICENSE reads "Copyright (c) 2026 The Wikanvas contributors": own code is MIT only; Inkboard and
`patches/inkboard/` stay MIT OR Apache-2.0 (`web/inkboard-frame/frame.css` now names Inkboard, used here under MIT,
with a matching THIRD_PARTY.md row). Kept on purpose: the token prefix `qpat_` (it holds no name, and changing it
would invalidate every issued token).

In hand-written files the old name is left only in README's NOTE and in the D100 row. The commits below are the
detailed record; the sweep that proves it is in "The gate".

## Commits on `ws/R-rename` (from main `000fb0b`, oldest first)

| Commit | What |
|---|---|
| `5a4ed05` | Move paths: `git mv` only (176 paths), no content change |
| `329834c` | Every name the project owns, all regions in one commit (288 files) |
| `f771eed` | Version `0.1.0-alpha.1`, the LICENSE copyright line, the `frame.css` attribution |
| `8c3f0ef` | Follow-up: two comment columns realigned (R1 sweep findings 8 and 9) |
| `3956e1b` | Follow-up: ARCHITECTURE §14.1 keeps its commands on one line (finding 12) |
| `e937d9b` | Follow-up: the main checkout is named by its role, not its path (findings 5 and 7) |
| `cf549ae` | The owner's new mark in `web/public/favicon.svg` (finding 6) |
| `5b34d8b` | Regenerated: `Cargo.lock`, the six members' versions (B1) |
| `a188de8` | Regenerated: rustfmt after the longer names, formatting only (B2) |
| `abfcec9` | Regenerated: fixtures, TypeScript bindings and OpenAPI (B4 to B7) |
| `bcdd931` | Regenerated: the patches' `index` lines (B8) |
| `42ce845` | Review fix: README "Data from before v0.1.0-alpha.1" (review finding 17, with 15) |
| this note | `docs/handoff/R.md` |

R1 first made the rename in four region commits, three of which paired old and new names inconsistently (sweep
finding 11). They were squashed into `329834c` by rebuilding the commits on the same trees (`git commit-tree`, the
branch moved with a guarded `git update-ref`): the tip tree was the same before and after. Since then the branch only
gained new commits.

`5a4ed05` alone does not build (the manifests still name the old paths), as the rename plan accepted. If every commit
on main must build, squash it into `329834c` at the merge; rename detection then falls to similarity, and five small
files (three crate roots and the two slices) are under 50 % similar.

## What was regenerated (steps B0 to B9, at launch tip `cf549ae`)

Every `scripts/dev.sh` command ran with `WIKANVAS_SHARED_CARGO_HOME=/home/staffan/.cache/wikanvas-cargo`. Every build
printed "wikanvas-build.slice not installed: per-container caps only" and ran on the builder `wikanvas-limited` (cores
10-15, 12 GiB, no swap, cgroup parent none), which was not recreated.

| Step | What | Result |
|---|---|---|
| B0 | `scripts/install-build-slice.sh --check`; `wikanvas-toolchain:latest`; `scripts/toolchain.sh --builder-only` | the slice check is owner-pending (exit 3, "not installed"); the toolchain tag is the old toolchain image (`8011426db92e`); the builder was kept, nothing asked to recreate it |
| B1 | `Cargo.lock` | exactly the six members' `version` lines (`0.1.0` to `0.1.0-alpha.1`); cargo moved nothing else, so R1's hand sort was right |
| B2 | `scripts/fmt.sh` | 89 files, +285 −184, formatting only (checked per file: whitespace, the `use` re-sort, and 7 closure bodies turned into blocks past 100 columns, with their trailing commas) |
| B3 | `pnpm install --frozen-lockfile` in `web` and `e2e` | both lockfiles unchanged (neither records the name or version) |
| B4 | `pnpm --dir web fixtures`, twice | 15 files of four ydoc fixtures (getting-started, marks, table, writing-in-wikanvas); the second run gave the same hashes. The `.bin` files were decoded with Yjs: same structs, client and delete set, only the name and link scheme changed; the extra bytes in two of them are varint clocks that moved past 127 |
| B5 | `WRITE_FIXTURES=1 … convert::tests::write_server_fixtures` and `markdown_goldens` | `fixtures/ydoc/server/{links,tables}` (4 files); the Markdown goldens unchanged, which confirms R1's hand edit |
| B6 | `cargo test -p wikanvas-api export_bindings` | 4 files in `web/src/api/gen`, one doc-comment line each |
| B7 | `UPDATE_OPENAPI=1 … openapi::` | `docs/api/openapi.json` +18 −18, byte for byte the old file with the case-aware substitution; title "Wikanvas API", cookie `wikanvas_session`, `info.version` still "1.0" |
| B8 | `inkboard.sh src --force`, `export`, `export --upstream`, `check` | 35 `index` lines in the six patches; the hashes chain from the base blobs to the patched copy for all 24 files; the upstream export is notice-free; "10 cargo runs, no new warnings (11 distinct known)" |
| B9 | `inkboard.sh build` | `web/dist-inkboard` (git-ignored): `engine_version` `0.2.0-beta.3+embed.wikanvas.3`, patchset 3; `frame.js` defines `window.wikanvasBoardFrame` |

Corrections to R1's step list, found while running it: B8's `From <sha>` lines cannot change (`format-patch
--zero-commit`); B7's case-insensitive count can never be 0, because it also counts an ordinary English word that
contains the old name (the case-aware pattern gives 0, which is what the check means); B2 was larger than estimated;
B4's `.links.json` files hold no name and did not change. Not regenerated, as planned: the THIRD_PARTY.md package
tables and `fixtures/boards/*.inkboard` (no name in either).

## The gate

### The M1 release gate (HEAD `bcdd931`, 2026-10-29 11:32–11:49 +01:00)

Every line ran verbatim from the branch's worktree, from docs/PLAN.md "Acceptance (the M1 release gate)", each in its
own `bash -c` by a runner that records every exit code and does not stop on a failure, with
`WIKANVAS_SHARED_CARGO_HOME=/home/staffan/.cache/wikanvas-cargo` exported in the same command.

| # | Gate line (abridged where long) | Result | Note |
|---|---|---|---|
| 1 | `scripts/install-build-slice.sh --check` | **owner-pending** | exit 3, "wikanvas-build.slice is not installed (no FragmentPath)"; the install needs root (runbook A4) |
| 2 | `scripts/check-pins.sh` | pass | "every pin holds" |
| 3 | `docker compose -f compose.yaml config --format json \| jq -e '(.services.wikanvas.ports // []) == []'` | pass | render only |
| 4 | `! grep -qE '^\s*(encode\|header)\b' docker/Caddyfile` | pass | |
| 5 | `scripts/fmt.sh --check` | pass | |
| 6 | `scripts/dev.sh cargo clippy --workspace --all-targets --locked -- -D warnings` | pass | 0 warnings |
| 7 | `scripts/dev.sh cargo tree --locked -e normal -i libsqlite3-sys` | pass | only through rusqlite in `wikanvas-store` and `wikanvas-server` |
| 8 | `scripts/dev.sh cargo test --workspace --locked` | pass | 706 passed, 0 failed, 0 ignored: api 123 · cli 44 + `tests/cli` 69 · core 77 · server 58 + `tests/api` 196 · store 21 + `tests/store` 118 |
| 9 | `… cargo test -p wikanvas-api --locked export_bindings && git diff --exit-code web/src/api/gen` | pass | no diff |
| 10 | `… cargo test -p wikanvas-server --locked --test api -- openapi:: && git diff --exit-code docs/api/openapi.json` | pass | no diff |
| 11 | `scripts/dev.sh cargo test -p wikanvas-cli --locked` | pass | 113 (44 unit, 69 `tests/cli`) |
| 12 | `! … cargo tree --locked -p wikanvas-server -e normal --prefix none \| grep -qE '^(rustls\|ureq\|…'` | pass | the tree alone: exit 0, 463 lines |
| 13 | `! … cargo tree --locked -p wikanvas-cli -e normal --prefix none \| grep -qE '^(openssl\|…'` | pass | the tree alone: exit 0, 133 lines |
| 14 | `! grep -rn '\.route(' crates/wikanvas-server/src/routes` | pass | |
| 15 | `! grep -rn '\.route("/api' crates/wikanvas-server/src` | pass | |
| 16 | `scripts/dev.sh pnpm --dir web install --frozen-lockfile` | pass | |
| 17 | `scripts/dev.sh pnpm --dir web fixtures && git diff --exit-code fixtures` | pass | no diff |
| 18 | `… WRITE_FIXTURES=1 … write_server_fixtures && git diff --exit-code fixtures/ydoc/server` | pass | no diff |
| 19 | `… pnpm --dir web typecheck && … pnpm --dir web test && … pnpm --dir web build` | pass | 79 files, 403 tests |
| 20 | `scripts/dev.sh scripts/inkboard.sh check && … inkboard.sh build && scripts/e2e.sh frame` | pass | no new warnings; `0.2.0-beta.3+embed.wikanvas.3`; frame 25 passed, 1 skipped (W5's case) |
| 21 | `scripts/image.sh` | pass | cold build, 391 s: `wikanvas:bcdd931` = `wikanvas:latest`; `/licenses.txt` 261 crates, 102 npm packages, 217 licence texts |
| 22 | `WIKANVAS_SMOKE_LEVEL=full scripts/image-smoke.sh` | pass | 20 checks: `wikanvas-server 0.1.0-alpha.1 (bcdd931)`, no published app port, HTTPS through Caddy with the local CA, caps and no swap (cgroup parent none: no slice yet), CSPs, the br `.wasm` byte for byte, `__Host-wikanvas_session` Secure, no HSTS, peak memory under the limits |
| 23 | `scripts/e2e.sh smoke` | pass | 1 test, 72 s, on the real engine (WebGL 2 under SwiftShader), step 17 included |
| 24 | `scripts/agent-test.sh` | pass | 12 scenarios, 71 steps, 0 failed; the image's revision is HEAD (no `--allow-stale`) |
| 25 | `scripts/cli-dist.sh && scripts/dev.sh dist/cli/wikanvas-x86_64-unknown-linux-musl --version` | pass | `wikanvas 0.1.0-alpha.1` |
| 26 | `! grep -rnE 'todo!\(\|unimplemented!\(\|TODO\(W\|not implemented \(W' crates web/src web/inkboard-frame scripts e2e/agent` | pass | |
| 27 | `git -C vendor/inkboard status --porcelain \| wc -l \| grep -qx 0` | pass | |

The runner's first start of line 21 was stopped before any build step ran (a `--progress=plain` had been added to
it); line 21 above is the verbatim rerun.

Also run at `bcdd931`:

| Check | Result | Note |
|---|---|---|
| M1.1 gate item 2: `scripts/dev.sh scripts/inkboard.sh build && scripts/e2e.sh frame` | pass | covered by line 20 (no engine build followed it) |
| M1.1 gate item 5: the `TODO(E` grep | pass | |
| B9: the dist's manifest and `window.wikanvasBoardFrame` | pass | as in the regeneration table |
| B11: the image's `/licenses.txt` | pass | starts "Third-party software in Wikanvas" with a 32-character `=` line; no old name |
| B13: `wikanvas-server --version` in `wikanvas:latest` | pass | `wikanvas-server 0.1.0-alpha.1 (bcdd931)`, not `unknown` |
| B14: the sweep (see below) | pass | no tracked path with the old name; old-name lines only README.md:13 and docs/ARCHITECTURE.md:126; no fixture hit (`.bin` included); `vendor/inkboard` clean; real words 699, as before the rename; the broken-word check finds only the editor's camelCase mention identifier (3 lines), as in R1 |
| The new mark | pass | the image serves `/favicon.svg` (200, `image/svg+xml`), equal to the file of `cf549ae`, and `index.html` links it |

The sweep is R1's step B14 (`git ls-files`, a case-aware `git grep -nIP` over the tree outside `vendor`, a
`git grep -lP --text` over the fixtures' `.bin` files, the real-word count, `vendor/inkboard`'s status), run without
the table escapes: in a Markdown table a `\|` is a literal pipe to PCRE, so a pattern copied from one matches nothing
and passes on any tree. This note does not spell the patterns, since they hold the old name; the runbook's C11 has
them ready to copy.

Information only: the engine in the image (build id `c3c72fa7e6d3`, built in the Dockerfile's `inkboard-web` stage)
and the local `web/dist-inkboard` the frame suite loads (`667ddb9aa330`, from `inkboard.sh build`) are separate builds
of the same `0.2.0-beta.3+embed.wikanvas.3`. No gate compares them; the image's own check (the server embeds its own
engine build's files and manifest) passed.

### Reruns at `42ce845`, after the review fixes

`42ce845` changes only README.md, which is not in the image, the web app, the engine, the patches or the frame, so the
web, engine and frame lines were not rerun. The image was rebuilt anyway, so that `wikanvas:latest` carries the tip's
revision, and every image check ran on it.

| Rerun | Result | Note |
|---|---|---|
| `scripts/install-build-slice.sh --check` | **owner-pending** | exit 3, as line 1 |
| `scripts/fmt.sh --check` | pass | |
| `scripts/dev.sh cargo clippy --workspace --all-targets --locked -- -D warnings` | pass | 0 warnings |
| `scripts/dev.sh cargo test --workspace --locked` | pass | 706 passed, 0 failed, 0 ignored |
| `scripts/image.sh` | pass | 36 s, 37 steps cached: `wikanvas:42ce845` = `wikanvas:latest` = `sha256:a541c3f62f35…`, `wikanvas-server 0.1.0-alpha.1 (42ce845)` |
| `WIKANVAS_SMOKE_LEVEL=full scripts/image-smoke.sh` | pass | 20 checks, as line 22, at `(42ce845)` |
| `scripts/e2e.sh smoke` | pass | 1 test, 1.1 min |
| `scripts/agent-test.sh` | pass | 12 scenarios, 71 steps, 0 failed, no `--allow-stale` |
| B14 sweep | pass | as above; `git status` clean |
| The runbook's changed steps | pass | C0 against the main checkout read-only (fast-forward to `42ce845` possible, nothing in the way, all three images of the downtime present); C3's render of the new `.env` against the branch's `compose.yaml`; C4b's merge and `.env` install in a scratch clone at `000fb0b`; `bash -n` of `downtime.sh`; `admin check --json` on a throwaway volume |

## Review findings and outcomes

### R1's own sweep (12 findings)

All twelve held. Six were fixed in R1 (`8c3f0ef`, `3956e1b`, `e937d9b`, and the squash into `329834c`); five were
generated output and were regenerated in B4 to B8 (the ydoc and server fixtures, the bindings, the OpenAPI document,
the patches' `index` lines); the sixth, a favicon that read as the old initial, was the owner's design decision and is
answered by the owner's new mark (`cf549ae`). Parts of three proposed fixes were corrected before they were applied
(one would have put the old name back, one would have left no space before a comment, one named a test that does not
fail).

### The three R2 reviews (at `bcdd931`; behaviour, operations, docs and data)

Each finding was checked against the code, the scripts and the host (read-only inspects) before anything changed. The
cut-over runbook had already been corrected during the rehearsal, so a finding against R1's first runbook that the
rehearsal had already fixed is rejected as handled. The reviews found no defect in the repo's code or scripts: the
longer name changes nothing that the code does (prefix lengths, slices, padding, the compose trees and the set of
settings all checked).

| # | Finding (review) | Verdict | Outcome |
|---|---|---|---|
| 1 | The runbook's volume copy and rename use a Caddy tag this host does not have (behaviour) | holds | the runbook uses the digest form, and C0 now checks every image the downtime needs (Caddy, the new image, the rollback image) before any downtime |
| 2 | C8's `curl -sk https://127.0.0.1:8443` cannot reach the site (behaviour) | rejected | already handled: C8 uses `localhost` with the saved CA |
| 3 | C3's check cannot show the bind address (behaviour) | rejected | already handled: C3 renders the ports as JSON |
| 4 | C0 merges into the main checkout while the live Caddy bind-mounts that checkout's Caddyfile (operations) | holds | the merge moved into the downtime, right after the old containers are removed; C0 only checks that the fast-forward applies, C3 prepares the new `.env` beside it |
| 5 | Same as 1 (operations) | holds | as 1 |
| 6 | No image pin at the cut-over (operations) | rejected | already handled: C3 pins the gate's image and C0 checks its revision label |
| 7 | C8's checks cannot pass, and Caddy has no healthcheck (operations) | rejected | already handled |
| 8 | A5 fails as written, and the remedy the script prints runs a full toolchain build (operations) | holds for the runbook | A5 now removes the builder and runs `scripts/toolchain.sh --builder-only` right after A4; the script's remedy text is the same as before the rename, so it is not the rename's |
| 9 | C3's `sed` keeps a comment with the old name (operations) | rejected | already handled: C3 writes the whole file |
| 10 | The `.env` backup is not git-ignored (operations) | rejected | already handled: the backup lives under the ignored `.cache/` |
| 11 | The rollback silently drops every write made after the cut-over, and D6 removes the worktree the rolled-back Caddy mounts (operations) | holds | the rollback says it is meant for before C10, what it loses and how to save it first; D6 removes the worktree only when no container of the old project is left |
| 12 | A4's review diff cannot show the slices' values unchanged (operations) | holds | the diff names the old and new paths with `-M30%` |
| 13 | The sweep patterns as written in R1's report carry table escapes and match nothing (operations) | holds | the copyable sweep is in the runbook's C11 and above |
| 14 | D3's askpass text differs from the command it approves (operations) | holds | the text names the command exactly |
| 15 | A first start without `wikanvas.db` makes an empty database, and the purge job's orphan sweep then deletes every attachment and board image of the copied volume (docs and data, high) | holds | C7's `test -s` gate already stops the downtime script before such a start; C8 now also runs `admin check --json` (`ok`, no missing blob) and its recovery rebuilds the volume from the untouched old one; README warns in `42ce845`. An optional code guard is owner-pending (below) |
| 16 | README has no upgrade path for an instance from before the rename (docs and data) | rejected | the only such instance is the owner's live wiki, migrated once by the runbook (D100); the repository was not published yet; the data-safety part is 17 |
| 17 | Restoring an export or backup from before the rename is neither handled nor documented (docs and data) | holds | `42ce845`: README "Data from before v0.1.0-alpha.1": rename the database file (with its `-wal` and `-shm`) or the backup's or export's database to `wikanvas.db` before the first start, and why; old backups are neither listed nor pruned until renamed |
| 18 | The version goes backwards in semver: `0.1.0-alpha.1` sorts before the earlier `0.1.0` (docs and data) | holds, **owner-pending** | no code orders versions (the update banner compares equality); the wording or number is the owner's choice |

## Migration rehearsal

The live wiki's migration (the coordinator's cut-over runbook, Phase C) was rehearsed on 2026-10-29 on a copy of the
live database, in volumes, containers and a network with rehearsal-only names, all removed afterwards; the live
containers and volumes were never mounted or touched. The copy is the owner's private data; only counts and checks are
recorded here.

- **The copy:** 0.8 MB, 4 collections, 28 items (24 documents, 4 boards), no blob rows (so the copied `files/` was
  empty and the attachments' path was not rehearsed; C8's `admin check` covers it at the real cut-over), plus two
  dated backups to rehearse their rename and the backup schedule. Baseline taken on the old image (`95b452a`), the
  migration run on `wikanvas:bcdd931`.
- **The volume copy and rename (C6, C7):** as first written, the copy failed at once (the Caddy tag is not on this
  host, exit 125, nothing created); with the digest it ran in 0.18 s and the rename in 0.19 s. C7 exited 0 with no
  output; the files kept their owner and mtimes and the database was byte-identical to its source. A negative test
  (a file with the old name left behind) made C7 fail with exit 1, as it must.
- **The start (C8):** healthy in 1.16 s, no "First-run setup" line, the purge report removed nothing, the first
  scheduled backup was written under the new name; a write-scope token made on the old server worked on the new one
  (200; 401 without it).
- **Before and after (`compare.py`): PROBLEMS 0.** The 28 items, their details, the collections, the workspace and 4
  trees are equal; the ProseMirror JSON of all 24 documents is equal; the 4 board bodies are byte for byte equal; 5
  searches return the same items in the same order. The one expected difference: the Markdown of an item link is
  printed with the new scheme (15 documents, 41 links: 39 mentions, 2 board embeds).
- **Stored links (C9a):** 24 documents scanned, none holds the old scheme in an `href` or `src` (the item links are
  mention and embed nodes with an id), so nothing to rewrite. A PUT with an old-scheme `href` is refused with 422
  `invalid_document` and changes nothing.
- **The wiki's own text (C9b, "pass 2"):** the proposal lists 40 mentions of the old name in 14 items (36 to change,
  4 kept as history). The dry run listed 13 items (12 documents, 1 board) with 32 edits; the first run changed those
  13, every item verified; the second run changed nothing (idempotent). A search for the old name went from 14 hits to
  2 (the two items that keep it as history), one for Wikanvas from 20 to 23. Two of the edits name
  `wikanvas-build.slice`; the runbook skips them while the slice is not installed.
- **The workspace name:** `wikanvas workspace set` needs the admin scope (a write token gets 403, exit 6), so the
  owner renames the workspace in Settings; no admin token is made.
- **TLS:** a rehearsal Caddy on the copied Caddy volumes served the same root CA and the same leaf, the chain verified
  with the root saved before, and a request without the CA failed as it must.
- **Boards:** an unchanged board saved through the CLI and one through the API with `X-Wikanvas-Engine` both returned
  200 and read back byte for byte.
- **Token revoke (C9e):** 204, then 401 on the new server; the same token stays valid in the old volume (200), so the
  runbook revokes it there too after a rollback.
- **Rollback:** the old image restarted on the untouched old volume in 1.13 s without setup and served the same 28
  items (PROBLEMS 0). The rollback project's render names the old project, its three volumes and its network.
- **Downtime:** the C6 to C8 script body ran in 1.53 s (data volume only); the real downtime is a few seconds plus
  Caddy's start.

Not rehearsed: the owner's steps (C1, C10); the merge and the `.env` install in the real main checkout, and C11's
`inkboard.sh src --force` there (rehearsed in a scratch clone); the live containers' stop, removal and
`docker compose create`/`up` (rehearsed as `docker run` with the same image, caps and environment); the live Caddy CA;
the rollback's `git worktree add` and `up` (its render was rehearsed); Phase A's A4 and A5 (root).

## What remains

- **The build slice (owner, root).** `scripts/install-build-slice.sh --check` exits 3 until the owner installs
  `wikanvas-build.slice` with the askpass command that `scripts/install-build-slice.sh` prints (runbook A4). Right
  after it, A5 recreates the builder with the slice as its cgroup parent (`docker buildx rm wikanvas-limited; docker rm
  -f wikanvas-buildkit; scripts/toolchain.sh --builder-only`, never the plain `scripts/toolchain.sh` that the refusal
  prints), and gate line 1 is run again. Until then every build runs with the per-container caps only.
- **The live cut-over (coordinator, the cut-over runbook, after the owner agrees).** Phase C: C0's read-only checks,
  the owner drains the browser (C1), the downtime script (C4 to C8, which also merges `ws/R-rename` into main as a
  fast-forward and installs the new `.env`), the data fixes (C9), the owner's checks (C10), C11's `src --force` and
  sweep in the main checkout. The runbook pins `wikanvas:42ce845` and checks for that commit after the merge; this
  note moves the tip, so the coordinator pins the new tip's image in C0, C3, the downtime script, C8 and the
  verification, as C0 says when the tip has moved. The main checkout stays at `000fb0b` until the downtime script
  (the live Caddy bind-mounts its Caddyfile).
- **Removing the old resources (coordinator, after the owner confirms the new wiki; Phase D).** The old volumes after
  the rollback window; the old builder, its container and state volume; the two old slices (root, the owner's
  dialog); the old toolchain tag, and the old images after the rollback window (the rollback image among them); the
  two compatibility symlinks under `~/.cache`; the main checkout's ignored leftovers and the rollback worktree if one
  was made; optionally the old browser storage keys (owner).
- **The checkout folders (owner's call).** The main checkout's folder and the folder of this branch's worktree still
  carry the old name. Renaming them needs `git worktree repair`; nothing in the repo names either path.
- **The workspace name (owner).** The live workspace's name holds the old name; renaming it needs the admin scope, so
  the owner does it in Settings (C10).
- **Owner decisions left open:** the version number or its wording (finding 18); an optional code guard against the
  startup orphan sweep on a new, empty database while `/data/files` is not empty (finding 15).
- **At the merge:** whether to squash `5a4ed05` into `329834c` (only if every commit on main must build).
- **The owner's earlier open questions** are untouched: the engine's fonts' licences and the base image's OS-layer
  licences.
