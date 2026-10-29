//! Board route tests (owner: W6)
//!
//! `GET`/`PUT …/board`, `…/board.inkboard` and `…/board/pages/{n}` over HTTP, with the session cookie and with
//! bearer tokens: If-Match / 412 / 428, the 413 limit, the engine header, the title as `document.name`, unknown
//! fields, the `pre-api` recovery point, the preview headers of §11 and the preview worker.

use std::time::Duration;

use axum_test::TestServer;
use inkboard_core::geom::Frame;
use inkboard_core::model::{BoxGeom, Document, ImageObj, Kind, Label, Object, Page};
use inkboard_core::serial;
use inkboard_core::style::{Align, Style};
use wikanvas_api::{BoardPutResult, ErrorCode, ItemKind, NewCollection, NewItem, Scope};
use serde_json::{Value, json};

use crate::common::{TestApp, assert_error};

const PNG: &[u8] =
    b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x02\0\0\0\x02\x08\x06\0\0\0\x72\xb6\x0d\x24";

/// A collection and a board item titled `title`, made through the store; returns the board's id.
async fn new_item(app: &TestApp, kind: ItemKind, title: &str) -> String {
    let owner = app.owner().to_owned();
    let title = title.to_owned();
    app.state
        .db
        .write(move |tx| {
            let c = wikanvas_store::collections::create(
                tx,
                &owner,
                &NewCollection {
                    name: "Boards".into(),
                    ..Default::default()
                },
                0,
            )?;
            let item = wikanvas_store::items::create(
                tx,
                &owner,
                &NewItem {
                    collection_id: c.id,
                    parent_id: None,
                    kind,
                    title: Some(title),
                    position: None,
                },
                false,
                0,
            )?;
            Ok(item.id.to_string())
        })
        .await
        .expect("create the item")
}

pub async fn new_board(app: &TestApp, title: &str) -> String {
    new_item(app, ItemKind::Board, title).await
}

fn label_box(text: &str) -> Object {
    Object::new_box(
        BoxGeom::Rect,
        Frame::new(0.0, 0.0, 200.0, 80.0),
        Style::default(),
        Label::new(text, 20.0, Align::Center),
    )
}

/// A board with a labelled box and an image, named `name`.
pub fn board(name: &str, text: &str) -> String {
    let mut d = Document::new(name);
    let asset = d.add_asset("image/png", PNG.to_vec());
    d.pages[0].objects.extend([
        label_box(text),
        Object::new(
            Kind::Image(ImageObj {
                frame: Frame::new(300.0, 0.0, 40.0, 40.0),
                asset,
                latex: None,
                mono: false,
                recolor: Vec::new(),
            }),
            Style::default(),
        ),
    ]);
    serial::to_json(&d, false)
}

fn board_url(id: &str) -> String {
    format!("/api/v1/items/{id}/board")
}

async fn put(server: &TestServer, id: &str, body: &str, if_match: &str) -> axum_test::TestResponse {
    server
        .put(&board_url(id))
        .add_header("if-match", if_match.to_owned())
        .text(body.to_owned())
        .content_type("application/json")
        .await
}

async fn put_ok(server: &TestServer, id: &str, body: &str, if_match: &str) -> BoardPutResult {
    let res = put(server, id, body, if_match).await;
    res.assert_status_ok();
    let result: BoardPutResult = res.json();
    assert_eq!(res.header("etag"), result.etag.as_str());
    assert_eq!(result.etag, format!("\"r{}\"", result.rev));
    result
}

async fn query<T: Send + 'static>(
    app: &TestApp,
    sql: &'static str,
    id: &str,
    get: impl Fn(&rusqlite::Row<'_>) -> rusqlite::Result<T> + Send + 'static,
) -> T {
    let id = id.to_owned();
    app.state
        .db
        .read(move |c| Ok(c.query_row(sql, [&id], |r| get(r))?))
        .await
        .expect("query")
}

#[tokio::test]
async fn get_put_round_trip_with_etags() {
    let app = TestApp::with_owner().await;
    let id = new_board(&app, "Plan").await;
    let res = app.server.get(&board_url(&id)).await;
    res.assert_status_ok();
    assert_eq!(res.content_type(), "application/json");
    assert_eq!(res.header("etag"), "\"r1\"");
    assert_eq!(res.header("cache-control"), "no-store");
    let doc = serial::from_json(&res.text()).unwrap();
    assert_eq!((doc.name.as_str(), doc.pages.len()), ("Plan", 1));
    // Never 304, even for the current tag: the body holds the live title (D45).
    let res = app
        .server
        .get(&board_url(&id))
        .add_header("if-none-match", "W/\"r1\"")
        .await;
    res.assert_status_ok();
    assert_eq!(res.header("etag"), "\"r1\"");
    assert_eq!(serial::from_json(&res.text()).unwrap().name, "Plan");

    let body = board("Plan", "Hello board");
    let saved = put_ok(&app.server, &id, &body, "\"r1\"").await;
    assert_eq!(saved.rev, 2);
    assert_eq!(saved.meta.rev, 2);
    assert_eq!(saved.meta.objects, 2);
    assert_eq!(saved.meta.text_excerpt, "Page 1\nHello board");
    assert_eq!(saved.meta.bytes as usize, body.len());
    let res = app.server.get(&board_url(&id)).await;
    assert_eq!(res.header("etag"), "\"r2\"");
    assert_eq!(res.text(), body, "byte for byte, the image spliced back");
    // The same body again is a no-op that keeps the rev, even with a stale If-Match.
    let again = put_ok(&app.server, &id, &body, "\"r1\"").await;
    assert_eq!((again.rev, again.updated_at), (2, saved.updated_at));
    // A GET of an item that is not a board is 404.
    let doc = new_item(&app, ItemKind::Doc, "Doc").await;
    assert_error(
        &app.server.get(&board_url(&doc)).await,
        404,
        ErrorCode::NotFound,
    );
    assert_error(
        &app.server
            .get(&board_url(&uuid::Uuid::now_v7().to_string()))
            .await,
        404,
        ErrorCode::NotFound,
    );
    assert_error(
        &app.server.get("/api/v1/items/aB3dE5gH9k/board").await,
        404,
        ErrorCode::NotFound,
    );
}

#[tokio::test]
async fn missing_if_match_428() {
    let app = TestApp::with_owner().await;
    let id = new_board(&app, "B").await;
    let res = app
        .server
        .put(&board_url(&id))
        .text(board("B", "x"))
        .content_type("application/json")
        .await;
    assert_error(&res, 428, ErrorCode::PreconditionRequired);
    // Nothing changed.
    assert_eq!(
        app.server.get(&board_url(&id)).await.header("etag"),
        "\"r1\""
    );
}

#[tokio::test]
async fn stale_if_match_412_with_rev() {
    let app = TestApp::with_owner().await;
    let id = new_board(&app, "B").await;
    put_ok(&app.server, &id, &board("B", "first"), "\"r1\"").await;
    let res = put(&app.server, &id, &board("B", "second"), "\"r1\"").await;
    let body = assert_error(&res, 412, ErrorCode::RevMismatch);
    assert_eq!(
        body.error.details,
        Some(json!({"etag": "\"r2\"", "rev": 2}))
    );
    // A tag that is not a board revision never matches; a weak current one does.
    let res = put(&app.server, &id, &board("B", "second"), "\"d2\"").await;
    assert_error(&res, 412, ErrorCode::RevMismatch);
    let saved = put_ok(&app.server, &id, &board("B", "second"), "W/\"r2\"").await;
    assert_eq!(saved.rev, 3);
}

#[tokio::test]
async fn if_match_star_overwrites() {
    let app = TestApp::with_owner().await;
    let id = new_board(&app, "B").await;
    put_ok(&app.server, &id, &board("B", "one"), "\"r1\"").await;
    put_ok(&app.server, &id, &board("B", "two"), "\"r2\"").await;
    let saved = put_ok(&app.server, &id, &board("B", "three"), "*").await;
    assert_eq!(saved.rev, 4);
    assert!(
        app.server
            .get(&board_url(&id))
            .await
            .text()
            .contains("three")
    );
}

#[tokio::test]
async fn get_sets_document_name_to_title() {
    let app = TestApp::with_owner().await;
    let id = new_board(&app, "Quarterly plan").await;
    put_ok(
        &app.server,
        &id,
        &board("Whatever the client said", "x"),
        "*",
    )
    .await;
    let name = |res: &axum_test::TestResponse| res.json::<Value>()["document"]["name"].clone();
    assert_eq!(
        name(&app.server.get(&board_url(&id)).await),
        "Quarterly plan"
    );
    // A rename shows in the next GET without a board write.
    let (owner, item) = (app.owner().to_owned(), id.clone());
    app.state
        .db
        .write(move |tx| wikanvas_store::items::rename(tx, &item, "Renamed \"plan\"", &owner, 1))
        .await
        .unwrap();
    let res = app.server.get(&board_url(&id)).await;
    assert_eq!(name(&res), "Renamed \"plan\"");
    assert_eq!(res.header("etag"), "\"r2\"");
    // A client that revalidates with the tag it holds still gets the new name: the rename did not change the tag.
    let res = app
        .server
        .get(&board_url(&id))
        .add_header("if-none-match", "\"r2\"")
        .await;
    res.assert_status_ok();
    assert_eq!(name(&res), "Renamed \"plan\"");
}

#[tokio::test]
async fn over_limit_413() {
    let mut app = TestApp::with_config(|c| c.max_board_mb = 2).await;
    let owner = app.insert_owner().await;
    let session = app.insert_session(&owner).await;
    app.owner_id = Some(owner);
    let server = app.cookie_client(&session);
    let id = new_board(&app, "Big").await;
    let with_asset = |n: usize| {
        let mut d = Document::new("Big");
        d.add_asset("image/png", (0..n).map(|i| (i % 251) as u8).collect());
        serial::to_json(&d, false)
    };
    // Over WIKANVAS_MAX_BOARD_MB (2 MiB): 413 with the board limit.
    let big = with_asset(1_800_000);
    assert!(big.len() > 2 * 1024 * 1024);
    let res = put(&server, &id, &big, "*").await;
    let body = assert_error(&res, 413, ErrorCode::PayloadTooLarge);
    assert_eq!(body.error.details, Some(json!({"limit_bytes": 2097152})));
    // Under it, but over the 1 MiB default limit of the other routes: accepted.
    let medium = with_asset(1_000_000);
    assert!(medium.len() > 1024 * 1024 && medium.len() < 2 * 1024 * 1024);
    let saved = put_ok(&server, &id, &medium, "*").await;
    assert_eq!(saved.meta.bytes as usize, medium.len());
    assert_eq!(server.get(&board_url(&id)).await.text(), medium);
}

#[tokio::test]
async fn put_validates_media_type_engine_and_body() {
    let app = TestApp::with_owner().await;
    let id = new_board(&app, "B").await;
    let res = app
        .server
        .put(&board_url(&id))
        .add_header("if-match", "*")
        .text(board("B", "x"))
        .content_type("text/plain")
        .await;
    assert_error(&res, 415, ErrorCode::UnsupportedMediaType);
    let res = put(&app.server, &id, "{\"format\":\"inkboard\"", "*").await;
    assert_error(&res, 422, ErrorCode::InvalidBoard);
    let res = put(
        &app.server,
        &id,
        &board("B", "x").replacen("\"version\":2", "\"version\":3", 1),
        "*",
    )
    .await;
    assert_error(&res, 422, ErrorCode::BoardTooNew);
    let res = put(
        &app.server,
        &id,
        r#"{"format":"excalidraw","version":2,"document":{"name":"x","pages":[]}}"#,
        "*",
    )
    .await;
    assert_error(&res, 422, ErrorCode::InvalidBoard);
    let res = app
        .server
        .put(&board_url(&id))
        .add_header("if-match", "*")
        .add_header("x-wikanvas-engine", "x".repeat(200))
        .text(board("B", "x"))
        .content_type("application/json")
        .await;
    let body = assert_error(&res, 422, ErrorCode::Validation);
    assert_eq!(body.error.details, Some(json!({"field": "X-Wikanvas-Engine"})));
    // A document's board, a trashed board.
    let doc = new_item(&app, ItemKind::Doc, "D").await;
    let body = assert_error(
        &put(&app.server, &doc, &board("D", "x"), "*").await,
        422,
        ErrorCode::Validation,
    );
    assert_eq!(body.error.details, Some(json!({"field": "kind"})));
    app.exec(
        "UPDATE items SET deleted_at = 1, trash_root_id = id WHERE id = ?1",
        vec![id.clone()],
    )
    .await;
    assert_error(
        &put(&app.server, &id, &board("B", "x"), "*").await,
        409,
        ErrorCode::ReadOnly,
    );
}

#[tokio::test]
async fn put_without_engine_header_stores_api() {
    let app = TestApp::with_owner().await;
    let id = new_board(&app, "B").await;
    let engine = |app: &TestApp, id: &str| {
        let (app, id) = (app.state.clone(), id.to_owned());
        async move {
            app.db
                .read(move |c| {
                    Ok(c.query_row(
                        "SELECT engine_version FROM board_state WHERE item_id = ?1",
                        [&id],
                        |r| r.get::<_, String>(0),
                    )?)
                })
                .await
                .unwrap()
        }
    };
    assert_eq!(engine(&app, &id).await, "server");
    put_ok(&app.server, &id, &board("B", "api"), "*").await;
    assert_eq!(engine(&app, &id).await, "api");
    let res = app
        .server
        .put(&board_url(&id))
        .add_header("if-match", "*")
        .add_header("x-wikanvas-engine", "0.2.0-beta.3+embed.wikanvas.1")
        .text(board("B", "web"))
        .content_type("application/json")
        .await;
    res.assert_status_ok();
    assert_eq!(engine(&app, &id).await, "0.2.0-beta.3+embed.wikanvas.1");
}

#[tokio::test]
async fn token_put_writes_pre_api_revision() {
    let app = TestApp::with_owner().await;
    let id = new_board(&app, "B").await;
    let token = app.mint_token(app.owner(), Scope::Write).await;
    let agent = app.token_client(&token);
    let reasons = |app: &TestApp, id: &str| {
        let (state, id) = (app.state.clone(), id.to_owned());
        async move {
            state
                .db
                .read(move |c| {
                    Ok(c.prepare("SELECT reason, source_seq FROM revisions WHERE item_id = ?1 ORDER BY created_at, id")?
                        .query_map([&id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
                        .collect::<rusqlite::Result<Vec<_>>>()?)
                })
                .await
                .unwrap()
        }
    };
    // Session saves: the first writes the `auto` revision of r2, the second (within 10 minutes) none.
    put_ok(&app.server, &id, &board("B", "owner one"), "\"r1\"").await;
    put_ok(&app.server, &id, &board("B", "owner two"), "\"r2\"").await;
    assert_eq!(reasons(&app, &id).await, vec![("auto".to_owned(), 2)]);
    // The token's first PUT after the session's save keeps the owner's r3 as a `pre-api` revision …
    put_ok(&agent, &id, &board("B", "agent one"), "\"r3\"").await;
    let after = reasons(&app, &id).await;
    assert_eq!(after.len(), 2);
    assert_eq!(after[1], ("pre-api".to_owned(), 3));
    let last: Option<String> = query(
        &app,
        "SELECT last_token_id FROM board_state WHERE item_id = ?1",
        &id,
        |r| r.get(0),
    )
    .await;
    assert!(last.is_some());
    // … and its next PUT keeps none: the last write was its own.
    put_ok(&agent, &id, &board("B", "agent two"), "*").await;
    assert_eq!(reasons(&app, &id).await.len(), 2);
    // A session write clears last_token_id; the token's next PUT keeps a recovery point again.
    put_ok(&app.server, &id, &board("B", "owner three"), "*").await;
    let last: Option<String> = query(
        &app,
        "SELECT last_token_id FROM board_state WHERE item_id = ?1",
        &id,
        |r| r.get(0),
    )
    .await;
    assert_eq!(last, None);
    put_ok(&agent, &id, &board("B", "agent three"), "*").await;
    let all = reasons(&app, &id).await;
    assert_eq!(all.len(), 3);
    assert_eq!(all[2], ("pre-api".to_owned(), 6));
    // A read token cannot write.
    let reader = app.token_client(&app.mint_token(app.owner(), Scope::Read).await);
    assert_error(
        &put(&reader, &id, &board("B", "nope"), "*").await,
        403,
        ErrorCode::InsufficientScope,
    );
}

#[tokio::test]
async fn token_put_get_preserves_unknown_fields() {
    let app = TestApp::with_owner().await;
    let id = new_board(&app, "Future").await;
    let agent = app.token_client(&app.mint_token(app.owner(), Scope::Write).await);
    let mut v: Value = serde_json::from_str(&board("Future", "kept")).unwrap();
    v["document"]["pages"][0]["objects"][0]["x_future"] = 1.into();
    v["document"]["x_future"] = 1.into();
    let body = serde_json::to_string(&v).unwrap();
    put_ok(&agent, &id, &body, "*").await;
    let res = agent.get(&board_url(&id)).await;
    res.assert_status_ok();
    assert_eq!(
        res.text(),
        body,
        "unknown fields and every other byte survive"
    );
    let back: Value = res.json();
    assert_eq!(back["document"]["x_future"], 1);
    assert_eq!(back["document"]["pages"][0]["objects"][0]["x_future"], 1);
    // The text and meta still come from Inkboard's own reading of it.
    let text: String = query(
        &app,
        "SELECT body FROM item_text WHERE item_id = ?1",
        &id,
        |r| r.get(0),
    )
    .await;
    assert_eq!(text, "Page 1\nkept");
}

#[tokio::test]
async fn inkboard_download_disposition() {
    let app = TestApp::with_owner().await;
    let id = new_board(&app, "Été plan").await;
    let body = board("Été plan", "download me");
    put_ok(&app.server, &id, &body, "*").await;
    let res = app
        .server
        .get(&format!("/api/v1/items/{id}/board.inkboard"))
        .await;
    res.assert_status_ok();
    assert_eq!(
        res.header("content-disposition"),
        "attachment; filename=\"_t_ plan.inkboard\"; filename*=UTF-8''%C3%89t%C3%A9%20plan.inkboard"
    );
    assert_eq!(res.content_type(), "application/json");
    assert_eq!(res.header("etag"), "\"r2\"");
    assert_eq!(res.text(), body, "the same bytes as GET …/board");
    assert_eq!(res.text(), app.server.get(&board_url(&id)).await.text());
}

#[tokio::test]
async fn preview_route_headers_immutable_307_and_current_without_rev() {
    let app = TestApp::with_owner().await;
    let id = new_board(&app, "Pages").await;
    let mut d = Document::new("Pages");
    d.pages[0].objects.push(label_box("Page zero label"));
    // Enough to make the page's SVG larger than 1 KiB, the compression threshold.
    for i in 0..4 {
        d.pages[0]
            .objects
            .push(label_box(&format!("Another label {i}")));
    }
    d.pages.push(Page::new("Empty page"));
    put_ok(&app.server, &id, &serial::to_json(&d, false), "*").await;
    let page = |n: &str| format!("/api/v1/items/{id}/board/pages/{n}");

    // Without rev: the current page, rendered on demand, no-cache with an ETag.
    let res = app.server.get(&page("0")).await;
    res.assert_status_ok();
    assert_eq!(res.content_type(), "image/svg+xml");
    assert_eq!(res.header("cache-control"), "no-cache");
    assert_eq!(res.header("etag"), "\"r2\"");
    assert_eq!(res.header("x-content-type-options"), "nosniff");
    assert_eq!(
        res.header("content-security-policy"),
        "default-src 'none'; img-src data:; style-src 'unsafe-inline'; sandbox"
    );
    let svg = res.text();
    assert!(
        svg.contains("<svg") && svg.contains("Page zero label"),
        "{svg}"
    );
    let res = app
        .server
        .get(&page("0"))
        .add_header("if-none-match", "\"r2\"")
        .await;
    assert_eq!(res.status_code(), 304);
    // The current rev: immutable, from the stored preview.
    let res = app.server.get(&format!("{}?rev=2", page("0"))).await;
    res.assert_status_ok();
    assert_eq!(
        res.header("cache-control"),
        "private, max-age=31536000, immutable"
    );
    assert_eq!(res.text(), svg);
    // Another rev: 307 to the current one.
    let res = app.server.get(&format!("{}?rev=1", page("0"))).await;
    assert_eq!(res.status_code(), 307);
    assert_eq!(
        res.header("location"),
        format!("/api/v1/items/{id}/board/pages/0?rev=2").as_str()
    );
    // An empty page: 204, with the same cache rules.
    let res = app.server.get(&format!("{}?rev=2", page("1"))).await;
    assert_eq!(res.status_code(), 204);
    assert_eq!(
        res.header("cache-control"),
        "private, max-age=31536000, immutable"
    );
    let res = app.server.get(&page("1")).await;
    assert_eq!(res.status_code(), 204);
    assert_eq!(res.header("cache-control"), "no-cache");
    // No such page.
    for n in ["2", "-1", "x"] {
        assert_error(&app.server.get(&page(n)).await, 404, ErrorCode::NotFound);
    }
    // gzip for a client that accepts it: the server's compression layer (§11), the page being over 1 KiB.
    assert!(svg.len() > 1024, "{}", svg.len());
    let res = app
        .server
        .get(&page("0"))
        .add_header("accept-encoding", "gzip")
        .await;
    assert_eq!(res.header("content-encoding"), "gzip");
    assert_eq!(res.header("vary"), "Accept-Encoding");
    assert_eq!(gunzip(&res.as_bytes()[..]), svg);
    // The previews are stored for r2: BoardMeta reports them.
    let item = id.clone();
    let meta = app
        .state
        .db
        .read(move |c| wikanvas_store::boards::board_meta(c, &item))
        .await
        .unwrap();
    assert_eq!(meta.preview_rev, Some(2));
    assert_eq!(meta.pages[0].svg_len, Some(svg.len() as u64));
    assert_eq!(meta.pages[1].svg_len, Some(0));
    // `ItemDetail.board` is that same BoardMeta, also when the stored previews come from different revisions: the
    // newest one counts, and a page whose preview is older has no `svg_len`.
    let detail = app
        .server
        .get(&format!("/api/v1/items/{id}"))
        .await
        .json::<wikanvas_api::ItemDetail>();
    assert_eq!(detail.board.as_ref(), Some(&meta));
    let item = id.clone();
    app.state
        .db
        .write(move |tx| {
            Ok(tx.execute(
                "UPDATE board_previews SET rev = 1 WHERE item_id = ?1 AND page_index = 1",
                [item],
            )?)
        })
        .await
        .unwrap();
    let item = id.clone();
    let mixed = app
        .state
        .db
        .read(move |c| wikanvas_store::boards::board_meta(c, &item))
        .await
        .unwrap();
    assert_eq!((mixed.preview_rev, mixed.pages[1].svg_len), (Some(2), None));
    let detail = app
        .server
        .get(&format!("/api/v1/items/{id}"))
        .await
        .json::<wikanvas_api::ItemDetail>();
    assert_eq!(detail.board, Some(mixed));
    // A new revision makes the old URL redirect, and the plain URL serve the new page.
    let mut d2 = d.clone();
    d2.pages[0].objects.push(label_box("Second label"));
    put_ok(&app.server, &id, &serial::to_json(&d2, false), "*").await;
    let res = app.server.get(&format!("{}?rev=2", page("0"))).await;
    assert_eq!(res.status_code(), 307);
    let res = app.server.get(&page("0")).await;
    assert_eq!(res.header("etag"), "\"r3\"");
    assert!(res.text().contains("Second label"));
}

/// A page with no stored preview is rendered on demand and sent as a stream (its size is not known when the body
/// budget is taken); the compression layer then gzips that stream for a gzip client. Found by the W7 review fix of the
/// board embed card (the seeded board's preview is first rendered this way): the stream must stay ended once it has
/// ended, or the encoder's last poll panics the connection task.
#[tokio::test]
async fn on_demand_preview_gzipped_for_a_gzip_client() {
    let app = TestApp::with_owner().await;
    let id = new_board(&app, "On demand").await;
    let mut d = Document::new("On demand");
    for i in 0..6 {
        d.pages[0]
            .objects
            .push(label_box(&format!("On-demand label {i}")));
    }
    put_ok(&app.server, &id, &serial::to_json(&d, false), "*").await;
    let url = format!("/api/v1/items/{id}/board/pages/0");
    // No preview stored yet (the job does not run in tests): rendered now, streamed, gzipped by the layer.
    let res = app
        .server
        .get(&url)
        .add_header("accept-encoding", "gzip")
        .await;
    res.assert_status_ok();
    assert_eq!(res.content_type(), "image/svg+xml");
    assert_eq!(res.header("content-encoding"), "gzip");
    let svg = gunzip(&res.as_bytes()[..]);
    assert!(
        svg.len() > 1024 && svg.contains("On-demand label 5"),
        "{svg}"
    );
    // Stored now: the next GET is the same SVG, from the store.
    assert_eq!(app.server.get(&url).await.text(), svg);
}

/// A minimal gunzip (the server crate has no gzip decoder): the 10-byte header without optional fields that the
/// encoder writes, the raw DEFLATE stream, and the 8-byte trailer.
fn gunzip(bytes: &[u8]) -> String {
    assert_eq!(
        &bytes[..4],
        &[0x1f, 0x8b, 8, 0],
        "gzip, deflate, no optional fields"
    );
    wikanvas_store::boards::inflate(&bytes[10..bytes.len() - 8]).expect("inflate")
}

#[tokio::test]
async fn preview_worker_renders_enqueued_boards() {
    let app = TestApp::with_owner().await;
    let id = new_board(&app, "Worker").await;
    let mut d = Document::new("Worker");
    d.pages[0].objects.push(label_box("Background"));
    put_ok(&app.server, &id, &serial::to_json(&d, false), "*").await;
    let queue = wikanvas_server::board_previews::spawn_with(
        app.state.clone(),
        Duration::from_millis(20),
        Duration::from_millis(200),
    );
    queue.enqueue(&id);
    queue.enqueue(&id);
    queue.enqueue("not-a-board");
    let mut rendered = None;
    for _ in 0..200 {
        let item = id.clone();
        let rev = app
            .state
            .db
            .read(move |c| wikanvas_store::previews::rev(c, &item))
            .await
            .unwrap();
        if rev.is_some() {
            rendered = rev;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(rendered, Some(2));
    let item = id.clone();
    let stored = app
        .state
        .db
        .read(move |c| wikanvas_store::previews::get(c, &item, 0, 2))
        .await
        .unwrap();
    match stored {
        Some(wikanvas_store::previews::Preview::Svg(svg)) => {
            assert!(svg.contains("Background"), "{svg}")
        }
        other => panic!("no stored preview: {other:?}"),
    }
}

#[tokio::test]
async fn streamed_body_over_limit_413_without_content_length() {
    use axum::body::Body;
    use tower::ServiceExt as _;
    let mut app = TestApp::with_config(|c| c.max_board_mb = 1).await;
    let owner = app.insert_owner().await;
    let session = app.insert_session(&owner).await;
    app.owner_id = Some(owner);
    let id = new_board(&app, "Streamed").await;
    // 2 MiB in 64 KiB chunks, with no Content-Length: only the streaming check can refuse it.
    let chunks =
        (0..32).map(|_| Ok::<_, std::io::Error>(bytes::Bytes::from(vec![b' '; 64 * 1024])));
    let request = axum::http::Request::put(board_url(&id))
        .header("host", "localhost")
        .header("sec-fetch-site", "same-origin")
        .header("cookie", format!("wikanvas_session={session}"))
        .header("if-match", "*")
        .header("content-type", "application/json")
        .body(Body::from_stream(futures_util::stream::iter(chunks)))
        .unwrap();
    assert!(request.headers().get("content-length").is_none());
    let res = wikanvas_server::app(app.state.clone())
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(res.status(), 413);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: wikanvas_api::ErrorBody = serde_json::from_slice(&body).unwrap();
    assert_eq!(body.error.code, ErrorCode::PayloadTooLarge);
    assert_eq!(body.error.details, Some(json!({"limit_bytes": 1048576})));
    // Nothing was written.
    assert_eq!(
        app.cookie_client(&session)
            .get(&board_url(&id))
            .await
            .header("etag"),
        "\"r1\""
    );
}

#[tokio::test]
async fn put_refuses_json_nested_deeper_than_the_limit() {
    let app = TestApp::with_owner().await;
    let id = new_board(&app, "Deep").await;
    let deep = |levels: usize| {
        let mut v: Value = serde_json::from_str(&board("Deep", "x")).unwrap();
        // The envelope is one level; the arrays under `x_future` add the rest.
        let mut inner = json!(0);
        for _ in 0..levels - 1 {
            inner = json!([inner]);
        }
        v["x_future"] = inner;
        serde_json::to_string(&v).unwrap()
    };
    let res = put(&app.server, &id, &deep(101), "*").await;
    let body = assert_error(&res, 422, ErrorCode::InvalidBoard);
    assert!(
        body.error.message.contains("101 levels"),
        "{}",
        body.error.message
    );
    // At the limit (and below serde_json's own 128): accepted, and kept.
    put_ok(&app.server, &id, &deep(100), "*").await;
    assert_eq!(
        app.server
            .get(&board_url(&id))
            .await
            .text()
            .matches('[')
            .count(),
        99 + board("Deep", "x").matches('[').count()
    );
}

#[tokio::test]
async fn repeated_image_preview_is_bounded() {
    let app = TestApp::with_owner().await;
    let id = new_board(&app, "Repeated").await;
    // One 1 MiB image drawn 300 times: a 1.4 MB envelope that would inline 400 MB of base64.
    let mut d = Document::new("Repeated");
    let big: Vec<u8> = PNG
        .iter()
        .copied()
        .chain((0..1 << 20).map(|i| (i % 251) as u8))
        .collect();
    let asset = d.add_asset("image/png", big);
    for k in 0..300 {
        d.pages[0].objects.push(Object::new(
            Kind::Image(ImageObj {
                frame: Frame::new(f64::from(k) * 50.0, 0.0, 40.0, 40.0),
                asset: asset.clone(),
                latex: None,
                mono: false,
                recolor: Vec::new(),
            }),
            Style::default(),
        ));
    }
    d.pages[0].objects.push(label_box("Still drawn"));
    let body = serial::to_json(&d, false);
    assert!(body.len() < 2 << 20);
    put_ok(&app.server, &id, &body, "*").await;
    let res = app
        .server
        .get(&format!("/api/v1/items/{id}/board/pages/0"))
        .await;
    res.assert_status_ok();
    let svg = res.text();
    assert!(svg.len() < 1 << 20, "{} bytes", svg.len());
    assert!(svg.contains("Still drawn"));
    assert_eq!(
        svg.matches("<image ").count(),
        300,
        "each copy is a placeholder"
    );
    // The large envelope itself comes back whole (a streamed body under the body budget).
    let res = app.server.get(&board_url(&id)).await;
    res.assert_status_ok();
    assert_eq!(
        res.header("content-length"),
        body.len().to_string().as_str()
    );
    assert_eq!(res.text(), body);
}

#[tokio::test]
async fn page_request_that_waited_for_a_render_serves_the_stored_page() {
    let app = TestApp::with_owner().await;
    let id = new_board(&app, "Waiting").await;
    let mut d = Document::new("Waiting");
    d.pages[0].objects.push(label_box("Rendered by the worker"));
    put_ok(&app.server, &id, &serial::to_json(&d, false), "*").await;
    // The worker holds the permit; a GET comes in, finds no preview, and waits for the permit.
    let permit = app.state.board_work.clone().acquire_owned().await.unwrap();
    let client = app.cookie_client(app.session.as_deref().unwrap());
    let url = format!("/api/v1/items/{id}/board/pages/0");
    let waiting = tokio::spawn(async move { client.get(&url).await.text() });
    tokio::time::sleep(Duration::from_millis(300)).await;
    // Meanwhile the worker stores the previews of the current revision.
    let item = id.clone();
    app.state
        .db
        .write(move |tx| {
            wikanvas_store::previews::replace(tx, &item, 2, &[Some("<svg>stored</svg>".to_owned())])
        })
        .await
        .unwrap();
    drop(permit);
    // The GET serves what was stored instead of rendering again.
    assert_eq!(waiting.await.unwrap(), "<svg>stored</svg>");
}

#[tokio::test]
async fn page_after_the_preview_budget_is_a_card() {
    let app = TestApp::with_owner().await;
    let id = new_board(&app, "Budget").await;
    let mut d = Document::new("Budget");
    d.pages[0].objects.push(label_box("first"));
    let mut second = Page::new("Second");
    second.objects.push(label_box("second"));
    d.pages.push(second);
    put_ok(&app.server, &id, &serial::to_json(&d, false), "*").await;
    // A render that stopped after page 0 (the board's budget was spent).
    let item = id.clone();
    app.state
        .db
        .write(move |tx| {
            wikanvas_store::previews::replace(tx, &item, 2, &[Some("<svg>zero</svg>".to_owned())])
        })
        .await
        .unwrap();
    let res = app
        .server
        .get(&format!("/api/v1/items/{id}/board/pages/1?rev=2"))
        .await;
    res.assert_status_ok();
    assert_eq!(res.content_type(), "image/svg+xml");
    assert_eq!(
        res.header("cache-control"),
        "private, max-age=31536000, immutable"
    );
    assert_eq!(
        res.text(),
        wikanvas_core::board::card_svg("#ffffff", wikanvas_core::board::TOO_LARGE)
    );
    // A page under 1 KiB is sent identity even to a gzip client: one compression rule, the layer's (§11).
    let res = app
        .server
        .get(&format!("/api/v1/items/{id}/board/pages/0?rev=2"))
        .add_header("accept-encoding", "gzip")
        .await;
    res.assert_status_ok();
    assert!(res.maybe_header("content-encoding").is_none());
    assert_eq!(res.header("vary"), "Accept-Encoding");
    assert_eq!(res.text(), "<svg>zero</svg>");
}

// ---------------------------------------------------------------- board document embeds (M1.1, §18.8; owner: E2)

/// The placeholder targets of `fixtures/boards/embed.inkboard` (§18.2).
const P1: &str = "0192f0e0-0000-7000-8000-000000000001";
const P2: &str = "0192f0e0-0000-7000-8000-000000000002";

/// The shared fixture: two cards ("First page" → [`P1`], "Second page" → [`P2`]) and a plain box, named "Embed test".
fn embed_fixture() -> String {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/boards/embed.inkboard");
    std::fs::read_to_string(&path).expect("the embed fixture")
}

/// The fixture with its targets replaced.
fn embed_board(first: &str, second: &str) -> String {
    embed_fixture().replace(P1, first).replace(P2, second)
}

/// The `embed` link targets of `board`, sorted.
async fn embed_links(app: &TestApp, board: &str) -> Vec<String> {
    let board = board.to_owned();
    app.state
        .db
        .read(move |c| {
            Ok(c.prepare(
                "SELECT target_id FROM item_links WHERE source_id = ?1 AND kind = 'embed' ORDER BY target_id",
            )?
            .query_map([&board], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?)
        })
        .await
        .expect("links")
}

fn sorted(ids: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = ids.iter().map(|s| (*s).to_owned()).collect();
    v.sort();
    v
}

#[tokio::test]
async fn put_with_embeds_keeps_them_byte_for_byte() {
    let app = TestApp::with_owner().await;
    // Titled as the fixture is named, so a GET sends the PUT's exact bytes back.
    let id = new_board(&app, "Embed test").await;
    let agent = app.token_client(&app.mint_token(app.owner(), Scope::Write).await);
    let swapped = embed_board(P2, P1);
    for (client, body) in [(&app.server, embed_fixture()), (&agent, swapped)] {
        let saved = put_ok(client, &id, &body, "*").await;
        assert_eq!(saved.meta.objects, 3);
        assert_eq!(
            saved.meta.text_excerpt,
            "Page 1\nFirst page\nSecond page\nPlain box"
        );
        let res = client.get(&board_url(&id)).await;
        res.assert_status_ok();
        assert_eq!(res.text(), body, "every byte of the cards survives");
        let back: Value = res.json();
        assert_eq!(
            back["document"]["pages"][0]["objects"][2]["kind"].get("embed"),
            None,
            "the plain box gets no embed"
        );
        // The .inkboard download is the same envelope.
        let dl = client
            .get(&format!("/api/v1/items/{id}/board.inkboard"))
            .await;
        dl.assert_status_ok();
        assert_eq!(dl.text(), body);
    }
    // With another title only `document.name` differs.
    let renamed = new_board(&app, "Cards").await;
    put_ok(&agent, &renamed, &embed_fixture(), "\"r1\"").await;
    assert_eq!(
        agent.get(&board_url(&renamed)).await.text(),
        embed_fixture().replacen("\"name\":\"Embed test\"", "\"name\":\"Cards\"", 1)
    );
}

#[tokio::test]
async fn malformed_embeds_are_accepted_and_kept() {
    let app = TestApp::with_owner().await;
    let id = new_board(&app, "Embed test").await;
    let mut v: Value = serde_json::from_str(&embed_fixture()).unwrap();
    let objects = v["document"]["pages"][0]["objects"].as_array_mut().unwrap();
    let card = objects[0].clone();
    objects[0]["kind"]["embed"] = json!(5);
    objects[1]["kind"]["embed"] = json!({"item": "not-a-uuid"});
    let variants = [
        json!({}),
        json!({"item": 3}),
        json!(null),
        json!({"item": P1.to_uppercase()}),
        json!({"item": P2, "x_future": {"kept": true}}),
    ];
    for (n, embed) in variants.into_iter().enumerate() {
        let mut o = card.clone();
        o["id"] = format!("variant{n}").into();
        o["kind"]["embed"] = embed;
        objects.push(o);
    }
    let mut ellipse = card.clone();
    ellipse["id"] = "ellipse".into();
    ellipse["kind"]["geom"] = "ellipse".into();
    ellipse["kind"]["embed"] = json!({"item": P1});
    objects.push(ellipse);
    let body = serde_json::to_string(&v).unwrap();
    let agent = app.token_client(&app.mint_token(app.owner(), Scope::Write).await);
    let saved = put_ok(&agent, &id, &body, "\"r1\"").await;
    assert_eq!(saved.meta.objects, 9);
    let res = agent.get(&board_url(&id)).await;
    assert_eq!(
        res.text(),
        body,
        "no value of `embed` is refused or changed"
    );
    // Only the canonical UUID of a rect card is linked (the extra field inside `embed` does not matter).
    assert_eq!(embed_links(&app, &id).await, sorted(&[P2]));
}

#[tokio::test]
async fn repeated_embed_key_is_refused() {
    let app = TestApp::with_owner().await;
    let id = new_board(&app, "Embed test").await;
    let stored = embed_fixture();
    put_ok(&app.server, &id, &stored, "\"r1\"").await;
    let card = format!(r#""embed":{{"item":"{P1}"}}"#);
    let repeated = stored.replacen(&card, &format!(r#"{card},"embed":{{"item":"{P2}"}}"#), 1);
    assert_ne!(repeated, stored);
    let agent = app.token_client(&app.mint_token(app.owner(), Scope::Write).await);
    for client in [&app.server, &agent] {
        let res = put(client, &id, &repeated, "*").await;
        let body = assert_error(&res, 422, ErrorCode::InvalidBoard);
        assert!(
            body.error.message.contains("duplicate field `embed`"),
            "{}",
            body.error.message
        );
    }
    // The stored board is unchanged: same revision, same bytes, same links.
    let res = app.server.get(&board_url(&id)).await;
    assert_eq!(res.header("etag"), "\"r2\"");
    assert_eq!(res.text(), stored);
    assert_eq!(embed_links(&app, &id).await, sorted(&[P1, P2]));
    // The same key once in each of two boxes is what every card holds.
    put_ok(&app.server, &id, &embed_board(P2, P1), "\"r2\"").await;
}

#[tokio::test]
async fn put_with_embeds_records_links() {
    let app = TestApp::with_owner().await;
    let id = new_board(&app, "Board").await;
    let doc = new_item(&app, ItemKind::Doc, "Notes").await;
    assert_eq!(embed_links(&app, &id).await, sorted(&[]));
    put_ok(&app.server, &id, &embed_board(&doc, P2), "\"r1\"").await;
    assert_eq!(
        embed_links(&app, &id).await,
        sorted(&[&doc, P2]),
        "existence is not checked"
    );
    // The same body again (a no-op) keeps them; a body with the board itself as a target never links it.
    put_ok(&app.server, &id, &embed_board(&doc, P2), "\"r1\"").await;
    assert_eq!(embed_links(&app, &id).await, sorted(&[&doc, P2]));
    put_ok(&app.server, &id, &embed_board(&id, &doc), "\"r2\"").await;
    assert_eq!(embed_links(&app, &id).await, sorted(&[&doc]));
    // A board without cards has none.
    put_ok(&app.server, &id, &board("Board", "no cards"), "\"r3\"").await;
    assert_eq!(embed_links(&app, &id).await, sorted(&[]));
    // A "Text doc" (a new, empty, untitled child of a board, made through the API) cannot be discarded while a card
    // shows it; once the board's save without the card is in, it can.
    let c = crate::items::collection(&app, "Embeds").await;
    let parent = crate::items::item(&app, c.id, None, "board", "Parent").await;
    let text_doc = crate::items::item(&app, c.id, Some(parent.id), "doc", "").await;
    let (parent, text_doc) = (parent.id.to_string(), text_doc.id.to_string());
    put_ok(
        &app.server,
        &parent,
        &embed_board(&text_doc, &doc),
        "\"r1\"",
    )
    .await;
    let discard = |item: &str| {
        app.server
            .delete(&format!("/api/v1/items/{item}?discard_empty=true"))
    };
    let refused = assert_error(&discard(&text_doc).await, 409, ErrorCode::NotEmpty);
    assert!(
        refused.error.message.contains("embeds"),
        "{}",
        refused.error.message
    );
    put_ok(&app.server, &parent, &embed_board(&doc, &doc), "\"r2\"").await;
    let res = discard(&text_doc).await;
    res.assert_status_ok();
    res.assert_json(&json!({"trashed": 1}));
}

#[tokio::test]
async fn duplicate_board_copies_embed_links() {
    let app = TestApp::with_owner().await;
    let id = new_board(&app, "Embed test").await;
    let doc = new_item(&app, ItemKind::Doc, "Notes").await;
    put_ok(&app.server, &id, &embed_board(&doc, P2), "\"r1\"").await;
    let res = app
        .server
        .post(&format!("/api/v1/items/{id}/duplicate"))
        .await;
    res.assert_status(axum::http::StatusCode::CREATED);
    let copy: wikanvas_api::ItemSummary = res.json();
    let copy = copy.id.to_string();
    assert_eq!(embed_links(&app, &copy).await, sorted(&[&doc, P2]));
    assert_eq!(embed_links(&app, &id).await, sorted(&[&doc, P2]));
    // The copy's cards show the same documents: the body is the original's, under the copy's title.
    let body = app.server.get(&board_url(&copy)).await.text();
    assert_eq!(
        body,
        embed_board(&doc, P2).replacen(
            "\"name\":\"Embed test\"",
            "\"name\":\"Embed test (copy)\"",
            1
        )
    );
}
