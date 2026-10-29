//! Routes /api/v1/invites: invite links, the only way to a new account (D148)
//!
//! §7.2, §8:
//! - `POST /invites`, `GET /invites`, `DELETE /invites/{id}` (`admin` scope, [`Action::Admin`]): create one (an
//!   admin invite only by the owner, `wikanvas_store::invites::create`), list the pending ones (never a token), revoke
//!   one. The create response is the only place the link `<public URL>/invite/<token>` appears.
//! - `GET /invites/{id}` and `POST /invites/{id}/accept` (public), where the `id` is the invite's token: what the
//!   invite page shows (its state, the workspace name and the role, never who made it), and the accept, which creates
//!   the account with the invite's role and signs it in like setup (a browser-session cookie). The path parameter is
//!   named `id` because axum's router allows one name per path position (§7.1).
//!
//! The accept shares the per-IP limiter of login and setup (5 per minute), and like them takes the CSRF and Host rules
//! of every cookie mutation (`crate::csrf`). A lookup of a token that no invite has counts against the client IP like
//! a failed bearer authentication (30 per minute, then 429 `auth_failures`).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use wikanvas_api::{
    AcceptInvite, CreatedInvite, ErrorBody, Invite, InviteLookup, InviteState, NewInvite,
    RateLimitReason, UserEnvelope,
};
use wikanvas_store::invites::{self, Acceptance};
use wikanvas_store::users::Manager;

use crate::authz::{self, Action, Resource};
use crate::error::ApiError;
use crate::extract::{Admin, Auth, ClientInfo, IdPath};
use crate::state::{AppState, check_password};

/// This module's routes, relative to `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(create_invite))
        .routes(routes!(list_invites))
        .routes(routes!(revoke_invite))
        .routes(routes!(get_invite))
        .routes(routes!(accept_invite))
}

/// The link of an invite: `<public URL>/invite/<token>`.
pub fn invite_url(state: &AppState, token: &str) -> String {
    format!("{}/invite/{token}", state.config.public_url)
}

/// Creates an invite link for a new `admin` (the owner only), `member` or `viewer`; the response is the only place
/// its link appears. It works once and expires after `expires_in_days` (1–30, default 7).
#[utoipa::path(
    post,
    path = "/invites",
    tag = "invites",
    operation_id = "create_invite",
    security(("bearer" = ["admin"]), ("session" = [])),
    request_body = NewInvite,
    responses(
        (status = 201, description = "The invite and its link", body = CreatedInvite),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `forbidden` (a `member` or `viewer`; an admin invite by \
            an admin), `csrf_rejected`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
        (status = 415, description = "`unsupported_media_type`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `role`, `expires_in_days`, `note`, or `invites` at 100 \
            pending invites)", body = ErrorBody),
    )
)]
async fn create_invite(
    auth: Auth<Admin>,
    State(state): State<AppState>,
    Json(new): Json<NewInvite>,
) -> Result<(StatusCode, Json<CreatedInvite>), ApiError> {
    authz::require(&state, &auth, Action::Admin, Resource::Workspace).await?;
    let actor = auth.into_actor();
    let now = state.now();
    let created = state
        .db
        .write(move |tx| {
            invites::create(
                tx,
                Manager::User {
                    id: &actor.user_id,
                    role: actor.role,
                },
                &new,
                now,
            )
        })
        .await?;
    let url = invite_url(&state, &created.token);
    Ok((
        StatusCode::CREATED,
        Json(CreatedInvite {
            invite: created.invite.to_api()?,
            url,
        }),
    ))
}

/// The pending invites (not used, revoked or expired), newest first, at most 100. Never a token or a link.
#[utoipa::path(
    get,
    path = "/invites",
    tag = "invites",
    operation_id = "list_invites",
    security(("bearer" = ["admin"]), ("session" = [])),
    responses(
        (status = 200, description = "The pending invites", body = Vec<Invite>),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `forbidden` (a `member` or `viewer`)", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn list_invites(
    auth: Auth<Admin>,
    State(state): State<AppState>,
) -> Result<Json<Vec<Invite>>, ApiError> {
    authz::require(&state, &auth, Action::Admin, Resource::Workspace).await?;
    let now = state.now();
    let list = state
        .db
        .read(move |conn| {
            invites::list_pending(conn, now)?
                .iter()
                .map(|i| i.to_api())
                .collect()
        })
        .await?;
    Ok(Json(list))
}

/// Revokes a pending invite: its link stops working at once.
#[utoipa::path(
    delete,
    path = "/invites/{id}",
    tag = "invites",
    operation_id = "revoke_invite",
    security(("bearer" = ["admin"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The invite's id")),
    responses(
        (status = 204, description = "Revoked"),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `forbidden` (a `member` or `viewer`), `csrf_rejected`",
            body = ErrorBody),
        (status = 404, description = "`not_found`: no pending invite has this id", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn revoke_invite(
    auth: Auth<Admin>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
) -> Result<StatusCode, ApiError> {
    let id = id.to_string();
    authz::require(&state, &auth, Action::Admin, Resource::Invite(&id)).await?;
    let now = state.now();
    let n = state
        .db
        .write(move |tx| invites::revoke(tx, &id, now))
        .await?;
    if n == 0 {
        return Err(ApiError::not_found(
            "no pending invite has this id (it was used, revoked or has expired)",
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// A lookup of a token no invite has: 404, or 429 once the client IP failed 30 times within a minute.
fn unknown_invite(state: &AppState, client: ClientInfo) -> ApiError {
    match state.auth_fail_limiter.record_failure(client.limiter_ip()) {
        Ok(()) => invites::not_pending(None).into(),
        Err(retry) => ApiError::rate_limited(RateLimitReason::AuthFailures, retry),
    }
}

/// What the invite page shows (public; the `id` is the invite's token, the last segment of its link): whether it can
/// be accepted, and else why, the workspace's name and the role. Never who made it.
#[utoipa::path(
    get,
    path = "/invites/{id}",
    tag = "invites",
    operation_id = "get_invite",
    security(()),
    params(("id" = String, Path, description = "The invite's token, `qinv_…` (the last segment of its link)")),
    responses(
        (status = 200, description = "The invite (any state)", body = InviteLookup),
        (status = 404, description = "`not_found`: no invite has this token", body = ErrorBody),
        (status = 429, description = "`rate_limited` (`reason: auth_failures`, per IP)", body = ErrorBody,
            headers(("Retry-After" = u64, description = "Seconds"))),
    )
)]
async fn get_invite(
    State(state): State<AppState>,
    client: ClientInfo,
    Path(token): Path<String>,
) -> Result<Json<InviteLookup>, ApiError> {
    let now = state.now();
    let found = state
        .db
        .read(move |conn| {
            let Some(invite) = invites::lookup(conn, &token)? else {
                return Ok(None);
            };
            let workspace = wikanvas_store::settings::workspace(conn)?;
            Ok(Some((invite, workspace.name)))
        })
        .await?;
    let Some((invite, workspace_name)) = found else {
        return Err(unknown_invite(&state, client));
    };
    let invite_state = invite.state(now);
    Ok(Json(InviteLookup {
        valid: invite_state == InviteState::Pending,
        state: invite_state,
        workspace_name,
        role: invite.role,
        expires_at: wikanvas_api::Timestamp(invite.expires_at),
    }))
}

/// Accepts an invite (public; the `id` is its token): creates the account with the invite's role and signs it in
/// (a browser-session cookie, as setup does). Works once. Checked in this order: the per-IP limiter of login and
/// setup (429), the invite (404 unless it is pending), the fields (`email`, `name`, `password`; 422), then a taken
/// email (409).
#[utoipa::path(
    post,
    path = "/invites/{id}/accept",
    tag = "invites",
    operation_id = "accept_invite",
    security(()),
    params(("id" = String, Path, description = "The invite's token, `qinv_…` (the last segment of its link)")),
    request_body = AcceptInvite,
    responses(
        (status = 201, description = "The new account, signed in", body = UserEnvelope,
            headers(("Set-Cookie" = String, description = "The session cookie"))),
        (status = 403, description = "`csrf_rejected`", body = ErrorBody),
        (status = 404, description = "`not_found`: no pending invite has this token (unknown, used, expired or \
            revoked)", body = ErrorBody),
        (status = 409, description = "`conflict` (`field: email`: another account has this email)",
            body = ErrorBody),
        (status = 415, description = "`unsupported_media_type`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `email`, `name` or `password`)", body = ErrorBody),
        (status = 429, description = "`rate_limited` (`reason: requests`, per IP)", body = ErrorBody,
            headers(("Retry-After" = u64, description = "Seconds"))),
    )
)]
async fn accept_invite(
    State(state): State<AppState>,
    client: ClientInfo,
    headers: HeaderMap,
    Path(token): Path<String>,
    Json(req): Json<AcceptInvite>,
) -> Result<Response, ApiError> {
    state
        .login_limiter
        .attempt(client.limiter_ip())
        .map_err(|retry| ApiError::rate_limited(RateLimitReason::Requests, retry))?;
    let now = state.now();
    let lookup = token.clone();
    let found = state
        .db
        .read(move |conn| invites::lookup(conn, &lookup))
        .await?;
    let invite_state = found.as_ref().map(|i| i.state(now));
    if invite_state != Some(InviteState::Pending) {
        return Err(invites::not_pending(invite_state).into());
    }
    wikanvas_store::users::check_email(req.email.trim())?;
    wikanvas_store::users::check_name(&req.name)?;
    check_password(&req.password)?;
    let password_hash = state.hash_password(req.password).await?;
    let who = Acceptance {
        email: req.email,
        name: req.name,
        password_hash,
    };
    let now = state.now();
    let user = state
        .db
        .write(move |tx| invites::accept(tx, &token, &who, now)?.to_api())
        .await?;
    tracing::info!(email = %user.email, role = %user.role, "an invite was accepted");
    let cookie =
        crate::routes::auth::sign_in(&state, user.id.to_string(), false, client, &headers).await?;
    let mut res = (StatusCode::CREATED, Json(UserEnvelope { user })).into_response();
    res.headers_mut().append(header::SET_COOKIE, cookie);
    Ok(res)
}
