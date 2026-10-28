# Merge of W1–W6 and W8 into main (2026-10-28)

W7 reads this next. Main started clean at `47c3c67` (W0b). The seven workstream branches were merged one at a time with
`git merge --no-ff`, in the order W1, W2, W4, W6, W5, W3, W8. Every check below ran after each merge. Nothing was
pushed. `vendor/inkboard` and the worktrees `../Quire-W*` were not touched.

## Merges

| # | Branch (tip) | Merge commit | Conflicts and how they were resolved |
|---|---|---|---|
| 1 | `ws/W1-tree` (`1a122a2`) | `4182111` Merge W1 | None. W1's `Cargo.lock` change (unicode-normalization in quire-core) came in cleanly; `scripts/check-pins.sh` passes. |
| 2 | `ws/W2-platform` (`e76a030`) | `8535715` Merge W2 | `docs/api/openapi.json`: took main's side, then regenerated it (R4). The result has 47 paths and 59 operations: W1's 43 plus W2's 23, less W0b's 7 shared ones. |
| 3 | `ws/W4-documents` (`b560aab`) | `879cc02` Merge W4 | `docs/api/openapi.json`: the same steps. Result: 52 paths, 66 operations (59 + 14 − 7). |
| 4 | `ws/W6-boards` (`99e4043`) | `cc67803` Merge W6 | `docs/api/openapi.json`: the same steps. Result: 55 paths, 70 operations (66 + 11 − 7). |
| 5 | `ws/W5-inkboard` (`2bf58a8`) | `ab29987` Merge W5 | None. |
| 6 | `ws/W3-web-shell` (`fd42c4b`) | `43263e6` Merge W3 | None. |
| 7 | `ws/W8-cli` (`2837654`) | `706c4a1` Merge W8 | None. |

To regenerate the spec, the command was
`scripts/dev.sh env UPDATE_OPENAPI=1 cargo test -p quire-server --locked --test api -- openapi::`. Each time, the
regenerated file was committed as part of the merge commit. No other file conflicted, because the ownership tables held.
No lockfile conflicted, because only W1 touched `Cargo.lock` and no branch touched a `pnpm-lock.yaml`. No
`web/src/api/gen/*` file conflicted. Three branches changed bindings, each its own files:
- W1: `MoveItem.ts`;
- W4: `DocSchema.ts`, `NodeSpec.ts`, `MarkSpec.ts` and `AttrSpec.ts`;
- W6: `BoardMeta.ts`, `BoardPageMeta.ts` and `BoardPreviewQuery.ts`.

The bindings drift check passes on main.

## Fix commits

| Commit | After | What and why |
|---|---|---|
| `fba0612` | W8 | **Fix after merging W8: stale-ETag flows tests send a changed body.** Two tests failed once W4's and W6's routes were present: `flows::doc_markdown_roundtrip_and_stale_etag_exit_9` and `flows::board_put_get_preserves_unknown_fields`. Both sent the already-stored content again with a stale `If-Match` and expected exit 9. The server answers 200 with no change. That is the documented rule: for documents, ARCHITECTURE D50 says a stale base whose target equals the current state is `changed: false`; for boards, §6.2 step 4 runs the no-op hash before `If-Match`. Each stale PUT now sends a body that differs from the stored state, so the 412 and exit 9 are still checked. The board test also checks that `--force` stores the change. All 11 `flows::` tests pass. This changes W8-owned test code, and W8 should review it. |

No schema request was outstanding: every report had `schema_requests: []`. No cross-owner request was needed to get
main green, so none was applied. The open ones are listed below.

## Checks after each merge

Every command ran with `QUIRE_SHARED_CARGO_HOME=/home/staffan/.cache/quire-cargo`:

- `scripts/dev.sh cargo build --workspace --locked`
- `scripts/dev.sh cargo test --workspace --locked` (nothing skipped)
- `scripts/dev.sh cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- `scripts/fmt.sh --check`
- `scripts/dev.sh pnpm --dir web install --frozen-lockfile`, then `typecheck`, then `test`
- the openapi drift check: regenerate with `UPDATE_OPENAPI=1 … openapi::`, then `git diff --exit-code docs/api/openapi.json`
- the bindings drift check: `cargo test -p quire-api --locked export_bindings`, then `git diff --exit-code web/src/api/gen` with no untracked files
- `scripts/check-pins.sh`
- `git -C vendor/inkboard status --porcelain` and `git status --porcelain`, both empty

| After | Result | Rust tests (api · cli unit + `tests/cli` · core · server lib + `tests/api` · store lib + `tests/store`) | Web (files / tests) |
|---|---|---|---|
| W1 | all pass | 123 · 10 + 6 · 19 · 35 + 57 · 17 + 67 | 8 / 43 |
| W2 | all pass | 123 · 10 + 6 · 19 · 49 + 143 · 20 + 81 | 8 / 43 |
| W4 | all pass | 123 · 10 + 6 · 52 · 50 + 160 · 19 + 103 | 22 / 107 |
| W6 | all pass | 123 · 10 + 6 · 68 · 58 + 178 · 22 + 112 | 30 / 143 |
| W5 | all pass | same as W6 | 30 / 143 |
| W3 | all pass | same as W6 | 59 / 280 |
| W8 | **test failed**: 2 of the 68 `tests/cli` tests (see `fba0612`); everything else passed | 123 · 43 + 66 (cargo stopped at the failing binary) | 59 / 280 |
| `fba0612` | all pass | 123 · 43 + 68 · 68 · 58 + 178 · 22 + 112 | 59 / 280 |

The store's unit test count dropping from 20 to 19 after W4 is expected. `quire-store/src/docs.rs` has 2 unit tests
on W4's branch, against W0b's 3, which matches the per-branch counts in the W1, W2 and W4 reports.

`security::every_operation_enforces_its_declared_scope` passes over all 70 operations. W8's `coverage::` test passes
against the full spec.

### Tests that are W7-gated or were not run here

- `flows::` (W8, gated by W7 under PLAN R7) was **not skipped**. All 11 pass in the workspace run.
- `tests/api/integration.rs` still holds only W0b's one test, which passes. The PLAN's integration tests are W7's to
  write.
- Not run during the merge, because each needs an Inkboard build, an image build or the agent harness:
  - `scripts/inkboard.sh build`, `check` and `size`;
  - `scripts/e2e.sh frame`. Against the stub dist that main has, it now fails on purpose in W5's `beforeAll` (W5
    request W7-6), so run `scripts/dev.sh scripts/inkboard.sh build` first;
  - `scripts/image.sh` and `scripts/image-smoke.sh`;
  - `scripts/cli-dist.sh`;
  - `scripts/agent-test.sh`;
  - the Playwright smoke.
- Web test stderr noise (the tests pass): happy-dom's `Failed to load iframe page "about:blank"` comes from
  `BoardFrameHost` mounting its iframe, now also inside W3's AppShell tests. React Router's "No route matches URL
  /doc/untitled-…" comes from W6's `boardPage.test.tsx` ("Nested lists the child pages and adds one"). Both were
  already there before W3 merged.

## Integration problems the checks do not catch (for W7)

These came up while merging. Main is green with all of them, so none was fixed here. Each one needs a cross-owner change
or a design decision.

1. **High: the frame cannot load its manifest on the real server.** `web/inkboard-frame/frame.js` (W5) fetches
   `manifest.json` next to `frame.html`, and `connect()` rejects with `download` when that fails. W2's
   `static_files.rs` (`inkboard_file`) answers 404 for `/inkboard/{build}/manifest.json`, and
   `static_files::old_inkboard_build_404` asserts that 404. W5's request W2-1 to serve it was not applied. Unless that
   changes, every board in the real image fails to connect. The e2e frame test misses this because it runs against
   `e2e/serve-static.mjs`. The fix needs a decision on §11's table (W5 request W7-2): which headers, and whether the
   response is immutable and identity-encoded.
2. **Scenario `e2e/agent/scenarios/04-boards.sh` will fail at "a stale --if-match exits 9".** Lines 32–36 send the
   unchanged `future.inkboard` with `--if-match r1`, which hits the no-op rule (§6.2 step 4) and returns 0. This is the
   same problem as `fba0612`. A suggested fix (W8 owns the file):
   ```sh
   changed="$AGENT_RESULTS/changed.inkboard"
   jq -c '.document.x_future.note = "changed"' "$board" >"$changed"
   ```
   Then use `<"$changed"` for the stale PUT and for the `--force` PUT. It was not applied because the harness was not
   run here. Scenario 03 already sends different content and is fine.
3. **The board warm-up is not wired.** W6 exports `useBoardWarmup()` and asks W3 to spread it on sidebar board rows
   (W6 request W3-1). W3 exports `onBoardRowHover(handler)` and asks W6 to register a handler (W3 request W6-2).
   Neither side calls the other, so the 300 ms hover warm-up of §9.5 never runs.
4. **The Full width setting is not applied.** `DocPage.tsx` (W4) calls neither `fullWidthOf` nor `useFullWidthOf`
   (W3 request W4), and `ItemDetail` still has no `full_width` (W3 request W1-2). As a result, the setting survives
   only for the session, and nothing renders it.
5. **The discard of new empty items is inert.** W3's top bar "New doc", sidebar "+" and item menu "New" do not
   navigate with `{ state: { created: true } }` (W4 request W3-1). Only DocPage's own "Add a page" does. Since W4's
   review fix, these new items are no longer discarded when the owner leaves them empty.
6. **The seed does not use W4's seed helpers** (W4 request W2-1). `seed::getting_started` calls
   `docs::init_with(welcome, getting-started.bin)`, so the welcome document's links still point at the placeholder ids
   `docs::SEED_WRITING_ID` and `SEED_BOARD_ID` instead of the seeded items. The fix is
   `docs::init_seed(…, &[(SEED_WRITING_ID, writing_id), (SEED_BOARD_ID, board_id)], …)`. Also, "Writing in Quire"
   stays empty, although W4 added `fixtures/ydoc/writing-in-quire.bin` for it (`docs::init_with`). This is related to
   `integration::seeded_getting_started_is_published_and_searchable`.
7. **`ItemDetail.board` uses W1's own `items::board_meta`, not `boards::board_meta`** (W6 request W1-1). They differ
   in two ways:
   - W1 sets `preview_rev` to the oldest stored preview rev, and W6 to the newest;
   - W1 takes each page's `svg_len` from any rev, and W6 only from `preview_rev`.

   W1 D17 says "W6 may refine this".
8. **Board-page SVG is gzipped in two places.** W2 now gzips `image/svg+xml` on `…/board/pages/{n}` (W2 D10) and
   withdrew request W6-2. W6's route still gzips the page itself (W6-D29) and sets `Content-Encoding`, so W2's layer
   skips it. The two are compatible, but one of them should go.
9. **`quire item move` without `--parent` sends `parent_id: null`** (`crates/quire-cli/src/cmd/item.rs`), so a reorder
   moves the item to the top level, or gets a 422 when the position names a sibling. W1 request W8 (D23) asks it to
   send the current parent instead.
10. **The board host has its own update banner.** On a `frame.html` 404 the board host sets `updateAvailable` in its
    own store, and does not call W3's `checkForUpdate()` (W3 request W6-1). Check that only one "Quire was updated"
    banner can show.
11. **Tree title order differs.** W3-D1 sorts accent-insensitively with numbers in numeric order; W1 D10 sorts
    case-insensitively only (W3 request W1-1). An optimistic reorder in the web UI can briefly disagree with the
    server.

## Cross-owner requests not applied

None was needed for green (see above). They are listed by target, with the source report.

**W1 (from W3, W6):**
- W3 W1-2: add `full_width: boolean` to `ItemDetail` and regenerate the bindings (item 4).
- W3 W1-1: title order (item 11).
- W6 W1-1: `boards::board_meta` (item 7).
- W1's own follow-up D19: `dark_theme` should use W2's `users::get`, which now exists.
- Already satisfied on main: W3 W1-3 (both neighbours are accepted, `shared::place`) and W6 W1-2/4 (duplicate uses
  `boards::copy`; `/items/{id}/children` exists).

**W2 (from W4, W5, W6):**
- W5 W2-1: serve the manifest (item 1).
- W4 W2-1: the seed (item 6).
- W4 W2-3: set `oas3 = { …, default-features = false }` in `crates/quire-server/Cargo.toml`, because `preserve-order`
  changes serde_json's `Map` order in test builds. This needs a lockfile update (R6).
- W6 W2-7: `files::low_disk_507` should also cover a board PUT through an injected probe in
  `quire_store::shared::check_free_space`.
- W6 W2-4 and W2-8 are optional: move `content_disposition` to `headers.rs`, and move the board body budget into
  `AppState`.
- Already satisfied on main:
  - W6 W2-1: `app::deadline` gives board PUTs the 120 s `upload_timeout`;
  - W6 W2-3: `state.rs` calls `seed_ids`;
  - W6 W2-5: tower-http skips responses that are already encoded;
  - W6 W2-6 and W5 W2-2: a stub manifest gives `inkboard: null`, and a `format_version` mismatch stops startup;
  - W6 W2-2: no seed test assumed an empty board;
  - W4 W2-2: the `config::MAX_*` limits are listed in `/config`.
- W2's request W1 ("`revisions::prune` is still W0b's no-op") is resolved by the merge: W1's prune is on main.

**W3 (from W1, W4, W6):**
- W4 W3-1: `state.created` (item 5).
- W6 W3-1: `useBoardWarmup` (item 3).
- W6 W3-3: menus, popovers, toasts and the drawer need a z-index above the frame layer's 1. `.menu [role=menu]` is at
  10; check the rest in a browser.
- W1 W3 (D23): `POST /items/{id}/move` always sends `parent_id`. The generated `MoveItem` type requires it, and
  typecheck passes.

**W4 (from W3):** read `useFullWidthOf(item)` in DocPage (item 4).

**W6 (from W2, W3):**
- W2's withdrawal of W6-2 (item 8).
- W3 W6-1: `checkForUpdate` (item 10).
- W3 W6-2: `onBoardRowHover` (item 3).
- Already satisfied: W3 W6-3 (`host.ts` passes frame keys to `runHostShortcut`).

**W8 (from W1, W6, and this merge):**
- W1 D23: `item move` sends the current parent (item 9).
- Scenario 04 (item 2).
- Review `fba0612`.
- W6's notes (`board get` never 304; 429 `busy` with `Retry-After`; no `rev` on `board preview`) are information only.

**W7 (docs, gate and integration), collected from the handoff notes:**
- W1:
  - §4.1 unicode-normalization (D2);
  - §7.2: D3, D4, D6, D7, D14, D15 and D16;
  - §5 "Trash": D22;
  - §7.2 `…/move`: a body without `parent_id` is 422 (D23);
  - PLAN W1 "Produces": `quire_store::search::build_match`;
  - integration tests: a document's Y-derived text through `/text` and search, a board label found by search, and
    duplicate copying `item_attachments`.
- W2:
  - §12/D34: the `sqlite_stat` rows;
  - §7.2 setup 401 (D15); §7.8 admin JSON (D22); `QUIRE_BACKUP_KEEP=0` (D21);
  - the PLAN W2 table wording (D26);
  - §8/§11: no deadline for export and backup (D9), the compression rule (D10), `nosniff`/`no-store` (D11);
  - §12: the first-backup schedule, the `backups/.lock` lock and the flags held during a copy (D20, D21);
  - §8 Login: a malformed email gets 401 and is not counted (D4).
- W3:
  - §9.1: the update check compares `commit` (W3-D6);
  - §9.6.2 and §9.6.3: sort directions and drop details (W3-D1, W3-D2);
  - one real drag in the e2e smoke;
  - optionally, split the 545 kB main chunk.
- W4:
  - THIRD_PARTY.md: `convert/pm.rs` ports @tiptap/y-tiptap 3.0.10 (MIT);
  - §6.4: D4, D5, D7, D8 and D9; §6.1: D6; §7.2: `DocSchema` (D1);
  - §6.4 Writing step 1: the two-step nesting limit (D11b);
  - the §6.4 printer bullet (D8a);
  - D25 and §6.4: permits taken before the body is read (D14a).
- W5:
  - §14 and §16: the paint counter, the round trip without `view`, and a manual background-tab check (D15);
  - §11: `manifest.json` (W7-2, item 1);
  - §10.2/§10.3/§10.5: D2, D4–D7, D11 and D12, and the 0003 rows for `web/Trunk.toml`, `scripts/build_web.sh` and
    `build_web.ps1`;
  - the first full image build with the real `inkboard-web` stage, then `QUIRE_SMOKE_LEVEL=full scripts/image-smoke.sh`;
  - `docs/handoff/W0b.md`'s "e2e.sh frame (stub) PASS" no longer holds;
  - **D18**: one host-side `node --check e2e/serve-static.mjs` broke R1 (a syntax check only, which leaves nothing
    behind). It is recorded here for the owner and W7 to acknowledge; the merge did not repeat it.
- W6:
  - §6.2: the D1 signatures; §5 and §9.5: `text_excerpt` is 4 KB (D3);
  - a real-browser check of the board host, including the item menu and Nested popover over the canvas (W6-D30);
  - integration tests for board blobs and GC, board label search, and a token PUT then text and preview;
  - §7.1 and the §7.2 board GET row: no 304 and no `If-None-Match` (W6-D24);
  - D25: the 96 MiB body budget and its permit order (W6-D22, W6-D23);
  - §12 and §5: preview budgets, placeholders and omitted pages (W6-D20);
  - §6.2 step 4: asset bytes in the no-op check (W6-D25);
  - malformed tables are 422 (W6-D27);
  - board JSON deeper than 100 levels is 422 `invalid_board` (W6-D26).
- W8:
  - §7.6 and §7.7: `--json-input` and the `--json F` rewrite, the extra error and warning codes, and
    `server_unavailable`;
  - the coverage allow-list, `GET /openapi.json` and `GET /items/{id}/board.inkboard` (D-W8-17);
  - §14.1 `--expires 2d` (D-W8-18);
  - the gate: the full CLI suite and `scripts/image.sh && scripts/agent-test.sh` with all eleven scenarios (see item 2);
  - PLAN line 1300: run `bash -n` once per file (W7-6);
  - §7.6: `logout --revoke` (D-W8-25), retrying a GET after a failed body read (D-W8-24), and control-character
    escaping (D-W8-26).

## W7 merge

Main started clean at `aa644c0`. The three W7 branches were merged with `git merge --no-ff`, in the order fixes,
integration tests, docs, and every check of "Checks after each merge" above ran after each one. Nothing was pushed.
`vendor/inkboard` and the worktrees `../Quire-W7-*` were not touched.

### Merges

| # | Branch (tip) | Merge commit | Conflicts and how they were resolved |
|---|---|---|---|
| 1 | `ws/W7-fixes` (`6c58d41`) | `718998a` Merge W7 fixes | None. Its `Cargo.lock` change (oas3 without default features: serde_json loses its `indexmap`/`foldhash` edges, no version changes, still 366 + 6 packages) came in cleanly; `scripts/check-pins.sh` passes. Its `openapi.json` and `ItemDetail.ts` changes match a regeneration. |
| 2 | `ws/W7-integration-tests` (`da477f6`) | `00889fd` Merge W7 integration tests | `crates/quire-store/src/seed.rs`: both branches fixed "Writing in Quire" (W7 fixes `487096f`, W7 tests `b6b08ec`, which says so and asks to take the other side). Kept the fixes branch's version, the better fix: it stores "Writing in Quire" from its fixture too, and also maps the welcome document's placeholder ids with `docs::init_seed` (merge item 6). Both branches' seed tests pass on it: the store seed test and `integration::seeded_getting_started_is_published_and_searchable`. |
| 3 | `ws/W7-docs` (`1664e0e`) | `8979f64` Merge W7 docs | None. `docs/AGENTS.md` merged on its own: the fixes changed the `item` row and added an example, the docs the `admin` row. |

No lockfile conflicted, no `docs/api/openapi.json` or `web/src/api/gen/*` file conflicted, and only W7 fixes changed
them. The integration-tests branch's other fix, the export's second `VACUUM INTO` (`9a614d6`), touches nothing the
fixes branch changed, and its test (`integration::export_after_api_writes_has_no_tokens_or_sessions`) passes.

### Fix commits

| Commit | After | What and why |
|---|---|---|
| `8c49b8d` | W7 docs | **Fix after merging W7 docs: ARCHITECTURE follows the merged code.** The docs branch was written next to the other two, from the handoff notes, and four of its §17.4 decisions contradicted the code on main: it made `/inkboard/{build}/manifest.json` immutable with the frame CSP (the code sends `no-cache` + ETag, identity, `application/json`, `nosniff`, because the build id does not hash the manifest's `patchset`); kept the board-page route's own gzip (the code gzips once, in the compression layer, over 1 KiB); kept two title orders (the code has one key on both sides); and read Full width through `useFullWidthOf` (removed by the fixes). The text now states what main does, the four §17.4 rows say "Revised at the W7 merge" and keep the first wording as a rejected alternative, and the fixes agent's other doc requests are applied: §3 `fixtures/tree/`, §4.1 and §6.4 oas3, §5 sibling order and ties, §6.2 step 3 `boards::prepare_with`, §7.2 `ItemDetail`, §7.6 `item move`, §8 and §9.6.7 the seed, §9.5 the warm-up and the one update banner, §9.6.4 `data-full-width`, §14 the image smoke's manifest check. From the integration-tests agent: D34 and §12 describe the second `VACUUM INTO` and that `/data/tmp` briefly holds two copies of the database (the D33 floor does not scale with it). Two §17.4 rows were added (`item move` without `--parent`, and the export scrub), §16 item 27 now says the eleven merge items are fixed, and item 28 records the docs agent's point against the layer gzip (up to 16 MiB of SVG on an async worker) as a follow-up. Documentation only; no test reads `ARCHITECTURE.md`. |

The remaining doc requests of the fixes report were already in the docs branch: §9.1 (both update paths call
`checkForUpdate()`), §5 "Previews" (`preview_rev` is the newest rev, `svg_len` that rev's), and `ItemDetail.full_width`
in §7.2. W1 D19 (`dark_theme` through `users::get`) has no text in ARCHITECTURE to change.

### Checks after each merge

The same commands as above, all with `QUIRE_SHARED_CARGO_HOME=/home/staffan/.cache/quire-cargo`, `cargo test` with
`--no-fail-fast`. Nothing was skipped or ignored.

| After | Result | Rust tests (api · cli unit + `tests/cli` · core · server lib + `tests/api` · store lib + `tests/store`) | Web (files / tests) |
|---|---|---|---|
| W7 fixes | all pass | 123 · 43 + 69 · 69 · 58 + 182 · 21 + 113 | 63 / 300 |
| W7 integration tests | all pass | 123 · 43 + 69 · 69 · 58 + 190 · 21 + 113 | 63 / 300 |
| W7 docs | all pass | same as above | 64 / 304 |
| `8c49b8d` | all pass (the final run, on the merge notes commit) | same as above | 64 / 304 |

`tests/api` grew by 8: the integration tests (`integration::` now has 9, W0b's one included). The store's lib count of
21 (22 before) is the removed `previews::gzip` test. The web suite grew by W7 fixes' 4 files / 20 tests and the
docs branch's `about.test.tsx` (4 tests). The stderr noise noted above (happy-dom's `about:blank` iframe, React
Router's "No route matches") is unchanged.

### Not run here (the rest of the W7 gate)

Each needs an image or a browser, and none was part of this merge step:
- `scripts/image.sh` with the real `inkboard-web` stage (the first full build, now also with the docs branch's
  `licenses` stage), then `QUIRE_SMOKE_LEVEL=full scripts/image-smoke.sh`: the only check of the manifest fix
  (merge item 1) in the real image. Note: `quire:latest` is currently the docs agent's stub image
  (`quire:aa644c0-dirty`, built with `INKBOARD_BUILD=stub` from the docs worktree), so rebuild before any image test;
- `scripts/agent-test.sh` with all eleven scenarios, including the fixed scenario 04 (merge item 2);
- `scripts/e2e.sh smoke` and, after `scripts/dev.sh scripts/inkboard.sh build`, `scripts/e2e.sh frame`;
- `scripts/cli-dist.sh`;
- the real-browser checks of ARCHITECTURE §16 item 26: the board host with the real dist, menus and popovers over the
  canvas (held only by `styles/layers.test.ts`), the editor's menus, one real sidebar drag, and the About page.

### Still open

- Owner decisions, current pins kept, not decided here (ARCHITECTURE §16 items 24 and 25): lightningcss (MPL-2.0,
  build-only, through Vite 8.3.5: allow MPL-2.0 for build-only tools, or re-pin Vite 7); image digests younger than two
  weeks and Node 24.22.1 about 7 days old; host port 443 held by the owner's Caddy (the README explains
  `QUIRE_HTTPS_PORT`/`QUIRE_HTTP_PORT` and running behind an existing proxy). The docs agent added to the same list:
  the Inkboard wasm's fonts (SIL OFL 1.1, Ubuntu Font Licence 1.0), the runtime base image's OS layer (LGPL, GPL with
  the runtime exception, MPL-2.0 CA data), and Inkboard's own list naming Unlicense, BSL-1.0 and CDLA-Permissive-2.0
  crates that should be compiled out of the embed build (not verified with a crate list).
- Optional W6 refactors W2-4 (`content_disposition` into `headers.rs`) and W2-8 (the board body budget into
  `AppState`): not done, no behaviour change.
- Follow-ups (ARCHITECTURE §16 items 27–29): the 545 kB main web chunk; the layer's board-page gzip on an async worker;
  a CLI-only `licenses.txt`; `compose.yaml` sets `QUIRE_TRUSTED_PROXIES` from `QUIRE_PROXY_IP`, so the variable has no
  effect in `.env` (the README says to set it in `compose.override.yaml`).
- W8 should review the fixes branch's changes to its files (`crates/quire-cli/src/{args.rs,cmd/item.rs}`,
  `tests/cli/{flows,mocked}.rs`, `e2e/agent/scenarios/04-boards.sh`), as for `fba0612`, which the fixes agent reviewed
  and found correct (D50, §6.2 step 4).
