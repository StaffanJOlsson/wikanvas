//! Routes /api/v1 lists: archive, drafts, starred, recent, updated, a collection's updated, and the trash (owner: W1)
//!
//! D46: every paginated list takes `?cursor&limit` and returns `Page<ListedItem>` (`{items, next_cursor}`,
//! `next_cursor = null` on the last page); a cursor reused with other parameters is 422 `validation`
//! (`field: "cursor"`). `GET /trash` lists roots only and is unpaginated; purging needs the `admin` scope and, through
//! `authz::require` (§8), Admin (the owner or an `admin`); every list here is Read.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use wikanvas_api::{ErrorBody, ListedItem, Page, PageQuery, TrashList};
use wikanvas_store::lists::PageRequest;

use crate::authz::{self, Action, Resource};
use crate::error::ApiError;
use crate::extract::{Admin, Auth, IdPath, Read};
use crate::state::AppState;

/// This module's routes, relative to `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_archive))
        .routes(routes!(list_drafts))
        .routes(routes!(list_starred))
        .routes(routes!(list_recent))
        .routes(routes!(list_updated))
        .routes(routes!(list_collection_updated))
        .routes(routes!(get_trash))
        .routes(routes!(purge_trash_item))
        .routes(routes!(purge_trash_collection))
        .routes(routes!(empty_trash))
}

fn request(list: &str, q: PageQuery) -> PageRequest {
    PageRequest::new(list, q.cursor, q.limit)
}

/// Archived items (archive roots that are not in the trash), most recently archived first, with `archived_at`.
#[utoipa::path(
    get,
    path = "/archive",
    tag = "lists",
    operation_id = "list_archive",
    security(("bearer" = ["read"]), ("session" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page", body = Page<ListedItem>),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `cursor`, `limit` or `query`)", body = ErrorBody),
    )
)]
async fn list_archive(
    auth: Auth<Read>,
    State(state): State<AppState>,
    Query(q): Query<PageQuery>,
) -> Result<Json<Page<ListedItem>>, ApiError> {
    authz::require(&state, &auth, Action::Read, Resource::Workspace).await?;
    let viewer = auth.viewer();
    let req = request("archive", q);
    Ok(Json(
        state
            .db
            .read(move |c| wikanvas_store::lists::archive(c, &viewer, &req))
            .await?,
    ))
}

/// Unpublished items (in M1 only those unpublished through the API or CLI, D23), most recently updated first.
#[utoipa::path(
    get,
    path = "/drafts",
    tag = "lists",
    operation_id = "list_drafts",
    security(("bearer" = ["read"]), ("session" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page", body = Page<ListedItem>),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `cursor`, `limit` or `query`)", body = ErrorBody),
    )
)]
async fn list_drafts(
    auth: Auth<Read>,
    State(state): State<AppState>,
    Query(q): Query<PageQuery>,
) -> Result<Json<Page<ListedItem>>, ApiError> {
    authz::require(&state, &auth, Action::Read, Resource::Workspace).await?;
    let viewer = auth.viewer();
    let req = request("drafts", q);
    Ok(Json(
        state
            .db
            .read(move |c| wikanvas_store::lists::drafts(c, &viewer, &req))
            .await?,
    ))
}

/// The caller's starred items (visible, unarchived) in star order, with `starred_at`.
#[utoipa::path(
    get,
    path = "/starred",
    tag = "lists",
    operation_id = "list_starred",
    security(("bearer" = ["read"]), ("session" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page", body = Page<ListedItem>),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `cursor`, `limit` or `query`)", body = ErrorBody),
    )
)]
async fn list_starred(
    auth: Auth<Read>,
    State(state): State<AppState>,
    Query(q): Query<PageQuery>,
) -> Result<Json<Page<ListedItem>>, ApiError> {
    authz::require(&state, &auth, Action::Read, Resource::Workspace).await?;
    let viewer = auth.viewer();
    let req = request("starred", q);
    Ok(Json(
        state
            .db
            .read(move |c| wikanvas_store::lists::starred(c, &viewer, &req))
            .await?,
    ))
}

/// The items the caller viewed (`POST /items/{id}/view`), most recently viewed first, with `last_viewed_at`.
#[utoipa::path(
    get,
    path = "/recent",
    tag = "lists",
    operation_id = "list_recent",
    security(("bearer" = ["read"]), ("session" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page", body = Page<ListedItem>),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `cursor`, `limit` or `query`)", body = ErrorBody),
    )
)]
async fn list_recent(
    auth: Auth<Read>,
    State(state): State<AppState>,
    Query(q): Query<PageQuery>,
) -> Result<Json<Page<ListedItem>>, ApiError> {
    authz::require(&state, &auth, Action::Read, Resource::Workspace).await?;
    let viewer = auth.viewer();
    let req = request("recent", q);
    Ok(Json(
        state
            .db
            .read(move |c| wikanvas_store::lists::recent(c, &viewer, &req))
            .await?,
    ))
}

/// Items at every depth of every collection, most recently updated first.
#[utoipa::path(
    get,
    path = "/updated",
    tag = "lists",
    operation_id = "list_updated",
    security(("bearer" = ["read"]), ("session" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page", body = Page<ListedItem>),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `cursor`, `limit` or `query`)", body = ErrorBody),
    )
)]
async fn list_updated(
    auth: Auth<Read>,
    State(state): State<AppState>,
    Query(q): Query<PageQuery>,
) -> Result<Json<Page<ListedItem>>, ApiError> {
    authz::require(&state, &auth, Action::Read, Resource::Workspace).await?;
    let viewer = auth.viewer();
    let req = request("updated", q);
    Ok(Json(
        state
            .db
            .read(move |c| wikanvas_store::lists::updated(c, &viewer, None, &req))
            .await?,
    ))
}

/// A collection's items at every depth, most recently updated first.
#[utoipa::path(
    get,
    path = "/collections/{id}/updated",
    tag = "lists",
    operation_id = "list_collection_updated",
    security(("bearer" = ["read"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The collection's id"), PageQuery),
    responses(
        (status = 200, description = "One page", body = Page<ListedItem>),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 404, description = "`not_found` (also while the collection is in the trash)", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `cursor`, `limit` or `query`)", body = ErrorBody),
    )
)]
async fn list_collection_updated(
    auth: Auth<Read>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
    Query(q): Query<PageQuery>,
) -> Result<Json<Page<ListedItem>>, ApiError> {
    let id = id.to_string();
    authz::require(&state, &auth, Action::Read, Resource::Collection(&id)).await?;
    let viewer = auth.viewer();
    let req = request("collection_updated", q).param("collection_id", &id);
    Ok(Json(
        state
            .db
            .read(move |c| {
                wikanvas_store::collections::get_live(c, &viewer, &id)?;
                wikanvas_store::lists::updated(c, &viewer, Some(&id), &req)
            })
            .await?,
    ))
}

/// The trash, roots only: deleted collections and item roots, each with its purge date (30 days after deletion);
/// an item root also with the number of descendants trashed with it and its path.
#[utoipa::path(
    get,
    path = "/trash",
    tag = "lists",
    operation_id = "get_trash",
    security(("bearer" = ["read"]), ("session" = [])),
    responses(
        (status = 200, description = "The trash", body = TrashList),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn get_trash(
    auth: Auth<Read>,
    State(state): State<AppState>,
) -> Result<Json<TrashList>, ApiError> {
    authz::require(&state, &auth, Action::Read, Resource::Workspace).await?;
    let viewer = auth.viewer();
    Ok(Json(
        state
            .db
            .read(move |c| wikanvas_store::lists::trash(c, &viewer))
            .await?,
    ))
}

/// Permanently deletes an item in the trash with its whole subtree.
#[utoipa::path(
    delete,
    path = "/trash/items/{id}",
    tag = "lists",
    operation_id = "purge_trash_item",
    security(("bearer" = ["admin"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The item's id")),
    responses(
        (status = 204, description = "Purged"),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `forbidden` (a `member` or `viewer`), `csrf_rejected`",
            body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`conflict` (not in the trash), `setup_required`", body = ErrorBody),
    )
)]
async fn purge_trash_item(
    auth: Auth<Admin>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
) -> Result<StatusCode, ApiError> {
    authz::require(
        &state,
        &auth,
        Action::Admin,
        Resource::Item(&id.to_string()),
    )
    .await?;
    state
        .db
        .write(move |tx| wikanvas_store::items::purge(tx, &id.to_string()))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Permanently deletes a collection in the trash with every item in it.
#[utoipa::path(
    delete,
    path = "/trash/collections/{id}",
    tag = "lists",
    operation_id = "purge_trash_collection",
    security(("bearer" = ["admin"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The collection's id")),
    responses(
        (status = 204, description = "Purged"),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `forbidden` (a `member` or `viewer`), `csrf_rejected`",
            body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`conflict` (not in the trash), `setup_required`", body = ErrorBody),
    )
)]
async fn purge_trash_collection(
    auth: Auth<Admin>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
) -> Result<StatusCode, ApiError> {
    authz::require(
        &state,
        &auth,
        Action::Admin,
        Resource::Collection(&id.to_string()),
    )
    .await?;
    state
        .db
        .write(move |tx| wikanvas_store::collections::purge(tx, &id.to_string()))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Empties the trash: every item root and every deleted collection.
#[utoipa::path(
    delete,
    path = "/trash",
    tag = "lists",
    operation_id = "empty_trash",
    security(("bearer" = ["admin"]), ("session" = [])),
    responses(
        (status = 204, description = "The trash is empty"),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `forbidden` (a `member` or `viewer`), `csrf_rejected`",
            body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn empty_trash(
    auth: Auth<Admin>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    authz::require(&state, &auth, Action::Admin, Resource::Workspace).await?;
    state.db.write(wikanvas_store::items::purge_all).await?;
    Ok(StatusCode::NO_CONTENT)
}
