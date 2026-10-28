//! Routes under /api/v1/auth: login, logout, me, password, sessions (owner: W2)
//!
//! W0b: `GET /auth/me`. The user and workspace reads are plain SQL here until `quire_store::{users, settings}` have
//! them (W2).

use axum::Json;
use axum::extract::State;
use quire_api::{ApiToken, AuthMethodKind, ErrorBody, Me, MeAuth, Role, Theme, User, Workspace};
use rusqlite::{Connection, OptionalExtension};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extract::{Auth, AuthMethod, Read};
use crate::state::AppState;

/// This module's routes, relative to `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(get_me))
}

/// A user row: the API shape plus `must_change_password`.
pub fn load_user(conn: &Connection, user_id: &str) -> quire_store::Result<(User, bool)> {
    let row = conn.query_row(
        "SELECT id, email, name, role, theme, must_change_password FROM users WHERE id = ?1",
        [user_id],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, bool>(5)?,
            ))
        },
    )?;
    let (id, email, name, role, theme, must_change) = row;
    let bad = |what: &str| {
        quire_store::Error::new(quire_api::ErrorCode::Internal, format!("bad users.{what}"))
    };
    Ok((
        User {
            id: Uuid::parse_str(&id).map_err(|_| bad("id"))?,
            email,
            name,
            role: role.parse::<Role>().map_err(|_| bad("role"))?,
            theme: theme.parse::<Theme>().map_err(|_| bad("theme"))?,
        },
        must_change,
    ))
}

/// A setting's JSON value, `None` when unset.
pub fn setting(conn: &Connection, key: &str) -> quire_store::Result<Option<serde_json::Value>> {
    let raw: Option<String> = conn
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(raw.and_then(|v| serde_json::from_str(&v).ok()))
}

/// The workspace's name and logo URL from the settings.
pub fn load_workspace(conn: &Connection) -> quire_store::Result<Workspace> {
    let name = setting(conn, "workspace.name")?
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    let logo_url = setting(conn, "workspace.logo_attachment_id")?
        .and_then(|v| v.as_str().map(|id| format!("/api/v1/attachments/{id}")));
    Ok(Workspace { name, logo_url })
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
                AuthMethod::Token { token_id, .. } => {
                    let token: Option<ApiToken> =
                        quire_store::tokens::get(conn, &actor.user_id, token_id)?;
                    (AuthMethodKind::Token, token)
                }
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
