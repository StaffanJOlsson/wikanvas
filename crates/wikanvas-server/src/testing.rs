//! TestServer: an in-process server on 127.0.0.1:0 for the CLI tests, feature `testing` (owner: W2)
//!
//! [`TestServer::start`] binds a free loopback port, runs [`crate::app`] on its own tokio runtime thread with a
//! temporary data directory and cheap argon2 parameters, creates the owner ([`OWNER_EMAIL`], [`OWNER_PASSWORD`];
//! without the "Getting started" seed, so the workspace starts empty) and one token per scope (`created_via` admin,
//! never expiring), and returns once the server accepts connections. The server is the real one: the Host allowlist,
//! CSRF, the limiters and the peer address all apply as in `serve`, but no background job runs. Dropping it stops the
//! server and deletes the data directory.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread::JoinHandle;

use tokio::sync::oneshot;
use wikanvas_api::Scope;
use wikanvas_api::tokens::NewToken;

use crate::admin::{self, NewOwner};
use crate::config::{Config, Origin, Scheme};
use crate::state::{self, AppState};

/// The owner's email.
pub const OWNER_EMAIL: &str = "owner@wikanvas.invalid";
/// The owner's password.
pub const OWNER_PASSWORD: &str = "test-owner-password";
/// The owner's name.
pub const OWNER_NAME: &str = "Owner";
/// The workspace name.
pub const WORKSPACE_NAME: &str = "Test workspace";

/// A running in-process server. Stops on drop.
pub struct TestServer {
    /// `http://127.0.0.1:<port>`, no trailing slash.
    pub url: String,
    /// A `read` token of the owner.
    pub read_token: String,
    /// A `write` token of the owner.
    pub write_token: String,
    /// An `admin` token of the owner.
    pub admin_token: String,
    /// The server's `WIKANVAS_DATA_DIR` (deleted on drop).
    pub data_dir: PathBuf,
    /// The owner's user id.
    pub owner_id: String,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
    _dir: tempfile::TempDir,
}

struct Ready {
    owner_id: String,
    read_token: String,
    write_token: String,
    admin_token: String,
}

impl TestServer {
    /// Starts a server (see the module docs). Panics when it cannot.
    pub fn start() -> TestServer {
        TestServer::start_with(|_| {})
    }

    /// Like [`start`](Self::start), with `configure` applied to the configuration first (the data directory,
    /// listen address and public URL are set by the server).
    pub fn start_with(configure: impl FnOnce(&mut Config)) -> TestServer {
        let dir = tempfile::Builder::new()
            .prefix("wikanvas-test-server-")
            .tempdir()
            .expect("a temporary data directory");
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind 127.0.0.1:0 for the test server");
        let addr: SocketAddr = listener.local_addr().expect("the bound address");
        listener
            .set_nonblocking(true)
            .expect("non-blocking listener");
        let mut config = Config::for_tests(dir.path());
        configure(&mut config);
        config.data_dir = dir.path().to_path_buf();
        config.listen = addr;
        config.public_url = Origin {
            scheme: Scheme::Http,
            host: "127.0.0.1".into(),
            port: Some(addr.port()),
        };
        config.test_argon2 = true;

        let (ready_tx, ready_rx) = mpsc::channel::<Result<Ready, String>>();
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("wikanvas-test-server".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = ready_tx.send(Err(format!("tokio runtime: {e}")));
                        return;
                    }
                };
                runtime.block_on(async move {
                    let prepared = async {
                        let state = AppState::open(config).await.map_err(|e| e.to_string())?;
                        let tokens = provision(&state).await.map_err(|e| e.to_string())?;
                        let listener = tokio::net::TcpListener::from_std(listener)
                            .map_err(|e| e.to_string())?;
                        Ok::<_, String>((state, tokens, listener))
                    }
                    .await;
                    let (state, tokens, listener) = match prepared {
                        Ok(p) => p,
                        Err(e) => {
                            let _ = ready_tx.send(Err(e));
                            return;
                        }
                    };
                    let _ = ready_tx.send(Ok(tokens));
                    let shutdown = async {
                        let _ = shutdown_rx.await;
                    };
                    if let Err(e) = crate::app::serve(state, listener, shutdown).await {
                        eprintln!("wikanvas test server: {e}");
                    }
                });
            })
            .expect("spawn the test server thread");

        let ready = match ready_rx.recv() {
            Ok(Ok(ready)) => ready,
            Ok(Err(e)) => panic!("the test server did not start: {e}"),
            Err(_) => panic!("the test server thread ended before it was ready"),
        };
        TestServer {
            url: format!("http://{addr}"),
            read_token: ready.read_token,
            write_token: ready.write_token,
            admin_token: ready.admin_token,
            data_dir: dir.path().to_path_buf(),
            owner_id: ready.owner_id,
            shutdown: Some(shutdown_tx),
            thread: Some(thread),
            _dir: dir,
        }
    }

    /// The token of `scope`.
    pub fn token(&self, scope: Scope) -> &str {
        match scope {
            Scope::Read => &self.read_token,
            Scope::Write => &self.write_token,
            Scope::Admin => &self.admin_token,
        }
    }
}

/// Creates the owner and one token per scope.
async fn provision(state: &AppState) -> Result<Ready, crate::error::ApiError> {
    let password_hash = state::hash_password(true, OWNER_PASSWORD.into()).await?;
    let user = admin::create_owner(
        state,
        NewOwner {
            email: OWNER_EMAIL.into(),
            name: OWNER_NAME.into(),
            workspace: WORKSPACE_NAME.into(),
            password_hash,
            must_change_password: false,
            seed: false,
        },
        state.now(),
    )
    .await?;
    let mut tokens = Vec::new();
    for scope in [Scope::Read, Scope::Write, Scope::Admin] {
        let new = NewToken {
            name: format!("test-{scope}"),
            scopes: vec![scope],
            expires_in_days: None,
            password: None,
        };
        let created = admin::create_token(state, user.id.to_string(), new, state.now()).await?;
        tokens.push(created.token);
    }
    let [read_token, write_token, admin_token]: [String; 3] =
        tokens.try_into().expect("three tokens");
    Ok(Ready {
        owner_id: user.id.to_string(),
        read_token,
        write_token,
        admin_token,
    })
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_serves_and_stops() {
        let server = TestServer::start();
        assert!(server.url.starts_with("http://127.0.0.1:"));
        for token in [&server.read_token, &server.write_token, &server.admin_token] {
            assert!(wikanvas_api::token::check(token).is_ok(), "{token}");
        }
        assert!(server.data_dir.join("wikanvas.db").exists());
        let url = format!("{}/healthz", server.url);
        assert_eq!(
            crate::cli::probe(&url, std::time::Duration::from_secs(5)),
            Ok(200)
        );
        let dir = server.data_dir.clone();
        drop(server);
        assert!(crate::cli::probe(&url, std::time::Duration::from_millis(500)).is_err());
        assert!(!dir.exists());
    }
}
