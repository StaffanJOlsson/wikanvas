//! The wikanvas-server command line: serve, healthcheck and admin (owner: W2)
//!
//! D1, §7.8: `serve` (configuration from `WIKANVAS_*`), `healthcheck [--url URL]` (exit 0 when `/healthz` answers 200,
//! 1 otherwise; the default URL is `WIKANVAS_LISTEN`'s port on 127.0.0.1) and `admin …`. Usage errors exit 2 (§7.7).

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use clap::{ArgGroup, Args, Parser, Subcommand};

use crate::config::{self, Config};

/// `wikanvas-server --version`: the crate version and the commit the binary was built from (`WIKANVAS_COMMIT`).
pub fn version() -> &'static str {
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION.get_or_init(|| format!("{} ({})", env!("CARGO_PKG_VERSION"), commit()))
}

/// The git commit of the build (`WIKANVAS_COMMIT` at compile time; `unknown` without it).
pub fn commit() -> &'static str {
    option_env!("WIKANVAS_COMMIT").unwrap_or("unknown")
}

#[derive(Debug, Parser)]
#[command(name = "wikanvas-server", version = version(), about = "Wikanvas's server and its admin commands")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Serve the wiki (configuration from the WIKANVAS_* environment variables).
    Serve,
    /// Probe /healthz: exit 0 when it answers 200, 1 otherwise.
    Healthcheck {
        /// The URL to probe (default: WIKANVAS_LISTEN's port on 127.0.0.1). A URL without a path gets /healthz.
        #[arg(long)]
        url: Option<String>,
    },
    /// Administration of the database in WIKANVAS_DATA_DIR (may run while the server is up).
    Admin(AdminArgs),
}

#[derive(Debug, Args)]
pub struct AdminArgs {
    /// Print the result as one JSON document on stdout, and errors as a JSON line on stderr.
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: AdminCommand,
}

#[derive(Debug, Subcommand)]
pub enum AdminCommand {
    /// Create the owner and the workspace (the POST /setup transaction without the setup code). Exit 8 when an owner
    /// exists.
    CreateOwner(CreateOwnerArgs),
    /// Create a personal access token for the owner (or for --email) and print it once.
    CreateToken(CreateTokenArgs),
    /// List the owner's tokens.
    ListTokens,
    /// Revoke a token (and the tokens it created), or every token with --all --yes.
    #[command(group(ArgGroup::new("target").required(true).args(["id", "all"])))]
    RevokeToken {
        /// The token's id.
        id: Option<String>,
        /// Revoke every token.
        #[arg(long, requires = "yes")]
        all: bool,
        /// Confirm --all.
        #[arg(long)]
        yes: bool,
    },
    /// Set a random temporary password, print it, require a change, and revoke every session.
    ResetPassword {
        #[arg(long)]
        email: String,
        /// Also revoke every API token of the user.
        #[arg(long)]
        revoke_tokens: bool,
    },
    /// Back up the database (VACUUM INTO, then quick_check).
    Backup {
        /// Where to write the copy (default: /data/backups/wikanvas-<time>.db, counted against WIKANVAS_BACKUP_KEEP).
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Check the database: quick_check, foreign keys, cycles, the search index and missing blob files.
    Check,
}

#[derive(Debug, Args)]
#[command(group(ArgGroup::new("password").required(true).args(["password_stdin", "generate_password"])))]
pub struct CreateOwnerArgs {
    #[arg(long)]
    pub email: String,
    #[arg(long)]
    pub name: String,
    /// The workspace name.
    #[arg(long)]
    pub workspace: String,
    /// Read the password (12–1024 bytes) from stdin.
    #[arg(long)]
    pub password_stdin: bool,
    /// Generate a password, print it once, and require a change at the first sign-in.
    #[arg(long)]
    pub generate_password: bool,
    /// Do not seed the "Getting started" collection.
    #[arg(long)]
    pub no_seed: bool,
}

#[derive(Debug, Args)]
pub struct CreateTokenArgs {
    #[arg(long)]
    pub name: String,
    #[arg(long, value_parser = ["read", "write", "admin"])]
    pub scope: String,
    /// `<days>d` or `never`.
    #[arg(long, default_value = "90d")]
    pub expires: String,
    /// The user (default: the owner).
    #[arg(long)]
    pub email: Option<String>,
    /// Write the token to this file (mode 0600) instead of stdout.
    #[arg(long)]
    pub token_out: Option<PathBuf>,
}

/// Parses argv and runs the command; returns the process exit code.
pub fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let code = match cli.command {
        Command::Serve => serve(),
        Command::Healthcheck { url } => healthcheck(url),
        Command::Admin(args) => crate::admin::run(args).code(),
    };
    std::process::ExitCode::from(code)
}

fn serve() -> u8 {
    init_tracing();
    let config = match Config::from_env() {
        Ok(config) => config,
        Err(e) => {
            tracing::error!("configuration: {e}");
            return 3;
        }
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(config.worker_threads)
        .max_blocking_threads(config.max_blocking_threads)
        .enable_all()
        .build();
    let runtime = match runtime {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!("tokio runtime: {e}");
            return 1;
        }
    };
    match runtime.block_on(crate::app::run(config, shutdown_signal())) {
        Ok(()) => 0,
        Err(e) => {
            tracing::error!("{e}");
            1
        }
    }
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    // Colours only on a terminal: `docker compose logs` and log files get plain text.
    let ansi = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(ansi)
        .try_init();
}

/// SIGTERM (Docker's stop) or SIGINT.
async fn shutdown_signal() {
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    tokio::select! {
        _ = term => {}
        _ = tokio::signal::ctrl_c() => {}
    }
    tracing::info!("shutting down");
}

/// `healthcheck`: 0 when the URL answers 200 within 3 s, 1 otherwise (also for a bad URL or configuration).
fn healthcheck(url: Option<String>) -> u8 {
    let url = match url {
        Some(u) => u,
        None => match Config::from_env() {
            Ok(c) => config::healthcheck_url(c.listen),
            Err(e) => {
                eprintln!("wikanvas-server healthcheck: {e}");
                return 1;
            }
        },
    };
    match probe(&url, Duration::from_secs(3)) {
        Ok(200) => 0,
        Ok(status) => {
            eprintln!("wikanvas-server healthcheck: {url} answered {status}");
            1
        }
        Err(e) => {
            eprintln!("wikanvas-server healthcheck: {url}: {e}");
            1
        }
    }
}

/// A minimal HTTP/1.1 GET over plain TCP (the server links no HTTP client, D29); returns the status code.
pub fn probe(url: &str, timeout: Duration) -> Result<u16, String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or("only http:// URLs can be probed")?;
    let (authority, path) = match rest.find('/') {
        Some(i) if &rest[i..] != "/" => (&rest[..i], &rest[i..]),
        Some(i) => (&rest[..i], "/healthz"),
        None => (rest, "/healthz"),
    };
    let target = if authority.contains(':') && !authority.ends_with(']') {
        authority.to_owned()
    } else {
        format!("{authority}:80")
    };
    let addrs: Vec<SocketAddr> = target
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .collect();
    let addr = addrs.first().ok_or("no address")?;
    let mut stream = TcpStream::connect_timeout(addr, timeout).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {authority}\r\nUser-Agent: wikanvas-server-healthcheck\r\n\
         Connection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|e| e.to_string())?;
    let mut head = [0u8; 64];
    let mut len = 0;
    while len < 12 {
        let n = stream.read(&mut head[len..]).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        len += n;
    }
    let line = String::from_utf8_lossy(&head[..len]);
    let status = line
        .strip_prefix("HTTP/1.")
        .and_then(|s| s.get(2..5))
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| format!("not an HTTP response: {line:?}"))?;
    Ok(status)
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn clap_tree_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn admin_flags_parse() {
        let cli = Cli::try_parse_from([
            "wikanvas-server",
            "admin",
            "create-owner",
            "--email",
            "a@wikanvas.invalid",
            "--name",
            "A",
            "--workspace",
            "W",
            "--generate-password",
            "--json",
        ])
        .unwrap();
        let Command::Admin(AdminArgs {
            json: true,
            command: AdminCommand::CreateOwner(args),
        }) = cli.command
        else {
            panic!("{cli:?}");
        };
        assert!(args.generate_password && !args.password_stdin && !args.no_seed);
        let both = Cli::try_parse_from([
            "wikanvas-server",
            "admin",
            "create-owner",
            "--email",
            "a@x",
            "--name",
            "A",
            "--workspace",
            "W",
            "--generate-password",
            "--password-stdin",
        ]);
        assert_eq!(both.unwrap_err().exit_code(), 2);
        let neither = Cli::try_parse_from([
            "wikanvas-server",
            "admin",
            "create-owner",
            "--email",
            "a@x",
            "--name",
            "A",
            "--workspace",
            "W",
        ]);
        assert_eq!(neither.unwrap_err().exit_code(), 2);
        let token = Cli::try_parse_from([
            "wikanvas-server",
            "admin",
            "--json",
            "create-token",
            "--name",
            "t",
            "--scope",
            "write",
        ])
        .unwrap();
        let Command::Admin(AdminArgs {
            command: AdminCommand::CreateToken(t),
            ..
        }) = token.command
        else {
            panic!();
        };
        assert_eq!((t.scope.as_str(), t.expires.as_str()), ("write", "90d"));
        assert!(
            Cli::try_parse_from([
                "wikanvas-server",
                "admin",
                "create-token",
                "--name",
                "t",
                "--scope",
                "root"
            ])
            .is_err()
        );
        assert!(Cli::try_parse_from(["wikanvas-server", "admin", "revoke-token", "--all"]).is_err());
        assert!(
            Cli::try_parse_from(["wikanvas-server", "admin", "revoke-token", "--all", "--yes"])
                .is_ok()
        );
        assert!(Cli::try_parse_from(["wikanvas-server", "admin", "revoke-token"]).is_err());
    }

    #[test]
    fn probe_refuses_https_and_reports_refused() {
        assert!(probe("https://localhost/healthz", Duration::from_millis(100)).is_err());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert!(
            probe(
                &format!("http://127.0.0.1:{port}"),
                Duration::from_millis(500)
            )
            .is_err()
        );
    }

    #[test]
    fn probe_reads_the_status_line() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 512];
            let n = conn.read(&mut buf).unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            conn.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
                .unwrap();
            request
        });
        assert_eq!(
            probe(&format!("http://127.0.0.1:{port}"), Duration::from_secs(2)),
            Ok(200)
        );
        let request = server.join().unwrap();
        assert!(
            request.starts_with("GET /healthz HTTP/1.1\r\n"),
            "{request}"
        );
    }
}
