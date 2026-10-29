//! Routes /api/v1/workspace (owner: W2)
//!
//! §7.2: `GET` (read) returns `{name, logo_url}`; `PATCH` (admin) sets the name (1–100 characters) or the logo (an
//! existing image attachment; `null` removes it). The logo attachment is a GC root while it is set (§5).

use axum::Json;
use axum::extract::State;
use serde_json::Value;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use wikanvas_api::{ErrorBody, Workspace, WorkspacePatch};

use crate::authz::{self, Action, Resource};
use crate::error::ApiError;
use crate::extract::{Admin, Auth, Read};
use crate::state::AppState;

/// The media types a logo may have (shown inline, §11).
const LOGO_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/avif",
];

/// This module's routes, relative to `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(get_workspace))
        .routes(routes!(patch_workspace))
}

/// The workspace's name and logo.
#[utoipa::path(
    get,
    path = "/workspace",
    tag = "workspace",
    operation_id = "get_workspace",
    security(("bearer" = ["read"]), ("session" = [])),
    responses(
        (status = 200, description = "The workspace", body = Workspace),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn get_workspace(
    auth: Auth<Read>,
    State(state): State<AppState>,
) -> Result<Json<Workspace>, ApiError> {
    authz::require(&auth, Action::Read, Resource::Workspace)?;
    Ok(Json(
        state.db.read(wikanvas_store::settings::workspace).await?,
    ))
}

/// Renames the workspace or sets its logo (`logo_attachment_id: null` removes it).
#[utoipa::path(
    patch,
    path = "/workspace",
    tag = "workspace",
    operation_id = "patch_workspace",
    security(("bearer" = ["admin"]), ("session" = [])),
    request_body = WorkspacePatch,
    responses(
        (status = 200, description = "The workspace", body = Workspace),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
        (status = 415, description = "`unsupported_media_type`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `name` or `logo_attachment_id`)", body = ErrorBody),
    )
)]
async fn patch_workspace(
    auth: Auth<Admin>,
    State(state): State<AppState>,
    Json(patch): Json<WorkspacePatch>,
) -> Result<Json<Workspace>, ApiError> {
    authz::require(&auth, Action::Admin, Resource::Workspace)?;
    if let Some(name) = &patch.name
        && !(1..=100).contains(&name.trim().chars().count())
    {
        return Err(ApiError::validation(
            "name",
            "a workspace name is 1 to 100 characters",
        ));
    }
    let workspace = state
        .db
        .write(move |tx| {
            if let Some(name) = &patch.name {
                wikanvas_store::settings::set(
                    tx,
                    wikanvas_store::settings::WORKSPACE_NAME,
                    &Value::String(name.trim().to_owned()),
                )?;
            }
            match patch.logo_attachment_id {
                None => {}
                Some(None) => {
                    wikanvas_store::settings::delete(tx, wikanvas_store::settings::WORKSPACE_LOGO)?
                }
                Some(Some(id)) => {
                    let id = id.to_string();
                    let found = wikanvas_store::attachments::get(tx, &id)?;
                    if !found.is_some_and(|a| LOGO_TYPES.contains(&a.mime.as_str())) {
                        return Err(wikanvas_store::Error::validation(
                            "logo_attachment_id",
                            "the logo must be an uploaded PNG, JPEG, GIF, WebP or AVIF image",
                        ));
                    }
                    wikanvas_store::settings::set(
                        tx,
                        wikanvas_store::settings::WORKSPACE_LOGO,
                        &Value::String(id),
                    )?;
                }
            }
            wikanvas_store::settings::workspace(tx)
        })
        .await?;
    Ok(Json(workspace))
}
