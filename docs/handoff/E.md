# M1.1 board document embeds: merge and gate (2026-10-29)

## Status

The owner's request of 2026-10-28, "I want text docs to be able to be embedded and editable inside a board", is on
main. E1 (engine), E2 (server) and E3 (web) are merged, the merge stage's integration and docs are done, and **the
M1.1 gate and the whole M1 release gate pass at HEAD `f335198`, every line, in order, in one pass** (gate pass 2,
2026-10-29 00:16–00:30, clean tree at start and end; table below). Gate pass 1, at `437089b`, found one real bug: a
full image could embed an earlier build's engine. It is fixed at its cause with a check that fails the image build
otherwise (`2886079`), and pass 2 then ran from the top.

Nothing was skipped, ignored or loosened: no `#[ignore]`, no skipped test, no `--allow-stale`. The one skip in the
frame suite is W5's designed "background page for 60 s" case (headless Chromium reports a background page as
visible; ARCHITECTURE §16 item 26), as in every M1 run. Nothing ran as root, nothing was pushed, `git stash` was not
used, and `vendor/inkboard` is clean. The owner's live project `quire` (containers `quire-quire-1`, `quire-caddy-1`,
volumes `quire_*`) was never touched; every server, browser and compose project of the gate ran in the scripts' own
projects and was removed by them.

This note is committed after the gate, so `quire:f335198` is one commit behind HEAD and the next
`scripts/agent-test.sh` calls it stale until the next `scripts/image.sh` (only `docs/handoff/` changed, which
`.dockerignore` keeps out of the image).

## What was merged

| Commit | What |
|---|---|
| `01b6902` | Merge `ws/E2-server`: cards become `item_links` of kind `embed` (board → document), a box kind with `embed` twice is 422, `discard_empty` keeps embedded items, AGENTS.md §8 "Embedded pages", agent scenario 12 |
| `168d54a` | Merge `ws/E1-engine`: Inkboard patches 0005 (code) and 0006 (docs), patchset 3 (`0.2.0-beta.3+embed.quire.3`), `frame.js` embeds and the sidebar drop, 11 new frame tests |
| `05eb2be` | Merge `ws/E3-web`: the overlay with static and editing cards, "Text doc", "Embed page…", the sidebar drag source and the drop paths, the fake frame's embed members, 12 new or updated test files |
| `b0ce12a` | E3 request 3: `layers.test.ts` knows `.board-embed-popups` |
| `2bde02c` | Integration fix: "Text doc" expands the board's sidebar row, so the new page shows in the tree (D92; test in `textDoc.test.tsx`, fails without the fix) |
| `56e219a` | Smoke step 17 in `e2e/tests/smoke.spec.ts` |
| `437089b` | The workstreams' deviations folded into ARCHITECTURE §10.2, §16 item 31, §18.3–§18.14 (rows D76–D92) and PLAN M1.1; README's sentence on cards; `docs/handoff/E-merge.md` |
| `2886079` | Gate fix: the image's server stage re-embeds both dists on every run and checks the binary embeds them (ARCHITECTURE §13.2) |
| `f335198` | `E-merge.md`: the gate's finding |

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

## The gate (pass 2, HEAD `f335198`)

Every command ran from `/home/staffan/Projects/Quire` with `QUIRE_SHARED_CARGO_HOME=/home/staffan/.cache/quire-cargo`
in its environment, verbatim from `docs/PLAN.md` ("Acceptance (the M1.1 gate)" and, as its step 4, "Acceptance (the
M1 release gate)"), each in its own `bash -c`, by one runner that records every exit code and does not stop on a
failure. Images: `quire:f335198` (stub at line 22, then full at 31 and again at 54, all at this HEAD).

| # | Gate step | Command (abridged where long) | Result | Note |
|---|---|---|---|---|
| 1 | M1.1-1 E1 | `scripts/dev.sh scripts/inkboard.sh src` | pass | base + 6 patches |
| 2 | M1.1-1 E1 | `scripts/dev.sh scripts/inkboard.sh check` | pass | 10 cargo runs, no new warnings (11 distinct known) |
| 3 | M1.1-1 E1 | `… cargo test --locked -p inkboard-core` | pass | 153 unit tests and the integration tests, 0 failed |
| 4 | M1.1-1 E1 | `… cargo test --locked -p inkboard-app embed` | pass | 28 (24 `tests_embed::` and 4 font tests) |
| 5 | M1.1-1 E1 | `… cargo test --locked -p inkboard-app i18n` | pass | 7 |
| 6 | M1.1-1 E1 | `… cargo test --locked -p inkboard-formats mermaid` | pass | |
| 7 | M1.1-1 E1 | `scripts/dev.sh scripts/inkboard.sh build && jq -e '.patchset == "3" and .engine_version == "0.2.0-beta.3+embed.quire.3"' …` | pass | release build, format 2 |
| 8 | M1.1-1 E1 | `… inkboard.sh export --upstream && ! grep -rq 'Changed for Quire' .cache/inkboard-upstream` | pass | 6 notice-free patches |
| 9 | M1.1-1 E1 | `test "$(grep -c 'Changed for Quire' …0005…)" -ge 1` | pass | 7 notice lines |
| 10 | M1.1-1 E1 | `scripts/dev.sh pnpm --dir web typecheck` | pass | |
| 11 | M1.1-1 E1 | `scripts/dev.sh pnpm --dir web test src/board` | pass | |
| 12 | M1.1-1 E1 | `scripts/e2e.sh frame` | pass | 24 passed (13 M1 tests, 11 `embeds:` tests), 1 skipped (W5's designed background-page case) |
| 13 | M1.1-1 E1 | `scripts/dev.sh cargo test -p quire-core --locked -- board::` | pass | the unpatched core reads the boards |
| 14 | M1.1-1 E1 | `git -C vendor/inkboard status --porcelain \| wc -l \| grep -qx 0` | pass | |
| 15 | M1.1-1 E2 | `scripts/dev.sh cargo test -p quire-core --locked -- board::` | pass | 24 |
| 16 | M1.1-1 E2 | `… -p quire-store --locked --test store -- boards::` | pass | 18 |
| 17 | M1.1-1 E2 | `… -p quire-server --locked --test api -- boards::` | pass | 24 |
| 18 | M1.1-1 E2 | `… --test api -- openapi:: && git diff --exit-code docs/api/openapi.json` | pass | unchanged |
| 19 | M1.1-1 E2 | `… -p quire-api --locked export_bindings && git diff --exit-code web/src/api/gen` | pass | unchanged |
| 20 | M1.1-1 E2 | `scripts/dev.sh cargo clippy --workspace --all-targets --locked -- -D warnings` | pass | |
| 21 | M1.1-1 E2 | `scripts/fmt.sh --check` | pass | |
| 22 | M1.1-1 E2 | `scripts/image.sh --build-arg INKBOARD_BUILD=stub` | pass | `quire:f335198` (stub) |
| 23 | M1.1-1 E2 | `scripts/agent-test.sh --scenario '12-*'` | pass | 13 steps, 0 failed, against `quire:f335198` |
| 24 | M1.1-1 E2 | `scripts/agent-test.sh` | pass | 71 steps (01–12), 0 failed scenarios |
| 25 | M1.1-1 E3 | `scripts/dev.sh pnpm --dir web typecheck` | pass | |
| 26 | M1.1-1 E3 | `… pnpm --dir web test src/board src/editor src/components/sidebar src/pages` | pass | |
| 27 | M1.1-1 E3 | `scripts/dev.sh pnpm --dir web test` | pass | 79 files, 396 tests |
| 28 | M1.1-1 E3 | `scripts/dev.sh pnpm --dir web build` | pass | |
| 29 | M1.1-1 E3 | `! grep -rnE 'TODO\(E' web/src` | pass | |
| 30 | M1.1-2 | `scripts/dev.sh scripts/inkboard.sh build && scripts/e2e.sh frame` | pass | 24 passed, 1 skipped (as line 12) |
| 31 | M1.1-3 | `scripts/image.sh` | pass | `quire:f335198`, full engine, after the stub image of line 22: the case gate pass 1 failed on |
| 32 | M1.1-3 | `scripts/e2e.sh smoke` | pass | 1 test, 1.1 min: steps 1–16, step 17 on the engine ("smoke: step 17 ran on the engine…"), no CSP violation; WebGL 2 under SwiftShader, so no fallback |
| 33 | M1.1-3 | `scripts/agent-test.sh` | pass | 71 steps, 0 failed, `quire:f335198` (= HEAD, no `--allow-stale`) |
| 34 | M1.1-4 M1 | `scripts/install-build-slice.sh --check` | pass | the slice is installed and holds its values |
| 35 | M1.1-4 M1 | `scripts/check-pins.sh` | pass | |
| 36 | M1.1-4 M1 | `docker compose -f compose.yaml config --format json \| jq -e '(.services.quire.ports // []) == []'` | pass | reads the config only |
| 37 | M1.1-4 M1 | `! grep -qE '^\s*(encode\|header)\b' docker/Caddyfile` | pass | |
| 38 | M1.1-4 M1 | `scripts/fmt.sh --check` | pass | |
| 39 | M1.1-4 M1 | `scripts/dev.sh cargo clippy --workspace --all-targets --locked -- -D warnings` | pass | |
| 40 | M1.1-4 M1 | `scripts/dev.sh cargo tree --locked -e normal -i libsqlite3-sys` | pass | |
| 41 | M1.1-4 M1 | `scripts/dev.sh cargo test --workspace --locked` | pass | api 123 · cli 44 + `tests/cli` 69 · core 77 · server 58 + `tests/api` 196 · store 21 + `tests/store` 118; 0 failed, 0 ignored |
| 42 | M1.1-4 M1 | `… export_bindings && git diff --exit-code web/src/api/gen` | pass | |
| 43 | M1.1-4 M1 | `… --test api -- openapi:: && git diff --exit-code docs/api/openapi.json` | pass | |
| 44 | M1.1-4 M1 | `scripts/dev.sh cargo test -p quire-cli --locked` | pass | 44 unit + 69 `tests/cli` (the `flows::` and AGENTS.md coverage tests included) |
| 45 | M1.1-4 M1 | `! … cargo tree --locked -p quire-server -e normal --prefix none \| grep -qE '^(rustls\|ureq\|reqwest\|native-tls\|openssl) '` | pass | the tree itself has 465 lines (checked apart, since the `!` would also pass on an empty tree) |
| 46 | M1.1-4 M1 | `! … cargo tree --locked -p quire-cli … \| grep -qE '^(openssl\|…\|webpki-roots) '` | pass | 132 lines, checked the same way |
| 47 | M1.1-4 M1 | `! grep -rn '\.route(' crates/quire-server/src/routes` | pass | |
| 48 | M1.1-4 M1 | `! grep -rn '\.route("/api' crates/quire-server/src` | pass | |
| 49 | M1.1-4 M1 | `scripts/dev.sh pnpm --dir web install --frozen-lockfile` | pass | |
| 50 | M1.1-4 M1 | `scripts/dev.sh pnpm --dir web fixtures && git diff --exit-code fixtures` | pass | |
| 51 | M1.1-4 M1 | `… WRITE_FIXTURES=1 cargo test -p quire-core --locked -- convert::tests::write_server_fixtures && git diff --exit-code fixtures/ydoc/server` | pass | |
| 52 | M1.1-4 M1 | `pnpm --dir web typecheck && … test && … build` | pass | 79 files, 396 tests |
| 53 | M1.1-4 M1 | `scripts/dev.sh scripts/inkboard.sh check && … build && scripts/e2e.sh frame` | pass | 24 passed, 1 skipped (as line 12) |
| 54 | M1.1-4 M1 | `scripts/image.sh` | pass | `quire:f335198` = `quire:latest` |
| 55 | M1.1-4 M1 | `QUIRE_SMOKE_LEVEL=full scripts/image-smoke.sh` | pass | 20 checks: caps, no published app port, HTTPS through Caddy with the local CA, the engine's br `.wasm` byte for byte, `__Host-` Secure cookie, no HSTS, CSPs, peak memory under the limits |
| 56 | M1.1-4 M1 | `scripts/e2e.sh smoke` | pass | as line 32 |
| 57 | M1.1-4 M1 | `scripts/agent-test.sh` | pass | 71 steps, 0 failed, `quire:f335198` |
| 58 | M1.1-4 M1 | `scripts/cli-dist.sh && scripts/dev.sh dist/cli/quire-x86_64-unknown-linux-musl --version` | pass | `quire 0.1.0` |
| 59 | M1.1-4 M1 | `! grep -rnE 'todo!\(\|unimplemented!\(\|TODO\(W\|not implemented \(W' crates web/src web/inkboard-frame scripts e2e/agent` | pass | |
| 60 | M1.1-4 M1 | `git -C vendor/inkboard status --porcelain \| wc -l \| grep -qx 0` | pass | |
| 61 | M1.1-5 | `! grep -rnE 'todo!\(\|unimplemented!\(\|TODO\(W\|TODO\(E\|not implemented \(W' crates web/src web/inkboard-frame scripts e2e/agent` | pass | |

### Gate pass 1 (HEAD `437089b`)

58 lines passed; lines 32 (`scripts/e2e.sh smoke`), 55 (`QUIRE_SMOKE_LEVEL=full scripts/image-smoke.sh`) and 56
(`scripts/e2e.sh smoke`) failed, all for one cause: the full image built right after the stub image of line 22, at
the same commit, served the stub engine ("Whiteboard engine not installed"). The server stage compiles into a cache
mount shared by every image build, and cargo judges by mtime whether `quire-assets` (the rust-embed of `web/dist` and
`web/dist-inkboard`) is stale; the dists come from other stages whose cached outputs keep the older mtimes of the
build that made them, so cargo kept the stub's expansion. A reverted web change or an older commit would do the same
to a production image. Fixed in `2886079`: the step touches both dists before `cargo build`, then checks that the
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
  synthetic fallback is not used); the image fix above; this stage's note keeps PLAN's name `E-merge.md` (the task
  text said `merge-E.md`).

## What remains

- **Real-browser checks that stay manual** (ARCHITECTURE §16 item 31): the drag and the `focus` end in Firefox and
  Safari; a drop from another window on the editing card; cards on the engine's boxes during a pan, a zoom or a drag;
  the clip around the engine's menus and the one-frame occluder lag of a closing engine popup; crisp text after
  zooming in; a pinch over the editing card; a card's slash, mention, bubble and table menus, also in Focus; the
  picker's focus order; IME; a rotated card; a background tab with a card editing.
- **Documented gaps** (§18.13 items 13 and 14): a duplicate of a board with a card of itself has no `(copy, source)`
  link; boards saved before M1.1 get their links at their next save (no backfill).
- **Not mutation-checked in the real browser:** smoke step 17's "the overlay never scrolls" check was not run against
  an image with `overflow: hidden` (a full image per mutant); `css.test.ts` guards the rule statically.
- **Follow-ups from the notes:** backlinks on `item_links` kind `embed` are recorded, not served (filter through
  `shared::visible_items` when they are, §18.12); the `DELETE /items/{id}` doc comment and the `discard_empty` parameter
  description still list only the old conditions (no OpenAPI change, by §18.8).
- **Worktrees** `../Quire-E1-engine`, `../Quire-E2-server` and `../Quire-E3-web` are merged and clean, and are left in
  place (their branches too) for the coordinator to remove; this stage was not asked to remove them.
- **The owner's open questions** are untouched: the engine's fonts' licences, the base image's OS-layer licences, the
  `LICENSE-MIT` copyright line. No dependency was added, so `THIRD_PARTY.md` is unchanged.
- **The owner's live wiki** still runs the M1 image (`quire:95b452a`); `quire:latest` is now `f335198` with the
  feature. Moving the live project to it is the owner's step (`docker compose up -d` from the checkout), not done here.
