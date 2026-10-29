//! Routes /api/v1/users: the accounts of the workspace (D148)
//!
//! §7.2, §8: `GET /users` lists every account, disabled ones included; `PATCH /users/{id}` sets its role and/or
//! disables or enables it. Both are `admin` scope and [`Action::Admin`] in `authz::require` (the owner and admins);
//! the finer rules (nobody changes the owner, their own account, or, unless they are the owner, an admin) are
//! `wikanvas_store::users::change`, which knows the target's role. Disabling ends the account's sessions, API tokens
//! and pending invites in the same transaction, so its next request is 401.

use axum::Json;
use axum::extract::State;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use wikanvas_api::{ErrorBody, UserInfo, UserPatch};
use wikanvas_store::users::{Manager, UserChange};

use crate::authz::{self, Action, Resource};
use crate::error::ApiError;
use crate::extract::{Admin, Auth, IdPath};
use crate::state::AppState;

/// This module's routes, relative to `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_users))
        .routes(routes!(patch_user))
}

/// Every account, the owner first, then in the order they joined; disabled ones too.
#[utoipa::path(
    get,
    path = "/users",
    tag = "users",
    operation_id = "list_users",
    security(("bearer" = ["admin"]), ("session" = [])),
    responses(
        (status = 200, description = "The accounts", body = Vec<UserInfo>),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `forbidden` (a `member` or `viewer`)", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn list_users(
    auth: Auth<Admin>,
    State(state): State<AppState>,
) -> Result<Json<Vec<UserInfo>>, ApiError> {
    authz::require(&auth, Action::Admin, Resource::Workspace)?;
    let users = state
        .db
        .read(|conn| {
            wikanvas_store::users::list(conn)?
                .iter()
                .map(|u| u.to_info())
                .collect()
        })
        .await?;
    Ok(Json(users))
}

/// Changes an account's role (`admin`, `member` or `viewer`) and/or disables (`disabled: true`) or enables it.
///
/// The owner's role never changes and the owner cannot be disabled; nobody changes their own role or disables
/// themselves; only the owner makes, changes or disables an admin; admins manage members and viewers (403
/// `forbidden`). Disabling ends the account's sessions and API tokens and revokes the invites it made, at once;
/// taking the admin role away revokes those invites too. An empty body, or what the account already is, changes
/// nothing.
#[utoipa::path(
    patch,
    path = "/users/{id}",
    tag = "users",
    operation_id = "patch_user",
    security(("bearer" = ["admin"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The account's id")),
    request_body = UserPatch,
    responses(
        (status = 200, description = "The account", body = UserInfo),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `forbidden` (the role, or the rules above), \
            `csrf_rejected`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
        (status = 415, description = "`unsupported_media_type`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `role`, which is never `owner`)", body = ErrorBody),
    )
)]
async fn patch_user(
    auth: Auth<Admin>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
    Json(patch): Json<UserPatch>,
) -> Result<Json<UserInfo>, ApiError> {
    let id = id.to_string();
    authz::require(&auth, Action::Admin, Resource::User(&id))?;
    let actor = auth.into_actor();
    let now = state.now();
    let user = state
        .db
        .write(move |tx| {
            let changed = wikanvas_store::users::change(
                tx,
                Manager::User {
                    id: &actor.user_id,
                    role: actor.role,
                },
                &id,
                &UserChange {
                    role: patch.role,
                    disabled: patch.disabled,
                },
                now,
            )?;
            if changed.sessions_revoked + changed.tokens_revoked + changed.invites_revoked > 0 {
                tracing::info!(
                    user = %changed.user.id,
                    sessions = changed.sessions_revoked,
                    tokens = changed.tokens_revoked,
                    invites = changed.invites_revoked,
                    "account disabled or demoted: its credentials were revoked"
                );
            }
            changed.user.to_info()
        })
        .await?;
    Ok(Json(user))
}
