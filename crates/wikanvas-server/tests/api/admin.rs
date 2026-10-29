//! Admin command tests (owner: W2)
//!
//! `wikanvas-server admin …` runs as the real binary on a temporary data directory, also while an in-process server
//! has the same database open (WAL, a second process). The background jobs of §12 are called directly: the hourly
//! run, purge and GC in their order, the backup schedule, and backups that never overlap across processes. The
//! data-safety guard of D144 runs both ways: purge and GC sweep no orphan file while no owner exists, and a real
//! `wikanvas-server serve` refuses to start, and `admin create-owner` to create the owner, over files without an owner.

use std::time::{Duration, UNIX_EPOCH};

use axum::http::StatusCode;
use serde_json::json;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use wikanvas_api::{Attachment, ErrorCode, Me, Scope};
use wikanvas_core::api::{ItemKind, NewCollection, NewItem};
use wikanvas_server::AppState;
use wikanvas_server::config::Config;
use wikanvas_server::jobs::{self, BACKUP_STARTUP_DELAY};

use crate::common::{OWNER_EMAIL, TestApp, admin, assert_error, stdout_json};

const OWNER: [&str; 8] = [
    "create-owner",
    "--email",
    "a@wikanvas.invalid",
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
    let db = wikanvas_store::Db::open(
        dir.join("wikanvas.db"),
        wikanvas_store::DbOptions::default(),
    )
    .unwrap();
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
    assert_eq!(created["user"]["email"], "a@wikanvas.invalid");
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
        "a@wikanvas.invalid",
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
    assert!(wikanvas_api::token::check(created["token"].as_str().unwrap()).is_ok());
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
    assert!(wikanvas_api::token::check(secret.trim_end()).is_ok());

    let unknown = admin(
        dir.path(),
        &[
            "create-token",
            "--name",
            "t",
            "--scope",
            "read",
            "--email",
            "b@wikanvas.invalid",
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
    assert!(wikanvas_api::token::check(&token).is_ok());
    assert!(!String::from_utf8_lossy(&out.stderr).contains(&token));

    let state = AppState::open(Config::for_tests(dir.path())).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(wikanvas_server::app::serve(state, listener, async {
        let _ = stopped.await;
    }));
    let (status, body) = http_get(addr, "/api/v1/auth/me", &token).await;
    assert_eq!(status, 200, "{body}");
    let me: Me = serde_json::from_str(&body).unwrap();
    assert_eq!(me.auth.token.unwrap().name, "ci");
    let (status, _) = http_get(
        addr,
        "/api/v1/auth/me",
        &wikanvas_api::token::encode(&[1; 32]),
    )
    .await;
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
    let a: wikanvas_api::Attachment = res.json();
    let sha = wikanvas_store::Sha256::parse(&a.sha256).unwrap();
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
    let status: wikanvas_api::SetupStatus = client.get("/api/v1/setup").await.json();
    assert!(!status.needs_setup);
    // The setup code no longer works.
    let res = client
        .post("/api/v1/setup")
        .json(&json!({
            "code": app.state.setup_code.to_string(),
            "workspace_name": "X",
            "name": "X",
            "email": "x@wikanvas.invalid",
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

const DAY: i64 = 86_400_000;

#[tokio::test]
async fn backup_busy_while_another_process_holds_the_lock() {
    let app = TestApp::with_owner().await;
    let dir = app.state.config.backups_dir();
    std::fs::create_dir_all(&dir).unwrap();
    // This process plays the server whose scheduled backup is running.
    let held = wikanvas_store::backup::lock(&dir).unwrap();
    let out = admin(app.dir.path(), &["--json", "backup"], None);
    assert_eq!(out.status.code(), Some(11), "{out:?}");
    let err = error_line(&out);
    assert_eq!(err["error"]["code"], "rate_limited");
    assert_eq!(err["error"]["details"], json!({"reason": "busy"}));
    // And the other way round: the server's on-demand backup while the command runs.
    let res = app.server.post("/api/v1/admin/backup").await;
    let body = assert_error(&res, 429, ErrorCode::RateLimited);
    assert_eq!(body.error.details, Some(json!({"reason": "busy"})));
    assert!(res.headers().contains_key("retry-after"));
    assert!(wikanvas_store::backup::list(&dir).unwrap().is_empty());
    drop(held);
    ok(&admin(app.dir.path(), &["--json", "backup"], None));
    app.server
        .post("/api/v1/admin/backup")
        .await
        .assert_status_ok();
    assert_eq!(wikanvas_store::backup::list(&dir).unwrap().len(), 2);
}

#[tokio::test]
async fn hourly_job_expires_sessions_purges_tokens_and_prunes_limiters() {
    let mut app = TestApp::with_fake_clock().await;
    app.sign_in_owner().await;
    let owner = app.owner().to_owned();
    let now = app.state.now();
    let token = app.mint_token(&owner, Scope::Admin).await;
    let mut dead = Vec::new();
    for _ in 0..3 {
        dead.push(wikanvas_store::tokens::hash(
            &app.mint_token(&owner, Scope::Read).await,
        ));
    }
    let user = owner.clone();
    app.state
        .db
        .write(move |tx| {
            tx.execute(
                "INSERT INTO sessions (id, token_hash, user_id, persistent, created_at, last_seen_at, expires_at)
                 VALUES ('stale', randomblob(32), ?1, 0, ?2, ?2, ?3)",
                rusqlite::params![user, now - 2 * DAY, now - 1],
            )?;
            // Revoked 31 days ago and expired 31 days ago go; revoked 29 days ago stays.
            let sql = [
                "UPDATE api_tokens SET revoked_at = ?2 WHERE token_hash = ?1",
                "UPDATE api_tokens SET expires_at = ?2 WHERE token_hash = ?1",
                "UPDATE api_tokens SET revoked_at = ?2 WHERE token_hash = ?1",
            ];
            let at = [now - 31 * DAY, now - 31 * DAY, now - 29 * DAY];
            for ((sql, hash), at) in sql.iter().zip(&dead).zip(at) {
                tx.execute(sql, rusqlite::params![hash.as_slice(), at])?;
            }
            Ok(())
        })
        .await
        .unwrap();
    // A key in every limiter and the last-use map.
    let client = app.token_client(&token);
    client.get("/api/v1/auth/me").await.assert_status_ok();
    let res = client
        .post("/api/v1/tokens")
        .json(&json!({"name": "child", "scopes": ["read"], "expires_in_days": 1}))
        .await;
    assert!(res.status_code().is_success(), "{}", res.text());
    let res = app
        .token_client("qpat_unknown")
        .get("/api/v1/auth/me")
        .await;
    assert_error(&res, 401, ErrorCode::InvalidToken);
    let res = app.login("a wrong password", false, None).await;
    assert_error(&res, 401, ErrorCode::InvalidCredentials);
    let keys = |app: &TestApp| {
        let s = &app.state;
        [
            s.token_limiter.keys(),
            s.token_bytes_limiter.keys(),
            s.auth_fail_limiter.keys(),
            s.login_limiter.ip_keys(),
            s.login_limiter.per_email.keys(),
            s.last_used.keys(),
        ]
    };
    assert_eq!(keys(&app), [1; 6]);
    let report = jobs::hourly(&app.state).await.unwrap();
    assert_eq!((report.sessions_deleted, report.tokens_deleted), (1, 2));
    let sessions: i64 = app
        .state
        .db
        .read(|c| Ok(c.query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))?))
        .await
        .unwrap();
    assert_eq!(sessions, 1, "the live session stays");
    let tokens: i64 = app
        .state
        .db
        .read(|c| Ok(c.query_row("SELECT count(*) FROM api_tokens", [], |r| r.get(0))?))
        .await
        .unwrap();
    assert_eq!(
        tokens, 3,
        "the admin token, its child and the one revoked 29 days ago"
    );
    assert_eq!(
        keys(&app)[4..],
        [1, 1],
        "recent failures and last uses stay"
    );
    // Two hours later every key is idle and goes.
    app.state.clock.advance(Duration::from_secs(2 * 3600));
    jobs::hourly(&app.state).await.unwrap();
    assert_eq!(keys(&app), [0; 6]);
}

#[tokio::test]
async fn purge_and_gc_runs_purge_then_attachment_gc_then_blob_gc() {
    let mut app = TestApp::with_fake_clock().await;
    app.sign_in_owner().await;
    let res = app
        .server
        .post("/api/v1/attachments?filename=a.txt")
        .bytes("only in a trashed doc\n".into())
        .content_type("application/octet-stream")
        .await;
    res.assert_status(StatusCode::CREATED);
    let attachment: Attachment = res.json();
    let (owner, now) = (app.owner().to_owned(), app.state.now());
    let attachment_id = attachment.id.to_string();
    // A doc in the Trash that is the attachment's only reference.
    app.state
        .db
        .write(move |tx| {
            let new = NewCollection {
                name: "Notes".into(),
                ..Default::default()
            };
            let by = wikanvas_store::Viewer {
                user_id: owner.clone(),
                role: wikanvas_api::Role::Owner,
            };
            let c = wikanvas_store::collections::create(tx, &by, &new, now)?;
            let new = NewItem {
                collection_id: c.id,
                parent_id: None,
                kind: ItemKind::Doc,
                title: Some("d".into()),
                position: None,
            };
            let id = wikanvas_store::items::create(tx, &owner, &new, false, now)?
                .id
                .to_string();
            tx.execute(
                "INSERT INTO item_attachments (item_id, attachment_id) VALUES (?1, ?2)",
                rusqlite::params![id, attachment_id],
            )?;
            tx.execute(
                "UPDATE items SET deleted_at = ?2, trash_root_id = id WHERE id = ?1",
                rusqlite::params![id, now],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let sha = &attachment.sha256;
    let file = app
        .state
        .config
        .files_dir()
        .join(&sha[0..2])
        .join(&sha[2..4])
        .join(sha);
    assert!(file.exists());
    // Nothing is due yet.
    assert_eq!(
        jobs::purge_and_gc(&app.state).await.unwrap(),
        jobs::PurgeReport::default()
    );
    // 31 days later one run frees all three, which only works in the order purge, attachments, blobs.
    app.state.clock.advance(Duration::from_secs(31 * 86_400));
    let report = jobs::purge_and_gc(&app.state).await.unwrap();
    assert_eq!(report.items_purged, 1);
    assert_eq!(report.attachments_deleted, 1);
    assert_eq!(report.blobs.blobs, 1);
    assert!(!file.exists(), "the blob file is unlinked");
}

/// Writes `bytes` as `files/aa/bb/<sha256>` under `dir` with no row, as a volume whose database is missing holds them,
/// with its mtime `age` in the past; returns the path.
fn put_file(dir: &std::path::Path, bytes: &[u8], age: Duration) -> std::path::PathBuf {
    let path =
        wikanvas_store::BlobStore::new(dir.to_path_buf()).path(&wikanvas_store::Sha256::of(bytes));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, bytes).unwrap();
    let file = std::fs::File::options().append(true).open(&path).unwrap();
    file.set_modified(std::time::SystemTime::now() - age)
        .unwrap();
    path
}

#[tokio::test]
async fn purge_and_gc_sweeps_orphan_files_only_with_an_owner() {
    let app = TestApp::new().await;
    let old = wikanvas_store::gc::ORPHAN_FILE_AGE + Duration::from_secs(3600);
    let file = put_file(app.dir.path(), b"an image of another database", old);
    // No owner (D144): the database is not the files' own, so purge and GC sweep nothing, however old the file.
    assert_eq!(
        jobs::purge_and_gc(&app.state).await.unwrap(),
        jobs::PurgeReport::default()
    );
    assert!(file.exists());
    // With an owner, a file without a row that is older than the age is swept as before.
    app.insert_owner().await;
    let report = jobs::purge_and_gc(&app.state).await.unwrap();
    assert_eq!(report.blobs.orphan_files, 1);
    assert!(!file.exists());
}

/// `wikanvas-server serve` on `dir`, on a free loopback port, with its output piped.
fn spawn_serve(dir: &std::path::Path) -> (std::process::Child, std::net::SocketAddr) {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_wikanvas-server"))
        .arg("serve")
        .env_clear()
        .env("WIKANVAS_DATA_DIR", dir)
        .env("WIKANVAS_LISTEN", addr.to_string())
        .env("WIKANVAS_TEST_ARGON2", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("run wikanvas-server serve");
    (child, addr)
}

/// Both output streams of a finished server.
fn output_text(out: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Serves `dir` until `/healthz` answers 200, then kills the server; returns its log.
fn serve_until_healthy(dir: &std::path::Path) -> String {
    let (mut child, addr) = spawn_serve(dir);
    let mut up = false;
    for _ in 0..400 {
        if wikanvas_server::cli::probe(&format!("http://{addr}"), Duration::from_millis(200))
            == Ok(200)
        {
            up = true;
            break;
        }
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let _ = child.kill();
    let log = output_text(&child.wait_with_output().unwrap());
    assert!(up, "the server did not come up: {log}");
    log
}

/// Runs `serve` on `dir`, which must stop by itself within 30 s (it is killed otherwise, and the code is `None`);
/// returns the exit code and the log.
fn serve_until_exit(dir: &std::path::Path) -> (Option<i32>, String) {
    let (mut child, _) = spawn_serve(dir);
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while child.try_wait().unwrap().is_none() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    let _ = child.kill();
    let out = child.wait_with_output().unwrap();
    (out.status.code(), output_text(&out))
}

/// The refusal of D144 on `dir`: `action` is "start" (`serve`) or "create the owner" (`admin create-owner`),
/// `database` "does not exist" or "has no owner", `files` the count with its noun.
fn refusal(dir: &std::path::Path, action: &str, database: &str, files: &str) -> String {
    let (db, files_dir) = (dir.join("wikanvas.db"), dir.join("files"));
    format!(
        "refusing to {action}: the database {} {database}, but {} holds {files} (attachments and board images of an \
         existing wiki); put that wiki's database in place as {} (see \"Backups and restore\" in README.md), or move \
         {} away to start a new wiki",
        db.display(),
        files_dir.display(),
        db.display(),
        files_dir.display()
    )
}

/// Asserts `serve`'s refusal of D144: exit 1 with the database path, the number of files and both ways out, and
/// nothing of a serving server (no listener, no setup code).
fn assert_refused(
    code: Option<i32>,
    log: &str,
    dir: &std::path::Path,
    database: &str,
    files: &str,
) {
    assert_eq!(code, Some(1), "{log}");
    let expected = refusal(dir, "start", database, files);
    assert!(log.contains(&expected), "want {expected:?} in {log}");
    assert!(!log.contains("First-run setup"), "no setup code: {log}");
    assert!(!log.contains("is serving"), "{log}");
}

#[test]
fn serve_starts_on_an_empty_data_dir() {
    let dir = tempfile::tempdir().unwrap();
    let log = serve_until_healthy(dir.path());
    assert!(log.contains("First-run setup: http://localhost:"), "{log}");
    assert!(dir.path().join("wikanvas.db").exists());
}

#[test]
fn serve_refuses_files_without_a_database() {
    let dir = tempfile::tempdir().unwrap();
    let file = put_file(dir.path(), b"an attachment", Duration::from_secs(7200));
    let (code, log) = serve_until_exit(dir.path());
    assert_refused(code, &log, dir.path(), "does not exist", "1 file");
    assert!(file.exists());
    assert!(
        !dir.path().join("wikanvas.db").exists(),
        "a refused start creates no database"
    );
}

#[test]
fn serve_refuses_files_with_an_ownerless_database() {
    let dir = tempfile::tempdir().unwrap();
    // An earlier start made the empty database; then files arrive (any regular file counts, at any depth).
    serve_until_healthy(dir.path());
    let file = put_file(dir.path(), b"a board image", Duration::from_secs(7200));
    std::fs::write(dir.path().join("files").join("stray"), b"x").unwrap();
    let (code, log) = serve_until_exit(dir.path());
    assert_refused(code, &log, dir.path(), "has no owner", "2 files");
    assert!(file.exists());
}

#[test]
fn serve_starts_with_files_and_an_owner() {
    let dir = tempfile::tempdir().unwrap();
    ok(&admin(dir.path(), &OWNER, None));
    let file = put_file(dir.path(), b"a young file without a row", Duration::ZERO);
    let log = serve_until_healthy(dir.path());
    assert!(
        !log.contains("refusing") && !log.contains("First-run setup"),
        "{log}"
    );
    assert!(file.exists(), "younger than the orphan age");
}

async fn owners(dir: &std::path::Path) -> i64 {
    let db = wikanvas_store::Db::open(
        dir.join("wikanvas.db"),
        wikanvas_store::DbOptions::default(),
    )
    .unwrap();
    db.read(|c| {
        Ok(
            c.query_row("SELECT count(*) FROM users WHERE role = 'owner'", [], |r| {
                r.get(0)
            })?,
        )
    })
    .await
    .unwrap()
}

#[test]
fn create_owner_refuses_files_without_a_database() {
    let dir = tempfile::tempdir().unwrap();
    let file = put_file(dir.path(), b"an attachment", Duration::from_secs(7200));
    let mut args = OWNER.to_vec();
    args.push("--json");
    let out = admin(dir.path(), &args, None);
    assert_eq!(out.status.code(), Some(3), "{}", output_text(&out));
    let err = error_line(&out);
    assert_eq!(err["error"]["code"], "conflict");
    assert_eq!(err["exit_code"], 3);
    assert_eq!(
        err["error"]["message"],
        refusal(dir.path(), "create the owner", "does not exist", "1 file")
    );
    assert!(out.stdout.is_empty(), "no owner and no password printed");
    assert!(
        !dir.path().join("wikanvas.db").exists(),
        "a refused create-owner creates no database"
    );
    assert!(file.exists());
    // serve still refuses: nothing changed.
    let (code, log) = serve_until_exit(dir.path());
    assert_refused(code, &log, dir.path(), "does not exist", "1 file");
}

#[tokio::test]
async fn create_owner_refuses_files_with_an_ownerless_database() {
    let dir = tempfile::tempdir().unwrap();
    serve_until_healthy(dir.path());
    let file = put_file(dir.path(), b"a board image", Duration::from_secs(7200));
    let out = admin(dir.path(), &OWNER, None);
    assert_eq!(out.status.code(), Some(3), "{}", output_text(&out));
    let expected = format!(
        "wikanvas-server admin: {}",
        refusal(dir.path(), "create the owner", "has no owner", "1 file")
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(&expected),
        "want {expected:?} in {}",
        output_text(&out)
    );
    assert!(out.stdout.is_empty());
    assert_eq!(owners(dir.path()).await, 0, "no owner row");
    assert!(file.exists());
    let (code, log) = serve_until_exit(dir.path());
    assert_refused(code, &log, dir.path(), "has no owner", "1 file");
}

#[tokio::test]
async fn create_owner_on_an_empty_data_dir_still_works() {
    // An empty data directory, files/ with directories only: create-owner works as before, and serve then starts.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("files").join("aa").join("bb")).unwrap();
    let mut args = OWNER.to_vec();
    args.push("--json");
    let out = admin(dir.path(), &args, None);
    ok(&out);
    assert_eq!(stdout_json(&out)["user"]["role"], "owner");
    assert_eq!(owners(dir.path()).await, 1);
    let log = serve_until_healthy(dir.path());
    assert!(
        !log.contains("refusing") && !log.contains("First-run setup"),
        "{log}"
    );
}

#[test]
fn backup_schedule_follows_the_newest_backup() {
    const NOW: i64 = 1_793_179_800_000;
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::for_tests(dir.path());
    config.backup_interval = Duration::from_secs(24 * 3600);
    config.backup_keep = 7;
    // None yet: soon after startup.
    assert_eq!(
        jobs::first_backup_delay(&config, NOW),
        Some(BACKUP_STARTUP_DELAY)
    );
    // WIKANVAS_BACKUP_KEEP=0 schedules nothing (D21).
    config.backup_keep = 0;
    assert_eq!(jobs::first_backup_delay(&config, NOW), None);
    config.backup_keep = 7;
    let backups = config.backups_dir();
    std::fs::create_dir_all(&backups).unwrap();
    let backup_at = |name: &str, at: i64| {
        let file = std::fs::File::create(backups.join(name)).unwrap();
        file.set_modified(UNIX_EPOCH + Duration::from_millis(at as u64))
            .unwrap();
    };
    // Pre-migration backups do not count.
    backup_at("pre-migrate-v1-20261028T0930Z.db", NOW);
    assert_eq!(
        jobs::first_backup_delay(&config, NOW),
        Some(BACKUP_STARTUP_DELAY)
    );
    // An overdue one: soon after startup.
    backup_at("wikanvas-20261027T0830Z.db", NOW - 25 * 3_600_000);
    assert_eq!(
        jobs::first_backup_delay(&config, NOW),
        Some(BACKUP_STARTUP_DELAY)
    );
    // The newest is an hour old: the rest of the interval, however recently the server started.
    backup_at("wikanvas-20261028T0830Z.db", NOW - 3_600_000);
    assert_eq!(
        jobs::first_backup_delay(&config, NOW),
        Some(Duration::from_secs(23 * 3600))
    );
    // Due in 30 s: still the startup delay first.
    assert_eq!(
        jobs::first_backup_delay(&config, NOW + 23 * 3_600_000 - 30_000),
        Some(BACKUP_STARTUP_DELAY)
    );
}

#[tokio::test]
async fn account_commands_follow_the_account_rules() {
    let app = TestApp::with_owner().await;
    let dir = app.dir.path();
    // create-user: a generated password must be changed; stdin takes 12–1024 bytes; a taken email is exit 8.
    let out = admin(
        dir,
        &[
            "--json",
            "create-user",
            "--email",
            "Bea@wikanvas.invalid",
            "--name",
            "Bea",
            "--role",
            "member",
            "--generate-password",
        ],
        None,
    );
    ok(&out);
    let created = stdout_json(&out);
    assert_eq!(created["user"]["role"], "member");
    assert_eq!(created["user"]["disabled_at"], serde_json::Value::Null);
    let password = created["generated_password"].as_str().unwrap().to_owned();
    assert_eq!(password.len(), 24);
    let bea = created["user"]["id"].as_str().unwrap().to_owned();
    let login = app
        .anon_client()
        .post("/api/v1/auth/login")
        .json(&json!({"email": "bea@wikanvas.invalid", "password": password}))
        .await;
    login.assert_status_ok();
    let cookie = crate::common::cookie_from(&login, "wikanvas_session").unwrap();
    let me: Me = app
        .cookie_client(&cookie)
        .get("/api/v1/auth/me")
        .await
        .json();
    assert!(me.must_change_password);
    let viewer = [
        "--json",
        "create-user",
        "--email",
        "cy@wikanvas.invalid",
        "--name",
        "Cy",
        "--role",
        "viewer",
        "--password-stdin",
    ];
    assert_eq!(admin(dir, &viewer, Some("short\n")).status.code(), Some(10));
    ok(&admin(dir, &viewer, Some("a long enough password\n")));
    let taken = admin(dir, &viewer, Some("a long enough password\n"));
    assert_eq!(taken.status.code(), Some(8));
    assert_eq!(error_line(&taken)["error"]["code"], "conflict");
    let usage = admin(
        dir,
        &[
            "create-user",
            "--email",
            "d@x",
            "--name",
            "D",
            "--role",
            "owner",
            "--generate-password",
        ],
        None,
    );
    assert_eq!(usage.status.code(), Some(2));
    // list-users: the owner first.
    let out = admin(dir, &["--json", "list-users"], None);
    ok(&out);
    let list = stdout_json(&out);
    let roles: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, ["owner", "member", "viewer"]);
    // set-role: the operator may make an admin; never changes the owner (exit 6).
    let out = admin(
        dir,
        &[
            "--json",
            "set-role",
            "--email",
            "bea@wikanvas.invalid",
            "--role",
            "admin",
        ],
        None,
    );
    ok(&out);
    assert_eq!(stdout_json(&out)["user"]["role"], "admin");
    for args in [
        &[
            "--json",
            "set-role",
            "--email",
            OWNER_EMAIL,
            "--role",
            "admin",
        ][..],
        &["--json", "disable-user", "--email", OWNER_EMAIL][..],
    ] {
        let refused = admin(dir, args, None);
        assert_eq!(refused.status.code(), Some(6), "{args:?}");
        assert_eq!(error_line(&refused)["error"]["code"], "forbidden");
    }
    let unknown = admin(
        dir,
        &["--json", "disable-user", "--email", "no@wikanvas.invalid"],
        None,
    );
    assert_eq!(unknown.status.code(), Some(7));
    // disable-user ends the sessions and tokens at once; list-tokens and revoke-token take --email.
    let token = app.mint_token(&bea, Scope::Read).await;
    let out = admin(
        dir,
        &["--json", "list-tokens", "--email", "bea@wikanvas.invalid"],
        None,
    );
    ok(&out);
    assert_eq!(stdout_json(&out).as_array().unwrap().len(), 1);
    let owners = admin(dir, &["--json", "list-tokens"], None);
    ok(&owners);
    assert!(stdout_json(&owners).as_array().unwrap().is_empty());
    let out = admin(
        dir,
        &["--json", "disable-user", "--email", "bea@wikanvas.invalid"],
        None,
    );
    ok(&out);
    let disabled = stdout_json(&out);
    assert!(disabled["user"]["disabled_at"].is_string());
    assert_eq!(
        (
            disabled["sessions_revoked"].as_u64(),
            disabled["tokens_revoked"].as_u64(),
            disabled["invites_revoked"].as_u64()
        ),
        (Some(1), Some(1), Some(0))
    );
    assert_error(
        &app.cookie_client(&cookie).get("/api/v1/auth/me").await,
        401,
        ErrorCode::Unauthenticated,
    );
    assert_error(
        &app.token_client(&token).get("/api/v1/auth/me").await,
        401,
        ErrorCode::InvalidToken,
    );
    let out = admin(
        dir,
        &["--json", "enable-user", "--email", "bea@wikanvas.invalid"],
        None,
    );
    ok(&out);
    assert_eq!(
        stdout_json(&out)["user"]["disabled_at"],
        serde_json::Value::Null
    );
    let second = app.mint_token(&bea, Scope::Read).await;
    let out = admin(
        dir,
        &[
            "--json",
            "revoke-token",
            "--all",
            "--yes",
            "--email",
            "bea@wikanvas.invalid",
        ],
        None,
    );
    ok(&out);
    assert_eq!(stdout_json(&out)["revoked"], 1);
    assert_error(
        &app.token_client(&second).get("/api/v1/auth/me").await,
        401,
        ErrorCode::InvalidToken,
    );
    let text = admin(dir, &["list-users"], None);
    ok(&text);
    assert!(String::from_utf8_lossy(&text.stdout).contains("cy@wikanvas.invalid"));
}

#[test]
fn create_user_needs_an_owner_first() {
    let dir = tempfile::tempdir().unwrap();
    let out = admin(
        dir.path(),
        &[
            "--json",
            "create-user",
            "--email",
            "m@wikanvas.invalid",
            "--name",
            "M",
            "--role",
            "member",
            "--generate-password",
        ],
        None,
    );
    assert_eq!(out.status.code(), Some(8));
    assert_eq!(error_line(&out)["error"]["code"], "setup_required");
}
