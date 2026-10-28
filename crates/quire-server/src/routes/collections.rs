//! Routes /api/v1/collections (owner: W1)
//!
//! §7.2: list, create, get (by id or `url_id`), edit, reorder, soft-delete and restore. A deleted collection exists
//! only in Trash: every route here but restore answers 404 for it. The tree and the per-collection `updated` list
//! live in `routes::tree` and `routes::lists`.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use quire_api::{
    Collection, CollectionDeleted, CollectionPatch, ErrorBody, MoveCollection, NewCollection,
};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::ApiError;
use crate::extract::{Auth, IdPath, Read, Write};
use crate::state::{AppState, now_ms};

/// This module's routes, relative to `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_collections))
        .routes(routes!(create_collection))
        .routes(routes!(get_collection))
        .routes(routes!(patch_collection))
        .routes(routes!(move_collection))
        .routes(routes!(delete_collection))
        .routes(routes!(restore_collection))
}

/// The live collections in sidebar order.
#[utoipa::path(
    get,
    path = "/collections",
    tag = "collections",
    operation_id = "list_collections",
    security(("bearer" = ["read"]), ("session" = [])),
    responses(
        (status = 200, description = "The collections", body = Vec<Collection>),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn list_collections(
    auth: Auth<Read>,
    State(state): State<AppState>,
) -> Result<Json<Vec<Collection>>, ApiError> {
    let viewer = auth.viewer();
    let list = state
        .db
        .read(move |conn| quire_store::collections::list(conn, &viewer))
        .await?;
    Ok(Json(list))
}

/// Creates a collection (last, unless `position` says otherwise).
#[utoipa::path(
    post,
    path = "/collections",
    tag = "collections",
    operation_id = "create_collection",
    security(("bearer" = ["write"]), ("session" = [])),
    request_body = NewCollection,
    responses(
        (status = 201, description = "The new collection", body = Collection),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
        (status = 413, description = "`payload_too_large`", body = ErrorBody),
        (status = 415, description = "`unsupported_media_type`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `name`, `color`, `icon`, `description` or `position`)",
            body = ErrorBody),
    )
)]
async fn create_collection(
    auth: Auth<Write>,
    State(state): State<AppState>,
    Json(new): Json<NewCollection>,
) -> Result<(StatusCode, Json<Collection>), ApiError> {
    let actor_id = auth.into_actor().user_id;
    let now = now_ms();
    let collection = state
        .db
        .write(move |tx| quire_store::collections::create(tx, &actor_id, &new, now))
        .await?;
    Ok((StatusCode::CREATED, Json(collection)))
}

/// A live collection, by id or `url_id`.
#[utoipa::path(
    get,
    path = "/collections/{id}",
    tag = "collections",
    operation_id = "get_collection",
    security(("bearer" = ["read"]), ("session" = [])),
    params(("id" = String, Path, description = "The collection's id or url_id")),
    responses(
        (status = 200, description = "The collection", body = Collection),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 404, description = "`not_found` (also while it is in the trash)", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn get_collection(
    _auth: Auth<Read>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Collection>, ApiError> {
    if uuid::Uuid::parse_str(&id).is_err() && !quire_core::ids::is_url_id(&id) {
        return Err(ApiError::not_found("no such collection"));
    }
    let collection = state
        .db
        .read(move |conn| {
            let id = quire_store::collections::resolve(conn, &id)?;
            quire_store::collections::get_live(conn, &id)
        })
        .await?;
    Ok(Json(collection))
}

/// Edits a collection: name, icon (`null` clears it), colour, description, tree order.
#[utoipa::path(
    patch,
    path = "/collections/{id}",
    tag = "collections",
    operation_id = "patch_collection",
    security(("bearer" = ["write"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The collection's id")),
    request_body = CollectionPatch,
    responses(
        (status = 200, description = "The collection", body = Collection),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected`", body = ErrorBody),
        (status = 404, description = "`not_found` (also while it is in the trash)", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
        (status = 415, description = "`unsupported_media_type`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `name`, `color`, `icon`, `description` or `body`)",
            body = ErrorBody),
    )
)]
async fn patch_collection(
    auth: Auth<Write>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
    Json(patch): Json<CollectionPatch>,
) -> Result<Json<Collection>, ApiError> {
    let actor = auth.into_actor().user_id;
    let now = now_ms();
    let collection = state
        .db
        .write(move |tx| quire_store::collections::update(tx, &id.to_string(), &patch, &actor, now))
        .await?;
    Ok(Json(collection))
}

/// Moves a collection to `position` among the collections.
#[utoipa::path(
    post,
    path = "/collections/{id}/move",
    tag = "collections",
    operation_id = "move_collection",
    security(("bearer" = ["write"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The collection's id")),
    request_body = MoveCollection,
    responses(
        (status = 200, description = "The collection", body = Collection),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected`", body = ErrorBody),
        (status = 404, description = "`not_found` (also while it is in the trash)", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
        (status = 415, description = "`unsupported_media_type`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `position` or `body`)", body = ErrorBody),
    )
)]
async fn move_collection(
    auth: Auth<Write>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
    Json(mv): Json<MoveCollection>,
) -> Result<Json<Collection>, ApiError> {
    let actor = auth.into_actor().user_id;
    let now = now_ms();
    let collection = state
        .db
        .write(move |tx| {
            quire_store::collections::move_to(tx, &id.to_string(), &mv.position, &actor, now)
        })
        .await?;
    Ok(Json(collection))
}

/// Moves a collection to the trash with its live items (it appears in `GET /trash`; items trashed earlier keep their
/// own entries).
#[utoipa::path(
    delete,
    path = "/collections/{id}",
    tag = "collections",
    operation_id = "delete_collection",
    security(("bearer" = ["write"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The collection's id")),
    responses(
        (status = 200, description = "How many items went to the trash with it", body = CollectionDeleted),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected`", body = ErrorBody),
        (status = 404, description = "`not_found` (also when it is already in the trash)", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn delete_collection(
    auth: Auth<Write>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
) -> Result<Json<CollectionDeleted>, ApiError> {
    let actor = auth.into_actor().user_id;
    let now = now_ms();
    let trashed_items = state
        .db
        .write(move |tx| quire_store::collections::delete(tx, &id.to_string(), &actor, now))
        .await?;
    Ok(Json(CollectionDeleted { trashed_items }))
}

/// Restores a collection from the trash, with exactly the items its deletion trashed.
#[utoipa::path(
    post,
    path = "/collections/{id}/restore",
    tag = "collections",
    operation_id = "restore_collection",
    security(("bearer" = ["write"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The collection's id")),
    responses(
        (status = 200, description = "The collection", body = Collection),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`conflict` (not in the trash), `setup_required`", body = ErrorBody),
    )
)]
async fn restore_collection(
    auth: Auth<Write>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
) -> Result<Json<Collection>, ApiError> {
    let actor = auth.into_actor().user_id;
    let now = now_ms();
    let collection = state
        .db
        .write(move |tx| quire_store::collections::restore(tx, &id.to_string(), &actor, now))
        .await?;
    Ok(Json(collection))
}
