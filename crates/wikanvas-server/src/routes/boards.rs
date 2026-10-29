//! Routes /api/v1/items/{id}/board* (owner: W6)
//!
//! §6.2, §7.2, §7.5, D45, D51: the web client and API clients use the same pipeline.
//! - `GET …/board`: the native envelope with the blob bytes spliced back and `document.name` = the title, under the
//!   `board_work` permit; `ETag: "r{rev}"` for a later `If-Match`. Never 304: the body holds the live title, which the
//!   tag does not version (D45, as for the Markdown GET).
//! - `PUT …/board`: `If-Match` required (428; `"r{rev}"`, also `W/`, or `*`), `application/json` (415), up to
//!   `WIKANVAS_MAX_BOARD_MB` (413), `X-Wikanvas-Engine` optional (stored as `"api"` when absent). Under the permit:
//!   read the body (120 s, else 408), the JSON nesting limit (422 `invalid_board`), then `boards::prepare` (inspect,
//!   split, the free-space check, staging) and `boards::save` in the writer (the no-op test, 412 `rev_mismatch` with
//!   `{rev, etag}`, `pre_api` and `last_token_id` for a token); then the preview job. Answers
//!   `{rev, etag, updated_at, meta}`.
//! - `GET …/board.inkboard`: the GET's bytes as an attachment download.
//! - `GET …/board/pages/{n}`: page `n`'s SVG preview (§11): with the current `?rev=` immutable, without one the current
//!   page with `no-cache` and an ETag, another rev a 307 to the current one, an empty page 204, a page the preview
//!   budget left out a "too large to preview" card. A missing preview is rendered on demand.
//!
//! Cards (D152, §18.12): a card's label is the title snapshot of its target. For a caller who may not read some of the
//! board's card targets (never the owner or an admin), the GET, the download and the previews replace those cards'
//! labels with `board::REDACTED_LABEL` ([`card_filter`]); such a preview is rendered for the request and sent
//! `no-store` without an ETag, never from or into the preview cache.
//!
//! Memory (D25). The `board_work` permit is owned and moves into each blocking closure, so a cancelled request never
//! frees it while its work still runs. Large response bodies (envelopes, downloads, page SVGs of 1 MiB or more) also
//! hold a share of [`BODY_BUDGET_MIB`] from before the body is built until it is sent: it is taken before
//! `board_work` and never after it, so the two cannot deadlock. A body waits at most [`BODY_WAIT`] for its share
//! (else 429 `rate_limited`, `reason: "busy"`), and is sent by a task in 256 KiB chunks through a 4-chunk channel; a
//! client that stops reading for [`SEND_STALL`], or is not done after [`SEND_DEADLINE`], loses the connection and
//! frees the body.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use futures_util::StreamExt;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use tokio::sync::{Semaphore, SemaphorePermit, mpsc};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use wikanvas_api::common::header::ENGINE;
use wikanvas_api::{
    BoardPreviewQuery, BoardPutResult, ErrorBody, ErrorCode, RateLimitDetails, RateLimitReason,
    Timestamp,
};
use wikanvas_core::board;
use wikanvas_store::boards::{self, IfMatch};
use wikanvas_store::previews::{self, PageState, Preview};

use crate::authz::{self, Action, Resource};
use crate::board_previews;
use crate::config::JSON_DEPTH;
use crate::error::ApiError;
use crate::extract::{Actor, Auth, IdPath, Read, Write};
use crate::openapi::if_none_match;
use crate::state::{AppState, now_ms};

/// §8: board PUT bodies may take this long to arrive (the default request timeout is 30 s).
pub const BODY_DEADLINE: Duration = Duration::from_secs(120);

/// MiB of large board response bodies (envelopes, downloads, page SVGs) that may be in memory at once, across the
/// process (D25: they share the ~250 MiB headroom). A body larger than the budget takes all of it.
pub const BODY_BUDGET_MIB: u32 = 96;

/// Bodies from this size on hold a share of [`BODY_BUDGET_MIB`] (1 MiB, rounded up per MiB).
pub const LARGE_BODY: u64 = 1 << 20;

/// The longest a large body waits for its share before the request gets 429 `busy`.
pub const BODY_WAIT: Duration = Duration::from_secs(10);

/// A client that reads nothing for this long loses a large body's connection.
pub const SEND_STALL: Duration = Duration::from_secs(30);

/// The longest a large body may take to be sent.
pub const SEND_DEADLINE: Duration = Duration::from_secs(600);

/// The chunks a large body is sent in.
const CHUNK: usize = 256 * 1024;

static BODY_BUDGET: Semaphore = Semaphore::const_new(BODY_BUDGET_MIB as usize);

/// The longest accepted `X-Wikanvas-Engine`.
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
/// Never answers 304: the body holds the live title, which the ETag does not version (D45).
#[utoipa::path(
    get,
    path = "/items/{id}/board",
    tag = "boards",
    operation_id = "get_board",
    security(("bearer" = ["read"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The board's id")),
    responses(
        (status = 200, description = "The envelope", body = Object, content_type = "application/json",
            headers(("ETag" = String, description = "`\"r{rev}\"`, for a later `If-Match`"))),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 404, description = "`not_found`: no such item, or it is not a board", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
        (status = 429, description = "`rate_limited` (`reason: \"busy\"`, `Retry-After`): too many large board \
            bodies are being sent", body = ErrorBody),
    )
)]
async fn get_board(
    auth: Auth<Read>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
) -> Result<Response, ApiError> {
    let item_id = id.to_string();
    authz::require(&state, &auth, Action::Read, Resource::Item(&item_id)).await?;
    let keep = card_filter(&state, &auth, &item_id).await?;
    let slot = body_slot(envelope_len(&state, &item_id).await?).await?;
    let (rev, body, _) = load_with_title(&state, &item_id, keep).await?;
    Ok(envelope_response(rev, body, None, slot))
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
        (status = 429, description = "`rate_limited` (`reason: \"busy\"`, `Retry-After`): too many large board \
            bodies are being sent", body = ErrorBody),
    )
)]
async fn download_board(
    auth: Auth<Read>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
) -> Result<Response, ApiError> {
    let item_id = id.to_string();
    authz::require(&state, &auth, Action::Read, Resource::Item(&item_id)).await?;
    let keep = card_filter(&state, &auth, &item_id).await?;
    let slot = body_slot(envelope_len(&state, &item_id).await?).await?;
    let (rev, body, title) = load_with_title(&state, &item_id, keep).await?;
    Ok(envelope_response(rev, body, Some(&title), slot))
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
        ("X-Wikanvas-Engine" = Option<String>, Header, description = "The writing engine's version (the web client \
            sends its `engineVersion`); stored as `api` when absent. At most 128 printable ASCII characters"),
    ),
    request_body(content = Object, content_type = "application/json",
        description = "The native Inkboard envelope `{\"format\": \"inkboard\", \"version\": 2, \"document\": {…}}`, \
            up to `limits.max_board_mb` (`GET /config`)"),
    responses(
        (status = 200, description = "Saved (or unchanged)", body = BoardPutResult,
            headers(("ETag" = String, description = "`\"r{rev}\"`, for the next `If-Match`"))),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `forbidden` (a `viewer`), `csrf_rejected`",
            body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`setup_required`, `read_only` (trashed or archived)", body = ErrorBody),
        (status = 412, description = "`rev_mismatch`: the board changed; `details {rev, etag}` name the current \
            revision", body = ErrorBody),
        (status = 408, description = "`request_timeout`: the body did not arrive within 120 s", body = ErrorBody),
        (status = 413, description = "`payload_too_large` (`details.limit_bytes`)", body = ErrorBody),
        (status = 415, description = "`unsupported_media_type`: not `application/json`", body = ErrorBody),
        (status = 422, description = "`invalid_board` (also JSON nested deeper than `limits.json_depth`), \
            `board_too_new`, `validation` (`field`: `kind` for a document, `X-Wikanvas-Engine`, `body`)", body = ErrorBody),
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
    authz::require(&state, &actor, Action::Write, Resource::Item(&item_id)).await?;
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
            let row = wikanvas_store::shared::item_row(c, &check)?;
            if row.kind != wikanvas_api::ItemKind::Board {
                return Err(wikanvas_store::Error::validation(
                    "kind",
                    "the item is a document, not a board",
                ));
            }
            if row.deleted_at.is_some() || row.archived_at.is_some() {
                return Err(wikanvas_store::Error::new(
                    ErrorCode::ReadOnly,
                    "the item is in the trash or archived",
                ));
            }
            Ok(())
        })
        .await?;

    // One board body in memory at a time (D25): the permit is taken before the body is read. It is owned and moves
    // into the blocking work below, so a cancelled request cannot free it while that work still runs.
    let permit = Arc::clone(&state.board_work)
        .acquire_owned()
        .await
        .map_err(ApiError::internal)?;
    let bytes = read_body_within(body, limit, BODY_DEADLINE).await?;
    let (blobs, min_free) = (state.blobs.clone(), state.config.min_free_mb);
    let free_space = Arc::clone(&state.free_space);
    let (permit, prepared) = tokio::task::spawn_blocking(move || -> Result<_, ApiError> {
        let text = String::from_utf8(bytes)
            .map_err(|_| ApiError::new(ErrorCode::InvalidBoard, "the body is not UTF-8"))?;
        check_depth(&text)?;
        let check = || free_space(blobs.root(), min_free);
        Ok((permit, boards::prepare_with(&blobs, &text, check)?))
    })
    .await
    .map_err(ApiError::internal)??;
    let (blobs, item, now) = (state.blobs.clone(), item_id.clone(), now_ms());
    let saved = state
        .db
        .write(move |tx| {
            let _permit = permit;
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
/// answers 204. Images that would make the SVG too large are drawn as grey boxes, and a page (or a page after the
/// board's preview budget) that is too large to preview is a small card that says so. Over 1 KiB it is sent
/// gzip-encoded when the client accepts it.
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
        (status = 429, description = "`rate_limited` (`reason: \"busy\"`, `Retry-After`): too many large board \
            bodies are being sent", body = ErrorBody),
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
    authz::require(&state, &auth, Action::Read, Resource::Item(&item_id)).await?;
    let page = params
        .get("n")
        .and_then(|n| n.parse::<u32>().ok())
        .ok_or_else(|| ApiError::not_found("no such page"))? as usize;
    let item = item_id.clone();
    let (current, found) = state
        .db
        .read(move |c| {
            let s = boards::state(c, &item)?;
            let p = if page < s.pages {
                previews::state(c, &item, page, s.rev)?
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
    if let Some(keep) = card_filter(&state, &auth, &item_id).await? {
        return redacted_page(&state, &item_id, page, keep).await;
    }
    if if_none_match(&headers, &boards::etag(current.rev)) {
        return Ok(preview_headers(
            StatusCode::NOT_MODIFIED.into_response(),
            current.rev,
            query.rev.is_some(),
        ));
    }
    // The body's share of the budget comes first, before the SVG is read or rendered.
    let slot = match found {
        Some(PageState::Svg { len }) => body_slot(len as u64).await?,
        None => body_slot(board::PAGE_SVG_MAX as u64).await?,
        Some(PageState::Empty | PageState::Omitted) => BodySlot(None),
    };
    let stored = match found {
        Some(PageState::Empty) => Some(Preview::Empty),
        Some(PageState::Omitted) => Some(Preview::Omitted),
        Some(PageState::Svg { .. }) => {
            let (item, rev) = (item_id.clone(), current.rev);
            state
                .db
                .read(move |c| previews::get(c, &item, page, rev))
                .await?
        }
        None => None,
    };
    let (rev, preview) = match stored {
        Some(p) => (current.rev, p),
        // Not rendered yet (or replaced since the first read): render it now.
        None => {
            let rendered = board_previews::render(&state, &item_id, page)
                .await
                .map_err(not_a_board)?;
            if query.rev.is_some_and(|r| r != rendered.rev) {
                return Ok(redirect(&item_id, page, rendered.rev));
            }
            let preview = rendered
                .page
                .ok_or_else(|| ApiError::not_found("no such page"))?;
            (rendered.rev, preview)
        }
    };
    let res = match preview {
        Preview::Empty => StatusCode::NO_CONTENT.into_response(),
        Preview::Omitted => {
            let card = board::card_svg(&current.background, board::TOO_LARGE);
            svg_response(card, BodySlot(None))
        }
        Preview::Svg(svg) => svg_response(svg, slot),
    };
    Ok(preview_headers(res, rev, query.rev.is_some()))
}

/// The SVG response, sent under `slot`. The server's compression layer gzips it for a client that accepts gzip (§11,
/// `app::compressible`), so it is identity here; `Vary` is set for the identity answers too.
fn svg_response(svg: String, slot: BodySlot) -> Response {
    let mut res = send(svg.into_bytes(), slot);
    set(&mut res, header::CONTENT_TYPE, "image/svg+xml");
    set(&mut res, header::VARY, "Accept-Encoding");
    res
}

/// [`boards::envelope_len`]: what a GET of the board will send, about; a non-board is 404.
async fn envelope_len(state: &AppState, item_id: &str) -> Result<u64, ApiError> {
    let id = item_id.to_owned();
    state
        .db
        .read(move |c| boards::envelope_len(c, &id))
        .await
        .map_err(not_a_board)
}

/// The card targets a caller may open, when the board must be redacted for them (module docs, D152): `None` for the
/// owner and admins, and for a caller who may read every target the board links (and the board links fewer than
/// `EMBED_LINKS_MAX`, so every card's target is linked); otherwise the targets they may read, and the board itself.
async fn card_filter(
    state: &AppState,
    actor: &Actor,
    item_id: &str,
) -> Result<Option<HashSet<String>>, ApiError> {
    if matches!(
        actor.role,
        wikanvas_api::Role::Owner | wikanvas_api::Role::Admin
    ) {
        return Ok(None);
    }
    let (viewer, id) = (actor.viewer(), item_id.to_owned());
    let (mut readable, linked) = state
        .db
        .read(move |c| wikanvas_store::access::readable_embeds(c, &viewer, &id))
        .await?;
    if readable.len() == linked && linked < board::EMBED_LINKS_MAX {
        return Ok(None);
    }
    readable.insert(item_id.to_owned());
    Ok(Some(readable))
}

/// The envelope with the label of every card whose target is a canonical id outside `keep` redacted
/// (`board::redact_cards`). A card whose `item` is no canonical id names no item the host would open.
fn redact(body: &str, keep: &HashSet<String>) -> Result<String, ApiError> {
    board::redact_cards(body, |item| {
        !board::is_canonical_uuid(item) || keep.contains(item)
    })
    .map_err(ApiError::internal)
}

/// A page preview redacted for the caller (D152): rendered for this request from the redacted envelope, under the
/// `board_work` permit, and sent `no-store` without an ETag; 204 for an empty page.
async fn redacted_page(
    state: &AppState,
    item_id: &str,
    page: usize,
    keep: HashSet<String>,
) -> Result<Response, ApiError> {
    let permit = Arc::clone(&state.board_work)
        .acquire_owned()
        .await
        .map_err(ApiError::internal)?;
    let id = item_id.to_owned();
    let (permit, stored) = state
        .db
        .read(move |c| Ok((permit, boards::stored(c, &id)?)))
        .await
        .map_err(not_a_board)?;
    let blobs = state.blobs.clone();
    let rendered = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let full = boards::assemble(&blobs, &stored)?;
        drop(stored);
        let redacted = redact(&full, &keep)?;
        drop(full);
        let mut previewer = board::Previewer::new(&redacted).map_err(ApiError::internal)?;
        drop(redacted);
        Ok::<_, ApiError>(previewer.render(page))
    })
    .await
    .map_err(ApiError::internal)??;
    let mut res = match rendered {
        None => StatusCode::NO_CONTENT.into_response(),
        Some(svg) => {
            let slot = body_slot(svg.len() as u64).await?;
            svg_response(svg, slot)
        }
    };
    set(&mut res, header::CACHE_CONTROL, "no-store");
    set(&mut res, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(&mut res, header::CONTENT_SECURITY_POLICY, PREVIEW_CSP);
    Ok(res)
}

/// The full envelope, its rev and the title, read and spliced under the `board_work` permit, with the cards outside
/// `keep` redacted when it is given ([`card_filter`]). The permit is owned and moves into the reader and the
/// `spawn_blocking` closure, so it is held until the blocking work ends.
async fn load_with_title(
    state: &AppState,
    item_id: &str,
    keep: Option<HashSet<String>>,
) -> Result<(i64, String, String), ApiError> {
    let permit = Arc::clone(&state.board_work)
        .acquire_owned()
        .await
        .map_err(ApiError::internal)?;
    let id = item_id.to_owned();
    let (permit, stored) = state
        .db
        .read(move |c| Ok((permit, boards::stored(c, &id)?)))
        .await
        .map_err(not_a_board)?;
    let blobs = state.blobs.clone();
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let mut body = boards::assemble(&blobs, &stored)?;
        if let Some(keep) = &keep {
            body = redact(&body, keep)?;
        }
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

fn envelope_response(rev: i64, body: String, download: Option<&str>, slot: BodySlot) -> Response {
    let mut res = send(body.into_bytes(), slot);
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

/// `X-Wikanvas-Engine`, or `"api"` when absent (D51); 422 `validation` when it is not 1–128 printable ASCII characters.
fn engine(headers: &HeaderMap) -> Result<String, ApiError> {
    let Some(value) = headers.get(ENGINE) else {
        return Ok(boards::API_ENGINE.to_owned());
    };
    let v = value.to_str().map(str::trim).unwrap_or("");
    if v.is_empty() || v.len() > MAX_ENGINE_LEN || !v.bytes().all(|b| (0x20..0x7f).contains(&b)) {
        return Err(ApiError::validation(
            ENGINE,
            "X-Wikanvas-Engine is 1 to 128 printable ASCII characters",
        ));
    }
    Ok(v.to_owned())
}

/// `WIKANVAS_MAX_BOARD_MB` in bytes (MiB, like the other body limits).
fn board_limit(state: &AppState) -> u64 {
    state.config.max_board_mb.saturating_mul(1024 * 1024)
}

/// A share of [`BODY_BUDGET_MIB`] for one response body (none for a small body).
struct BodySlot(Option<SemaphorePermit<'static>>);

/// Waits (at most [`BODY_WAIT`]) for the share of a body of about `bytes`; 429 `rate_limited` (`busy`) after that.
async fn body_slot(bytes: u64) -> Result<BodySlot, ApiError> {
    if bytes < LARGE_BODY {
        return Ok(BodySlot(None));
    }
    let mib = u32::try_from(bytes.div_ceil(1 << 20))
        .unwrap_or(u32::MAX)
        .min(BODY_BUDGET_MIB);
    match tokio::time::timeout(BODY_WAIT, BODY_BUDGET.acquire_many(mib)).await {
        Ok(Ok(permit)) => Ok(BodySlot(Some(permit))),
        Ok(Err(e)) => Err(ApiError::internal(e)),
        Err(_) => Err(ApiError::new(
            ErrorCode::RateLimited,
            "the server is sending other large boards; retry shortly",
        )
        .with_details(&RateLimitDetails {
            reason: RateLimitReason::Busy,
        })
        .with_header(header::RETRY_AFTER, HeaderValue::from_static("5"))),
    }
}

/// The response for `body`, with its `Content-Length`. A body without a slot is sent as it is. A body with one is
/// sent by a task that holds the slot and the bytes, in [`CHUNK`]s through a 4-chunk channel: when the client reads
/// nothing for [`SEND_STALL`], or is not done after [`SEND_DEADLINE`], the task stops, which frees the bytes and the
/// slot, and the body ends with an error (the connection is closed, never a short body that looks complete).
fn send(body: Vec<u8>, slot: BodySlot) -> Response {
    let len = body.len();
    let mut res = if slot.0.is_none() {
        Body::from(body).into_response()
    } else {
        let (tx, rx) = mpsc::channel::<Bytes>(4);
        tokio::spawn(async move {
            let _slot = slot;
            let deadline = tokio::time::Instant::now() + SEND_DEADLINE;
            for chunk in body.chunks(CHUNK) {
                let wait =
                    SEND_STALL.min(deadline.saturating_duration_since(tokio::time::Instant::now()));
                match tokio::time::timeout(wait, tx.send(Bytes::copy_from_slice(chunk))).await {
                    Ok(Ok(())) => {}
                    // The client went away.
                    Ok(Err(_)) => return,
                    Err(_) => {
                        tracing::info!(
                            bytes = len,
                            "a board response was too slow to read; its connection is cut"
                        );
                        return;
                    }
                }
            }
        });
        // Fused: a consumer may poll again after the end (the compression layer's encoder does, for a gzip client),
        // and `unfold` panics when polled after it returned `None`.
        let stream = futures_util::stream::unfold((rx, 0usize), move |(mut rx, sent)| async move {
            match rx.recv().await {
                Some(chunk) => {
                    let sent = sent + chunk.len();
                    Some((Ok(chunk), (rx, sent)))
                }
                None if sent >= len => None,
                // The sender stopped early: fail the body, and the connection with it. `sent` is set past the end
                // so the stream ends after this error.
                None => Some((
                    Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "the board response was not read in time",
                    )),
                    (rx, usize::MAX),
                )),
            }
        });
        Body::from_stream(futures_util::StreamExt::fuse(stream)).into_response()
    };
    set(&mut res, header::CONTENT_LENGTH, &len.to_string());
    res
}

/// 422 `invalid_board` when the body nests arrays and objects deeper than `limits.json_depth` (§7.1).
fn check_depth(text: &str) -> Result<(), ApiError> {
    let depth = json_depth(text.as_bytes());
    if depth > JSON_DEPTH as usize {
        return Err(ApiError::new(
            ErrorCode::InvalidBoard,
            format!("the board nests JSON {depth} levels deep; the limit is {JSON_DEPTH}"),
        ));
    }
    Ok(())
}

/// The deepest nesting of arrays and objects in a JSON text, by a scan without recursion (strings are skipped).
fn json_depth(json: &[u8]) -> usize {
    let (mut depth, mut max, mut in_string, mut escaped) = (0usize, 0usize, false, false);
    for &b in json {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth += 1;
                max = max.max(depth);
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    max
}

/// [`read_body`] within `deadline`: 408 `request_timeout` when the body takes longer.
async fn read_body_within(body: Body, limit: u64, deadline: Duration) -> Result<Vec<u8>, ApiError> {
    tokio::time::timeout(deadline, read_body(body, limit))
        .await
        .map_err(|_| {
            ApiError::new(
                ErrorCode::RequestTimeout,
                format!("the board did not arrive within {} s", deadline.as_secs()),
            )
        })?
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
            HeaderValue::from_static("0.2.0-beta.3+embed.wikanvas.1"),
        );
        assert_eq!(engine(&h).unwrap(), "0.2.0-beta.3+embed.wikanvas.1");
        h.insert(ENGINE, HeaderValue::from_static(""));
        assert_eq!(engine(&h).unwrap_err().code, ErrorCode::Validation);
        h.insert(ENGINE, HeaderValue::from_str(&"x".repeat(129)).unwrap());
        assert!(engine(&h).is_err());
    }

    #[tokio::test]
    async fn body_reads_stop_at_the_limit_and_the_deadline() {
        // A chunked body without Content-Length: refused as soon as it passes the limit.
        let chunks = (0..8).map(|_| Ok::<_, std::io::Error>(Bytes::from(vec![b' '; 1000])));
        let body = Body::from_stream(futures_util::stream::iter(chunks));
        let e = read_body_within(body, 4000, BODY_DEADLINE)
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::PayloadTooLarge);
        // A body that stops arriving: 408 after the deadline.
        let stalled =
            Body::from_stream(futures_util::stream::pending::<Result<Bytes, std::io::Error>>());
        let e = read_body_within(stalled, 4000, Duration::from_millis(50))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::RequestTimeout);
        // Under both: the bytes.
        let ok = read_body_within(Body::from("{}"), 4000, BODY_DEADLINE)
            .await
            .unwrap();
        assert_eq!(ok, b"{}");
    }

    #[test]
    fn json_depth_skips_strings() {
        assert_eq!(json_depth(b"0"), 0);
        assert_eq!(json_depth(br#"{"a":[1,{"b":[]}]}"#), 4);
        assert_eq!(json_depth(br#"{"a":"[[[{{{\"]]]"}"#), 1);
        assert_eq!(json_depth(br#"[[["\\"],[]]]"#), 3);
        let deep = format!("{}{}", "[".repeat(101), "]".repeat(101));
        assert_eq!(json_depth(deep.as_bytes()), 101);
        assert_eq!(
            check_depth(&deep).unwrap_err().code,
            ErrorCode::InvalidBoard
        );
        check_depth(&deep[1..deep.len() - 1]).unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn large_bodies_wait_for_the_budget_then_429() {
        assert!(body_slot(LARGE_BODY - 1).await.unwrap().0.is_none());
        let all = BODY_BUDGET.acquire_many(BODY_BUDGET_MIB).await.unwrap();
        let e = body_slot(2 << 20).await.err().unwrap();
        assert_eq!(e.code, ErrorCode::RateLimited);
        assert_eq!(e.details, Some(serde_json::json!({"reason": "busy"})));
        drop(all);
        let slot = body_slot((3 << 20) - 5).await.unwrap();
        assert_eq!(
            BODY_BUDGET.available_permits(),
            BODY_BUDGET_MIB as usize - 3
        );
        drop(slot);
        // A body larger than the budget takes all of it.
        let whole = body_slot(500 << 20).await.unwrap();
        assert_eq!(BODY_BUDGET.available_permits(), 0);
        drop(whole);
        assert_eq!(BODY_BUDGET.available_permits(), BODY_BUDGET_MIB as usize);
    }

    #[tokio::test]
    async fn large_bodies_are_sent_whole() {
        static SLOT: Semaphore = Semaphore::const_new(1);
        let body: Vec<u8> = (0..5 * CHUNK + 17).map(|i| (i % 253) as u8).collect();
        let res = send(body.clone(), BodySlot(Some(SLOT.acquire().await.unwrap())));
        assert_eq!(
            res.headers()[header::CONTENT_LENGTH],
            body.len().to_string().as_str()
        );
        let back = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(back, body);
        // The sending task ends with the body and frees the slot.
        let _again = tokio::time::timeout(Duration::from_secs(5), SLOT.acquire())
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_client_that_stops_reading_loses_a_large_body() {
        static SLOT: Semaphore = Semaphore::const_new(1);
        let body: Vec<u8> = vec![7; 10 * CHUNK];
        let res = send(body, BodySlot(Some(SLOT.acquire().await.unwrap())));
        let mut stream = res.into_body().into_data_stream();
        let first = stream.next().await.unwrap().unwrap();
        assert_eq!(first.len(), CHUNK);
        // Nothing is read for longer than SEND_STALL: the task gives up, and the bytes and the slot go.
        tokio::time::sleep(SEND_STALL + Duration::from_secs(1)).await;
        assert_eq!(SLOT.available_permits(), 1);
        let mut got = first.len();
        let mut failed = false;
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(c) => got += c.len(),
                Err(_) => {
                    failed = true;
                    break;
                }
            }
        }
        assert!(failed, "the body ends with an error, not short");
        assert!(got < 10 * CHUNK);
    }

    #[test]
    fn content_types() {
        let mut h = HeaderMap::new();
        assert!(!is_json(&h));
        h.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
        assert!(is_json(&h));
        h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
        assert!(!is_json(&h));
    }
}
