# Third-party software

Quire is licensed under the MIT License (`LICENSE`).
It builds on the software below, each under its own licence.

**This list is not complete yet.** Workstream W7 (docs/PLAN.md) completes it before the first release: every crate in
the workspace `Cargo.lock` (so it also covers the standalone `quire` CLI), the npm packages bundled into `web/dist`,
and every piece of code borrowed under PLAN rule R9, each with its version, licence and source.

## Inkboard

- **What:** the whiteboard engine behind Quire's boards. `inkboard-core` is compiled into `quire-server`; the Inkboard
  web app is compiled to WebAssembly and served from `/inkboard/` inside Quire's board frame.
- **Source:** `vendor/inkboard`, a git submodule of <https://github.com/VictorLetirand/inkboard> at commit `41e0295`
  (version 0.2.0-beta.3). Quire never modifies the submodule: its changes are the patch series in `patches/inkboard/`,
  and every file a patch changes carries a "Changed for Quire" notice line.
- **Licence:** MIT OR Apache-2.0. Copyright (c) 2026 The Inkboard contributors. The licence texts are
  `vendor/inkboard/LICENSE-MIT` and `vendor/inkboard/LICENSE-APACHE`; the image ships them in
  `/usr/share/doc/quire/inkboard/`.
- **Its own third-party components:** `vendor/inkboard/THIRD_PARTY_LICENSES.md` (libraries and fonts) and
  `vendor/inkboard/crates/inkboard-app/assets/ICONS-LICENSES.txt` (the icon library). Both are part of the engine's
  dist (`web/dist-inkboard`, embedded in `quire-server`), and the image also ships `THIRD_PARTY_LICENSES.md` in
  `/usr/share/doc/quire/inkboard/`.
