//! Routes /api/v1/attachments (owner: W2)
//!
//! §7.2, §8 "Uploads", §11:
//! - `POST /attachments` (write): the raw body (clients send `application/octet-stream`; the form types a cross-site
//!   form can send are 415) up to `WIKANVAS_MAX_UPLOAD_MB` (413). After the free-space check (507) it streams to
//!   `/data/tmp` under the `upload_work` permit, hashed and sniffed as it arrives, never buffered whole. The type is
//!   sniffed, never taken from the client; image dimensions come from the header only, and more than 100 megapixels
//!   is 422. The blob and the attachment row are committed in one writer transaction.
//! - `GET /attachments/{id}` (read): the bytes, streamed from the blob file, `private, max-age=31536000, immutable`,
//!   `nosniff`, `CSP: default-src 'none'; sandbox`; inline only for PNG, JPEG, GIF, WebP and AVIF, every other type
//!   (SVG included) as a download. `Content-Disposition` follows RFC 6266: an ASCII `filename` and a UTF-8
//!   `filename*`.

use axum::Json;
use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use futures_util::StreamExt;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use wikanvas_api::{Attachment, ErrorBody, ErrorCode, UploadQuery};
use wikanvas_store::Sha256;

use crate::authz::{self, Action, Resource};
use crate::error::ApiError;
use crate::extract::{Auth, IdPath, Read, Write};
use crate::headers::ATTACHMENT_CSP;
use crate::state::AppState;

/// Images larger than this many pixels are refused (D33).
pub const MAX_PIXELS: u64 = 100_000_000;
/// The media types shown inline (§11).
pub const INLINE_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/avif",
];
/// The types a cross-site form can send without a preflight: never accepted (§7.1).
const FORM_TYPES: &[&str] = &[
    "application/x-www-form-urlencoded",
    "multipart/form-data",
    "text/plain",
];
/// Read buffer of a download.
const CHUNK: usize = 256 * 1024;

/// This module's routes, relative to `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(upload))
        .routes(routes!(download))
}

/// Whether `headers` carry one of the form types (415).
pub fn is_form_type(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        })
        .is_some_and(|t| FORM_TYPES.contains(&t.as_str()))
}

/// RFC 5987 `attr-char`: everything else is percent-encoded in `filename*`.
const ATTR_CHAR: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'!')
    .remove(b'#')
    .remove(b'$')
    .remove(b'&')
    .remove(b'+')
    .remove(b'-')
    .remove(b'.')
    .remove(b'^')
    .remove(b'_')
    .remove(b'`')
    .remove(b'|')
    .remove(b'~');

/// `Content-Disposition: <inline|attachment>; filename="<ASCII fallback>"; filename*=UTF-8''<percent-encoded>` (RFC
/// 6266). The fallback replaces anything outside printable ASCII, and `"` and `\`, with `_`.
pub fn content_disposition(inline: bool, filename: &str) -> String {
    let fallback: String = filename
        .chars()
        .map(|c| {
            if c.is_ascii() && !c.is_ascii_control() && c != '"' && c != '\\' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!(
        "{}; filename=\"{fallback}\"; filename*=UTF-8''{}",
        if inline { "inline" } else { "attachment" },
        utf8_percent_encode(filename, ATTR_CHAR)
    )
}

/// Uploads a file (the raw body, sent as `application/octet-stream`).
#[utoipa::path(
    post,
    path = "/attachments",
    tag = "attachments",
    operation_id = "upload_attachment",
    security(("bearer" = ["write"]), ("session" = [])),
    params(UploadQuery),
    request_body(content = Vec<u8>, content_type = "application/octet-stream",
        description = "The file's bytes; the type is sniffed"),
    responses(
        (status = 201, description = "The attachment", body = Attachment),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `forbidden` (a `viewer`), `csrf_rejected`",
            body = ErrorBody),
        (status = 404, description = "`not_found` (`item_id`)", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
        (status = 413, description = "`payload_too_large`", body = ErrorBody),
        (status = 415, description = "`unsupported_media_type`: a form type", body = ErrorBody),
        (status = 422, description = "`validation` (an image over 100 megapixels: `field: body`)", body = ErrorBody),
        (status = 429, description = "`rate_limited`", body = ErrorBody),
        (status = 507, description = "`insufficient_storage`", body = ErrorBody),
    )
)]
async fn upload(
    auth: Auth<Write>,
    State(state): State<AppState>,
    Query(query): Query<UploadQuery>,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<Attachment>), ApiError> {
    let actor = auth.into_actor();
    let item_id = query.item_id.map(|id| id.to_string());
    let resource = item_id
        .as_deref()
        .map_or(Resource::Workspace, Resource::Item);
    authz::require(&actor, Action::Write, resource)?;
    if is_form_type(&headers) {
        return Err(ApiError::new(
            ErrorCode::UnsupportedMediaType,
            "send the file as application/octet-stream",
        ));
    }
    let limit = state.config.max_upload_mb.saturating_mul(1024 * 1024);
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok());
    if declared.is_some_and(|n| n > limit) {
        return Err(ApiError::payload_too_large(limit));
    }
    state.check_free_space()?;
    let _permit = state
        .upload_work
        .clone()
        .acquire_owned()
        .await
        .map_err(ApiError::internal)?;
    let stream = body
        .into_data_stream()
        .map(|chunk| chunk.map_err(std::io::Error::other));
    let staged = state.blobs.stage_stream(stream, limit).await?;
    if let Some((w, h)) = staged.dimensions()
        && u64::from(w) * u64::from(h) > MAX_PIXELS
    {
        return Err(ApiError::validation(
            "body",
            format!("the image is {w}×{h}, more than 100 megapixels"),
        ));
    }
    let now = state.now();
    let blobs = state.blobs.clone();
    let filename = query.filename.unwrap_or_default();
    let row = state
        .db
        .write(move |tx| {
            let blob = blobs.commit(tx, staged, now)?;
            wikanvas_store::attachments::create(
                tx,
                &blob,
                item_id.as_deref(),
                &filename,
                &actor.user_id,
                now,
            )
        })
        .await?;
    Ok((StatusCode::CREATED, Json(row.to_api()?)))
}

/// The bytes of an attachment.
#[utoipa::path(
    get,
    path = "/attachments/{id}",
    tag = "attachments",
    operation_id = "download_attachment",
    security(("bearer" = ["read"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The attachment's id")),
    responses(
        (status = 200, description = "The bytes, with the sniffed type", content_type = "application/octet-stream",
            body = Vec<u8>, headers(("Content-Disposition" = String, description = "RFC 6266"))),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn download(
    auth: Auth<Read>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
) -> Result<Response, ApiError> {
    let id = id.to_string();
    authz::require(&auth, Action::Read, Resource::Attachment(&id))?;
    let row = state
        .db
        .read(move |conn| wikanvas_store::attachments::get(conn, &id))
        .await?
        .ok_or_else(|| ApiError::not_found("no such attachment"))?;
    let sha = Sha256::parse(&row.sha256).map_err(ApiError::internal)?;
    let file = tokio::fs::File::open(state.blobs.path(&sha))
        .await
        .map_err(|e| ApiError::internal(format!("attachment {}: {e}", row.id)))?;
    let body = Body::from_stream(tokio_util::io::ReaderStream::with_capacity(file, CHUNK));
    let mut res = Response::new(body);
    let h = res.headers_mut();
    let inline = INLINE_TYPES.contains(&row.mime.as_str());
    let set = |h: &mut HeaderMap, name: header::HeaderName, value: &str| {
        if let Ok(v) = HeaderValue::from_str(value) {
            h.insert(name, v);
        }
    };
    set(h, header::CONTENT_TYPE, &row.mime);
    set(h, header::CONTENT_LENGTH, &row.size.to_string());
    set(
        h,
        header::CACHE_CONTROL,
        "private, max-age=31536000, immutable",
    );
    set(h, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(h, header::CONTENT_SECURITY_POLICY, ATTACHMENT_CSP);
    set(
        h,
        header::CONTENT_DISPOSITION,
        &content_disposition(inline, &row.filename),
    );
    set(h, header::ETAG, &format!("\"{}\"", &row.sha256[..32]));
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispositions() {
        assert_eq!(
            content_disposition(true, "cat.png"),
            "inline; filename=\"cat.png\"; filename*=UTF-8''cat.png"
        );
        assert_eq!(
            content_disposition(false, "Résumé 1.pdf"),
            "attachment; filename=\"R_sum_ 1.pdf\"; filename*=UTF-8''R%C3%A9sum%C3%A9%201.pdf"
        );
        let d = content_disposition(false, "a\"b\\c\r\n");
        assert!(!d.contains('\r') && !d.contains('\n'));
        assert!(d.starts_with("attachment; filename=\"a_b_c__\";"), "{d}");
    }

    #[test]
    fn form_types() {
        let h = |v: &'static str| {
            let mut m = HeaderMap::new();
            m.insert(header::CONTENT_TYPE, HeaderValue::from_static(v));
            m
        };
        assert!(is_form_type(&h("text/plain; charset=utf-8")));
        assert!(is_form_type(&h("multipart/form-data; boundary=x")));
        assert!(is_form_type(&h("application/x-www-form-urlencoded")));
        assert!(!is_form_type(&h("application/octet-stream")));
        assert!(!is_form_type(&h("image/png")));
        assert!(!is_form_type(&HeaderMap::new()));
    }
}
