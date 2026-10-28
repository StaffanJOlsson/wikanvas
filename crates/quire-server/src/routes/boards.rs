//! Routes /api/v1/items/{id}/board* (owner: W6)
//!
//! §6.2, §7.2, §7.5, D45, D51: the web client and API clients use the same pipeline.
//! - `GET …/board`: the native envelope with the blob bytes spliced back and `document.name` = the title, under the
//!   `board_work` permit; `ETag: "r{rev}"`, 304 for a matching `If-None-Match`.
//! - `PUT …/board`: `If-Match` required (428; `"r{rev}"`, also `W/`, or `*`), `application/json` (415), up to
//!   `QUIRE_MAX_BOARD_MB` (413), `X-Quire-Engine` optional (stored as `"api"` when absent). Under the permit: read the
//!   body, then `boards::prepare` (inspect, split, the free-space check, staging) and `boards::save` in the writer
//!   (the no-op test, 412 `rev_mismatch` with `{rev, etag}`, `pre_api` and `last_token_id` for a token); then the
//!   preview job. Answers `{rev, etag, updated_at, meta}`.
//! - `GET …/board.inkboard`: the GET's bytes as an attachment download.
//! - `GET …/board/pages/{n}`: page `n`'s SVG preview (§11): with the current `?rev=` immutable, without one the current
//!   page with `no-cache` and an ETag, another rev a 307 to the current one, an empty page 204. A missing preview is
//!   rendered on demand.

use std::collections::HashMap;
use std::time::Duration;

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use quire_api::common::header::ENGINE;
use quire_api::{BoardPreviewQuery, BoardPutResult, ErrorBody, ErrorCode, Timestamp};
use quire_store::boards::{self, IfMatch};
use quire_store::previews::{self, Preview};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::authz::{self, Action, Resource};
use crate::board_previews;
use crate::error::ApiError;
use crate::extract::{Auth, IdPath, Read, Write};
use crate::openapi::if_none_match;
use crate::state::{AppState, now_ms};

/// §8: board PUT bodies may take this long to arrive (the default request timeout is 30 s).
pub const BODY_DEADLINE: Duration = Duration::from_secs(120);

/// The longest accepted `X-Quire-Engine`.
pub const MAX_ENGINE_LEN: usize = 128;

/// The page preview CSP (§11).
pub const PREVIEW_CSP: &str =
    "default-src 'none'; img-src data:; style-src 'unsafe-inline'; sandbox";

/// `Cache-Control` of a preview addressed by its current revision.
const IMMUTABLE: &str = "private, max-age=31536000, immutable";

/// This module's routes, relative to `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(get_board))
        .routes(routes!(put_board))
        .routes(routes!(download_board))
        .routes(routes!(get_board_page))
}

/// The board's native Inkboard envelope `{"format":"inkboard","version":2,"document":…}` (D8, D51): its images
/// inline as base64 and `document.name` set to the item's title (D31). Unknown fields a newer engine wrote are kept.
#[utoipa::path(
    get,
    path = "/items/{id}/board",
    tag = "boards",
    operation_id = "get_board",
    security(("bearer" = ["read"]), ("session" = [])),
    params(
        ("id" = uuid::Uuid, Path, description = "The board's id"),
        ("If-None-Match" = Option<String>, Header, description = "A previous ETag (`\"r{rev}\"`): 304 when unchanged"),
    ),
    responses(
        (status = 200, description = "The envelope", body = Object, content_type = "application/json",
            headers(("ETag" = String, description = "`\"r{rev}\"`, for a later `If-Match`"))),
        (status = 304, description = "Unchanged since the ETag in `If-None-Match`"),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 404, description = "`not_found`: no such item, or it is not a board", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn get_board(
    auth: Auth<Read>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let item_id = id.to_string();
    authz::require(&auth, Action::Read, Resource::Item(&item_id))?;
    let current = board_state(&state, &item_id).await?;
    let tag = boards::etag(current.rev);
    if if_none_match(&headers, &tag) {
        return Ok(not_modified(&tag));
    }
    let (rev, body) = load(&state, &item_id).await?;
    Ok(envelope_response(rev, body, None))
}

/// The board as a `.inkboard` file: the bytes of `GET …/board`, as an attachment named after the title.
#[utoipa::path(
    get,
    path = "/items/{id}/board.inkboard",
    tag = "boards",
    operation_id = "download_board",
    security(("bearer" = ["read"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The board's id")),
    responses(
        (status = 200, description = "The envelope, with `Content-Disposition: attachment; filename=\"<title>.inkboard\"; \
            filename*=UTF-8''<title>.inkboard`", body = Object, content_type = "application/json",
            headers(("ETag" = String, description = "`\"r{rev}\"`"),
                ("Content-Disposition" = String, description = "The download's file name (RFC 6266)"))),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 404, description = "`not_found`: no such item, or it is not a board", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn download_board(
    auth: Auth<Read>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
) -> Result<Response, ApiError> {
    let item_id = id.to_string();
    authz::require(&auth, Action::Read, Resource::Item(&item_id))?;
    let (rev, body, title) = load_with_title(&state, &item_id).await?;
    Ok(envelope_response(rev, body, Some(&title)))
}

/// Replaces the board with a native envelope (§6.2, D51).
///
/// The body is validated with Inkboard's own reader; its inline images move to the blob store, everything else is
/// stored as sent, so fields this server does not know survive. `document.name` is ignored: the title is the item's.
/// A body equal to the stored one changes nothing and answers the current revision. A token's write first keeps a
/// `pre-api` revision of the current state, unless the last write came from the same token (D53).
#[utoipa::path(
    put,
    path = "/items/{id}/board",
    tag = "boards",
    operation_id = "put_board",
    security(("bearer" = ["write"]), ("session" = [])),
    params(
        ("id" = uuid::Uuid, Path, description = "The board's id"),
        ("If-Match" = String, Header, description = "`\"r{rev}\"` of the revision the body is based on (a `W/` prefix \
            is ignored), or `*` to overwrite whatever is current"),
        ("X-Quire-Engine" = Option<String>, Header, description = "The writing engine's version (the web client sends \
            its `engineVersion`); stored as `api` when absent. At most 128 printable ASCII characters"),
    ),
    request_body(content = Object, content_type = "application/json",
        description = "The native Inkboard envelope `{\"format\": \"inkboard\", \"version\": 2, \"document\": {…}}`, \
            up to `limits.max_board_mb` (`GET /config`)"),
    responses(
        (status = 200, description = "Saved (or unchanged)", body = BoardPutResult,
            headers(("ETag" = String, description = "`\"r{rev}\"`, for the next `If-Match`"))),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`setup_required`, `read_only` (trashed or archived)", body = ErrorBody),
        (status = 412, description = "`rev_mismatch`: the board changed; `details {rev, etag}` name the current \
            revision", body = ErrorBody),
        (status = 413, description = "`payload_too_large` (`details.limit_bytes`)", body = ErrorBody),
        (status = 415, description = "`unsupported_media_type`: not `application/json`", body = ErrorBody),
        (status = 422, description = "`invalid_board`, `board_too_new`, `validation` (`field`: `kind` for a \
            document, `X-Quire-Engine`, `body`)", body = ErrorBody),
        (status = 428, description = "`precondition_required`: no `If-Match`", body = ErrorBody),
        (status = 507, description = "`insufficient_storage`: below the free-space floor", body = ErrorBody),
    )
)]
async fn put_board(
    auth: Auth<Write>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let actor = auth.into_actor();
    let item_id = id.to_string();
    authz::require(&actor, Action::Write, Resource::Item(&item_id))?;
    let if_match = headers
        .get(header::IF_MATCH)
        .map(|v| IfMatch::parse(v.to_str().unwrap_or("")))
        .ok_or_else(|| {
            ApiError::new(
                ErrorCode::PreconditionRequired,
                "send If-Match: \"r{rev}\" (from the GET's ETag), or If-Match: * to overwrite",
            )
        })?;
    if !is_json(&headers) {
        return Err(ApiError::new(
            ErrorCode::UnsupportedMediaType,
            "a board is sent as application/json",
        ));
    }
    let engine = engine(&headers)?;
    let limit = board_limit(&state);
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|n| n > limit) {
        return Err(ApiError::payload_too_large(limit));
    }
    // Refuse a missing, trashed or non-board item before reading a large body.
    let check = item_id.clone();
    state
        .db
        .read(move |c| {
            let row = quire_store::shared::item_row(c, &check)?;
            if row.kind != quire_api::ItemKind::Board {
                return Err(quire_store::Error::validation(
                    "kind",
                    "the item is a document, not a board",
                ));
            }
            if row.deleted_at.is_some() || row.archived_at.is_some() {
                return Err(quire_store::Error::new(
                    ErrorCode::ReadOnly,
                    "the item is in the trash or archived",
                ));
            }
            Ok(())
        })
        .await?;

    // One board body in memory at a time (D25): the permit is taken before the body is read.
    let _permit = state
        .board_work
        .acquire()
        .await
        .map_err(ApiError::internal)?;
    let bytes = tokio::time::timeout(BODY_DEADLINE, read_body(body, limit))
        .await
        .map_err(|_| {
            ApiError::new(
                ErrorCode::RequestTimeout,
                "the board did not arrive within 120 s",
            )
        })??;
    let text = String::from_utf8(bytes)
        .map_err(|_| ApiError::new(ErrorCode::InvalidBoard, "the body is not UTF-8"))?;
    let (blobs, min_free) = (state.blobs.clone(), state.config.min_free_mb);
    let prepared = tokio::task::spawn_blocking(move || boards::prepare(&blobs, &text, min_free))
        .await
        .map_err(ApiError::internal)??;
    let (blobs, item, now) = (state.blobs.clone(), item_id.clone(), now_ms());
    let saved = state
        .db
        .write(move |tx| {
            boards::save(
                tx,
                &blobs,
                &item,
                prepared,
                &if_match,
                &actor.write_by(),
                &engine,
                now,
            )
        })
        .await?;
    if saved.changed {
        state.previews.enqueue(&item_id);
    }
    let etag = boards::etag(saved.rev);
    let mut res = Json(BoardPutResult {
        rev: saved.rev,
        etag: etag.clone(),
        updated_at: Timestamp(saved.updated_at),
        meta: saved.meta,
    })
    .into_response();
    set(&mut res, header::ETAG, &etag);
    set(&mut res, header::CACHE_CONTROL, "no-store");
    Ok(res)
}

/// One page of the board as SVG (`svg::export_approx`, D10), for thumbnails, the loading placeholder and the
/// read-only view.
///
/// Without `rev`: the current page, `Cache-Control: no-cache` with `ETag: "r{rev}"`. With the current `rev`:
/// `Cache-Control: private, max-age=31536000, immutable`. With another `rev`: 307 to the current one. An empty page
/// answers 204. Sent gzip-encoded when the client accepts it.
#[utoipa::path(
    get,
    path = "/items/{id}/board/pages/{n}",
    tag = "boards",
    operation_id = "get_board_page",
    security(("bearer" = ["read"]), ("session" = [])),
    params(
        ("id" = uuid::Uuid, Path, description = "The board's id"),
        ("n" = u32, Path, description = "The page, 0-based"),
        BoardPreviewQuery,
        ("If-None-Match" = Option<String>, Header, description = "A previous ETag: 304 when unchanged"),
    ),
    responses(
        (status = 200, description = "The page's SVG (`nosniff`, a sandboxing CSP)", body = String,
            content_type = "image/svg+xml",
            headers(("ETag" = String, description = "`\"r{rev}\"`"),
                ("Cache-Control" = String, description = "`no-cache` without `rev`, immutable with the current one"))),
        (status = 204, description = "The page is empty"),
        (status = 304, description = "Unchanged since the ETag in `If-None-Match`"),
        (status = 307, description = "`rev` is not the current revision: the URL of the current one",
            headers(("Location" = String, description = "`…/board/pages/{n}?rev={current}`"))),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 404, description = "`not_found`: no such item, not a board, or no such page", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `query`)", body = ErrorBody),
    )
)]
async fn get_board_page(
    auth: Auth<Read>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
    Path(params): Path<HashMap<String, String>>,
    Query(query): Query<BoardPreviewQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let item_id = id.to_string();
    authz::require(&auth, Action::Read, Resource::Item(&item_id))?;
    let page = params
        .get("n")
        .and_then(|n| n.parse::<u32>().ok())
        .ok_or_else(|| ApiError::not_found("no such page"))? as usize;
    let item = item_id.clone();
    let (current, stored) = state
        .db
        .read(move |c| {
            let s = boards::state(c, &item)?;
            let p = if page < s.pages {
                previews::get(c, &item, page, s.rev)?
            } else {
                None
            };
            Ok((s, p))
        })
        .await
        .map_err(not_a_board)?;
    if page >= current.pages {
        return Err(ApiError::not_found(format!(
            "the board has {} page(s); pages count from 0",
            current.pages
        )));
    }
    if let Some(rev) = query.rev
        && rev != current.rev
    {
        return Ok(redirect(&item_id, page, current.rev));
    }
    if if_none_match(&headers, &boards::etag(current.rev)) {
        return Ok(preview_headers(
            StatusCode::NOT_MODIFIED.into_response(),
            current.rev,
            query.rev.is_some(),
        ));
    }
    let (rev, preview) = match stored {
        Some(p) => (current.rev, p),
        None => {
            let rendered = board_previews::render(&state, &item_id)
                .await
                .map_err(not_a_board)?;
            if query.rev.is_some_and(|r| r != rendered.rev) {
                return Ok(redirect(&item_id, page, rendered.rev));
            }
            let preview = match rendered.pages.into_iter().nth(page) {
                Some(Some(svg)) => Preview::Svg(svg),
                Some(None) => Preview::Empty,
                None => return Err(ApiError::not_found("no such page")),
            };
            (rendered.rev, preview)
        }
    };
    let res = match preview {
        Preview::Empty => StatusCode::NO_CONTENT.into_response(),
        Preview::Svg(svg) => {
            let gzip = accepts_gzip(&headers);
            let mut res = if gzip {
                let body = quire_store::previews::gzip(svg.as_bytes())?;
                let mut r = body.into_response();
                set(&mut r, header::CONTENT_ENCODING, "gzip");
                r
            } else {
                svg.into_response()
            };
            set(&mut res, header::CONTENT_TYPE, "image/svg+xml");
            set(&mut res, header::VARY, "Accept-Encoding");
            res
        }
    };
    Ok(preview_headers(res, rev, query.rev.is_some()))
}

/// The stored board's rev and page count; a non-board is 404.
async fn board_state(state: &AppState, item_id: &str) -> Result<boards::BoardState, ApiError> {
    let id = item_id.to_owned();
    state
        .db
        .read(move |c| boards::state(c, &id))
        .await
        .map_err(not_a_board)
}

/// The full envelope and its rev, read and spliced under the `board_work` permit.
async fn load(state: &AppState, item_id: &str) -> Result<(i64, String), ApiError> {
    let (rev, body, _) = load_with_title(state, item_id).await?;
    Ok((rev, body))
}

async fn load_with_title(
    state: &AppState,
    item_id: &str,
) -> Result<(i64, String, String), ApiError> {
    let _permit = state
        .board_work
        .acquire()
        .await
        .map_err(ApiError::internal)?;
    let id = item_id.to_owned();
    let stored = state
        .db
        .read(move |c| boards::stored(c, &id))
        .await
        .map_err(not_a_board)?;
    let blobs = state.blobs.clone();
    tokio::task::spawn_blocking(move || {
        let body = boards::assemble(&blobs, &stored)?;
        Ok::<_, ApiError>((stored.rev, body, stored.title))
    })
    .await
    .map_err(ApiError::internal)?
}

/// A store `not_found` (or a document's `kind` refusal) as 404 "the item has no board".
fn not_a_board(e: impl Into<ApiError>) -> ApiError {
    let e = e.into();
    if e.code == ErrorCode::NotFound {
        ApiError::not_found("no such board")
    } else {
        e
    }
}

fn envelope_response(rev: i64, body: String, download: Option<&str>) -> Response {
    let mut res = body.into_response();
    set(&mut res, header::CONTENT_TYPE, "application/json");
    set(&mut res, header::ETAG, &boards::etag(rev));
    set(&mut res, header::CACHE_CONTROL, "no-store");
    if let Some(title) = download {
        set(
            &mut res,
            header::CONTENT_DISPOSITION,
            &content_disposition(title, "inkboard"),
        );
    }
    res
}

fn not_modified(etag: &str) -> Response {
    let mut res = StatusCode::NOT_MODIFIED.into_response();
    set(&mut res, header::ETAG, etag);
    set(&mut res, header::CACHE_CONTROL, "no-store");
    res
}

fn redirect(item_id: &str, page: usize, rev: i64) -> Response {
    let mut res = StatusCode::TEMPORARY_REDIRECT.into_response();
    set(
        &mut res,
        header::LOCATION,
        &format!("/api/v1/items/{item_id}/board/pages/{page}?rev={rev}"),
    );
    set(&mut res, header::CACHE_CONTROL, "no-store");
    res
}

/// §11's preview headers: cache policy (immutable for an explicit current `rev`, else `no-cache`), the ETag,
/// `nosniff` and the CSP.
fn preview_headers(mut res: Response, rev: i64, immutable: bool) -> Response {
    set(
        &mut res,
        header::CACHE_CONTROL,
        if immutable { IMMUTABLE } else { "no-cache" },
    );
    set(&mut res, header::ETAG, &boards::etag(rev));
    set(&mut res, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(&mut res, header::CONTENT_SECURITY_POLICY, PREVIEW_CSP);
    res
}

fn set(res: &mut Response, name: HeaderName, value: &str) {
    if let Ok(v) = HeaderValue::from_str(value) {
        res.headers_mut().insert(name, v);
    }
}

/// `Content-Type: application/json`, with or without parameters.
fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .is_some_and(|t| t.trim().eq_ignore_ascii_case("application/json"))
}

fn accepts_gzip(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::ACCEPT_ENCODING)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|coding| {
            let mut parts = coding.split(';');
            let name = parts.next().unwrap_or("").trim();
            let refused = parts.any(|p| {
                p.trim()
                    .strip_prefix("q=")
                    .and_then(|q| q.trim().parse::<f32>().ok())
                    == Some(0.0)
            });
            name.eq_ignore_ascii_case("gzip") && !refused
        })
}

/// `X-Quire-Engine`, or `"api"` when absent (D51); 422 `validation` when it is not 1–128 printable ASCII characters.
fn engine(headers: &HeaderMap) -> Result<String, ApiError> {
    let Some(value) = headers.get(ENGINE) else {
        return Ok(boards::API_ENGINE.to_owned());
    };
    let v = value.to_str().map(str::trim).unwrap_or("");
    if v.is_empty() || v.len() > MAX_ENGINE_LEN || !v.bytes().all(|b| (0x20..0x7f).contains(&b)) {
        return Err(ApiError::validation(
            ENGINE,
            "X-Quire-Engine is 1 to 128 printable ASCII characters",
        ));
    }
    Ok(v.to_owned())
}

/// `QUIRE_MAX_BOARD_MB` in bytes (MiB, like the other body limits).
fn board_limit(state: &AppState) -> u64 {
    state.config.max_board_mb.saturating_mul(1024 * 1024)
}

/// Reads the body, refusing it with 413 as soon as it passes `limit` bytes.
async fn read_body(body: Body, limit: u64) -> Result<Vec<u8>, ApiError> {
    let mut stream = body.into_data_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| {
            ApiError::validation("body", format!("the body could not be read: {e}"))
        })?;
        if (buf.len() + chunk.len()) as u64 > limit {
            return Err(ApiError::payload_too_large(limit));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// RFC 5987 `attr-char`s stay as they are in `filename*`; everything else is percent-encoded.
const ATTR: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'!')
    .remove(b'#')
    .remove(b'$')
    .remove(b'&')
    .remove(b'+')
    .remove(b'-')
    .remove(b'.')
    .remove(b'^')
    .remove(b'_')
    .remove(b'`')
    .remove(b'|')
    .remove(b'~');

/// `attachment; filename="<ASCII fallback>"; filename*=UTF-8''<percent-encoded>` (§8, RFC 6266) for `<title>.<ext>`:
/// control characters, `"`, `\` and `/` are dropped, the name is cut to 200 characters, `board` when nothing is left;
/// the fallback replaces every non-ASCII character with `_`.
pub fn content_disposition(title: &str, ext: &str) -> String {
    let clean: String = title
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '"' | '\\' | '/'))
        .take(200)
        .collect();
    let clean = clean.trim();
    let name = format!("{}.{ext}", if clean.is_empty() { "board" } else { clean });
    let ascii: String = name
        .chars()
        .map(|c| if c.is_ascii() { c } else { '_' })
        .collect();
    format!(
        "attachment; filename=\"{ascii}\"; filename*=UTF-8''{}",
        utf8_percent_encode(&name, ATTR)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disposition_follows_the_filename_rule() {
        assert_eq!(
            content_disposition("Plan", "inkboard"),
            "attachment; filename=\"Plan.inkboard\"; filename*=UTF-8''Plan.inkboard"
        );
        assert_eq!(
            content_disposition("Été \"2026\" a/b\\c\n", "inkboard"),
            "attachment; filename=\"_t_ 2026 abc.inkboard\"; filename*=UTF-8''%C3%89t%C3%A9%202026%20abc.inkboard"
        );
        assert_eq!(
            content_disposition("  ", "inkboard"),
            "attachment; filename=\"board.inkboard\"; filename*=UTF-8''board.inkboard"
        );
        let long = "x".repeat(300);
        assert!(
            content_disposition(&long, "inkboard")
                .contains(&format!("{}.inkboard", "x".repeat(200)))
        );
    }

    #[test]
    fn engine_header() {
        let mut h = HeaderMap::new();
        assert_eq!(engine(&h).unwrap(), "api");
        h.insert(
            ENGINE,
            HeaderValue::from_static("0.2.0-beta.3+embed.quire.1"),
        );
        assert_eq!(engine(&h).unwrap(), "0.2.0-beta.3+embed.quire.1");
        h.insert(ENGINE, HeaderValue::from_static(""));
        assert_eq!(engine(&h).unwrap_err().code, ErrorCode::Validation);
        h.insert(ENGINE, HeaderValue::from_str(&"x".repeat(129)).unwrap());
        assert!(engine(&h).is_err());
    }

    #[test]
    fn content_types_and_encodings() {
        let mut h = HeaderMap::new();
        assert!(!is_json(&h));
        h.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
        assert!(is_json(&h));
        h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
        assert!(!is_json(&h));
        assert!(!accepts_gzip(&h));
        h.insert(
            header::ACCEPT_ENCODING,
            HeaderValue::from_static("br, gzip;q=0.8"),
        );
        assert!(accepts_gzip(&h));
        h.insert(
            header::ACCEPT_ENCODING,
            HeaderValue::from_static("gzip;q=0, br"),
        );
        assert!(!accepts_gzip(&h));
    }
}
