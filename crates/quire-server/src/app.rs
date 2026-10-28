//! Router assembly: the API routes, /healthz, static files and the middleware stack (owner: W2)
//!
//! [`app`] is the whole HTTP surface: `/api/v1` from [`crate::openapi::router`] (the same registrations as the spec,
//! with the CORS layer of D48 when `QUIRE_CORS_ORIGINS` is set), `GET /healthz` outside the spec and the Host
//! allowlist, the JSON 404 for any other path under `/api/`, and the static files for everything else. Layers,
//! outermost first:
//! 1. sensitive headers (`Authorization`, `Cookie` and `Set-Cookie` never reach a log);
//! 2. the access log (one span per request with method, path, client IP and, for a token, `token_id`);
//! 3. gzip at the fast level for JSON and Markdown bodies over 1 KiB (§11; static files bring their own variants);
//! 4. the error envelope with `X-Request-Id` / `Quire-Api-Version`;
//! 5. [`crate::headers::layer`]: client address, security headers, HSTS, the renewed session cookie;
//! 6. the panic catcher;
//! 7. the timeout: 30 s, 120 s for uploads and board PUTs (408 `request_timeout`);
//! 8. the 1 MiB default body limit;
//! 9. the Host allowlist with the CSRF check.

use std::future::Future;
use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use quire_api::{ErrorCode, Health};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::compression::predicate::{Predicate as _, SizeAbove};
use tower_http::compression::{CompressionLayer, CompressionLevel};
use tower_http::sensitive_headers::SetSensitiveHeadersLayer;
use tower_http::trace::TraceLayer;
use tracing::Span;

use crate::config::{Config, MAX_BODY_BYTES};
use crate::error::{self, ApiError};
use crate::state::AppState;
use crate::{cors, csrf, headers, openapi};

/// The application: every route and layer, with `state`.
pub fn app(state: AppState) -> Router {
    let (api, _spec) = openapi::router().split_for_parts();
    let api = match cors::layer(&state.config) {
        Some(cors) => api.layer(cors),
        None => api,
    };
    Router::new()
        .route("/healthz", get(healthz))
        .merge(api)
        .fallback(fallback)
        .layer(middleware::from_fn_with_state(state.clone(), csrf::layer))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES as usize))
        .layer(middleware::from_fn_with_state(state.clone(), timeout))
        .layer(CatchPanicLayer::new())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            headers::layer,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            error::envelope,
        ))
        .layer(
            CompressionLayer::new()
                .gzip(true)
                .no_br()
                .no_deflate()
                .no_zstd()
                .quality(CompressionLevel::Fastest)
                .compress_when(SizeAbove::new(1024).and(compressible)),
        )
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|req: &Request<Body>| {
                    tracing::info_span!(
                        "request",
                        method = %req.method(),
                        path = %req.uri().path(),
                        client_ip = tracing::field::Empty,
                        token_id = tracing::field::Empty,
                    )
                })
                .on_request(|req: &Request<Body>, _span: &Span| {
                    tracing::debug!(headers = ?req.headers(), "request");
                })
                .on_response(|res: &Response, latency: Duration, _span: &Span| {
                    tracing::info!(
                        status = res.status().as_u16(),
                        latency_ms = u64::try_from(latency.as_millis()).unwrap_or(u64::MAX),
                        "response"
                    );
                }),
        )
        .layer(SetSensitiveHeadersLayer::new([
            header::AUTHORIZATION,
            header::COOKIE,
            header::SET_COOKIE,
        ]))
        .with_state(state)
}

/// gzip only JSON and Markdown bodies that are not encoded yet (§11; [`SizeAbove`] adds "over 1 KiB").
fn compressible(
    _status: StatusCode,
    _version: axum::http::Version,
    headers: &HeaderMap,
    _extensions: &axum::http::Extensions,
) -> bool {
    if headers.contains_key(header::CONTENT_ENCODING) {
        return false;
    }
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json") || v.starts_with("text/markdown"))
}

/// Whether the request is an upload or a board PUT, which get the longer body deadline (§8).
fn long_body(method: &Method, path: &str) -> bool {
    (method == Method::POST && path == "/api/v1/attachments")
        || (method == Method::PUT && path.starts_with("/api/v1/items/") && path.ends_with("/board"))
}

/// The request deadline (§8): 408 `request_timeout` when the response is not ready in time.
async fn timeout(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let limit = if long_body(req.method(), req.uri().path()) {
        state.config.upload_timeout
    } else {
        state.config.request_timeout
    };
    match tokio::time::timeout(limit, next.run(req)).await {
        Ok(res) => res,
        Err(_) => ApiError::new(
            ErrorCode::RequestTimeout,
            format!(
                "the request took longer than {} s; it may have been processed",
                limit.as_secs_f32()
            ),
        )
        .into_response(),
    }
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
async fn fallback(
    State(state): State<AppState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let path = uri.path();
    if path == "/api" || path.starts_with("/api/") {
        return error::api_not_found().into_response();
    }
    state.site.serve(&method, &uri, &headers)
}

/// `quire-server serve`: opens the state, logs the setup link while no owner exists, starts the background jobs,
/// listens on `QUIRE_LISTEN`, serves until `shutdown`, drains, and checkpoints the WAL (§5).
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
        tracing::info!("{}", state.setup_message());
    }
    let jobs = crate::jobs::spawn(state.clone());
    serve(state.clone(), listener, shutdown)
        .await
        .map_err(|e| format!("server: {e}"))?;
    jobs.stop();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_body_routes() {
        assert!(long_body(&Method::POST, "/api/v1/attachments"));
        assert!(long_body(&Method::PUT, "/api/v1/items/x/board"));
        assert!(!long_body(&Method::GET, "/api/v1/items/x/board"));
        assert!(!long_body(&Method::POST, "/api/v1/tokens"));
    }
}
