//! Router assembly: the API routes, /healthz, static files and the middleware stack (owner: W2)
//!
//! [`app`] is the whole HTTP surface: `/api/v1` from [`crate::openapi::router`] (the same registrations as the spec),
//! `GET /healthz` outside the spec and the Host allowlist, the JSON 404 for any other path under `/api/`, and the
//! static files for everything else. Layers, outermost first: sensitive headers, trace, the error envelope with
//! `X-Request-Id` / `Quire-Api-Version`, panic catcher, the 30 s timeout (408), the 1 MiB default body limit, and the
//! Host allowlist with the CSRF check.

use std::future::Future;
use std::net::SocketAddr;

use axum::Router;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{StatusCode, Uri, header};
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use quire_api::{ErrorCode, Health};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::sensitive_headers::SetSensitiveHeadersLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::config::{Config, MAX_BODY_BYTES, REQUEST_TIMEOUT};
use crate::error::{self, ApiError};
use crate::state::AppState;
use crate::{csrf, openapi, static_files};

/// The application: every route and layer, with `state`.
pub fn app(state: AppState) -> Router {
    let (api, _spec) = openapi::router().split_for_parts();
    Router::new()
        .route("/healthz", get(healthz))
        .merge(api)
        .fallback(fallback)
        .layer(middleware::from_fn_with_state(state.clone(), csrf::layer))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES as usize))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .layer(CatchPanicLayer::new())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            error::envelope,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(SetSensitiveHeadersLayer::new([
            header::AUTHORIZATION,
            header::COOKIE,
            header::SET_COOKIE,
        ]))
        .with_state(state)
}

/// `GET /healthz`: `{"status":"ok"}` after `SELECT 1`; 503 `unavailable` when the database does not answer.
async fn healthz(State(state): State<AppState>) -> Result<axum::Json<Health>, ApiError> {
    state
        .db
        .read(|conn| Ok(conn.query_row("SELECT 1", [], |r| r.get::<_, i64>(0))?))
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "healthz: the database does not answer");
            ApiError::new(ErrorCode::Unavailable, "the database does not answer")
        })?;
    Ok(axum::Json(Health {
        status: "ok".into(),
    }))
}

/// Unknown paths: the JSON 404 under `/api/`, static files elsewhere.
async fn fallback(uri: Uri) -> Response {
    let path = uri.path();
    if path == "/api" || path.starts_with("/api/") {
        return error::api_not_found().into_response();
    }
    static_files::serve(uri).await
}

/// `quire-server serve`: opens the state, listens on `QUIRE_LISTEN`, serves until `shutdown`, drains, and
/// checkpoints the WAL (§5).
pub async fn run(
    config: Config,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), String> {
    let state = AppState::open(config)
        .await
        .map_err(|e| format!("cannot start: {e}"))?;
    let listener = tokio::net::TcpListener::bind(state.config.listen)
        .await
        .map_err(|e| format!("cannot listen on {}: {e}", state.config.listen))?;
    tracing::info!(
        listen = %state.config.listen,
        public_url = %state.config.public_url,
        version = crate::cli::version(),
        "quire-server is serving"
    );
    if !state.owner_exists().await.unwrap_or(true) {
        tracing::info!(
            "no owner yet: run `quire-server admin create-owner` (first-run setup in the browser arrives with W2)"
        );
    }
    serve(state.clone(), listener, shutdown)
        .await
        .map_err(|e| format!("server: {e}"))?;
    state
        .db
        .with_writer(|conn| Ok(conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?))
        .await
        .map_err(|e| format!("WAL checkpoint: {e}"))?;
    tracing::info!("stopped");
    Ok(())
}

/// Serves `app(state)` on `listener` until `shutdown` resolves, then drains open requests. The peer address reaches
/// the handlers as `ConnectInfo<SocketAddr>` (the client-address rule of §8).
pub async fn serve(
    state: AppState,
    listener: tokio::net::TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let service = app(state).into_make_service_with_connect_info::<SocketAddr>();
    axum::serve(listener, service)
        .with_graceful_shutdown(shutdown)
        .await
}
