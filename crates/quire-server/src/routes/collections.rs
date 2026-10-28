//! Routes /api/v1/collections (owner: W1)
//!
//! W0b: `POST /collections` (W1 completes it and adds the rest of the §7.2 collection routes).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use quire_api::{Collection, ErrorBody, NewCollection};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::ApiError;
use crate::extract::{Auth, Write};
use crate::state::{AppState, now_ms};

/// This module's routes, relative to `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(create_collection))
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
        (status = 422, description = "`validation` (`field`: `name`, `color`, `description` or `position`)",
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
