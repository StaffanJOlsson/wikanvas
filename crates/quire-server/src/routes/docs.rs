//! Routes /api/v1/items/{id}/doc*, and /api/v1/schema/doc (owner: W4)
//!
//! W0b registers no route here yet; the owner adds its handlers with `.routes(routes!(…))` on `#[utoipa::path]`
//! functions whose `security` matches their `Auth<S>` (PLAN R4, ARCHITECTURE §7.4).

use utoipa_axum::router::OpenApiRouter;

use crate::state::AppState;

/// This module's routes, relative to `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
}
