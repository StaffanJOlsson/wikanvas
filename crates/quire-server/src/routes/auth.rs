//! Routes under /api/v1/auth: login, logout, me, password, sessions (owner: W2)
//!
//! §7.2, §8:
//! - `POST /auth/login` (public): 5 attempts per minute per client IP, 10 failed attempts per 15 minutes per email;
//!   argon2id under the shared permit, with a dummy hash for an unknown email so the timing is the same; always the
//!   generic `invalid_credentials`. Success creates a session and sets the cookie named for the request's scheme.
//! - `POST /auth/logout` (session): deletes the session and clears the cookie.
//! - `GET /auth/me` (read) and `PATCH /auth/me` (admin; an email change needs a session).
//! - `POST /auth/password` (session): revokes every other session; tokens stay (separate credentials).
//! - `GET /auth/sessions`, `DELETE /auth/sessions/{id}`, `DELETE /auth/sessions?others=true` (admin; from a token,
//!   `others=true` revokes every session).

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use quire_api::{
    AuthMethodKind, ErrorBody, ErrorCode, LoginRequest, Me, MeAuth, MePatch, PasswordChange,
    RateLimitReason, SessionInfo, SessionRevokeQuery, User, UserEnvelope, Workspace,
};
use quire_store::sessions::NewSession;
use quire_store::users::ProfilePatch;
use rusqlite::Connection;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::ApiError;
use crate::extract::{
    Admin, Auth, AuthMethod, ClientInfo, IdPath, Read, Session, clear_session_cookie, cookie_value,
    session_cookie, session_cookie_name,
};
use crate::state::{AppState, check_password_field};

/// This module's routes, relative to `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(login))
        .routes(routes!(logout))
        .routes(routes!(get_me))
        .routes(routes!(patch_me))
        .routes(routes!(change_password))
        .routes(routes!(list_sessions))
        .routes(routes!(revoke_sessions))
        .routes(routes!(revoke_session))
}

/// A user row: the API shape plus `must_change_password`.
pub fn load_user(conn: &Connection, user_id: &str) -> quire_store::Result<(User, bool)> {
    let row = quire_store::users::get(conn, user_id)?;
    Ok((row.to_api()?, row.must_change_password))
}

/// The workspace's name and logo URL from the settings.
pub fn load_workspace(conn: &Connection) -> quire_store::Result<Workspace> {
    quire_store::settings::workspace(conn)
}

/// The `User-Agent` of a request, for `sessions.user_agent`.
fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// Creates a session for `user_id` and returns the `Set-Cookie` value (§8). Used by login and setup.
pub async fn sign_in(
    state: &AppState,
    user_id: String,
    persistent: bool,
    client: ClientInfo,
    headers: &HeaderMap,
) -> Result<HeaderValue, ApiError> {
    let now = state.now();
    let new = NewSession {
        user_id,
        persistent,
        user_agent: user_agent(headers),
        ip: client.ip.map(|ip| ip.to_string()),
    };
    // Login rotates the token: a session this browser still presents is ended.
    let previous = cookie_value(headers, session_cookie_name(client.https)).map(str::to_owned);
    let (row, cookie) = state
        .db
        .write(move |tx| {
            if let Some(old) = previous
                && let Some(old) = quire_store::sessions::lookup(tx, &old, now)?
            {
                quire_store::sessions::delete(tx, &old.user_id, &old.id)?;
            }
            quire_store::sessions::create(tx, &new, now)
        })
        .await?;
    Ok(session_cookie(client.https, &cookie, row.max_age_secs(now)))
}

/// Signs in with email and password and sets the session cookie (`__Host-quire_session; Secure` over HTTPS behind
/// the proxy, `quire_session` otherwise; `Max-Age` only with `remember`).
#[utoipa::path(
    post,
    path = "/auth/login",
    tag = "auth",
    operation_id = "login",
    security(()),
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Signed in", body = UserEnvelope,
            headers(("Set-Cookie" = String, description = "The session cookie"))),
        (status = 401, description = "`invalid_credentials`", body = ErrorBody),
        (status = 403, description = "`csrf_rejected`", body = ErrorBody),
        (status = 415, description = "`unsupported_media_type`", body = ErrorBody),
        (status = 422, description = "`validation`", body = ErrorBody),
        (status = 429, description = "`rate_limited` (`reason`: `requests` per IP, `auth_failures` per email)",
            body = ErrorBody, headers(("Retry-After" = u64, description = "Seconds"))),
    )
)]
async fn login(
    State(state): State<AppState>,
    client: ClientInfo,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> Result<Response, ApiError> {
    state
        .login_limiter
        .attempt(client.limiter_ip())
        .map_err(|retry| ApiError::rate_limited(RateLimitReason::Requests, retry))?;
    let invalid_credentials = || {
        ApiError::new(
            ErrorCode::InvalidCredentials,
            "the email or password is wrong",
        )
    };
    // An address no account can have (over 254 bytes, no `@`, …) gets the same answer after the same work, and is
    // never recorded: the per-email map holds only addresses that could be real (§8).
    if quire_store::users::check_email(req.email.trim()).is_err() {
        state
            .verify_password(state.dummy_hash().await?, req.password)
            .await?;
        return Err(invalid_credentials());
    }
    state
        .login_limiter
        .per_email
        .check(&req.email)
        .map_err(|retry| ApiError::rate_limited(RateLimitReason::AuthFailures, retry))?;
    let email = req.email.clone();
    let user = state
        .db
        .read(move |conn| quire_store::users::by_email(conn, &email))
        .await?;
    let usable = user
        .as_ref()
        .filter(|u| u.disabled_at.is_none())
        .and_then(|u| u.password_hash.clone().map(|h| (u, h)));
    let (hash, candidate) = match usable {
        Some((u, hash)) => (hash, Some(u)),
        None => (state.dummy_hash().await?, None),
    };
    let verified = state.verify_password(hash, req.password).await?;
    let Some(user) = candidate.filter(|_| verified) else {
        state.login_limiter.per_email.record_failure(&req.email);
        return Err(invalid_credentials());
    };
    let cookie = sign_in(&state, user.id.clone(), req.remember, client, &headers).await?;
    let body = UserEnvelope {
        user: user.to_api()?,
    };
    let mut res = Json(body).into_response();
    res.headers_mut().append(header::SET_COOKIE, cookie);
    Ok(res)
}

/// Signs out: deletes this session and clears the cookie.
#[utoipa::path(
    post,
    path = "/auth/logout",
    tag = "auth",
    operation_id = "logout",
    security(("session" = [])),
    responses(
        (status = 204, description = "Signed out; the cookie is cleared"),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`session_required` (a token), `csrf_rejected`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn logout(
    auth: Auth<Session>,
    State(state): State<AppState>,
    client: ClientInfo,
) -> Result<Response, ApiError> {
    let actor = auth.into_actor();
    let session = actor.session_id().unwrap_or_default().to_owned();
    state
        .db
        .write(move |tx| quire_store::sessions::delete(tx, &actor.user_id, &session))
        .await?;
    let mut res = StatusCode::NO_CONTENT.into_response();
    res.headers_mut()
        .append(header::SET_COOKIE, clear_session_cookie(client.https));
    Ok(res)
}

/// The current user, the workspace, and how this request was authenticated (`auth.token` for a token).
#[utoipa::path(
    get,
    path = "/auth/me",
    tag = "auth",
    operation_id = "get_me",
    security(("bearer" = ["read"]), ("session" = [])),
    responses(
        (status = 200, description = "The caller", body = Me),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn get_me(auth: Auth<Read>, State(state): State<AppState>) -> Result<Json<Me>, ApiError> {
    let actor = auth.into_actor();
    let me = state
        .db
        .read(move |conn| {
            let (user, must_change_password) = load_user(conn, &actor.user_id)?;
            let workspace = load_workspace(conn)?;
            let (method, token) = match &actor.method {
                AuthMethod::Session { .. } => (AuthMethodKind::Session, None),
                AuthMethod::Token { token_id, .. } => (
                    AuthMethodKind::Token,
                    quire_store::tokens::get(conn, &actor.user_id, token_id)?,
                ),
            };
            Ok(Me {
                user,
                workspace,
                must_change_password,
                auth: MeAuth { method, token },
            })
        })
        .await?;
    Ok(Json(me))
}

/// Changes the caller's name, email or theme. Changing the email needs a session (a token gets 403
/// `session_required`).
#[utoipa::path(
    patch,
    path = "/auth/me",
    tag = "auth",
    operation_id = "patch_me",
    security(("bearer" = ["admin"]), ("session" = [])),
    request_body = MePatch,
    responses(
        (status = 200, description = "The user", body = UserEnvelope),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `session_required` (email), `csrf_rejected`",
            body = ErrorBody),
        (status = 409, description = "`setup_required`, `conflict` (the email is taken)", body = ErrorBody),
        (status = 415, description = "`unsupported_media_type`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `name` or `email`)", body = ErrorBody),
    )
)]
async fn patch_me(
    auth: Auth<Admin>,
    State(state): State<AppState>,
    Json(patch): Json<MePatch>,
) -> Result<Json<UserEnvelope>, ApiError> {
    let actor = auth.into_actor();
    let now = state.now();
    let is_token = actor.token_id().is_some();
    let user = state
        .db
        .write(move |tx| {
            let current = quire_store::users::get(tx, &actor.user_id)?;
            if let Some(email) = &patch.email
                && !email.trim().eq_ignore_ascii_case(&current.email)
                && is_token
            {
                return Err(quire_store::Error::new(
                    ErrorCode::SessionRequired,
                    "changing the email needs a signed-in session, not a token",
                ));
            }
            let row = quire_store::users::update_profile(
                tx,
                &actor.user_id,
                &ProfilePatch {
                    name: patch.name,
                    email: patch.email,
                    theme: patch.theme,
                },
                now,
            )?;
            row.to_api()
        })
        .await?;
    Ok(Json(UserEnvelope { user }))
}

/// Changes the password (session only): `current` must be right (422 `field: "current"` otherwise, never 401), `new`
/// is 12–1024 bytes. Every other session is revoked; API tokens stay.
#[utoipa::path(
    post,
    path = "/auth/password",
    tag = "auth",
    operation_id = "change_password",
    security(("session" = [])),
    request_body = PasswordChange,
    responses(
        (status = 204, description = "Changed; the other sessions are signed out"),
        (status = 401, description = "`unauthenticated`", body = ErrorBody),
        (status = 403, description = "`session_required` (a token), `csrf_rejected`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
        (status = 415, description = "`unsupported_media_type`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `current` or `new`)", body = ErrorBody),
    )
)]
async fn change_password(
    auth: Auth<Session>,
    State(state): State<AppState>,
    Json(change): Json<PasswordChange>,
) -> Result<StatusCode, ApiError> {
    let actor = auth.into_actor();
    check_password_field(&change.new, "new")?;
    let user_id = actor.user_id.clone();
    let user = state
        .db
        .read(move |conn| quire_store::users::get(conn, &user_id))
        .await?;
    let ok = match user.password_hash {
        Some(hash) => state.verify_password(hash, change.current).await?,
        None => false,
    };
    if !ok {
        return Err(ApiError::validation(
            "current",
            "the current password is wrong",
        ));
    }
    let hash = state.hash_password(change.new).await?;
    let now = state.now();
    let keep = actor.session_id().map(str::to_owned);
    state
        .db
        .write(move |tx| {
            quire_store::users::set_password(tx, &actor.user_id, &hash, false, now)?;
            quire_store::sessions::delete_others(tx, &actor.user_id, keep.as_deref())?;
            Ok(())
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The caller's live sessions, most recently seen first; `current` marks this request's session.
#[utoipa::path(
    get,
    path = "/auth/sessions",
    tag = "auth",
    operation_id = "list_sessions",
    security(("bearer" = ["admin"]), ("session" = [])),
    responses(
        (status = 200, description = "The sessions", body = Vec<SessionInfo>),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn list_sessions(
    auth: Auth<Admin>,
    State(state): State<AppState>,
) -> Result<Json<Vec<SessionInfo>>, ApiError> {
    let actor = auth.into_actor();
    let now = state.now();
    let sessions = state
        .db
        .read(move |conn| {
            quire_store::sessions::list(conn, &actor.user_id, now)?
                .iter()
                .map(|s| s.to_api(actor.session_id() == Some(s.id.as_str())))
                .collect()
        })
        .await?;
    Ok(Json(sessions))
}

/// `?others=true`: signs out every other session (from a token: every session).
#[utoipa::path(
    delete,
    path = "/auth/sessions",
    tag = "auth",
    operation_id = "revoke_other_sessions",
    security(("bearer" = ["admin"]), ("session" = [])),
    params(SessionRevokeQuery),
    responses(
        (status = 204, description = "Signed out"),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `others`, which must be true)", body = ErrorBody),
    )
)]
async fn revoke_sessions(
    auth: Auth<Admin>,
    State(state): State<AppState>,
    Query(query): Query<SessionRevokeQuery>,
) -> Result<StatusCode, ApiError> {
    if !query.others {
        return Err(ApiError::validation(
            "others",
            "send ?others=true, or DELETE /auth/sessions/{id} for one session",
        ));
    }
    let actor = auth.into_actor();
    let keep = actor.session_id().map(str::to_owned);
    state
        .db
        .write(move |tx| quire_store::sessions::delete_others(tx, &actor.user_id, keep.as_deref()))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Signs out one session of the caller.
#[utoipa::path(
    delete,
    path = "/auth/sessions/{id}",
    tag = "auth",
    operation_id = "revoke_session",
    security(("bearer" = ["admin"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The session's id")),
    responses(
        (status = 204, description = "Signed out"),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn revoke_session(
    auth: Auth<Admin>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
) -> Result<StatusCode, ApiError> {
    let user_id = auth.into_actor().user_id;
    let id = id.to_string();
    let n = state
        .db
        .write(move |tx| quire_store::sessions::delete(tx, &user_id, &id))
        .await?;
    if n == 0 {
        return Err(ApiError::not_found("no such session"));
    }
    Ok(StatusCode::NO_CONTENT)
}
