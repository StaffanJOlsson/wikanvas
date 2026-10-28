//! Admin command tests (owner: W2)
//!
//! `quire-server admin …` runs as the real binary on a temporary data directory, also while an in-process server
//! has the same database open (WAL, a second process).

use quire_api::{ErrorCode, Me, Scope};
use quire_server::AppState;
use quire_server::config::Config;
use serde_json::json;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use crate::common::{OWNER_EMAIL, TestApp, admin, assert_error, stdout_json};

const OWNER: [&str; 8] = [
    "create-owner",
    "--email",
    "a@quire.invalid",
    "--name",
    "A",
    "--workspace",
    "W",
    "--generate-password",
];

fn ok(out: &std::process::Output) {
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout {:?}, stderr {:?}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn error_line(out: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(out.stderr.trim_ascii()).expect("a JSON error line on stderr")
}

async fn collections(dir: &std::path::Path) -> i64 {
    let db =
        quire_store::Db::open(dir.join("quire.db"), quire_store::DbOptions::default()).unwrap();
    db.read(|c| Ok(c.query_row("SELECT count(*) FROM collections", [], |r| r.get(0))?))
        .await
        .unwrap()
}

#[tokio::test]
async fn create_owner_once_then_exit_8() {
    let dir = tempfile::tempdir().unwrap();
    let mut args = OWNER.to_vec();
    args.push("--json");
    let out = admin(dir.path(), &args, None);
    ok(&out);
    let created = stdout_json(&out);
    assert_eq!(created["user"]["email"], "a@quire.invalid");
    assert_eq!(created["user"]["role"], "owner");
    assert_eq!(created["workspace"]["name"], "W");
    assert_eq!(created["generated_password"].as_str().unwrap().len(), 24);
    assert_eq!(collections(dir.path()).await, 1, "seeded Getting started");

    let again = admin(dir.path(), &args, None);
    assert_eq!(again.status.code(), Some(8));
    let err = error_line(&again);
    assert_eq!(err["error"]["code"], "already_setup");
    assert_eq!(err["exit_code"], 8);
    assert_eq!(err["request_id"], serde_json::Value::Null);
    assert!(again.stdout.is_empty());

    let unseeded = tempfile::tempdir().unwrap();
    let mut args = OWNER.to_vec();
    args.push("--no-seed");
    ok(&admin(unseeded.path(), &args, None));
    assert_eq!(collections(unseeded.path()).await, 0);
}

#[test]
fn create_owner_password_stdin_and_validation() {
    let dir = tempfile::tempdir().unwrap();
    let args = [
        "create-owner",
        "--email",
        "a@quire.invalid",
        "--name",
        "A",
        "--workspace",
        "W",
        "--password-stdin",
    ];
    let short = admin(dir.path(), &args, Some("short\n"));
    assert_eq!(
        short.status.code(),
        Some(10),
        "{}",
        String::from_utf8_lossy(&short.stderr)
    );
    ok(&admin(dir.path(), &args, Some("a long enough password\n")));
    let usage = admin(dir.path(), &["create-owner", "--email", "a@x"], None);
    assert_eq!(usage.status.code(), Some(2));
}

#[test]
fn create_token_token_out_0600() {
    let dir = tempfile::tempdir().unwrap();
    let no_owner = admin(
        dir.path(),
        &["create-token", "--name", "t", "--scope", "read", "--json"],
        None,
    );
    assert_eq!(
        no_owner.status.code(),
        Some(8),
        "{}",
        String::from_utf8_lossy(&no_owner.stderr)
    );
    ok(&admin(dir.path(), &OWNER, None));

    let out = admin(
        dir.path(),
        &[
            "create-token",
            "--name",
            "t",
            "--scope",
            "write",
            "--expires",
            "1d",
            "--json",
        ],
        None,
    );
    ok(&out);
    let created = stdout_json(&out);
    assert!(quire_api::token::check(created["token"].as_str().unwrap()).is_ok());
    assert_eq!(created["api_token"]["created_via"], "admin");
    assert_eq!(created["api_token"]["scopes"], json!(["read", "write"]));

    let file = dir.path().join("token");
    std::fs::write(
        &file,
        "old contents that are longer than a token, to be truncated",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    let out = admin(
        dir.path(),
        &[
            "create-token",
            "--name",
            "t2",
            "--scope",
            "read",
            "--token-out",
            file.to_str().unwrap(),
            "--json",
        ],
        None,
    );
    ok(&out);
    let written = stdout_json(&out);
    assert!(
        written.get("token").is_none(),
        "the secret goes to the file only"
    );
    assert_eq!(written["token_file"], file.to_str().unwrap());
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let secret = std::fs::read_to_string(&file).unwrap();
    assert!(!String::from_utf8_lossy(&out.stdout).contains(secret.trim_end()));
    assert!(secret.ends_with('\n'));
    assert!(quire_api::token::check(secret.trim_end()).is_ok());

    let unknown = admin(
        dir.path(),
        &[
            "create-token",
            "--name",
            "t",
            "--scope",
            "read",
            "--email",
            "b@quire.invalid",
        ],
        None,
    );
    assert_eq!(unknown.status.code(), Some(7));
    let bad = admin(
        dir.path(),
        &[
            "create-token",
            "--name",
            "t",
            "--scope",
            "read",
            "--expires",
            "1w",
        ],
        None,
    );
    assert_eq!(bad.status.code(), Some(10));
}

/// `GET path` over real HTTP; returns the status and the body.
async fn http_get(addr: std::net::SocketAddr, path: &str, token: &str) -> (u16, String) {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n",
        addr.port()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let text = String::from_utf8_lossy(&response).to_string();
    let status = text[9..12].parse().unwrap();
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_owned())
        .unwrap_or_default();
    (status, body)
}

#[tokio::test]
async fn create_token_prints_once_and_authenticates_over_http() {
    let dir = tempfile::tempdir().unwrap();
    ok(&admin(dir.path(), &OWNER, None));
    let out = admin(
        dir.path(),
        &["create-token", "--name", "ci", "--scope", "read"],
        None,
    );
    ok(&out);
    let stdout = String::from_utf8(out.stdout.clone()).unwrap();
    let token = stdout.trim_end().to_owned();
    assert_eq!(stdout, format!("{token}\n"), "only the token on stdout");
    assert!(quire_api::token::check(&token).is_ok());
    assert!(!String::from_utf8_lossy(&out.stderr).contains(&token));

    let state = AppState::open(Config::for_tests(dir.path())).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(quire_server::app::serve(state, listener, async {
        let _ = stopped.await;
    }));
    let (status, body) = http_get(addr, "/api/v1/auth/me", &token).await;
    assert_eq!(status, 200, "{body}");
    let me: Me = serde_json::from_str(&body).unwrap();
    assert_eq!(me.auth.token.unwrap().name, "ci");
    let (status, _) = http_get(addr, "/api/v1/auth/me", &quire_api::token::encode(&[1; 32])).await;
    assert_eq!(status, 401);
    // The token is listed, never with its secret.
    let out = admin(dir.path(), &["--json", "list-tokens"], None);
    ok(&out);
    let listed = stdout_json(&out);
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert!(!String::from_utf8_lossy(&out.stdout).contains(&token));
    let _ = stop.send(());
    server.await.unwrap().unwrap();
}

#[tokio::test]
async fn runs_while_server_is_up() {
    let app = TestApp::with_owner().await;
    app.server.get("/api/v1/auth/me").await.assert_status_ok();
    let out = admin(
        app.dir.path(),
        &[
            "--json",
            "create-token",
            "--name",
            "agent",
            "--scope",
            "write",
            "--expires",
            "1d",
        ],
        None,
    );
    ok(&out);
    let created = stdout_json(&out);
    let token = created["token"].as_str().unwrap().to_owned();
    let id = created["api_token"]["id"].as_str().unwrap().to_owned();
    app.token_client(&token)
        .get("/api/v1/auth/me")
        .await
        .assert_status_ok();
    // The server's own writes go on meanwhile.
    app.server
        .post("/api/v1/collections")
        .json(&json!({"name": "Notes"}))
        .await
        .assert_status(axum::http::StatusCode::CREATED);
    let out = admin(app.dir.path(), &["--json", "revoke-token", &id], None);
    ok(&out);
    assert_eq!(stdout_json(&out)["revoked"], 1);
    assert_error(
        &app.token_client(&token).get("/api/v1/auth/me").await,
        401,
        ErrorCode::InvalidToken,
    );
    let again = admin(app.dir.path(), &["revoke-token", &id], None);
    assert_eq!(again.status.code(), Some(7));
    // Backup and check run against the live database.
    let out = admin(app.dir.path(), &["--json", "backup"], None);
    ok(&out);
    let backup = stdout_json(&out);
    let path = std::path::PathBuf::from(backup["path"].as_str().unwrap());
    assert!(path.starts_with(app.dir.path().join("backups")) && path.exists());
    assert_eq!(backup["size"], std::fs::metadata(&path).unwrap().len());
    let out_file = app.dir.path().join("copy.db");
    let out = admin(
        app.dir.path(),
        &["--json", "backup", "--out", out_file.to_str().unwrap()],
        None,
    );
    ok(&out);
    assert!(out_file.exists());
    let exists = admin(
        app.dir.path(),
        &["backup", "--out", out_file.to_str().unwrap()],
        None,
    );
    assert_eq!(exists.status.code(), Some(10));
    let out = admin(app.dir.path(), &["--json", "check"], None);
    ok(&out);
    let report = stdout_json(&out);
    assert_eq!(report["ok"], true);
    assert_eq!(report["quick_check"], json!(["ok"]));
    assert_eq!(report["fts"], "ok");
    // --all revokes every token.
    let a = app.mint_token(app.owner(), Scope::Read).await;
    let b = app.mint_token(app.owner(), Scope::Admin).await;
    let all = admin(app.dir.path(), &["revoke-token", "--all"], None);
    assert_eq!(all.status.code(), Some(2), "--all needs --yes");
    let out = admin(
        app.dir.path(),
        &["--json", "revoke-token", "--all", "--yes"],
        None,
    );
    ok(&out);
    assert_eq!(stdout_json(&out)["revoked"], 2);
    for t in [a, b] {
        assert_error(
            &app.token_client(&t).get("/api/v1/auth/me").await,
            401,
            ErrorCode::InvalidToken,
        );
    }
}

#[tokio::test]
async fn check_reports_missing_blob_files() {
    let app = TestApp::with_owner().await;
    let res = app
        .server
        .post("/api/v1/attachments")
        .bytes("x".into())
        .content_type("application/octet-stream")
        .await;
    let a: quire_api::Attachment = res.json();
    let sha = quire_store::Sha256::parse(&a.sha256).unwrap();
    std::fs::remove_file(app.state.blobs.path(&sha)).unwrap();
    let out = admin(app.dir.path(), &["--json", "check"], None);
    assert_eq!(out.status.code(), Some(1));
    let report = stdout_json(&out);
    assert_eq!(report["ok"], false);
    assert_eq!(report["missing_blobs"], json!([a.sha256]));
}

#[tokio::test]
async fn create_owner_while_serving_clears_setup_required() {
    let app = TestApp::new().await;
    let client = app.anon_client();
    assert_error(
        &client.get("/api/v1/auth/me").await,
        409,
        ErrorCode::SetupRequired,
    );
    ok(&admin(app.dir.path(), &OWNER, None));
    assert_error(
        &client.get("/api/v1/auth/me").await,
        401,
        ErrorCode::Unauthenticated,
    );
    let status: quire_api::SetupStatus = client.get("/api/v1/setup").await.json();
    assert!(!status.needs_setup);
    // The setup code no longer works.
    let res = client
        .post("/api/v1/setup")
        .json(&json!({
            "code": app.state.setup_code.to_string(),
            "workspace_name": "X",
            "name": "X",
            "email": "x@quire.invalid",
            "password": "a long enough password"
        }))
        .await;
    assert_error(&res, 409, ErrorCode::AlreadySetup);
}

#[tokio::test]
async fn reset_password_revoke_tokens() {
    let app = TestApp::with_owner().await;
    let token = app.mint_token(app.owner(), Scope::Read).await;
    let out = admin(
        app.dir.path(),
        &["--json", "reset-password", "--email", OWNER_EMAIL],
        None,
    );
    ok(&out);
    assert_eq!(stdout_json(&out)["tokens_revoked"], 0);
    app.token_client(&token)
        .get("/api/v1/auth/me")
        .await
        .assert_status_ok();
    let out = admin(
        app.dir.path(),
        &[
            "--json",
            "reset-password",
            "--email",
            OWNER_EMAIL,
            "--revoke-tokens",
        ],
        None,
    );
    ok(&out);
    let reset = stdout_json(&out);
    assert_eq!(reset["tokens_revoked"], 1);
    assert_eq!(reset["user"]["email"], OWNER_EMAIL);
    assert_error(
        &app.token_client(&token).get("/api/v1/auth/me").await,
        401,
        ErrorCode::InvalidToken,
    );
    assert_error(
        &app.server.get("/api/v1/auth/me").await,
        401,
        ErrorCode::Unauthenticated,
    );
    let text = admin(
        app.dir.path(),
        &["reset-password", "--email", OWNER_EMAIL],
        None,
    );
    ok(&text);
    assert!(String::from_utf8_lossy(&text.stdout).contains("Temporary password"));
}
