# Inkboard patches for Quire

Quire embeds [Inkboard](https://github.com/VictorLetirand/inkboard) (MIT OR Apache-2.0) as a WebAssembly whiteboard
engine. The submodule `vendor/inkboard` is never modified and never built in place. Quire's changes live here as a
patch series, applied to a self-contained copy in `.cache/inkboard-src` by `scripts/inkboard.sh`. The design is in
`docs/ARCHITECTURE.md` §10.

## Base

| | |
|---|---|
| Upstream | `https://github.com/VictorLetirand/inkboard.git` |
| Commit | `41e0295d50433faa5d05b2ff48f85eac079a981d` (tag `v0.2.0-beta.3`, merge of PR #31, 2026-10-27) |
| Workspace version | `0.2.0-beta.3` (`vendor/inkboard/Cargo.toml`; `scripts/inkboard.sh src` refuses any other) |
| Board format | `inkboard_core::serial::VERSION = 2` (`crates/inkboard-core/src/serial.rs`) |
| Patchset | `1` |

`scripts/check-pins.sh` (on the host) checks that the submodule is at this commit and clean, and that the crates Quire
shares with `inkboard-core` (serde, serde_json, base64, thiserror) have the same versions in both lockfiles.

## Patch series

`git format-patch` output against the base copy (`base` commit in `.cache/inkboard-src`), applied in order with
`git am --no-3way`. Nothing under `inkboard-core` or `inkboard-session` changes, so the server compiles those crates
unpatched from the same commit as the wasm.

| Patch | Change | Upstream PR |
|---|---|---|
| `0001-theme-sync-egui-theme.patch` | `theme::apply` syncs egui's theme, so the OS light/dark switch no longer reverts the visuals | not opened yet |
| `0002-embed-js-api.patch` | feature `embed`: the JS API on `globalThis.inkboardEmbed` (§10.3), host mode, `host_load` resets, pause, single mount, Present hidden when hosted | not opened yet |
| `0003-optional-net.patch` | feature `net`: `inkboard-net` optional for the wasm build; droppable | not opened yet |
| `0004-docs-embedding.patch` | "Embedding (feature `embed`)" in `docs/WEB.md`, `docs/fr/WEB.md` and `CHANGELOG.md` | not opened yet |

No patch exists yet: W5 writes the series (PLAN W5). Until then `scripts/inkboard.sh stub` provides a placeholder dist.

## Notices (Apache-2.0 §4(b))

Every file a patch changes gets one notice line near the top, in the file's comment syntax:

- `.rs`: `// Changed for Quire (embed feature): see patches/inkboard/README.md`
- `.toml`: `# Changed for Quire (embed feature): see patches/inkboard/README.md`
- `.md`: `<!-- Changed for Quire (embed feature): see patches/inkboard/README.md -->`

Inkboard's `LICENSE-MIT`, `LICENSE-APACHE` and `THIRD_PARTY_LICENSES.md` stay untouched and ship with the dist and the
image. The patches add no new `t()` strings (Inkboard's tests require a French entry for every string), so hiding UI
is preferred over adding text. `scripts/inkboard.sh export --upstream` writes the series without the notice lines to
`.cache/inkboard-upstream/` for upstream PRs.

## Working on the series

Always through `scripts/dev.sh` (the capped toolchain container). The script never runs git against Quire's own
repository, so it works the same in the main tree, a worktree and the Docker build.

```sh
scripts/dev.sh scripts/inkboard.sh src       # fresh .cache/inkboard-src: copy of the submodule, `base` commit, git am of this series
# edit and commit inside .cache/inkboard-src, then:
scripts/dev.sh scripts/inkboard.sh export    # regenerate patches/inkboard/*.patch (old files removed first)
scripts/dev.sh scripts/inkboard.sh check     # check + clippy for every feature set of the applied patches
scripts/dev.sh scripts/inkboard.sh build     # web/dist-inkboard/
scripts/dev.sh scripts/inkboard.sh size      # the numbers below
scripts/dev.sh scripts/inkboard.sh stub      # placeholder dist (engine_version "stub")
```

`src`, `build`, `export`, `check` and `size` print "not implemented (W5)" until W5 implements them; `stub` works.

## Dist layout (`web/dist-inkboard/`, git-ignored)

A flat directory that `quire-assets` embeds and the server serves under `/inkboard/<build>/`:

- `build`: `frame.html`, `frame.js`, `frame.css` (from `web/inkboard-frame/`), `inkboard.js`, `inkboard_bg.wasm`,
  `icons.bin`, `ICONS-LICENSES.txt`, `LICENSE-MIT`, `LICENSE-APACHE`, `THIRD_PARTY_LICENSES.md`, with `.br` and `.gz`
  next to every `.wasm`, `.js`, `.html` and `.css` (never `icons.bin`), and `manifest.json`;
- `stub`: `frame.html` ("Inkboard engine not built") and `frame.js` (its `quireBoardFrame.connect` reports
  `onError{kind:'unsupported'}` and rejects), each with `.br` and `.gz`, and `manifest.json`.

`manifest.json`:

```json
{
  "build": "0123456789ab",
  "engine_version": "0.2.0-beta.3+embed.quire.1",
  "format_version": 2,
  "patchset": "1",
  "files": {
    "inkboard_bg.wasm": { "raw": 0, "br": 0, "gz": 0 },
    "icons.bin": { "raw": 0, "br": null, "gz": null }
  }
}
```

- `build`: the first 12 hex characters of the sha256 over the `sha256sum` lines of every file except `manifest.json`
  (precompressed variants included), sorted by name.
- `engine_version`: `0.2.0-beta.3+embed.quire.<patchset>`; `"stub"` for the placeholder.
- `format_version`: `pub const VERSION: u32 = N;` from `vendor/inkboard/crates/inkboard-core/src/serial.rs`. The server
  refuses to start when it differs from `inkboard_core::serial::VERSION`.
- `patchset`: the id of this patch series, bumped by W5 with every change to it; `"stub"` for the placeholder.
- `files`: every file except `manifest.json` and the `.br`/`.gz` variants, with its raw size and the sizes of its
  variants (`null` when there is none).

## Sizes (`scripts/inkboard.sh size`)

| `.wasm` | raw | br | gz |
|---|---|---|---|
| with patch 0003 (no `net`) | not measured yet | | |
| without patch 0003 | not measured yet | | |

W5 records the numbers when the series builds.
