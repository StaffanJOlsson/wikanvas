//! Attachment tests (owner: W2)
//!
//! Uploads are raw bodies (`application/octet-stream`); the type is sniffed, images are checked from their header,
//! bodies stream to `/data/tmp` under the `upload_work` permit, and downloads carry the §11 headers.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use bytes::Bytes;
use quire_api::{Attachment, ErrorCode, Scope};
use serde_json::json;
use tokio::sync::mpsc;
use tower::ServiceExt as _;

use crate::common::{TestApp, assert_error, router};

/// A PNG header claiming `w`×`h` (what the sniffer and imagesize read).
pub fn png(w: u32, h: u32) -> Vec<u8> {
    let mut v = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    v.extend_from_slice(&w.to_be_bytes());
    v.extend_from_slice(&h.to_be_bytes());
    v.extend_from_slice(b"\x08\x06\0\0\0\0\0\0\0");
    v.extend_from_slice(b"\0\0\0\0IEND\xaeB`\x82");
    v
}

async fn upload(
    app: &TestApp,
    query: &str,
    bytes: Vec<u8>,
    content_type: &str,
) -> axum_test::TestResponse {
    app.server
        .post(&format!("/api/v1/attachments{query}"))
        .bytes(bytes.into())
        .content_type(content_type)
        .await
}

#[tokio::test]
async fn upload_mime_sniffed_not_trusted() {
    let app = TestApp::with_token(Scope::Write).await;
    let res = upload(
        &app,
        "?filename=photo.txt",
        png(3, 2),
        "application/octet-stream",
    )
    .await;
    res.assert_status(StatusCode::CREATED);
    let a: Attachment = res.json();
    assert_eq!(a.mime, "image/png");
    assert_eq!((a.width, a.height), (Some(3), Some(2)));
    assert_eq!(a.url, format!("/api/v1/attachments/{}", a.id));
    assert_eq!(a.sha256.len(), 64);
    let res = upload(
        &app,
        "?filename=fake.png",
        b"just some text\n".to_vec(),
        "image/png",
    )
    .await;
    res.assert_status(StatusCode::CREATED);
    let t: Attachment = res.json();
    assert_eq!(
        (t.mime.as_str(), t.width, t.height),
        ("text/plain", None, None)
    );
    let res = app.server.get(&t.url).await;
    res.assert_status_ok();
    assert_eq!(res.header("content-type"), "text/plain");
    assert_eq!(res.text(), "just some text\n");
    // Read may download, not upload.
    let read = app.mint_token(app.owner(), Scope::Read).await;
    app.token_client(&read).get(&a.url).await.assert_status_ok();
    let res = app
        .token_client(&read)
        .post("/api/v1/attachments")
        .bytes(png(1, 1).into())
        .content_type("application/octet-stream")
        .await;
    assert_error(&res, 403, ErrorCode::InsufficientScope);
    assert_error(
        &app.server
            .get(&format!("/api/v1/attachments/{}", uuid::Uuid::now_v7()))
            .await,
        404,
        ErrorCode::NotFound,
    );
    // The upload context must exist.
    let res = upload(
        &app,
        &format!("?item_id={}", uuid::Uuid::now_v7()),
        png(1, 1),
        "application/octet-stream",
    )
    .await;
    assert_error(&res, 404, ErrorCode::NotFound);
}

#[tokio::test]
async fn svg_attachment_is_download_with_sandbox_csp() {
    let app = TestApp::with_owner().await;
    let svg = b"<svg xmlns='http://www.w3.org/2000/svg'><script>alert(1)</script></svg>".to_vec();
    let a: Attachment = upload(&app, "?filename=x.svg", svg, "application/octet-stream")
        .await
        .json();
    assert_eq!(a.mime, "image/svg+xml");
    let res = app.server.get(&a.url).await;
    res.assert_status_ok();
    assert!(
        res.header("content-disposition")
            .to_str()
            .unwrap()
            .starts_with("attachment; ")
    );
    assert_eq!(
        res.header("content-security-policy"),
        "default-src 'none'; sandbox"
    );
    assert_eq!(res.header("x-content-type-options"), "nosniff");
    assert_eq!(
        res.header("cache-control"),
        "private, max-age=31536000, immutable"
    );
    let p: Attachment = upload(
        &app,
        "?filename=p.png",
        png(2, 2),
        "application/octet-stream",
    )
    .await
    .json();
    let res = app.server.get(&p.url).await;
    assert!(
        res.header("content-disposition")
            .to_str()
            .unwrap()
            .starts_with("inline; ")
    );
    assert_eq!(
        res.header("content-security-policy"),
        "default-src 'none'; sandbox"
    );
    assert!(res.maybe_header("content-encoding").is_none());
}

#[tokio::test]
async fn content_disposition_rfc6266_sanitized() {
    let app = TestApp::with_token(Scope::Write).await;
    let a: Attachment = upload(
        &app,
        "?filename=a%22b%0D%0Ac%5C%2F%20%C3%A9.png",
        png(1, 1),
        "application/octet-stream",
    )
    .await
    .json();
    let res = app.server.get(&a.url).await;
    let cd = res
        .header("content-disposition")
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        cd,
        "inline; filename=\"abc _.png\"; filename*=UTF-8''abc%20%C3%A9.png"
    );
    let stored: String = app
        .state
        .db
        .read(|c| Ok(c.query_row("SELECT filename FROM attachments", [], |r| r.get(0))?))
        .await
        .unwrap();
    assert_eq!(stored, "abc é.png");
    // No name: `file`.
    let b: Attachment = upload(&app, "", b"x".to_vec(), "application/octet-stream")
        .await
        .json();
    let res = app.server.get(&b.url).await;
    assert_eq!(
        res.header("content-disposition"),
        "attachment; filename=\"file\"; filename*=UTF-8''file"
    );
}

#[tokio::test]
async fn image_over_100_megapixels_422() {
    let app = TestApp::with_token(Scope::Write).await;
    let res = upload(&app, "", png(20_000, 20_000), "application/octet-stream").await;
    let err = assert_error(&res, 422, ErrorCode::Validation);
    assert_eq!(err.error.details, Some(json!({"field": "body"})));
    upload(&app, "", png(10_000, 10_000), "application/octet-stream")
        .await
        .assert_status(StatusCode::CREATED);
    let rows: i64 = app
        .state
        .db
        .read(|c| Ok(c.query_row("SELECT count(*) FROM attachments", [], |r| r.get(0))?))
        .await
        .unwrap();
    assert_eq!(rows, 1);
    assert_eq!(
        std::fs::read_dir(app.state.blobs.tmp_dir())
            .unwrap()
            .count(),
        0,
        "the refused file is gone"
    );
}

#[tokio::test]
async fn upload_over_limit_413() {
    let mut app = TestApp::with_config(|c| c.max_upload_mb = 1).await;
    let owner = app.insert_owner().await;
    let token = app.mint_token(&owner, Scope::Write).await;
    app.server = app.token_client(&token);
    let res = upload(
        &app,
        "",
        vec![b'a'; 1024 * 1024 + 1],
        "application/octet-stream",
    )
    .await;
    let err = assert_error(&res, 413, ErrorCode::PayloadTooLarge);
    assert_eq!(err.error.details, Some(json!({"limit_bytes": 1048576})));
    // Without Content-Length the limit holds while streaming.
    let chunks = (0..17).map(|_| Ok::<_, std::io::Error>(Bytes::from(vec![b'b'; 64 * 1024])));
    let req = Request::post("/api/v1/attachments")
        .header("host", "localhost")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/octet-stream")
        .body(Body::from_stream(futures_util::stream::iter(chunks)))
        .unwrap();
    let res = router(&app.state).oneshot(req).await.unwrap();
    assert_eq!(res.status(), 413);
    upload(
        &app,
        "",
        vec![b'a'; 1024 * 1024],
        "application/octet-stream",
    )
    .await
    .assert_status(StatusCode::CREATED);
}

#[tokio::test]
async fn low_disk_507() {
    let mut app = TestApp::with_owner().await;
    app.state.free_space = Arc::new(|_, _| {
        Err(quire_store::Error::new(
            ErrorCode::InsufficientStorage,
            "the disk is full",
        ))
    });
    app.server = app.cookie_client(app.session.as_deref().unwrap());
    let res = upload(&app, "", png(1, 1), "application/octet-stream").await;
    assert_error(&res, 507, ErrorCode::InsufficientStorage);
    let res = app.server.get("/api/v1/export").await;
    assert_error(&res, 507, ErrorCode::InsufficientStorage);
    let res = app.server.post("/api/v1/admin/backup").await;
    assert_error(&res, 507, ErrorCode::InsufficientStorage);
    // A board PUT checks through the same probe before it stages its assets (§6.2 step 3): nothing is written.
    let board = crate::boards::new_board(&app, "Full disk").await;
    let res = app
        .server
        .put(&format!("/api/v1/items/{board}/board"))
        .add_header("if-match", "*")
        .text(crate::boards::board("Full disk", "x"))
        .content_type("application/json")
        .await;
    assert_error(&res, 507, ErrorCode::InsufficientStorage);
    let detail: quire_api::ItemDetail = app
        .server
        .get(&format!("/api/v1/items/{board}"))
        .await
        .json();
    assert_eq!(detail.board.map(|b| (b.rev, b.objects)), Some((1, 0)));
    assert_eq!(
        std::fs::read_dir(app.state.blobs.tmp_dir())
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        std::fs::read_dir(app.state.config.backups_dir())
            .unwrap()
            .count(),
        0
    );
}

/// A request body fed through a channel; dropping the sender ends it.
fn streamed(token: &str) -> (mpsc::Sender<Bytes>, Request<Body>) {
    let (tx, rx) = mpsc::channel::<Bytes>(1);
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|b| (Ok::<_, std::io::Error>(b), rx))
    });
    let req = Request::post("/api/v1/attachments?filename=big.bin")
        .header("host", "localhost")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/octet-stream")
        .body(Body::from_stream(stream))
        .unwrap();
    (tx, req)
}

fn staged_bytes(app: &TestApp) -> Vec<u64> {
    std::fs::read_dir(app.state.blobs.tmp_dir())
        .map(|d| {
            d.flatten()
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .collect()
        })
        .unwrap_or_default()
}

async fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    for _ in 0..500 {
        if ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upload_streams_under_upload_work() {
    let app = TestApp::with_token(Scope::Write).await;
    let token = app.token.clone().unwrap();
    let chunk = Bytes::from(vec![7u8; 64 * 1024]);
    let (tx1, req1) = streamed(&token);
    let (tx2, req2) = streamed(&token);
    let first = tokio::spawn(router(&app.state).oneshot(req1));
    let second = tokio::spawn(router(&app.state).oneshot(req2));
    // 1 MiB of the first body reaches the disk before the body is complete: it is streamed, not buffered.
    for _ in 0..16 {
        tx1.send(chunk.clone()).await.unwrap();
    }
    tx2.send(chunk.clone()).await.unwrap();
    wait_for("1 MiB staged", || {
        staged_bytes(&app).iter().any(|n| *n >= 1024 * 1024)
    })
    .await;
    wait_for("both permits taken", || {
        app.state.upload_work.available_permits() == 0
    })
    .await;
    // A third upload waits for a permit.
    let (tx3, req3) = streamed(&token);
    let third = tokio::spawn(router(&app.state).oneshot(req3));
    tx3.send(Bytes::from_static(b"small")).await.unwrap();
    drop(tx3);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!third.is_finished(), "the third upload must wait");
    assert_eq!(staged_bytes(&app).len(), 2, "and stage nothing yet");
    // More data keeps flowing to disk in chunks.
    for _ in 0..16 {
        tx1.send(chunk.clone()).await.unwrap();
    }
    drop(tx1);
    let res = first.await.unwrap().unwrap();
    assert_eq!(res.status(), 201);
    let body = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    let a: Attachment = serde_json::from_slice(&body).unwrap();
    assert_eq!(a.size, 32 * 64 * 1024);
    let res = tokio::time::timeout(Duration::from_secs(5), third)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(res.status(), 201);
    drop(tx2);
    assert_eq!(second.await.unwrap().unwrap().status(), 201);
    assert_eq!(app.state.upload_work.available_permits(), 2);
}

#[tokio::test]
async fn octet_stream_txt_upload_accepted() {
    let app = TestApp::with_owner().await;
    let res = upload(
        &app,
        "?filename=notes.txt",
        b"hello\n".to_vec(),
        "application/octet-stream",
    )
    .await;
    res.assert_status(StatusCode::CREATED);
    let a: Attachment = res.json();
    assert_eq!((a.mime.as_str(), a.size), ("text/plain", 6));
    let res = upload(
        &app,
        "?filename=notes.txt",
        b"hello\n".to_vec(),
        "text/plain",
    )
    .await;
    assert_error(&res, 415, ErrorCode::UnsupportedMediaType);
    // The same bytes again share the blob.
    let b: Attachment = upload(
        &app,
        "?filename=copy.txt",
        b"hello\n".to_vec(),
        "application/octet-stream",
    )
    .await
    .json();
    assert_eq!(a.sha256, b.sha256);
    assert_ne!(a.id, b.id);
}
