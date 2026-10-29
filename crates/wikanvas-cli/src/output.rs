//! Human and --json output, JSON Lines on stderr, exit codes (owner: W8)
//!
//! stdout carries only the result (§7.7): with `--json` exactly one JSON document, an API body unchanged; without
//! it, short human text. stderr carries, with `--json`, JSON Lines only: `{"warning": {"code", "message"}}`,
//! `{"debug": {…}}` with `--debug`, then at most one error object; without it, human lines. Human text, on both
//! streams, shows control characters as escapes ([`human::clean`]): titles and messages come from the server, and
//! nothing may reach the terminal as an escape sequence (§7.6 "Non-interactive"). Raw content (`doc get`, `board
//! get` without `--json`, downloads) is written byte for byte.

use std::ffi::OsString;
use std::io::Write;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use wikanvas_api::cli::ExitCode;
use serde::Serialize;
use serde_json::{Value, json};

use crate::error::Failure;
use crate::human;

struct Mode {
    json: bool,
    debug: bool,
}

static MODE: OnceLock<Mode> = OnceLock::new();
static API_VERSION_WARNED: AtomicBool = AtomicBool::new(false);

/// Sets the output mode once, before any output.
pub fn init(json: bool, debug: bool) {
    let _ = MODE.set(Mode { json, debug });
}

/// Whether `--json` is in effect.
pub fn json() -> bool {
    MODE.get().is_some_and(|m| m.json)
}

/// Whether `--debug` is in effect.
pub fn debug_enabled() -> bool {
    MODE.get().is_some_and(|m| m.debug)
}

/// Whether `--json` appears anywhere in argv (the program name excluded): clap's own errors then use the JSON form
/// of §7.7 too.
pub fn json_in_argv(argv: &[OsString]) -> bool {
    argv.iter().skip(1).any(|arg| arg == "--json")
}

fn stderr_line(text: &str) {
    let mut err = std::io::stderr().lock();
    let _ = err.write_all(text.as_bytes());
    let _ = err.write_all(b"\n");
}

/// A human stderr line, its control characters escaped.
fn human_line(text: &str) {
    stderr_line(&human::clean(text));
}

/// A CLI warning: `truncated`, `attach_skipped`, `api_version_older`, `token_file_permissions`, and a few more
/// named in docs/AGENTS.md. `details` is optional extra data.
pub fn warn(code: &str, message: &str, details: Option<Value>) {
    if json() {
        let mut warning = json!({ "code": code, "message": message });
        if let Some(details) = details {
            warning["details"] = details;
        }
        stderr_line(&json!({ "warning": warning }).to_string());
    } else {
        human_line(&format!("warning: {message} [{code}]"));
    }
}

/// `api_version_older`, at most once per process.
pub fn warn_api_version_once(server: &str) {
    if !API_VERSION_WARNED.swap(true, Ordering::Relaxed) {
        warn(
            "api_version_older",
            &format!(
                "the server speaks API {server}, older than this CLI's {}: a newer command may exit 7",
                wikanvas_api::API_VERSION
            ),
            Some(json!({ "server": server, "cli": wikanvas_api::API_VERSION })),
        );
    }
}

/// A `--debug` line (method, URL, status, duration, request id; never a header).
pub fn debug(fields: Value) {
    if !debug_enabled() {
        return;
    }
    if json() {
        stderr_line(&json!({ "debug": fields }).to_string());
    } else {
        let get = |k: &str| match &fields[k] {
            Value::Null => "-".to_owned(),
            Value::String(s) => s.clone(),
            v => v.to_string(),
        };
        let mut line = format!(
            "debug: {} {} -> {} in {} ms, request {}",
            get("method"),
            get("url"),
            get("status"),
            get("duration_ms"),
            get("request_id")
        );
        if !fields["error"].is_null() {
            line.push_str(&format!(" ({})", get("error")));
        }
        human_line(&line);
    }
}

/// Prints a human note to stderr (never with `--json`).
pub fn note(text: &str) {
    if !json() {
        human_line(text);
    }
}

/// Prints the error of a failed command and returns its exit code.
pub fn fail(f: &Failure) -> ExitCode {
    if json() {
        stderr_line(&f.line().to_string());
    } else {
        let mut line = format!("error: {} [{}", f.message(), f.code());
        if let Some(status) = f.status {
            line.push_str(&format!(", HTTP {status}"));
        }
        if let Some(id) = f.request_id() {
            line.push_str(&format!(", request {id}"));
        }
        line.push(']');
        if let Some(details) = f.envelope["error"].get("details").filter(|d| !d.is_null()) {
            line.push_str(&format!("\n  details: {details}"));
        }
        human_line(&line);
    }
    f.exit
}

/// Writes bytes to stdout. A write failure (such as a closed pipe) is exit 1.
pub fn stdout_bytes(bytes: &[u8]) -> Result<(), Failure> {
    let mut out = std::io::stdout().lock();
    out.write_all(bytes)
        .and_then(|()| out.flush())
        .map_err(|e| Failure::io("writing stdout", &e))
}

/// Writes text to stdout, with a trailing newline when it has none (and is not empty), without copying it.
fn write_with_newline(text: &str) -> Result<(), Failure> {
    let mut out = std::io::stdout().lock();
    out.write_all(text.as_bytes())
        .and_then(|()| {
            if text.is_empty() || text.ends_with('\n') {
                Ok(())
            } else {
                out.write_all(b"\n")
            }
        })
        .and_then(|()| out.flush())
        .map_err(|e| Failure::io("writing stdout", &e))
}

/// Writes a human result to stdout, its control characters but `\n` escaped, with a trailing newline when it has
/// none.
pub fn print_text(text: &str) -> Result<(), Failure> {
    write_with_newline(&human::clean(text))
}

/// Writes `value` as the one JSON document of stdout (pretty-printed, with a trailing newline). It is serialized
/// straight into stdout, so a `RawValue` inside (`board get`, `doc get`) is never copied into a second buffer (§7.6
/// "Memory").
pub fn print_json(value: &impl Serialize) -> Result<(), Failure> {
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    serde_json::to_writer_pretty(&mut out, value).map_err(|e| {
        if e.is_io() {
            Failure::io("writing stdout", &std::io::Error::from(e))
        } else {
            Failure::bad_response(format!("cannot print the result: {e}"))
        }
    })?;
    out.write_all(b"\n")
        .and_then(|()| out.flush())
        .map_err(|e| Failure::io("writing stdout", &e))
}

/// Writes an API body unchanged as the one JSON document of stdout (plus a newline when it has none).
pub fn print_body(body: &str) -> Result<(), Failure> {
    write_with_newline(body)
}

/// The body-less success object of §7.7: `{"ok": true, "status": 204, "request_id": "…", "id": "…"}`.
pub fn ok_object(status: Option<u16>, request_id: Option<&str>, id: Option<&str>) -> Value {
    let mut v = json!({ "ok": true, "status": status, "request_id": request_id });
    if let Some(id) = id {
        v["id"] = Value::from(id);
    }
    v
}

/// Prints the ok object (`--json`) or a short human line.
pub fn print_ok(
    status: Option<u16>,
    request_id: Option<&str>,
    id: Option<&str>,
) -> Result<(), Failure> {
    if json() {
        print_json(&ok_object(status, request_id, id))
    } else {
        match id {
            Some(id) => print_text(&format!("ok {id}")),
            None => print_text("ok"),
        }
    }
}

/// A wrapped content result (`doc get`, `board get`): the content stays the server's raw JSON, never a value tree
/// (§7.6 "Memory"), and the fields print in this order.
#[derive(Serialize)]
pub struct Wrapped<'a, C: Serialize + ?Sized> {
    pub item_id: &'a str,
    pub format: &'a str,
    pub etag: Option<&'a str>,
    pub content: &'a C,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warnings: Option<&'a [String]>,
}

/// Where a message goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// What a clap error prints, where, and with which exit code. `--help` and `--version` go to stdout with exit 0;
/// a usage error goes to stderr with exit 2, as one §7.7 error line (`cli_usage`, `request_id: null`) when `json`.
pub fn render_clap_error(err: &clap::Error, json: bool) -> (Stream, String, ExitCode) {
    let text = err.render().to_string();
    if !err.use_stderr() {
        return (Stream::Stdout, text, ExitCode::Success);
    }
    if !json {
        return (Stream::Stderr, text, ExitCode::Usage);
    }
    let line = Failure::usage(usage_message(&text)).line().to_string();
    (Stream::Stderr, format!("{line}\n"), ExitCode::Usage)
}

/// The message of a rendered clap usage error: its first paragraph on one line, without `error: `. The paragraph
/// holds the lines that name the arguments (a missing `<--if-match <ETAG>|--force>`, `--yes`, `--kind <KIND>`); the
/// tip and `Usage:` paragraphs after the blank line are left out.
fn usage_message(text: &str) -> String {
    let mut lines = text
        .lines()
        .take_while(|l| !l.trim().is_empty())
        .map(str::trim);
    let first = lines.next().unwrap_or_default();
    let first = first.strip_prefix("error: ").unwrap_or(first);
    let rest: Vec<&str> = lines.collect();
    if rest.is_empty() {
        first.to_owned()
    } else {
        format!("{first} {}", rest.join(", "))
    }
}

/// Prints a clap error as [`render_clap_error`] says and returns its exit code.
pub fn clap_error(err: &clap::Error, json: bool) -> ExitCode {
    let (stream, text, code) = render_clap_error(err, json);
    let written = match stream {
        Stream::Stdout => std::io::stdout().lock().write_all(text.as_bytes()),
        Stream::Stderr => std::io::stderr().lock().write_all(text.as_bytes()),
    };
    match (written, code) {
        (Err(_), ExitCode::Success) => ExitCode::Unexpected,
        _ => code,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::Cli;
    use clap::Parser;

    fn err(list: &[&str]) -> clap::Error {
        Cli::try_parse_from(std::iter::once("wikanvas").chain(list.iter().copied()))
            .expect_err("a clap error")
    }

    #[test]
    fn json_flag_anywhere_in_argv() {
        let argv = |list: &[&str]| list.iter().map(OsString::from).collect::<Vec<_>>();
        assert!(json_in_argv(&argv(&["wikanvas", "exit-codes", "--json"])));
        assert!(json_in_argv(&argv(&["wikanvas", "--json", "x", "--bogus"])));
        assert!(!json_in_argv(&argv(&["wikanvas", "exit-codes"])));
        assert!(!json_in_argv(&argv(&["--json"])));
    }

    #[test]
    fn usage_error_json_line() {
        let (stream, text, code) = render_clap_error(&err(&["exit-codes", "--bogus"]), true);
        assert_eq!((stream, code), (Stream::Stderr, ExitCode::Usage));
        assert!(
            text.ends_with('\n') && text.trim_end().lines().count() == 1,
            "{text}"
        );
        let value: serde_json::Value = serde_json::from_str(&text).expect("one JSON line");
        assert_eq!(value["error"]["code"], "cli_usage");
        assert_eq!(value["request_id"], serde_json::Value::Null);
        assert_eq!(value["status"], serde_json::Value::Null);
        assert_eq!(value["exit_code"], 2);
        let message = value["error"]["message"].as_str().expect("a message");
        assert!(message.contains("--bogus"), "{message}");
        assert!(!message.starts_with("error:"), "{message}");
    }

    #[test]
    fn usage_error_json_names_the_missing_arguments() {
        for (list, wanted) in [
            (&["doc", "put", "x", "--md", "f.md"][..], "--if-match"),
            (&["trash", "purge", "--all"][..], "--yes"),
        ] {
            let (_, text, code) = render_clap_error(&err(list), true);
            assert_eq!(code, ExitCode::Usage, "{list:?}");
            assert_eq!(text.trim_end().lines().count(), 1, "{text}");
            let value: serde_json::Value = serde_json::from_str(&text).expect("one JSON line");
            let message = value["error"]["message"].as_str().expect("a message");
            assert!(
                message.starts_with("the following required arguments were not provided:"),
                "{message}"
            );
            assert!(message.contains(wanted), "{list:?}: {message}");
            assert!(!message.contains("Usage:"), "{message}");
        }
    }

    #[test]
    fn usage_error_human() {
        let (stream, text, code) = render_clap_error(&err(&["exit-codes", "--bogus"]), false);
        assert_eq!((stream, code), (Stream::Stderr, ExitCode::Usage));
        assert!(text.starts_with("error: "), "{text}");
        assert!(!text.contains('\u{1b}'), "no colour: {text}");
    }

    #[test]
    fn help_and_version_go_to_stdout() {
        for (list, json) in [(&["--help"][..], false), (&["--version"][..], true)] {
            let (stream, text, code) = render_clap_error(&err(list), json);
            assert_eq!(
                (stream, code),
                (Stream::Stdout, ExitCode::Success),
                "{list:?}"
            );
            assert!(!text.contains('\u{1b}'), "no colour: {text}");
        }
        let (_, text, _) = render_clap_error(&err(&["--version"]), false);
        assert_eq!(text, format!("wikanvas {}\n", env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn ok_object_shape() {
        assert_eq!(
            ok_object(Some(204), Some("r1"), Some("i1")),
            json!({"ok": true, "status": 204, "request_id": "r1", "id": "i1"})
        );
        assert_eq!(
            ok_object(None, None, None),
            json!({"ok": true, "status": null, "request_id": null})
        );
    }
}
