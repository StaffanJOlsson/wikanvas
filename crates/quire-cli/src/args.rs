//! The clap command tree of ARCHITECTURE §7.6 (owner: W8)
//!
//! W0b: the global flags and `exit-codes`; W8 adds the rest of the command table.

use std::ffi::OsString;
use std::path::PathBuf;

use clap::{ArgAction, Args, ColorChoice, Parser, Subcommand};
use quire_api::cli::ExitCode;

use crate::output;

/// `quire`: the command-line client for Quire's HTTP API.
#[derive(Debug, Parser)]
#[command(
    name = "quire",
    version,
    about = "The command-line client for Quire's HTTP API (/api/v1)",
    color = ColorChoice::Never,
    subcommand_required = true,
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,
    #[command(subcommand)]
    pub command: Command,
}

/// The global flags of §7.6. Every one is accepted before or after the command. Environment variables and the
/// profile file are read by `config` (the URL and the token resolve as a pair), never by clap.
#[derive(Debug, Args)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "W0b reads only --json; W8's config and http modules read the rest"
    )
)]
pub struct GlobalArgs {
    /// Print exactly one JSON document on stdout, and JSON Lines on stderr
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    pub json: bool,

    /// The Quire URL, e.g. https://wiki.example.org (overrides QUIRE_URL and the profile's url)
    #[arg(long, global = true, value_name = "URL")]
    pub url: Option<String>,

    /// The profile of the config file to use (QUIRE_URL, QUIRE_TOKEN and QUIRE_TOKEN_FILE are then ignored)
    #[arg(long, global = true, value_name = "NAME")]
    pub profile: Option<String>,

    /// Read the API token from this file
    #[arg(long, global = true, value_name = "FILE")]
    pub token_file: Option<PathBuf>,

    /// Read the API token from stdin
    #[arg(long, global = true, action = ArgAction::SetTrue, conflicts_with = "token_file")]
    pub token_stdin: bool,

    /// Request timeout in seconds [default: 30; 300 for transfers]
    #[arg(long, global = true, value_name = "SECONDS", value_parser = clap::value_parser!(u64).range(1..))]
    pub timeout: Option<u64>,

    /// Retries of requests the server did not process
    #[arg(long, global = true, value_name = "N", default_value_t = 2)]
    pub retries: u32,

    /// Trust only the CA certificates in this PEM file (overrides QUIRE_CA_FILE and the system bundle)
    #[arg(long, global = true, value_name = "FILE")]
    pub ca_file: Option<PathBuf>,

    /// Print method, URL, status, duration and request id of every request to stderr
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    pub debug: bool,
}

/// The command table of §7.6.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print the exit-code table (§7.7)
    ExitCodes,
}

/// Parses the process arguments. A usage error is printed (as a JSON line when `--json` appears anywhere in argv,
/// §7.7) and returned as its exit code; `--help` and `--version` print to stdout and return success.
pub fn parse() -> Result<Cli, ExitCode> {
    let argv: Vec<OsString> = std::env::args_os().collect();
    let json = output::json_in_argv(&argv);
    Cli::try_parse_from(argv).map_err(|err| output::clap_error(&err, json))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

    fn parse_from(list: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("quire").chain(list.iter().copied()))
    }

    #[test]
    fn command_tree_is_valid() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    #[test]
    fn global_flags_before_and_after_the_command() {
        for list in [
            &["--json", "--debug", "--retries", "0", "exit-codes"][..],
            &["exit-codes", "--json", "--debug", "--retries", "0"][..],
        ] {
            let cli = parse_from(list).expect("parses");
            assert!(cli.global.json);
            assert!(cli.global.debug);
            assert_eq!(cli.global.retries, 0);
            assert!(matches!(cli.command, Command::ExitCodes));
        }
    }

    #[test]
    fn defaults() {
        let g = parse_from(&["exit-codes"]).expect("parses").global;
        assert!(!g.json);
        assert!(!g.debug);
        assert!(!g.token_stdin);
        assert_eq!(g.retries, 2);
        assert_eq!(g.timeout, None);
        assert_eq!(g.url, None);
        assert_eq!(g.profile, None);
        assert_eq!(g.token_file, None);
        assert_eq!(g.ca_file, None);
    }

    #[test]
    fn every_global_flag_parses() {
        let g = parse_from(&[
            "exit-codes",
            "--url",
            "https://wiki.example.org",
            "--profile",
            "home",
            "--token-file",
            "/run/secrets/quire-token",
            "--timeout",
            "5",
            "--retries",
            "4",
            "--ca-file",
            "ca.pem",
        ])
        .expect("parses")
        .global;
        assert_eq!(g.url.as_deref(), Some("https://wiki.example.org"));
        assert_eq!(g.profile.as_deref(), Some("home"));
        assert_eq!(
            g.token_file.as_deref(),
            Some(std::path::Path::new("/run/secrets/quire-token"))
        );
        assert_eq!(g.timeout, Some(5));
        assert_eq!(g.retries, 4);
        assert_eq!(g.ca_file.as_deref(), Some(std::path::Path::new("ca.pem")));
        assert!(
            parse_from(&["--token-stdin", "exit-codes"])
                .expect("parses")
                .global
                .token_stdin
        );
    }

    #[test]
    fn usage_errors() {
        let cases: [(&[&str], ErrorKind); 7] = [
            (
                &["--token-stdin", "--token-file", "t", "exit-codes"],
                ErrorKind::ArgumentConflict,
            ),
            (
                &["exit-codes", "--token-file", "t", "--token-stdin"],
                ErrorKind::ArgumentConflict,
            ),
            (
                &["exit-codes", "--timeout", "0"],
                ErrorKind::ValueValidation,
            ),
            (
                &["exit-codes", "--retries", "-1"],
                ErrorKind::UnknownArgument,
            ),
            (&["exit-codes", "--bogus"], ErrorKind::UnknownArgument),
            (&["no-such-command"], ErrorKind::InvalidSubcommand),
            (&["--json"], ErrorKind::MissingSubcommand),
        ];
        for (list, kind) in cases {
            let err = parse_from(list)
                .err()
                .unwrap_or_else(|| panic!("{list:?} parsed"));
            assert_eq!(err.kind(), kind, "{list:?}");
            assert_eq!(err.exit_code(), 2, "{list:?}");
        }
        // No token flag takes the secret from argv (§7.6).
        assert!(parse_from(&["exit-codes", "--token", "qpat_x"]).is_err());
    }
}
