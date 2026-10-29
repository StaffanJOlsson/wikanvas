//! The exit-code contract of the `wikanvas` CLI and of `wikanvas-server admin` (ARCHITECTURE §7.7). Changing a code is
//! a new major CLI version; `docs/AGENTS.md` repeats the table (owner: W8).

use serde::{Deserialize, Serialize};

/// A process exit code of `wikanvas` and `wikanvas-server admin`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum ExitCode {
    Success = 0,
    Unexpected = 1,
    Usage = 2,
    Configuration = 3,
    Network = 4,
    NotAuthenticated = 5,
    Forbidden = 6,
    NotFound = 7,
    Conflict = 8,
    PreconditionFailed = 9,
    InvalidInput = 10,
    RateLimited = 11,
    ServerError = 12,
}

/// One row of `wikanvas exit-codes --json`: `{"code": 0, "name": "success", "meaning": "Success"}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitCodeInfo {
    pub code: u8,
    pub name: String,
    pub meaning: String,
}

impl ExitCode {
    /// Every code, in numeric order.
    pub const ALL: [ExitCode; 13] = [
        ExitCode::Success,
        ExitCode::Unexpected,
        ExitCode::Usage,
        ExitCode::Configuration,
        ExitCode::Network,
        ExitCode::NotAuthenticated,
        ExitCode::Forbidden,
        ExitCode::NotFound,
        ExitCode::Conflict,
        ExitCode::PreconditionFailed,
        ExitCode::InvalidInput,
        ExitCode::RateLimited,
        ExitCode::ServerError,
    ];

    /// The numeric code.
    pub fn code(self) -> u8 {
        self as u8
    }

    /// The code for a number, if it is one of the table's.
    pub fn from_code(code: u8) -> Option<ExitCode> {
        ExitCode::ALL.get(usize::from(code)).copied()
    }

    /// The stable snake_case name.
    pub fn name(self) -> &'static str {
        match self {
            ExitCode::Success => "success",
            ExitCode::Unexpected => "unexpected",
            ExitCode::Usage => "usage",
            ExitCode::Configuration => "configuration",
            ExitCode::Network => "network",
            ExitCode::NotAuthenticated => "not_authenticated",
            ExitCode::Forbidden => "forbidden",
            ExitCode::NotFound => "not_found",
            ExitCode::Conflict => "conflict",
            ExitCode::PreconditionFailed => "precondition_failed",
            ExitCode::InvalidInput => "invalid_input",
            ExitCode::RateLimited => "rate_limited",
            ExitCode::ServerError => "server_error",
        }
    }

    /// The "Meaning" column of §7.7.
    pub fn meaning(self) -> &'static str {
        match self {
            ExitCode::Success => "Success",
            ExitCode::Unexpected => "Unexpected error",
            ExitCode::Usage => "Usage",
            ExitCode::Configuration => "Configuration",
            ExitCode::Network => "Network",
            ExitCode::NotAuthenticated => "Not authenticated",
            ExitCode::Forbidden => "Forbidden",
            ExitCode::NotFound => "Not found",
            ExitCode::Conflict => "Conflict",
            ExitCode::PreconditionFailed => "Precondition failed",
            ExitCode::InvalidInput => "Invalid input",
            ExitCode::RateLimited => "Rate limited",
            ExitCode::ServerError => "Server error",
        }
    }

    /// The "Typical causes" column of §7.7 (empty for success).
    pub fn causes(self) -> &'static str {
        match self {
            ExitCode::Success => "",
            ExitCode::Unexpected => "A CLI bug or an unparseable response, nothing else",
            ExitCode::Usage => {
                "Bad flags; a missing --yes, or --if-match/--force; a refused api path or header"
            }
            ExitCode::Configuration => {
                "No URL or token; a malformed token (prefix, length or checksum); a token not bound to the URL; an \
                 unreadable or group-readable profile; plain HTTP to a non-loopback host; no CA bundle; a 3xx \
                 redirect; wikanvas-server not found for admin; 421 host_not_allowed"
            }
            ExitCode::Network => {
                "DNS, connection refused, TLS failure, timeout or reset; details.sent says whether a write may have \
                 happened"
            }
            ExitCode::NotAuthenticated => {
                "401: unauthenticated, invalid_token, invalid_credentials"
            }
            ExitCode::Forbidden => {
                "403: insufficient_scope, session_required, forbidden, csrf_rejected"
            }
            ExitCode::NotFound => "404",
            ExitCode::Conflict => {
                "409: cycle, parent_invalid, read_only, not_empty, missing_deps, setup_required, already_setup, \
                 schema_too_new, conflict; for admin create-owner, an owner exists"
            }
            ExitCode::PreconditionFailed => {
                "412 rev_mismatch, 428 precondition_required: GET again (or use --merge) and retry"
            }
            ExitCode::InvalidInput => "400, 405, 413, 415, 422, and any other 4xx not listed here",
            ExitCode::RateLimited => "429 after the retries",
            ExitCode::ServerError => "408, 500, 503, 507 and any other 5xx, after the retries",
        }
    }

    /// The exit code for a failed (or finished) request: the HTTP `status` decides, except for the codes of errors
    /// the CLI raises itself (`cli_usage` → 2, `cli_config` → 3, `network`, `timeout` and `tls` → 4) and
    /// `host_not_allowed` (3). Unlisted 4xx → 10; 408 and unlisted 5xx → 12; 3xx → 3 (redirects are never followed);
    /// 2xx → 0; anything else → 1.
    pub fn from_status(status: u16, code: Option<&str>) -> ExitCode {
        match code {
            Some("cli_usage") => return ExitCode::Usage,
            Some("cli_config" | "host_not_allowed") => return ExitCode::Configuration,
            Some("network" | "timeout" | "tls") => return ExitCode::Network,
            _ => {}
        }
        match status {
            200..=299 => ExitCode::Success,
            300..=399 => ExitCode::Configuration,
            401 => ExitCode::NotAuthenticated,
            403 => ExitCode::Forbidden,
            404 => ExitCode::NotFound,
            408 => ExitCode::ServerError,
            409 => ExitCode::Conflict,
            412 | 428 => ExitCode::PreconditionFailed,
            421 => ExitCode::Configuration,
            429 => ExitCode::RateLimited,
            400..=499 => ExitCode::InvalidInput,
            500..=599 => ExitCode::ServerError,
            _ => ExitCode::Unexpected,
        }
    }

    /// The row of `wikanvas exit-codes --json`.
    pub fn info(self) -> ExitCodeInfo {
        ExitCodeInfo {
            code: self.code(),
            name: self.name().to_owned(),
            meaning: self.meaning().to_owned(),
        }
    }

    /// The whole table, in numeric order.
    pub fn table() -> Vec<ExitCodeInfo> {
        ExitCode::ALL.iter().map(|c| c.info()).collect()
    }
}

impl From<ExitCode> for std::process::ExitCode {
    fn from(code: ExitCode) -> Self {
        std::process::ExitCode::from(code.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_numbered_in_order() {
        for (i, code) in ExitCode::ALL.iter().enumerate() {
            assert_eq!(usize::from(code.code()), i);
            assert_eq!(ExitCode::from_code(code.code()), Some(*code));
        }
        assert_eq!(ExitCode::from_code(13), None);
        let table = ExitCode::table();
        assert_eq!(table.len(), 13);
        assert_eq!(
            serde_json::to_value(&table[9]).unwrap(),
            serde_json::json!({"code": 9, "name": "precondition_failed", "meaning": "Precondition failed"})
        );
        let names: std::collections::HashSet<_> = table.iter().map(|r| r.name.clone()).collect();
        assert_eq!(names.len(), 13);
    }

    #[test]
    fn listed_statuses() {
        use ExitCode::*;
        let cases = [
            (200, Success),
            (201, Success),
            (204, Success),
            (400, InvalidInput),
            (401, NotAuthenticated),
            (403, Forbidden),
            (404, NotFound),
            (405, InvalidInput),
            (408, ServerError),
            (409, Conflict),
            (412, PreconditionFailed),
            (413, InvalidInput),
            (415, InvalidInput),
            (421, Configuration),
            (422, InvalidInput),
            (428, PreconditionFailed),
            (429, RateLimited),
            (500, ServerError),
            (503, ServerError),
            (507, ServerError),
        ];
        for (status, expected) in cases {
            assert_eq!(ExitCode::from_status(status, None), expected, "{status}");
        }
    }

    #[test]
    fn unlisted_statuses() {
        use ExitCode::*;
        // Any other 4xx is invalid input; 408 and any other 5xx are server errors; 3xx is a configuration error.
        for status in [
            402, 406, 407, 410, 411, 414, 416, 417, 418, 422, 423, 424, 425, 426, 431, 451, 499,
        ] {
            assert_eq!(
                ExitCode::from_status(status, None),
                InvalidInput,
                "{status}"
            );
        }
        for status in [408, 501, 502, 504, 505, 508, 511, 599] {
            assert_eq!(ExitCode::from_status(status, None), ServerError, "{status}");
        }
        for status in [300, 301, 302, 303, 304, 307, 308, 399] {
            assert_eq!(
                ExitCode::from_status(status, None),
                Configuration,
                "{status}"
            );
        }
        for status in [0, 100, 101, 199, 600, 999] {
            assert_eq!(ExitCode::from_status(status, None), Unexpected, "{status}");
        }
    }

    #[test]
    fn codes_that_decide_alone() {
        use ExitCode::*;
        assert_eq!(ExitCode::from_status(0, Some("cli_usage")), Usage);
        assert_eq!(ExitCode::from_status(0, Some("cli_config")), Configuration);
        assert_eq!(ExitCode::from_status(0, Some("network")), Network);
        assert_eq!(ExitCode::from_status(0, Some("timeout")), Network);
        assert_eq!(ExitCode::from_status(0, Some("tls")), Network);
        assert_eq!(
            ExitCode::from_status(421, Some("host_not_allowed")),
            Configuration
        );
        // Server codes do not override the status, and an unknown code falls back to it.
        assert_eq!(ExitCode::from_status(409, Some("setup_required")), Conflict);
        assert_eq!(
            ExitCode::from_status(401, Some("invalid_token")),
            NotAuthenticated
        );
        assert_eq!(
            ExitCode::from_status(418, Some("some_future_code")),
            InvalidInput
        );
        assert_eq!(
            ExitCode::from_status(502, Some("some_future_code")),
            ServerError
        );
    }

    #[test]
    fn converts_to_process_exit_code() {
        assert_eq!(
            std::process::ExitCode::from(ExitCode::PreconditionFailed),
            std::process::ExitCode::from(9)
        );
    }
}
