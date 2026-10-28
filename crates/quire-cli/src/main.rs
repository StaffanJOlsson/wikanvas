//! The quire binary: the CLI over /api/v1 (ARCHITECTURE §7.6, §7.7; owner: W8)

mod args;
mod cmd;
mod config;
mod http;
mod output;
mod refs;

fn main() -> std::process::ExitCode {
    match args::parse() {
        Ok(cli) => cmd::run(cli).into(),
        Err(code) => code.into(),
    }
}
