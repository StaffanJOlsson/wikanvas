//! Export tests (owner: W2)
//!
//! `GET /api/v1/export`: the tar layout, a database copy without credentials (checked on the extracted file and its
//! raw bytes), single-flight (also while a copy whose client left still runs, for the on-demand backup too), no
//! request deadline for the copy, and the cookie rule for cross-site navigations.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request};
use wikanvas_api::{Attachment, ErrorBody, ErrorCode, Scope};
use tower::ServiceExt as _;

use crate::common::{OWNER_PASSWORD, TestApp, assert_error, router};

/// The entries of a ustar archive: (name, bytes).
fn untar(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 512 <= bytes.len() {
        let header = &bytes[at..at + 512];
        if header.iter().all(|b| *b == 0) {
            break;
        }
        let field = |r: std::ops::Range<usize>| {
            let raw = &header[r];
            let end = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
            String::from_utf8(raw[..end].to_vec()).unwrap()
        };
        let mut name = field(0..100);
        let prefix = field(345..500);
        if !prefix.is_empty() {
            name = format!("{prefix}/{name}");
        }
        let size = u64::from_str_radix(field(124..136).trim(), 8).unwrap() as usize;
        let data = bytes[at + 512..at + 512 + size].to_vec();
        out.push((name, data));
        at += 512 + size.div_ceil(512) * 512;
    }
    out
}

/// An owner with a session, a token, a password hash, and one uploaded file.
async fn populated() -> (TestApp, String, Attachment) {
    let app = TestApp::with_owner().await;
    let token = app.mint_token(app.owner(), Scope::Admin).await;
    let res = app
        .server
        .post("/api/v1/attachments?filename=hello.txt")
        .bytes("hello, export\n".into())
        .content_type("application/octet-stream")
        .await;
    let attachment: Attachment = res.json();
    (app, token, attachment)
}

async fn export(app: &TestApp) -> Vec<(String, Vec<u8>)> {
    let res = app.server.get("/api/v1/export").await;
    res.assert_status_ok();
    assert_eq!(res.header("content-type"), "application/x-tar");
    assert!(
        res.header("content-disposition")
            .to_str()
            .unwrap()
            .starts_with("attachment; filename=\"wikanvas-export-")
    );
    assert_eq!(res.header("cache-control"), "no-store");
    untar(res.as_bytes())
}

#[tokio::test]
async fn tar_has_manifest_db_and_files() {
    let (app, _, attachment) = populated().await;
    let entries = export(&app).await;
    let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
    let sha = &attachment.sha256;
    let blob = format!("files/{}/{}/{sha}", &sha[0..2], &sha[2..4]);
    assert_eq!(names, ["manifest.json", "wikanvas.db", blob.as_str()]);
    let manifest: serde_json::Value = serde_json::from_slice(&entries[0].1).unwrap();
    assert_eq!(manifest["format"], "wikanvas-export");
    assert_eq!(manifest["format_version"], 1);
    assert_eq!(manifest["api_version"], wikanvas_api::API_VERSION);
    assert_eq!(manifest["schema_version"], wikanvas_store::migrate::latest());
    assert_eq!(entries[2].1, b"hello, export\n");
    assert!(entries[1].1.starts_with(b"SQLite format 3\0"));
    // The copy in /data/tmp is gone once the stream ended.
    let leftovers: Vec<_> = std::fs::read_dir(app.state.config.tmp_dir())
        .unwrap()
        .flatten()
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

/// Extracts `wikanvas.db` into an empty directory and opens it read-only (no `-wal` next to it).
fn extracted_db(
    entries: &[(String, Vec<u8>)],
) -> (tempfile::TempDir, rusqlite::Connection, Vec<u8>) {
    let dir = tempfile::tempdir().unwrap();
    let db = entries
        .iter()
        .find(|(n, _)| n == "wikanvas.db")
        .unwrap()
        .1
        .clone();
    std::fs::write(dir.path().join("wikanvas.db"), &db).unwrap();
    let conn = rusqlite::Connection::open_with_flags(
        dir.path().join("wikanvas.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    (dir, conn, db)
}

fn count(conn: &rusqlite::Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

#[tokio::test]
async fn copy_has_no_sessions_or_password_hashes() {
    let (app, _, _) = populated().await;
    let (_dir, conn, _) = extracted_db(&export(&app).await);
    assert_eq!(count(&conn, "SELECT count(*) FROM sessions"), 0);
    assert_eq!(count(&conn, "SELECT count(*) FROM users"), 1);
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM users WHERE password_hash IS NOT NULL"
        ),
        0
    );
    assert_eq!(count(&conn, "SELECT count(*) FROM attachments"), 1);
    // The live database keeps everything.
    let live: i64 = app
        .state
        .db
        .read(|c| Ok(c.query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))?))
        .await
        .unwrap();
    assert_eq!(live, 1);
    app.login(OWNER_PASSWORD, false, None)
        .await
        .assert_status_ok();
}

#[tokio::test]
async fn copy_has_no_api_tokens() {
    let (app, token, _) = populated().await;
    let (_dir, conn, _) = extracted_db(&export(&app).await);
    assert_eq!(count(&conn, "SELECT count(*) FROM api_tokens"), 0);
    assert_eq!(count(&conn, "SELECT count(*) FROM api_token_scopes"), 0);
    app.token_client(&token)
        .get("/api/v1/auth/me")
        .await
        .assert_status_ok();
}

#[tokio::test]
async fn extracted_db_has_no_credentials_without_sidecars() {
    let (app, token, _) = populated().await;
    let entries = export(&app).await;
    assert!(
        entries
            .iter()
            .all(|(n, _)| !n.ends_with("-wal") && !n.ends_with("-journal") && !n.ends_with("-shm"))
    );
    let (dir, conn, raw) = extracted_db(&entries);
    let check: String = conn
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(check, "ok");
    assert_eq!(count(&conn, "SELECT count(*) FROM sessions"), 0);
    assert_eq!(count(&conn, "SELECT count(*) FROM api_tokens"), 0);
    drop(conn);
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().into_string().unwrap())
        .collect();
    assert_eq!(
        names,
        ["wikanvas.db"],
        "opened read-only, nothing was created next to it"
    );
    let contains = |needle: &[u8]| raw.windows(needle.len()).any(|w| w == needle);
    assert!(
        !contains(b"$argon2id$"),
        "a password hash survived in the raw bytes"
    );
    assert!(
        !contains(&wikanvas_store::tokens::hash(&token)),
        "a token hash survived"
    );
    let session = wikanvas_store::sessions::token_hash(app.session.as_deref().unwrap()).unwrap();
    assert!(!contains(&session), "a session hash survived");
}

#[tokio::test]
async fn single_flight_429_busy() {
    let (app, token, _) = populated().await;
    let req = || {
        Request::get("/api/v1/export")
            .header("host", "localhost")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    };
    let first = router(&app.state).oneshot(req()).await.unwrap();
    assert_eq!(first.status(), 200);
    // The body is not read yet: the export is still running.
    let second = router(&app.state).oneshot(req()).await.unwrap();
    assert_eq!(second.status(), 429);
    assert!(second.headers().contains_key("retry-after"));
    let body = axum::body::to_bytes(second.into_body(), 1 << 20)
        .await
        .unwrap();
    let err: ErrorBody = serde_json::from_slice(&body).unwrap();
    assert_eq!(err.error.code, ErrorCode::RateLimited);
    assert_eq!(
        err.error.details,
        Some(serde_json::json!({"reason": "busy"}))
    );
    let bytes = axum::body::to_bytes(first.into_body(), 64 << 20)
        .await
        .unwrap();
    assert!(untar(&bytes).len() >= 2);
    let third = router(&app.state).oneshot(req()).await.unwrap();
    assert_eq!(third.status(), 200);
    drop(third);
    // A dropped response (the client went away) frees it too.
    let fourth = router(&app.state).oneshot(req()).await.unwrap();
    assert_eq!(fourth.status(), 200);
}

#[tokio::test]
async fn cookie_cross_site_navigation_403() {
    let (app, token, _) = populated().await;
    let cookie = format!("wikanvas_session={}", app.session.clone().unwrap());
    let get = |headers: &[(&'static str, &'static str)]| {
        let mut req = app
            .client()
            .get("/api/v1/export")
            .add_header("cookie", cookie.clone());
        for (k, v) in headers {
            req = req.add_header(*k, *v);
        }
        req
    };
    for headers in [
        &[("sec-fetch-site", "cross-site")][..],
        &[("sec-fetch-site", "same-site")],
        &[],
        &[("referer", "https://evil.example/page")],
        &[
            ("sec-fetch-site", "cross-site"),
            ("referer", "http://localhost/settings"),
        ],
    ] {
        assert_error(&get(headers).await, 403, ErrorCode::CsrfRejected);
    }
    for headers in [
        &[("sec-fetch-site", "same-origin")][..],
        &[("sec-fetch-site", "none")],
        &[("referer", "http://localhost/settings/export")],
    ] {
        get(headers).await.assert_status_ok();
    }
    // A token is not a navigation: no rule.
    app.token_client(&token)
        .get("/api/v1/export")
        .add_header("sec-fetch-site", "cross-site")
        .await
        .assert_status_ok();
}

/// A free-space probe that blocks while the gate is closed, standing in for a slow copy (the export and the backup
/// call it on the blocking pool, holding their single-flight flag); it counts its calls.
#[derive(Default)]
struct Gate {
    closed: Mutex<bool>,
    opened: Condvar,
    calls: AtomicUsize,
}

impl Gate {
    fn closed() -> Arc<Gate> {
        let gate = Gate::default();
        *gate.closed.lock().unwrap() = true;
        Arc::new(gate)
    }

    fn probe(self: &Arc<Self>) -> wikanvas_server::state::FreeSpaceProbe {
        let gate = Arc::clone(self);
        Arc::new(move |_, _| {
            gate.calls.fetch_add(1, Ordering::SeqCst);
            let mut closed = gate.closed.lock().unwrap();
            while *closed {
                closed = gate.opened.wait(closed).unwrap();
            }
            Ok(())
        })
    }

    fn open(&self) {
        *self.closed.lock().unwrap() = false;
        self.opened.notify_all();
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    async fn wait_for_calls(&self, n: usize) {
        for _ in 0..500 {
            if self.calls() >= n {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the probe was called {} times, not {n}", self.calls());
    }
}

/// Opens the gate when the test ends, even when it fails, so no blocking task stays stuck.
struct OpenOnDrop(Arc<Gate>);

impl Drop for OpenOnDrop {
    fn drop(&mut self) {
        self.0.open();
    }
}

const COPIES: [(Method, &str); 2] = [
    (Method::GET, "/api/v1/export"),
    (Method::POST, "/api/v1/admin/backup"),
];

fn bearer(method: Method, path: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("host", "localhost")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

fn files_named(dir: &std::path::Path, prefix: &str) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().into_string().unwrap())
                .filter(|n| n.starts_with(prefix))
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn single_flight_held_while_an_abandoned_copy_runs() {
    let (mut app, token, _) = populated().await;
    let gate = Gate::closed();
    let _open = OpenOnDrop(Arc::clone(&gate));
    app.state.free_space = gate.probe();
    // Each copy starts, then its client goes away: the request future is dropped while the copy runs.
    for (n, (method, path)) in COPIES.into_iter().enumerate() {
        let task = tokio::spawn(router(&app.state).oneshot(bearer(method, path, &token)));
        gate.wait_for_calls(n + 1).await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
    }
    // The abandoned copies still hold their flags: a retry is busy and starts no second copy.
    for (method, path) in COPIES {
        let res = router(&app.state)
            .oneshot(bearer(method, path, &token))
            .await
            .unwrap();
        assert_eq!(res.status(), 429, "{path}");
        let body = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        let err: ErrorBody = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            err.error.details,
            Some(serde_json::json!({"reason": "busy"}))
        );
    }
    assert_eq!(gate.calls(), 2, "no second copy started");
    // Once they are done, the flags are free; the abandoned export deleted its copy.
    gate.open();
    for (method, path) in COPIES {
        let mut res = None;
        for _ in 0..500 {
            let r = router(&app.state)
                .oneshot(bearer(method.clone(), path, &token))
                .await
                .unwrap();
            if r.status() != 429 {
                res = Some(r);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let res = res.expect("the flag is released");
        assert_eq!(res.status(), 200, "{path}");
        let body = axum::body::to_bytes(res.into_body(), 64 << 20)
            .await
            .unwrap();
        if method == Method::GET {
            assert!(untar(&body).len() >= 2);
        }
    }
    assert_eq!(
        files_named(&app.state.config.tmp_dir(), "export-"),
        Vec::<String>::new()
    );
    assert_eq!(
        files_named(&app.state.config.backups_dir(), "wikanvas-").len(),
        2,
        "the abandoned backup finished, then the retried one"
    );
}

#[tokio::test]
async fn slow_copy_is_not_cut_by_the_request_deadline() {
    let mut app = TestApp::with_config(|c| c.request_timeout = Duration::from_millis(100)).await;
    app.sign_in_owner().await;
    let token = app.mint_token(app.owner(), Scope::Admin).await;
    // Each copy takes longer than the 100 ms deadline of other requests.
    app.state.free_space = Arc::new(|_, _| {
        std::thread::sleep(Duration::from_millis(400));
        Ok(())
    });
    for (method, path) in COPIES {
        let res = router(&app.state)
            .oneshot(bearer(method, path, &token))
            .await
            .unwrap();
        assert_eq!(res.status(), 200, "{path}");
        axum::body::to_bytes(res.into_body(), 64 << 20)
            .await
            .unwrap();
    }
}
