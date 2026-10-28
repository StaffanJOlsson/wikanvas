//! Human and --json output, JSON Lines on stderr, exit codes (owner: W8)
//!
//! W0b: writing a result to stdout, and clap's usage errors in both forms of §7.7.

use std::ffi::OsString;
use std::io::Write;

use quire_api::cli::ExitCode;
use serde::Serialize;

/// Whether `--json` appears anywhere in argv (the program name excluded): clap's own errors then use the JSON form
/// of §7.7 too.
pub fn json_in_argv(argv: &[OsString]) -> bool {
    argv.iter().skip(1).any(|arg| arg == "--json")
}

/// Writes a human result to stdout. A write failure (such as a closed pipe) exits 1 without further output.
pub fn print_text(text: &str) -> ExitCode {
    write_stdout(text.as_bytes())
}

/// Writes `value` as the one JSON document of stdout (pretty-printed, with a trailing newline).
pub fn print_json(value: &impl Serialize) -> ExitCode {
    match serde_json::to_vec_pretty(value) {
        Ok(mut bytes) => {
            bytes.push(b'\n');
            write_stdout(&bytes)
        }
        Err(_) => ExitCode::Unexpected,
    }
}

fn write_stdout(bytes: &[u8]) -> ExitCode {
    let mut out = std::io::stdout().lock();
    match out.write_all(bytes).and_then(|()| out.flush()) {
        Ok(()) => ExitCode::Success,
        Err(_) => ExitCode::Unexpected,
    }
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
    let first = text.lines().next().unwrap_or_default();
    let message = first.strip_prefix("error: ").unwrap_or(first).trim();
    let line = ErrorLine {
        error: ErrorLineInfo {
            code: "cli_usage",
            message,
        },
        request_id: None,
        status: None,
        exit_code: ExitCode::Usage.code(),
    };
    let line = serde_json::to_string(&line).unwrap_or_default();
    (Stream::Stderr, format!("{line}\n"), ExitCode::Usage)
}

/// The §7.7 error line of an error the CLI raises itself, in the envelope's field order.
#[derive(Serialize)]
struct ErrorLine<'a> {
    error: ErrorLineInfo<'a>,
    request_id: Option<&'a str>,
    /// The HTTP status; `null` when no response was involved.
    status: Option<u16>,
    exit_code: u8,
}

#[derive(Serialize)]
struct ErrorLineInfo<'a> {
    code: &'a str,
    message: &'a str,
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
        Cli::try_parse_from(std::iter::once("quire").chain(list.iter().copied()))
            .expect_err("a clap error")
    }

    #[test]
    fn json_flag_anywhere_in_argv() {
        let argv = |list: &[&str]| list.iter().map(OsString::from).collect::<Vec<_>>();
        assert!(json_in_argv(&argv(&["quire", "exit-codes", "--json"])));
        assert!(json_in_argv(&argv(&["quire", "--json", "x", "--bogus"])));
        assert!(!json_in_argv(&argv(&["quire", "exit-codes"])));
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
        assert_eq!(text, format!("quire {}\n", env!("CARGO_PKG_VERSION")));
    }
}
