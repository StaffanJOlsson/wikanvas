//! Routes /api/v1/items/{id}/doc*, and /api/v1/schema/doc (owner: W4)
//!
//! §6.1, §6.4, §7.2, §7.5. Every document route takes `doc_work` permits (D25): one for a read or a Yjs POST, two at
//! once (`acquire_many(2)`) for a whole-document PUT. A writing route takes them after its header checks (415, 428)
//! and before it reads the body, so a request waiting for a permit holds no body, and holds them until the commit;
//! bodies are parsed in `spawn_blocking`. The bodies are read here with the route's own limit (Yjs update 16 MiB,
//! ProseMirror JSON 4 MiB, Markdown 1 MiB), so a 413 names that limit. A slow sender holds its permits for at most the
//! 30 s request timeout.

use std::borrow::Cow;

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{Query, State};
use axum::http::header::{self, HeaderMap, HeaderValue};
use axum::http::{HeaderName, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};
use tokio::sync::SemaphorePermit;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use wikanvas_api::{
    DocJson, DocJsonPut, DocQuery, DocSchema, DocState, DocUpdate, DocUpdateResult, DocWriteResult,
    ErrorBody, ErrorCode, InvalidDocumentDetails, LimitDetails, MarkdownGetQuery, MarkdownPutQuery,
    MarkdownPutTitle, MarkdownTitle, ProseMirrorPutQuery, SCHEMA_VERSION, Timestamp,
};
use wikanvas_store::docs::{self, Author, ContentInput, ContentWrite, IfMatch, PmJson};
use wikanvas_store::shared::WriteBy;

use crate::authz::{self, Action, Resource};
use crate::config::{MAX_JSON_BYTES, MAX_MARKDOWN_BYTES, MAX_UPDATE_BYTES};
use crate::error::ApiError;
use crate::extract::{Actor, Auth, AuthMethod, IdPath, Read, Write};
use crate::state::{AppState, now_ms};

/// The `Wikanvas-Conversion-Warnings` response header of the Markdown GET (§6.4).
pub const WARNINGS_HEADER: &str = "wikanvas-conversion-warnings";

/// This module's routes, relative to `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(get_doc))
        .routes(routes!(post_updates))
        .routes(routes!(get_markdown))
        .routes(routes!(put_markdown))
        .routes(routes!(get_prosemirror))
        .routes(routes!(put_prosemirror))
        .routes(routes!(get_schema))
}

fn b64d(field: &str, s: &str) -> Result<Vec<u8>, ApiError> {
    base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .map_err(|_| ApiError::validation(field, format!("{field} is not base64")))
}

fn too_large(limit: u64) -> ApiError {
    ApiError::new(
        ErrorCode::PayloadTooLarge,
        format!("the body is larger than {limit} bytes"),
    )
    .with_details(&LimitDetails { limit_bytes: limit })
}

/// Reads the body with the route's limit (413 `payload_too_large` with that limit).
async fn body(body: Body, limit: u64) -> Result<Bytes, ApiError> {
    axum::body::to_bytes(body, limit as usize)
        .await
        .map_err(|_| too_large(limit))
}

/// The media type of the request (lowercase, without parameters).
fn media_type(headers: &HeaderMap) -> String {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .map(|v| v.trim().to_ascii_lowercase())
        .unwrap_or_default()
}

fn require_media(headers: &HeaderMap, wanted: &str) -> Result<(), ApiError> {
    if media_type(headers) == wanted {
        return Ok(());
    }
    Err(ApiError::new(
        ErrorCode::UnsupportedMediaType,
        format!("send the body as {wanted}"),
    ))
}

/// Runs CPU-bound work (parsing a body) on the blocking pool.
async fn blocking<T, F>(f: F) -> Result<T, ApiError>
where
    F: FnOnce() -> Result<T, ApiError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(ApiError::internal)?
}

/// The body of `POST …/doc/updates` (the wire shape of [`DocUpdate`]), read without building a JSON tree:
/// `update` is borrowed from the body when it has no escapes, and `pm_json` stays the text it was sent as.
#[derive(Deserialize)]
struct UpdateBody<'a> {
    #[serde(borrow)]
    update: Cow<'a, str>,
    #[serde(borrow, default)]
    pm_json: Option<&'a RawValue>,
}

/// The update bytes and the `pm_json` text of a Yjs POST body: 422 `validation` when `update` is not base64 or
/// `pm_json` is not a JSON object (null is the same as absent).
fn parse_update(bytes: &[u8]) -> Result<(Vec<u8>, Option<String>), ApiError> {
    let body: UpdateBody<'_> = json_body(bytes)?;
    let update = b64d("update", &body.update)?;
    let pm_json = match body.pm_json {
        None => None,
        Some(raw) if raw.get().starts_with('{') => Some(raw.get().to_owned()),
        Some(_) => {
            return Err(ApiError::validation(
                "pm_json",
                "pm_json is the editor's JSON document (an object)",
            ));
        }
    };
    Ok((update, pm_json))
}

/// A JSON body: 422 `validation` (`field: "body"`) when it does not parse, `invalid_document` when it nests too deep.
fn json_body<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T, ApiError> {
    serde_json::from_slice(bytes).map_err(|e| {
        if e.to_string().contains("recursion limit") {
            ApiError::new(
                ErrorCode::InvalidDocument,
                "the document is nested too deep",
            )
            .with_details(&InvalidDocumentDetails {
                path: "/doc".into(),
                reason: "the JSON is nested deeper than the server reads".into(),
            })
        } else {
            ApiError::validation("body", format!("the body is not valid JSON: {e}"))
        }
    })
}

fn if_match(headers: &HeaderMap) -> Result<IfMatch, ApiError> {
    let Some(value) = headers.get(header::IF_MATCH) else {
        return Err(ApiError::new(
            ErrorCode::PreconditionRequired,
            "send If-Match with the document's ETag (\"d<seq>\"), or * to overwrite whatever is current",
        ));
    };
    let value = value.to_str().unwrap_or("").trim();
    if value == "*" {
        return Ok(IfMatch::Any);
    }
    let first = value.split(',').next().unwrap_or("");
    // An If-Match that is no document ETag is stale by definition.
    Ok(IfMatch::Seq(docs::parse_etag(first).unwrap_or(-1)))
}

fn if_none_match(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',').any(|t| {
                let t = t.trim();
                t == "*" || t.strip_prefix("W/").unwrap_or(t) == etag
            })
        })
}

fn etag_header(etag: &str) -> (HeaderName, HeaderValue) {
    (
        header::ETAG,
        HeaderValue::from_str(etag).unwrap_or_else(|_| HeaderValue::from_static("\"d0\"")),
    )
}

fn credential(actor: &Actor) -> String {
    match &actor.method {
        AuthMethod::Token { token_id, .. } => format!("token:{token_id}"),
        AuthMethod::Session { session_id } => format!("session:{session_id}"),
    }
}

fn author(actor: &Actor) -> Author {
    Author {
        user_id: actor.user_id.clone(),
        token_id: actor.token_id().map(str::to_owned),
    }
}

/// The stored-document cap (`WIKANVAS_MAX_DOC_MB`).
fn max_doc_bytes(state: &AppState) -> usize {
    (state.config.max_doc_mb as usize).saturating_mul(1024 * 1024)
}

/// The download file name: the title's ASCII slug (as the web client's `slugify`), `untitled` when empty.
fn slug(title: &str) -> String {
    let mut s = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            s.push(c);
        } else if !s.ends_with('-') {
            s.push('-');
        }
    }
    let s: String = s.trim_matches('-').chars().take(60).collect();
    let s = s.trim_end_matches('-').to_owned();
    if s.is_empty() { "untitled".into() } else { s }
}

/// The document's Yjs state: the whole update, or with `since` only what that state vector lacks.
#[utoipa::path(
    get,
    path = "/items/{id}/doc",
    tag = "documents",
    operation_id = "get_doc",
    security(("bearer" = ["read"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The document's id"), DocQuery),
    responses(
        (status = 200, description = "The Yjs state (base64 v1 update and state vector)", body = DocState),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
        (status = 422, description = "`validation` (`field`: `since` or `kind`)", body = ErrorBody),
    )
)]
async fn get_doc(
    auth: Auth<Read>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
    Query(q): Query<DocQuery>,
) -> Result<Json<DocState>, ApiError> {
    let actor = auth.into_actor();
    let id = id.to_string();
    authz::require(&actor, Action::Read, Resource::Item(&id))?;
    let since = q.since.as_deref().map(|s| b64d("since", s)).transpose()?;
    let _permit = state.doc_work.acquire().await.map_err(ApiError::internal)?;
    let doc = state
        .db
        .read(move |conn| docs::get(conn, &id, since.as_deref()))
        .await?;
    Ok(Json(doc))
}

/// Saves one Yjs update (§6.1): 409 `missing_deps` with the server's state vector when it depends on structs the
/// server lacks; a re-sent update is a no-op. A token's first write after someone else's stores a `pre-api`
/// revision (D53).
#[utoipa::path(
    post,
    path = "/items/{id}/doc/updates",
    tag = "documents",
    operation_id = "post_doc_updates",
    security(("bearer" = ["write"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The document's id")),
    request_body(content = DocUpdate, description = "A base64 Yjs v1 update (up to 16 MiB), and the editor's JSON"),
    responses(
        (status = 200, description = "Saved (or nothing new)", body = DocUpdateResult),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`missing_deps` (`details.state_vector`), `read_only`, `setup_required`",
            body = ErrorBody),
        (status = 413, description = "`payload_too_large`, `doc_too_large`", body = ErrorBody),
        (status = 415, description = "`unsupported_media_type`", body = ErrorBody),
        (status = 422, description = "`invalid_update`, `validation`", body = ErrorBody),
    )
)]
async fn post_updates(
    auth: Auth<Write>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
    headers: HeaderMap,
    raw: Body,
) -> Result<Json<DocUpdateResult>, ApiError> {
    let actor = auth.into_actor();
    let id = id.to_string();
    authz::require(&actor, Action::Write, Resource::Item(&id))?;
    require_media(&headers, "application/json")?;
    let author = author(&actor);
    let cap = max_doc_bytes(&state);
    let _permit = state.doc_work.acquire().await.map_err(ApiError::internal)?;
    let bytes = body(raw, MAX_UPDATE_BYTES).await?;
    let (update, pm_json) = blocking(move || parse_update(&bytes)).await?;
    let now = now_ms();
    let saved = state
        .db
        .write(move |tx| {
            let pm = match &pm_json {
                Some(text) => PmJson::Raw(text),
                None => PmJson::Keep,
            };
            docs::save_update(tx, &id, &update, pm, &author.write_by(), cap, now)
        })
        .await?;
    Ok(Json(DocUpdateResult {
        seq: saved.seq,
        state_vector: base64::engine::general_purpose::STANDARD.encode(&saved.state_vector),
        updated_at: Timestamp(saved.updated_at),
    }))
}

/// The document as Wikanvas Markdown 1 (§6.4). Never 304: it prints live titles. The lossy steps are listed in
/// `Wikanvas-Conversion-Warnings`.
#[utoipa::path(
    get,
    path = "/items/{id}/doc/markdown",
    tag = "documents",
    operation_id = "get_doc_markdown",
    security(("bearer" = ["read"]), ("session" = [])),
    params(("id" = uuid::Uuid, Path, description = "The document's id"), MarkdownGetQuery),
    responses(
        (status = 200, description = "Markdown, with `ETag: \"d{seq}\"` and `Wikanvas-Conversion-Warnings`",
            body = String, content_type = "text/markdown"),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn get_markdown(
    auth: Auth<Read>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
    Query(q): Query<MarkdownGetQuery>,
) -> Result<Response, ApiError> {
    let actor = auth.into_actor();
    let id = id.to_string();
    authz::require(&actor, Action::Read, Resource::Item(&id))?;
    let include = q.title == Some(MarkdownTitle::Include);
    let origin = Some(state.config.public_url.to_string());
    let _permit = state.doc_work.acquire().await.map_err(ApiError::internal)?;
    let printed = state
        .db
        .read(move |conn| docs::markdown(conn, &id, include, origin))
        .await?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/markdown; charset=utf-8"),
    );
    let (k, v) = etag_header(&docs::etag(printed.seq));
    headers.insert(k, v);
    // §11: never cached (private text, and the route never answers 304), and sandboxed like an attachment, so the
    // user's text is never a page of the app's origin even where a browser ignores the media type.
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(crate::headers::ATTACHMENT_CSP),
    );
    if !printed.warnings.is_empty() {
        let codes: Vec<&str> = printed.warnings.iter().map(|w| w.code.as_str()).collect();
        if let Ok(v) = HeaderValue::from_str(&codes.join(", ")) {
            headers.insert(HeaderName::from_static(WARNINGS_HEADER), v);
        }
    }
    if q.download {
        let v = format!("attachment; filename=\"{}.md\"", slug(&printed.title));
        if let Ok(v) = HeaderValue::from_str(&v) {
            headers.insert(header::CONTENT_DISPOSITION, v);
        }
    }
    Ok((StatusCode::OK, headers, printed.markdown).into_response())
}

/// Writes the document from Markdown (§6.4): a diff against the `If-Match` base, stored as one Yjs update. A stale
/// `If-Match` is 412 (`details.mergeable`), unless the document already equals the target (200 `changed: false`)
/// or `?merge=true` merges it with the newer updates. Only `text/markdown` is accepted.
#[utoipa::path(
    put,
    path = "/items/{id}/doc/markdown",
    tag = "documents",
    operation_id = "put_doc_markdown",
    security(("bearer" = ["write"]), ("session" = [])),
    params(
        ("id" = uuid::Uuid, Path, description = "The document's id"),
        ("If-Match" = String, Header, description = "`\"d{seq}\"` from a GET, or `*`"),
        MarkdownPutQuery,
    ),
    request_body(content = String, content_type = "text/markdown", description = "Wikanvas Markdown 1, up to 1 MiB"),
    responses(
        (status = 200, description = "Written (or already equal), with the new `ETag`", body = DocWriteResult),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`schema_too_new`, `read_only`, `setup_required`", body = ErrorBody),
        (status = 412, description = "`rev_mismatch` (`details {etag, mergeable}`)", body = ErrorBody),
        (status = 413, description = "`payload_too_large`, `doc_too_large`", body = ErrorBody),
        (status = 415, description = "`unsupported_media_type`", body = ErrorBody),
        (status = 422, description = "`invalid_document` (`details {path: \"line:column\", reason}`), `validation`",
            body = ErrorBody),
        (status = 428, description = "`precondition_required`", body = ErrorBody),
    )
)]
async fn put_markdown(
    auth: Auth<Write>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
    Query(q): Query<MarkdownPutQuery>,
    headers: HeaderMap,
    raw: Body,
) -> Result<Response, ApiError> {
    let actor = auth.into_actor();
    let id = id.to_string();
    authz::require(&actor, Action::Write, Resource::Item(&id))?;
    require_media(&headers, "text/markdown")?;
    let if_match = if_match(&headers)?;
    let _permits = two_permits(&state).await?;
    let bytes = body(raw, MAX_MARKDOWN_BYTES).await?;
    let (markdown, body_sha256) = blocking(move || {
        let sha: [u8; 32] = Sha256::digest(&bytes).into();
        let markdown = String::from_utf8(Vec::from(bytes))
            .map_err(|_| ApiError::validation("body", "the body is not UTF-8"))?;
        Ok((markdown, sha))
    })
    .await?;
    let title_from_h1 = q.title == Some(MarkdownPutTitle::FromH1);
    let query = format!(
        "merge={}&title={}",
        q.merge,
        if title_from_h1 { "from_h1" } else { "" }
    );
    let req = ContentWrite {
        item_id: id,
        body_sha256,
        input: ContentInput::Markdown(markdown),
        if_match,
        merge: q.merge,
        title_from_h1,
        credential: credential(&actor),
        query: format!("markdown?{query}"),
        public_origin: Some(state.config.public_url.to_string()),
        max_doc_bytes: max_doc_bytes(&state),
    };
    write(&state, req, &actor).await
}

/// Two of the three `doc_work` permits, taken at once (D25) before the body is read, held until the commit.
async fn two_permits(state: &AppState) -> Result<SemaphorePermit<'_>, ApiError> {
    state
        .doc_work
        .acquire_many(2)
        .await
        .map_err(ApiError::internal)
}

/// The §6.4 write, under the caller's two permits.
async fn write(state: &AppState, req: ContentWrite, actor: &Actor) -> Result<Response, ApiError> {
    let by = WriteBy {
        user_id: &actor.user_id,
        token_id: actor.token_id(),
    };
    let result = docs::write_content(&state.db, req, &by, now_ms()).await?;
    let (k, v) = etag_header(&result.etag);
    Ok(([(k, v)], Json(result)).into_response())
}

/// The document as normalized ProseMirror JSON (`editor.getJSON()`, §6.4), `ETag: "d{seq}"`; 304 for a matching
/// `If-None-Match`.
#[utoipa::path(
    get,
    path = "/items/{id}/doc/prosemirror",
    tag = "documents",
    operation_id = "get_doc_prosemirror",
    security(("bearer" = ["read"]), ("session" = [])),
    params(
        ("id" = uuid::Uuid, Path, description = "The document's id"),
        ("If-None-Match" = Option<String>, Header, description = "A previous `ETag`"),
    ),
    responses(
        (status = 200, description = "The document", body = DocJson),
        (status = 304, description = "Not modified"),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn get_prosemirror(
    auth: Auth<Read>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let actor = auth.into_actor();
    let id = id.to_string();
    authz::require(&actor, Action::Read, Resource::Item(&id))?;
    let _permit = state.doc_work.acquire().await.map_err(ApiError::internal)?;
    let seq_id = id.clone();
    let seq: i64 = state
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    "SELECT d.next_seq - 1 FROM doc_state d JOIN items i ON i.id = d.item_id WHERE d.item_id = ?1",
                    [seq_id],
                    |r| r.get(0),
                )
                .ok())
        })
        .await?
        .unwrap_or(-1);
    let etag = docs::etag(seq);
    if seq >= 0 && if_none_match(&headers, &etag) {
        return Ok((StatusCode::NOT_MODIFIED, [etag_header(&etag)]).into_response());
    }
    let doc = state
        .db
        .read(move |conn| docs::prosemirror(conn, &id))
        .await?;
    Ok(([etag_header(&docs::etag(doc.seq))], Json(doc)).into_response())
}

/// Writes the document from ProseMirror JSON (§6.4): validated against schema v1 (422 `invalid_document` with a
/// JSON pointer), then diffed against the `If-Match` base by `blockId` and content, and stored as one Yjs update.
#[utoipa::path(
    put,
    path = "/items/{id}/doc/prosemirror",
    tag = "documents",
    operation_id = "put_doc_prosemirror",
    security(("bearer" = ["write"]), ("session" = [])),
    params(
        ("id" = uuid::Uuid, Path, description = "The document's id"),
        ("If-Match" = String, Header, description = "`\"d{seq}\"` from a GET, or `*`"),
        ProseMirrorPutQuery,
    ),
    request_body(content = DocJsonPut, description = "Up to 4 MiB"),
    responses(
        (status = 200, description = "Written (or already equal), with the new `ETag`", body = DocWriteResult),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`, `csrf_rejected`", body = ErrorBody),
        (status = 404, description = "`not_found`", body = ErrorBody),
        (status = 409, description = "`schema_too_new`, `read_only`, `setup_required`", body = ErrorBody),
        (status = 412, description = "`rev_mismatch` (`details {etag, mergeable}`)", body = ErrorBody),
        (status = 413, description = "`payload_too_large`, `doc_too_large`", body = ErrorBody),
        (status = 415, description = "`unsupported_media_type`", body = ErrorBody),
        (status = 422, description = "`invalid_document` (`details {path, reason}`), `validation`", body = ErrorBody),
        (status = 428, description = "`precondition_required`", body = ErrorBody),
    )
)]
async fn put_prosemirror(
    auth: Auth<Write>,
    State(state): State<AppState>,
    IdPath(id): IdPath,
    Query(q): Query<ProseMirrorPutQuery>,
    headers: HeaderMap,
    raw: Body,
) -> Result<Response, ApiError> {
    let actor = auth.into_actor();
    let id = id.to_string();
    authz::require(&actor, Action::Write, Resource::Item(&id))?;
    require_media(&headers, "application/json")?;
    let if_match = if_match(&headers)?;
    let _permits = two_permits(&state).await?;
    let bytes = body(raw, MAX_JSON_BYTES).await?;
    let (put, body_sha256) = blocking(move || {
        let sha: [u8; 32] = Sha256::digest(&bytes).into();
        let put: DocJsonPut = json_body(&bytes)?;
        Ok((put, sha))
    })
    .await?;
    if put.schema_version.is_some_and(|v| v != SCHEMA_VERSION) {
        return Err(ApiError::validation(
            "schema_version",
            format!("this server writes schema version {SCHEMA_VERSION}"),
        ));
    }
    let req = ContentWrite {
        item_id: id,
        body_sha256,
        input: ContentInput::Json(put.doc),
        if_match,
        merge: q.merge,
        title_from_h1: false,
        credential: credential(&actor),
        query: format!("prosemirror?merge={}", q.merge),
        public_origin: Some(state.config.public_url.to_string()),
        max_doc_bytes: max_doc_bytes(&state),
    };
    write(&state, req, &actor).await
}

/// Document schema v1 as data: every node and mark with its attrs, types, defaults and content rules (§6.4); the
/// nodes and marks a ProseMirror PUT may use.
#[utoipa::path(
    get,
    path = "/schema/doc",
    tag = "documents",
    operation_id = "get_doc_schema",
    security(("bearer" = ["read"]), ("session" = [])),
    responses(
        (status = 200, description = "Schema v1", body = DocSchema),
        (status = 401, description = "`unauthenticated`, `invalid_token`", body = ErrorBody),
        (status = 403, description = "`insufficient_scope`", body = ErrorBody),
        (status = 409, description = "`setup_required`", body = ErrorBody),
    )
)]
async fn get_schema(_auth: Auth<Read>) -> Json<DocSchema> {
    Json(wikanvas_core::schema::v1().doc_schema())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slug("Hello, World!"), "hello-world");
        assert_eq!(slug(""), "untitled");
        assert_eq!(slug("---"), "untitled");
        assert_eq!(slug(&"a".repeat(80)).len(), 60);
    }
}
