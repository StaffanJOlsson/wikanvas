//! Routes /api/v1/setup: first-run setup with the setup code (owner: W2)
//!
//! §8, D19: while no owner exists the server has a setup code (`WIKANVAS_SETUP_TOKEN`, or generated at startup and
//! logged with the setup link). `POST /setup` shares the per-IP login limiter, checks the code, and runs one
//! IMMEDIATE transaction guarded by `NOT EXISTS` an owner: the owner, the workspace settings and the "Getting started"
//! seed; then it signs the owner in. Once an owner exists it answers 409 `already_setup`, also for the right code.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use wikanvas_api::{
    ErrorBody, ErrorCode, RateLimitReason, SetupRequest, SetupStatus, UserEnvelope,
};

use crate::admin::{NewOwner, create_owner};
use crate::error::ApiError;
use crate::extract::ClientInfo;
use crate::state::{AppState, check_password, setup_code_matches};

/// This module's routes, relative to `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(get_setup))
        .routes(routes!(post_setup))
}

/// Whether first-run setup is still needed.
#[utoipa::path(
    get,
    path = "/setup",
    tag = "setup",
    operation_id = "get_setup",
    security(()),
    responses((status = 200, description = "Setup status", body = SetupStatus))
)]
async fn get_setup(State(state): State<AppState>) -> Result<Json<SetupStatus>, ApiError> {
    Ok(Json(SetupStatus {
        needs_setup: !state.owner_exists().await?,
    }))
}

/// First-run setup: creates the owner and the workspace with the setup code from the server log, seeds "Getting
/// started", and signs the owner in.
#[utoipa::path(
    post,
    path = "/setup",
    tag = "setup",
    operation_id = "setup",
    security(()),
    request_body = SetupRequest,
    responses(
        (status = 201, description = "The owner, signed in", body = UserEnvelope,
            headers(("Set-Cookie" = String, description = "The session cookie"))),
        (status = 401, description = "`invalid_credentials`: the setup code is wrong", body = ErrorBody),
        (status = 403, description = "`csrf_rejected`", body = ErrorBody),
        (status = 409, description = "`already_setup`", body = ErrorBody),
        (status = 415, description = "`unsupported_media_type`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `email`, `name`, `workspace_name` or `password`)",
            body = ErrorBody),
        (status = 429, description = "`rate_limited` (`reason: requests`, per IP)", body = ErrorBody,
            headers(("Retry-After" = u64, description = "Seconds"))),
    )
)]
async fn post_setup(
    State(state): State<AppState>,
    client: ClientInfo,
    headers: HeaderMap,
    Json(req): Json<SetupRequest>,
) -> Result<Response, ApiError> {
    state
        .login_limiter
        .attempt(client.limiter_ip())
        .map_err(|retry| ApiError::rate_limited(RateLimitReason::Requests, retry))?;
    if state.owner_exists().await? {
        return Err(already_setup());
    }
    if !setup_code_matches(&state.setup_code, &req.code) {
        return Err(ApiError::new(
            ErrorCode::InvalidCredentials,
            "the setup code is wrong; it is in the server log (or WIKANVAS_SETUP_TOKEN)",
        ));
    }
    wikanvas_store::users::check_email(req.email.trim())?;
    wikanvas_store::users::check_name(&req.name)?;
    if !(1..=100).contains(&req.workspace_name.trim().chars().count()) {
        return Err(ApiError::validation(
            "workspace_name",
            "a workspace name is 1 to 100 characters",
        ));
    }
    check_password(&req.password)?;
    let password_hash = state.hash_password(req.password).await?;
    let user = create_owner(
        &state,
        NewOwner {
            email: req.email,
            name: req.name,
            workspace: req.workspace_name,
            password_hash,
            must_change_password: false,
            seed: true,
        },
        state.now(),
    )
    .await?;
    tracing::info!(email = %user.email, "first-run setup done");
    let cookie =
        crate::routes::auth::sign_in(&state, user.id.to_string(), false, client, &headers).await?;
    let mut res = (StatusCode::CREATED, Json(UserEnvelope { user })).into_response();
    res.headers_mut().append(header::SET_COOKIE, cookie);
    Ok(res)
}

fn already_setup() -> ApiError {
    ApiError::new(ErrorCode::AlreadySetup, "setup is done: sign in instead")
}
