//! Embedded static files: Web (web/dist) and Inkboard (web/dist-inkboard), allow_missing (owner: W2)
//!
//! Release builds embed the files; debug builds read them from disk at run time (rust-embed's default), so a
//! `pnpm build` shows up without recompiling. A missing folder embeds nothing (`allow_missing`), so the server
//! builds before the web and Inkboard builds have run. The folders are relative to this crate's manifest directory
//! (rust-embed resolves them against `CARGO_MANIFEST_DIR`; `interpolate-folder-path` is not enabled).

use std::borrow::Cow;

use rust_embed::RustEmbed;

/// `web/dist`: the SPA and its precompressed `.br` / `.gz` variants.
#[derive(RustEmbed)]
#[folder = "../../web/dist"]
#[allow_missing = true]
pub struct Web;

/// `web/dist-inkboard`: a flat folder of `manifest.json` and the engine files (`frame.html`, `*.js`, `*.css`,
/// `*.wasm`, `icons.bin` and their `.br` / `.gz` copies, §10.5), served under `/inkboard/{build}/` (§11).
#[derive(RustEmbed)]
#[folder = "../../web/dist-inkboard"]
#[allow_missing = true]
pub struct Inkboard;

/// One embedded file.
#[derive(Clone, Debug)]
pub struct Asset {
    pub data: Cow<'static, [u8]>,
    /// Guessed from the extension (`application/octet-stream` when unknown).
    pub mime: String,
    /// sha256 of `data` (the static ETag, §11).
    pub sha256: [u8; 32],
}

fn convert(file: rust_embed::EmbeddedFile) -> Asset {
    Asset {
        mime: file.metadata.mimetype().to_owned(),
        sha256: file.metadata.sha256_hash(),
        data: file.data,
    }
}

/// A file of `web/dist` by its path relative to the folder (no leading `/`).
pub fn web(path: &str) -> Option<Asset> {
    Web::get(path).map(convert)
}

/// A file of `web/dist-inkboard` by its path relative to the folder (no leading `/`).
pub fn inkboard(path: &str) -> Option<Asset> {
    Inkboard::get(path).map(convert)
}

/// Every path of `web/dist`.
pub fn web_paths() -> impl Iterator<Item = Cow<'static, str>> {
    Web::iter()
}

/// Every path of `web/dist-inkboard`.
pub fn inkboard_paths() -> impl Iterator<Item = Cow<'static, str>> {
    Inkboard::iter()
}
