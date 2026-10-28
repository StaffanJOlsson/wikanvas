//! The error of a command: the §7.7 error object (owner: W8)
//!
//! An API error keeps the server's `ErrorBody` unchanged (unknown fields included) and gains `status` and
//! `exit_code`. Errors the CLI raises itself use the same envelope with `request_id: null` and one of the codes
//! `cli_usage` (2), `cli_config` (3), `network`, `timeout` or `tls` (4), or, for exit 1, `bad_response` (an
//! unparseable response) and `cli_io` (a local file could not be written).

use quire_api::cli::ExitCode;
use serde_json::{Map, Value, json};

/// A failed command: what `output::fail` prints and the exit code.
#[derive(Debug)]
pub struct Failure {
    pub exit: ExitCode,
    /// The HTTP status; `None` when no response was involved.
    pub status: Option<u16>,
    /// `{"error": {"code", "message", "details"?}, "request_id"}`.
    pub envelope: Map<String, Value>,
}

impl Failure {
    /// An error the CLI raises itself, with `request_id: null`. The exit code follows from `code` (§7.7).
    pub fn cli(code: &str, message: impl Into<String>, details: Option<Value>) -> Failure {
        let exit = match code {
            "bad_response" | "cli_io" => ExitCode::Unexpected,
            _ => ExitCode::from_status(0, Some(code)),
        };
        Failure::with_exit(exit, None, code, message, details, None)
    }

    /// An envelope with an explicit exit code and status.
    pub fn with_exit(
        exit: ExitCode,
        status: Option<u16>,
        code: &str,
        message: impl Into<String>,
        details: Option<Value>,
        request_id: Option<String>,
    ) -> Failure {
        let mut error = Map::new();
        error.insert("code".into(), Value::from(code));
        error.insert("message".into(), Value::from(message.into()));
        if let Some(details) = details {
            error.insert("details".into(), details);
        }
        let mut envelope = Map::new();
        envelope.insert("error".into(), Value::Object(error));
        envelope.insert(
            "request_id".into(),
            request_id.map(Value::from).unwrap_or(Value::Null),
        );
        Failure {
            exit,
            status,
            envelope,
        }
    }

    /// Exit 2: bad flags or arguments, a refused `api` path or header, an unreadable input file.
    pub fn usage(message: impl Into<String>) -> Failure {
        Failure::cli("cli_usage", message, None)
    }

    /// Exit 3: no URL or token, a malformed or unbound token, an unreadable profile, plain HTTP, no CA bundle.
    pub fn config(message: impl Into<String>) -> Failure {
        Failure::cli("cli_config", message, None)
    }

    /// Exit 1: a response the CLI cannot use.
    pub fn bad_response(message: impl Into<String>) -> Failure {
        Failure::cli("bad_response", message, None)
    }

    /// Exit 1: a local file could not be written.
    pub fn io(what: &str, err: &std::io::Error) -> Failure {
        Failure::cli("cli_io", format!("{what}: {err}"), None)
    }

    /// Exit 4: the request failed in the network; `sent` says whether it may have reached the server.
    pub fn network(code: &str, message: impl Into<String>, sent: bool) -> Failure {
        Failure::cli(code, message, Some(json!({ "sent": sent })))
    }

    /// Exit 3: a 3xx, which the CLI never follows (a token never travels to another URL).
    pub fn redirect(status: u16, location: Option<&str>, request_id: Option<String>) -> Failure {
        let message = match location {
            Some(loc) => format!(
                "the server answered {status} with a redirect to {loc}; redirects are never followed: fix the URL"
            ),
            None => format!(
                "the server answered {status} (a redirect); redirects are never followed: fix the URL"
            ),
        };
        Failure::with_exit(
            ExitCode::Configuration,
            Some(status),
            "cli_config",
            message,
            Some(json!({ "location": location })),
            request_id,
        )
    }

    /// An HTTP error response. The server's envelope is kept as it is; a body that is not one (a proxy's page, an
    /// empty body) becomes `{"code": "http_error", …}`. The exit code follows from the status and the code (§7.7).
    pub fn from_response(status: u16, request_id: Option<String>, body: &[u8]) -> Failure {
        let parsed: Option<Map<String, Value>> = serde_json::from_slice(body).ok();
        if let Some(mut envelope) = parsed.filter(|m| {
            m.get("error")
                .and_then(|e| e.get("code"))
                .is_some_and(Value::is_string)
        }) {
            let code = envelope["error"]["code"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            if envelope.get("request_id").is_none_or(Value::is_null) {
                envelope.insert(
                    "request_id".into(),
                    request_id.map(Value::from).unwrap_or(Value::Null),
                );
            }
            return Failure {
                exit: ExitCode::from_status(status, Some(&code)),
                status: Some(status),
                envelope,
            };
        }
        Failure::with_exit(
            ExitCode::from_status(status, None),
            Some(status),
            "http_error",
            format!("the server answered HTTP {status} without an error body"),
            None,
            request_id,
        )
    }

    /// The `error.code`.
    pub fn code(&self) -> &str {
        self.envelope
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default()
    }

    /// The `error.message`.
    pub fn message(&self) -> &str {
        self.envelope
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(Value::as_str)
            .unwrap_or_default()
    }

    /// The `request_id`, when there is one.
    pub fn request_id(&self) -> Option<&str> {
        self.envelope.get("request_id").and_then(Value::as_str)
    }

    /// The §7.7 error object: the envelope plus `status` and `exit_code`.
    pub fn line(&self) -> Value {
        let mut line = self.envelope.clone();
        line.insert(
            "status".into(),
            self.status.map(Value::from).unwrap_or(Value::Null),
        );
        line.insert("exit_code".into(), Value::from(self.exit.code()));
        Value::Object(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_codes_decide_the_exit_code() {
        assert_eq!(Failure::usage("x").exit, ExitCode::Usage);
        assert_eq!(Failure::config("x").exit, ExitCode::Configuration);
        assert_eq!(
            Failure::network("timeout", "x", true).exit,
            ExitCode::Network
        );
        assert_eq!(Failure::bad_response("x").exit, ExitCode::Unexpected);
        let line = Failure::network("network", "refused", false).line();
        assert_eq!(line["error"]["details"]["sent"], false);
        assert_eq!(line["request_id"], Value::Null);
        assert_eq!(line["status"], Value::Null);
        assert_eq!(line["exit_code"], 4);
    }

    #[test]
    fn server_envelope_is_kept() {
        let body = br#"{"error":{"code":"cycle","message":"m","details":{"x":1}},"request_id":"r1","extra":true}"#;
        let f = Failure::from_response(409, Some("ignored".into()), body);
        assert_eq!(f.exit, ExitCode::Conflict);
        let line = f.line();
        assert_eq!(line["error"]["details"]["x"], 1);
        assert_eq!(line["request_id"], "r1");
        assert_eq!(line["extra"], true);
        assert_eq!(line["status"], 409);
        assert_eq!(line["exit_code"], 8);
    }

    #[test]
    fn other_bodies_become_http_error() {
        let f = Failure::from_response(502, Some("r2".into()), b"<html>bad gateway</html>");
        assert_eq!(f.exit, ExitCode::ServerError);
        assert_eq!(f.code(), "http_error");
        assert_eq!(f.request_id(), Some("r2"));
        let f = Failure::from_response(421, None, b"");
        assert_eq!(f.exit, ExitCode::Configuration);
        let f = Failure::from_response(
            421,
            None,
            br#"{"error":{"code":"host_not_allowed","message":"m"},"request_id":"r"}"#,
        );
        assert_eq!(f.exit, ExitCode::Configuration);
    }

    #[test]
    fn redirect_names_the_location() {
        let f = Failure::redirect(302, Some("https://wiki.example.org/"), None);
        assert_eq!(f.exit, ExitCode::Configuration);
        assert!(f.message().contains("https://wiki.example.org/"));
        assert_eq!(
            f.line()["error"]["details"]["location"],
            "https://wiki.example.org/"
        );
    }
}
