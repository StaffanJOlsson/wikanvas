# M1.1 board document embeds: merge, review fixes and gate (2026-10-29)

## Status

The owner's request of 2026-10-28, "I want text docs to be able to be embedded and editable inside a board", is on
main. E1 (engine), E2 (server) and E3 (web) are merged, the merge stage's integration and docs are done, and the
independent review of the merged feature found six defects (one high, one medium, four low). **All six hold, all six
are fixed at their cause, each with a test that fails without its fix, and the M1.1 gate and the whole M1 release gate
pass at HEAD `9c6e60a`, every line, in order, in one pass** (gate pass 3, 2026-10-29 02:01–02:17, clean tree at start
and end; table below). No finding was rejected.

Nothing was skipped, ignored or loosened: no `#[ignore]`, no skipped test, no `--allow-stale`, no assertion weakened
(the unit tests' Esc became stricter: it now carries `keyCode` 27, as a browser's does). The one skip in the frame
suite is W5's designed "background page for 60 s" case (headless Chromium reports a background page as visible;
ARCHITECTURE §16 item 26), as in every M1 run. Nothing ran as root, nothing was pushed, `git stash` was not used, and
`vendor/inkboard` is clean. The owner's live project `wikanvas` (containers `wikanvas-wikanvas-1`, `wikanvas-caddy-1`,
volumes `wikanvas_*`) was never touched; every server, browser and compose project of this stage ran in the scripts' own
projects and was removed by them.

This note is committed after the gate, so `wikanvas:9c6e60a` is one commit behind HEAD and the next
`scripts/agent-test.sh` calls it stale until the next `scripts/image.sh` (only `docs/handoff/` changed, which
`.dockerignore` keeps out of the image).

## What was merged

| Commit | What |
|---|---|
| `40b3faf` | Merge `ws/E2-server`: cards become `item_links` of kind `embed` (board → document), a box kind with `embed` twice is 422, `discard_empty` keeps embedded items, AGENTS.md §8 "Embedded pages", agent scenario 12 |
| `0be0d3f` | Merge `ws/E1-engine`: Inkboard patches 0005 (code) and 0006 (docs), patchset 3 (`0.2.0-beta.3+embed.wikanvas.3`), `frame.js` embeds and the sidebar drop, 11 new frame tests |
| `871975a` | Merge `ws/E3-web`: the overlay with static and editing cards, "Text doc", "Embed page…", the sidebar drag source and the drop paths, the fake frame's embed members, 12 new or updated test files |
| `98f29b5` | E3 request 3: `layers.test.ts` knows `.board-embed-popups` |
| `3b6ebda` | Integration fix: "Text doc" expands the board's sidebar row, so the new page shows in the tree (D92; test in `textDoc.test.tsx`, fails without the fix) |
| `b8907ac` | Smoke step 17 in `e2e/tests/smoke.spec.ts` |
| `22fb45c` | The workstreams' deviations folded into ARCHITECTURE §10.2, §16 item 31, §18.3–§18.14 (rows D76–D92) and PLAN M1.1; README's sentence on cards; `docs/handoff/E-merge.md` |
| `e1f6928` | Gate fix: the image's server stage re-embeds both dists on every run and checks the binary embeds them (ARCHITECTURE §13.2) |
| `b808bc9` | `E-merge.md`: the gate's finding |
| `d5d3c94` | This note, after gate pass 2 |
| `f3418d1` | Review fixes in the engine (0005, 0006; patchset 3 kept) and `frame.js`: the wheel's own modifiers, the middle-button pan, no style panel for cards, `clipboard`, `onEmbedFocusBack`; a new frame test |
| `c19d77b` | Review fixes in the board host: the editor's own Esc, the focus back after a pan, the clipboard's documents kept, a refused save, the drop's own detail; unit tests |
| `707ed8a` | Smoke step 17: Esc in a card's body, the pan while a card is edited |
| `9c6e60a` | ARCHITECTURE §18 (D93–D99), PLAN M1.1 and AGENTS.md describe the review's fixes |

Merged E2, E1, E3 in that order with `git merge --no-ff`, one at a time. The branches share no file and none touches
a lockfile, so there was no conflict and nothing to regenerate; `docs/api/openapi.json` and `web/src/api/gen` are
unchanged. After each merge its workstream's checks passed on main (`docs/handoff/E-merge.md`, "What was merged").

## The feature, as the owner decided it

1. **A real document.** "Text doc" creates a child document of the board (`POST /items` with `parent_id` = the board),
   which is in the tree (the board's row opens), searchable, openable full-page, and can be placed on other boards; a
   card names its document by id, so a rename, a move or another card of the same document all keep working.
2. **Editing like Inkboard's text.** A single click selects, moves and resizes the card in the engine; a double-click
   puts the caret where it was clicked; Esc or a click outside leaves; the text autosaves through the document's own
   session.
3. **Three ways to add.** "Text doc" (a new child, centred, editing at once), "Embed page…" (a picker of documents,
   placed at the centre), and a drag of a document row from the sidebar onto the canvas (placed at the drop point).

Smoke step 17 checks all of it in the real browser, on the real engine and server; the frame suite checks the
engine's side; the unit suites check the host against the fake frame.


## Review findings and outcomes

The independent review (2026-10-29, runs A–F against `wikanvas:b808bc9`, evidence in `.cache/review-embeds/`) reported
six findings. Each was checked in the code before it was fixed; each fix has a test that was seen to fail without it
(a mutation of the fix, or the test run against the pre-fix image).

| # | Severity | Finding | Verdict (checked in the code) | Fix | Tests | Commit |
|---|---|---|---|---|---|---|
| 1 | high | Esc in a card's body never leaves editing | **Holds.** prosemirror-view 1.42.6 `captureKeyDown` (`dist/index.js:2808`) returns true for keyCode 27, and its keydown handler then calls `preventDefault()`; `EmbedCard`'s bubble listener only acted when `!e.defaultPrevented`. The unit tests sent `keyCode` 0, so ProseMirror's branch never ran | `WikanvasEditor` and `editorExtensions` take `onEscape`: a keymap extension of priority 1, whose `Escape` runs after every other plugin's keys (the slash and mention suggestions take their own Esc first); `EmbedCard` passes `controller.escape`. The card's listener stays for the header (title input, buttons) and the card itself while the session opens (D93) | `editing.test.tsx`: every Esc now has `keyCode` 27; the Esc test also checks the event was `defaultPrevented`; a new title-input Esc test; the slash-menu test (first Esc closes the menu, the second leaves). Without the keymap, 2 tests fail. Smoke step 17 item 7: Esc pressed in C's body makes the card static, the frame focused, the card selected; against `wikanvas:b808bc9` (with the pan block left out of a scratch copy of the spec) it fails with the card still `editing` | `c19d77b`, `707ed8a` |
| 2 | medium | While a card is edited the board cannot be panned (Shift+wheel nothing, Alt+wheel zooms, a middle drag ends the editing) | **Holds.** `tools.rs` read egui's global `i.modifiers`, which eframe 0.33.3 sets only from key events on its canvas (and mouse moves inside the frame); while a card is edited the keys go to the parent. egui itself already applies a wheel event's own Shift/Alt to the scroll delta, so Inkboard's branch disagreed with it. The middle press (and eframe's explicit `canvas.focus()` on every `pointerup` over the canvas, `web/events.rs`) focused the frame, and `embed_frame_start` ended the editing on any focus gain | Patch 0005: the canvas takes the modifiers of the frame's last `MouseWheel` event (falling back to `i.modifiers`); a focus gain while the middle or an extra button is down, pressed or released does not end the editing, a secondary press ends it as `press` (it opens the board's menu). `frame.js`: while the host edits a card, a middle `mousedown` has its default prevented (the browser keeps the focus in the parent), and after a middle release that left the canvas focused (eframe's handler) it sends the new optional `onEmbedFocusBack`; the host puts the focus back where it last was in the card (D94) | Native: `the_wheel_follows_its_own_modifiers`, `the_middle_button_pans_without_ending_the_editing` (killed by mutating each rule). Frame suite: `embeds: while the host edits, Shift and Alt with the wheel pan and the middle button pans, keeping the focus` (real Chromium: the camera, the focus during the drag, `onEmbedFocusBack`, no end, a right-click ending it). Unit: focus back to the editor (with its selection) or the title input. Smoke step 17 item 7: Shift/Alt + wheel and a middle drag pan while C is edited, C stays editing with the caret back, " wallaby" is saved; against `wikanvas:b808bc9` it fails at "Shift + wheel pans sideways" | `f3418d1`, `c19d77b`, `707ed8a` |
| 3 | low | A new empty "Text doc" card cut (or copied and deleted) and pasted on another board is a dead card | **Holds.** `discardTextDocs` discarded every created document absent from the board's `items`; a card on the engine's clipboard (`App::clipboard`, kept across `setBoard`) has no `embed` link yet, so the server's check could not keep it; `discardTextDoc` left `['item', id]` and `['docJson', id]` cached | Patch 0005: the layout reports `clipboard`, the targets of the cards on the engine's clipboard. `textDocs.ts`: a created document on the clipboard is not discarded but kept (one list per tab) for a later leave, which discards it once no clipboard holds it and the board left does not show it (the server still decides). A successful discard resets the document's cached `ItemDetail` and `DocJson`, so a card that still names it shows "Page not found" (D95) | Native: `embed_layout_holds_the_cards_on_the_clipboard` (cut, kept across `host_load`, replaced by a copy). Frame suite: `embeds: the layout of the fixture` (`clipboard` `[]`) and `embeds: the editing state` (`[ITEM1]` after Ctrl+C). Unit (`textDoc.test.tsx`): kept at two leaves while on the clipboard, discarded at the third, its cache reset and a later card "missing"; a pasted one stays. Both mutants (no clipboard check; no cache reset) fail | `f3418d1`, `c19d77b` |
| 4 | low | A target trashed or archived elsewhere during editing: editing does not end, every save is 409, the static card later shows unsent text and a stale trash message | **Holds.** `DocSession.failed` only set an `error` status; nothing told the controller; `end()` wrote the editor's JSON into `['docJson', id]` with the previous ETag, so the refetch got a 304 and kept the unsent text; the static badge showed the session's message and the destroyed session's retry | `DocSession` gains an optional `gone` dependency, called on 409 `read_only` or 404. The card's session passes it: the editing ends and the target's detail is refetched (the card then shows "In Trash" or "Archived"). When the last flush left updates unsent, the cached `DocJson` loses its ETag before the refetch (the server's content comes back), and the static badge shows the state's label ("Not saved") with a Retry, shown while the target is writable, that opens a new session of the target to send the outbox (`retrySave`) (D96) | `editing.test.tsx`: a save refused as `read_only` ends the editing, the card becomes `trashed`, shows the server's text and "Not saved" without a Retry; restored, the Retry GETs the document and POSTs the outbox and the badge goes. `docSession.test.ts`: `gone` on 409 `read_only` and 404, not on a 500, the updates still queued. Three mutants (no `gone`, the ETag kept, the old retry) each fail | `c19d77b` |
| 5 | low | Selecting or editing a card shows Inkboard's style panel, whose text settings format the hidden title snapshot and which covers the edited card | **Holds.** `panel::visible` was true for any selection, `props` showed the text and rich-text sections for every box. That the text settings act on the label follows from `text_section`/`rich_section`, which change `b.label` of the selection (the review had not run it in the browser) | Patch 0005, `panel.rs` (now with its notice line): no panel while `embed_editing` is set; when hosted, no text sections for a selection of cards alone (Outline, Fill, Corners, Shadow, Opacity, Order stay). A mixed selection keeps them (§18.13 item 15) (D97) | Native: `the_style_panel_has_no_text_for_cards_and_hides_while_a_card_is_edited` (a plain box keeps its settings, cards have none, no panel while editing, not hosted a card has them); both mutants fail | `f3418d1` |
| 6 | low | A drop whose target is not cached is inserted whatever its kind, with the drag data's title (any length) as the stored label | **Holds.** `dropItem` checked the kind only for a cached detail and fell back to `item.title`; `snapshotTitle` had no length limit | `dropItem` fetches the target's `ItemDetail` when it is not cached, places only a live document (a doc, not in the trash, not archived), titled from that detail; a dead target places nothing, a detail that does not load gives the error toast. `snapshotTitle` cuts to 200 characters (code points), for drops, inserts and the titles map (D98) | `drop.test.tsx`: the uncached target fetched and titled from its detail, not the crafted drag title; a board, trashed or missing target (cached or fetched) places nothing; a 500 gives the toast; a 400-character title is cut to 200. Both mutants (no live-document check; no cap) fail | `c19d77b` |


The native tests are four (the message of `f3418d1` says five), killed by six mutants; the mutation runs are not part
of the gate (each one changed a file in `.cache/inkboard-src` or `web/src` and restored it; the trees were clean
before the commits).

## The gate (pass 3, HEAD `9c6e60a`)

Every command ran from `/home/staffan/Projects/Quire` with
`WIKANVAS_SHARED_CARGO_HOME=/home/staffan/.cache/wikanvas-cargo` in its environment, verbatim from `docs/PLAN.md`
("Acceptance (the M1.1 gate)" and, as its step 4, "Acceptance (the M1 release gate)"), each in its own `bash -c`, by one
runner that records every exit code and does not stop on a failure (the same runner as passes 1 and 2). Images:
`wikanvas:9c6e60a` (stub at line 22, then full at 31 and again at 54, all at this HEAD); the engine dist was rebuilt
from the patched copy at lines 7, 30 and 53.

| # | Gate step | Command (abridged where long) | Result | Note |
|---|---|---|---|---|
| 1 | M1.1-1 E1 | `scripts/dev.sh scripts/inkboard.sh src` | pass | base + 6 patches |
| 2 | M1.1-1 E1 | `scripts/dev.sh scripts/inkboard.sh check` | pass | 10 cargo runs, no new warnings (11 distinct known) |
| 3 | M1.1-1 E1 | `… cargo test --locked -p inkboard-core` | pass | 153 unit tests and the integration tests, 0 failed |
| 4 | M1.1-1 E1 | `… cargo test --locked -p inkboard-app embed` | pass | 32 (28 `tests_embed::`, 4 of them new, and 4 font tests) |
| 5 | M1.1-1 E1 | `… cargo test --locked -p inkboard-app i18n` | pass | 7 |
| 6 | M1.1-1 E1 | `… cargo test --locked -p inkboard-formats mermaid` | pass |  |
| 7 | M1.1-1 E1 | `scripts/dev.sh scripts/inkboard.sh build && jq -e '.patchset == "3" and .engine_version == "0.2.0-beta.3+em…` | pass | release build, patchset 3 kept (D99) |
| 8 | M1.1-1 E1 | `scripts/dev.sh scripts/inkboard.sh export --upstream && ! grep -rq 'Changed for Wikanvas' .cache/inkboard-upst…` | pass | 6 notice-free patches |
| 9 | M1.1-1 E1 | `test "$(grep -c 'Changed for Wikanvas' patches/inkboard/0005-embed-documents.patch)" -ge 1` | pass | 8 notice lines (7 before; `panel.rs` is new in 0005) |
| 10 | M1.1-1 E1 | `scripts/dev.sh pnpm --dir web typecheck` | pass |  |
| 11 | M1.1-1 E1 | `scripts/dev.sh pnpm --dir web test src/board` | pass | 110 tests |
| 12 | M1.1-1 E1 | `scripts/e2e.sh frame` | pass | 25 passed (13 M1 tests, 12 `embeds:` tests), 1 skipped (W5's designed background-page case) |
| 13 | M1.1-1 E1 | `scripts/dev.sh cargo test -p wikanvas-core --locked -- board::` | pass | the unpatched core reads the boards |
| 14 | M1.1-1 E1 | `git -C vendor/inkboard status --porcelain \| wc -l \| grep -qx 0` | pass |  |
| 15 | M1.1-1 E2 | `scripts/dev.sh cargo test -p wikanvas-core   --locked -- board::` | pass | 24 |
| 16 | M1.1-1 E2 | `scripts/dev.sh cargo test -p wikanvas-store  --locked --test store -- boards::` | pass | 18 |
| 17 | M1.1-1 E2 | `scripts/dev.sh cargo test -p wikanvas-server --locked --test api   -- boards::` | pass | 24 |
| 18 | M1.1-1 E2 | `scripts/dev.sh cargo test -p wikanvas-server --locked --test api   -- openapi:: && git diff --exit-code docs/a…` | pass | unchanged |
| 19 | M1.1-1 E2 | `scripts/dev.sh cargo test -p wikanvas-api --locked export_bindings && git diff --exit-code web/src/api/gen` | pass | unchanged |
| 20 | M1.1-1 E2 | `scripts/dev.sh cargo clippy --workspace --all-targets --locked -- -D warnings` | pass |  |
| 21 | M1.1-1 E2 | `scripts/fmt.sh --check` | pass |  |
| 22 | M1.1-1 E2 | `scripts/image.sh --build-arg INKBOARD_BUILD=stub` | pass | `wikanvas:9c6e60a` (stub) |
| 23 | M1.1-1 E2 | `scripts/agent-test.sh --scenario '12-*` | pass | 13 steps, 0 failed, against `wikanvas:9c6e60a` |
| 24 | M1.1-1 E2 | `scripts/agent-test.sh` | pass | 71 steps (01–12), 0 failed scenarios |
| 25 | M1.1-1 E3 | `scripts/dev.sh pnpm --dir web typecheck` | pass |  |
| 26 | M1.1-1 E3 | `scripts/dev.sh pnpm --dir web test src/board src/editor src/components/sidebar src/pages` | pass | 233 tests |
| 27 | M1.1-1 E3 | `scripts/dev.sh pnpm --dir web test` | pass | 79 files, 403 tests |
| 28 | M1.1-1 E3 | `scripts/dev.sh pnpm --dir web build` | pass |  |
| 29 | M1.1-1 E3 | `! grep -rnE 'TODO\(E' web/src` | pass |  |
| 30 | M1.1-2 | `scripts/dev.sh scripts/inkboard.sh build && scripts/e2e.sh frame` | pass | 25 passed, 1 skipped (as line 12) |
| 31 | M1.1-3 | `scripts/image.sh` | pass | `wikanvas:9c6e60a`, full engine, after the stub image of line 22 |
| 32 | M1.1-3 | `scripts/e2e.sh smoke` | pass | 1 test, 1.2 min: steps 1–16, step 17 on the engine with the review's checks (Esc in the body, Shift/Alt + wheel, the middle-button pan, " wallaby" saved), no CSP violation; WebGL 2 under SwiftShader |
| 33 | M1.1-3 | `scripts/agent-test.sh` | pass | 71 steps, 0 failed, `wikanvas:9c6e60a` (= HEAD, no `--allow-stale`) |
| 34 | M1.1-4 M1 | `scripts/install-build-slice.sh --check` | pass | the slice is installed and holds its values |
| 35 | M1.1-4 M1 | `scripts/check-pins.sh` | pass |  |
| 36 | M1.1-4 M1 | `docker compose -f compose.yaml config --format json \| jq -e '(.services.wikanvas.ports // []) == []` | pass | reads the config only |
| 37 | M1.1-4 M1 | `! grep -qE '^\s*(encode\|header)\b' docker/Caddyfile` | pass |  |
| 38 | M1.1-4 M1 | `scripts/fmt.sh --check` | pass |  |
| 39 | M1.1-4 M1 | `scripts/dev.sh cargo clippy --workspace --all-targets --locked -- -D warnings` | pass |  |
| 40 | M1.1-4 M1 | `scripts/dev.sh cargo tree --locked -e normal -i libsqlite3-sys` | pass |  |
| 41 | M1.1-4 M1 | `scripts/dev.sh cargo test --workspace --locked` | pass | api 123 · cli 44 + `tests/cli` 69 · core 77 · server 58 + `tests/api` 196 · store 21 + `tests/store` 118; 0 failed, 0 ignored |
| 42 | M1.1-4 M1 | `scripts/dev.sh cargo test -p wikanvas-api --locked export_bindings && git diff --exit-code web/src/api/gen` | pass |  |
| 43 | M1.1-4 M1 | `scripts/dev.sh cargo test -p wikanvas-server --locked --test api -- openapi:: && git diff --exit-code docs/api…` | pass |  |
| 44 | M1.1-4 M1 | `scripts/dev.sh cargo test -p wikanvas-cli --locked` | pass | 44 unit + 69 `tests/cli` |
| 45 | M1.1-4 M1 | `! scripts/dev.sh cargo tree --locked -p wikanvas-server -e normal --prefix none \| grep -qE '^(rustls\|ureq\|r…` | pass | the tree itself has 465 lines (checked apart, since the `!` would also pass on an empty tree) |
| 46 | M1.1-4 M1 | `! scripts/dev.sh cargo tree --locked -p wikanvas-cli -e normal --prefix none \| grep -qE '^(openssl\|openssl-s…` | pass | 132 lines, checked the same way |
| 47 | M1.1-4 M1 | `! grep -rn '\.route(' crates/wikanvas-server/src/routes` | pass |  |
| 48 | M1.1-4 M1 | `! grep -rn '\.route("/api' crates/wikanvas-server/src` | pass |  |
| 49 | M1.1-4 M1 | `scripts/dev.sh pnpm --dir web install --frozen-lockfile` | pass |  |
| 50 | M1.1-4 M1 | `scripts/dev.sh pnpm --dir web fixtures && git diff --exit-code fixtures` | pass |  |
| 51 | M1.1-4 M1 | `scripts/dev.sh env WRITE_FIXTURES=1 cargo test -p wikanvas-core --locked -- convert::tests::write_server_fixtu…` | pass |  |
| 52 | M1.1-4 M1 | `scripts/dev.sh pnpm --dir web typecheck && scripts/dev.sh pnpm --dir web test && scripts/dev.sh pnpm --dir …` | pass | 79 files, 403 tests |
| 53 | M1.1-4 M1 | `scripts/dev.sh scripts/inkboard.sh check && scripts/dev.sh scripts/inkboard.sh build && scripts/e2e.sh frame` | pass | 25 passed, 1 skipped (as line 12) |
| 54 | M1.1-4 M1 | `scripts/image.sh` | pass | `wikanvas:9c6e60a` = `wikanvas:latest` |
| 55 | M1.1-4 M1 | `WIKANVAS_SMOKE_LEVEL=full scripts/image-smoke.sh` | pass | 20 checks: caps, no published app port, HTTPS through Caddy with the local CA, the engine's br `.wasm` byte for byte, `__Host-` Secure cookie, no HSTS, CSPs, peak memory under the limits |
| 56 | M1.1-4 M1 | `scripts/e2e.sh smoke` | pass | as line 32 (1.1 min) |
| 57 | M1.1-4 M1 | `scripts/agent-test.sh` | pass | 71 steps, 0 failed, `wikanvas:9c6e60a` |
| 58 | M1.1-4 M1 | `scripts/cli-dist.sh && scripts/dev.sh dist/cli/wikanvas-x86_64-unknown-linux-musl --version` | pass | `wikanvas 0.1.0` |
| 59 | M1.1-4 M1 | `! grep -rnE 'todo!\(\|unimplemented!\(\|TODO\(W\|not implemented \(W' crates web/src web/inkboard-frame scr…` | pass |  |
| 60 | M1.1-4 M1 | `git -C vendor/inkboard status --porcelain \| wc -l \| grep -qx 0` | pass |  |
| 61 | M1.1-5 | `! grep -rnE 'todo!\(\|unimplemented!\(\|TODO\(W\|TODO\(E\|not implemented \(W' crates web/src web/inkboard-…` | pass |  |

### Gate pass 2 (HEAD `b808bc9`, before the review)

All 61 lines passed (2026-10-29 00:16–00:30). The review then ran against `wikanvas:b808bc9` and found the six defects
above; none of them was covered by a gate line, since the unit tests sent Esc without its `keyCode` and smoke step 17
pressed Esc only in the title input and never panned while a card was edited.

### Gate pass 1 (HEAD `22fb45c`)

58 lines passed; lines 32 (`scripts/e2e.sh smoke`), 55 (`WIKANVAS_SMOKE_LEVEL=full scripts/image-smoke.sh`) and 56
(`scripts/e2e.sh smoke`) failed, all for one cause: the full image built right after the stub image of line 22, at
the same commit, served the stub engine ("Whiteboard engine not installed"). The server stage compiles into a cache
mount shared by every image build, and cargo judges by mtime whether `wikanvas-assets` (the rust-embed of `web/dist` and
`web/dist-inkboard`) is stale; the dists come from other stages whose cached outputs keep the older mtimes of the
build that made them, so cargo kept the stub's expansion. A reverted web change or an older commit would do the same
to a production image. Fixed in `e1f6928`: the step touches both dists before `cargo build`, then checks that the
binary embeds every file name of both dists and the engine's build id. With the check alone the stub-then-full
sequence failed the image build; with the touch it builds and the full image smoke passes. Pass 2 ran from the top.


## Deviations and decisions

All recorded where they apply; nothing was decided outside the owner decisions and §18:
- **The workstreams' deviations** (E1-D1–D12, E2-D1–D11 and reviews R-1, R-3, R-4, E3-D1–D15 and reviews R1–R7) are
  written into ARCHITECTURE §18.3–§18.13, each tagged with its source, with decision rows D76–D91 in §18.14; PLAN
  M1.1 has the frame-test readings, the frame-pixels test and scenario step 10b.
- **The merge's own** (`docs/handoff/E-merge.md`): "Text doc" expands the board's sidebar row (D92); smoke step 17 runs
  in a 1600 × 1000 window, starts its item 7 with a single click that must select without editing, compares C's text
  before and after the ten undos, and uses a real mouse drag (Playwright carries it into the frame, so the PLAN's
  synthetic fallback is not used); the image fix of gate pass 1; this stage's note keeps PLAN's name `E-merge.md` (the
  task text said `merge-E.md`).
- **The review's** (§18.14 D93–D99, each written into the section it amends): the editor's own Esc (D93); the wheel's
  modifiers, the middle and secondary buttons, and `onEmbedFocusBack` (D94); the clipboard's documents kept for a later
  leave and the cache reset (D95); a refused save ending the editing, the server's content and "Not saved" with a Retry
  that sends the outbox (D96); the style panel for cards (D97); the drop's own detail and the 200-character snapshot
  (D98). **D99: patchset `3` is kept** for the review's changes to 0005 and 0006: patchset 3 never shipped (the owner's
  wiki runs M1's patchset 2), PLAN M1.1 names `3` as the series M1.1 ships and its gate checks it, and builds are still
  told apart by their content-hashed build id (the image's engine is now build `0a485ce28d2e`). M1 had bumped 1 to 2
  for its review fixes; if the owner prefers that precedent, the bump is the patchset line of
  `patches/inkboard/README.md` plus PLAN M1.1's E1 "Produces" text and its `jq -e` acceptance line (gate line 7).
- **Additive contract changes** (all optional, so the M1 frame and the fake frame still satisfy the bridge):
  `EngineEmbedLayout.clipboard` (`inkboard.d.ts`), `EmbedLayout.clipboard?`, `FrameEvents.onEmbedFocusBack?` and
  `EmbedFocusBackEvent` (`bridge.ts`), `EmbedSink.focusBack?`, `DocSessionDeps.gone?`, `WikanvasEditorProps.onEscape?`.
  No route, DTO, OpenAPI or CLI change; no dependency, so `THIRD_PARTY.md` is unchanged.

## What remains

- **Real-browser checks that stay manual** (ARCHITECTURE §16 item 31): the drag and the `focus` end in Firefox and
  Safari; in Firefox and Safari also the middle-button pan's focus (whether their mousedown default moves the focus, and
  `onEmbedFocusBack` after eframe's release focus); a trackpad's horizontal scroll while a card is edited; a drop from
  another window on the editing card; cards on the engine's boxes during a pan, a zoom or a drag; the clip around the
  engine's menus and the one-frame occluder lag of a closing engine popup; crisp text after zooming in; a pinch over the
  editing card; a card's slash, mention, bubble and table menus, also in Focus; the picker's focus order; IME; a rotated
  card; a background tab with a card editing. Only headless Chromium is automated.
- **Documented gaps** (§18.13): items 13 and 14 as before (a duplicate of a board with a card of itself has no
  `(copy, source)` link; boards saved before M1.1 get their links at their next save), and from the review items 15–17
  (a mixed selection keeps the text settings; a "Text doc" kept for the clipboard stays when the tab closes before a
  later leave; after a refused save the unsent text waits in the outbox until Retry or the document's next session).
- **Not mutation-checked in the real browser:** smoke step 17's "the overlay never scrolls" check against an image
  with `overflow: hidden` (a full image per mutant; `css.test.ts` guards the rule statically). The review's two smoke
  checks were run against the pre-fix image instead and fail there (finding 1 and 2 rows above).
- **Follow-ups:** backlinks on `item_links` kind `embed` are recorded, not served (filter through
  `shared::visible_items` when they are, §18.12); the `DELETE /items/{id}` doc comment and the `discard_empty`
  parameter description still list only the old conditions (no OpenAPI change, by §18.8); the frame suite's
  `embeds: open instead of label editing` failed once in an iteration run against a `--quick` engine (the plain box's
  label had not taken the typed text yet) and passed in every other run, including the gate's three frame-suite runs:
  if it shows up again, its fixed 300 ms waits should become polls.
- **Upstream Inkboard wishes** (none opened; the series stays MIT OR Apache-2.0 and `export --upstream` writes it
  notice-free): propose the optional `BoxObj.embed` field and its lenient reader (0005's `inkboard-core` part), so
  that upstream keeps cards instead of dropping `embed` on save (§18.13 item 10); the host card API (`onEmbedLayout`
  with `clipboard`, `onEmbedOpen`, `onEmbedEditEnd`, `insertEmbed`, `setEmbedEditing`, `setEmbedTitles`,
  `requestEmbedLayout`); reading the wheel's modifiers from the wheel event, which helps any embedding whose page keeps
  the keyboard focus elsewhere; and patches 0001–0004 as before. For egui/eframe: its web runner focuses the canvas on
  every `pointerup` over it, whatever the button (`web/events.rs`, "forcing the focus on any click"), which is why the
  frame has to hand the focus back after a middle-button pan.
- **Worktrees** `/home/staffan/Projects/Wikanvas-E1-engine`, `/home/staffan/Projects/Wikanvas-E2-server` and
  `/home/staffan/Projects/Wikanvas-E3-web` are merged and clean, and are left in place (their branches too) for the
  coordinator to remove.
- **The owner's open questions** are untouched: the engine's fonts' licences, the base image's OS-layer licences, the
  `LICENSE-MIT` copyright line.
- **The owner's live wiki** still runs the M1 image (`wikanvas:1e52425`); `wikanvas:latest` is now `9c6e60a` with the
  feature and the review's fixes. Moving the live project to it is the owner's step (`docker compose up -d` from the
  checkout), not done here.
