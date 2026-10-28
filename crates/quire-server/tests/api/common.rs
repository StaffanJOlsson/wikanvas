//! Shared test setup: TestApp::new, with_owner and with_token (owner: W2)
//!
//! - [`TestApp::new`]: `quire_server::app()` on a fresh temporary data directory with cheap argon2, no owner;
//! - [`TestApp::with_owner`]: plus the owner and a session inserted with plain SQL; `server` sends the session
//!   cookie, `Host: localhost` and `Sec-Fetch-Site: same-origin` with every request;
//! - [`TestApp::with_token`]: plus the owner and a token of one scope; `server` sends `Authorization: Bearer` and
//!   no cookie.
//!
//! [`TestApp::client`] builds another client on the same state (no default headers), so one test can mix a cookie
//! client, token clients and anonymous requests.

#![allow(dead_code)]

use axum_test::TestServer;
use base64::Engine as _;
use quire_api::tokens::NewToken;
use quire_api::{ErrorBody, ErrorCode, Scope};
use quire_server::AppState;
use quire_server::config::Config;
use quire_store::tokens::CreatedVia;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

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
        let server = TestServer::new(quire_server::app(state.clone()));
        TestApp {
            server,
            state,
            owner_id: None,
            session: None,
            token: None,
            dir,
        }
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
        TestServer::new(quire_server::app(self.state.clone()))
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
