//! Shared test setup: TestApp::new, with_owner and with_token (owner: W2)
//!
//! - [`TestApp::new`]: `quire_server::app()` on a fresh temporary data directory with cheap argon2, no owner;
//! - [`TestApp::with_owner`]: plus the owner and a session inserted with plain SQL; `server` sends the session
//!   cookie, `Host: localhost` and `Sec-Fetch-Site: same-origin` with every request;
//! - [`TestApp::with_token`]: plus the owner and a token of one scope; `server` sends `Authorization: Bearer` and
//!   no cookie.
//!
//! [`TestApp::client`] builds another client on the same state (no default headers), so one test can mix a cookie
//! client, token clients and anonymous requests. Every client's router injects the peer address from an
//! `x-test-peer: <ip:port>` request header as `ConnectInfo` (the socket peer of §8), so tests can play a direct client
//! or the trusted proxy; without the header a request has no peer, like axum-test's mock transport.

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::{ConnectInfo, Request};
use axum::middleware::Next;
use axum::response::Response;
use axum_test::TestServer;
use base64::Engine as _;
use quire_api::tokens::NewToken;
use quire_api::{ErrorBody, ErrorCode, Scope};
use quire_server::AppState;
use quire_server::config::Config;
use quire_server::static_files::AssetSource;
use quire_store::tokens::CreatedVia;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

/// The header the test routers read the peer address from.
pub const PEER: &str = "x-test-peer";

async fn inject_peer(mut req: Request, next: Next) -> Response {
    let peer = req
        .headers()
        .get(PEER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<SocketAddr>().ok());
    if let Some(peer) = peer {
        req.extensions_mut().insert(ConnectInfo(peer));
    }
    next.run(req).await
}

/// `quire_server::app(state)` with the peer injection of the module docs.
pub fn router(state: &AppState) -> Router {
    quire_server::app(state.clone()).layer(axum::middleware::from_fn(inject_peer))
}

/// The owner's email in every TestApp.
pub const OWNER_EMAIL: &str = "owner@quire.invalid";
/// The owner's password in every TestApp.
pub const OWNER_PASSWORD: &str = "test-owner-password";

/// A test application.
pub struct TestApp {
    /// The default client (see the module docs for its headers).
    pub server: TestServer,
    pub state: AppState,
    /// The owner's id, once there is one.
    pub owner_id: Option<String>,
    /// The session cookie's value (`with_owner`).
    pub session: Option<String>,
    /// The token (`with_token`).
    pub token: Option<String>,
    pub dir: TempDir,
}

impl TestApp {
    /// No owner, no credentials.
    pub async fn new() -> TestApp {
        TestApp::with_config(|_| {}).await
    }

    /// Like [`new`](Self::new), with `configure` applied to the test configuration.
    pub async fn with_config(configure: impl FnOnce(&mut Config)) -> TestApp {
        let dir = tempfile::Builder::new()
            .prefix("quire-api-test-")
            .tempdir()
            .expect("temp dir");
        let mut config = Config::for_tests(dir.path());
        configure(&mut config);
        let state = AppState::open(config).await.expect("the app state opens");
        let state = state
            .with_assets(Arc::new(quire_server::static_files::MemoryAssets::new()))
            .expect("no manifest");
        let server = TestServer::new(router(&state));
        TestApp {
            server,
            state,
            owner_id: None,
            session: None,
            token: None,
            dir,
        }
    }

    /// Like [`with_config`](Self::with_config), from `QUIRE_*` variables as `serve` reads them (the data directory
    /// and cheap argon2 are the test's).
    pub async fn from_vars(vars: &[(&str, &str)]) -> TestApp {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let parsed =
            Config::from_lookup(|k| vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()))
                .expect("the variables parse");
        TestApp::with_config(move |c| {
            let (data_dir, argon2) = (c.data_dir.clone(), c.test_argon2);
            *c = parsed;
            c.data_dir = data_dir;
            c.test_argon2 = argon2;
        })
        .await
    }

    /// No owner, a fake clock (`state.clock.advance`).
    pub async fn with_fake_clock() -> TestApp {
        TestApp::with_config(|c| c.fake_clock = true).await
    }

    /// The same state with `source` as the static files; `server` becomes an anonymous client of it.
    pub fn set_assets(&mut self, source: impl AssetSource) {
        self.state = self
            .state
            .clone()
            .with_assets(Arc::new(source))
            .expect("the assets' manifest is valid");
        self.server = self.client();
    }

    /// Makes this app's default client the owner's cookie client (an owner and a session are inserted).
    pub async fn sign_in_owner(&mut self) {
        let owner = self.insert_owner().await;
        let session = self.insert_session(&owner).await;
        self.server = self.cookie_client(&session);
        self.owner_id = Some(owner);
        self.session = Some(session);
    }

    /// An owner and a session; `server` sends the cookie, an allowed `Host` and `Sec-Fetch-Site: same-origin`.
    pub async fn with_owner() -> TestApp {
        let mut app = TestApp::new().await;
        let owner = app.insert_owner().await;
        let session = app.insert_session(&owner).await;
        app.server = app.cookie_client(&session);
        app.owner_id = Some(owner);
        app.session = Some(session);
        app
    }

    /// An owner and a token of `scope`; `server` sends `Authorization: Bearer` and no cookie.
    pub async fn with_token(scope: Scope) -> TestApp {
        let mut app = TestApp::new().await;
        let owner = app.insert_owner().await;
        let token = app.mint_token(&owner, scope).await;
        app.server = app.token_client(&token);
        app.owner_id = Some(owner);
        app.token = Some(token);
        app
    }

    /// The owner's id (panics before there is one).
    pub fn owner(&self) -> &str {
        self.owner_id.as_deref().expect("an owner")
    }

    /// A client on the same state without default headers.
    pub fn client(&self) -> TestServer {
        TestServer::new(router(&self.state))
    }

    /// A client without credentials that passes the CSRF check (`Host: localhost`, `Sec-Fetch-Site: same-origin`).
    pub fn anon_client(&self) -> TestServer {
        let mut server = self.client();
        server.add_header("host", "localhost");
        server.add_header("sec-fetch-site", "same-origin");
        server
    }

    /// A client that sends the session cookie, `Host: localhost` and `Sec-Fetch-Site: same-origin`.
    pub fn cookie_client(&self, session: &str) -> TestServer {
        let mut server = self.client();
        server.add_header("cookie", format!("quire_session={session}"));
        server.add_header("host", "localhost");
        server.add_header("sec-fetch-site", "same-origin");
        server
    }

    /// A client that sends `Authorization: Bearer <token>`.
    pub fn token_client(&self, token: &str) -> TestServer {
        let mut server = self.client();
        server.add_header("authorization", format!("Bearer {token}"));
        server
    }

    /// Inserts the owner (and the workspace name) with plain SQL; returns the owner's id.
    pub async fn insert_owner(&self) -> String {
        let id = uuid::Uuid::now_v7().to_string();
        let hash = quire_server::state::hash_password(true, OWNER_PASSWORD.into())
            .await
            .expect("hash");
        let now = quire_server::state::now_ms();
        let owner = id.clone();
        self.state
            .db
            .write(move |tx| {
                tx.execute(
                    "INSERT INTO users (id, email, name, password_hash, role, created_at, updated_at)
                     VALUES (?1, ?2, 'Owner', ?3, 'owner', ?4, ?4)",
                    rusqlite::params![owner, OWNER_EMAIL, hash, now],
                )?;
                tx.execute(
                    "INSERT OR REPLACE INTO settings (key, value) VALUES ('workspace.name', '\"Test workspace\"')",
                    [],
                )?;
                Ok(())
            })
            .await
            .expect("insert the owner");
        id
    }

    /// Signs in through `POST /auth/login` from `peer` (or no peer) and returns the response.
    pub async fn login(
        &self,
        password: &str,
        remember: bool,
        peer: Option<&str>,
    ) -> axum_test::TestResponse {
        let mut req = self.anon_client().post("/api/v1/auth/login").json(
            &serde_json::json!({"email": OWNER_EMAIL, "password": password, "remember": remember}),
        );
        if let Some(peer) = peer {
            req = req.add_header(PEER, peer.to_owned());
        }
        req.await
    }

    /// Inserts a session of `user_id` valid for a day; returns the cookie value (base64url of 32 random bytes).
    pub async fn insert_session(&self, user_id: &str) -> String {
        let mut raw = [0u8; 32];
        getrandom::fill(&mut raw).expect("random");
        let cookie = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw);
        let hash: [u8; 32] = Sha256::digest(raw).into();
        let now = quire_server::state::now_ms();
        let (id, user) = (uuid::Uuid::now_v7().to_string(), user_id.to_owned());
        self.state
            .db
            .write(move |tx| {
                tx.execute(
                    "INSERT INTO sessions (id, token_hash, user_id, persistent, created_at, last_seen_at, expires_at)
                     VALUES (?1, ?2, ?3, 0, ?4, ?4, ?5)",
                    rusqlite::params![id, hash.as_slice(), user, now, now + 86_400_000],
                )?;
                Ok(())
            })
            .await
            .expect("insert a session");
        cookie
    }

    /// Creates a token of `scope` for `user_id` through the store; returns its secret.
    pub async fn mint_token(&self, user_id: &str, scope: Scope) -> String {
        let user = user_id.to_owned();
        let now = quire_server::state::now_ms();
        let new = NewToken {
            name: format!("test-{scope}"),
            scopes: vec![scope],
            expires_in_days: None,
            password: None,
        };
        self.state
            .db
            .write(move |tx| {
                quire_store::tokens::create(tx, &user, &new, CreatedVia::Admin, None, now)
            })
            .await
            .expect("create a token")
            .token
    }

    /// Runs `sql` with `params` on the writer (test fixtures: expire a token, revoke a session, …).
    pub async fn exec(&self, sql: &'static str, params: Vec<String>) {
        self.state
            .db
            .write(move |tx| {
                tx.execute(sql, rusqlite::params_from_iter(params.iter()))?;
                Ok(())
            })
            .await
            .expect("exec");
    }
}

/// Every `Set-Cookie` of a response.
pub fn set_cookies(res: &axum_test::TestResponse) -> Vec<String> {
    res.headers()
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap().to_owned())
        .collect()
}

/// The value of cookie `name` in a `Set-Cookie` of the response.
pub fn cookie_from(res: &axum_test::TestResponse, name: &str) -> Option<String> {
    set_cookies(res).iter().find_map(|c| {
        let (pair, _) = c.split_once(';').unwrap_or((c, ""));
        let (k, v) = pair.split_once('=')?;
        (k == name).then(|| v.to_owned())
    })
}

/// Runs `quire-server admin <args>` on `dir` (cheap argon2), with `stdin`.
pub fn admin(dir: &std::path::Path, args: &[&str], stdin: Option<&str>) -> std::process::Output {
    use std::io::Write as _;
    use std::process::{Command, Stdio};
    let mut child = Command::new(env!("CARGO_BIN_EXE_quire-server"))
        .arg("admin")
        .args(args)
        .env_clear()
        .env("QUIRE_DATA_DIR", dir)
        .env("QUIRE_TEST_ARGON2", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run quire-server admin");
    {
        let mut pipe = child.stdin.take().unwrap();
        if let Some(input) = stdin {
            pipe.write_all(input.as_bytes()).unwrap();
        }
    }
    child.wait_with_output().unwrap()
}

/// stdout of a command as JSON (panics with both streams otherwise).
pub fn stdout_json(out: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "{e}: status {:?}, stdout {:?}, stderr {:?}",
            out.status.code(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

/// Asserts that `res` is the error envelope with `status` and `code`, with `X-Request-Id` equal to `request_id`;
/// returns the body.
pub fn assert_error(res: &axum_test::TestResponse, status: u16, code: ErrorCode) -> ErrorBody {
    assert_eq!(res.status_code().as_u16(), status, "body: {}", res.text());
    assert_eq!(
        res.content_type(),
        "application/json",
        "body: {}",
        res.text()
    );
    let body: ErrorBody = res.json();
    assert_eq!(body.error.code, code, "body: {}", res.text());
    let id = res.header("x-request-id");
    assert_eq!(
        body.request_id.as_deref(),
        Some(id.to_str().unwrap()),
        "request_id must equal X-Request-Id"
    );
    body
}
