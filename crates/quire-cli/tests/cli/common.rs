//! Shared helpers of the CLI tests (owner: W8)
//!
//! [`Sandbox`] is a temporary directory holding `HOME` and the `QUIRE_CONFIG` file, and builds an assert_cmd
//! [`Command`] for the `quire` binary that inherits no `QUIRE_*` variable and no config or CA location from the
//! environment the tests run in. [`Harness`] adds a running `quire_server::testing::TestServer` and points
//! `QUIRE_URL` and `QUIRE_TOKEN` at it.

use std::ffi::OsStr;
use std::fs::DirBuilder;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use quire_api::Scope;
use quire_server::testing::TestServer;
use tempfile::TempDir;

/// Variables outside `QUIRE_*` that the CLI reads for its config directory, its CA bundle or logging; a test sets
/// them explicitly when it needs them.
const SCRUBBED: &[&str] = &[
    "XDG_CONFIG_HOME",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "RUST_LOG",
];

/// A temporary `HOME` and config file for one `quire` invocation or more. Deleted on drop.
pub struct Sandbox {
    dir: TempDir,
}

impl Sandbox {
    /// Creates `<tmp>/home` (mode 0700). The config file's directory is left to the CLI, which creates it 0700.
    pub fn new() -> Sandbox {
        let dir = tempfile::Builder::new()
            .prefix("quire-cli-test-")
            .tempdir()
            .expect("a temporary directory");
        DirBuilder::new()
            .mode(0o700)
            .create(dir.path().join("home"))
            .expect("the temporary HOME");
        Sandbox { dir }
    }

    /// The temporary directory; also the working directory of every command.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// `HOME`: `<tmp>/home`.
    pub fn home(&self) -> PathBuf {
        self.path().join("home")
    }

    /// `QUIRE_CONFIG`: `<tmp>/config/config.toml` (not created).
    pub fn config_path(&self) -> PathBuf {
        self.path().join("config").join("config.toml")
    }

    /// `quire` with `HOME` and `QUIRE_CONFIG` in the sandbox, run in the sandbox, and with no URL or token.
    pub fn quire(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_quire"));
        for (name, _) in std::env::vars_os() {
            if name.as_encoded_bytes().starts_with(b"QUIRE_") {
                cmd.env_remove(&name);
            }
        }
        for name in SCRUBBED {
            cmd.env_remove(OsStr::new(name));
        }
        cmd.env("HOME", self.home())
            .env("QUIRE_CONFIG", self.config_path())
            .current_dir(self.path());
        cmd
    }
}

/// A [`TestServer`] and a [`Sandbox`] whose commands talk to it.
pub struct Harness {
    /// Stops on drop, before the sandbox is deleted.
    pub server: TestServer,
    pub sandbox: Sandbox,
}

impl Harness {
    /// Starts a server with an owner and one token per scope.
    pub fn start() -> Harness {
        Harness {
            server: TestServer::start(),
            sandbox: Sandbox::new(),
        }
    }

    /// `quire` as the owner's write token: [`Sandbox::quire`] plus `QUIRE_URL` and `QUIRE_TOKEN`.
    pub fn quire(&self) -> Command {
        self.quire_as(Scope::Write)
    }

    /// `quire` as the owner's token of `scope`.
    pub fn quire_as(&self, scope: Scope) -> Command {
        let mut cmd = self.sandbox.quire();
        cmd.env("QUIRE_URL", &self.server.url)
            .env("QUIRE_TOKEN", self.server.token(scope));
        cmd
    }
}
