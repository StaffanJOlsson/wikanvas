//! Routes /api/v1/sidebar, /collections/{id}/tree and /items/{id}/children (owner: W1)
//!
//! D21: the sidebar loads a whole collection's visible, unarchived tree as one flat list, with
//! `ETag: "t{tree_version}.{viewer_epoch}"` and 304 on a matching `If-None-Match`. `viewer_epoch` is the caller's
//! `users.access_epoch`, which moves whenever their access may have changed (a membership, their role, disabling or
//! enabling the account, D152); a new default access bumps the collection's `tree_version`. So a cached tree, which
//! holds the caller's `my_access`, is never confirmed across a change of access. All three are Read
//! (`authz::require`, §8): on the workspace, the collection and the item.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use wikanvas_api::{CollectionTree, ErrorBody, ItemSummary, Sidebar};

use crate::authz::{self, Action, Resource};
use crate::error::ApiError;
use crate::extract::{Auth, IdPath, Read};
use crate::openapi::if_none_match;
use crate::state::AppState;

/// This module's routes, relative to `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(get_sidebar))
        .routes(routes!(get_tree))
        .routes(routes!(get_children))
}

/// The tree ETag of a collection for a viewer whose access epoch is `viewer_epoch` (D21).
pub fn tree_etag(tree_version: i64, viewer_epoch: i64) -> String {
    format!("\"t{tree_version}.{viewer_epoch}\"")
}

/// The sidebar: the live collections, the caller's stars (visible, unarchived, in star order) and the drafts count.
#[utoipa::path(
    get,
    path = "/sidebar",
    tag = "tree",
    operation_id = "get_sidebar",
    security(("bearer" = ["read"]), ("session" = [])),
    responses(
        (status = 200, description = "The sidebar", body = Sidebar),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn get_sidebar(
    auth: Auth<Read>,
    State(state): State<AppState>,
) -> Result<Json<Sidebar>, ApiError> {
    authz::require(&state, &auth, Action::Read, Resource::Workspace).await?;
    let viewer = auth.viewer();
    let sidebar = state
        .db
        .read(move |conn| wikanvas_store::lists::sidebar(conn, &viewer))
        .await?;
    Ok(Json(sidebar))
}

/// A collection's visible, unarchived items as one flat list, parents before children, siblings in the
/// collection's `child_order`. 304 while `If-None-Match` holds the current ETag.
#[utoipa::path(
    get,
    path = "/collections/{id}/tree",
    tag = "tree",
    operation_id = "get_tree",
    security(("bearer" = ["read"]), ("session" = [])),
    params(
        ("id" = uuid::Uuid, Path, description = "The collection's id"),
        ("If-None-Match" = Option<String>, Header, description = "A previous ETag: 304 while the tree is unchanged"),
    ),
    responses(
        (status = 200, description = "The tree (`Cache-Control: no-cache`)", body = CollectionTree,
            headers(("ETag" = String, description = "`\"t{tree_version}.{viewer_epoch}\"`"))),
        (status = 304, description = "Unchanged since the ETag in `If-None-Match`"),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 404, description = "`not_found` (also while the collection is in the trash)", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn get_tree(
    auth: Auth<Read>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    authz::require(
        &state,
        &auth,
        Action::Read,
        Resource::Collection(&id.to_string()),
    )
    .await?;
    let viewer = auth.viewer();
    let (etag, tree) = state
        .db
        .read(move |conn| {
            let id = id.to_string();
            let version = wikanvas_store::tree::tree_version(conn, &id)?;
            let etag = tree_etag(
                version,
                wikanvas_store::access::epoch(conn, &viewer.user_id)?,
            );
            if if_none_match(&headers, &etag) {
                return Ok((etag, None));
            }
            Ok((
                etag,
                Some(wikanvas_store::tree::collection_tree(conn, &viewer, &id)?),
            ))
        })
        .await?;
    let mut res = match tree {
        None => StatusCode::NOT_MODIFIED.into_response(),
        Some(tree) => Json(tree).into_response(),
    };
    let h = res.headers_mut();
    h.insert(header::ETAG, HeaderValue::from_str(&etag).expect("ASCII"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    Ok(res)
}

/// An item's visible children in its collection's `child_order` (an archived item's archived children).
#[utoipa::path(
    get,
    path = "/items/{id}/children",
    tag = "tree",
    operation_id = "get_children",
    security(("bearer" = ["read"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The item's id")),
    responses(
        (status = 200, description = "The children", body = Vec<ItemSummary>),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn get_children(
    auth: Auth<Read>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
) -> Result<Json<Vec<ItemSummary>>, ApiError> {
    authz::require(&state, &auth, Action::Read, Resource::Item(&id.to_string())).await?;
    let viewer = auth.viewer();
    let children = state
        .db
        .read(move |conn| wikanvas_store::tree::children(conn, &viewer, &id.to_string()))
        .await?;
    Ok(Json(children))
}
