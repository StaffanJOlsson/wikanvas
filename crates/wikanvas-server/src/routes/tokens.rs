//! Routes /api/v1/tokens: personal access tokens (owner: W2)
//!
//! §7.2, §7.3, D42: list, create (with the caller's scope ceiling, no `admin` from a token, the calling token's
//! expiry as a cap, the 100-live-token limit, and the password for a session's `admin` or never-expiring token),
//! revoke (with the tokens it created) and revoke-current. The last-use writes and the per-token limiters run in
//! `Auth<S>` (`crate::extract`). Every route acts on the caller's own tokens only (the store checks `user_id`), so
//! `authz::require` is on [`Resource::OwnAccount`], which every role may manage (§8).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use wikanvas_api::tokens::{MAX_EXPIRY_DAYS, NewToken};
use wikanvas_api::{ApiToken, CreatedToken, ErrorBody, ErrorCode, Scope};
use wikanvas_store::tokens::{CreatedVia, TokenRow};

use crate::authz::{self, Action, Resource};
use crate::error::ApiError;
use crate::extract::{Actor, Admin, AnyToken, Auth, AuthMethod, IdPath};
use crate::state::AppState;

/// One day in milliseconds.
const DAY_MS: i64 = 86_400_000;

/// This module's routes, relative to `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_tokens))
        .routes(routes!(create_token))
        .routes(routes!(revoke_current))
        .routes(routes!(revoke_token))
}

/// The caller's live tokens and expired ones (not revoked), newest first, at most 100. Never the secret or its hash.
#[utoipa::path(
    get,
    path = "/tokens",
    tag = "tokens",
    operation_id = "list_tokens",
    security(("bearer" = ["admin"]), ("session" = [])),
    responses(
        (status = 200, description = "The tokens", body = Vec<ApiToken>),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn list_tokens(
    auth: Auth<Admin>,
    State(state): State<AppState>,
) -> Result<Json<Vec<ApiToken>>, ApiError> {
    authz::require(&state, &auth, Action::Read, Resource::OwnAccount).await?;
    let user_id = auth.into_actor().user_id;
    let now = state.now();
    let tokens = state
        .db
        .read(move |conn| wikanvas_store::tokens::list(conn, &user_id, now))
        .await?;
    Ok(Json(tokens))
}

/// Creates a token; the response is the only place its secret ever appears.
///
/// Scopes cannot exceed the caller's; a calling token cannot grant `admin` (403 `session_required`) and the new
/// token cannot outlive it (422, `field: "expires_in_days"`). A session sends `password` for an `admin` or
/// never-expiring token (422, `field: "password"`, when missing or wrong). At most 100 live tokens per user.
#[utoipa::path(
    post,
    path = "/tokens",
    tag = "tokens",
    operation_id = "create_token",
    security(("bearer" = ["admin"]), ("session" = [])),
    request_body = NewToken,
    responses(
        (status = 201, description = "The token and its secret", body = CreatedToken),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `session_required`, `csrf_rejected`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
        (status = 413, description = "`payload_too_large`", body = ErrorBody),
        (status = 415, description = "`unsupported_media_type`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `name`, `scopes`, `expires_in_days` or `password`)",
            body = ErrorBody),
    )
)]
async fn create_token(
    auth: Auth<Admin>,
    State(state): State<AppState>,
    Json(new): Json<NewToken>,
) -> Result<(StatusCode, Json<CreatedToken>), ApiError> {
    authz::require(&state, &auth, Action::Write, Resource::OwnAccount).await?;
    let actor = auth.into_actor();
    validate(&new)?;
    let now = state.now();
    let requested = Scope::highest(&new.scopes).expect("validated: not empty");
    let parent = match &actor.method {
        AuthMethod::Token { token_id, scopes } => {
            if requested == Scope::Admin {
                return Err(ApiError::new(
                    ErrorCode::SessionRequired,
                    "a token cannot create an admin token; use a signed-in session or wikanvas-server admin",
                ));
            }
            if !Scope::granted(scopes, requested) {
                return Err(ApiError::validation(
                    "scopes",
                    "a token cannot grant a scope it does not have",
                ));
            }
            let row = caller_row(&state, token_id.clone(), now).await?;
            let expires = new.expiry_days().map(|d| now + i64::from(d) * DAY_MS);
            if let Some(cap) = row.expires_at
                && expires.is_none_or(|e| e > cap)
            {
                return Err(ApiError::validation(
                    "expires_in_days",
                    "a token cannot create a token that outlives it",
                ));
            }
            Some(row)
        }
        AuthMethod::Session { .. } => {
            if requested == Scope::Admin || new.expiry_days().is_none() {
                check_password(&state, &actor, new.password.clone()).await?;
            }
            None
        }
    };
    let via = if parent.is_some() {
        CreatedVia::Api
    } else {
        CreatedVia::Web
    };
    let user_id = actor.user_id.clone();
    let created = state
        .db
        .write(move |tx| {
            wikanvas_store::tokens::create(tx, &user_id, &new, via, parent.as_ref(), now)
        })
        .await?;
    Ok((StatusCode::CREATED, Json(created)))
}

/// Request checks that need no database: name, scopes and the expiry range.
fn validate(new: &NewToken) -> Result<(), ApiError> {
    let name_len = new.name.chars().count();
    if !(1..=100).contains(&name_len) || new.name.trim().is_empty() {
        return Err(ApiError::validation(
            "name",
            "a token name is 1 to 100 characters",
        ));
    }
    if new.scopes.is_empty() {
        return Err(ApiError::validation("scopes", "give at least one scope"));
    }
    if let Some(Some(days)) = new.expires_in_days
        && !(1..=MAX_EXPIRY_DAYS).contains(&days)
    {
        return Err(ApiError::validation(
            "expires_in_days",
            format!("expires_in_days is 1 to {MAX_EXPIRY_DAYS}, or null for never"),
        ));
    }
    Ok(())
}

/// The calling token's row (for the expiry cap and `parent_token_id`).
async fn caller_row(state: &AppState, token_id: String, now: i64) -> Result<TokenRow, ApiError> {
    state
        .db
        .read(move |conn| wikanvas_store::tokens::live_row(conn, &token_id, now))
        .await?
        .ok_or_else(|| ApiError::invalid_token(false))
}

/// A session's re-entered password (D42): missing or wrong is 422 `validation` on `password`, never 401.
async fn check_password(
    state: &AppState,
    actor: &Actor,
    password: Option<String>,
) -> Result<(), ApiError> {
    let missing = || {
        ApiError::validation(
            "password",
            "enter your password to create an admin or never-expiring token",
        )
    };
    let password = password.ok_or_else(missing)?;
    let user_id = actor.user_id.clone();
    let hash = state
        .db
        .read(move |conn| Ok(wikanvas_store::users::get(conn, &user_id)?.password_hash))
        .await?;
    let Some(hash) = hash else {
        return Err(missing());
    };
    if state.verify_password(hash, password).await? {
        Ok(())
    } else {
        Err(ApiError::validation("password", "the password is wrong"))
    }
}

/// Revokes the calling token itself (`wikanvas auth logout --revoke`), and the tokens it created.
#[utoipa::path(
    delete,
    path = "/tokens/current",
    tag = "tokens",
    operation_id = "revoke_current_token",
    security(("bearer" = [])),
    responses(
        (status = 204, description = "Revoked"),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`forbidden` (a session, not a token)", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn revoke_current(
    auth: Auth<AnyToken>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    authz::require(&state, &auth, Action::Write, Resource::OwnAccount).await?;
    let actor = auth.into_actor();
    let token_id = actor.token_id().unwrap_or_default().to_owned();
    revoke(&state, actor.user_id, token_id).await
}

/// Revokes a token of the caller, and the tokens it created. Takes effect on the next request.
#[utoipa::path(
    delete,
    path = "/tokens/{id}",
    tag = "tokens",
    operation_id = "revoke_token",
    security(("bearer" = ["admin"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The token's id")),
    responses(
        (status = 204, description = "Revoked"),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected`", body = ErrorBody),
        (status = 404, description = "`not_found`: no live token of the caller has this id", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn revoke_token(
    auth: Auth<Admin>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
) -> Result<StatusCode, ApiError> {
    authz::require(&state, &auth, Action::Write, Resource::OwnAccount).await?;
    revoke(&state, auth.into_actor().user_id, id.to_string()).await
}

async fn revoke(
    state: &AppState,
    user_id: String,
    token_id: String,
) -> Result<StatusCode, ApiError> {
    let now = state.now();
    let revoked = state
        .db
        .write(move |tx| wikanvas_store::tokens::revoke(tx, &user_id, &token_id, now))
        .await?;
    if revoked == 0 {
        return Err(ApiError::not_found("no such token"));
    }
    Ok(StatusCode::NO_CONTENT)
}
