//! ApiError and the error envelope for every rejection, X-Request-Id and Wikanvas-Api-Version (owner: W2).
//!
//! Every error leaves the server as `{"error": {"code", "message", "details"?}, "request_id"}` (§7.1):
//! - handlers and extractors return [`ApiError`];
//! - [`envelope`], the outermost middleware, gives every request an id, renders each `ApiError` with it, turns the
//!   remaining non-envelope errors under `/api/` (axum's own rejections, the timeout and body-limit layers, a caught
//!   panic) into the same shape, and sets `X-Request-Id` on every response and `Wikanvas-Api-Version` on every
//!   `/api/v1` response.

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::http::header::{self, HeaderMap, HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use wikanvas_api::{
    ErrorBody, ErrorCode, ErrorInfo, InsufficientScopeDetails, InvalidTokenDetails,
    InvalidTokenReason, LimitDetails, RateLimitDetails, RateLimitReason, Scope, ValidationDetails,
};
use serde::Serialize;

use crate::state::AppState;

/// An API error: a §7.1 code, a message for people, optional per-code details and extra headers
/// (`WWW-Authenticate`, `Retry-After`, `Allow`).
#[derive(Clone, Debug)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
    pub details: Option<serde_json::Value>,
    /// Boxed, so `Result<T, ApiError>` stays small.
    pub headers: Box<HeaderMap>,
}

/// The id of the current request: `X-Request-Id` and `request_id` of every error body. Stored in the request's
/// extensions by [`envelope`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestId(pub String);

impl RequestId {
    fn new() -> RequestId {
        RequestId(uuid::Uuid::now_v7().simple().to_string())
    }
}

/// Marks a response produced by [`ApiError`]; [`envelope`] renders the body with the request id.
#[derive(Clone, Debug)]
struct Pending(ErrorInfo);

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> ApiError {
        ApiError {
            code,
            message: message.into(),
            details: None,
            headers: Box::default(),
        }
    }

    /// Adds `details` (one of the `*Details` types of `wikanvas_api::error`).
    pub fn with_details(mut self, details: &impl Serialize) -> ApiError {
        self.details = serde_json::to_value(details).ok();
        self
    }

    /// Adds a response header.
    pub fn with_header(mut self, name: HeaderName, value: HeaderValue) -> ApiError {
        self.headers.insert(name, value);
        self
    }

    /// The HTTP status of the code.
    pub fn status(&self) -> StatusCode {
        StatusCode::from_u16(self.code.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    }

    /// 404 `not_found`.
    pub fn not_found(message: impl Into<String>) -> ApiError {
        ApiError::new(ErrorCode::NotFound, message)
    }

    /// 422 `validation` with `details.field`.
    pub fn validation(field: &str, message: impl Into<String>) -> ApiError {
        ApiError::new(ErrorCode::Validation, message).with_details(&ValidationDetails {
            field: field.to_owned(),
        })
    }

    /// 500 `internal`. The cause goes to the log, never to the client.
    pub fn internal(cause: impl std::fmt::Display) -> ApiError {
        tracing::error!(%cause, "internal error");
        ApiError::new(ErrorCode::Internal, "internal server error")
    }

    /// 401 `unauthenticated`.
    pub fn unauthenticated() -> ApiError {
        ApiError::new(
            ErrorCode::Unauthenticated,
            "sign in, or send Authorization: Bearer qpat_…",
        )
    }

    /// 401 `invalid_token` with `WWW-Authenticate: Bearer error="invalid_token"`; `details.reason = "expired"` only
    /// for an expired token (§7.3).
    pub fn invalid_token(expired: bool) -> ApiError {
        let err = ApiError::new(
            ErrorCode::InvalidToken,
            if expired {
                "the token has expired"
            } else {
                "the token is invalid or revoked"
            },
        )
        .with_header(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Bearer error=\"invalid_token\""),
        );
        if expired {
            err.with_details(&InvalidTokenDetails {
                reason: InvalidTokenReason::Expired,
            })
        } else {
            err
        }
    }

    /// 403 `insufficient_scope` with `details {required, granted}` and
    /// `WWW-Authenticate: Bearer error="insufficient_scope", scope="<required>"`.
    pub fn insufficient_scope(required: Scope, granted: &[Scope]) -> ApiError {
        let challenge = format!("Bearer error=\"insufficient_scope\", scope=\"{required}\"");
        ApiError::new(
            ErrorCode::InsufficientScope,
            format!("this operation needs a token with the {required} scope"),
        )
        .with_details(&InsufficientScopeDetails {
            required,
            granted: granted.to_vec(),
        })
        .with_header(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_str(&challenge).expect("ASCII"),
        )
    }

    /// 413 `payload_too_large` with `details.limit_bytes`.
    pub fn payload_too_large(limit_bytes: u64) -> ApiError {
        ApiError::new(
            ErrorCode::PayloadTooLarge,
            format!("the request body is larger than {limit_bytes} bytes"),
        )
        .with_details(&LimitDetails { limit_bytes })
    }

    /// 429 `rate_limited` with `details.reason` and `Retry-After` (whole seconds, at least 1).
    pub fn rate_limited(reason: RateLimitReason, retry_after: std::time::Duration) -> ApiError {
        let secs = crate::ratelimit::retry_after_secs(retry_after);
        let message = match reason {
            RateLimitReason::Requests => "too many requests",
            RateLimitReason::Bytes => "too many request bytes for this token",
            RateLimitReason::AuthFailures => "too many failed attempts",
            RateLimitReason::Busy => "another export or backup is running",
        };
        ApiError::new(
            ErrorCode::RateLimited,
            format!("{message}; retry in {secs} s"),
        )
        .with_details(&RateLimitDetails { reason })
        .with_header(header::RETRY_AFTER, HeaderValue::from(secs))
    }

    /// 429 `rate_limited` with `reason: "busy"`: a single-flight operation is running (D47).
    pub fn busy() -> ApiError {
        ApiError::rate_limited(RateLimitReason::Busy, std::time::Duration::from_secs(5))
    }

    fn info(&self) -> ErrorInfo {
        ErrorInfo {
            code: self.code,
            message: self.message.clone(),
            details: self.details.clone(),
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let info = self.info();
        // A body with `request_id: null` in case nothing renders it again; `envelope` always does.
        let body = render(&info, None);
        let mut res = (self.status(), body).into_response();
        res.headers_mut().extend(*self.headers);
        res.extensions_mut().insert(Pending(info));
        res
    }
}

impl From<wikanvas_store::error::Error> for ApiError {
    fn from(err: wikanvas_store::error::Error) -> ApiError {
        crate::state::store_error(err)
    }
}

fn render(info: &ErrorInfo, request_id: Option<&str>) -> Response {
    let body = ErrorBody {
        error: info.clone(),
        request_id: request_id.map(str::to_owned),
    };
    let json = serde_json::to_vec(&body).expect("the envelope serializes");
    let mut res = Response::new(Body::from(json));
    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    res
}

/// The outermost middleware (see the module docs).
pub async fn envelope(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    let id = RequestId::new();
    req.extensions_mut().insert(id.clone());
    let path = req.uri().path();
    let api = path.starts_with("/api/") || path == "/api";
    let v1 = path == "/api/v1" || path.starts_with("/api/v1/");

    let mut res = next.run(req).await;

    let pending = res.extensions_mut().remove::<Pending>();
    if let Some(Pending(info)) = pending {
        res = replace_body(res, &info, &id.0);
    } else if api && (res.status().is_client_error() || res.status().is_server_error()) {
        res = convert(res, &id.0, state.config.limits().max_body_bytes).await;
    }
    let headers = res.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&id.0) {
        headers.insert(HeaderName::from_static("x-request-id"), v);
    }
    if v1 {
        headers.insert(
            HeaderName::from_static("wikanvas-api-version"),
            HeaderValue::from_static(wikanvas_api::API_VERSION),
        );
    }
    res
}

/// Keeps status and headers, swaps the body for the envelope with the request id.
fn replace_body(res: Response, info: &ErrorInfo, request_id: &str) -> Response {
    let (mut parts, _) = res.into_parts();
    let rendered = render(info, Some(request_id));
    parts.headers.remove(header::CONTENT_LENGTH);
    parts.headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    Response::from_parts(parts, rendered.into_body())
}

/// An error response under `/api/` that is not an envelope yet (an axum rejection or a tower layer's answer).
async fn convert(res: Response, request_id: &str, body_limit: u64) -> Response {
    let is_json = res
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"));
    if is_json {
        return res;
    }
    let (mut parts, body) = res.into_parts();
    let text = axum::body::to_bytes(body, 4096)
        .await
        .map(|b| String::from_utf8_lossy(&b).trim().to_owned())
        .unwrap_or_default();
    let status = parts.status;
    let message = |fallback: &str| {
        if text.is_empty() {
            fallback.to_owned()
        } else {
            text.clone()
        }
    };
    let err = match status {
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
            let field = if text.contains("JSON") {
                "body"
            } else if text.contains("query") {
                "query"
            } else {
                "path"
            };
            ApiError::validation(field, message("the request is malformed"))
        }
        StatusCode::UNAUTHORIZED => ApiError::unauthenticated(),
        StatusCode::FORBIDDEN => ApiError::new(ErrorCode::Forbidden, message("forbidden")),
        StatusCode::NOT_FOUND => ApiError::not_found(message("no such route")),
        StatusCode::METHOD_NOT_ALLOWED => ApiError::new(
            ErrorCode::MethodNotAllowed,
            message("this route does not take that method"),
        ),
        StatusCode::REQUEST_TIMEOUT => ApiError::new(
            ErrorCode::RequestTimeout,
            "the request took longer than 30 s; it may have been processed",
        ),
        StatusCode::CONFLICT => ApiError::new(ErrorCode::Conflict, message("conflict")),
        StatusCode::PAYLOAD_TOO_LARGE => ApiError::payload_too_large(body_limit),
        StatusCode::UNSUPPORTED_MEDIA_TYPE => ApiError::new(
            ErrorCode::UnsupportedMediaType,
            message("unsupported media type"),
        ),
        StatusCode::PRECONDITION_REQUIRED => {
            ApiError::new(ErrorCode::PreconditionRequired, message("send If-Match"))
        }
        StatusCode::SERVICE_UNAVAILABLE => ApiError::new(
            ErrorCode::Unavailable,
            "the server is shutting down; the request was not processed",
        ),
        s if s.is_client_error() => ApiError::validation("request", message("bad request")),
        _ => {
            tracing::error!(status = %status, body = %text, "internal error response");
            ApiError::new(ErrorCode::Internal, "internal server error")
        }
    };
    parts.status = err.status();
    parts.headers.remove(header::CONTENT_LENGTH);
    parts.headers.extend(*err.headers.clone());
    replace_body(
        Response::from_parts(parts, Body::empty()),
        &err.info(),
        request_id,
    )
}

/// The JSON 404 for every path under `/api/` that is not a route (§7.1, §11).
pub fn api_not_found() -> ApiError {
    ApiError::not_found("no such API route; the API lives under /api/v1")
}

/// Response header names of the API contract, as `HeaderName`s.
pub fn api_version_header() -> HeaderName {
    HeaderName::from_static("wikanvas-api-version")
}

#[cfg(test)]
mod tests {
    use wikanvas_api::common::header as api_header;

    use super::*;

    #[test]
    fn header_names_match_the_contract() {
        assert!(api_header::API_VERSION.eq_ignore_ascii_case(api_version_header().as_str()));
        assert!(api_header::REQUEST_ID.eq_ignore_ascii_case("x-request-id"));
    }

    #[test]
    fn scope_error_has_challenge_and_details() {
        let err = ApiError::insufficient_scope(Scope::Write, &[Scope::Read]);
        assert_eq!(err.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            err.headers[header::WWW_AUTHENTICATE],
            "Bearer error=\"insufficient_scope\", scope=\"write\""
        );
        assert_eq!(
            err.details.unwrap(),
            serde_json::json!({"required": "write", "granted": ["read"]})
        );
        let expired = ApiError::invalid_token(true);
        assert_eq!(
            expired.details.unwrap(),
            serde_json::json!({"reason": "expired"})
        );
        assert!(ApiError::invalid_token(false).details.is_none());
    }
}
