//! One module per command group of ARCHITECTURE §7.6 (owner: W8)

mod admin;
mod api;
mod attachment;
mod auth;
mod board;
mod collection;
mod doc;
mod export;
mod item;
mod list;
mod search;
mod star;
mod token;
mod trash;
mod tree;
mod version;
mod workspace;

use quire_api::cli::ExitCode;

use crate::args::{Cli, Command};

/// Runs a parsed command line and returns its exit code.
pub fn run(cli: Cli) -> ExitCode {
    match cli.command {
        Command::ExitCodes => version::exit_codes(cli.global.json),
    }
}
