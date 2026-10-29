//! Routes /api/v1/admin: on-demand backup (owner: W2)
//!
//! `POST /admin/backup` (admin): a backup now, single-flight with the scheduled one (429 `busy`), after the
//! free-space check (507), counted against `QUIRE_BACKUP_KEEP` (§12).

use axum::Json;
use axum::extract::State;
use quire_api::{BackupResult, ErrorBody};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::authz::{self, Action, Resource};
use crate::error::ApiError;
use crate::extract::{Admin, Auth};
use crate::state::AppState;

/// This module's routes, relative to `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(backup))
}

/// Writes a backup of the database on the server (`<data dir>/backups/quire-<time>.db`).
#[utoipa::path(
    post,
    path = "/admin/backup",
    tag = "admin",
    operation_id = "backup",
    security(("bearer" = ["admin"]), ("session" = [])),
    responses(
        (status = 200, description = "The backup file", body = BackupResult),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
        (status = 429, description = "`rate_limited` (`reason: busy`)", body = ErrorBody),
        (status = 507, description = "`insufficient_storage`", body = ErrorBody),
    )
)]
async fn backup(
    auth: Auth<Admin>,
    State(state): State<AppState>,
) -> Result<Json<BackupResult>, ApiError> {
    authz::require(&auth, Action::Admin, Resource::Workspace)?;
    let (path, size) = crate::jobs::backup_now(&state).await?;
    Ok(Json(BackupResult {
        path: path.display().to_string(),
        size,
    }))
}
