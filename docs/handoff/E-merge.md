# M1.1 merge stage: board document embeds (E1, E2, E3 into main)

Owner request (2026-10-28): "I want text docs to be able to be embedded and editable inside a board." The binding owner
decisions are ARCHITECTURE §18's three. This note is the merge stage's own record (PLAN M1.1 "Merge stage (M)");
the gate results are in `docs/handoff/E.md`.

Naming: PLAN M1.1 names this note `docs/handoff/E-merge.md` (and smoke step 5 refers to it), while the workflow's task
text called it `merge-E.md`. The PLAN is the normative document, so the note keeps its name; `E.md` is the gate report.

## What was merged

From main `ec62389` (the M1.1 spec and contracts), one branch at a time, `git merge --no-ff`, in the order E2, E1, E3.
No file is shared between the three branches (checked with `git diff --stat main...<branch>` before merging), so
there was no conflict, no lockfile in any branch and nothing to regenerate (`docs/api/openapi.json` and
`web/src/api/gen` unchanged).

| Merge | Branch | What it brings | Checks on main right after it |
|---|---|---|---|
| `40b3faf` | `ws/E2-server` (7 commits) | `wikanvas-core::board` embed scan, `item_links` kind `embed` for boards, the repeated-`embed` 422, `discard_empty` keeps embedded items, AGENTS.md §8 "Embedded pages", agent scenario 12 | `scripts/fmt.sh --check` PASS; clippy `--workspace --all-targets -D warnings` PASS; core `board::` 24 PASS; store `boards::` 18 PASS; API `boards::` 24 PASS |
| `0be0d3f` | `ws/E1-engine` (6 commits) | Patches 0005 and 0006 (patchset 3), `frame.js` embeds and drag and drop, 11 new frame tests | `inkboard.sh src`, `check`; `inkboard-core` tests (153 unit + integration); `inkboard-app embed` 28; `i18n` 7; `inkboard-formats mermaid`; `build` (patchset `3`, `0.2.0-beta.3+embed.wikanvas.3`, build id `085b5dbedb2d`, the same as E1's); `export --upstream` notice-free; 7 notice lines in 0005; `vendor/inkboard` clean: all PASS |
| `871975a` | `ws/E3-web` (12 commits) | The overlay, static and editing cards, "Text doc", "Embed page…", the sidebar drag source and the drop paths, the fake frame's embed members | `pnpm typecheck` PASS; `pnpm test` 79 files, 396 tests PASS (after 98f29b5); `pnpm build` PASS; `scripts/e2e.sh frame` 24 passed, 1 skipped (W5's designed "background page" skip in headless Chromium) |

## Requests applied

| From | Request | Done |
|---|---|---|
| E1 1 | §10.2: 23 notice lines, patchset 3 | §10.2 amended (also the "nothing under `inkboard-core` changes" sentence and the `engineVersion` example of §10.4) |
| E1 2 | Fold deviations 1–11 into §18.3/§18.4 and §18.14 | In place, tagged E1-Dn; rows D76–D82 |
| E1 3, 6 | PLAN E1 frame-test table: the two layouts after `open()`, the read-only control, the frame-pixels test | PLAN M1.1 E1 table |
| E1 4 | §16 item 31: the real drag in Chrome, Firefox and Safari; the occluder lag | §16 item 31 (the Chromium drag is now covered by the smoke; Firefox and Safari stay manual) |
| E1 5 | E3's fake frame and the two layouts | Nothing to do: E3's tests do not count layouts after `open()` |
| E1 7 | §18.3 "Native unit tests": the three review tests | §18.3 |
| E2 M-1, M-3 | Fold E2-D1–D7 and review R-1, R-4 into §18.8/§18.13 | §18.8 "As built", §18.13 items 13 (R-4) and 14 (boards saved before M1.1); rows D83–D85 |
| E2 M-2 | Keep AGENTS.md §8's marker comment | Not edited by the merge; §18.10 now says the marker must be kept |
| E2 E1-1 | The engine reads `embed` as a `serde_json::Value`, so the last `item` wins on both sides | Holds (E2's review read 0005's `lenient_embed`; §18.8) |
| E2 M-3 | PLAN E2 step 10b, 13 TAP steps | PLAN M1.1 E2 scenario list |
| E3 1 | Smoke step 17 with the §18.5 selectors | `e2e/tests/smoke.spec.ts` step 17 |
| E3 2 | Fold E3-D1–D15 into §18.5/§18.6/§18.14 | In place, tagged E3-Dn; rows D86–D91 |
| E3 3 | `layers.test.ts`: `.board-embed-popups` in `OVERLAYS` | `98f29b5` |
| E3 4 | The host's assumptions about the engine, checked once both are merged | All four hold, below |
| E3 5 | §16 item 31: the real-browser checks | §16 item 31 |

## Integration: the real engine against E3's host, the server's links against the web

Checked by reading both sides and by smoke step 17 against the real image (engine, host and server together):

1. **E3's request 4, against the engine's code** (`embed_cards.rs` in 0005, `frame.js`):
   - layouts reach the host only for the current board: `frame.js` stamps them with `current`, which `openNow` sets
     right after `setBoard`, and the engine's forced first report comes from a later frame. Holds.
   - the first layout after `onEmbedOpen` still says `editing: null`: `embed_frame_end` builds the layout at the end of
     the frame in which `open_embed` queued the open. Holds; the host's `sawEditing` handles it.
   - a card inserted by `insertEmbed` can be put into editing in the same task: `insert_embed` adds it with
     `ed.insert` before returning, and `set_embed_editing` checks `is_visible_card` on the current page. Holds; smoke
     step 17 item 2 (a "Text doc" card editing at once) is the real-browser proof.
   - `setEmbedEditing(null)` sends no `onEmbedEditEnd`: `set_embed_editing(None)` only clears the field. Holds.
2. **"Text doc" left the new page out of view (fixed, `3b6ebda`, D92).** The tree expands only the current item's
   ancestors (§9.6.2), so the board's own row stayed closed and its new child was invisible, while smoke step 17 (PLAN,
   item 2) expects the sidebar to show the "Untitled" child right after the POST. Expanding it by hand would end the
   editing (a pointerdown outside the card). The button now expands the board's row once the card is placed; textDoc
   "creates a child document…" checks it and fails without the fix.
3. **The server's `embed` links and the web's discard.** The web discards an empty "Text doc" document on no card
   with `DELETE …?discard_empty=true` and ignores a 409 (§18.6 (a)); E2's `discard_empty` answers 409 `not_empty` for
   any item an `embed` link targets, with its own message, which the web never shows. Consistent; no change.
4. **The sidebar drag crosses into the frame in Playwright.** A real mouse drag (down on A's row, twenty moves, up over
   the canvas) gave the card centred on the drop point, so the synthetic `DataTransfer` fallback of PLAN smoke item 5
   is not used and no reason needed recording. Firefox and Safari stay manual (§16 item 31).

## Found by the gate

5. **A full image could embed an earlier build's engine or web app (fixed, `e1f6928`).** Gate pass 1 (HEAD `22fb45c`)
   built the E2 block's stub image and then, at the same commit, the full image of gate step 3: that image served
   the stub engine ("Whiteboard engine not installed"), so smoke step 9 and the full image smoke failed (gate lines
   32, 55 and 56; the other 58 lines passed). The server stage compiles into a cache mount that every image build
   shares, and cargo judges by mtime whether `wikanvas-assets` (the rust-embed of `web/dist` and `web/dist-inkboard`) is
   stale. The two dists are copied from other stages, whose cached outputs keep the mtimes of the build that made
   them, which were older than the stub build's compile; cargo kept the stub's expansion and `wikanvas-server`, which
   recompiles for every new `WIKANVAS_COMMIT`, linked it. A reverted web change or an older commit does the same, so a
   production image could have shipped another build's files. The step now touches both dists before `cargo build`
   (so `wikanvas-assets` embeds them again, about 2 s) and then checks that the binary embeds every file name of both
   dists and the engine's build id, failing the build otherwise. With the check alone, the stub-then-full sequence
   failed the build ("wikanvas-server does not embed web/dist-inkboard/inkboard.js.br"; only `wikanvas-server` had
   recompiled); with the touch it builds, and `WIKANVAS_SMOKE_LEVEL=full scripts/image-smoke.sh` passes on the result.
   ARCHITECTURE §13.2 quotes the new stage.

## Smoke step 17: decisions (within PLAN M1.1 "Smoke step 17")

- **Window.** 1600 × 1000 for the step, restored after it. In the default 1280 × 720, the two cards that "Text doc"
  and "Embed page…" both centre on the view leave almost no empty canvas for the drop at P and the clicks "on empty
  canvas".
- **P** is at 86 % of the canvas width, at its vertical centre: right of the two centred cards, clear of the engine's
  floating cards (top, left, bottom).
- **Single click.** The task asked for "single click selects (no editing)" too: item 7 starts with a single click on
  C's card, which shows "Double-click to edit" (selected, static) and stays static for 600 ms before the double-click.
- **"Ctrl+Z ten times … leave the text as it was".** C's stored text is read after " numbat" is saved and must be the
  same after the ten undos and the click that ends the editing (the card's new session has its own undo stack, so the
  undos reach the ten Enters only).
- **"A's text".** A's body is the board embed of "Sketch" (step 11), so A's card must show "Sketch".
- **Empty canvas** for the clicks and the wheel is 40 px above C's card's box (28 px above the engine's box): the wheel
  zooms about the pointer, so the point stays above the card while it grows.
- **The reload** is `page.reload()` on the board's URL.
- Not mutation-checked in the real browser: the `scrollTop`/`scrollLeft` 0 and box checks of item 7 were not run
  against an image with `overflow: hidden` (a full image rebuild per mutant); E3's `css.test.ts` guards the rule
  statically.

## Gaps left as they are

- E2 review R-4 (a duplicate of a board with a card of itself has no `(copy, source)` link) and boards saved before
  M1.1 having no links until their next save: §18.13 items 13 and 14.
- `crates/wikanvas-server/src/routes/items.rs`' doc comment of `DELETE /items/{id}` and the `discard_empty` parameter
  description in `wikanvas-api` still list only the old conditions, as §18.8 requires (no OpenAPI change).
- Backlinks on `item_links` kind `embed` are recorded, not served (§18.12).

## Worktrees

`../Wikanvas-E1-engine`, `../Wikanvas-E2-server` and `../Wikanvas-E3-web` are merged and clean. They were left in place
for the owner to remove (the coordinator's memory says "remove after"); their branches are kept, as for M1's.
