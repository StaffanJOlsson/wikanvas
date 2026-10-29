//! Routes /api/v1/collections (owner: W1; access and members: D152)
//!
//! §7.2: list, create, get (by id or `url_id`), edit, reorder, soft-delete and restore, and the members. A deleted
//! collection exists only in Trash: every route here but restore answers 404 for it. The tree lives in
//! `routes::tree`; the per-collection `updated` list and purging live in `routes::lists`. `authz::require` (§8): Read
//! for the GETs but the members list, Write on the workspace to create one, and Manage on the collection for the rest
//! (its settings and default access, its place, moving it to the trash and restoring it, and its members). A
//! collection the caller may not read is 404, as one that does not exist.

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use wikanvas_api::{
    Collection, CollectionDeleted, CollectionMember, CollectionPatch, ErrorBody, MoveCollection,
    NewCollection, SetMember,
};

use crate::authz::{self, Action, Resource};
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
        .routes(routes!(list_members))
        .routes(routes!(set_member))
        .routes(routes!(remove_member))
}

/// The `{user_id}` of a members route: a UUID, else 404 (§7.1).
fn user_id(params: &HashMap<String, String>) -> Result<String, ApiError> {
    params
        .get("user_id")
        .and_then(|u| uuid::Uuid::parse_str(u).ok())
        .map(|u| u.to_string())
        .ok_or_else(|| ApiError::not_found("no such user"))
}

/// The live collections the caller may read, in sidebar order, each with the caller's `my_access`.
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
    authz::require(&state, &auth, Action::Read, Resource::Workspace).await?;
    let viewer = auth.viewer();
    let list = state
        .db
        .read(move |conn| wikanvas_store::collections::list(conn, &viewer))
        .await?;
    Ok(Json(list))
}

/// Creates a collection (last, unless `position` says otherwise), with `default_access` (omitted: `write`). The
/// creator gets `manage` on it through a membership.
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
        (status = 403, description = "`insufficient_scope`, `forbidden` (a `viewer`), `csrf_rejected`",
            body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
        (status = 413, description = "`payload_too_large`", body = ErrorBody),
        (status = 415, description = "`unsupported_media_type`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `name`, `color`, `icon`, `description`, `position` or \
            `body`)", body = ErrorBody),
    )
)]
async fn create_collection(
    auth: Auth<Write>,
    State(state): State<AppState>,
    Json(new): Json<NewCollection>,
) -> Result<(StatusCode, Json<Collection>), ApiError> {
    authz::require(&state, &auth, Action::Write, Resource::Workspace).await?;
    let viewer = auth.viewer();
    let now = now_ms();
    let collection = state
        .db
        .write(move |tx| wikanvas_store::collections::create(tx, &viewer, &new, now))
        .await?;
    Ok((StatusCode::CREATED, Json(collection)))
}

/// A live collection, by id or `url_id`, with the caller's `my_access`.
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
        (status = 404, description = "`not_found` (also while it is in the trash, and for a collection the caller may \
            not read)", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn get_collection(
    auth: Auth<Read>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Collection>, ApiError> {
    authz::require(&state, &auth, Action::Read, Resource::Collection(&id)).await?;
    if uuid::Uuid::parse_str(&id).is_err() && !wikanvas_core::ids::is_url_id(&id) {
        return Err(ApiError::not_found("no such collection"));
    }
    let viewer = auth.viewer();
    let collection = state
        .db
        .read(move |conn| {
            let id = wikanvas_store::collections::resolve(conn, &id)?;
            wikanvas_store::collections::get_live(conn, &viewer, &id)
        })
        .await?;
    Ok(Json(collection))
}

/// Edits a collection: name, icon (`null` clears it), colour, description, tree order, default access (`manage`).
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
        (status = 403, description = "`insufficient_scope`, `forbidden` (no `manage` on it), `csrf_rejected`",
            body = ErrorBody),
        (status = 404, description = "`not_found` (also while it is in the trash, and for a collection the caller may \
            not read)", body = ErrorBody),
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
    authz::require(
        &state,
        &auth,
        Action::Manage,
        Resource::Collection(&id.to_string()),
    )
    .await?;
    let viewer = auth.viewer();
    let now = now_ms();
    let collection = state
        .db
        .write(move |tx| {
            wikanvas_store::collections::update(tx, &viewer, &id.to_string(), &patch, now)
        })
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
        (status = 403, description = "`insufficient_scope`, `forbidden` (no `manage` on it), `csrf_rejected`",
            body = ErrorBody),
        (status = 404, description = "`not_found` (also while it is in the trash, and for a collection the caller may \
            not read)", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
        (status = 415, description = "`unsupported_media_type`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `position` (also for a collection the caller may not \
            read) or `body`)", body = ErrorBody),
    )
)]
async fn move_collection(
    auth: Auth<Write>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
    Json(mv): Json<MoveCollection>,
) -> Result<Json<Collection>, ApiError> {
    authz::require(
        &state,
        &auth,
        Action::Manage,
        Resource::Collection(&id.to_string()),
    )
    .await?;
    let viewer = auth.viewer();
    let now = now_ms();
    let collection = state
        .db
        .write(move |tx| {
            wikanvas_store::collections::move_to(tx, &viewer, &id.to_string(), &mv.position, now)
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
        (status = 403, description = "`insufficient_scope`, `forbidden` (no `manage` on it), `csrf_rejected`",
            body = ErrorBody),
        (status = 404, description = "`not_found` (also when it is already in the trash, and for a collection the \
            caller may not read)", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn delete_collection(
    auth: Auth<Write>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
) -> Result<Json<CollectionDeleted>, ApiError> {
    authz::require(
        &state,
        &auth,
        Action::Manage,
        Resource::Collection(&id.to_string()),
    )
    .await?;
    let actor = auth.into_actor().user_id;
    let now = now_ms();
    let trashed_items = state
        .db
        .write(move |tx| wikanvas_store::collections::delete(tx, &id.to_string(), &actor, now))
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
        (status = 403, description = "`insufficient_scope`, `forbidden` (no `manage` on it), `csrf_rejected`",
            body = ErrorBody),
        (status = 404, description = "`not_found` (also for a collection the caller may not read)", body = ErrorBody),
        (status = 409, description = "`conflict` (not in the trash), `setup_required`", body = ErrorBody),
    )
)]
async fn restore_collection(
    auth: Auth<Write>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
) -> Result<Json<Collection>, ApiError> {
    authz::require(
        &state,
        &auth,
        Action::Manage,
        Resource::Collection(&id.to_string()),
    )
    .await?;
    let viewer = auth.viewer();
    let now = now_ms();
    let collection = state
        .db
        .write(move |tx| wikanvas_store::collections::restore(tx, &viewer, &id.to_string(), now))
        .await?;
    Ok(Json(collection))
}

/// The collection's members: who has more than its default access (or any, for a `none` collection), with what
/// that gives each (`effective_access`). The owner and admins manage every collection whether listed or not.
#[utoipa::path(
    get,
    path = "/collections/{id}/members",
    tag = "collections",
    operation_id = "list_members",
    security(("bearer" = ["read"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The collection's id")),
    responses(
        (status = 200, description = "The members, by name", body = Vec<CollectionMember>),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `forbidden` (no `manage` on it)", body = ErrorBody),
        (status = 404, description = "`not_found` (also while it is in the trash, and for a collection the caller may \
            not read)", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn list_members(
    auth: Auth<Read>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
) -> Result<Json<Vec<CollectionMember>>, ApiError> {
    let id = id.to_string();
    authz::require(&state, &auth, Action::Manage, Resource::Collection(&id)).await?;
    let viewer = auth.viewer();
    let members = state
        .db
        .read(move |conn| {
            wikanvas_store::collections::get_live(conn, &viewer, &id)?;
            wikanvas_store::access::members(conn, &id)
        })
        .await?;
    Ok(Json(members))
}

/// Adds a person to the collection with `access`, or changes their access. Their role still caps it: a `viewer`
/// reads at most. A disabled account cannot be added.
#[utoipa::path(
    put,
    path = "/collections/{id}/members/{user_id}",
    tag = "collections",
    operation_id = "set_member",
    security(("bearer" = ["write"]), ("session" = [])),
    params(
        ("id" = uuid::Uuid, Path, description = "The collection's id"),
        ("user_id" = uuid::Uuid, Path, description = "The person's id (`GET /directory`)"),
    ),
    request_body = SetMember,
    responses(
        (status = 200, description = "The membership", body = CollectionMember),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `forbidden` (no `manage` on it), `csrf_rejected`",
            body = ErrorBody),
        (status = 404, description = "`not_found`: no such user, or the collection is in the trash or the caller may \
            not read it", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
        (status = 415, description = "`unsupported_media_type`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `user_id` for a disabled account, or `body`)",
            body = ErrorBody),
    )
)]
async fn set_member(
    auth: Auth<Write>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
    Path(params): Path<HashMap<String, String>>,
    Json(set): Json<SetMember>,
) -> Result<Json<CollectionMember>, ApiError> {
    let id = id.to_string();
    authz::require(&state, &auth, Action::Manage, Resource::Collection(&id)).await?;
    let user = user_id(&params)?;
    let viewer = auth.viewer();
    let now = now_ms();
    let member = state
        .db
        .write(move |tx| {
            wikanvas_store::collections::get_live(tx, &viewer, &id)?;
            wikanvas_store::access::set_member(
                tx,
                &id,
                &user,
                set.access,
                Some(&viewer.user_id),
                now,
            )
        })
        .await?;
    Ok(Json(member))
}

/// Removes a person from the collection (also when they are not a member): they keep its default access.
#[utoipa::path(
    delete,
    path = "/collections/{id}/members/{user_id}",
    tag = "collections",
    operation_id = "remove_member",
    security(("bearer" = ["write"]), ("session" = [])),
    params(
        ("id" = uuid::Uuid, Path, description = "The collection's id"),
        ("user_id" = uuid::Uuid, Path, description = "The person's id"),
    ),
    responses(
        (status = 204, description = "Not a member"),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `forbidden` (no `manage` on it), `csrf_rejected`",
            body = ErrorBody),
        (status = 404, description = "`not_found` (also while it is in the trash, and for a collection the caller may \
            not read)", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn remove_member(
    auth: Auth<Write>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
    Path(params): Path<HashMap<String, String>>,
) -> Result<StatusCode, ApiError> {
    let id = id.to_string();
    authz::require(&state, &auth, Action::Manage, Resource::Collection(&id)).await?;
    let user = user_id(&params)?;
    let viewer = auth.viewer();
    state
        .db
        .write(move |tx| {
            wikanvas_store::collections::get_live(tx, &viewer, &id)?;
            wikanvas_store::access::remove_member(tx, &id, &user)
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
