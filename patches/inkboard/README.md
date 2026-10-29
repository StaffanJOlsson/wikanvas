# Inkboard patches for Wikanvas

Wikanvas embeds [Inkboard](https://github.com/VictorLetirand/inkboard) (MIT OR Apache-2.0) as a WebAssembly whiteboard
engine. The submodule `vendor/inkboard` is never modified and never built in place. Wikanvas's changes live here as a
patch series, applied to a self-contained copy in `.cache/inkboard-src` by `scripts/inkboard.sh`. The design is in
`docs/ARCHITECTURE.md` §10.

## Base

| | |
|---|---|
| Upstream | `https://github.com/VictorLetirand/inkboard.git` |
| Commit | `41e0295d50433faa5d05b2ff48f85eac079a981d` (tag `v0.2.0-beta.3`, merge of PR #31, 2026-10-27) |
| Workspace version | `0.2.0-beta.3` (`vendor/inkboard/Cargo.toml`; `scripts/inkboard.sh src` refuses any other) |
| Board format | `inkboard_core::serial::VERSION = 2` (`crates/inkboard-core/src/serial.rs`) |
| Patchset | `3` (bump it with every change to a series that has shipped: the dist's `engine_version` is `0.2.0-beta.3+embed.wikanvas.<patchset>`; the M1.1 review's changes to 0005 and 0006, and the rename's changes to the notices, comments and `From:` lines, came before 3 shipped and kept it, docs/ARCHITECTURE.md D99, D100) |

`scripts/check-pins.sh` (on the host) checks that the submodule is at this commit and clean, and that the crates
Wikanvas shares with `inkboard-core` (serde, serde_json, base64, thiserror) have the same versions in both lockfiles.

## Patch series

`git format-patch` output against the base copy (`base` commit in `.cache/inkboard-src`), applied in order with
`git am --no-3way`. Nothing under `inkboard-session` changes. `inkboard-core` changes in one place only: 0005 adds the
optional `BoxObj.embed` field (with `embed: None` at its construction sites in `inkboard-core`'s tests and in
`inkboard-formats`), read leniently, so the server still compiles `inkboard-core` unpatched from the same commit as the
wasm and reads every board the engine writes (the unpatched model ignores the unknown field; docs/ARCHITECTURE.md §18.2,
D59).

| Patch | Change | Upstream PR |
|---|---|---|
| `0001-theme-sync-egui-theme.patch` | `theme::apply` syncs egui's theme, so the OS light/dark switch no longer reverts the visuals; with a regression test | not opened yet |
| `0002-embed-js-api.patch` | feature `embed`: the JS API on `globalThis.inkboardEmbed` (§10.3, `src/embed.rs`), host mode (`App::host`), `host_load` resets, pause, single mount, Present and the host-owned menu entries hidden when hosted, `platform::init` split | not opened yet |
| `0003-optional-net.patch` | feature `net` (default): `inkboard-net` optional for the wasm build, `session_net_off.rs`; the public page keeps `net` (`web/Trunk.toml`, and the WebGPU build of `scripts/build_web.sh` / `build_web.ps1`, whose `--features` list replaces Trunk.toml's); droppable | not opened yet |
| `0004-docs-embedding.patch` | "Embedding (feature `embed`)" in `docs/WEB.md`, `docs/fr/WEB.md`, and the `CHANGELOG.md` entries | not opened yet |
| `0005-embed-documents.patch` | embedded documents (§18.3): `inkboard-core`'s optional `BoxObj.embed` (`Embed {item}`, lenient, `is_embed`); in `inkboard-app` the cards of a hosted board (`src/embed_cards.rs`): the per-frame layout report, `onEmbedOpen` instead of a card's label editing (none when read-only), the host's editing state (no keys, no handles, no style panel, ended by a primary or secondary press or the focus, a primary press swallowed, the middle button still panning), `insertEmbed`, titles in place, no rotation handle and no text settings for cards, the cards on the clipboard in the layout; the canvas reads the wheel's Shift and Alt from the wheel event (`src/tools.rs`); the JS side in `src/embed.rs`; native tests (`src/tests_embed.rs`, `cargo test -p inkboard-app embed`) | not opened yet |
| `0006-docs-embedded-documents.patch` | "Embedded documents (cards)" in `docs/WEB.md` and `docs/fr/WEB.md`, and the `CHANGELOG.md` entry | not opened yet |

The engine Wikanvas ships is built with `--no-default-features --features web-glow,embed` (no `net`). 0003 stays
droppable: without it the same build compiles `inkboard-net` in (a slightly larger `.wasm`, see Sizes), and host mode
still hides the Join entry.

`check-baseline.txt` holds the warnings of `check` on the unpatched base (rustc 1.99.1: clippy pedantic on the wasm
target, in `live_web.rs`, `platform_web.rs`, `session_net_web.rs`, `watch_web.rs` and `inkboard-net`; 0002 and 0003
change `platform_web.rs`). Each line is `<count><TAB><kind><TAB><file>: <message>`, without line and column, where
`<count>` is the most times that warning appears in one cargo run. `check` fails when a warning appears more often in
one of its cargo runs than the baseline's count (0 for a warning the baseline does not have), so a patch that moves code
adds nothing, and one more cast in `platform_web.rs` is still a new warning.

## Notices (Apache-2.0 §4(b))

Every file a patch changes gets one notice line near the top, in the file's comment syntax:

- `.rs`: `// Changed for Wikanvas (embed feature): see patches/inkboard/README.md`
- `.toml`: `# Changed for Wikanvas (embed feature): see patches/inkboard/README.md`
- `.md`: `<!-- Changed for Wikanvas (embed feature): see patches/inkboard/README.md -->`
- `.sh`, `.ps1`: `# Changed for Wikanvas (embed feature): see patches/inkboard/README.md` (after the `#!` line)

Inkboard's `LICENSE-MIT`, `LICENSE-APACHE` and `THIRD_PARTY_LICENSES.md` stay untouched and ship with the dist and the
image. The patches add no new `t()` strings (Inkboard's tests require a French entry for every string), so hiding UI
is preferred over adding text. `scripts/inkboard.sh export --upstream` writes the series without the notice lines to
`.cache/inkboard-upstream/` for upstream PRs.

## Working on the series

Always through `scripts/dev.sh` (the capped toolchain container). The script never runs git against Wikanvas's own
repository, so it works the same in the main tree, a worktree and the Docker build.

```sh
scripts/dev.sh scripts/inkboard.sh src       # fresh .cache/inkboard-src: copy of the submodule, `base` commit, git am of this series
# edit and commit inside .cache/inkboard-src (one commit per patch; `git commit --amend` / rebase as needed), then:
scripts/dev.sh scripts/inkboard.sh export    # regenerate patches/inkboard/*.patch (old files removed first)
scripts/dev.sh scripts/inkboard.sh check     # check + clippy for every feature set of the applied patches
scripts/dev.sh scripts/inkboard.sh build --quick   # iteration: release code without LTO, incremental, no wasm-opt (~2 min)
scripts/dev.sh scripts/inkboard.sh build     # web/dist-inkboard/, as the image ships it (10-20 min)
scripts/e2e.sh frame                         # the frame test (e2e/tests/frame.spec.ts) against web/dist-inkboard
scripts/dev.sh scripts/inkboard.sh size      # the numbers below (two release builds)
scripts/dev.sh scripts/inkboard.sh export --upstream   # the series without notice lines, in .cache/inkboard-upstream/
scripts/dev.sh scripts/inkboard.sh stub      # placeholder dist (engine_version "stub")
```

- `src` refuses to throw away work: uncommitted changes in `.cache/inkboard-src`, or commits that `export` has not
  written yet (`src --force` discards them). `build`, `check` and `size` run `src` themselves when the submodule or the
  patch files changed since the last `src` or `export`, and otherwise use the tree as it is (unexported commits
  included, which is how the series is iterated on).
- Subjects of the commits give the patch names (`format-patch`), and no commit message may contain the notice text.
- `check --record-baseline` rewrites `check-baseline.txt`; it runs only on the unpatched base (`HEAD` = `base`): for
  example `git -C .cache/inkboard-src checkout -q base`, the command, then `git -C .cache/inkboard-src checkout -q main`;
  or move the patch files away and run `src --force`.
- The feature sets of `check`: native default (`--all-targets`) and wasm `web-glow` always; wasm `web-glow,embed` with
  0002; wasm `--no-default-features --features web-glow,embed` and `--no-default-features --features web-glow,net`
  (the public page) with 0003.

## Dist layout (`web/dist-inkboard/`, git-ignored)

A flat directory that `wikanvas-assets` embeds and the server serves under `/inkboard/<build>/`:

- `build`: `frame.html`, `frame.js`, `frame.css` (from `web/inkboard-frame/`), `inkboard.js`, `inkboard_bg.wasm`,
  `icons.bin`, `ICONS-LICENSES.txt`, `LICENSE-MIT`, `LICENSE-APACHE`, `THIRD_PARTY_LICENSES.md`, with `.br` and `.gz`
  next to every `.wasm`, `.js`, `.html` and `.css` (never `icons.bin`), and `manifest.json`;
- `stub`: `frame.html` ("Inkboard engine not built") and `frame.js` (its `wikanvasBoardFrame.connect` reports
  `onError{kind:'unsupported'}` and rejects), each with `.br` and `.gz`, and `manifest.json`.

`manifest.json`:

```json
{
  "build": "0123456789ab",
  "engine_version": "0.2.0-beta.3+embed.wikanvas.3",
  "format_version": 2,
  "patchset": "3",
  "files": {
    "inkboard_bg.wasm": { "raw": 0, "br": 0, "gz": 0 },
    "icons.bin": { "raw": 0, "br": null, "gz": null }
  }
}
```

- `build`: the first 12 hex characters of the sha256 over the `sha256sum` lines of every file except `manifest.json`
  (precompressed variants included), sorted by name.
- `engine_version`: `0.2.0-beta.3+embed.wikanvas.<patchset>`; `"stub"` for the placeholder.
- `format_version`: `pub const VERSION: u32 = N;` from `vendor/inkboard/crates/inkboard-core/src/serial.rs`. The server
  refuses to start when it differs from `inkboard_core::serial::VERSION`.
- `patchset`: the id of this patch series, bumped by W5 with every change to it; `"stub"` for the placeholder.
- `files`: every file except `manifest.json` and the `.br`/`.gz` variants, with its raw size and the sizes of its
  variants (`null` when there is none).

## Sizes (`scripts/inkboard.sh size`)

The engine as Wikanvas builds it (`--no-default-features --features web-glow,embed`, release, `wasm-opt -Os`), measured
on 2026-10-28 with rustc 1.99.1, wasm-bindgen 0.2.129 and binaryen 131 (bytes):

| `.wasm` | raw | br | gz |
|---|---|---|---|
| patchset 3 (0001-0006), as shipped | 15,680,620 | 4,034,465 | 6,690,587 |
| patchset 2, with patch 0003 (no `net`) | 15,603,998 | 4,017,817 | 6,665,795 |
| patchset 2, without patch 0003 (`net` compiled in) | 15,849,407 | 4,093,719 | 6,767,612 |

The patchset-3 row is the `manifest.json` of `scripts/inkboard.sh build` (the same brotli 11 and gzip 9 as `size`):
0005 adds 76,622 bytes raw (0.5 %) and 16,648 br. The two patchset-2 rows are `size`'s (not rerun for patchset 3).

0003 saves only about 1.6 %: the `embed` build's `main` never installs the guest factory, so the linker already drops
most of the session network. Its gains are elsewhere: iroh and `ring` are not compiled at all (no C compiler for the
wasm target, a shorter build), and the embedded engine cannot open a network connection. For comparison, Inkboard's
own page (with `net`) is about 18 MB raw (4.7 MB br) per its `docs/WEB.md`.
