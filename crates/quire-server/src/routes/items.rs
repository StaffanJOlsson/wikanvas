//! Routes /api/v1/items: metadata, moves, publication, archive, trash, stars and views (owner: W1)
//!
//! §7.2. Every handler takes `Auth<S>` first (D44) with the scope its `#[utoipa::path]` declares. `GET /items/{id}`
//! accepts a `url_id` too and sends `ETag: "m{meta_version}"`, never 304 (D45); `PATCH /items/{id}` takes an optional
//! `If-Match: "m{n}"` (a `W/` prefix ignored, `*` any), 412 `rev_mismatch` when stale.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{FromRequest, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use quire_api::{
    DeleteItemQuery, ErrorBody, ItemDetail, ItemPatch, ItemSummary, ItemText, ItemTrashed,
    ItemsArchived, MoveItem, NewItem, PublishRequest, RestoredItem, StarRequest,
};
use serde::de::DeserializeOwned;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::ApiError;
use crate::extract::{Auth, IdPath, ItemRef, Read, Write};
use crate::state::{AppState, now_ms};

/// This module's routes, relative to `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(create_item))
        .routes(routes!(get_item))
        .routes(routes!(patch_item))
        .routes(routes!(delete_item))
        .routes(routes!(item_text))
        .routes(routes!(move_item))
        .routes(routes!(duplicate_item))
        .routes(routes!(publish_item))
        .routes(routes!(unpublish_item))
        .routes(routes!(archive_item))
        .routes(routes!(unarchive_item))
        .routes(routes!(restore_item))
        .routes(routes!(star_item))
        .routes(routes!(unstar_item))
        .routes(routes!(view_item))
}

/// A JSON body that may also be absent: an empty body is `T::default()`; a non-empty one must be
/// `application/json` (415 otherwise) and parse (422 `validation`, `field: "body"`).
pub struct OptionalJson<T>(pub T);

impl<T: DeserializeOwned + Default, S: Send + Sync> FromRequest<S> for OptionalJson<T> {
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Response> {
        let json = req
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|v| {
                v.split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .eq_ignore_ascii_case("application/json")
            });
        // A body over the limit is axum's 413, which the error envelope renders with `limit_bytes`.
        let bytes = Bytes::from_request(req, state)
            .await
            .map_err(IntoResponse::into_response)?;
        if bytes.is_empty() {
            return Ok(OptionalJson(T::default()));
        }
        if json != Some(true) {
            return Err(ApiError::new(
                quire_api::ErrorCode::UnsupportedMediaType,
                "send the body as application/json",
            )
            .into_response());
        }
        serde_json::from_slice(&bytes)
            .map(OptionalJson)
            .map_err(|e| {
                ApiError::validation("body", format!("the JSON body is not valid: {e}"))
                    .into_response()
            })
    }
}

/// The `n` of an `If-Match: "m{n}"` header: `None` when there is none or it is `*`; a tag of another shape never
/// matches (`Some(-1)`). A `W/` prefix is ignored (D45).
pub fn if_match_meta(headers: &HeaderMap) -> Option<i64> {
    let raw = headers.get(header::IF_MATCH)?.to_str().unwrap_or("\"?\"");
    let tag = raw.split(',').next().unwrap_or("").trim();
    let tag = tag.strip_prefix("W/").unwrap_or(tag);
    if tag == "*" {
        return None;
    }
    Some(
        tag.strip_prefix("\"m")
            .and_then(|t| t.strip_suffix('"'))
            .and_then(|n| n.parse::<i64>().ok())
            .unwrap_or(-1),
    )
}

/// The `ETag` of item metadata.
pub fn meta_etag(meta_version: i64) -> String {
    format!("\"m{meta_version}\"")
}

/// Whether the caller's theme is dark (new boards get the dark background): `users.theme` through
/// `quire_store::users::get`; light when the user cannot be read.
async fn dark_theme(state: &AppState, user_id: &str) -> bool {
    let user = user_id.to_owned();
    state
        .db
        .read(move |conn| quire_store::users::get(conn, &user))
        .await
        .is_ok_and(|u| u.theme == quire_api::Theme::Dark)
}

/// Creates a document or board, published (D23), with its content initialized, at `position` (omitted: last).
#[utoipa::path(
    post,
    path = "/items",
    tag = "items",
    operation_id = "create_item",
    security(("bearer" = ["write"]), ("session" = [])),
    request_body = NewItem,
    responses(
        (status = 201, description = "The new item", body = ItemSummary),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected`", body = ErrorBody),
        (status = 409, description = "`parent_invalid` (collection or parent missing, trashed, archived or foreign), `setup_required`", body = ErrorBody),
        (status = 413, description = "`payload_too_large`", body = ErrorBody),
        (status = 415, description = "`unsupported_media_type`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `title`, `position` or `body`)", body = ErrorBody),
    )
)]
async fn create_item(
    auth: Auth<Write>,
    State(state): State<AppState>,
    Json(new): Json<NewItem>,
) -> Result<(StatusCode, Json<ItemSummary>), ApiError> {
    let actor = auth.into_actor().user_id;
    let dark = dark_theme(&state, &actor).await;
    let now = now_ms();
    let item = state
        .db
        .write(move |tx| quire_store::items::create(tx, &actor, &new, dark, now))
        .await?;
    Ok((StatusCode::CREATED, Json(item)))
}

/// An item in any state, with its collection, breadcrumbs, star, effective publication, trash and archive dates and
/// board meta. `ETag: "m{meta_version}"` is for a later `If-Match`; this route never answers 304.
#[utoipa::path(
    get,
    path = "/items/{id}",
    tag = "items",
    operation_id = "get_item",
    security(("bearer" = ["read"]), ("session" = [])),
    params(("id" = String, Path, description = "The item's id or url_id")),
    responses(
        (status = 200, description = "The item", body = ItemDetail,
            headers(("ETag" = String, description = "`\"m{meta_version}\"`, for `If-Match` on PATCH"))),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn get_item(
    auth: Auth<Read>,
    State(state): State<AppState>,
    item: ItemRef,
) -> Result<Response, ApiError> {
    let viewer = auth.viewer();
    let detail = state
        .db
        .read(move |conn| {
            let row = quire_store::items::find(conn, &item.value())?;
            quire_store::items::detail(conn, &viewer, &row)
        })
        .await?;
    let etag = meta_etag(detail.meta_version);
    let mut res = Json(detail).into_response();
    res.headers_mut()
        .insert(header::ETAG, HeaderValue::from_str(&etag).expect("ASCII"));
    Ok(res)
}

/// Renames an item or sets `full_width`. With `If-Match: "m{n}"`, a stale tag is 412 `rev_mismatch`; without it the
/// last write wins.
#[utoipa::path(
    patch,
    path = "/items/{id}",
    tag = "items",
    operation_id = "patch_item",
    security(("bearer" = ["write"]), ("session" = [])),
    params(
        ("id" = uuid::Uuid, Path, description = "The item's id"),
        ("If-Match" = Option<String>, Header, description = "`\"m{meta_version}\"` from GET; `*` or none: last write wins"),
    ),
    request_body = ItemPatch,
    responses(
        (status = 200, description = "The item", body = ItemSummary,
            headers(("ETag" = String, description = "The new `\"m{meta_version}\"`"))),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`read_only` (trashed or archived), `setup_required`", body = ErrorBody),
        (status = 412, description = "`rev_mismatch` (`details.etag` is the current tag)", body = ErrorBody),
        (status = 415, description = "`unsupported_media_type`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `title` or `body`)", body = ErrorBody),
    )
)]
async fn patch_item(
    auth: Auth<Write>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
    headers: HeaderMap,
    Json(patch): Json<ItemPatch>,
) -> Result<Response, ApiError> {
    let actor = auth.into_actor().user_id;
    let if_match = if_match_meta(&headers);
    let now = now_ms();
    let (summary, meta) = state
        .db
        .write(move |tx| {
            let id = id.to_string();
            let summary = quire_store::items::update(tx, &id, &patch, if_match, &actor, now)?;
            Ok((
                summary,
                quire_store::shared::item_row(tx, &id)?.meta_version,
            ))
        })
        .await?;
    let mut res = Json(summary).into_response();
    res.headers_mut().insert(
        header::ETAG,
        HeaderValue::from_str(&meta_etag(meta)).expect("ASCII"),
    );
    Ok(res)
}

/// Moves an item and its subtree to the trash; with `?discard_empty=true`, deletes a new empty item permanently
/// instead (created under 24 h ago, no children, blank title, empty body; published or not), else 409 `not_empty`.
#[utoipa::path(
    delete,
    path = "/items/{id}",
    tag = "items",
    operation_id = "delete_item",
    security(("bearer" = ["write"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The item's id"), DeleteItemQuery),
    responses(
        (status = 200, description = "How many items were trashed (or deleted)", body = ItemTrashed),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`not_empty`, `read_only` (already in the trash), `setup_required`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `query`)", body = ErrorBody),
    )
)]
async fn delete_item(
    auth: Auth<Write>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
    Query(q): Query<DeleteItemQuery>,
) -> Result<Json<ItemTrashed>, ApiError> {
    let actor = auth.into_actor().user_id;
    let now = now_ms();
    let trashed = state
        .db
        .write(move |tx| {
            let id = id.to_string();
            if q.discard_empty {
                quire_store::items::discard_empty(tx, &id, now)
            } else {
                quire_store::items::trash(tx, &id, &actor, now)
            }
        })
        .await?;
    Ok(Json(ItemTrashed { trashed }))
}

/// The derived plain text that search indexes (§6.1, §6.2), in any state.
#[utoipa::path(
    get,
    path = "/items/{id}/text",
    tag = "items",
    operation_id = "get_item_text",
    security(("bearer" = ["read"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The item's id")),
    responses(
        (status = 200, description = "The text", body = ItemText),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn item_text(
    _auth: Auth<Read>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
) -> Result<Json<ItemText>, ApiError> {
    let text = state
        .db
        .read(move |conn| quire_store::items::text(conn, &id.to_string()))
        .await?;
    Ok(Json(text))
}

/// Moves an item with its subtree: to another parent (`null`: the top level), optionally in another collection, at
/// `position` among the new siblings (omitted: last).
#[utoipa::path(
    post,
    path = "/items/{id}/move",
    tag = "items",
    operation_id = "move_item",
    security(("bearer" = ["write"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The item's id")),
    request_body = MoveItem,
    responses(
        (status = 200, description = "The moved item", body = ItemSummary),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`cycle`, `parent_invalid`, `read_only`, `setup_required`", body = ErrorBody),
        (status = 415, description = "`unsupported_media_type`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `position` or `body`)", body = ErrorBody),
    )
)]
async fn move_item(
    auth: Auth<Write>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
    Json(mv): Json<MoveItem>,
) -> Result<Json<ItemSummary>, ApiError> {
    let actor = auth.into_actor().user_id;
    let now = now_ms();
    let item = state
        .db
        .write(move |tx| quire_store::items::move_item(tx, &id.to_string(), &mv, &actor, now))
        .await?;
    Ok(Json(item))
}

/// Duplicates an item right after it: its content (Y state or board bytes, attachments, text) and `full_width`;
/// the title gets " (copy)". Children are not copied; the copy is published.
#[utoipa::path(
    post,
    path = "/items/{id}/duplicate",
    tag = "items",
    operation_id = "duplicate_item",
    security(("bearer" = ["write"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The item's id")),
    responses(
        (status = 201, description = "The copy", body = ItemSummary),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`read_only` (trashed or archived), `setup_required`", body = ErrorBody),
    )
)]
async fn duplicate_item(
    auth: Auth<Write>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
) -> Result<(StatusCode, Json<ItemSummary>), ApiError> {
    let actor = auth.into_actor().user_id;
    let now = now_ms();
    let item = state
        .db
        .write(move |tx| quire_store::items::duplicate(tx, &id.to_string(), &actor, now))
        .await?;
    Ok((StatusCode::CREATED, Json(item)))
}

/// Publishes an item; with `with_ancestors: true`, its draft ancestors too. Not called by the M1 web client (D23).
/// The body may be omitted (`with_ancestors: false`).
#[utoipa::path(
    post,
    path = "/items/{id}/publish",
    tag = "items",
    operation_id = "publish_item",
    security(("bearer" = ["write"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The item's id")),
    request_body(content = Option<PublishRequest>, description = "Optional; `{}` or none publishes the item alone"),
    responses(
        (status = 200, description = "The item", body = ItemSummary),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`read_only` (trashed or archived), `setup_required`", body = ErrorBody),
        (status = 415, description = "`unsupported_media_type`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `body`)", body = ErrorBody),
    )
)]
async fn publish_item(
    auth: Auth<Write>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
    OptionalJson(req): OptionalJson<PublishRequest>,
) -> Result<Json<ItemSummary>, ApiError> {
    let actor = auth.into_actor().user_id;
    let now = now_ms();
    let item = state
        .db
        .write(move |tx| {
            quire_store::items::publish(tx, &id.to_string(), req.with_ancestors, &actor, now)
        })
        .await?;
    Ok(Json(item))
}

/// Makes an item a draft. Always allowed (whatever its parent and children); in M1 the only way an item becomes a
/// draft (D23).
#[utoipa::path(
    post,
    path = "/items/{id}/unpublish",
    tag = "items",
    operation_id = "unpublish_item",
    security(("bearer" = ["write"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The item's id")),
    responses(
        (status = 200, description = "The item", body = ItemSummary),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`read_only` (trashed or archived), `setup_required`", body = ErrorBody),
    )
)]
async fn unpublish_item(
    auth: Auth<Write>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
) -> Result<Json<ItemSummary>, ApiError> {
    let actor = auth.into_actor().user_id;
    let now = now_ms();
    let item = state
        .db
        .write(move |tx| quire_store::items::unpublish(tx, &id.to_string(), &actor, now))
        .await?;
    Ok(Json(item))
}

/// Archives an item with its subtree (the descendants that are neither archived nor trashed).
#[utoipa::path(
    post,
    path = "/items/{id}/archive",
    tag = "items",
    operation_id = "archive_item",
    security(("bearer" = ["write"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The item's id")),
    responses(
        (status = 200, description = "How many items were archived", body = ItemsArchived),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`read_only` (trashed or already archived), `setup_required`", body = ErrorBody),
    )
)]
async fn archive_item(
    _auth: Auth<Write>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
) -> Result<Json<ItemsArchived>, ApiError> {
    let now = now_ms();
    let archived = state
        .db
        .write(move |tx| quire_store::items::archive(tx, &id.to_string(), now))
        .await?;
    Ok(Json(ItemsArchived { archived }))
}

/// Unarchives the item's archive unit (its archive root). The root goes to the end of the top level when its parent
/// is gone (`moved_to_top`). Returns the root.
#[utoipa::path(
    post,
    path = "/items/{id}/unarchive",
    tag = "items",
    operation_id = "unarchive_item",
    security(("bearer" = ["write"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The item's id")),
    responses(
        (status = 200, description = "The unarchived root", body = RestoredItem),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`conflict` (not archived), `read_only` (in the trash), `setup_required`", body = ErrorBody),
    )
)]
async fn unarchive_item(
    _auth: Auth<Write>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
) -> Result<Json<RestoredItem>, ApiError> {
    let item = state
        .db
        .write(move |tx| quire_store::items::unarchive(tx, &id.to_string()))
        .await?;
    Ok(Json(item))
}

/// Restores the item's trash unit (its trash root). The root goes to the end of the top level when its parent is
/// gone (`moved_to_top`). Returns the root.
#[utoipa::path(
    post,
    path = "/items/{id}/restore",
    tag = "items",
    operation_id = "restore_item",
    security(("bearer" = ["write"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The item's id")),
    responses(
        (status = 200, description = "The restored root", body = RestoredItem),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`parent_invalid` (the collection is in the trash: restore it first), `conflict` (not in the trash), `setup_required`", body = ErrorBody),
    )
)]
async fn restore_item(
    _auth: Auth<Write>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
) -> Result<Json<RestoredItem>, ApiError> {
    let item = state
        .db
        .write(move |tx| quire_store::items::restore(tx, &id.to_string()))
        .await?;
    Ok(Json(item))
}

/// Stars an item at `position` among the caller's stars (omitted: last; an existing star keeps its place). The body
/// may be omitted.
#[utoipa::path(
    put,
    path = "/items/{id}/star",
    tag = "items",
    operation_id = "star_item",
    security(("bearer" = ["write"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The item's id")),
    request_body(content = Option<StarRequest>, description = "Optional"),
    responses(
        (status = 204, description = "Starred"),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`read_only` (in the trash), `setup_required`", body = ErrorBody),
        (status = 415, description = "`unsupported_media_type`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `position` or `body`)", body = ErrorBody),
    )
)]
async fn star_item(
    auth: Auth<Write>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
    OptionalJson(req): OptionalJson<StarRequest>,
) -> Result<StatusCode, ApiError> {
    let actor = auth.into_actor().user_id;
    let now = now_ms();
    state
        .db
        .write(move |tx| {
            quire_store::stars::star(tx, &actor, &id.to_string(), req.position.as_ref(), now)
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Removes the caller's star (also when there is none).
#[utoipa::path(
    delete,
    path = "/items/{id}/star",
    tag = "items",
    operation_id = "unstar_item",
    security(("bearer" = ["write"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The item's id")),
    responses(
        (status = 204, description = "Not starred"),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn unstar_item(
    auth: Auth<Write>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
) -> Result<StatusCode, ApiError> {
    let actor = auth.into_actor().user_id;
    state
        .db
        .write(move |tx| quire_store::stars::unstar(tx, &actor, &id.to_string()))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Records that the caller viewed the item (`GET /recent`). The web client calls it when it opens an item; the CLI
/// never does.
#[utoipa::path(
    post,
    path = "/items/{id}/view",
    tag = "items",
    operation_id = "view_item",
    security(("bearer" = ["write"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The item's id")),
    responses(
        (status = 204, description = "Recorded"),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn view_item(
    auth: Auth<Write>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
) -> Result<StatusCode, ApiError> {
    let actor = auth.into_actor().user_id;
    let now = now_ms();
    state
        .db
        .write(move |tx| quire_store::views::record(tx, &actor, &id.to_string(), now))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn if_match_tags() {
        let h = |v: &str| {
            let mut h = HeaderMap::new();
            h.insert(header::IF_MATCH, HeaderValue::from_str(v).unwrap());
            if_match_meta(&h)
        };
        assert_eq!(if_match_meta(&HeaderMap::new()), None);
        assert_eq!(h("*"), None);
        assert_eq!(h("\"m3\""), Some(3));
        assert_eq!(h("W/\"m12\""), Some(12));
        assert_eq!(h(" \"m4\" , \"m5\""), Some(4));
        assert_eq!(h("\"r3\""), Some(-1));
        assert_eq!(h("m3"), Some(-1));
        assert_eq!(meta_etag(7), "\"m7\"");
    }
}
