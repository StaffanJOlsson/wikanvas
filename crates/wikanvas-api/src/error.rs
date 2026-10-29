//! The error envelope and every error code with its HTTP status (ARCHITECTURE §7.1; owner: W2).
//!
//! Every error answers `{"error": {"code", "message", "details"?}, "request_id"}` with an `X-Request-Id` header holding
//! the same id. `code` is stable; `message` is English for people and may change; `details` has a per-code shape
//! (the `*Details` types below and [`crate::docs::MissingDepsDetails`]). Clients must treat an unknown `code` by its
//! HTTP status: within v1 codes may be added, so a client that must not fail on one parses `code` as a string and
//! uses [`ErrorCode::from_str`](std::str::FromStr).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;
use utoipa::ToSchema;

use crate::common::string_enum;
use crate::tokens::Scope;

string_enum! {
    /// A stable, machine-readable error code.
    pub enum ErrorCode {
        /// 401: no credentials, or an expired session.
        Unauthenticated = "unauthenticated",
        /// 401: a failed login.
        InvalidCredentials = "invalid_credentials",
        /// 401: a malformed, unknown, revoked or expired token (`WWW-Authenticate: Bearer error="invalid_token"`).
        InvalidToken = "invalid_token",
        /// 403.
        Forbidden = "forbidden",
        /// 403: a cookie-authenticated mutation failed the Fetch-Metadata / Origin check.
        CsrfRejected = "csrf_rejected",
        /// 403: the token lacks the route's scope (`details {required, granted}`).
        InsufficientScope = "insufficient_scope",
        /// 403: a session-only route called with a token.
        SessionRequired = "session_required",
        /// 404.
        NotFound = "not_found",
        /// 405.
        MethodNotAllowed = "method_not_allowed",
        /// 408: the 30 s timeout; the request may have been processed.
        RequestTimeout = "request_timeout",
        /// 409.
        Conflict = "conflict",
        /// 409: the move would put an item inside itself.
        Cycle = "cycle",
        /// 409: the target parent or collection is missing, trashed, archived or foreign.
        ParentInvalid = "parent_invalid",
        /// 409: a Yjs update depends on structs the server lacks (`details {state_vector}`).
        MissingDeps = "missing_deps",
        /// 409: no owner exists yet.
        SetupRequired = "setup_required",
        /// 409: setup was already done.
        AlreadySetup = "already_setup",
        /// 409: writing to a trashed or archived item.
        ReadOnly = "read_only",
        /// 409: `discard_empty` on an item that is not new and empty.
        NotEmpty = "not_empty",
        /// 409: the stored document is newer than schema v1.
        SchemaTooNew = "schema_too_new",
        /// 412: `If-Match` is stale (`details {etag}` plus `rev` or `mergeable`).
        RevMismatch = "rev_mismatch",
        /// 413 (`details {limit_bytes}`).
        PayloadTooLarge = "payload_too_large",
        /// 413: the stored Y document would exceed its cap (`details {limit_bytes}`).
        DocTooLarge = "doc_too_large",
        /// 415.
        UnsupportedMediaType = "unsupported_media_type",
        /// 421: `Host` is not allowed.
        HostNotAllowed = "host_not_allowed",
        /// 422 (`details {field}`).
        Validation = "validation",
        /// 422: the board envelope does not parse.
        InvalidBoard = "invalid_board",
        /// 422: the board envelope version is newer than the server's.
        BoardTooNew = "board_too_new",
        /// 422: the Yjs update does not decode.
        InvalidUpdate = "invalid_update",
        /// 422: the Markdown or ProseMirror JSON is not schema v1 (`details {path, reason}`).
        InvalidDocument = "invalid_document",
        /// 428: a content write without `If-Match`.
        PreconditionRequired = "precondition_required",
        /// 429 with `Retry-After` (`details {reason}`).
        RateLimited = "rate_limited",
        /// 500.
        Internal = "internal",
        /// 503: sent only before a handler runs (shutdown drain); the request was not processed.
        Unavailable = "unavailable",
        /// 507: below the free-space floor (D33).
        InsufficientStorage = "insufficient_storage",
    }
}

impl ErrorCode {
    /// The HTTP status this code is sent with.
    pub fn status(self) -> u16 {
        use ErrorCode::*;
        match self {
            Unauthenticated | InvalidCredentials | InvalidToken => 401,
            Forbidden | CsrfRejected | InsufficientScope | SessionRequired => 403,
            NotFound => 404,
            MethodNotAllowed => 405,
            RequestTimeout => 408,
            Conflict | Cycle | ParentInvalid | MissingDeps | SetupRequired | AlreadySetup
            | ReadOnly | NotEmpty | SchemaTooNew => 409,
            RevMismatch => 412,
            PayloadTooLarge | DocTooLarge => 413,
            UnsupportedMediaType => 415,
            HostNotAllowed => 421,
            Validation | InvalidBoard | BoardTooNew | InvalidUpdate | InvalidDocument => 422,
            PreconditionRequired => 428,
            RateLimited => 429,
            Internal => 500,
            Unavailable => 503,
            InsufficientStorage => 507,
        }
    }
}

/// The error envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct ErrorBody {
    pub error: ErrorInfo,
    /// The `X-Request-Id` of the response. Always set by the server; `null` only in errors the CLI raises itself.
    pub request_id: Option<String>,
}

/// `ErrorBody.error`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct ErrorInfo {
    pub code: ErrorCode,
    /// English, for people; may change.
    pub message: String,
    /// Per-code details, see the `*Details` schemas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub details: Option<Value>,
}

impl ErrorBody {
    /// An envelope without details.
    pub fn new(code: ErrorCode, message: impl Into<String>, request_id: impl Into<String>) -> Self {
        ErrorBody {
            error: ErrorInfo {
                code,
                message: message.into(),
                details: None,
            },
            request_id: Some(request_id.into()),
        }
    }

    /// The same envelope with `details` (any of the `*Details` types).
    pub fn with_details(mut self, details: &impl Serialize) -> Self {
        self.error.details = serde_json::to_value(details).ok();
        self
    }

    /// The HTTP status of `error.code`.
    pub fn status(&self) -> u16 {
        self.error.code.status()
    }
}

/// `details` of 422 `validation`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct ValidationDetails {
    /// The offending field or query parameter, e.g. `cursor`, `expires_in_days`, `password`.
    pub field: String,
}

/// `details` of 403 `insufficient_scope` (also named in `WWW-Authenticate`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct InsufficientScopeDetails {
    pub required: Scope,
    /// The token's scopes, expanded.
    pub granted: Vec<Scope>,
}

string_enum! {
    /// `details.reason` of 401 `invalid_token`, sent only when it helps the holder (an expired token).
    pub enum InvalidTokenReason {
        Expired = "expired",
    }
}

/// `details` of 401 `invalid_token` for an expired token; revoked, unknown and disabled tokens get no details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct InvalidTokenDetails {
    pub reason: InvalidTokenReason,
}

/// `details` of 412 `rev_mismatch`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct RevMismatchDetails {
    /// The current ETag, exactly as the header would send it.
    pub etag: String,
    /// Boards: the current revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rev: Option<i64>,
    /// Documents: whether `?merge=true` can still merge from the sent base (D50).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mergeable: Option<bool>,
}

/// `details` of 413 `payload_too_large` and `doc_too_large`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct LimitDetails {
    pub limit_bytes: u64,
}

/// `details` of 422 `invalid_document`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct InvalidDocumentDetails {
    /// A JSON pointer into the body, or `line:column` for Markdown.
    pub path: String,
    pub reason: String,
}

string_enum! {
    /// `details.reason` of 429 `rate_limited`.
    pub enum RateLimitReason {
        /// The per-token request rate.
        Requests = "requests",
        /// The per-token body bytes per hour.
        Bytes = "bytes",
        /// Too many failed bearer authentications from this IP.
        AuthFailures = "auth_failures",
        /// A single-flight operation (export, backup) is running.
        Busy = "busy",
    }
}

/// `details` of 429 `rate_limited` (with `Retry-After` in seconds).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct RateLimitDetails {
    pub reason: RateLimitReason,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_has_its_status() {
        let expect: &[(u16, &[&str])] = &[
            (
                401,
                &["unauthenticated", "invalid_credentials", "invalid_token"],
            ),
            (
                403,
                &[
                    "forbidden",
                    "csrf_rejected",
                    "insufficient_scope",
                    "session_required",
                ],
            ),
            (404, &["not_found"]),
            (405, &["method_not_allowed"]),
            (408, &["request_timeout"]),
            (
                409,
                &[
                    "conflict",
                    "cycle",
                    "parent_invalid",
                    "missing_deps",
                    "setup_required",
                    "already_setup",
                    "read_only",
                    "not_empty",
                    "schema_too_new",
                ],
            ),
            (412, &["rev_mismatch"]),
            (413, &["payload_too_large", "doc_too_large"]),
            (415, &["unsupported_media_type"]),
            (421, &["host_not_allowed"]),
            (
                422,
                &[
                    "validation",
                    "invalid_board",
                    "board_too_new",
                    "invalid_update",
                    "invalid_document",
                ],
            ),
            (428, &["precondition_required"]),
            (429, &["rate_limited"]),
            (500, &["internal"]),
            (503, &["unavailable"]),
            (507, &["insufficient_storage"]),
        ];
        let listed: usize = expect.iter().map(|(_, codes)| codes.len()).sum();
        assert_eq!(
            listed,
            ErrorCode::ALL.len(),
            "the §7.1 table and ErrorCode differ"
        );
        for (status, codes) in expect {
            for code in *codes {
                let parsed: ErrorCode = code.parse().unwrap();
                assert_eq!(parsed.status(), *status, "{code}");
                assert_eq!(parsed.as_str(), *code);
            }
        }
    }

    #[test]
    fn envelope_shape() {
        let body = ErrorBody::new(ErrorCode::Validation, "bad cursor", "req-1").with_details(
            &ValidationDetails {
                field: "cursor".into(),
            },
        );
        assert_eq!(body.status(), 422);
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({
                "error": {"code": "validation", "message": "bad cursor", "details": {"field": "cursor"}},
                "request_id": "req-1"
            })
        );
        let plain = ErrorBody::new(ErrorCode::NotFound, "no such item", "req-2");
        assert_eq!(
            serde_json::to_string(&plain).unwrap(),
            r#"{"error":{"code":"not_found","message":"no such item"},"request_id":"req-2"}"#
        );
        let back: ErrorBody = serde_json::from_str(&serde_json::to_string(&body).unwrap()).unwrap();
        assert_eq!(back, body);
    }
}
