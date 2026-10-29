//! The store error type, which wraps quire_core::api::error::ErrorCode (owner: W1).
//!
//! Every store function returns [`Result`]. An [`Error`] always carries the API [`ErrorCode`] the server answers with
//! (`ApiError: From<quire_store::Error>`), an English message, and optional `details` in the per-code shape of §7.1.
//! Database, I/O and migration failures become `internal` and keep their source for the log.

use std::fmt;

use quire_core::api::{ErrorCode, LimitDetails, ValidationDetails};
use serde::Serialize;
use serde_json::Value;

/// The store's result type.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// A store error: an API error code with its message and details, plus the underlying cause for `internal` errors.
pub struct Error {
    /// The code the server answers with (its HTTP status is `code.status()`).
    pub code: ErrorCode,
    /// English, for people.
    pub message: String,
    /// Per-code details (`ValidationDetails`, `LimitDetails`, …), already serialized.
    pub details: Option<Value>,
    source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

impl Error {
    /// An error with a code and a message, without details.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Error {
            code,
            message: message.into(),
            details: None,
            source: None,
        }
    }

    /// The same error with `details` (any of the `quire_api` `*Details` types).
    pub fn with_details(mut self, details: &impl Serialize) -> Self {
        self.details = serde_json::to_value(details).ok();
        self
    }

    /// The same error with an underlying cause (kept for the log, never sent).
    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    /// 422 `validation` with `details {field}`.
    pub fn validation(field: &str, message: impl Into<String>) -> Self {
        Error::new(ErrorCode::Validation, message).with_details(&ValidationDetails {
            field: field.to_owned(),
        })
    }

    /// 404 `not_found`.
    pub fn not_found(message: impl Into<String>) -> Self {
        Error::new(ErrorCode::NotFound, message)
    }

    /// 413 `payload_too_large` with `details {limit_bytes}`.
    pub fn too_large(limit_bytes: u64) -> Self {
        Error::new(
            ErrorCode::PayloadTooLarge,
            format!("the body is larger than {limit_bytes} bytes"),
        )
        .with_details(&LimitDetails { limit_bytes })
    }

    /// 500 `internal` with a cause.
    pub fn internal(
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Error::new(ErrorCode::Internal, message).with_source(source)
    }

    /// Whether this error has `code`.
    pub fn is(&self, code: ErrorCode) -> bool {
        self.code == code
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = f.debug_struct("Error");
        s.field("code", &self.code).field("message", &self.message);
        if let Some(details) = &self.details {
            s.field("details", details);
        }
        if let Some(source) = &self.source {
            s.field("source", source);
        }
        s.finish()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)?;
        if let Some(source) = &self.source {
            write!(f, " ({source})")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|s| s as &(dyn std::error::Error + 'static))
    }
}

impl From<rusqlite::Error> for Error {
    /// `RAISE(ABORT, '<code>')` from a trigger (e.g. `parent_invalid`) becomes that code; no row becomes
    /// `not_found`; everything else is `internal`.
    fn from(e: rusqlite::Error) -> Self {
        if let rusqlite::Error::QueryReturnedNoRows = e {
            return Error::not_found("not found");
        }
        if let rusqlite::Error::SqliteFailure(_, Some(msg)) = &e
            && let Ok(code) = msg.parse::<ErrorCode>()
        {
            return Error::new(code, msg.clone()).with_source(e);
        }
        Error::internal("database error", e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::internal("file system error", e)
    }
}

impl From<rusqlite_migration::Error> for Error {
    fn from(e: rusqlite_migration::Error) -> Self {
        Error::internal("migration failed", e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::internal("JSON error", e)
    }
}

impl From<quire_core::cursor::CursorError> for Error {
    /// A refused cursor is 422 `validation` with `details.field = "cursor"` (D46, §7.1 "Pagination").
    fn from(e: quire_core::cursor::CursorError) -> Self {
        Error::validation(quire_core::cursor::CursorError::FIELD, e.to_string())
    }
}

impl From<quire_core::rank::RankError> for Error {
    /// A sort key that cannot be made is a store bug or a corrupt row: `internal`.
    fn from(e: quire_core::rank::RankError) -> Self {
        Error::internal("sort key", e)
    }
}

impl From<quire_core::tree::TreeError> for Error {
    /// The tree rules' refusals, with their §7.1 codes.
    fn from(e: quire_core::tree::TreeError) -> Self {
        use quire_core::tree::TreeError;
        let code = match e {
            TreeError::Cycle => ErrorCode::Cycle,
            TreeError::ParentInvalid => ErrorCode::ParentInvalid,
            TreeError::ReadOnly => ErrorCode::ReadOnly,
            TreeError::Conflict => ErrorCode::Conflict,
            TreeError::NotFound => ErrorCode::NotFound,
        };
        Error::new(code, e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_messages_become_their_code() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE t (x INTEGER);
             CREATE TRIGGER t_guard BEFORE INSERT ON t BEGIN SELECT RAISE(ABORT, 'parent_invalid'); END;",
        )
        .unwrap();
        let e: Error = conn
            .execute_batch("INSERT INTO t VALUES (1)")
            .unwrap_err()
            .into();
        assert!(e.is(ErrorCode::ParentInvalid), "{e:?}");
        let e: Error = conn
            .query_row("SELECT 1 WHERE 0", [], |r| r.get::<_, i64>(0))
            .unwrap_err()
            .into();
        assert!(e.is(ErrorCode::NotFound));
        let e: Error = conn.execute_batch("SELECT nope").unwrap_err().into();
        assert!(e.is(ErrorCode::Internal));
        assert!(std::error::Error::source(&e).is_some());
    }

    #[test]
    fn validation_has_field_details() {
        let e = Error::validation("cursor", "bad cursor");
        assert_eq!(e.code.status(), 422);
        assert_eq!(e.details, Some(serde_json::json!({"field": "cursor"})));
        assert_eq!(
            Error::too_large(5).details,
            Some(serde_json::json!({"limit_bytes": 5}))
        );
    }

    #[test]
    fn cursor_errors_are_422_on_the_cursor_field() {
        use quire_core::cursor::{self, Cursor, CursorError};
        for e in [CursorError::Malformed, CursorError::QueryMismatch] {
            let e = Error::from(e);
            assert!(e.is(ErrorCode::Validation), "{e:?}");
            assert_eq!(e.code.status(), 422);
            assert_eq!(e.details, Some(serde_json::json!({"field": "cursor"})));
        }
        let decode = |s: &str| -> Result<Cursor> { Ok(cursor::decode(s, [0; 8])?) };
        assert!(
            decode("not-a-cursor")
                .unwrap_err()
                .is(ErrorCode::Validation)
        );
    }
}
