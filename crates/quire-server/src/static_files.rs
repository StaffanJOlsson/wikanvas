//! Static serving of the embedded web and Inkboard files with precompressed variants (owner: W2).
//!
//! [`Site`] is the fallback of every path outside `/api/` and `/healthz` (§11). It is generic over an
//! [`AssetSource`]: [`Embedded`] (rust-embed, `quire-assets`) in production, [`MemoryAssets`] in tests.
//! - `/assets/*` (Vite-hashed): `public, max-age=31536000, immutable`, br / gzip / identity.
//! - `index.html` and every SPA route (a path without an extension outside `/assets` and `/inkboard`): `no-cache`,
//!   br / gzip / identity, the SPA CSP, `X-Frame-Options: DENY` and the HTML headers.
//! - Other files at the root (`/theme-init.js`, `/favicon.svg`): `no-cache`, identity.
//! - `/inkboard/{build}/…` for the embedded manifest's `build` only: immutable, br / gzip / identity (`.wasm` is
//!   `application/wasm`), the frame CSP and `X-Frame-Options: SAMEORIGIN`; `icons.bin` identity only. Any other build
//!   is 404, so a page left open across an upgrade asks for the new one.
//! - A variant is picked from `Accept-Encoding` (br, then gzip, then identity) with `Vary: Accept-Encoding`; its bytes
//!   are the embedded `.br` / `.gz` file unchanged. The ETag is the embedded sha256 of the chosen variant, and a
//!   matching `If-None-Match` gets 304. Unknown paths with an extension are 404.
//!
//! At startup [`Site::new`] reads the embedded `manifest.json` (§10.5): missing → `config.inkboard = null` (boards
//! open in preview mode); a `format_version` other than `inkboard_core::serial::VERSION`, or a manifest that does not
//! parse, refuses to start. The stub dist (`engine_version: "stub"`) also gives `inkboard: null`.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use quire_api::{InkboardInfo, WasmBytes};
use quire_assets::Asset;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::headers::{FRAME_CSP, SPA_CSP, html};

/// Where the static files come from.
pub trait AssetSource: Send + Sync + 'static {
    /// A file of `web/dist` by its path (no leading `/`).
    fn web(&self, path: &str) -> Option<Asset>;
    /// A file of `web/dist-inkboard` by its path (no leading `/`).
    fn inkboard(&self, path: &str) -> Option<Asset>;
}

/// The files embedded in the binary (`quire-assets`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Embedded;

impl AssetSource for Embedded {
    fn web(&self, path: &str) -> Option<Asset> {
        quire_assets::web(path)
    }

    fn inkboard(&self, path: &str) -> Option<Asset> {
        quire_assets::inkboard(path)
    }
}

/// An in-memory [`AssetSource`] for tests.
#[derive(Clone, Debug, Default)]
pub struct MemoryAssets {
    web: HashMap<String, Asset>,
    inkboard: HashMap<String, Asset>,
}

fn memory_asset(path: &str, bytes: Vec<u8>) -> Asset {
    Asset {
        mime: mime_for(path)
            .unwrap_or("application/octet-stream")
            .to_owned(),
        sha256: Sha256::digest(&bytes).into(),
        data: Cow::Owned(bytes),
    }
}

impl MemoryAssets {
    pub fn new() -> MemoryAssets {
        MemoryAssets::default()
    }

    /// Adds a `web/dist` file.
    pub fn with_web(mut self, path: &str, bytes: impl Into<Vec<u8>>) -> MemoryAssets {
        self.web
            .insert(path.to_owned(), memory_asset(path, bytes.into()));
        self
    }

    /// Adds a `web/dist-inkboard` file.
    pub fn with_inkboard(mut self, path: &str, bytes: impl Into<Vec<u8>>) -> MemoryAssets {
        self.inkboard
            .insert(path.to_owned(), memory_asset(path, bytes.into()));
        self
    }
}

impl AssetSource for MemoryAssets {
    fn web(&self, path: &str) -> Option<Asset> {
        self.web.get(path).cloned()
    }

    fn inkboard(&self, path: &str) -> Option<Asset> {
        self.inkboard.get(path).cloned()
    }
}

/// `manifest.json` of `web/dist-inkboard` (§10.5, `patches/inkboard/README.md`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Manifest {
    /// The first 12 hex digits of the dist's hash: the `{build}` of `/inkboard/{build}/`.
    pub build: String,
    /// `0.2.0-beta.3+embed.quire.<patchset>`, or `stub`.
    pub engine_version: String,
    pub format_version: u32,
    pub patchset: String,
    /// Every file but the manifest and the variants, with the sizes of the file and its variants.
    pub files: BTreeMap<String, FileSizes>,
}

/// The sizes of one dist file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub struct FileSizes {
    pub raw: u64,
    pub br: Option<u64>,
    pub gz: Option<u64>,
}

/// The static site: the asset source and the Inkboard manifest read from it.
pub struct Site {
    source: Arc<dyn AssetSource>,
    manifest: Option<Manifest>,
}

impl std::fmt::Debug for Site {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Site")
            .field("manifest", &self.manifest)
            .finish_non_exhaustive()
    }
}

impl Site {
    /// Reads and checks the manifest (see the module docs).
    pub fn new(source: Arc<dyn AssetSource>) -> Result<Site, String> {
        let manifest = match source.inkboard("manifest.json") {
            None => None,
            Some(asset) => {
                let m: Manifest = serde_json::from_slice(&asset.data)
                    .map_err(|e| format!("the embedded manifest.json does not parse: {e}"))?;
                if m.format_version != inkboard_core::serial::VERSION {
                    return Err(format!(
                        "the embedded Inkboard build writes format_version {}, this server reads {}; rebuild it \
                         (scripts/inkboard.sh build)",
                        m.format_version,
                        inkboard_core::serial::VERSION
                    ));
                }
                if m.build.is_empty() || !m.build.bytes().all(|b| b.is_ascii_alphanumeric()) {
                    return Err(format!(
                        "the embedded manifest has a bad build {:?}",
                        m.build
                    ));
                }
                Some(m)
            }
        };
        Ok(Site { source, manifest })
    }

    /// The embedded Inkboard manifest, if any.
    pub fn manifest(&self) -> Option<&Manifest> {
        self.manifest.as_ref()
    }

    /// `config.inkboard`: `None` without a manifest, for the stub build, or without a `.wasm` file.
    pub fn inkboard_info(&self) -> Option<InkboardInfo> {
        let m = self.manifest.as_ref()?;
        if m.engine_version == "stub" {
            return None;
        }
        let (_, wasm) = m.files.iter().find(|(name, _)| name.ends_with(".wasm"))?;
        Some(InkboardInfo {
            base: format!("/inkboard/{}/", m.build),
            engine_version: m.engine_version.clone(),
            format_version: m.format_version,
            wasm_bytes: WasmBytes {
                raw: wasm.raw,
                br: wasm.br.unwrap_or(0),
                gz: wasm.gz.unwrap_or(0),
            },
        })
    }

    /// Answers a request for a static path (see the module docs).
    pub fn serve(&self, method: &Method, uri: &Uri, headers: &HeaderMap) -> Response {
        if method != Method::GET && method != Method::HEAD {
            let mut res = (StatusCode::METHOD_NOT_ALLOWED, "method not allowed\n").into_response();
            res.headers_mut()
                .insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
            return res;
        }
        let path = uri.path().trim_start_matches('/');
        if path.split('/').any(|seg| seg == ".." || seg == ".")
            || path.contains('\\')
            || path.ends_with(".br")
            || path.ends_with(".gz")
        {
            return not_found();
        }
        if let Some(rest) = path.strip_prefix("inkboard/") {
            return self.inkboard_file(rest, headers).unwrap_or_else(not_found);
        }
        if path.starts_with("assets/") {
            let web = |p: &str| self.source.web(p);
            return file_response(&web, path, headers, Encodings::All, IMMUTABLE)
                .unwrap_or_else(not_found);
        }
        if !path.is_empty() && path != "index.html" {
            let web = |p: &str| self.source.web(p);
            if let Some(res) = file_response(&web, path, headers, Encodings::Identity, NO_CACHE) {
                return res;
            }
        }
        let last = path.rsplit('/').next().unwrap_or("");
        if last.contains('.') && path != "index.html" {
            return not_found();
        }
        let web = |p: &str| self.source.web(p);
        match file_response(&web, "index.html", headers, Encodings::All, NO_CACHE) {
            Some(mut res) => {
                let h = res.headers_mut();
                html(h);
                h.insert(
                    header::CONTENT_SECURITY_POLICY,
                    HeaderValue::from_static(SPA_CSP),
                );
                h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
                res
            }
            None => (
                StatusCode::NOT_FOUND,
                "the web client is not built (scripts/dev.sh pnpm --dir web build)\n",
            )
                .into_response(),
        }
    }

    fn inkboard_file(&self, rest: &str, headers: &HeaderMap) -> Option<Response> {
        let manifest = self.manifest.as_ref()?;
        let (build, file) = rest.split_once('/')?;
        if build != manifest.build || file.is_empty() || file == "manifest.json" {
            return None;
        }
        let get = |p: &str| self.source.inkboard(p);
        let icons = file == "icons.bin" || file.ends_with("/icons.bin");
        let encodings = if icons {
            Encodings::Identity
        } else {
            Encodings::All
        };
        let mut res = file_response(&get, file, headers, encodings, IMMUTABLE)?;
        let h = res.headers_mut();
        if icons {
            h.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            );
        } else {
            h.insert(
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static(FRAME_CSP),
            );
            h.insert(
                header::X_FRAME_OPTIONS,
                HeaderValue::from_static("SAMEORIGIN"),
            );
        }
        if h.get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/html"))
        {
            html(h);
        }
        Some(res)
    }
}

const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const NO_CACHE: &str = "no-cache";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Encodings {
    /// br, gzip or identity.
    All,
    /// Identity only.
    Identity,
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "not found\n").into_response()
}

/// The content type of a static file by its extension.
pub fn mime_for(path: &str) -> Option<&'static str> {
    let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "wasm" => "application/wasm",
        "json" | "map" => "application/json",
        "webmanifest" => "application/manifest+json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "txt" => "text/plain; charset=utf-8",
        "md" => "text/markdown; charset=utf-8",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "bin" | "inkboard" => "application/octet-stream",
        _ => return None,
    })
}

/// Whether `Accept-Encoding` offers `coding` with a non-zero q-value.
pub fn accepts(headers: &HeaderMap, coding: &str) -> bool {
    headers
        .get_all(header::ACCEPT_ENCODING)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|item| {
            let mut parts = item.split(';');
            let name = parts.next().unwrap_or("").trim();
            if !name.eq_ignore_ascii_case(coding) {
                return false;
            }
            let q = parts
                .filter_map(|p| p.trim().strip_prefix("q="))
                .find_map(|q| q.trim().parse::<f32>().ok())
                .unwrap_or(1.0);
            q > 0.0
        })
}

fn etag_of(asset: &Asset) -> String {
    let hex: String = asset.sha256[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("\"{hex}\"")
}

fn matches_etag(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|t| t.trim().trim_start_matches("W/"))
        .any(|t| t == etag || t == "*")
}

/// The response for file `name` from `get`, with the chosen variant; `None` when the file does not exist.
fn file_response(
    get: &dyn Fn(&str) -> Option<Asset>,
    name: &str,
    headers: &HeaderMap,
    encodings: Encodings,
    cache: &'static str,
) -> Option<Response> {
    let identity = get(name)?;
    let mime = mime_for(name)
        .map(str::to_owned)
        .unwrap_or_else(|| identity.mime.clone());
    let (asset, encoding) = match encodings {
        Encodings::Identity => (identity, None),
        Encodings::All => {
            let br = accepts(headers, "br")
                .then(|| get(&format!("{name}.br")))
                .flatten();
            match br {
                Some(a) => (a, Some("br")),
                None => match accepts(headers, "gzip")
                    .then(|| get(&format!("{name}.gz")))
                    .flatten()
                {
                    Some(a) => (a, Some("gzip")),
                    None => (identity, None),
                },
            }
        }
    };
    let etag = etag_of(&asset);
    let mut res = if matches_etag(headers, &etag) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        let mut res = Response::new(Body::from(asset.data.into_owned()));
        if let Some(enc) = encoding {
            res.headers_mut()
                .insert(header::CONTENT_ENCODING, HeaderValue::from_static(enc));
        }
        if let Ok(v) = HeaderValue::from_str(&mime) {
            res.headers_mut().insert(header::CONTENT_TYPE, v);
        }
        res
    };
    let h = res.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    if let Ok(v) = HeaderValue::from_str(&etag) {
        h.insert(header::ETAG, v);
    }
    if encodings == Encodings::All {
        h.insert(header::VARY, HeaderValue::from_static("accept-encoding"));
    }
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Some(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_encoding_parsing() {
        let h = |v: &'static str| {
            let mut m = HeaderMap::new();
            m.insert(header::ACCEPT_ENCODING, HeaderValue::from_static(v));
            m
        };
        assert!(accepts(&h("gzip, deflate, br"), "br"));
        assert!(accepts(&h("gzip, deflate, br"), "gzip"));
        assert!(!accepts(&h("gzip, br;q=0"), "br"));
        assert!(accepts(&h("BR;q=0.5"), "br"));
        assert!(!accepts(&h("identity"), "gzip"));
        assert!(!accepts(&HeaderMap::new(), "gzip"));
    }

    #[test]
    fn manifest_checks() {
        let manifest = |fv: u32, ev: &str| {
            format!(
                r#"{{"build":"0123456789ab","engine_version":"{ev}","format_version":{fv},"patchset":"1",
                   "files":{{"inkboard_bg.wasm":{{"raw":10,"br":4,"gz":5}},"icons.bin":{{"raw":3,"br":null,"gz":null}}}}}}"#
            )
        };
        let v = inkboard_core::serial::VERSION;
        let site = |m: String| {
            Site::new(Arc::new(
                MemoryAssets::new().with_inkboard("manifest.json", m),
            ))
        };
        let ok = site(manifest(v, "0.2.0-beta.3+embed.quire.1")).unwrap();
        let info = ok.inkboard_info().unwrap();
        assert_eq!(info.base, "/inkboard/0123456789ab/");
        assert_eq!(
            (info.wasm_bytes.raw, info.wasm_bytes.br, info.wasm_bytes.gz),
            (10, 4, 5)
        );
        assert!(site(manifest(v, "stub")).unwrap().inkboard_info().is_none());
        let err = site(manifest(v + 1, "x")).unwrap_err();
        assert!(err.contains("format_version"), "{err}");
        assert!(site("{".into()).is_err());
        assert!(
            Site::new(Arc::new(MemoryAssets::new()))
                .unwrap()
                .inkboard_info()
                .is_none()
        );
    }

    #[test]
    fn mime_types() {
        assert_eq!(mime_for("a/b.wasm"), Some("application/wasm"));
        assert_eq!(mime_for("index.html"), Some("text/html; charset=utf-8"));
        assert_eq!(mime_for("x.JS"), Some("text/javascript; charset=utf-8"));
        assert_eq!(mime_for("noext"), None);
    }
}
