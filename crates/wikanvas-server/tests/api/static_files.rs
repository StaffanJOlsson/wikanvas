//! Static serving and header tests (owner: W2)
//!
//! The embedded files are replaced by an in-memory `AssetSource`, so these tests need no web or Inkboard build.

use std::sync::Arc;

use quire_api::{Config as ApiConfig, ErrorCode};
use quire_server::headers::{FRAME_CSP, SPA_CSP};
use quire_server::static_files::MemoryAssets;

use crate::common::{TestApp, assert_error};

const BUILD: &str = "0123456789ab";

fn manifest(build: &str, engine: &str, format: u32) -> String {
    format!(
        r#"{{"build":"{build}","engine_version":"{engine}","format_version":{format},"patchset":"1",
            "files":{{"frame.html":{{"raw":20,"br":7,"gz":9}},"inkboard_bg.wasm":{{"raw":8,"br":5,"gz":6}},
                     "icons.bin":{{"raw":5,"br":null,"gz":null}}}}}}"#
    )
}

fn site(manifest_json: Option<String>) -> MemoryAssets {
    let mut assets = MemoryAssets::new()
        .with_web("index.html", "<!doctype html><title>Quire</title>")
        .with_web("index.html.br", "BR index")
        .with_web("index.html.gz", "GZ index")
        .with_web("assets/app-abc123.js", "console.log('identity')")
        .with_web("assets/app-abc123.js.br", "BR bytes, byte for byte")
        .with_web("assets/app-abc123.js.gz", "GZ bytes")
        .with_web("assets/style-def456.css", "body{}")
        .with_web("theme-init.js", "/* theme */")
        .with_web("theme-init.js.gz", "GZ theme")
        .with_web("favicon.svg", "<svg/>")
        .with_inkboard("frame.html", "<!doctype html><title>frame</title>")
        .with_inkboard("frame.html.br", "BR frame")
        .with_inkboard("frame.html.gz", "GZ frame")
        .with_inkboard("inkboard_bg.wasm", b"\0asm\x01\0\0\0".to_vec())
        .with_inkboard("inkboard_bg.wasm.br", "BR wasm")
        .with_inkboard("inkboard_bg.wasm.gz", "GZ wasm")
        .with_inkboard("icons.bin", "icons")
        .with_inkboard("icons.bin.br", "never served");
    if let Some(m) = manifest_json {
        assets = assets.with_inkboard("manifest.json", m);
    }
    assets
}

async fn app() -> TestApp {
    let mut app = TestApp::new().await;
    app.set_assets(site(Some(manifest(
        BUILD,
        "0.2.0-beta.3+embed.quire.1",
        inkboard_core::serial::VERSION,
    ))));
    app
}

#[tokio::test]
async fn spa_fallback_vs_api_json_404() {
    let app = app().await;
    for path in ["/", "/c/notes-abc", "/settings/tokens", "/index.html"] {
        let res = app.server.get(path).await;
        res.assert_status_ok();
        assert_eq!(res.text(), "<!doctype html><title>Quire</title>", "{path}");
        assert_eq!(res.header("content-type"), "text/html; charset=utf-8");
        assert_eq!(res.header("cache-control"), "no-cache");
    }
    for path in ["/api/v1/nope", "/api/nope", "/api", "/api/v2/items"] {
        assert_error(&app.server.get(path).await, 404, ErrorCode::NotFound);
    }
}

#[tokio::test]
async fn extension_404() {
    let app = app().await;
    for path in [
        "/missing.js",
        "/c/route.png",
        "/assets/missing-000.js",
        "/assets/no-extension",
        "/assets/app-abc123.js.br",
        "/../Cargo.toml",
    ] {
        let res = app.server.get(path).await;
        assert_eq!(res.status_code(), 404, "{path}");
        assert!(
            !res.content_type().starts_with("application/json"),
            "{path}"
        );
    }
    let res = app
        .server
        .post("/")
        .add_header("sec-fetch-site", "same-origin")
        .await;
    assert_eq!(res.status_code(), 405);
}

#[tokio::test]
async fn br_then_gzip_then_identity_with_vary() {
    let app = app().await;
    let get = |accept: Option<&'static str>| {
        let mut req = app.server.get("/assets/app-abc123.js");
        if let Some(a) = accept {
            req = req.add_header("accept-encoding", a);
        }
        req
    };
    let br = get(Some("gzip, deflate, br")).await;
    br.assert_status_ok();
    assert_eq!(
        br.as_bytes().as_ref(),
        b"BR bytes, byte for byte",
        "the embedded .br file, unchanged"
    );
    assert_eq!(br.header("content-encoding"), "br");
    assert_eq!(br.header("content-type"), "text/javascript; charset=utf-8");
    assert_eq!(br.header("vary"), "accept-encoding");
    assert_eq!(
        br.header("cache-control"),
        "public, max-age=31536000, immutable"
    );
    let gz = get(Some("gzip, br;q=0")).await;
    assert_eq!(gz.as_bytes().as_ref(), b"GZ bytes");
    assert_eq!(gz.header("content-encoding"), "gzip");
    let plain = get(None).await;
    assert_eq!(plain.text(), "console.log('identity')");
    assert!(plain.maybe_header("content-encoding").is_none());
    assert_eq!(plain.header("vary"), "accept-encoding");
    // ETags are per variant; a matching If-None-Match is 304.
    let (tag_br, tag_plain) = (br.header("etag"), plain.header("etag"));
    assert_ne!(tag_br, tag_plain);
    let res = get(Some("br"))
        .add_header("if-none-match", tag_br.clone())
        .await;
    assert_eq!(res.status_code(), 304);
    assert_eq!(res.header("etag"), tag_br);
    let res = get(None).add_header("if-none-match", tag_br).await;
    res.assert_status_ok();
    // A file without variants is identity; the SPA index has its own.
    let css = app
        .server
        .get("/assets/style-def456.css")
        .add_header("accept-encoding", "br")
        .await;
    assert_eq!(css.text(), "body{}");
    let index = app
        .server
        .get("/")
        .add_header("accept-encoding", "br")
        .await;
    assert_eq!(index.text(), "BR index");
    // Root files outside /assets are no-cache and identity only.
    let theme = app
        .server
        .get("/theme-init.js")
        .add_header("accept-encoding", "gzip")
        .await;
    assert_eq!(theme.text(), "/* theme */");
    assert_eq!(theme.header("cache-control"), "no-cache");
    assert!(theme.maybe_header("content-encoding").is_none());
    let icon = app.server.get("/favicon.svg").await;
    assert_eq!(icon.header("content-type"), "image/svg+xml");
}

#[tokio::test]
async fn compression_only_for_api_json() {
    let app = TestApp::with_owner().await;
    let res = app
        .server
        .get("/api/v1/openapi.json")
        .add_header("accept-encoding", "gzip")
        .await;
    res.assert_status_ok();
    assert_eq!(
        res.header("content-encoding"),
        "gzip",
        "openapi.json is over 1 KiB"
    );
    let small = app
        .server
        .get("/api/v1/auth/me")
        .add_header("accept-encoding", "gzip")
        .await;
    assert!(
        small.maybe_header("content-encoding").is_none(),
        "under 1 KiB"
    );
    assert_eq!(small.header("cache-control"), "no-store");
}

#[tokio::test]
async fn wasm_content_type_and_immutable() {
    let app = app().await;
    let url = format!("/inkboard/{BUILD}/inkboard_bg.wasm");
    let res = app
        .server
        .get(&url)
        .add_header("accept-encoding", "br, gzip")
        .await;
    res.assert_status_ok();
    assert_eq!(res.header("content-type"), "application/wasm");
    assert_eq!(res.header("content-encoding"), "br");
    assert_eq!(res.as_bytes().as_ref(), b"BR wasm");
    assert_eq!(
        res.header("cache-control"),
        "public, max-age=31536000, immutable"
    );
    let res = app.server.get(&url).await;
    assert_eq!(res.as_bytes().as_ref(), b"\0asm\x01\0\0\0");
    assert_eq!(res.header("content-type"), "application/wasm");
}

#[tokio::test]
async fn icons_bin_identity_only() {
    let app = app().await;
    let res = app
        .server
        .get(&format!("/inkboard/{BUILD}/icons.bin"))
        .add_header("accept-encoding", "br, gzip")
        .await;
    res.assert_status_ok();
    assert_eq!(res.text(), "icons");
    assert!(res.maybe_header("content-encoding").is_none());
    assert_eq!(res.header("content-type"), "application/octet-stream");
    assert_eq!(
        res.header("cache-control"),
        "public, max-age=31536000, immutable"
    );
}

fn assert_html_headers(res: &axum_test::TestResponse) {
    for (name, value) in [
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "same-origin"),
        ("cross-origin-opener-policy", "same-origin"),
        ("cross-origin-resource-policy", "same-origin"),
        (
            "permissions-policy",
            "camera=(), microphone=(), geolocation=(), payment=()",
        ),
    ] {
        assert_eq!(res.header(name), value, "{name}");
    }
}

#[tokio::test]
async fn spa_csp_and_frame_csp() {
    let app = app().await;
    let spa = app.server.get("/c/anything").await;
    assert_eq!(spa.header("content-security-policy"), SPA_CSP);
    assert_eq!(spa.header("x-frame-options"), "DENY");
    assert_html_headers(&spa);
    assert!(
        SPA_CSP.contains("frame-ancestors 'none'")
            && SPA_CSP.contains("style-src-attr 'unsafe-inline'")
    );
    let frame = app
        .server
        .get(&format!("/inkboard/{BUILD}/frame.html"))
        .await;
    frame.assert_status_ok();
    assert_eq!(frame.header("content-type"), "text/html; charset=utf-8");
    assert_eq!(frame.header("content-security-policy"), FRAME_CSP);
    assert_eq!(frame.header("x-frame-options"), "SAMEORIGIN");
    assert_html_headers(&frame);
    assert!(
        FRAME_CSP.contains("'wasm-unsafe-eval'") && FRAME_CSP.contains("frame-ancestors 'self'")
    );
}

/// The frame page fetches `manifest.json` next to `frame.html` before it downloads the engine (`frame.js`
/// `loadManifest`); without it every board fails to connect with `download`.
#[tokio::test]
async fn inkboard_manifest_served_next_to_the_frame() {
    let app = app().await;
    let url = format!("/inkboard/{BUILD}/manifest.json");
    let res = app
        .server
        .get(&url)
        .add_header("accept-encoding", "br, gzip")
        .await;
    res.assert_status_ok();
    assert_eq!(res.header("content-type"), "application/json");
    assert!(res.maybe_header("content-encoding").is_none(), "identity");
    assert_eq!(res.header("cache-control"), "no-cache");
    assert_eq!(res.header("x-content-type-options"), "nosniff");
    let served: serde_json::Value = res.json();
    let embedded: serde_json::Value = serde_json::from_str(&manifest(
        BUILD,
        "0.2.0-beta.3+embed.quire.1",
        inkboard_core::serial::VERSION,
    ))
    .unwrap();
    assert_eq!(served, embedded);
    assert_eq!(served["files"]["inkboard_bg.wasm"]["raw"], 8);
    // Revalidated with its ETag.
    let res = app
        .server
        .get(&url)
        .add_header("if-none-match", res.header("etag"))
        .await;
    assert_eq!(res.status_code(), 304);
    // Another build's manifest is 404 like its other files.
    let res = app.server.get("/inkboard/ffffffffffff/manifest.json").await;
    assert_eq!(res.status_code(), 404);
}

/// A manifest over 1 KiB keeps its bytes: the compression layer never re-encodes a static file.
#[tokio::test]
async fn large_inkboard_manifest_is_not_gzipped() {
    let mut app = TestApp::new().await;
    let mut files = String::new();
    for i in 0..60 {
        files.push_str(&format!(
            r#""chunk-{i:02}.js":{{"raw":100,"br":40,"gz":50}},"#
        ));
    }
    let big = format!(
        r#"{{"build":"{BUILD}","engine_version":"0.2.0-beta.3+embed.quire.1","format_version":{},"patchset":"1",
            "files":{{{files}"inkboard_bg.wasm":{{"raw":8,"br":5,"gz":6}}}}}}"#,
        inkboard_core::serial::VERSION
    );
    assert!(big.len() > 2048, "{}", big.len());
    app.set_assets(site(None).with_inkboard("manifest.json", big.clone()));
    let res = app
        .server
        .get(&format!("/inkboard/{BUILD}/manifest.json"))
        .add_header("accept-encoding", "gzip")
        .await;
    res.assert_status_ok();
    assert!(res.maybe_header("content-encoding").is_none());
    assert_eq!(res.text(), big);
}

#[tokio::test]
async fn old_inkboard_build_404() {
    let app = app().await;
    for path in [
        "/inkboard/ffffffffffff/frame.html".to_owned(),
        "/inkboard/ffffffffffff/manifest.json".to_owned(),
        format!("/inkboard/{BUILD}/manifest.json.gz"),
        format!("/inkboard/{BUILD}/missing.js"),
        format!("/inkboard/{BUILD}/"),
        "/inkboard/frame.html".to_owned(),
    ] {
        assert_eq!(app.server.get(&path).await.status_code(), 404, "{path}");
    }
}

#[tokio::test]
async fn missing_manifest_gives_null_inkboard_config() {
    let mut app = TestApp::new().await;
    app.set_assets(site(None));
    let config: ApiConfig = app.server.get("/api/v1/config").await.json();
    assert!(config.inkboard.is_none());
    assert_eq!(
        app.server
            .get(&format!("/inkboard/{BUILD}/frame.html"))
            .await
            .status_code(),
        404
    );
    // The stub dist is served but gives no engine.
    app.set_assets(site(Some(manifest(
        BUILD,
        "stub",
        inkboard_core::serial::VERSION,
    ))));
    let config: ApiConfig = app.server.get("/api/v1/config").await.json();
    assert!(config.inkboard.is_none());
    // A real build: base, versions and the wasm sizes from the manifest.
    app.set_assets(site(Some(manifest(
        BUILD,
        "0.2.0-beta.3+embed.quire.1",
        inkboard_core::serial::VERSION,
    ))));
    let config: ApiConfig = app.server.get("/api/v1/config").await.json();
    let ink = config.inkboard.expect("an engine");
    assert_eq!(ink.base, format!("/inkboard/{BUILD}/"));
    assert_eq!(ink.engine_version, "0.2.0-beta.3+embed.quire.1");
    assert_eq!(ink.format_version, inkboard_core::serial::VERSION);
    assert_eq!(
        (ink.wasm_bytes.raw, ink.wasm_bytes.br, ink.wasm_bytes.gz),
        (8, 5, 6)
    );
    // Another format version refuses to start.
    let err = app
        .state
        .clone()
        .with_assets(Arc::new(site(Some(manifest(
            BUILD,
            "x",
            inkboard_core::serial::VERSION + 1,
        )))))
        .unwrap_err();
    assert!(err.contains("format_version"), "{err}");
}
