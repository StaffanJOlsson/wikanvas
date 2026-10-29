//! Route /api/v1/export (owner: W2)
//!
//! D34, §12: `GET /export` (admin) streams a tar of `manifest.json`, a credential-free copy of the database and every
//! blob file. Single-flight (429 `busy`) until the stream ends or the client goes away, and while a copy runs even
//! when its request is gone; after the free-space check (507). The 30 s request deadline does not apply, because the
//! copy is made before the response starts (`app::deadline`). With the session cookie the request must come from
//! Wikanvas itself: `Sec-Fetch-Site` `same-origin` or `none`, or, without fetch metadata, a same-origin `Referer` (403
//! `csrf_rejected`), because SameSite=Lax sends the cookie on cross-site navigations. The tar is produced on the
//! blocking pool into 256 KiB chunks through a 4-chunk channel (D25), so at most about 1 MiB is in flight.

use std::io::Write;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, Uri, header};
use axum::response::Response;
use bytes::Bytes;
use wikanvas_api::{ErrorBody, ErrorCode};
use wikanvas_store::export::{CHUNK, Manifest};
use tokio::sync::mpsc;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::authz::{self, Action, Resource};
use crate::csrf::{navigation_allowed, request_host};
use crate::error::ApiError;
use crate::extract::{Admin, Auth};
use crate::state::{AppState, Flight};

/// Chunks in flight between the tar writer and the response (D25).
const CHANNEL_CHUNKS: usize = 4;

/// This module's routes, relative to `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(export))
}

/// A `Write` that sends 256 KiB chunks into the response channel; a closed channel (the client left) is an error.
struct ChannelWriter {
    tx: mpsc::Sender<std::io::Result<Bytes>>,
    buf: Vec<u8>,
}

impl ChannelWriter {
    fn send(&mut self) -> std::io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let chunk = std::mem::replace(&mut self.buf, Vec::with_capacity(CHUNK));
        self.tx.blocking_send(Ok(Bytes::from(chunk))).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "the client went away")
        })
    }
}

impl Write for ChannelWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(data);
        if self.buf.len() >= CHUNK {
            self.send()?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Downloads the whole workspace: a tar of `manifest.json`, `wikanvas.db` (without sessions, API tokens or password
/// hashes) and `files/**`.
#[utoipa::path(
    get,
    path = "/export",
    tag = "admin",
    operation_id = "export",
    security(("bearer" = ["admin"]), ("session" = [])),
    responses(
        (status = 200, description = "The tar stream", content_type = "application/x-tar", body = Vec<u8>,
            headers(("Content-Disposition" = String, description = "attachment; filename=\"wikanvas-export-<time>.tar\""))),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected` (a cookie request from elsewhere)",
            body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
        (status = 429, description = "`rate_limited` (`reason: busy`)", body = ErrorBody),
        (status = 507, description = "`insufficient_storage`", body = ErrorBody),
    )
)]
async fn export(
    auth: Auth<Admin>,
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, ApiError> {
    authz::require(&auth, Action::Admin, Resource::Workspace)?;
    if auth.session_id().is_some() {
        let host = request_host(&headers, &uri).unwrap_or_default();
        if !navigation_allowed(&state.config, &headers, &host) {
            return Err(ApiError::new(
                ErrorCode::CsrfRejected,
                "start the export from Wikanvas itself (Settings), or use a token",
            ));
        }
    }
    let guard = state
        .single_flight
        .try_start(Flight::Export)
        .ok_or_else(ApiError::busy)?;
    // The free-space check and the copy run on the blocking pool, which also owns the guard. When this request goes
    // away first (the client left), the pool still finishes the copy and then drops it (deleting the file) and the
    // guard, so no second export starts while a copy is being written.
    let worker = state.clone();
    let (copy, guard) = tokio::task::spawn_blocking(move || {
        worker.check_free_space()?;
        let copy = wikanvas_store::export::prepare_copy(&worker.db, &worker.config.tmp_dir())?;
        Ok::<_, ApiError>((copy, guard))
    })
    .await
    .map_err(ApiError::internal)??;
    let now = state.now();
    let manifest = Manifest::new(crate::cli::version(), copy.schema_version(), now);
    let files_dir = state.config.files_dir();
    let (tx, rx) = mpsc::channel::<std::io::Result<Bytes>>(CHANNEL_CHUNKS);
    tokio::task::spawn_blocking(move || {
        let writer = ChannelWriter {
            tx: tx.clone(),
            buf: Vec::with_capacity(CHUNK),
        };
        let result = wikanvas_store::export::write_tar(writer, &copy, &files_dir, &manifest)
            .and_then(|mut w| Ok(w.send()?));
        if let Err(e) = result {
            tracing::warn!(error = %e, "export stopped");
            let _ = tx.blocking_send(Err(std::io::Error::other(e.to_string())));
        }
        drop(copy);
    });
    // The single-flight guard lives in the body: it is released when the stream ends or the client goes away.
    let stream = futures_util::stream::unfold((rx, guard), |(mut rx, guard)| async move {
        rx.recv().await.map(|item| (item, (rx, guard)))
    });
    // Fused, as the board route's stream: `unfold` panics when a consumer polls it again after its end.
    let mut res = Response::new(Body::from_stream(futures_util::StreamExt::fuse(stream)));
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-tar"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    let name = format!(
        "attachment; filename=\"wikanvas-export-{}.tar\"",
        wikanvas_store::backup::file_timestamp(now)
    );
    if let Ok(v) = HeaderValue::from_str(&name) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(res)
}
