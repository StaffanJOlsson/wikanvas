//! Route /api/v1/config (owner: W2)
//!
//! §7.2, public: the server version and commit, `api_version`, `openapi_url`, `setup_required`, `multi_user: false`,
//! the embedded Inkboard build (`null` without a manifest or for the stub, §10.5) and every request limit.

use axum::Json;
use axum::extract::State;
use wikanvas_api::Config;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::ApiError;
use crate::state::AppState;

/// This module's routes, relative to `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(get_config))
}

/// What a client needs before signing in: versions, setup state, the Inkboard build and the limits.
#[utoipa::path(
    get,
    path = "/config",
    tag = "meta",
    operation_id = "get_config",
    security(()),
    responses((status = 200, description = "The configuration", body = Config))
)]
async fn get_config(State(state): State<AppState>) -> Result<Json<Config>, ApiError> {
    Ok(Json(Config {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        commit: crate::cli::commit().to_owned(),
        api_version: wikanvas_api::API_VERSION.to_owned(),
        openapi_url: "/api/v1/openapi.json".to_owned(),
        setup_required: !state.owner_exists().await?,
        multi_user: false,
        inkboard: state.site.inkboard_info(),
        limits: state.config.limits(),
    }))
}
