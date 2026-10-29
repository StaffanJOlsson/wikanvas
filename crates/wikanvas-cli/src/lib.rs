//! The `wikanvas` CLI over /api/v1 (ARCHITECTURE §7.6, §7.7; owner: W8)
//!
//! The binary (`src/main.rs`) calls [`main`]. The library exists so the tests can walk the clap tree ([`args`]) and
//! check the API coverage table ([`OPERATIONS`], [`API_ONLY`]) against `docs/api/openapi.json`.

pub mod args;
mod cmd;
mod config;
mod error;
mod http;
mod human;
mod markdown;
mod output;
mod refs;

pub use cmd::{ALL_CAP, API_ONLY, OPERATIONS};

/// Parses argv, runs the command and returns the process exit code (§7.7).
pub fn main() -> std::process::ExitCode {
    match args::parse() {
        Ok(cli) => cmd::run(cli).into(),
        Err(code) => code.into(),
    }
}
