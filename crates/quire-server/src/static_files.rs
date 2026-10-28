//! Static serving of the embedded web and Inkboard files with precompressed variants (owner: W2).
//!
//! The W0b version is the fallback of every path outside `/api/` and `/healthz`: an embedded file by its exact path
//! (`/inkboard/…` from `web/dist-inkboard`, everything else from `web/dist`), `index.html` for any path without an
//! extension (the SPA routes), and a plain 404 otherwise. W2 adds the precompressed variants, ETags, cache headers
//! and CSPs of §11.

use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};

fn respond(asset: quire_assets::Asset) -> Response {
    let mut res = Response::new(Body::from(asset.data.into_owned()));
    if let Ok(v) = HeaderValue::from_str(&asset.mime) {
        res.headers_mut().insert(header::CONTENT_TYPE, v);
    }
    res
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "not found\n").into_response()
}

/// The fallback handler for paths outside `/api/`.
pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.split('/').any(|seg| seg == "..") {
        return not_found();
    }
    if let Some(rest) = path.strip_prefix("inkboard/") {
        return quire_assets::inkboard(rest).map_or_else(not_found, respond);
    }
    if let Some(asset) = (!path.is_empty())
        .then(|| quire_assets::web(path))
        .flatten()
    {
        return respond(asset);
    }
    let last = path.rsplit('/').next().unwrap_or("");
    if last.contains('.') {
        return not_found();
    }
    match quire_assets::web("index.html") {
        Some(index) => {
            let mut res = respond(index);
            res.headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
            res
        }
        None => (
            StatusCode::NOT_FOUND,
            "the web client is not built (scripts/dev.sh pnpm --dir web build)\n",
        )
            .into_response(),
    }
}
